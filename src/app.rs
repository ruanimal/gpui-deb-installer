use gpui::{App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, prelude::FluentBuilder};
use std::sync::Arc;
use gpui_component::{
    ActiveTheme,
    tab::{Tab, TabBar},
    text::TextView,
    v_flex,
};

use crate::views::{install::InstallView, packages::PackagesView};

pub struct AppView {
    active_tab: usize,
    install_view: Entity<InstallView>,
    packages_view: Entity<PackagesView>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let packages_view = cx.new(|cx| PackagesView::new(window, cx));
        let install_view = cx.new(|cx| InstallView::new(window, cx));

        // When a package is installed/uninstalled, reload the Installed list.
        {
            let packages_weak = packages_view.downgrade();
            install_view.update(cx, |view, _cx| {
                view.on_installed = Some(Arc::new(move |_window: &mut Window, cx: &mut App| {
                    packages_weak
                        .update(cx, |packages, cx| {
                            packages.reload(cx);
                        })
                        .ok();
                }));
            });
        }

        // Wire PackagesView → InstallView for delegated uninstall.
        {
            let app_weak = cx.weak_entity();
            let install_weak = install_view.downgrade();
            packages_view.update(cx, |view, _cx| {
                view.install_view = Some(install_weak);
                view.on_tab_switch = Some(Arc::new(move |cx: &mut App| {
                    app_weak
                        .update(cx, |app, cx| {
                            app.active_tab = 0;
                            cx.notify();
                        })
                        .ok();
                }));
            });
        }

        Self {
            active_tab: 0,
            install_view,
            packages_view,
        }
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let deps_md = self.install_view.read(cx).deps_markdown();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                TabBar::new("main-tabs")
                    .selected_index(self.active_tab)
                    .on_click(cx.listener(|view, ix: &usize, _window, cx| {
                        view.active_tab = *ix;
                        cx.notify();
                    }))
                    .child(Tab::new().label("Install"))
                    .child(Tab::new().label("Dependencies"))
                    .child(Tab::new().label("Installed")),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .when(self.active_tab == 0, |el| {
                        el.child(self.install_view.clone())
                    })
                    .when(self.active_tab == 1, |el| {
                        el.child(
                            div().size_full().p_4().child(
                                TextView::markdown("deps-view", deps_md, window, cx)
                                    .scrollable(true)
                                    .selectable(true),
                            ),
                        )
                    })
                    .when(self.active_tab == 2, |el| {
                        el.child(self.packages_view.clone())
                    }),
            )
    }
}
