use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Model tier config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTier {
    pub provider: String,
    pub model: String,
    /// Fallback provider to use if the primary is rate-limited.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub backup_provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub backup_model: String,
}

impl ModelTier {
    #[allow(dead_code)]
    pub fn has_backup(&self) -> bool {
        !self.backup_provider.is_empty() && !self.backup_model.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTiers {
    /// Enable automatic difficulty-based tier routing.
    #[serde(default)]
    pub enabled: bool,
    /// Used for tasks scored ≤ default_max_difficulty (easy work).
    pub default: ModelTier,
    /// Used for tasks scored > default_max_difficulty (hard work).
    pub complex: ModelTier,
    /// Lightweight model used only for difficulty scoring (cheap fast calls).
    pub rater: ModelTier,
    /// Difficulty threshold 1–100; tasks at or below this use the default tier.
    #[serde(default = "default_difficulty_threshold")]
    pub default_max_difficulty: u8,
}

fn default_difficulty_threshold() -> u8 {
    40
}

impl Default for ModelTiers {
    fn default() -> Self {
        Self {
            enabled: false,
            default: ModelTier {
                provider: "claude".into(),
                model: "claude-haiku-4-5-20251001".into(),
                backup_provider: String::new(),
                backup_model: String::new(),
            },
            complex: ModelTier {
                provider: "claude".into(),
                model: "claude-sonnet-5".into(),
                backup_provider: String::new(),
                backup_model: String::new(),
            },
            rater: ModelTier {
                provider: "claude".into(),
                model: "claude-haiku-4-5-20251001".into(),
                backup_provider: String::new(),
                backup_model: String::new(),
            },
            default_max_difficulty: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AstMode {
    #[default]
    Off,
    SExpr,
    Harness,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    /// Commands require explicit /allow approval (default).
    #[default]
    Off,
    /// All commands allowed; run directly on the host (old "sandbox: true" behaviour).
    Permissive,
    /// Commands run via Microsoft eXecution Containers (MXC) — no outbound network,
    /// only the workdir mounted read-write.
    Mxc,
    /// Commands run inside a Docker container — the workdir is mounted read-write
    /// at its host path and the container has no network access.
    Docker,
}

impl SandboxMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Permissive => "permissive",
            Self::Mxc => "mxc",
            Self::Docker => "docker",
        }
    }

    /// True if all commands are implicitly allowed (no per-command /allow needed).
    pub fn allows_all(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Custom model designations previously used with this provider (most
    /// recent first), so switching away and back doesn't require re-typing
    /// e.g. "qwen/qwen3.5-35b" on groq. Capped at 12 entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_models: Vec<String>,
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
    /// Multi-model tier routing (default/complex/rater with per-tier backups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_tiers: Option<ModelTiers>,
    /// Delegate skill invocations to a nested subagent loop instead of running
    /// them inline. On by default; toggle with `/subagents [on|off]`. When off,
    /// skills run the old direct way (shell chunks executed as-is, prompt
    /// bodies expanded and handed straight back to the main model).
    #[serde(default = "default_skill_subagents")]
    pub skill_subagents: bool,
    /// Approximate context-window budget the sidebar meter and compaction
    /// trigger against — distinct from `max_tokens` (the per-response output
    /// cap). Set with `/budget <n>`.
    #[serde(default = "default_token_budget")]
    pub token_budget: usize,
    /// Hard cap on tool-call round-trips within one agentic turn. Set in the
    /// /config menu (or directly in config.json). Defaults to 100.
    #[serde(default = "default_tool_call_limit")]
    pub tool_call_limit: usize,
    /// Per-directory status bar background colors, keyed by work_dir. Lets you
    /// color-code different sessions (one per working directory) so you can
    /// tell them apart at a glance. Set with `/color <#rrggbb>`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub status_colors: HashMap<String, [u8; 3]>,
    /// Per-provider system prompt overrides. When set for the active provider,
    /// this replaces the global `system_prompt` for that provider only.
    /// Set with `/system <provider> <text>` or `/system <text>` for global.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_system_prompts: HashMap<String, String>,
    /// Request extended thinking / chain-of-thought reasoning from the model
    /// (Claude extended thinking, OpenAI reasoning models). Toggle with
    /// `/thinking [on|off]`. Off by default.
    #[serde(default)]
    pub thinking: bool,
    /// Create a git checkpoint before each agentic turn so `/undo` can roll
    /// the whole turn back. Toggle with `/checkpoints [on|off]`. Off by
    /// default (it creates commits in the user's repo).
    #[serde(default)]
    pub checkpoints: bool,
}

fn default_tool_call_limit() -> usize {
    100
}

fn default_skill_subagents() -> bool {
    true
}

fn default_max_tokens() -> usize {
    4096
}

fn default_token_budget() -> usize {
    100_000
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
            match serde_json::from_str::<Self>(&data) {
                Ok(loaded) => {
                    let mut merged = loaded;
                    for (k, v) in Self::defaults().providers {
                        merged.providers.entry(k).or_insert(v);
                    }
                    // Migrate legacy sandbox=true → Permissive
                    if merged.sandbox && merged.sandbox_mode == SandboxMode::Off {
                        merged.sandbox_mode = SandboxMode::Permissive;
                    }
                    // config.json is a single global file shared across every
                    // directory marlin is launched from. work_dir must reflect
                    // where *this* process actually started, not wherever the
                    // last session happened to /cd to — otherwise launching
                    // from a different project silently inherits the previous
                    // project's directory.
                    merged.work_dir = cfg.work_dir;
                    cfg = merged;
                }
                Err(e) => {
                    // A corrupt config must not brick startup: preserve the bad
                    // file for manual recovery (API keys live in it) and fall
                    // back to defaults.
                    let backup = path.with_extension("json.corrupt");
                    std::fs::rename(&path, &backup)?;
                    eprintln!(
                        "marlin: {} is corrupt ({e});\n  moved it to {} and starting with default settings",
                        path.display(),
                        backup.display()
                    );
                    cfg.save()?;
                }
            }
        } else {
            cfg.save()?;
        }
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let dir = marlin_dir()?;
        let data = serde_json::to_string_pretty(self)?;
        // Write-then-rename so a crash mid-write can't truncate config.json.
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, dir.join("config.json"))?;
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

    /// Record a model designation as used with `provider` so it resurfaces in
    /// the /config menu's model list after switching away and back.
    pub fn remember_model(&mut self, provider: &str, model: &str) {
        if model.is_empty() {
            return;
        }
        let entry = self.providers.entry(provider.to_string()).or_default();
        entry.extra_models.retain(|m| m != model);
        entry.extra_models.insert(0, model.to_string());
        entry.extra_models.truncate(12);
    }

    pub fn defaults() -> Self {
        let wd = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut providers = HashMap::new();
        providers.insert(
            "claude".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: String::new(),
                model: "claude-sonnet-5".into(),
                extra_models: vec![],
            },
        );
        providers.insert(
            "ollama".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: "http://localhost:11434".into(),
                model: "llama3".into(),
                extra_models: vec![],
            },
        );
        providers.insert(
            "fireworks".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: "https://api.fireworks.ai/inference/v1".into(),
                model: "accounts/fireworks/models/llama-v3-70b-instruct".into(),
                extra_models: vec![],
            },
        );
        providers.insert(
            "moonshot".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: "https://api.moonshot.cn/v1".into(),
                model: "moonshot-v1-8k".into(),
                extra_models: vec![],
            },
        );
        providers.insert(
            "groq".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: "https://api.groq.com/openai/v1".into(),
                model: "llama-3.3-70b-versatile".into(),
                extra_models: vec![],
            },
        );
        providers.insert(
            "openrouter".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: "https://openrouter.ai/api/v1".into(),
                model: "anthropic/claude-sonnet-5".into(),
                extra_models: vec![],
            },
        );
        providers.insert(
            "custom".into(),
            ProviderConfig {
                api_key: String::new(),
                endpoint: "http://localhost:8080/v1".into(),
                model: "default".into(),
                extra_models: vec![],
            },
        );

        Self {
            active_provider: "claude".into(),
            active_model: "claude-sonnet-5".into(),
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
            model_tiers: None,
            skill_subagents: true,
            token_budget: default_token_budget(),
            tool_call_limit: default_tool_call_limit(),
            status_colors: HashMap::new(),
            provider_system_prompts: HashMap::new(),
            thinking: false,
            checkpoints: false,
        }
    }
}

