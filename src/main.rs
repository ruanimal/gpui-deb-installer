mod app;
mod models;
mod utils;
mod views;

use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;
use gpui_component_assets::Assets;

use app::AppView;
use utils::dpkg;

fn main() {
    Application::new()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);

            if !dpkg::check_pkexec() {
                eprintln!(
                    "Warning: pkexec not found. Package installation/removal will fail.\n\
                     Please install policykit-1 (e.g. sudo apt install policykit-1)."
                );
            }

            let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: true,
                    show: true,
                    app_id: Some("gpui-deb-installer".to_string()),
                    ..Default::default()
                },
                |window, cx| {
                    let app_view = cx.new(|cx| AppView::new(window, cx));
                    let app_view: gpui::AnyView = app_view.into();
                    cx.new(|cx| Root::new(app_view, window, cx))
                },
            )
            .unwrap();

            cx.activate(true);
        });
}
