use anyhow::{Context, Result, bail};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Checks whether `pkexec` is available on this system.
pub fn check_pkexec() -> bool {
    Command::new("which")
        .arg("pkexec")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Builds a local package install that also reinstalls an already installed version.
fn install_deb_command(path: &Path) -> Command {
    let mut command = Command::new("pkexec");
    command
        .args([
            "stdbuf",
            "-oL",
            "-eL",
            "apt-get",
            "install",
            "-y",
            "--allow-downgrades",
            "--reinstall",
        ])
        .arg(path);
    command
}

/// Runs `pkexec apt-get install -y --allow-downgrades --reinstall <path>` and streams its output.
/// apt-get resolves dependencies, allows downgrades, and unpacks same-version packages again.
pub fn install_deb_streaming(path: PathBuf, log_tx: async_channel::Sender<String>) -> Result<()> {
    let mut child = install_deb_command(&path)
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

    fn package_status(architecture: &str, version: &str, status: &str) -> String {
        format!(
            "Package: reinstall-test\nStatus: {status}\nArchitecture: {architecture}\n\
             Version: {version}\nMaintainer: Test <test@example.invalid>\n\
             Description: Reinstall regression fixture\n\n"
        )
    }

    fn fixture_installed_version(status: &str, architecture: &str) -> Option<String> {
        let admin = tempfile::tempdir().unwrap();
        std::fs::write(admin.path().join("status"), status).unwrap();
        let mut query = Command::new("dpkg-query");
        query.arg(format!("--admindir={}", admin.path().display()));
        query_installed_version(query, "reinstall-test", architecture)
    }

    #[test]
    fn installed_version_requires_installed_status() {
        for (status, expected) in [
            ("install ok installed", Some("1.0".to_string())),
            ("deinstall ok config-files", None),
            ("purge ok not-installed", None),
            ("install ok unpacked", None),
            ("install ok half-configured", None),
        ] {
            assert_eq!(
                fixture_installed_version(&package_status("amd64", "1.0", status), "amd64"),
                expected,
                "status: {status}"
            );
        }
        assert_eq!(fixture_installed_version("", "amd64"), None);
    }

    #[test]
    fn installed_version_selects_one_architecture_without_concatenating_versions() {
        let status = ["amd64", "i386"]
            .map(|arch| {
                package_status(arch, "1.0", "install ok installed")
                    .replace("Version:", "Multi-Arch: same\nVersion:")
            })
            .concat();
        for arch in ["amd64", "i386"] {
            assert_eq!(
                fixture_installed_version(&status, arch).as_deref(),
                Some("1.0")
            );
        }
        assert_eq!(fixture_installed_version(&status, "arm64"), None);
    }

    #[test]
    fn installed_version_does_not_use_a_different_architecture() {
        let status = package_status("i386", "1.0", "install ok installed");
        assert_eq!(fixture_installed_version(&status, "amd64"), None);
        assert_eq!(
            fixture_installed_version(&status, "i386").as_deref(),
            Some("1.0")
        );
    }

    #[test]
    fn installed_version_handles_architecture_all() {
        let status = package_status("all", "1.0", "install ok installed");
        assert_eq!(
            fixture_installed_version(&status, "all").as_deref(),
            Some("1.0")
        );
    }

    #[test]
    fn local_deb_supports_fresh_install_reinstall_upgrade_and_downgrade() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        std::fs::create_dir_all(package.join("DEBIAN")).unwrap();
        std::fs::write(
            package.join("DEBIAN/control"),
            "Package: reinstall-test\nVersion: 1.0\nArchitecture: all\n\
             Maintainer: Test <test@example.invalid>\nDescription: Reinstall regression fixture\n",
        )
        .unwrap();
        let deb = root.path().join("reinstall test.deb");
        let build = Command::new("dpkg-deb")
            .arg("--build")
            .arg(&package)
            .arg(&deb)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );

        let status_path = root.path().join("status");
        let sources = root.path().join("sources.list");
        std::fs::write(&sources, "").unwrap();
        for directory in ["sources.list.d", "lists/partial", "archives/partial"] {
            std::fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        let install = install_deb_command(&deb);
        assert_eq!(install.get_program(), "pkexec");
        let args = install.get_args().collect::<Vec<_>>();
        assert_eq!(&args[..4], ["stdbuf", "-oL", "-eL", "apt-get"]);

        for (installed, summary) in [
            (None, "0 upgraded, 1 newly installed"),
            (Some("1.0"), "0 upgraded, 0 newly installed, 1 reinstalled"),
            (Some("0.9"), "1 upgraded, 0 newly installed"),
            (Some("1.1"), "0 upgraded, 0 newly installed, 1 downgraded"),
        ] {
            let status = installed
                .map(|version| package_status("all", version, "install ok installed"))
                .unwrap_or_default();
            std::fs::write(&status_path, status).unwrap();
            // Exercise the production apt arguments without privileges, repositories, or writes.
            let mut simulation = Command::new("apt-get");
            simulation.arg("--simulate").env("LC_ALL", "C");
            for (key, path) in [
                ("Dir::State::status", status_path.clone()),
                ("Dir::State::lists", root.path().join("lists")),
                ("Dir::Cache::archives", root.path().join("archives")),
                ("Dir::Etc::sourcelist", sources.clone()),
                ("Dir::Etc::sourceparts", root.path().join("sources.list.d")),
            ] {
                simulation.args(["-o", &format!("{key}={}", path.display())]);
            }
            let output = simulation.args(&args[4..]).output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "installed: {installed:?}\n{stdout}\n{stderr}"
            );
            assert!(
                stdout.contains(summary),
                "installed: {installed:?}\n{stdout}"
            );
            assert!(stdout.contains("Inst reinstall-test"), "{stdout}");
        }
    }

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

/// Returns the installed version for this exact package and architecture.
/// Removed packages with residual configuration are not considered installed.
pub fn installed_version(name: &str, architecture: &str) -> Option<String> {
    query_installed_version(Command::new("dpkg-query"), name, architecture)
}

fn query_installed_version(mut query: Command, name: &str, architecture: &str) -> Option<String> {
    let output = query
        .args(["-W", "-f=${db:Status-Status}\t${Version}\n", "--"])
        .arg(format!("{name}:{architecture}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (status, version) = stdout.trim().split_once('\t')?;
    (status == "installed" && !version.is_empty()).then(|| version.to_string())
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
