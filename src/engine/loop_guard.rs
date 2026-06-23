use std::collections::VecDeque;

const HISTORY_SIZE: usize = 5;
const THRESHOLD: usize = 3;

#[derive(Clone)]
struct Entry {
    cmd: String,
    failed: bool,
}

pub struct LoopGuard {
    history: VecDeque<Entry>,
}

impl LoopGuard {
    pub fn new() -> Self {
        Self { history: VecDeque::with_capacity(HISTORY_SIZE) }
    }

    /// Returns Some(intercept_message) if the engine is stuck in a loop.
    pub fn check(&mut self, cmd: &str, failed: bool) -> Option<String> {
        let entry = Entry { cmd: cmd.to_string(), failed };

        if self.history.iter().filter(|e| e.cmd == cmd && e.failed).count() >= THRESHOLD {
            return Some(format!(
                "System Intercept: You are stuck in a repeating loop on command {:?}. \
                Change your approach — inspect the error, check the file directly, or try a different method.",
                cmd
            ));
        }

        self.history.push_back(entry);
        if self.history.len() > HISTORY_SIZE {
            self.history.pop_front();
        }
        None
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }
}
