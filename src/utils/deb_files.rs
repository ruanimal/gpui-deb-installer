use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of content we extracted (or decided we can't preview).
#[derive(Debug, Clone)]
pub enum DebFileKind {
    /// UTF-8 text file (source code, config, plain text, etc.)
    Text(String),
    /// Raw image bytes (PNG, JPEG, GIF, SVG, …)
    Image(Vec<u8>),
    /// Everything else – not previewable
    Unsupported,
}

/// A single entry from inside the .deb package.
#[derive(Debug, Clone)]
pub struct DebFileEntry {
    /// Full path as stored in the tar archive (e.g. `./usr/bin/foo`)
    pub path: String,
    pub kind: DebFileKind,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Spawn `dpkg --fsys-tarfile <path>`, pipe its output through the `tar`
/// crate in a single pass, and return previewable file entries.
///
/// This is a **blocking** function and should be called from a background
/// executor (e.g. `cx.background_executor().spawn(...)`).
pub fn extract_previewable_files(path: &Path) -> Result<Vec<DebFileEntry>> {
    // Launch dpkg --fsys-tarfile and capture its stdout as a tar stream.
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

        // Only regular files
        if header.entry_type() != tar::EntryType::Regular {
            // Still need to consume the entry to advance the stream
            continue;
        }

        let raw_path = match entry.path() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(_) => continue,
        };

        // Read the first 8 KB to determine type; then read the rest if needed.
        const SNIFF_BYTES: usize = 8192;
        const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024; // 4 MB limit for text
        const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024; // 16 MB limit for images

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .unwrap_or_default();

        let kind = categorize(&raw_path, &buf, SNIFF_BYTES, MAX_TEXT_BYTES, MAX_IMAGE_BYTES);

        entries_out.push(DebFileEntry {
            path: raw_path,
            kind,
        });
    }

    // Wait for the child process (best-effort – don't fail on non-zero exit)
    let _ = child.wait();

    Ok(entries_out)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn categorize(
    path: &str,
    data: &[u8],
    _sniff: usize,
    max_text: usize,
    max_image: usize,
) -> DebFileKind {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // ---- Image by extension ------------------------------------------------
    const IMAGE_EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "tiff", "tif", "svg",
    ];
    if IMAGE_EXTS.contains(&ext.as_str()) {
        if data.len() <= max_image {
            return DebFileKind::Image(data.to_vec());
        } else {
            return DebFileKind::Unsupported;
        }
    }

    // ---- Image by magic bytes ----------------------------------------------
    if is_image_magic(data) {
        if data.len() <= max_image {
            return DebFileKind::Image(data.to_vec());
        } else {
            return DebFileKind::Unsupported;
        }
    }

    // ---- Known binary extensions → Unsupported ----------------------------
    const BINARY_EXTS: &[&str] = &[
        "so", "a", "o", "ko", "deb", "ar", "gz", "xz", "bz2", "zst", "lz4", "lzma",
        "zip", "tar", "whl", "egg", "pyc", "pyo", "class", "jar", "war",
        "exe", "dll", "dylib", "bin", "elf",
        "db", "sqlite", "sqlite3",
        "mp3", "mp4", "ogg", "wav", "flac", "aac", "mkv", "avi", "mov",
        "ttf", "otf", "woff", "woff2", "eot",
        "pdf",
    ];
    if BINARY_EXTS.contains(&ext.as_str()) {
        return DebFileKind::Unsupported;
    }

    // ---- Too large for text preview ----------------------------------------
    if data.len() > max_text {
        return DebFileKind::Unsupported;
    }

    // ---- Sniff for binary content: null bytes are a strong indicator --------
    let sniff = &data[..data.len().min(8192)];
    if sniff.contains(&0u8) {
        return DebFileKind::Unsupported;
    }

    // ---- Try to decode as UTF-8 -------------------------------------------
    match String::from_utf8(data.to_vec()) {
        Ok(text) => DebFileKind::Text(text),
        Err(_) => DebFileKind::Unsupported,
    }
}

fn is_image_magic(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    // PNG
    if data.starts_with(b"\x89PNG") {
        return true;
    }
    // JPEG
    if data.starts_with(b"\xFF\xD8\xFF") {
        return true;
    }
    // GIF
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return true;
    }
    // BMP
    if data.starts_with(b"BM") {
        return true;
    }
    // WebP: "RIFF????WEBP"
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return true;
    }
    false
}
