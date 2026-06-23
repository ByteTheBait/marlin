use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub tool: String,
    pub size: u64,
}

fn snap_dir(marlin_dir: &Path, work_dir: &str, abs_path: &str) -> PathBuf {
    // Hash work_dir + abs_path to a stable subdir name
    let key = format!("{work_dir}::{abs_path}");
    let hash = {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in key.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    };
    marlin_dir.join("snapshots").join(hash)
}

pub fn take(marlin_dir: &Path, work_dir: &str, abs_path: &str, tool: &str) {
    if !Path::new(abs_path).exists() { return; }
    let dir = snap_dir(marlin_dir, work_dir, abs_path);
    let _ = std::fs::create_dir_all(&dir);

    let id = Utc::now().format("%Y%m%d%H%M%S%3f").to_string();
    let data_path = dir.join(format!("{id}.dat"));
    let meta_path = dir.join(format!("{id}.json"));

    let _ = std::fs::copy(abs_path, &data_path);
    let size = std::fs::metadata(abs_path).map(|m| m.len()).unwrap_or(0);
    let snap = Snapshot {
        id: id.clone(),
        timestamp: Utc::now(),
        tool: tool.to_string(),
        size,
    };
    if let Ok(meta) = serde_json::to_string(&snap) {
        let _ = std::fs::write(meta_path, meta);
    }
}

pub fn list(marlin_dir: &Path, work_dir: &str, abs_path: &str) -> Vec<Snapshot> {
    let dir = snap_dir(marlin_dir, work_dir, abs_path);
    let mut snaps: Vec<Snapshot> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|data| serde_json::from_str::<Snapshot>(&data).ok())
        .collect();
    snaps.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    snaps
}

pub fn restore(marlin_dir: &Path, work_dir: &str, abs_path: &str, id: &str) -> Result<()> {
    let dir = snap_dir(marlin_dir, work_dir, abs_path);
    let data_path = dir.join(format!("{id}.dat"));
    std::fs::copy(&data_path, abs_path)?;
    Ok(())
}

pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{bytes}B") }
    else if bytes < 1024 * 1024 { format!("{:.1}KB", bytes as f64 / 1024.0) }
    else { format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0)) }
}
