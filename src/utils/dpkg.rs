use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Checks whether `pkexec` is available on this system.
pub fn check_pkexec() -> bool {
    Command::new("which")
        .arg("pkexec")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs `pkexec dpkg -i`, streams stdout+stderr line-by-line through `log_tx`,
/// closes the channel when done, then returns Ok/Err for the exit status.
pub fn install_deb_streaming(path: PathBuf, log_tx: async_channel::Sender<String>) -> Result<()> {
    let mut child = Command::new("pkexec")
        .args(["dpkg", "-i"])
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to launch pkexec dpkg -i")?;

    pipe_output_to_channel(child.stdout.take(), child.stderr.take(), log_tx);

    let status = child.wait()?;
    if !status.success() {
        bail!("dpkg -i failed (exit {})", status);
    }
    Ok(())
}

/// Runs `pkexec apt remove --yes`, streams stdout+stderr, returns Ok/Err.
pub fn remove_package_streaming(name: String, log_tx: async_channel::Sender<String>) -> Result<()> {
    let mut child = Command::new("pkexec")
        .args(["apt", "remove", "--yes"])
        .arg(&name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to launch pkexec apt remove")?;

    pipe_output_to_channel(child.stdout.take(), child.stderr.take(), log_tx);

    let status = child.wait()?;
    if !status.success() {
        bail!("apt remove failed (exit {})", status);
    }
    Ok(())
}

/// Spawns two threads to read stdout and stderr, sending each line to `log_tx`.
/// Blocks until both threads finish (i.e. the child has closed its pipes).
fn pipe_output_to_channel(
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    log_tx: async_channel::Sender<String>,
) {
    let tx_out = log_tx.clone();
    let t_out = std::thread::spawn(move || {
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().flatten() {
                if tx_out.send_blocking(line).is_err() {
                    break;
                }
            }
        }
    });

    let tx_err = log_tx.clone();
    let t_err = std::thread::spawn(move || {
        if let Some(err) = stderr {
            for line in BufReader::new(err).lines().flatten() {
                if tx_err.send_blocking(line).is_err() {
                    break;
                }
            }
        }
    });

    t_out.join().ok();
    t_err.join().ok();
    // Drop our clone so the channel closes once all thread senders also drop
    drop(log_tx);
}

/// Returns the currently installed version of a package, or None if not installed.
pub fn installed_version(name: &str) -> Option<String> {
    let output = Command::new("dpkg-query")
        .args(["-W", "-f=${Version}", name])
        .output()
        .ok()?;
    if output.status.success() {
        let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if v.is_empty() { None } else { Some(v) }
    } else {
        None
    }
}


