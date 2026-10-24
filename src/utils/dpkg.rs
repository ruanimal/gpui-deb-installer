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

/// Runs `pkexec apt-get install -y --allow-downgrades <path>`, streams stdout+stderr line-by-line through `log_tx`.
/// apt-get handles dependency resolution automatically.
/// `--allow-downgrades` is required so users can install an older version of a package
/// over a newer one already present on the system (apt otherwise refuses with an error).
pub fn install_deb_streaming(path: PathBuf, log_tx: async_channel::Sender<String>) -> Result<()> {
    let mut child = Command::new("pkexec")
        .args([
            "stdbuf",
            "-oL",
            "-eL",
            "apt-get",
            "install",
            "-y",
            "--allow-downgrades",
        ])
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

/// Builds one apt transaction for all selected packages.
fn remove_packages_command(names: &[String], purge: bool) -> Result<Command> {
    if names.is_empty() {
        bail!("no packages selected for removal");
    }
    let action = if purge { "purge" } else { "remove" };
    let mut command = Command::new("pkexec");
    command
        .args(["stdbuf", "-oL", "-eL", "apt", action, "--yes", "--"])
        .args(names);
    Ok(command)
}

/// Runs one `pkexec apt remove/purge --yes` transaction and streams stdout+stderr.
pub fn remove_packages_streaming(
    names: Vec<String>,
    purge: bool,
    log_tx: async_channel::Sender<String>,
) -> Result<()> {
    let action = if purge { "purge" } else { "remove" };
    let mut child = remove_packages_command(&names, purge)?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch pkexec apt {action}"))?;

    pipe_output_to_channel(child.stdout.take(), child.stderr.take(), log_tx);

    let status = child.wait()?;
    if !status.success() {
        bail!("apt {action} failed (exit {})", status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_removal_uses_one_command_with_separate_package_arguments() {
        let command =
            remove_packages_command(&["app:amd64".into(), "lib:i386".into()], false).unwrap();
        assert_eq!(command.get_program(), "pkexec");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "stdbuf",
                "-oL",
                "-eL",
                "apt",
                "remove",
                "--yes",
                "--",
                "app:amd64",
                "lib:i386"
            ]
        );
    }

    #[test]
    fn purge_applies_to_the_whole_batch() {
        let command = remove_packages_command(&["app".into(), "other".into()], true).unwrap();
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "stdbuf", "-oL", "-eL", "apt", "purge", "--yes", "--", "app", "other"
            ]
        );
    }

    #[test]
    fn empty_removal_is_rejected_before_launching_pkexec() {
        assert!(remove_packages_command(&[], false).is_err());
        assert!(remove_packages_command(&[], true).is_err());
    }
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
