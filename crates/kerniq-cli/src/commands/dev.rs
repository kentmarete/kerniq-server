//! `kerniq dev`: build, run, and restart on source changes.
//!
//! Changes are detected by polling modification times of `src/`,
//! `Cargo.toml` and `*.toml` files in the project root (no platform-specific
//! file watcher dependency). On a change the project is rebuilt *while the
//! old server keeps running*; only a successful build replaces it, so a
//! syntax error never leaves you without a server.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::cargo::cargo;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run(extra: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::current_dir()?;
    if !root.join("Cargo.toml").exists() {
        return Err(
            "no Cargo.toml in the current directory; run `kerniq dev` from a project root".into(),
        );
    }

    let mut snapshot = latest_change(&root);
    let mut server = build_and_start(extra)?;
    eprintln!("==> watching for changes (Ctrl+C to stop)");

    loop {
        thread::sleep(POLL_INTERVAL);
        if let Some(child) = server.as_mut()
            && let Some(status) = child.try_wait()?
        {
            eprintln!("==> server exited ({status}); waiting for changes");
            server = None;
        }
        let current = latest_change(&root);
        if current <= snapshot {
            continue;
        }
        snapshot = current;
        eprintln!("==> change detected, rebuilding");
        match cargo(&["build"], &[])? {
            status if status.success() => {
                if let Some(child) = server.take() {
                    stop(child);
                }
                server = start(extra)?;
            }
            _ => eprintln!("==> build failed; the previous server keeps running"),
        }
    }
}

fn build_and_start(extra: &[String]) -> Result<Option<Child>, Box<dyn std::error::Error>> {
    if cargo(&["build"], &[])?.success() {
        start(extra)
    } else {
        eprintln!("==> build failed; fix the errors and save to retry");
        Ok(None)
    }
}

fn start(extra: &[String]) -> Result<Option<Child>, Box<dyn std::error::Error>> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let child = Command::new(cargo)
        .arg("run")
        .arg("--quiet")
        .args(extra)
        .spawn()?;
    eprintln!("==> server started (pid {})", child.id());
    Ok(Some(child))
}

/// Asks the server to shut down gracefully (SIGTERM on Unix), then kills it
/// if it has not exited within the timeout.
fn stop(mut child: Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status();
        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The most recent modification time among watched files.
pub fn latest_change(root: &Path) -> SystemTime {
    let mut latest = SystemTime::UNIX_EPOCH;
    let mut pending: Vec<PathBuf> = vec![root.join("src")];
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                pending.push(path);
            }
        }
    }
    while let Some(path) = pending.pop() {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if let Ok(modified) = meta.modified() {
            latest = latest.max(modified);
        }
        if meta.is_dir()
            && let Ok(entries) = std::fs::read_dir(&path)
        {
            pending.extend(entries.flatten().map(|e| e.path()));
        }
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_changes_in_nested_sources() {
        let dir = std::env::temp_dir().join(format!("kerniq-dev-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/routes")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        let before = latest_change(&dir);
        thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("src/routes/users.rs"), "// changed").unwrap();
        assert!(latest_change(&dir) > before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
