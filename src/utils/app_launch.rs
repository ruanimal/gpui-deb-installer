//! Locating and opening the application a package provides.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// System-wide directories holding freedesktop application entries.
const APPLICATIONS_DIRS: [&str; 2] = ["/usr/share/applications", "/usr/local/share/applications"];

/// Commands that hand a `.desktop` entry to the desktop environment.
///
/// `open` is the alias some distributions (and `/etc/alternatives`) provide for
/// `xdg-open`; `gio` is tried separately because it needs a subcommand.
const OPENERS: [&str; 2] = ["open", "xdg-open"];

/// The application a package exposes, as a freedesktop `.desktop` entry.
///
/// Only desktop entries count: the executables a package ships are usually
/// helpers, so they are not offered as something to "open".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppTarget(PathBuf);

impl AppTarget {
    /// Path of the `.desktop` entry.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Picks the application among the files `pkg` installs, if it ships one.
///
/// The file list comes from the package itself (see the Files tab), so no
/// package database query is needed. Packages that only ship libraries,
/// documentation or system services have no `.desktop` entry, and therefore
/// nothing to open.
pub fn select_app_target(files: &[PathBuf], pkg: &str) -> Option<AppTarget> {
    select_app_target_in(files, pkg, &APPLICATIONS_DIRS)
}

/// Picks the best desktop entry among a package's files.
fn select_app_target_in(
    files: &[PathBuf],
    pkg: &str,
    applications_dirs: &[&str],
) -> Option<AppTarget> {
    // Visible entries first, then the one named after the package.
    let mut entries: Vec<&PathBuf> = files
        .iter()
        .filter(|file| desktop_id(file, applications_dirs).is_some())
        .collect();
    entries.sort_by_key(|file| {
        let stem = file
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        (!is_visible_desktop(file), name_rank(&stem, pkg), stem)
    });
    entries.first().map(|entry| AppTarget((*entry).clone()))
}

/// The freedesktop ID of a `.desktop` file, or `None` when it is not a
/// system-wide application entry.
fn desktop_id(path: &Path, applications_dirs: &[&str]) -> Option<String> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
        return None;
    }
    applications_dirs.iter().find_map(|dir| {
        let relative = path.strip_prefix(dir).ok()?;
        if relative.as_os_str().is_empty() {
            return None;
        }
        // The ID replaces the path separators of nested entries with dashes.
        Some(relative.to_string_lossy().replace('/', "-"))
    })
}

/// Whether an entry is meant to be shown to the user.
///
/// Entries that hide themselves are still usable, so they are ranked last
/// rather than discarded.
fn is_visible_desktop(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return true;
    };
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let self_hidden = ["NoDisplay=", "Hidden="].iter().any(|key| {
            line.strip_prefix(*key)
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
        });
        if self_hidden {
            return false;
        }
    }
    true
}

/// Ranks a file name against the package name: 0 for a match, then a partial
/// match, then anything else.
fn name_rank(name: &str, pkg: &str) -> u8 {
    let pkg = pkg.to_lowercase();
    if name.is_empty() || pkg.is_empty() {
        return 2;
    }
    if name == pkg {
        0
    } else if name.contains(pkg.as_str()) || pkg.contains(name) {
        1
    } else {
        2
    }
}

/// Opens `target` through the system opener.
pub fn launch(target: &AppTarget) -> Result<()> {
    let path = target.path();

    for opener in OPENERS {
        if spawn_detached(Command::new(opener).arg(path)).is_ok() {
            return Ok(());
        }
    }

    if spawn_detached(Command::new("gio").arg("launch").arg(path)).is_ok() {
        return Ok(());
    }

    // `gtk-launch` takes the entry ID rather than the path.
    if let Some(id) = desktop_id(path, &APPLICATIONS_DIRS)
        && spawn_detached(Command::new("gtk-launch").arg(&id)).is_ok()
    {
        return Ok(());
    }

    bail!("no application opener found (tried open, xdg-open, gio, gtk-launch)")
}

/// Spawns a detached process, reaping it on a separate thread so it cannot
/// become a zombie, and with its standard streams detached from this one.
fn spawn_detached(command: &mut Command) -> Result<()> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    reap(child);
    Ok(())
}

fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn desktop_ids_are_relative_to_the_applications_dir() {
        let dirs = ["/usr/share/applications"];
        let id = |path: &str| desktop_id(Path::new(path), &dirs);

        assert_eq!(
            id("/usr/share/applications/foo.desktop").as_deref(),
            Some("foo.desktop")
        );
        assert_eq!(
            id("/usr/share/applications/kde/foo.desktop").as_deref(),
            Some("kde-foo.desktop")
        );
        assert_eq!(id("/usr/share/doc/foo.desktop"), None);
        assert_eq!(id("/usr/share/applications/foo.png"), None);
    }

    #[test]
    fn a_visible_entry_beats_a_hidden_one() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("applications");
        let hidden = apps.join("foo.desktop");
        let visible = apps.join("foo-gui.desktop");
        write(&hidden, "[Desktop Entry]\nNoDisplay=true\nExec=foo\n");
        write(&visible, "[Desktop Entry]\nExec=foo-gui\n");

        let dirs = [apps.to_str().unwrap()];
        let files = vec![hidden, visible.clone()];

        assert_eq!(
            select_app_target_in(&files, "foo", &dirs),
            Some(AppTarget(visible))
        );
    }

    #[test]
    fn the_entry_named_after_the_package_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("applications");
        let other = apps.join("foo-helper.desktop");
        let main = apps.join("foo.desktop");
        write(&other, "[Desktop Entry]\nExec=foo-helper\n");
        write(&main, "[Desktop Entry]\nExec=foo\n");

        let dirs = [apps.to_str().unwrap()];
        let files = vec![other, main.clone()];

        assert_eq!(select_app_target_in(&files, "foo", &dirs), Some(AppTarget(main)));
    }

    #[test]
    fn a_package_without_a_desktop_entry_has_no_target() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("applications");
        let bin = tmp.path().join("bin");
        let docs = tmp.path().join("doc").join("README");
        write(&bin.join("foo"), "#!/bin/sh\n");
        write(&docs, "docs");

        let dirs = [apps.to_str().unwrap()];
        let files = vec![bin.join("foo"), docs, bin.join("foo.conf")];

        assert_eq!(select_app_target_in(&files, "foo", &dirs), None);
    }
}
