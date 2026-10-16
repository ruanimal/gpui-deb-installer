use gpui_kit::{App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div, prelude::FluentBuilder};
use std::{sync::Arc, path::PathBuf};
use gpui_kit::component::{
    ActiveTheme,
    tab::{Tab, TabBar},
    text::TextView,
    v_flex,
};

use crate::views::{
    install::InstallView,
    packages::PackagesView,
    files_preview::FilesPreviewView,
};
use crate::i18n::tr;

pub struct AppView {
    active_tab: usize,
    install_view: Entity<InstallView>,
    packages_view: Entity<PackagesView>,
    files_preview_view: Entity<FilesPreviewView>,
    /// Markdown shown on the Dependencies tab.
    ///
    /// Derived from `install_view`, so it is refreshed by the observation below
    /// rather than recomputed on every render.
    deps_markdown: String,
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, initial_deb_path: Option<PathBuf>, initial_auto_close: bool, cx: &mut Context<Self>) -> Self {
        let packages_view = cx.new(|cx| PackagesView::new(window, cx));
        let install_view = cx.new(|cx| InstallView::new(window, initial_deb_path, initial_auto_close, cx));
        let files_preview_view = cx.new(|cx| FilesPreviewView::new(window, cx));

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

        // The Files tab parses the .deb, so let the Install tab ask it which
        // application the package ships.
        {
            let files_weak = files_preview_view.downgrade();
            install_view.update(cx, |view, _cx| {
                view.desktop_entries = Some(Arc::new(move |cx: &App| {
                    files_weak
                        .upgrade()
                        .map(|files| files.read(cx).desktop_entries())
                        .unwrap_or_default()
                }));
            });
        }

        // When the install_view loads a deb file, trigger the files preview.
        {
            let files_weak = files_preview_view.downgrade();
            install_view.update(cx, |view, _cx| {
                view.on_deb_loaded = Some(Arc::new(move |path: PathBuf, window: &mut Window, cx: &mut App| {
                    window.set_window_title(&format!("{} - {}", tr("app.name"), path.display()));
                    files_weak.update(cx, |fv, cx| {
                        fv.trigger_load(path, window, cx);
                    }).ok();
                }));
            });
        }

        // Reuse the Files tab's extraction to display desktop menu categories.
        {
            let install_weak = install_view.downgrade();
            files_preview_view.update(cx, |view, _| {
                view.on_categories_loaded = Some(Arc::new(move |path, categories, window, cx| {
                    install_weak.update(cx, |view, cx| {
                        view.set_desktop_categories(&path, categories, window, cx);
                    }).ok();
                }));
            });
        }

        // When the install_view resets, clear the files preview.
        {
            let files_weak = files_preview_view.downgrade();
            install_view.update(cx, |view, _cx| {
                view.on_reset = Some(Arc::new(move |window: &mut Window, cx: &mut App| {
                    files_weak.update(cx, |fv, cx| {
                        fv.reset(window, cx);
                    }).ok();
                }));
            });
        }

        // The Dependencies tab renders the install view's state, so refresh it
        // whenever that state changes — including asynchronous transitions such
        // as a .deb finishing its load while the tab is already open.
        let deps_markdown = install_view.read(cx).deps_markdown();
        let subscriptions = vec![cx.observe(&install_view, |this, install, cx| {
            let markdown = install.read(cx).deps_markdown();
            if markdown != this.deps_markdown {
                this.deps_markdown = markdown;
                cx.notify();
            }
        })];

        Self {
            active_tab: 0,
            install_view,
            packages_view,
            files_preview_view,
            deps_markdown,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let deps_md = self.deps_markdown.clone();

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
                    .child(Tab::new().label(tr("app.tab.install")))
                    .child(Tab::new().label(tr("app.tab.dependencies")))
                    .child(Tab::new().label(tr("app.tab.files")))
                    .child(Tab::new().label(tr("app.tab.installed"))),
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
                                TextView::markdown("deps-view", deps_md)
                                    .scrollable(true)
                                    .selectable(true),
                            ),
                        )
                    })
                    .when(self.active_tab == 2, |el| {
                        el.child(self.files_preview_view.clone())
                    })
                    .when(self.active_tab == 3, |el| {
                        el.child(self.packages_view.clone())
                    }),
            )
    }
}
