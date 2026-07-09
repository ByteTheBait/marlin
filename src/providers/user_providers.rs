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
