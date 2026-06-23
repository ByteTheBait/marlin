use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub tf: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub files: Vec<FileEntry>,
    pub df: HashMap<String, usize>,       // document frequency per term
    pub file_count: usize,
    pub term_count: usize,
    pub built_at: DateTime<Utc>,
    #[serde(skip)]
    pub work_dir: String,
}

#[derive(Debug)]
pub struct BuildStats {
    pub files: usize,
    pub terms: usize,
    pub elapsed: Duration,
}

pub struct SearchResult {
    pub path: String,
    pub score: f64,
    pub snippet: String,
}

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", ".next", "vendor", "__pycache__"];
const SKIP_EXTS: &[&str] = &["exe", "dll", "so", "dylib", "png", "jpg", "jpeg", "gif", "webp",
    "ico", "pdf", "zip", "tar", "gz", "wasm", "bin", "lock"];

pub fn build(work_dir: &str, _ignored: Option<()>) -> Result<(Index, BuildStats)> {
    let start = Instant::now();
    let mut files_vec: Vec<FileEntry> = Vec::new();
    let mut df: HashMap<String, usize> = HashMap::new();

    walk_dir(Path::new(work_dir), work_dir, &mut files_vec, &mut df)?;

    let file_count = files_vec.len();
    let term_count = df.len();

    let idx = Index {
        files: files_vec,
        df,
        file_count,
        term_count,
        built_at: Utc::now(),
        work_dir: work_dir.to_string(),
    };

    Ok((idx, BuildStats {
        files: file_count,
        terms: term_count,
        elapsed: start.elapsed(),
    }))
}

fn walk_dir(dir: &Path, work_dir: &str, files: &mut Vec<FileEntry>, df: &mut HashMap<String, usize>) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if path.is_dir() {
            if !SKIP_DIRS.contains(&name) {
                let _ = walk_dir(&path, work_dir, files, df);
            }
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if SKIP_EXTS.contains(&ext.as_str()) { continue; }

        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let rel_path = path.strip_prefix(work_dir).unwrap_or(&path).to_string_lossy().to_string();

        let terms = tokenize(&text);
        let mut tf: HashMap<String, f64> = HashMap::new();
        let total = terms.len() as f64;
        if total == 0.0 { continue; }

        for term in &terms {
            *tf.entry(term.clone()).or_insert(0.0) += 1.0;
        }
        for v in tf.values_mut() {
            *v /= total;
        }

        for term in tf.keys() {
            *df.entry(term.clone()).or_insert(0) += 1;
        }

        files.push(FileEntry { path: rel_path, tf });
    }
    Ok(())
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2 && t.len() <= 40)
        .map(|t| t.to_lowercase())
        .collect()
}

pub fn search(idx: &Index, query: &str, limit: usize) -> Vec<SearchResult> {
    let n = idx.file_count as f64;
    let query_terms: Vec<String> = tokenize(query);
    if query_terms.is_empty() { return vec![]; }

    let mut scores: Vec<(usize, f64)> = idx.files.iter().enumerate().filter_map(|(i, f)| {
        let score: f64 = query_terms.iter().map(|term| {
            let tf = f.tf.get(term).copied().unwrap_or(0.0);
            let df = idx.df.get(term).copied().unwrap_or(1) as f64;
            let idf = (n / df + 1.0).ln();
            tf * idf
        }).sum();
        if score > 0.0 { Some((i, score)) } else { None }
    }).collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(limit);

    scores.into_iter().map(|(i, score)| {
        let f = &idx.files[i];
        let full_path = format!("{}/{}", idx.work_dir, f.path);
        let snippet = extract_snippet(&full_path, &query_terms);
        SearchResult { path: f.path.clone(), score, snippet }
    }).collect()
}

fn extract_snippet(path: &str, terms: &[String]) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return String::new() };
    for line in text.lines() {
        let lower = line.to_lowercase();
        if terms.iter().any(|t| lower.contains(t.as_str())) {
            let trimmed = line.trim();
            if trimmed.len() > 120 {
                return format!("{}…", &trimmed[..120]);
            }
            return trimmed.to_string();
        }
    }
    String::new()
}

pub fn format_results(results: &[SearchResult], query: &str) -> String {
    if results.is_empty() {
        return format!("No results for {:?}", query);
    }
    let mut out = format!("Search results for {:?}:\n", query);
    for r in results {
        out.push_str(&format!("  [{:.3}] {}\n", r.score, r.path));
        if !r.snippet.is_empty() {
            out.push_str(&format!("          {}\n", r.snippet));
        }
    }
    out
}

pub fn update_file(idx: &mut Index, abs_path: &str) {
    let Ok(text) = std::fs::read_to_string(abs_path) else { return };
    let rel_path = abs_path.strip_prefix(&idx.work_dir)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| abs_path.to_string());

    // Remove old entry
    if let Some(pos) = idx.files.iter().position(|f| f.path == rel_path) {
        let old = &idx.files[pos];
        for term in old.tf.keys() {
            if let Some(v) = idx.df.get_mut(term) {
                *v = v.saturating_sub(1);
            }
        }
        idx.files.remove(pos);
    }

    // Re-index
    let terms = tokenize(&text);
    let mut tf: HashMap<String, f64> = HashMap::new();
    let total = terms.len() as f64;
    if total == 0.0 { return; }
    for term in &terms { *tf.entry(term.clone()).or_insert(0.0) += 1.0; }
    for v in tf.values_mut() { *v /= total; }
    for term in tf.keys() { *idx.df.entry(term.clone()).or_insert(0) += 1; }

    idx.files.push(FileEntry { path: rel_path, tf });
    idx.file_count = idx.files.len();
    idx.term_count = idx.df.len();
}

fn index_path(marlin_dir: &Path, work_dir: &str) -> PathBuf {
    let hash = {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in work_dir.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
        format!("{h:016x}")
    };
    let dir = marlin_dir.join("index");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{hash}.json"))
}

pub fn save(marlin_dir: &Path, idx: &Index) {
    if let Ok(data) = serde_json::to_string(idx) {
        let _ = std::fs::write(index_path(marlin_dir, &idx.work_dir), data);
    }
}

pub fn load(marlin_dir: &Path, work_dir: &str) -> Result<Index> {
    let path = index_path(marlin_dir, work_dir);
    let data = std::fs::read_to_string(path)?;
    let mut idx: Index = serde_json::from_str(&data)?;
    idx.work_dir = work_dir.to_string();
    Ok(idx)
}
