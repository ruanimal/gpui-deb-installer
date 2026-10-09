//! Install / update times of installed packages.
//!
//! dpkg records no dates in its status database, but it writes every action to
//! `/var/log/dpkg.log`, which logrotate rotates (and gzips) by size/date. Those
//! files are the only place that distinguishes the *first* installation from a
//! later *upgrade*, so they are read first.
//!
//! Logs are eventually deleted, and packages from the base system predate them,
//! so a package with no surviving log entry falls back to the mtime of its
//! `/var/lib/dpkg/info/<name>.list` file, which dpkg rewrites on every unpack.
//! That fallback can only report the *last* unpack, so install and update time
//! are then shown as the same value.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const DPKG_LOG_DIR: &str = "/var/log";
const DPKG_INFO_DIR: &str = "/var/lib/dpkg/info";

/// Install and update times collected for one package.
#[derive(Debug, Default, Clone, Copy)]
pub struct PkgTimes {
    /// Time of the first `install` entry that came from `<none>`.
    pub installed: Option<DateTime<Local>>,
    /// Time of the most recent `install` or `upgrade` entry.
    pub updated: Option<DateTime<Local>>,
}

/// Scans every retained dpkg log and returns the times known for each package.
///
/// Never fails: unreadable or missing log files are skipped, because the
/// `.list` mtime fallback in [`info_mtime`] still covers those packages.
pub fn collect() -> HashMap<String, PkgTimes> {
    let mut times: HashMap<String, PkgTimes> = HashMap::new();
    let mut earliest: HashMap<String, DateTime<Local>> = HashMap::new();

    for path in log_files() {
        let Ok(reader) = open_log(&path) else {
            continue;
        };
        for line in reader.lines().map_while(Result::ok) {
            let Some((ts, name, first_install)) = parse_event(&line) else {
                continue;
            };
            let entry = times.entry(name.clone()).or_default();
            if entry.updated.is_none_or(|updated| ts > updated) {
                entry.updated = Some(ts);
            }
            if first_install && entry.installed.is_none_or(|installed| ts < installed) {
                entry.installed = Some(ts);
            }
            let first_seen = earliest.entry(name).or_insert(ts);
            if ts < *first_seen {
                *first_seen = ts;
            }
        }
    }

    // The real first install may predate the oldest surviving log, so the
    // earliest action still on record is a better guess than "unknown".
    for (name, ts) in earliest {
        let entry = times.entry(name).or_default();
        if entry.installed.is_none() {
            entry.installed = Some(ts);
        }
    }

    times
}

/// Modification time of a package's dpkg `.list` file — i.e. when its files
/// were last unpacked. Used when no log entry mentions the package.
pub fn info_mtime(name: &str, architecture: &str) -> Option<DateTime<Local>> {
    let mut candidates = vec![format!("{DPKG_INFO_DIR}/{name}.list")];
    if !architecture.is_empty() {
        candidates.push(format!("{DPKG_INFO_DIR}/{name}:{architecture}.list"));
    }
    candidates
        .iter()
        .find_map(|path| std::fs::metadata(path).ok()?.modified().ok())
        .map(DateTime::<Local>::from)
}

/// All retained dpkg logs, oldest first.
fn log_files() -> Vec<PathBuf> {
    let mut numbered: Vec<(u32, PathBuf)> = Vec::new();
    let mut plain: Option<PathBuf> = None;

    if let Ok(entries) = std::fs::read_dir(DPKG_LOG_DIR) {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name == "dpkg.log" {
                plain = Some(entry.path());
            } else if let Some(suffix) = file_name.strip_prefix("dpkg.log.") {
                let number = suffix.strip_suffix(".gz").unwrap_or(suffix);
                if let Ok(number) = number.parse::<u32>() {
                    numbered.push((number, entry.path()));
                }
            }
        }
    }

    // `dpkg.log.1` is the newest rotation, so a higher number is older.
    numbered.sort_by_key(|(number, _)| std::cmp::Reverse(*number));
    let mut files: Vec<PathBuf> = numbered.into_iter().map(|(_, path)| path).collect();
    if let Some(plain) = plain {
        files.push(plain);
    }
    files
}

/// Opens a log, transparently decompressing `.gz` rotations.
fn open_log(path: &Path) -> std::io::Result<Box<dyn BufRead>> {
    let file = File::open(path)?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("gz") {
        Ok(Box::new(BufReader::new(GzDecoder::new(file))))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

/// Parses one dpkg log line into `(timestamp, package, came_from_none)`.
///
/// Relevant lines look like `2026-10-03 10:49:56 install 115life:amd64 <none> 37.3.1`;
/// every other action (`status`, `configure`, `remove`, …) is ignored.
fn parse_event(line: &str) -> Option<(DateTime<Local>, String, bool)> {
    let mut fields = line.split_whitespace();
    let date = fields.next()?;
    let time = fields.next()?;
    let action = fields.next()?;
    if action != "install" && action != "upgrade" {
        return None;
    }
    let package = fields.next()?;
    let old_version = fields.next().unwrap_or_default();
    let new_version = fields.next().unwrap_or_default();
    if new_version.is_empty() || new_version == "<none>" {
        return None;
    }

    let naive =
        NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S").ok()?;
    let timestamp = Local.from_local_datetime(&naive).earliest()?;
    let name = package.split(':').next().unwrap_or(package).to_string();
    Some((timestamp, name, old_version == "<none>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_install_and_upgrade_lines() {
        let (timestamp, name, from_none) =
            parse_event("2026-10-03 10:49:56 install 115life:amd64 <none> 37.3.1").unwrap();
        assert_eq!(name, "115life");
        assert!(from_none);
        assert_eq!(
            timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-10-03 10:49:56"
        );

        let (_, name, from_none) =
            parse_event("2026-10-01 00:19:15 upgrade deepseek-harness-desktop:amd64 0.18.0 0.20.0")
                .unwrap();
        assert_eq!(name, "deepseek-harness-desktop");
        assert!(!from_none);
    }

    #[test]
    fn ignores_unrelated_lines() {
        assert!(parse_event("2026-10-01 00:19:15 status unpacked foo:amd64 1.0").is_none());
        assert!(parse_event("2026-10-01 00:19:15 startup archives unpack").is_none());
        assert!(parse_event("2026-10-01 00:19:15 remove foo:amd64 1.0 <none>").is_none());
        assert!(parse_event("").is_none());
    }
}