/// Parse a `#rrggbb` (or `rrggbb`) hex color string into an `[r, g, b]` triple.
/// Returns `None` for anything malformed.
pub fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

// ── Theme palette (optional ~/.marlin/theme.toml) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThemeColors {
    /// App background
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<[u8; 3]>,
    /// Your message text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<[u8; 3]>,
    /// Marlin label / primary accent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<[u8; 3]>,
    /// System messages and timestamps
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<[u8; 3]>,
    /// Success indicators (✓)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<[u8; 3]>,
    /// Error indicators (✗)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<[u8; 3]>,
    /// Tool call name highlight
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amber: Option<[u8; 3]>,
    /// Command keys and badge backgrounds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cobalt: Option<[u8; 3]>,
    /// Secondary text and argument labels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steel: Option<[u8; 3]>,
    /// "Scroll to bottom" indicator that appears when new content arrives
    /// while the user is scrolled up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_hint: Option<[u8; 3]>,
    /// Left-border accent on the live streaming buffer (distinct from committed
    /// entries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_highlight: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThemePalette {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<ThemeColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<ThemeColors>,
}

pub fn load_theme(marlin_dir: &Path) -> ThemePalette {
    let path = marlin_dir.join("theme.toml");
    if !path.exists() {
        return ThemePalette::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str::<ThemePalette>(&s).ok())
        .unwrap_or_default()
}

