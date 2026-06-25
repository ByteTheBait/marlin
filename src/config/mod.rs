use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AstMode {
    Off,
    SExpr,
    Harness,
}

impl Default for AstMode {
    fn default() -> Self { Self::Off }
}

impl AstMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::SExpr => "sexpr",
            Self::Harness => "harness",
        }
    }
}

/// How shell commands from the AI are executed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    /// Commands require explicit /allow approval (default).
    Off,
    /// All commands allowed; run directly on the host (old "sandbox: true" behaviour).
    Permissive,
    /// Commands run via Microsoft eXecution Containers (MXC) — no outbound network,
    /// only the workdir mounted read-write.
    Mxc,
}

impl Default for SandboxMode {
    fn default() -> Self { Self::Off }
}

impl SandboxMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Permissive => "permissive",
            Self::Mxc => "mxc",
        }
    }

    /// True if all commands are implicitly allowed (no per-command /allow needed).
    pub fn allows_all(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

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
    /// Legacy field — migrated to sandbox_mode on first load.
    #[serde(default, skip_serializing)]
    pub sandbox: bool,
    /// How AI-issued shell commands are sandboxed.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,
    /// Skip per-operation permission checks for file writes/edits.
    #[serde(default)]
    pub skip_permissions: bool,
    /// UI theme: "dark" or "light"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Shell command to run after every file edit (Write-Test-Fix loop)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_command: Option<String>,
    /// Strip environment variables from subprocesses (clean-env isolation)
    #[serde(default)]
    pub clean_env: bool,
    /// AST-driven context mode (off / sexpr / harness)
    #[serde(default)]
    pub ast_mode: AstMode,
}

fn default_max_tokens() -> usize {
    4096
}

fn default_theme() -> String {
    "dark".into()
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
            let mut merged = loaded;
            for (k, v) in Self::defaults().providers {
                merged.providers.entry(k).or_insert(v);
            }
            // Migrate legacy sandbox=true → Permissive
            if merged.sandbox && merged.sandbox_mode == SandboxMode::Off {
                merged.sandbox_mode = SandboxMode::Permissive;
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
        providers.insert("openrouter".into(), ProviderConfig {
            api_key: String::new(),
            endpoint: "https://openrouter.ai/api/v1".into(),
            model: "anthropic/claude-sonnet-4-5".into(),
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
            sandbox: false,
            sandbox_mode: SandboxMode::Off,
            skip_permissions: false,
            theme: "dark".into(),
            verify_command: None,
            clean_env: false,
            ast_mode: AstMode::Off,
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
