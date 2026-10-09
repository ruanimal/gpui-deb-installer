use chrono::{DateTime, Local};

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

/// A package installed on the system, as reported by `apt list`.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub architecture: String,
    /// When the package was first installed, if the dpkg logs still know.
    pub install_date: Option<DateTime<Local>>,
    /// When it was last installed or upgraded, if known.
    pub update_date: Option<DateTime<Local>>,
}
