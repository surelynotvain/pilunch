//! Process helpers shared by git, the agent's `run_command` tool and the terminal.

use std::sync::OnceLock;

/// A `tokio::process::Command` that never flashes a console window on Windows and
/// sees the user's login-shell PATH on Linux/macOS (GUI launchers often start apps with
/// a minimal PATH that lacks ~/.cargo/bin, nvm, etc.).
pub fn command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    if let Some(path) = login_path() {
        cmd.env("PATH", path);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

static LOGIN_PATH: OnceLock<Option<String>> = OnceLock::new();

/// PATH as seen by the user's interactive login shell, resolved once (in the background
/// at startup) and cached. Falls back to the process PATH.
pub fn login_path() -> Option<String> {
    LOGIN_PATH.get().cloned().flatten()
}

/// Resolve the login-shell PATH. Call from a background thread at startup.
pub fn resolve_login_path() {
    LOGIN_PATH.get_or_init(resolve_login_path_inner);
}

#[cfg(unix)]
fn resolve_login_path_inner() -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
    const MARK: &str = "__PILUNCH_PATH__";
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Some shell configs hang (e.g. waiting on a prompt); never wait more than 5s.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let start = out.find(MARK)? + MARK.len();
    let end = start + out[start..].find(MARK)?;
    let path = out[start..end].trim().to_string();
    (!path.is_empty()).then_some(path)
}

#[cfg(not(unix))]
fn resolve_login_path_inner() -> Option<String> {
    None
}

/// The shell used for the agent's `run_command` tool: (program, flag) such that
/// `program flag "<command>"` runs a command string.
pub fn command_shell() -> (String, &'static str) {
    #[cfg(windows)]
    {
        ("powershell.exe".to_string(), "-Command")
    }
    #[cfg(not(windows))]
    {
        if std::path::Path::new("/bin/bash").exists() {
            ("/bin/bash".to_string(), "-c")
        } else {
            ("/bin/sh".to_string(), "-c")
        }
    }
}

/// The interactive shell for the integrated terminal.
pub fn terminal_shell(configured: &str) -> String {
    if !configured.trim().is_empty() {
        return configured.trim().to_string();
    }
    #[cfg(windows)]
    {
        "powershell.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/bash".into())
    }
}
