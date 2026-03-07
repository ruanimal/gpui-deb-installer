use chrono::Utc;
use gpui::{
    App, Context, IntoElement, ParentElement, PathPromptOptions, Render, Styled, Window,
    div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use std::{path::PathBuf, sync::Arc};

use crate::{
    models::{
        db,
        package::{DebInfo, InstalledPackage},
    },
    utils::{deb_reader, dpkg},
};

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

pub enum InstallState {
    Idle,
    LoadingInfo(PathBuf),
    FileSelected { path: PathBuf, info: DebInfo },
    Installing { info: DebInfo },
    Done { message: String, success: bool },
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub struct InstallView {
    state: InstallState,
    /// Called when a package is successfully installed.
    pub on_installed: Option<Arc<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl InstallView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            state: InstallState::Idle,
            on_installed: None,
        }
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    fn select_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |weak, cx| {
            let receiver = cx
                .update(|_window, cx| {
                    cx.prompt_for_paths(PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some("Select .deb package".into()),
                    })
                })
                .ok();

            let Some(receiver) = receiver else { return };

            match receiver.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let path = paths.into_iter().next().unwrap();

                    if path.extension().and_then(|e| e.to_str()) != Some("deb") {
                        weak.update(cx, |view, cx| {
                            view.state = InstallState::Done {
                                message: "Selected file is not a .deb package.".into(),
                                success: false,
                            };
                            cx.notify();
                        })
                        .ok();
                        return;
                    }

                    let path_for_state = path.clone();
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::LoadingInfo(path_for_state);
                        cx.notify();
                    })
                    .ok();

                    let result = cx
                        .background_executor()
                        .spawn(async move { deb_reader::read_deb_info(&path) })
                        .await;

                    weak.update(cx, |view, cx| {
                        view.state = match result {
                            Ok(info) => {
                                let p = match &view.state {
                                    InstallState::LoadingInfo(p) => p.clone(),
                                    _ => PathBuf::new(),
                                };
                                InstallState::FileSelected { path: p, info }
                            }
                            Err(e) => InstallState::Done {
                                message: format!("Failed to read .deb info: {}", e),
                                success: false,
                            },
                        };
                        cx.notify();
                    })
                    .ok();
                }
                _ => {} // cancelled → stay Idle
            }
        })
        .detach();
    }

    fn install_package(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (path, info) = match &self.state {
            InstallState::FileSelected { path, info } => (path.clone(), info.clone()),
            _ => return,
        };

        self.state = InstallState::Installing { info: info.clone() };
        cx.notify();

        // Clone the callback so we can call it from the async closure
        let on_installed = self.on_installed.clone();

        cx.spawn_in(window, async move |weak, cx| {
            let path_bg = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { dpkg::install_deb(&path_bg) })
                .await;

            match result {
                Ok(_output) => {
                    let pkg = InstalledPackage {
                        name: info.name.clone(),
                        version: info.version.clone(),
                        architecture: info.architecture.clone(),
                        description: info.description.lines().next().unwrap_or("").to_string(),
                        install_date: Utc::now(),
                        source_file: Some(path),
                    };
                    let _ = db::add_package(pkg);

                    let pkg_name = info.name.clone();
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: format!("Package '{}' installed successfully.", pkg_name),
                            success: true,
                        };
                        cx.notify();
                    })
                    .ok();

                    // Fire the reload callback
                    if let Some(cb) = on_installed {
                        cx.update(|window, cx| cb(window, cx)).ok();
                    }
                }
                Err(e) => {
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: format!("Installation failed: {}", e),
                            success: false,
                        };
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.state = InstallState::Idle;
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for InstallView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .child(match &self.state {
                InstallState::Idle => render_idle(cx),
                InstallState::LoadingInfo(path) => {
                    render_loading_info(path.to_string_lossy().to_string())
                }
                InstallState::FileSelected { path, info } => {
                    render_file_selected(path.to_string_lossy().to_string(), info, cx)
                }
                InstallState::Installing { info } => render_installing(&info.name),
                InstallState::Done { message, success } => {
                    render_done(message.clone(), *success, cx)
                }
            })
    }
}

// ---------------------------------------------------------------------------
// Sub-renderers
// ---------------------------------------------------------------------------

fn render_idle(cx: &mut Context<InstallView>) -> gpui::AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .child(
            div()
                .w(gpui::px(340.))
                .h(gpui::px(180.))
                .border_2()
                .border_dashed()
                .border_color(cx.theme().border)
                .rounded_lg()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Click the button below to select a .deb file"),
        )
        .child(
            Button::new("select-file")
                .primary()
                .label("Select .deb File")
                .on_click(cx.listener(|view, _ev, window, cx| {
                    view.select_file(window, cx);
                })),
        )
        .into_any_element()
}

fn render_loading_info(path: String) -> gpui::AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(div().child("Reading package info…"))
        .child(div().text_sm().child(path))
        .into_any_element()
}

fn render_file_selected(
    path: String,
    info: &DebInfo,
    cx: &mut Context<InstallView>,
) -> gpui::AnyElement {
    let size_str = if info.installed_size_kb > 0 {
        format!("{} KB", info.installed_size_kb)
    } else {
        "unknown".to_string()
    };
    let deps_str = if info.depends.is_empty() {
        "none".to_string()
    } else {
        info.depends.join(", ")
    };

    v_flex()
        .flex_1()
        .gap_3()
        .child(
            v_flex()
                .p_4()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().list)
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(info.name.clone()),
                        )
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("v{} ({})", info.version, info.architecture)),
                        ),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .text_sm()
                        .child(path),
                )
                .child(div().h(gpui::px(1.)).bg(cx.theme().border))
                .child(info_row(
                    "Description",
                    info.description.lines().next().unwrap_or("").to_string(),
                    cx,
                ))
                .child(info_row("Maintainer", info.maintainer.clone(), cx))
                .child(info_row("Installed size", size_str, cx))
                .when_some(info.section.clone(), |el, sec| {
                    el.child(info_row("Section", sec, cx))
                })
                .child(info_row("Depends", deps_str, cx)),
        )
        .child(
            h_flex()
                .gap_3()
                .child(
                    Button::new("install-btn")
                        .primary()
                        .label("Install")
                        .on_click(cx.listener(|view, _ev, window, cx| {
                            view.install_package(window, cx);
                        })),
                )
                .child(
                    Button::new("cancel-btn")
                        .label("Cancel")
                        .on_click(cx.listener(|view, _ev, _window, cx| {
                            view.reset(cx);
                        })),
                ),
        )
        .into_any_element()
}

fn render_installing(pkg_name: &str) -> gpui::AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(div().child(format!("Installing '{}'…", pkg_name)))
        .child(div().text_sm().child("Waiting for pkexec authentication…"))
        .into_any_element()
}

fn render_done(message: String, success: bool, cx: &mut Context<InstallView>) -> gpui::AnyElement {
    let border_color = if success {
        cx.theme().success
    } else {
        cx.theme().danger
    };

    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .child(
            div()
                .px_4()
                .py_3()
                .rounded_lg()
                .border_1()
                .border_color(border_color)
                .child(message),
        )
        .child(
            Button::new("reset-btn")
                .label("Install Another")
                .on_click(cx.listener(|view, _ev, _window, cx| {
                    view.reset(cx);
                })),
        )
        .into_any_element()
}

fn info_row(label: &str, value: String, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(
            div()
                .w(gpui::px(130.))
                .text_color(cx.theme().muted_foreground)
                .flex_shrink_0()
                .child(label.to_string()),
        )
        .child(div().flex_1().child(value))
}
