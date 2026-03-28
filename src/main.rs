mod app;
mod i18n;
mod models;
mod utils;
mod views;

rust_i18n::i18n!("locales", fallback = "en");

use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;
use std::path::PathBuf;

use app::AppView;
use i18n::{init_locale, tr};
use utils::dpkg;
use crate::models::config::AppConfig;

fn main() {
    init_locale();

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
                eprintln!("{}", tr("warning.pkexec_missing"));
            }

            let config = AppConfig::load();
            let bounds = Bounds::centered(None, size(px(config.width), px(config.height)), cx);
            let initial_auto_close = config.auto_close;

            let _window = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(tr("app.name").into()),
                        ..Default::default()
                    }),
                    focus: true,
                    show: true,
                    app_id: Some("gpui-deb-installer".to_string()),
                    ..Default::default()
                },
                |window, cx| {
                    // Save window size + settings when the user closes the window
                    window.on_window_should_close(cx, |window, _cx| {
                        let b = window.bounds();
                        // auto_close is saved immediately on toggle; only update size here
                        let mut cfg = AppConfig::load();
                        cfg.width = f32::from(b.size.width);
                        cfg.height = f32::from(b.size.height);
                        cfg.save();
                        true
                    });

                    let app_view = cx.new(|cx| AppView::new(window, deb_path.clone(), initial_auto_close, cx));
                    let app_view: gpui::AnyView = app_view.into();
                    cx.new(|cx| Root::new(app_view, window, cx))
                },
            )
            .unwrap();

            cx.activate(true);
        });
}
