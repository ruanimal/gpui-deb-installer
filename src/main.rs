mod app;
mod i18n;
mod models;
mod utils;
mod views;

use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;
use std::path::PathBuf;

use app::AppView;
use i18n::tr;
use utils::dpkg;

fn main() {
    // Parse command line arguments for deb file path
    let deb_path = std::env::args().nth(1).and_then(|arg| {
        let path = PathBuf::from(&arg);
        if path.extension().and_then(|e| e.to_str()) == Some("deb") {
            Some(path)
        } else {
            None
        }
    });

    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);

            if !dpkg::check_pkexec() {
                eprintln!(
                    "{}",
                    tr(
                        "Warning: pkexec not found. Package installation/removal will fail.\nPlease install policykit-1 (e.g. sudo apt install policykit-1).",
                        "警告：未找到 pkexec，安装/卸载功能将无法使用。\n请先安装 policykit-1（例如：sudo apt install policykit-1）。",
                    )
                );
            }

            let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(tr("Deb Installer", "Deb 安装器").into()),
                        ..Default::default()
                    }),
                    focus: true,
                    show: true,
                    app_id: Some("gpui-deb-installer".to_string()),
                    ..Default::default()
                },
                |window, cx| {
                    let app_view = cx.new(|cx| AppView::new(window, deb_path.clone(), cx));
                    let app_view: gpui::AnyView = app_view.into();
                    cx.new(|cx| Root::new(app_view, window, cx))
                },
            )
            .unwrap();

            cx.activate(true);
        });
}
