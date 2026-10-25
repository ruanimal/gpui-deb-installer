use anyhow::{Context, Result, bail};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of content we can preview.
#[derive(Debug, Clone)]
pub enum DebFileKind {
    Text,
    Image,
    Unsupported,
}

/// A single entry from inside the .deb package.
/// Content is NOT held in memory — it lives on disk inside the temp directory.
#[derive(Debug, Clone)]
pub struct DebFileEntry {
    /// Full path as stored in the tar archive (e.g. `./usr/bin/foo`)
    pub path: String,
    pub kind: DebFileKind,
    /// Uncompressed size of the member in bytes, as recorded in the tar header.
    pub size: u64,
    /// Absolute path to the cached file on disk (None for Unsupported files
    /// that were not extracted).
    pub cache_path: Option<PathBuf>,
}

/// Result of extracting a .deb: the entry list plus the temp directory handle.
/// The temp directory is deleted when the last snapshot is dropped.
#[derive(Clone)]
pub struct ExtractedDeb {
    pub entries: Vec<DebFileEntry>,
    /// Keep alive until the worker and all UI snapshots have been dropped.
    pub _temp_dir: Arc<TempDir>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Stream preview metadata while decompressing the archive. Returning false from
/// `on_progress` cancels extraction. Snapshots keep their cached files alive.
/// This is a **blocking** function — call from a background executor.
pub fn extract_previewable_files(
    path: &Path,
    mut on_progress: impl FnMut(ExtractedDeb) -> bool,
) -> Result<ExtractedDeb> {
    let temp_dir = Arc::new(TempDir::new().context("failed to create temp directory")?);
    let mut child = Command::new("dpkg")
        .arg("--fsys-tarfile")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn dpkg --fsys-tarfile")?;
    let stdout = child.stdout.take().context("no stdout")?;
    let result = read_preview_archive(stdout, temp_dir, &mut on_progress);
    if result.is_err() {
        // Close the pipe and reap dpkg even when parsing fails or is cancelled.
        let _ = child.kill();
    }
    let status = child.wait().context("failed to wait for dpkg")?;
    let extracted = result?;
    if !status.success() {
        bail!("dpkg --fsys-tarfile failed: {status}");
    }
    Ok(extracted)
}

fn read_preview_archive(
    reader: impl Read,
    temp_dir: Arc<TempDir>,
    on_progress: &mut impl FnMut(ExtractedDeb) -> bool,
) -> Result<ExtractedDeb> {
    let mut archive = tar::Archive::new(BufReader::with_capacity(256 * 1024, reader));
    let mut entries_out: Vec<DebFileEntry> = Vec::new();
    let mut last_progress = None;

    for entry_result in archive.entries()? {
        let mut entry = entry_result.context("failed to read archive entry")?;

        let header = entry.header();

        if header.entry_type() != tar::EntryType::Regular {
            continue;
        }

        let raw_path = match entry.path() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(_) => continue,
        };

        const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
        const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

        let file_size = header.size().unwrap_or(0);

        let (kind, cache_path) =
            if is_known_binary_ext(&raw_path) || file_size > MAX_IMAGE_BYTES as u64 {
                (DebFileKind::Unsupported, None)
            } else {
                let mut buf = Vec::with_capacity(file_size as usize);
                entry
                    .read_to_end(&mut buf)
                    .context("failed to read preview content")?;
                let kind = categorize(&raw_path, &buf, MAX_TEXT_BYTES, MAX_IMAGE_BYTES);
                let cache_path = if matches!(kind, DebFileKind::Unsupported) {
                    None
                } else {
                    // Flat cache names avoid per-directory I/O and unsafe archive paths.
                    let dest = temp_dir.path().join(entries_out.len().to_string());
                    std::fs::write(&dest, &buf).context("failed to cache preview content")?;
                    Some(dest)
                };
                (kind, cache_path)
            };

        entries_out.push(DebFileEntry {
            path: raw_path,
            kind,
            size: file_size,
            cache_path,
        });

        // Publish the first file immediately, then throttle full snapshots.
        if last_progress.is_none_or(|last: Instant| last.elapsed() >= Duration::from_millis(250)) {
            if !on_progress(ExtractedDeb {
                entries: entries_out.clone(),
                _temp_dir: temp_dir.clone(),
            }) {
                bail!("preview extraction cancelled");
            }
            last_progress = Some(Instant::now());
        }
    }

    // tar stops at its end marker; drain any padding before waiting for dpkg.
    std::io::copy(&mut archive.into_inner(), &mut std::io::sink())?;

    Ok(ExtractedDeb {
        entries: entries_out,
        _temp_dir: temp_dir,
    })
}