// ── Named themes (~/.marlin/themes/<name>.toml) ───────────────────────────────

/// A named theme file — same color fields as ThemePalette, plus name/description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<ThemeColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<ThemeColors>,
}

pub fn themes_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("themes");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Load a named theme from `~/.marlin/themes/<name>.toml`, returning its palette.
pub fn load_named_theme(marlin_dir: &Path, name: &str) -> Option<ThemePalette> {
    let path = themes_dir(marlin_dir).join(format!("{name}.toml"));
    if !path.exists() {
        return None;
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str::<ThemeFile>(&s).ok())
        .map(|tf| ThemePalette {
            dark: tf.dark,
            light: tf.light,
        })
}

/// List all named themes: (name, description).
pub fn list_themes(marlin_dir: &Path) -> Vec<(String, String)> {
    let dir = themes_dir(marlin_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut themes: Vec<(String, String)> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("toml"))
        .filter_map(|e| {
            let data = std::fs::read_to_string(e.path()).ok()?;
            let tf: ThemeFile = toml::from_str(&data).ok()?;
            Some((tf.name, tf.description))
        })
        .collect();
    themes.sort_by(|a, b| a.0.cmp(&b.0));
    themes
}

/// Install one working example named theme on first run — same write-if-missing
/// pattern as `skills::install_defaults`. Never overwrites a user's file.
pub fn install_default_themes(marlin_dir: &Path) {
    let dir = themes_dir(marlin_dir);
    for theme in default_themes() {
        let path = dir.join(format!("{}.toml", theme.name));
        if !path.exists() {
            if let Ok(data) = toml::to_string_pretty(&theme) {
                let _ = std::fs::write(path, data);
            }
        }
    }
}

