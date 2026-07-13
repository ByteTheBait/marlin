use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

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
    set_field(marlin_dir, name, |p| p.endpoint = endpoint.to_string())
}

pub fn save_template(marlin_dir: &Path, name: &str) -> Result<PathBuf> {
    let p = UserProvider {
        name: name.to_string(),
        endpoint: "https://api.example.com/v1".into(),
        api_key: String::new(),
        model: "model-name".into(),
        models: vec!["model-name".into(), "model-name-fast".into()],
    };
    let filename = format!("{}.toml", name.replace([' ', '/'], "_").to_lowercase());
    let path = providers_dir(marlin_dir).join(filename);
    std::fs::write(&path, toml::to_string_pretty(&p)?)?;
    Ok(path)
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
