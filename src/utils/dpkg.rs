use anyhow::{Context, Result, bail};
use std::io::{BufReader, Read};
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

/// Runs `pkexec apt-get install -y <path>`, streams stdout+stderr line-by-line through `log_tx`.
/// apt-get handles dependency resolution automatically.
pub fn install_deb_streaming(path: PathBuf, log_tx: async_channel::Sender<String>) -> Result<()> {
    let mut child = Command::new("pkexec")
        .args(["stdbuf", "-oL", "-eL", "apt-get", "install", "-y"])
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to launch pkexec apt-get install")?;

    pipe_output_to_channel(child.stdout.take(), child.stderr.take(), log_tx);

    let status = child.wait()?;
    if !status.success() {
        bail!("apt-get install failed (exit {})", status);
    }
    Ok(())
}

/// Runs `pkexec apt remove --yes`, streams stdout+stderr, returns Ok/Err.
pub fn remove_package_streaming(name: String, log_tx: async_channel::Sender<String>) -> Result<()> {
    let mut child = Command::new("pkexec")
        .args(["stdbuf", "-oL", "-eL", "apt", "remove", "--yes"])
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

/// Reads from a stream and splits on both `\n` and `\r`, sending each non-empty
/// segment as a separate line. When dpkg emits `\r`-terminated progress updates,
/// each percentage step becomes its own line instead of all being jammed together.
fn read_lines_cr_lf<R: Read>(reader: R, tx: &async_channel::Sender<String>) {
    let mut buf = Vec::new();
    for byte in BufReader::new(reader).bytes().flatten() {
        if byte == b'\n' || byte == b'\r' {
            if !buf.is_empty() {
                let line = String::from_utf8_lossy(&buf).into_owned();
                if tx.send_blocking(line).is_err() {
                    return;
                }
                buf.clear();
            }
        } else {
            buf.push(byte);
        }
    }
    // flush any remaining content without a trailing newline
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).into_owned();
        tx.send_blocking(line).ok();
    }
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
            read_lines_cr_lf(out, &tx_out);
        }
    });

    let tx_err = log_tx.clone();
    let t_err = std::thread::spawn(move || {
        if let Some(err) = stderr {
            read_lines_cr_lf(err, &tx_err);
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

/// Compares two Debian version strings using `dpkg --compare-versions`.
pub fn compare_versions(v1: &str, v2: &str) -> std::cmp::Ordering {
    // Check if they are equal first.
    if Command::new("dpkg")
        .args(["--compare-versions", v1, "eq", v2])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return std::cmp::Ordering::Equal;
    }

    // Check if v1 is strictly less than v2.
    if Command::new("dpkg")
        .args(["--compare-versions", v1, "lt", v2])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return std::cmp::Ordering::Less;
    }

    // Otherwise, assume v1 is strictly greater than v2.
    std::cmp::Ordering::Greater
}


