//! Installed-package listing, read from apt instead of a local record.

use anyhow::{Context, Result};
use std::process::Command;

use crate::models::package::InstalledPackage;
use crate::utils::pkg_time;

/// apt pattern for packages installed from a local `.deb` (or removed from
/// every configured repository) — the default view of the Installed tab.
pub const LOCAL_PATTERN: &str = "?installed ?obsolete";

/// apt pattern for every installed package, including those still available in
/// a repository.
pub const ALL_PATTERN: &str = "?installed";

/// Lists installed packages, with install/update times.
///
/// `include_repository_packages` selects between "locally installed .deb
/// packages only" and everything `dpkg` knows about.
pub fn list_installed(include_repository_packages: bool) -> Result<Vec<InstalledPackage>> {
    let pattern = if include_repository_packages {
        ALL_PATTERN
    } else {
        LOCAL_PATTERN
    };
    let mut packages = query(pattern)?;
    let times = pkg_time::collect();

    for package in &mut packages {
        if let Some(known) = times.get(&package.name) {
            package.install_date = known.installed;
            package.update_date = known.updated;
        }
        if package.update_date.is_none() {
            package.update_date = pkg_time::info_mtime(&package.name, &package.architecture);
        }
        if package.install_date.is_none() {
            package.install_date = package.update_date;
        }
    }

    packages.sort_by_key(|package| package.name.to_lowercase());
    Ok(packages)
}

/// Runs `apt list <pattern>` and parses its output.
fn query(pattern: &str) -> Result<Vec<InstalledPackage>> {
    // `LC_ALL=C` keeps the `[installed,…]` status suffixes and the "Listing…"
    // header in a fixed, parseable form regardless of the user's locale.
    let output = Command::new("apt")
        .arg("list")
        .arg(pattern)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .context("failed to run `apt list`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("`apt list` failed");
        anyhow::bail!("{message}");
    }

    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
}

/// Parses lines shaped like `name/repository version architecture [installed,…]`.
fn parse(stdout: &str) -> Vec<InstalledPackage> {
    stdout.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<InstalledPackage> {
    let (name, rest) = line.trim().split_once('/')?;
    if name.is_empty() {
        return None;
    }

    let mut fields = rest.split_whitespace();
    // The archive name comes first (`now`, `noble,now`, …); it is not shown.
    let _archive = fields.next()?;
    let version = fields.next()?.to_string();
    let architecture = fields.next().unwrap_or_default().to_string();

    // apt prints the architecture only for foreign-arch packages (`foo:i386`).
    let name = match name.rsplit_once(':') {
        Some((base, arch)) if arch == architecture => base.to_string(),
        _ => name.to_string(),
    };

    Some(InstalledPackage {
        name,
        version,
        architecture,
        install_date: None,
        update_date: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apt_list_output() {
        let output = "Listing...\n\
115life/now 37.3.1 amd64 [installed,local]\n\
7zip/noble,now 23.01+dfsg-11 amd64 [installed,automatic]\n\
libdbus-1-3:amd64/noble,now 1.14.10-4ubuntu4.1 amd64 [installed]\n\
WARNING: apt does not have a stable CLI interface.\n";

        let packages = parse(output);
        assert_eq!(packages.len(), 3);
        assert_eq!(packages[0].name, "115life");
        assert_eq!(packages[0].version, "37.3.1");
        assert_eq!(packages[0].architecture, "amd64");
        assert_eq!(packages[1].version, "23.01+dfsg-11");
        // The foreign-arch suffix is dropped when it matches the printed arch.
        assert_eq!(packages[2].name, "libdbus-1-3");
    }
}
