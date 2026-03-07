use chrono::Utc;
use gpui::{
    App, Context, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
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
    FileSelected {
        path: PathBuf,
        info: DebInfo,
        /// Version already installed on the system, if any.
        installed_version: Option<String>,
    },
    Installing { info: DebInfo, log: String },
    Uninstalling { pkg_name: String, log: String },
    Done { message: String, success: bool, log: String },
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub struct InstallView {
    state: InstallState,
    /// Called when a package is successfully installed/removed.
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
                                log: String::new(),
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

                    // Read deb info AND check system installation status in one background task.
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let info = deb_reader::read_deb_info(&path)?;
                            let installed = dpkg::installed_version(&info.name);
                            Ok::<_, anyhow::Error>((info, installed))
                        })
                        .await;

                    weak.update(cx, |view, cx| {
                        view.state = match result {
                            Ok((info, installed_version)) => {
                                let p = match &view.state {
                                    InstallState::LoadingInfo(p) => p.clone(),
                                    _ => PathBuf::new(),
                                };
                                InstallState::FileSelected { path: p, info, installed_version }
                            }
                            Err(e) => InstallState::Done {
                                message: format!("Failed to read .deb info: {}", e),
                                success: false,
                                log: String::new(),
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
            InstallState::FileSelected { path, info, .. } => (path.clone(), info.clone()),
            _ => return,
        };

        self.state = InstallState::Installing { info: info.clone(), log: String::new() };
        cx.notify();

        let on_installed = self.on_installed.clone();

        cx.spawn_in(window, async move |weak, cx| {
            let (log_tx, log_rx) = async_channel::unbounded::<String>();

            // Background: run command, stream lines to log_tx
            let path_bg = path.clone();
            let result_task = cx
                .background_executor()
                .spawn(async move { dpkg::install_deb_streaming(path_bg, log_tx) });

            // Foreground: receive lines and update UI in real-time
            while let Ok(line) = log_rx.recv().await {
                let l = line.clone();
                weak.update(cx, |view, cx| {
                    if let InstallState::Installing { ref mut log, .. } = view.state {
                        if !log.is_empty() {
                            log.push('\n');
                        }
                        log.push_str(&l);
                    }
                    cx.notify();
                })
                .ok();
            }

            // Command finished — collect log and result
            let final_log = weak
                .read_with(cx, |view, _| match &view.state {
                    InstallState::Installing { log, .. } => log.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();

            let result = result_task.await;

            match result {
                Ok(()) => {
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
                            log: final_log,
                        };
                        cx.notify();
                    })
                    .ok();

                    if let Some(cb) = on_installed {
                        cx.update(|window, cx| cb(window, cx)).ok();
                    }
                }
                Err(e) => {
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: "Installation failed.".to_string(),
                            success: false,
                            log: format!("{}\n{}", final_log, e),
                        };
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn uninstall_package(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pkg_name = match &self.state {
            InstallState::FileSelected { info, .. } => info.name.clone(),
            _ => return,
        };

        self.state = InstallState::Uninstalling { pkg_name: pkg_name.clone(), log: String::new() };
        cx.notify();

        let on_installed = self.on_installed.clone();

        cx.spawn_in(window, async move |weak, cx| {
            let (log_tx, log_rx) = async_channel::unbounded::<String>();

            let name_bg = pkg_name.clone();
            let result_task = cx
                .background_executor()
                .spawn(async move { dpkg::remove_package_streaming(name_bg, log_tx) });

            while let Ok(line) = log_rx.recv().await {
                let l = line.clone();
                weak.update(cx, |view, cx| {
                    if let InstallState::Uninstalling { ref mut log, .. } = view.state {
                        if !log.is_empty() {
                            log.push('\n');
                        }
                        log.push_str(&l);
                    }
                    cx.notify();
                })
                .ok();
            }

            let final_log = weak
                .read_with(cx, |view, _| match &view.state {
                    InstallState::Uninstalling { log, .. } => log.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();

            let result = result_task.await;

            match result {
                Ok(()) => {
                    let _ = db::remove_package(&pkg_name);
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: format!("Package '{}' uninstalled successfully.", pkg_name),
                            success: true,
                            log: final_log,
                        };
                        cx.notify();
                    })
                    .ok();
                    if let Some(cb) = on_installed {
                        cx.update(|window, cx| cb(window, cx)).ok();
                    }
                }
                Err(e) => {
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: "Uninstall failed.".to_string(),
                            success: false,
                            log: format!("{}\n{}", final_log, e),
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
                InstallState::FileSelected { path, info, installed_version } => {
                    render_file_selected(
                        path.to_string_lossy().to_string(),
                        info,
                        installed_version.clone(),
                        cx,
                    )
                }
                InstallState::Installing { info, log } => {
                    render_with_log(&format!("Installing '{}'…", info.name), log, None, cx)
                }
                InstallState::Uninstalling { pkg_name, log } => {
                    render_with_log(&format!("Uninstalling '{}'…", pkg_name), log, None, cx)
                }
                InstallState::Done { message, success, log } => {
                    render_with_log("", log, Some((*success, message.clone())), cx)
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
    installed_version: Option<String>,
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

    // Determine install status label and button label
    let (status_text, status_color, install_label) = match &installed_version {
        None => (
            "Not installed".to_string(),
            None, // use muted_foreground
            "Install",
        ),
        Some(v) if v == &info.version => (
            format!("Already installed (v{})", v),
            Some("warning"), // same version → warn
            "Reinstall",
        ),
        Some(v) => (
            format!("Installed: v{}  →  v{}", v, info.version),
            Some("info"),
            "Upgrade / Overwrite",
        ),
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
                // Package name + version row
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
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
                        )
                        // Installation status badge
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_sm()
                                .map(|el| match status_color {
                                    Some("warning") => el
                                        .bg(cx.theme().warning.opacity(0.15))
                                        .border_1()
                                        .border_color(cx.theme().warning)
                                        .text_color(cx.theme().warning),
                                    Some("info") => el
                                        .bg(cx.theme().info.opacity(0.15))
                                        .border_1()
                                        .border_color(cx.theme().info)
                                        .text_color(cx.theme().info),
                                    _ => el
                                        .bg(cx.theme().muted.opacity(0.3))
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .text_color(cx.theme().muted_foreground),
                                })
                                .child(status_text),
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
                        .label(install_label)
                        .on_click(cx.listener(|view, _ev, window, cx| {
                            view.install_package(window, cx);
                        })),
                )
                // Show uninstall button only when already installed
                .when(installed_version.is_some(), |el| {
                    el.child(
                        Button::new("uninstall-btn")
                            .danger()
                            .label("Uninstall")
                            .on_click(cx.listener(|view, _ev, window, cx| {
                                view.uninstall_package(window, cx);
                            })),
                    )
                })
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

/// Unified renderer for the in-progress and done states.
/// - `done = None`            → in-progress: shows title + live log, no Back button
/// - `done = Some((ok, msg))` → finished: swaps title for result banner, shows Back button
fn render_with_log(
    title: &str,
    log: &str,
    done: Option<(bool, String)>,
    cx: &mut Context<InstallView>,
) -> gpui::AnyElement {
    let log_text = if log.is_empty() {
        "Waiting for pkexec authentication…".to_string()
    } else {
        log.to_string()
    };
    let is_done = done.is_some();

    v_flex()
        .flex_1()
        .gap_3()
        // Header: result banner when done, plain title when in-progress
        .child(match done {
            Some((success, message)) => {
                let border_color = if success { cx.theme().success } else { cx.theme().danger };
                h_flex()
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(border_color)
                    .child(div().flex_1().child(message))
                    .into_any_element()
            }
            None => div()
                .text_color(cx.theme().foreground)
                .font_weight(gpui::FontWeight::BOLD)
                .child(title.to_string())
                .into_any_element(),
        })
        // Log panel (always visible)
        .child(
            v_flex()
                .flex_1()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .overflow_hidden()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .bg(cx.theme().tab_bar)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Output"),
                )
                .child(
                    div()
                        .id("install-log-scroll")
                        .flex_1()
                        .p_3()
                        .overflow_y_scroll()
                        .font_family("monospace")
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(log_text),
                ),
        )
        // Back button appears only after completion
        .when(is_done, |el| {
            el.child(
                Button::new("reset-btn")
                    .label("Back")
                    .on_click(cx.listener(|view, _ev, _window, cx| {
                        view.reset(cx);
                    })),
            )
        })
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
