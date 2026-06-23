use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::config::Config;
use super::{Provider, claude::ClaudeProvider, openai_compat::OpenAiCompatProvider};

pub struct Registry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl Registry {
    pub fn new(cfg: &Config) -> Self {
        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();

        let claude_cfg = cfg.providers.get("claude").cloned().unwrap_or_default();
        providers.insert("claude".into(), Arc::new(ClaudeProvider::new(claude_cfg.api_key)));

        let ollama_cfg = cfg.providers.get("ollama").cloned().unwrap_or_default();
        let ollama_model = if ollama_cfg.model.is_empty() { "llama3" } else { &ollama_cfg.model };
        providers.insert("ollama".into(), Arc::new(
            OpenAiCompatProvider::new_ollama(&ollama_cfg.endpoint, ollama_model)
        ));

        let fw_cfg = cfg.providers.get("fireworks").cloned().unwrap_or_default();
        providers.insert("fireworks".into(), Arc::new(
            OpenAiCompatProvider::new_fireworks(&fw_cfg.api_key, &fw_cfg.endpoint)
        ));

        let groq_cfg = cfg.providers.get("groq").cloned().unwrap_or_default();
        providers.insert("groq".into(), Arc::new(
            OpenAiCompatProvider::new_groq(&groq_cfg.api_key, &groq_cfg.endpoint)
        ));

        let ms_cfg = cfg.providers.get("moonshot").cloned().unwrap_or_default();
        providers.insert("moonshot".into(), Arc::new(
            OpenAiCompatProvider::new_moonshot(&ms_cfg.api_key, &ms_cfg.endpoint)
        ));

        let custom_cfg = cfg.providers.get("custom").cloned().unwrap_or_default();
        providers.insert("custom".into(), Arc::new(
            OpenAiCompatProvider::new_custom(&custom_cfg.api_key, &custom_cfg.endpoint)
        ));

        Self { providers }
    }

    pub fn get(&self, name: &str) -> Result<Arc<dyn Provider>> {
        self.providers
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("unknown provider: {name}"))
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }
}
