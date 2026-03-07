use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Checks whether `pkexec` is available on this system.
pub fn check_pkexec() -> bool {
    Command::new("which")
        .arg("pkexec")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Installs a .deb file using `pkexec dpkg -i`.
/// Returns the combined stdout+stderr output on success.
pub fn install_deb(path: &Path) -> Result<String> {
    let output = Command::new("pkexec")
        .args(["dpkg", "-i"])
        .arg(path)
        .output()
        .context("failed to launch pkexec dpkg -i")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr);

    if output.status.success() {
        Ok(combined)
    } else {
        bail!("dpkg -i failed (exit {}): {}", output.status, combined);
    }
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

/// Removes a package using `pkexec apt remove --yes`.
/// Returns the combined stdout+stderr output on success.
pub fn remove_package(name: &str) -> Result<String> {
    let output = Command::new("pkexec")
        .args(["apt", "remove", "--yes"])
        .arg(name)
        .output()
        .context("failed to launch pkexec apt remove")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr);

    if output.status.success() {
        Ok(combined)
    } else {
        bail!(
            "apt remove failed (exit {}): {}",
            output.status,
            combined
        );
    }
}
