use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Endpoint validation ───────────────────────────────────────────────────────

/// Rejects anything that isn't a usable http(s) API base URL — in particular
/// `file://` and other schemes that have no business being a chat-completions
/// endpoint.
pub fn validate_endpoint(endpoint: &str) -> std::result::Result<(), String> {
    let url = reqwest::Url::parse(endpoint).map_err(|e| format!("not a valid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!(
            "unsupported scheme {other:?} — only http/https endpoints are supported"
        )),
    }
}

/// True if `endpoint`'s host is loopback, private, or link-local.
///
/// Informational only — never used to block, since pointing a provider at a
/// local/self-hosted inference server (Ollama, LM Studio, a home-LAN vLLM
/// box, ...) is a legitimate and common setup. Used to warn when an endpoint
/// change might be redirecting a *stored API key* somewhere the user didn't
/// intend, not to gate self-hosted use.
pub fn endpoint_is_private_host(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else { return false };
    let Some(host) = url.host_str() else { return false };
    // IPv6 literals come back bracketed (e.g. "[::1]") — strip for parsing.
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => {
            if v6.is_loopback() {
                return true;
            }
            let seg0 = v6.segments()[0];
            seg0 & 0xfe00 == 0xfc00 // fc00::/7 (unique local)
                || seg0 & 0xffc0 == 0xfe80 // fe80::/10 (link local)
        }
        Err(_) => false,
    }
}

// ── User-defined provider ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProvider {
    /// Provider name used in /provider <name> and config.
    pub name: String,
    /// OpenAI-compatible API base URL (e.g. "https://api.example.com/v1").
    pub endpoint: String,
    /// API key. Leave empty for local providers (Ollama, etc.).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// Default model for this provider.
    pub model: String,
    /// Optional list of models shown by /models. Falls back to [model] if empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

impl UserProvider {
    pub fn model_list(&self) -> Vec<String> {
        if self.models.is_empty() {
            vec![self.model.clone()]
        } else {
            self.models.clone()
        }
    }
}

// ── I/O ──────────────────────────────────────────────────────────────────────

pub fn providers_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("providers");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load_all(marlin_dir: &Path) -> Vec<UserProvider> {
    let dir = providers_dir(marlin_dir);
    let mut providers = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return providers };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") { continue; }
        if let Ok(data) = std::fs::read_to_string(&path) {
            match toml::from_str::<UserProvider>(&data) {
                Ok(p) => providers.push(p),
                Err(e) => eprintln!("providers: parse error in {:?}: {e}", path.file_name()),
            }
        }
    }
    providers.sort_by(|a, b| a.name.cmp(&b.name));
    providers
}

