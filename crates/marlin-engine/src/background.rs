//! Background / long-running process management.
//!
//! `run_command` is synchronous and blocking — it waits (up to `timeout`)
//! for the child to exit, which makes it useless for starting a dev server, a
//! watch build, or a long test and continuing to work while it runs. This
//! module adds a persistent process registry so the model can:
//!
//!   - `bg_start <cmd>`  — launch a process in the background, return an id,
//!   - `bg_log <id>`     — read the output accumulated since the last poll,
//!   - `bg_status <id>`  — is it still running? what was its exit code?
//!   - `bg_kill <id>`    — terminate it (SIGTERM then SIGKILL).
//!
//! Background processes survive across turns, so a server started early in the
//! session is still running and pollable much later.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::SystemTime;

use uuid::Uuid;

/// One live background process plus the output accumulated so far.
pub struct BackgroundProcess {
    pub id: String,
    pub cmd: String,
    pub started_at: SystemTime,
    child: Child,
    stdout_buf: Vec<u8>,
    stderr_buf: Vec<u8>,
    /// Offset into each buffer of what we've already reported via bg_log, so
    /// polling only returns *new* output.
    stdout_consumed: usize,
    stderr_consumed: usize,
    /// Set once the process has been reaped (Some(exit_code)).
    exit_code: Option<Option<i32>>,
}

/// The shared registry held by the engine. `Mutex` + std threads because the
/// process reaping happens on blocking threads, and the model may query it from
/// any tool-call thread.
#[derive(Default)]
pub struct BackgroundRegistry {
    processes: Mutex<HashMap<String, BackgroundProcess>>,
}

/// Snapshot of a process's state suitable for returning to the model.
pub struct BgStatus {
    pub id: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub elapsed_secs: u64,
    pub stdout_len: usize,
    pub stderr_len: usize,
    pub cmd: String,
}

impl BackgroundRegistry {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }

    /// Launch `cmd` via `sh -c` in `work_dir`. Returns the new process id.
    pub fn start(&self, cmd: &str, work_dir: &str) -> Result<String, String> {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(cmd)
            .current_dir(work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        // Detach into a new session/process group so that killing the shell
        // doesn't necessarily take down the whole tree (best-effort on unix).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: `pre_exec` only calls async-signal-safe setpgid(0,0).
            unsafe {
                command.pre_exec(|| {
                    // setpgid(0, 0) moves this child into its own process group.
                    extern "C" {
                        fn setpgid(pid: i32, pgid: i32) -> i32;
                    }
                    setpgid(0, 0);
                    Ok(())
                });
            }
        }

        let child = command
            .spawn()
            .map_err(|e| format!("failed to start: {e}"))?;

        // Read any immediate pipe data non-blocking-ish so the child can't fill
        // its pipe and block. We drain in poll(), but we need the handles.
        let id = Uuid::new_v4().simple().to_string()[..8].to_string();

        let proc = BackgroundProcess {
            id: id.clone(),
            cmd: cmd.to_string(),
            started_at: SystemTime::now(),
            child,
            stdout_buf: Vec::new(),
            stderr_buf: Vec::new(),
            stdout_consumed: 0,
            stderr_consumed: 0,
            exit_code: None,
        };

        let mut guard = self.processes.lock().unwrap();
        guard.insert(id.clone(), proc);
        Ok(id)
    }

    /// Reap finished processes (fills in exit_code) and drain any new output
    /// from live ones so status/log are current. Call before reporting.
    fn poll(&self) {
        let mut guard = self.processes.lock().unwrap();
        let mut finished: Vec<String> = Vec::new();
        for proc in guard.values_mut() {
            // Reap a finished child if it has exited.
            match proc.child.try_wait() {
                Ok(Some(status)) => proc.exit_code = Some(status.code()),
                Ok(None) => {}
                Err(_) => proc.exit_code = Some(None),
            }
            // Drain stdout.
            if let Some(ref mut out) = proc.child.stdout {
                let mut buf = [0u8; 8192];
                loop {
                    match out.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => proc.stdout_buf.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
            }
            // Drain stderr.
            if let Some(ref mut err) = proc.child.stderr {
                let mut buf = [0u8; 8192];
                loop {
                    match err.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => proc.stderr_buf.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
            }
            if proc.exit_code.is_some() {
                let _ = proc.child.wait();
                finished.push(proc.id.clone());
            }
        }
        // Prune entries whose exit we've already reported and that are older
        // than 10 minutes, to avoid unbounded growth.
        let now = SystemTime::now();
        guard.retain(|id, p| {
            p.exit_code.is_none()
                || now
                    .duration_since(p.started_at)
                    .map(|d| d.as_secs() < 600)
                    .unwrap_or(true)
                || finished.contains(id)
        });
    }

    /// Number of currently-running background processes.
    pub fn running_count(&self) -> usize {
        self.poll();
        self.processes
            .lock()
            .unwrap()
            .values()
            .filter(|p| p.exit_code.is_none())
            .count()
    }

    /// Status for one process (or all, if id is empty).
    pub fn status(&self, id: &str) -> Vec<BgStatus> {
        self.poll();
        let guard = self.processes.lock().unwrap();
        guard
            .iter()
            .filter(|(k, _)| id.is_empty() || *k == id)
            .map(|(_, p)| BgStatus {
                id: p.id.clone(),
                running: p.exit_code.is_none(),
                exit_code: p.exit_code.flatten(),
                elapsed_secs: SystemTime::now()
                    .duration_since(p.started_at)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                stdout_len: p.stdout_buf.len(),
                stderr_len: p.stderr_buf.len(),
                cmd: p.cmd.clone(),
            })
            .collect()
    }

    /// New output since the last log read for `id`. Returns (stdout, stderr).
    pub fn log(&self, id: &str) -> Result<(String, String), String> {
        self.poll();
        let mut guard = self.processes.lock().unwrap();
        let proc = guard
            .get_mut(id)
            .ok_or_else(|| format!("no background process with id '{id}'"))?;
        let new_stdout =
            String::from_utf8_lossy(&proc.stdout_buf[proc.stdout_consumed..]).into_owned();
        let new_stderr =
            String::from_utf8_lossy(&proc.stderr_buf[proc.stderr_consumed..]).into_owned();
        proc.stdout_consumed = proc.stdout_buf.len();
        proc.stderr_consumed = proc.stderr_buf.len();
        Ok((new_stdout, new_stderr))
    }

    /// Terminate `id` with SIGTERM then SIGKILL on unix (best-effort).
    pub fn kill(&self, id: &str) -> Result<String, String> {
        self.poll();
        let mut guard = self.processes.lock().unwrap();
        let proc = guard
            .get_mut(id)
            .ok_or_else(|| format!("no background process with id '{id}'"))?;
        if proc.exit_code.is_some() {
            return Ok(format!("process {id} already exited"));
        }
        let pid = proc.child.id();
        let _ = kill_by_pid(pid);
        // Give it a moment to exit, then force-kill.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = proc.child.try_wait();
        if proc.exit_code.is_none() {
            let _ = kill_by_pid_force(pid);
        }
        Ok(format!("sent SIGTERM to {id} (pid {pid})"))
    }
}

/// Send SIGTERM to a pid via the platform `kill` utility (std-only, no FFI).
#[cfg(unix)]
fn kill_by_pid(pid: u32) -> std::io::Result<()> {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("kill -TERM failed"))
    }
}

