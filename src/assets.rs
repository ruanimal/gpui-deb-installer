use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

// Compose the component bundle with only the additional icons this app uses.
icon_assets!(
    ExtraIcons,
    [
        Package,
        PackageOpen,
        Layers,
        LayoutList,
        ShieldCheck,
        Download,
        Trash,
        Terminal
    ]
);

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "brand/icon.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../icon.svg"))));
        }
        if let Some(asset) = ExtraIcons.load(path)? {
            return Ok(Some(asset));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        if "brand/icon.svg".starts_with(path) {
            paths.push("brand/icon.svg".into());
        }
        Ok(paths)
    }
}
