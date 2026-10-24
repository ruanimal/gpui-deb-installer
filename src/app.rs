use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    h_flex,
    text::TextView,
    v_flex,
};
use gpui_kit::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use std::{path::PathBuf, sync::Arc};

use crate::i18n::tr;
use crate::views::ui;
use crate::views::{files_preview::FilesPreviewView, install::InstallView, packages::PackagesView};
use gpui_kit::assets::IconName;

pub struct AppView {
    active_tab: usize,
    /// Tab that opened the uninstall log, consumed when the user returns.
    uninstall_return_tab: Option<usize>,
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
    pub fn new(
        window: &mut Window,
        initial_deb_path: Option<PathBuf>,
        initial_auto_close: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let packages_view = cx.new(|cx| PackagesView::new(window, cx));
        let install_view =
            cx.new(|cx| InstallView::new(window, initial_deb_path, initial_auto_close, cx));
        let files_preview_view = cx.new(|cx| FilesPreviewView::new(window, cx));

        // Reload after successful installs and all removal attempts.
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
                            app.uninstall_return_tab = Some(app.active_tab);
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
                view.on_deb_loaded = Some(Arc::new(
                    move |path: PathBuf, window: &mut Window, cx: &mut App| {
                        window.set_window_title(&format!(
                            "{} - {}",
                            tr("app.name"),
                            path.display()
                        ));
                        files_weak
                            .update(cx, |fv, cx| {
                                fv.trigger_load(path, window, cx);
                            })
                            .ok();
                    },
                ));
            });
        }

        {
            let app_weak = cx.weak_entity();
            files_preview_view.update(cx, |view, _| {
                view.on_select_package = Some(Arc::new(move |cx| {
                    app_weak
                        .update(cx, |app, cx| {
                            app.active_tab = 0;
                            cx.notify();
                        })
                        .ok();
                }));
            });
        }

        // Reuse the Files tab's extraction to display desktop menu categories.
        {
            let install_weak = install_view.downgrade();
            files_preview_view.update(cx, |view, _| {
                view.on_categories_loaded = Some(Arc::new(move |path, categories, window, cx| {
                    install_weak
                        .update(cx, |view, cx| {
                            view.set_desktop_categories(&path, categories, window, cx);
                        })
                        .ok();
                }));
            });
        }

        // Clear the preview and return to the tab that initiated uninstall.
        {
            let files_weak = files_preview_view.downgrade();
            let app_weak = cx.weak_entity();
            install_view.update(cx, |view, _cx| {
                view.on_reset = Some(Arc::new(move |window: &mut Window, cx: &mut App| {
                    window.set_window_title(&tr("app.name"));
                    files_weak
                        .update(cx, |fv, cx| {
                            fv.reset(window, cx);
                        })
                        .ok();
                    app_weak
                        .update(cx, |app, cx| {
                            if let Some(tab) = app.uninstall_return_tab.take() {
                                app.active_tab = tab;
                                cx.notify();
                            }
                        })
                        .ok();
                }));
            });
        }

        // The Dependencies tab renders the install view's state, so refresh it
        // whenever that state changes — including asynchronous transitions such
        // as a .deb finishing its load while the tab is already open.
        let deps_markdown = install_view.read(cx).deps_markdown();
        let mut subscriptions = vec![cx.observe(&install_view, |this, install, cx| {
            let markdown = install.read(cx).deps_markdown();
            if markdown != this.deps_markdown {
                this.deps_markdown = markdown;
            }
            cx.notify();
        })];

        subscriptions.push(window.observe_window_appearance(|window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            ui::apply_theme(cx);
        }));

        Self {
            active_tab: 0,
            uninstall_return_tab: None,
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
        let busy = self.install_view.read(cx).is_busy();
        let active_tab = self.active_tab;
        let primary = cx.theme().primary;
        let foreground = cx.theme().tab_foreground;
        let muted = cx.theme().muted_foreground;
        let tabs = [
            (IconName::Package, tr("app.tab.install")),
            (IconName::Layers, tr("app.tab.dependencies")),
            (IconName::FolderOpen, tr("app.tab.files")),
            (IconName::LayoutList, tr("app.tab.installed")),
        ];

        v_flex()
            .size_full()
            .font_family(cx.theme().font_family.clone())
            .text_color(cx.theme().foreground)
            .bg(cx.theme().muted.opacity(0.25))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .gap_3()
                    .bg(cx.theme().background)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(ui::brand_icon())
                    .child(
                        // Base tabs keep the active indicator independent of separator borders.
                        Tabs::new("main-tabs")
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .gap_4()
                            .children(tabs.into_iter().enumerate().map(
                                |(index, (icon, label))| {
                                    let selected = active_tab == index;
                                    let disabled = busy && index == 3;
                                    Tab::new(("main-tab", index))
                                        .selected(selected)
                                        .disabled(disabled)
                                        .accessibility_label(label.clone())
                                        .set_position(index + 1, 4)
                                        .relative()
                                        .h(px(36.))
                                        .gap_1()
                                        .text_sm()
                                        .text_color(if disabled {
                                            muted
                                        } else if selected {
                                            primary
                                        } else {
                                            foreground
                                        })
                                        .when(!disabled, |el| {
                                            el.cursor_pointer()
                                                .hover(|style| style.text_color(primary))
                                        })
                                        .child(Icon::new(icon).size_4())
                                        .child(label)
                                        .when(selected, |el| {
                                            el.child(
                                                div()
                                                    .absolute()
                                                    .left_0()
                                                    .right_0()
                                                    .bottom_0()
                                                    .h(px(2.))
                                                    .bg(primary),
                                            )
                                        })
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            if view.active_tab != index {
                                                view.install_view.update(cx, |install, _| {
                                                    install.cancel_auto_close();
                                                });
                                            }
                                            view.active_tab = index;
                                            cx.notify();
                                        }))
                                },
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("toggle-theme")
                            .ghost()
                            .small()
                            .icon(if cx.theme().is_dark() {
                                IconName::Sun
                            } else {
                                IconName::Moon
                            })
                            .tooltip(tr("app.toggle_theme"))
                            .on_click(cx.listener(|_, _, window, cx| {
                                let mode = if cx.theme().is_dark() {
                                    ThemeMode::Light
                                } else {
                                    ThemeMode::Dark
                                };
                                Theme::change(mode, Some(window), cx);
                                ui::apply_theme(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(self.active_tab == 0, |el| {
                        el.child(self.install_view.clone())
                    })
                    .when(self.active_tab == 1, |el| {
                        el.child(
                            v_flex().size_full().p_3().child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .p_4()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().background)
                                    .child(
                                        TextView::markdown("deps-view", deps_md)
                                            .scrollable(true)
                                            .selectable(true),
                                    ),
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
