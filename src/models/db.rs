use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::package::InstalledPackage;

fn db_path() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .context("cannot determine local data directory")?
        .join("gpui-deb-installer");
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("installed.json"))
}

pub fn load_packages() -> Result<Vec<InstalledPackage>> {
    let path = db_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let packages: Vec<InstalledPackage> = serde_json::from_str(&content)?;
    Ok(packages)
}

pub fn save_packages(packages: &[InstalledPackage]) -> Result<()> {
    let path = db_path()?;
    let content = serde_json::to_string_pretty(packages)?;
    fs::write(&path, content)?;
    Ok(())
}

pub fn add_package(pkg: InstalledPackage) -> Result<()> {
    let mut packages = load_packages()?;
    // Replace if already exists
    packages.retain(|p| p.name != pkg.name);
    packages.push(pkg);
    save_packages(&packages)
}

pub fn remove_package(name: &str) -> Result<()> {
    let mut packages = load_packages()?;
    packages.retain(|p| p.name != name);
    save_packages(&packages)
}
