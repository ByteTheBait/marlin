use std::collections::{HashMap, VecDeque};

use sha2::{Digest, Sha256};

const HISTORY_SIZE: usize = 8;
const REPEAT_THRESHOLD: usize = 3;

#[derive(Clone)]
struct Entry {
    cmd: String,
    failed: bool,
}

pub struct LoopGuard {
    history: VecDeque<Entry>,
    // path -> (hash_after_last_edit, consecutive_no-change_count)
    edit_hashes: HashMap<String, ([u8; 32], usize)>,
}

impl LoopGuard {
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(HISTORY_SIZE),
            edit_hashes: HashMap::new(),
        }
    }

    /// Call after any tool call. Returns a system intercept message if stuck.
    pub fn check(&mut self, tool: &str, failed: bool) -> Option<String> {
        let entry = Entry {
            cmd: tool.to_string(),
            failed,
        };

        let repeat_fails = self
            .history
            .iter()
            .filter(|e| e.cmd == tool && e.failed)
            .count();

        if repeat_fails >= REPEAT_THRESHOLD {
            return Some(format!(
                "System Intercept: You are stuck in a repeating loop on tool {:?} (failed {} times). \
                Change your approach — inspect the error, check the file directly, or try a different method.",
                tool, repeat_fails
            ));
        }

        self.history.push_back(entry);
        if self.history.len() > HISTORY_SIZE {
            self.history.pop_front();
        }
        None
    }

    /// Call after edit_file or write_file. Returns intercept if the file didn't actually change.
    pub fn check_file_edit(
        &mut self,
        path: &str,
        new_content: &[u8],
        failed: bool,
    ) -> Option<String> {
        let mut hasher = Sha256::new();
        hasher.update(new_content);
        let hash: [u8; 32] = hasher.finalize().into();

        if let Some((prev_hash, count)) = self.edit_hashes.get_mut(path) {
            if *prev_hash == hash {
                *count += 1;
                if *count >= 2 {
                    return Some(format!(
                        "System Warning: You have edited {:?} {} times without changing its content. \
                        The file hash is identical before and after your edit. \
                        Do not retry the same edit — use read_file to re-examine the full context, \
                        check surrounding dependencies, or try a different approach.",
                        path, *count
                    ));
                }
            } else {
                *prev_hash = hash;
                *count = if failed { 1 } else { 0 };
            }
        } else {
            self.edit_hashes
                .insert(path.to_string(), (hash, if failed { 1 } else { 0 }));
        }

        self.check("edit_file", failed)
    }

    pub fn reset(&mut self) {
        self.history.clear();
        self.edit_hashes.clear();
    }
}