/// Collect menu categories from application entries shipped in the archive.
/// Read the cached package contents, never an already installed desktop file.
pub fn desktop_categories(entries: &[DebFileEntry]) -> Vec<String> {
    let mut desktop_entries: Vec<_> = entries.iter().filter(|entry| {
        let path = installed_path(&entry.path);
        path.extension().and_then(|ext| ext.to_str()) == Some("desktop")
            && ["/usr/share/applications", "/usr/local/share/applications"]
                .iter().any(|dir| path.starts_with(dir))
    }).collect();
    desktop_entries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut categories = Vec::new();
    for entry in desktop_entries {
        let Some(data) = read_cached_file(entry) else { continue };
        let Ok(text) = std::str::from_utf8(&data) else { continue };
        for category in parse_desktop_categories(text) {
            if !categories.contains(&category) {
                categories.push(category);
            }
        }
    }
    categories
}

fn parse_desktop_categories(text: &str) -> Vec<String> {
    let mut in_entry = false;
    let mut is_application = false;
    let mut hidden = false;
    let mut categories = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') { continue; }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "Type" => is_application = value == "Application",
            "Hidden" | "NoDisplay" => hidden |= value == "true",
            "Categories" => categories = value.split(';')
                .map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
            _ => {}
        }
    }
    if is_application && !hidden { categories } else { Vec::new() }
}

/// Read file content from the cache on demand.
pub fn read_cached_file(entry: &DebFileEntry) -> Option<Vec<u8>> {
    entry.cache_path.as_ref().and_then(|p| std::fs::read(p).ok())
}

/// The absolute path a `.deb` entry is installed to.
///
/// Archive members are stored relative to the filesystem root, usually with a
/// leading `./`, which is not part of the installed path.
pub fn installed_path(entry_path: &str) -> PathBuf {
    PathBuf::from("/").join(entry_path.trim_start_matches("./"))
}

/// Render a byte count as a short human-readable string (`0 B`, `1.5 KB`, ...).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    // One decimal is enough to distinguish adjacent sizes; drop a trailing
    // `.0` so whole numbers stay tidy.
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded.fract()).abs() < f64::EPSILON {
        format!("{:.0} {}", rounded, UNITS[unit])
    } else {
        format!("{:.1} {}", rounded, UNITS[unit])
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn categorize(
    path: &str,
    data: &[u8],
    max_text: usize,
    max_image: usize,
) -> DebFileKind {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    const IMAGE_EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "tiff", "tif", "svg",
    ];
    if IMAGE_EXTS.contains(&ext.as_str()) {
        return if data.len() <= max_image { DebFileKind::Image } else { DebFileKind::Unsupported };
    }

    if is_image_magic(data) {
        return if data.len() <= max_image { DebFileKind::Image } else { DebFileKind::Unsupported };
    }

    if data.len() > max_text {
        return DebFileKind::Unsupported;
    }

    let sniff_len = data.len().min(8192);
    if sniff_len > 0 && data[..sniff_len].contains(&0u8) {
        return DebFileKind::Unsupported;
    }

    if std::str::from_utf8(data).is_ok() {
        DebFileKind::Text
    } else {
        DebFileKind::Unsupported
    }
}

fn is_known_binary_ext(path: &str) -> bool {
    const BINARY_EXTS: &[&str] = &[
        "so", "a", "o", "ko", "deb", "ar", "gz", "xz", "bz2", "zst", "lz4", "lzma",
        "zip", "tar", "whl", "egg", "pyc", "pyo", "class", "jar", "war",
        "exe", "dll", "dylib", "bin", "elf",
        "db", "sqlite", "sqlite3",
        "mp3", "mp4", "ogg", "wav", "flac", "aac", "mkv", "avi", "mov",
        "ttf", "otf", "woff", "woff2", "eot",
        "pdf",
    ];
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    BINARY_EXTS.contains(&ext.as_str())
}