fn default_themes() -> Vec<ThemeFile> {
    vec![
        ThemeFile {
            name: "nord".into(),
            description: "Nord color scheme".into(),
            dark: Some(ThemeColors {
                bg: Some([46, 52, 64]),
                assistant: Some([136, 192, 208]),
                cobalt: Some([94, 129, 172]),
                steel: Some([76, 86, 106]),
                user: Some([229, 233, 240]),
                ..Default::default()
            }),
            light: None,
        },
        ThemeFile {
            name: "dracula".into(),
            description: "Dracula dark theme".into(),
            dark: Some(ThemeColors {
                bg: Some([40, 42, 54]),
                assistant: Some([255, 121, 198]),
                cobalt: Some([139, 233, 253]),
                steel: Some([98, 114, 164]),
                system: Some([68, 71, 90]),
                user: Some([248, 248, 242]),
                success: Some([80, 250, 123]),
                error: Some([255, 85, 85]),
                amber: Some([255, 184, 108]),
                scroll_hint: Some([255, 184, 108]),
                stream_highlight: Some([139, 233, 253]),
                ..Default::default()
            }),
            light: None,
        },
        ThemeFile {
            name: "gruvbox".into(),
            description: "Gruvbox dark — warm retro".into(),
            dark: Some(ThemeColors {
                bg: Some([40, 40, 40]),
                assistant: Some([184, 187, 38]),
                cobalt: Some([131, 165, 152]),
                steel: Some([146, 131, 116]),
                system: Some([124, 111, 100]),
                user: Some([235, 219, 178]),
                success: Some([152, 151, 26]),
                error: Some([251, 73, 52]),
                amber: Some([215, 153, 33]),
                scroll_hint: Some([215, 153, 33]),
                stream_highlight: Some([184, 187, 38]),
                ..Default::default()
            }),
            light: Some(ThemeColors {
                bg: Some([251, 241, 199]),
                assistant: Some([102, 56, 84]),
                cobalt: Some([23, 131, 150]),
                steel: Some([69, 133, 136]),
                system: Some([124, 111, 100]),
                user: Some([40, 40, 40]),
                success: Some([102, 56, 84]),
                error: Some([204, 36, 29]),
                amber: Some([177, 98, 134]),
                ..Default::default()
            }),
        },
        ThemeFile {
            name: "one-dark".into(),
            description: "Atom One Dark".into(),
            dark: Some(ThemeColors {
                bg: Some([40, 44, 52]),
                assistant: Some([86, 182, 194]),
                cobalt: Some([97, 175, 239]),
                steel: Some([92, 99, 112]),
                system: Some([92, 99, 112]),
                user: Some([220, 223, 228]),
                success: Some([152, 195, 121]),
                error: Some([224, 108, 117]),
                amber: Some([229, 192, 123]),
                scroll_hint: Some([229, 192, 123]),
                stream_highlight: Some([86, 182, 194]),
                ..Default::default()
            }),
            light: None,
        },
    ]
}

// ── Layout config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// Width of the sidebar in terminal columns (default 34).
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u16,
    /// Minimum total terminal width for the sidebar to appear (default 100).
    #[serde(default = "default_min_sidebar_width")]
    pub min_sidebar_width: u16,
}

fn default_sidebar_width() -> u16 {
    34
}
fn default_min_sidebar_width() -> u16 {
    100
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            sidebar_width: default_sidebar_width(),
            min_sidebar_width: default_min_sidebar_width(),
        }
    }
}

/// Load `~/.marlin/layout.toml`, falling back to defaults if absent or invalid.
pub fn load_layout(marlin_dir: &Path) -> LayoutConfig {
    let path = marlin_dir.join("layout.toml");
    match std::fs::read_to_string(&path) {
        Err(_) => LayoutConfig::default(),
        Ok(data) => toml::from_str::<LayoutConfig>(&data).unwrap_or_else(|e| {
            eprintln!("layout.toml parse error: {e}");
            LayoutConfig::default()
        }),
    }
}

/// Per-project configuration loaded from `.marlonrc.toml` in the project root.
/// Mirrors a subset of `Config` fields that make sense to override per-project.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_mode: Option<String>,
}

/// Load `.marlonrc.toml` from `work_dir` if it exists. Returns None if the file
/// doesn't exist or can't be parsed.
pub fn load_project_config(work_dir: &str) -> Option<ProjectConfig> {
    let path = Path::new(work_dir).join(".marlonrc.toml");
    if !path.exists() {
        return None;
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str::<ProjectConfig>(&s).ok())
}