/// Send SIGKILL to a pid via the platform `kill` utility.
#[cfg(unix)]
fn kill_by_pid_force(pid: u32) -> std::io::Result<()> {
    let status = Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("kill -KILL failed"))
    }
}

#[cfg(not(unix))]
fn kill_by_pid(pid: u32) -> std::io::Result<()> {
    // Windows: kill via taskkill.
    let _ = Command::new("taskkill")
        .arg("/PID")
        .arg(pid.to_string())
        .arg("/T")
        .arg("/F")
        .status();
    Ok(())
}
#[cfg(not(unix))]
fn kill_by_pid_force(pid: u32) -> std::io::Result<()> {
    kill_by_pid(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn start_poll_log_kill_round_trip() {
        let reg = BackgroundRegistry::new();
        let tmp = std::env::temp_dir().join(format!("marlin_bg_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();

        // A command that prints then sleeps — we can poll while it's alive.
        let id = reg
            .start(
                "echo hello-bg && sleep 0.2 && echo world-bg",
                tmp.to_str().unwrap(),
            )
            .expect("bg start");

        // It should be running (or already done by the time we poll, given timing).
        std::thread::sleep(Duration::from_millis(50));
        let statuses = reg.status(&id);
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].running || statuses[0].exit_code.is_some());

        // Wait for it to finish and drain the full log.
        std::thread::sleep(Duration::from_millis(400));
        let (out, _err) = reg.log(&id).expect("bg log");
        assert!(
            out.contains("hello-bg"),
            "stdout should contain hello-bg, got: {out:?}"
        );
        assert!(
            out.contains("world-bg"),
            "stdout should contain world-bg, got: {out:?}"
        );

        let statuses = reg.status(&id);
        assert!(!statuses[0].running, "process should have exited");
        assert_eq!(statuses[0].exit_code, Some(0));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn kill_terminates_running_process() {
        let reg = BackgroundRegistry::new();
        let tmp = std::env::temp_dir().join(format!("marlin_bg_kill_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();

        let id = reg
            .start("sleep 30", tmp.to_str().unwrap())
            .expect("bg start");
        let statuses = reg.status(&id);
        assert!(statuses[0].running, "sleep should be running");

        let msg = reg.kill(&id).expect("bg kill");
        assert!(msg.contains(id.as_str()));

        std::thread::sleep(Duration::from_millis(400));
        let statuses = reg.status(&id);
        assert!(!statuses[0].running, "process should be dead after kill");

        std::fs::remove_dir_all(&tmp).ok();
    }
}
