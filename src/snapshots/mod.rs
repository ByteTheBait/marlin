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
    if !Path::new(abs_path).exists() {
        return;
    }
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

/// Read a snapshot's captured content back out (for /diff-mode).
pub fn read(marlin_dir: &Path, work_dir: &str, abs_path: &str, id: &str) -> Result<String> {
    let dir = snap_dir(marlin_dir, work_dir, abs_path);
    let data_path = dir.join(format!("{id}.dat"));
    Ok(std::fs::read_to_string(&data_path)?)
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

/// Old_lines × new_lines cap on the LCS table below (~16MB of u32 at the
/// limit) — large enough for any real source file, small enough to stay
/// fast and bounded for a one-shot interactive command.
const MAX_DIFF_CELLS: usize = 4_000_000;

/// Line-level diff via classic LCS dynamic programming, backtracked into
/// context/added/removed ops (same idea as `diff`'s output). Returns `None`
/// instead of building the O(n·m) table when both files together exceed
/// `MAX_DIFF_CELLS` — the caller should report that as "too large to diff"
/// rather than eating the memory/time.
pub fn diff_lines(old: &str, new: &str) -> Option<Vec<DiffLine>> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let n = old_lines.len();
    let m = new_lines.len();
    if n.saturating_mul(m) > MAX_DIFF_CELLS {
        return None;
    }

    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old_lines[i] == new_lines[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old_lines[i] == new_lines[j] {
            ops.push(DiffLine::Context(old_lines[i].to_string()));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(DiffLine::Removed(old_lines[i].to_string()));
            i += 1;
        } else {
            ops.push(DiffLine::Added(new_lines[j].to_string()));
            j += 1;
        }
    }
    while i < n {
        ops.push(DiffLine::Removed(old_lines[i].to_string()));
        i += 1;
    }
    while j < m {
        ops.push(DiffLine::Added(new_lines[j].to_string()));
        j += 1;
    }
    Some(ops)
}

pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    #[test]
    fn identical_files_are_all_context() {
        let ops = diff_lines("a\nb\nc", "a\nb\nc").unwrap();
        assert_eq!(
            ops,
            vec![
                DiffLine::Context("a".into()),
                DiffLine::Context("b".into()),
                DiffLine::Context("c".into()),
            ]
        );
    }

    #[test]
    fn pure_addition() {
        let ops = diff_lines("a\nb", "a\nb\nc").unwrap();
        assert_eq!(
            ops,
            vec![
                DiffLine::Context("a".into()),
                DiffLine::Context("b".into()),
                DiffLine::Added("c".into()),
            ]
        );
    }

    #[test]
    fn pure_removal() {
        let ops = diff_lines("a\nb\nc", "a\nc").unwrap();
        assert_eq!(
            ops,
            vec![
                DiffLine::Context("a".into()),
                DiffLine::Removed("b".into()),
                DiffLine::Context("c".into()),
            ]
        );
    }

    #[test]
    fn line_replaced_in_the_middle() {
        let ops = diff_lines("a\nb\nc", "a\nx\nc").unwrap();
        assert_eq!(
            ops,
            vec![
                DiffLine::Context("a".into()),
                DiffLine::Removed("b".into()),
                DiffLine::Added("x".into()),
                DiffLine::Context("c".into()),
            ]
        );
    }

    #[test]
    fn empty_old_file_is_all_additions() {
        let ops = diff_lines("", "a\nb").unwrap();
        assert_eq!(
            ops,
            vec![DiffLine::Added("a".into()), DiffLine::Added("b".into())]
        );
    }

    #[test]
    fn oversized_pair_returns_none() {
        let big = "x\n".repeat(3000);
        assert!(diff_lines(&big, &big).is_none());
    }
}