fn is_image_magic(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    if data.starts_with(b"\x89PNG") { return true; }
    if data.starts_with(b"\xFF\xD8\xFF") { return true; }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") { return true; }
    if data.starts_with(b"BM") { return true; }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" { return true; }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview_archive() -> Vec<u8> {
        let mut archive = tar::Builder::new(Vec::new());
        for (path, data) in [
            ("usr/share/example/readme.txt", b"hello".as_slice()),
            ("usr/share/example/icon.png", b"\x89PNGexample".as_slice()),
            ("usr/lib/example.bin", b"\0binary".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, data).unwrap();
        }
        archive.into_inner().unwrap()
    }

    #[test]
    fn progress_previews_stay_readable_and_archive_padding_is_drained() {
        let mut bytes = preview_archive();
        bytes.extend_from_slice(&[0; 65536]);
        let mut reader = std::io::Cursor::new(bytes);
        let mut first_snapshot = None;
        let extracted = read_preview_archive(
            &mut reader,
            Arc::new(tempfile::tempdir().unwrap()),
            &mut |snapshot| {
                if first_snapshot.is_none() {
                    assert_eq!(snapshot.entries.len(), 1);
                    assert_eq!(read_cached_file(&snapshot.entries[0]).unwrap(), b"hello");
                    first_snapshot = Some(snapshot);
                }
                true
            },
        )
        .unwrap();
        assert_eq!(reader.position(), reader.get_ref().len() as u64);
        assert_eq!(extracted.entries.len(), 3);
        assert!(matches!(extracted.entries[1].kind, DebFileKind::Image));
        assert!(matches!(
            extracted.entries[2].kind,
            DebFileKind::Unsupported
        ));
        assert!(extracted.entries[2].cache_path.is_none());
        let cache_dir = extracted._temp_dir.path().to_owned();
        drop(extracted);
        let snapshot = first_snapshot.unwrap();
        assert_eq!(read_cached_file(&snapshot.entries[0]).unwrap(), b"hello");
        drop(snapshot);
        assert!(!cache_dir.exists());
    }

    #[test]
    fn cancelled_or_truncated_archives_do_not_succeed() {
        let bytes = preview_archive();
        let result = read_preview_archive(
            bytes.as_slice(),
            Arc::new(tempfile::tempdir().unwrap()),
            &mut |_| false,
        );
        assert!(result.err().unwrap().to_string().contains("cancelled"));
        // The first member declares five bytes, but only two are available.
        let result = read_preview_archive(
            &bytes[..514],
            Arc::new(tempfile::tempdir().unwrap()),
            &mut |_| true,
        );
        assert!(result.is_err());
    }

    #[test]
    fn failed_dpkg_does_not_look_like_an_empty_package() {
        let invalid = tempfile::NamedTempFile::new().unwrap();
        let result = extract_previewable_files(invalid.path(), |_| true);
        assert!(result.is_err());
    }

    #[test]
    fn categories_come_only_from_the_application_group() {
        let text = "# Categories=Wrong;\n[Desktop Entry]\nType=Application\nCategories = Utility;Development;;\n[Desktop Action NewWindow]\nCategories=Wrong;\n";
        assert_eq!(parse_desktop_categories(text), vec!["Utility", "Development"]);
        for text in [
            "[Desktop Entry]\nType=Application\nHidden=true\nCategories=Utility;\n",
            "[Desktop Entry]\nType=Application\nNoDisplay=true\nCategories=Utility;\n",
            "[Desktop Entry]\nType=Directory\nCategories=Utility;\n",
            "[Desktop Entry]\nType=Application\n",
        ] {
            assert!(parse_desktop_categories(text).is_empty());
        }
    }

    #[test]
    fn package_categories_use_cached_application_entries_and_deduplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let mut entries = Vec::new();
        for (i, (path, categories)) in [
            ("./usr/share/applications/chatgpt.desktop", "Utility;Development;"),
            ("./usr/local/share/applications/helper.desktop", "Utility;Network;"),
            ("./usr/share/doc/example.desktop", "Wrong;"),
        ].iter().enumerate() {
            let cached = tmp.path().join(format!("{i}.desktop"));
            std::fs::write(&cached, format!("[Desktop Entry]\nType=Application\nCategories={categories}\n")).unwrap();
            entries.push(DebFileEntry {
                path: path.to_string(), kind: DebFileKind::Text, size: 0, cache_path: Some(cached),
            });
        }
        let categories = desktop_categories(&entries);
        assert_eq!(categories, vec!["Utility", "Network", "Development"]);
        entries.reverse();
        assert_eq!(desktop_categories(&entries), categories);
        assert!(desktop_categories(&[]).is_empty());
    }

    #[test]
    fn entry_paths_map_to_their_installed_location() {
        assert_eq!(
            installed_path("./usr/share/applications/foo.desktop"),
            PathBuf::from("/usr/share/applications/foo.desktop")
        );
        assert_eq!(installed_path("./usr/bin/foo"), PathBuf::from("/usr/bin/foo"));
        // An already absolute member must not gain a second root.
        assert_eq!(installed_path("/usr/bin/foo"), PathBuf::from("/usr/bin/foo"));
    }

    #[test]
    fn sizes_render_with_a_readable_unit() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(10 * 1024 * 1024), "10 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3 GB");
    }
}
