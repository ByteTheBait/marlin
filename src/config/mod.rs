use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub active_provider: String,
    pub active_model: String,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub work_dir: String,
}

fn default_max_tokens() -> usize {
    4096
}

pub fn marlin_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let dir = home.join(".marlin");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = marlin_dir()?.join("config.json");
        let mut cfg = Self::defaults();
        if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            let loaded: Self = serde_json::from_str(&data)?;
            // Merge: loaded values override defaults, but fill missing providers
            let mut merged = loaded;
            for (k, v) in Self::defaults().providers {
                merged.providers.entry(k).or_insert(v);
            }
            cfg = merged;
        } else {
            cfg.save()?;
        }
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = marlin_dir()?.join("config.json");
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn set_key(&mut self, provider: &str, key: &str) {
        let entry = self.providers.entry(provider.to_string()).or_default();
        entry.api_key = key.to_string();
    }

    pub fn set_endpoint(&mut self, provider: &str, endpoint: &str) {
        let entry = self.providers.entry(provider.to_string()).or_default();
        entry.endpoint = endpoint.to_string();
    }

    pub fn defaults() -> Self {
        let wd = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut providers = HashMap::new();
        providers.insert("claude".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: String::new(),
            model: "claude-sonnet-4-5".into(),
        });
        providers.insert("ollama".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: "http://localhost:11434".into(),
            model: "llama3".into(),
        });
        providers.insert("fireworks".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: "https://api.fireworks.ai/inference/v1".into(),
            model: "accounts/fireworks/models/llama-v3-70b-instruct".into(),
        });
        providers.insert("moonshot".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: "https://api.moonshot.cn/v1".into(),
            model: "moonshot-v1-8k".into(),
        });
        providers.insert("groq".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: "https://api.groq.com/openai/v1".into(),
            model: "llama-3.3-70b-versatile".into(),
        });
        providers.insert("custom".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: "http://localhost:8080/v1".into(),
            model: "default".into(),
        });

        Self {
            active_provider: "claude".into(),
            active_model: "claude-sonnet-4-5".into(),
            providers,
            allowed_commands: vec![],
            system_prompt: String::new(),
            max_tokens: 4096,
            work_dir: wd,
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            endpoint: String::new(),
            model: String::new(),
        }
    }
}