/// Update a field on a user-defined provider's toml file in place, matching
/// by name (case-insensitive). Returns `false` if no such provider exists —
/// callers should fall back to built-in config storage in that case.
fn set_field(marlin_dir: &Path, name: &str, f: impl FnOnce(&mut UserProvider)) -> Result<bool> {
    let dir = providers_dir(marlin_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(false) };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") { continue; }
        let Ok(data) = std::fs::read_to_string(&path) else { continue };
        let Ok(mut p) = toml::from_str::<UserProvider>(&data) else { continue };
        if p.name.eq_ignore_ascii_case(name) {
            f(&mut p);
            std::fs::write(&path, toml::to_string_pretty(&p)?)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Store an API key for a user-defined provider. Returns `false` if `name`
/// doesn't match any provider in `~/.marlin/providers/*.toml`.
pub fn set_api_key(marlin_dir: &Path, name: &str, key: &str) -> Result<bool> {
    set_field(marlin_dir, name, |p| p.api_key = key.to_string())
}

/// Update the endpoint for a user-defined provider. Returns `false` if `name`
/// doesn't match any provider in `~/.marlin/providers/*.toml`.
pub fn set_endpoint(marlin_dir: &Path, name: &str, endpoint: &str) -> Result<bool> {
    validate_endpoint(endpoint).map_err(|e| anyhow::anyhow!(e))?;
    set_field(marlin_dir, name, |p| p.endpoint = endpoint.to_string())
}

fn write_provider(marlin_dir: &Path, p: &UserProvider) -> Result<PathBuf> {
    let filename = format!("{}.toml", p.name.replace([' ', '/'], "_").to_lowercase());
    let path = providers_dir(marlin_dir).join(filename);
    std::fs::write(&path, toml::to_string_pretty(p)?)?;
    Ok(path)
}

pub fn save_template(marlin_dir: &Path, name: &str) -> Result<PathBuf> {
    write_provider(marlin_dir, &UserProvider {
        name: name.to_string(),
        endpoint: "https://api.example.com/v1".into(),
        api_key: String::new(),
        model: "model-name".into(),
        models: vec!["model-name".into(), "model-name-fast".into()],
    })
}

/// Create a user-defined provider from values gathered interactively (the
/// config menu's "New provider" wizard). Blank `endpoint`/`model` fall back
/// to the same placeholders `save_template` uses, so the file is still valid
/// TOML the user can hand-edit later; a non-blank endpoint is validated.
pub fn save_new(marlin_dir: &Path, name: &str, endpoint: &str, model: &str, api_key: &str) -> Result<PathBuf> {
    let endpoint = if endpoint.is_empty() {
        "https://api.example.com/v1".to_string()
    } else {
        validate_endpoint(endpoint).map_err(|e| anyhow::anyhow!(e))?;
        endpoint.to_string()
    };
    let model = if model.is_empty() { "model-name".to_string() } else { model.to_string() };
    write_provider(marlin_dir, &UserProvider {
        name: name.to_string(),
        endpoint,
        api_key: api_key.to_string(),
        model: model.clone(),
        models: vec![model],
    })
}

// ── Default providers ─────────────────────────────────────────────────────────

/// Install one working example provider on first run — same write-if-missing
/// pattern as `skills::install_defaults`. Never overwrites a user's file.
/// Local (no API key needed) so it's genuinely functional out of the box for
/// anyone running a local OpenAI-compatible server, not just a syntax sample.
pub fn install_defaults(marlin_dir: &Path) {
    let dir = providers_dir(marlin_dir);
    for p in default_providers() {
        let path = dir.join(format!("{}.toml", p.name));
        if !path.exists() {
            if let Ok(data) = toml::to_string_pretty(&p) {
                let _ = std::fs::write(path, data);
            }
        }
    }
}

fn default_providers() -> Vec<UserProvider> {
    vec![UserProvider {
        name: "lmstudio".into(),
        endpoint: "http://localhost:1234/v1".into(),
        api_key: String::new(),
        model: "local-model".into(),
        models: vec![],
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_endpoint_accepts_http_and_https() {
        assert!(validate_endpoint("https://api.example.com/v1").is_ok());
        assert!(validate_endpoint("http://localhost:1234/v1").is_ok());
    }

    #[test]
    fn validate_endpoint_rejects_non_http_schemes() {
        assert!(validate_endpoint("file:///etc/passwd").is_err());
        assert!(validate_endpoint("ftp://example.com").is_err());
    }

    #[test]
    fn validate_endpoint_rejects_garbage() {
        assert!(validate_endpoint("not a url").is_err());
    }

    #[test]
    fn private_host_detection() {
        assert!(endpoint_is_private_host("http://localhost:1234/v1"));
        assert!(endpoint_is_private_host("http://127.0.0.1:8080"));
        assert!(endpoint_is_private_host("http://192.168.1.50:8000"));
        assert!(endpoint_is_private_host("http://[::1]:8080"));
        assert!(!endpoint_is_private_host("https://api.example.com/v1"));
        assert!(!endpoint_is_private_host("https://8.8.8.8/v1"));
    }
}
