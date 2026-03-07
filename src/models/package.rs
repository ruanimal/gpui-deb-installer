use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Parsed metadata from a .deb file (not persisted).
#[derive(Debug, Clone)]
pub struct DebInfo {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub description: String,
    pub installed_size_kb: u64,
    pub depends: Vec<String>,
    pub maintainer: String,
    pub section: Option<String>,
}

/// A record of an installed package (persisted to JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub description: String,
    pub install_date: DateTime<Utc>,
    pub source_file: Option<PathBuf>,
}
