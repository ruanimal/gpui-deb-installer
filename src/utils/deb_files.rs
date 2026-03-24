use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
    /// Absolute path to the cached file on disk (None for Unsupported files
    /// that were not extracted).
    pub cache_path: Option<PathBuf>,
}

/// Result of extracting a .deb: the entry list plus the temp directory handle.
/// The temp directory is deleted when this struct is dropped.
pub struct ExtractedDeb {
    pub entries: Vec<DebFileEntry>,
    /// Keep alive — dropping this removes the temp directory.
    pub _temp_dir: TempDir,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Spawn `dpkg --fsys-tarfile <path>`, extract previewable files to a temp
/// directory, and return lightweight metadata entries.
///
/// This is a **blocking** function — call from a background executor.
pub fn extract_previewable_files(path: &Path) -> Result<ExtractedDeb> {
    let temp_dir = TempDir::new().context("failed to create temp directory")?;

    let mut child = Command::new("dpkg")
        .arg("--fsys-tarfile")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn dpkg --fsys-tarfile")?;

    let stdout = child.stdout.take().context("no stdout")?;

    let mut archive = tar::Archive::new(stdout);
    let mut entries_out: Vec<DebFileEntry> = Vec::new();

    for entry_result in archive.entries()? {
        let mut entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

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

        let file_size = header.size().unwrap_or(0) as usize;

        // Skip known binary extensions or oversized files
        if is_known_binary_ext(&raw_path) || file_size > MAX_IMAGE_BYTES {
            entries_out.push(DebFileEntry {
                path: raw_path,
                kind: DebFileKind::Unsupported,
                cache_path: None,
            });
            continue;
        }

        // Read content to classify
        let mut buf = Vec::with_capacity(file_size);
        entry.read_to_end(&mut buf).unwrap_or_default();

        let kind = categorize(&raw_path, &buf, MAX_TEXT_BYTES, MAX_IMAGE_BYTES);

        let cache_path = match &kind {
            DebFileKind::Unsupported => None,
            _ => {
                // Write to temp dir, preserving relative path structure
                let rel = raw_path.trim_start_matches("./");
                let dest = temp_dir.path().join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&dest, &buf).ok();
                Some(dest)
            }
        };

        entries_out.push(DebFileEntry {
            path: raw_path,
            kind,
            cache_path,
        });
    }

    let _ = child.wait();

    Ok(ExtractedDeb {
        entries: entries_out,
        _temp_dir: temp_dir,
    })
}

/// Read file content from the cache on demand.
pub fn read_cached_file(entry: &DebFileEntry) -> Option<Vec<u8>> {
    entry.cache_path.as_ref().and_then(|p| std::fs::read(p).ok())
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
