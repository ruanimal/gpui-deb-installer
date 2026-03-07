use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use crate::models::package::DebInfo;

/// Reads metadata from a .deb file using `dpkg-deb --info`.
pub fn read_deb_info(path: &Path) -> Result<DebInfo> {
    let output = Command::new("dpkg-deb")
        .arg("--info")
        .arg(path)
        .output()
        .context("failed to run dpkg-deb --info (is dpkg-deb installed?)")?;

    if !output.status.success() {
        bail!(
            "dpkg-deb --info failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let text = String::from_utf8_lossy(&output.stdout);
    parse_control_fields(&text)
}

fn parse_control_fields(text: &str) -> Result<DebInfo> {
    let mut fields: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut current_key: Option<String> = None;
    let mut current_value = String::new();

    for line in text.lines() {
        // Continuation line
        if line.starts_with(' ') || line.starts_with('\t') {
            if current_key.is_some() {
                let trimmed = line.trim();
                if trimmed != "." {
                    if !current_value.is_empty() {
                        current_value.push('\n');
                    }
                    current_value.push_str(trimmed);
                }
            }
        } else if let Some(colon_pos) = line.find(':') {
            // Save previous field
            if let Some(key) = current_key.take() {
                fields.insert(key, current_value.trim().to_string());
            }
            current_key = Some(line[..colon_pos].trim().to_lowercase());
            current_value = line[colon_pos + 1..].trim().to_string();
        }
    }
    // Save last field
    if let Some(key) = current_key.take() {
        fields.insert(key, current_value.trim().to_string());
    }

    let name = fields
        .get("package")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let version = fields
        .get("version")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let architecture = fields
        .get("architecture")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let description = fields
        .get("description")
        .cloned()
        .unwrap_or_else(|| String::new());
    let maintainer = fields
        .get("maintainer")
        .cloned()
        .unwrap_or_else(|| String::new());
    let section = fields.get("section").cloned();

    let installed_size_kb = fields
        .get("installed-size")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let depends = fields
        .get("depends")
        .map(|s| {
            s.split(',')
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Ok(DebInfo {
        name,
        version,
        architecture,
        description,
        installed_size_kb,
        depends,
        maintainer,
        section,
    })
}
