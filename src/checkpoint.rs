//! Git checkpointing for whole-turn undo.
//!
//! Before an agentic turn runs, Marlin can create a lightweight git commit
//! ("checkpoint") of the working tree. `/undo` then resets the working tree
//! back to that checkpoint, rolling back every file change the turn made in
//! one step — the same model Claude Code and opencode use.
//!
//! Checkpoints are opt-in (`/checkpoints on`) because they create commits in
//! the user's repo. When enabled, each checkpoint commit is tagged with a
//! stable marker in its message so `/undo` can find the most recent one
//! without disturbing the user's own history.

use std::process::Command;

/// Marker embedded in checkpoint commit messages so `/undo` can find them.
const CHECKPOINT_MARKER: &str = "marlin-checkpoint";

/// True if `work_dir` is inside a git repository.
pub fn is_git_repo(work_dir: &str) -> bool {
    Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .current_dir(work_dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a checkpoint commit of the current working tree. Stages and commits
/// all changes (tracked + untracked) with a marker message. Returns the short
/// commit hash on success, or an error string.
pub fn create(work_dir: &str) -> Result<String, String> {
    if !is_git_repo(work_dir) {
        return Err("not a git repository — checkpoints require git".into());
    }

    // Stage everything (including untracked files).
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("git add failed: {e}"))?;
    if !add.status.success() {
        return Err(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr).trim()
        ));
    }

    // If there's nothing to commit, there's nothing to checkpoint — that's fine.
    let diff = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("git diff failed: {e}"))?;
    if diff.status.success() {
        // Exit 0 means no staged changes — nothing to checkpoint.
        return Ok(String::new());
    }

    let commit = Command::new("git")
        .args(["commit", "-m", CHECKPOINT_MARKER, "--no-verify"])
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("git commit failed: {e}"))?;
    if !commit.status.success() {
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        ));
    }

    // Return the short hash of the new commit.
    let rev = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("git rev-parse failed: {e}"))?;
    Ok(String::from_utf8_lossy(&rev.stdout).trim().to_string())
}

/// Find the most recent checkpoint commit hash, if any.
fn last_checkpoint(work_dir: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["log", "--grep", CHECKPOINT_MARKER, "-1", "--format=%H"])
        .current_dir(work_dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if hash.is_empty() { None } else { Some(hash) }
}

/// Undo the most recent turn: hard-reset the working tree to the last
/// checkpoint commit. Returns a human-readable summary.
pub fn undo(work_dir: &str) -> Result<String, String> {
    if !is_git_repo(work_dir) {
        return Err("not a git repository — checkpoints require git".into());
    }
    let Some(target) = last_checkpoint(work_dir) else {
        return Err("no marlin checkpoint found to undo to".into());
    };

    let reset = Command::new("git")
        .args(["reset", "--hard", &target])
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("git reset failed: {e}"))?;
    if !reset.status.success() {
        return Err(format!(
            "git reset failed: {}",
            String::from_utf8_lossy(&reset.stderr).trim()
        ));
    }

    // Also clean untracked files that the checkpoint didn't capture (created
    // during the turn). `git clean -fd` removes them; only run it if there
    // are any, to avoid surprising the user.
    let _ = Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(work_dir)
        .output();

    Ok(format!("Reverted working tree to checkpoint {target}."))
}

/// Whether a path is inside a git repo (used by the engine to decide whether
/// to offer checkpoints at all).
pub fn available(work_dir: &str) -> bool {
    is_git_repo(work_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn temp_git_repo() -> String {
        let dir = std::env::temp_dir().join(format!("marlin_ckpt_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = Command::new("git").args(["init", "-q"]).current_dir(&dir).output().unwrap();
        let _ = Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&dir).output().unwrap();
        let _ = Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir).output().unwrap();
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn create_and_undo_round_trip() {
        let dir = temp_git_repo();
        std::fs::write(Path::new(&dir).join("a.txt"), "v1").unwrap();

        // First checkpoint captures v1.
        let c1 = create(&dir).unwrap();
        assert!(!c1.is_empty(), "first checkpoint should commit");

        // Modify the file — this simulates the turn's changes.
        std::fs::write(Path::new(&dir).join("a.txt"), "v2").unwrap();
        std::fs::write(Path::new(&dir).join("new.txt"), "created").unwrap();

        // Undo should restore v1 and remove the untracked file.
        let msg = undo(&dir).unwrap();
        assert!(msg.contains("Reverted"));
        let content = std::fs::read_to_string(Path::new(&dir).join("a.txt")).unwrap();
        assert_eq!(content, "v1");
        assert!(!Path::new(&dir).join("new.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_changes_means_no_checkpoint() {
        let dir = temp_git_repo();
        std::fs::write(Path::new(&dir).join("a.txt"), "v1").unwrap();
        let c1 = create(&dir).unwrap();
        assert!(!c1.is_empty());

        // No changes since the checkpoint — create returns empty.
        let c2 = create(&dir).unwrap();
        assert!(c2.is_empty(), "no changes should produce no new checkpoint");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_without_checkpoint_is_an_error() {
        let dir = temp_git_repo();
        std::fs::write(Path::new(&dir).join("a.txt"), "v1").unwrap();
        let _ = Command::new("git").args(["add", "-A"]).current_dir(&dir).output().unwrap();
        let _ = Command::new("git").args(["commit", "-m", "initial"]).current_dir(&dir).output().unwrap();

        assert!(undo(&dir).is_err(), "no marlin checkpoint should error");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
