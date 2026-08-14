use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Info {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub run: HashMap<String, String>,
    #[serde(default)]
    pub notes: String,
    #[serde(skip)]
    pub path: PathBuf,
}

impl Info {
    pub fn find(work_dir: &str) -> Result<Self> {
        let p = Path::new(work_dir).join("info.json");
        if !p.exists() {
            return Err(anyhow!("info.json not found in {work_dir}"));
        }
        let data = std::fs::read_to_string(&p)?;
        let mut info: Self = serde_json::from_str(&data)?;
        info.path = p;
        Ok(info)
    }

    pub fn new(work_dir: &str, name: &str, description: &str, run_cmd: &str) -> Self {
        let mut run = HashMap::new();
        if !run_cmd.is_empty() {
            run.insert("run".to_string(), run_cmd.to_string());
        }
        Self {
            name: name.to_string(),
            description: description.to_string(),
            cwd: work_dir.to_string(),
            run,
            notes: String::new(),
            path: Path::new(work_dir).join("info.json"),
        }
    }

    pub fn save(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(&self.path, data)?;
        Ok(())
    }

    pub fn merge_json(&mut self, json: &str) -> Result<()> {
        let patch: serde_json::Value = serde_json::from_str(json)?;
        let mut current = serde_json::to_value(&*self)?;
        if let (Some(obj), Some(p)) = (current.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        *self = serde_json::from_value(current)?;
        Ok(())
    }

    pub fn for_system_prompt(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn set_cwd(&mut self, cwd: &str) {
        self.cwd = cwd.to_string();
        let _ = self.save();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn execute(&self, cmd_name: &str) -> Result<String> {
        let cmd = self
            .run
            .get(cmd_name)
            .ok_or_else(|| anyhow!("no command {:?} in info.json run section", cmd_name))?;
        let work_dir = if self.cwd.is_empty() {
            self.path
                .parent()
                .unwrap_or(Path::new("."))
                .to_string_lossy()
                .to_string()
        } else {
            self.cwd.clone()
        };
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&work_dir)
            .output()?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .trim()
        .to_string())
    }
}
