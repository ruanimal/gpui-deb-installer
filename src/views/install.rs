use super::ui;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext, AsyncWindowContext, ClipboardItem, Context, Entity, ExternalPaths,
    InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement, PathPromptOptions,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, VisualContext,
    WeakEntity, Window, div, list, prelude::FluentBuilder, px,
};
use std::{path::PathBuf, sync::Arc};

use crate::{
    i18n::tr,
    models::{config::AppConfig, package::DebInfo},
    utils::{app_launch, app_launch::AppTarget, deb_reader, dpkg},
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
        /// Whether uninstalling this package should also purge its configuration.
        purge: bool,
    },
    Installing {
        info: DebInfo,
        log: Vec<String>,
    },
    Uninstalling {
        pkg_names: Vec<String>,
        log: Vec<String>,
    },
    Done {
        message: String,
        success: bool,
        log: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Pre-created InputState entities for every field on the info page.
/// These are always alive and simply get updated when a new .deb is loaded.
struct InfoInputs {
    name: Entity<InputState>,
    version: Entity<InputState>,
    path: Entity<InputState>,
    description: Entity<InputState>,
    maintainer: Entity<InputState>,
    size: Entity<InputState>,
    section: Entity<InputState>,
    categories: Entity<InputState>,
}

pub struct InstallView {
    state: InstallState,
    info_inputs: InfoInputs,
    auto_close: bool,
    /// Invalidates pending automatic exits without changing the saved preference.
    auto_close_generation: u64,
    selecting_file: bool,
    log_copied: bool,
    log_list_state: ListState,
    /// Application exposed by the package that was just installed, if any.
    ///
    /// Only set after a successful install, so the Done screen can offer to
    /// open it.
    open_target: Option<AppTarget>,
    /// Supplies the `.desktop` entries of the loaded .deb, as installed paths.
    ///
    /// The Files tab already parses the archive, so the Done screen reuses that
    /// rather than querying the package database again.
    pub desktop_entries: Option<Arc<dyn Fn(&App) -> Vec<PathBuf> + 'static>>,
    _subscriptions: Vec<Subscription>,
    /// Called after a successful install or any removal attempt (which may partially succeed).
    pub on_installed: Option<Arc<dyn Fn(&mut Window, &mut App) + 'static>>,
    /// Called when a .deb file has been successfully loaded (path provided).
    pub on_deb_loaded: Option<Arc<dyn Fn(PathBuf, &mut Window, &mut App) + 'static>>,
    /// Called when the view resets to Idle (e.g. Back button or Cancel).
    pub on_reset: Option<Arc<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl InstallView {
    /// Updates menu categories only for the package still being displayed.
    pub fn set_desktop_categories(
        &mut self,
        path: &std::path::Path,
        categories: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(&self.state, InstallState::FileSelected { path: current, .. } if current == path)
        {
            return;
        }
        let value = if categories.is_empty() {
            tr("install.unknown")
        } else {
            categories.join("; ")
        };
        self.info_inputs
            .categories
            .update(cx, |s, cx| s.set_value(value, window, cx));
    }

    pub fn new(
        window: &mut Window,
        initial_deb_path: Option<PathBuf>,
        initial_auto_close: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = InfoInputs {
            name: cx.new(|cx| InputState::new(window, cx)),
            version: cx.new(|cx| InputState::new(window, cx)),
            path: cx.new(|cx| InputState::new(window, cx)),
            description: cx.new(|cx| InputState::new(window, cx)),
            maintainer: cx.new(|cx| InputState::new(window, cx)),
            size: cx.new(|cx| InputState::new(window, cx)),
            section: cx.new(|cx| InputState::new(window, cx)),
            categories: cx.new(|cx| InputState::new(window, cx)),
        };

        // When any one input gains focus, unselect all the others.
        let all: Vec<Entity<InputState>> = vec![
            inputs.name.clone(),
            inputs.version.clone(),
            inputs.path.clone(),
            inputs.description.clone(),
            inputs.maintainer.clone(),
            inputs.size.clone(),
            inputs.section.clone(),
            inputs.categories.clone(),
        ];
        let mut subscriptions = Vec::new();
        for (i, focused) in all.iter().enumerate() {
            let others: Vec<Entity<InputState>> = all
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, e)| e.clone())
                .collect();
            subscriptions.push(cx.subscribe_in(
                focused,
                window,
                move |_view, _emitter, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Focus) {
                        for other in &others {
                            cx.update_entity(other, |s, cx| s.unselect(window, cx));
                        }
                    }
                },
            ));
        }

        let mut view = Self {
            state: InstallState::Idle,
            info_inputs: inputs,
            auto_close: initial_auto_close,
            auto_close_generation: 0,
            selecting_file: false,
            log_copied: false,
            log_list_state: ListState::new(0, ListAlignment::Top, px(1000.)),
            open_target: None,
            desktop_entries: None,
            _subscriptions: subscriptions,
            on_installed: None,
            on_deb_loaded: None,
            on_reset: None,
        };

        // If an initial deb path was provided, schedule loading it
        if let Some(path) = initial_deb_path {
            view.load_deb_file(path, window, cx);
        }

        view
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Cancels automatic closing for the current operation, including its countdown.
    pub fn cancel_auto_close(&mut self) {
        self.auto_close_generation = self.auto_close_generation.wrapping_add(1);
    }

    pub fn is_busy(&self) -> bool {
        self.selecting_file
            || matches!(
                self.state,
                InstallState::LoadingInfo(_)
                    | InstallState::Installing { .. }
                    | InstallState::Uninstalling { .. }
            )
    }

    fn load_deb_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        self.cancel_auto_close();
        self.log_copied = false;
        self.open_target = None;
        let path_str = path.to_string_lossy().to_string();
        self.state = InstallState::LoadingInfo(path.clone());
        cx.notify();

        cx.spawn_in(window, async move |weak, cx| {
            load_deb_async(path, path_str, weak, cx).await;
        })
        .detach();
    }

    fn select_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy() || self.selecting_file {
            return;
        }
        self.selecting_file = true;
        cx.notify();
        cx.spawn_in(window, async move |weak, cx| {
            let receiver = cx
                .update(|_window, cx| {
                    cx.prompt_for_paths(PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some(tr("install.select_deb_package").into()),
                    })
                })
                .ok();

            let result = if let Some(receiver) = receiver {
                Some(receiver.await)
            } else {
                None
            };
            weak.update(cx, |view, cx| {
                view.selecting_file = false;
                cx.notify();
            })
            .ok();

            match result {
                Some(Ok(Ok(Some(paths)))) if !paths.is_empty() => {
                    let path = paths.into_iter().next().unwrap();

                    if !is_deb_path(&path) {
                        weak.update(cx, |view, cx| {
                            view.state = InstallState::Done {
                                message: tr("install.selected_not_deb").into(),
                                success: false,
                                log: Vec::new(),
                            };
                            cx.notify();
                        })
                        .ok();
                        return;
                    }

                    let path_str = path.to_string_lossy().to_string();
                    weak.update(cx, |view, cx| {
                        view.log_copied = false;
                        view.open_target = None;
                        view.state = InstallState::LoadingInfo(path.clone());
                        cx.notify();
                    })
                    .ok();

                    load_deb_async(path, path_str, weak, cx).await;
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

        self.cancel_auto_close();
        let auto_close_generation = self.auto_close_generation;
        self.state = InstallState::Installing {
            info: info.clone(),
            log: Vec::new(),
        };
        self.open_target = None;
        self.log_list_state.reset(0);
        cx.notify();

        let on_installed = self.on_installed.clone();
        let auto_close = self.auto_close;

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
                        log.push(l);
                        view.log_copied = false;
                        let last = log.len().saturating_sub(1);
                        view.log_list_state.splice(last..last, 1);
                        view.log_list_state.scroll_to_reveal_item(last);
                    }
                    cx.notify();
                })
                .ok();
            }

            // Command finished — collect log and result
            let final_log = weak
                .read_with(cx, |view, _| match &view.state {
                    InstallState::Installing { log, .. } => log.clone(),
                    _ => Vec::new(),
                })
                .unwrap_or_default();

            let result = result_task.await;

            match result {
                Ok(()) => {
                    // The Installed tab re-reads `apt list` afterwards, so there
                    // is no local record to update here.
                    let pkg_name = info.name.clone();
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: rust_i18n::t!(
                                "install.package_installed",
                                pkg = pkg_name.as_str()
                            )
                            .to_string(),
                            success: true,
                            log: final_log,
                        };
                        cx.notify();
                    })
                    .ok();
                    if let Some(cb) = on_installed {
                        cx.update(|window, cx| cb(window, cx)).ok();
                    }

                    // Look for the application the package ships, so the Done
                    // screen can offer to open it. The Files tab has already
                    // parsed the archive, so this stays in memory.
                    let installed_pkg = info.name.clone();
                    let open_target = weak
                        .read_with(cx, |view, cx| {
                            let entries = view
                                .desktop_entries
                                .as_ref()
                                .map(|entries| entries(cx))
                                .unwrap_or_default();
                            app_launch::select_app_target(&entries, &installed_pkg)
                        })
                        .ok()
                        .flatten();
                    weak.update(cx, |view, cx| {
                        view.open_target = open_target;
                        cx.notify();
                    })
                    .ok();

                    if auto_close {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(10))
                            .await;
                        weak.update(cx, |view, cx| {
                            if view.auto_close_generation == auto_close_generation
                                && matches!(view.state, InstallState::Done { success: true, .. })
                            {
                                cx.quit();
                            }
                        })
                        .ok();
                    }
                }
                Err(e) => {
                    weak.update(cx, |view, cx| {
                        let mut log = final_log;
                        log.push(e.to_string());
                        view.log_list_state.reset(log.len());
                        view.log_list_state.scroll_to_reveal_item(log.len() - 1);
                        view.state = InstallState::Done {
                            message: tr("install.installation_failed"),
                            success: false,
                            log,
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
        let (pkg_name, purge) = match &self.state {
            InstallState::FileSelected { info, purge, .. } => (info.name.clone(), *purge),
            _ => return,
        };
        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let name = pkg_name.clone();
            let weak = weak.clone();
            dialog
                .confirm()
                .title(tr("common.uninstall"))
                .icon(
                    Icon::new(IconName::TriangleAlert)
                        .size_6()
                        .text_color(cx.theme().danger),
                )
                .description(
                    rust_i18n::t!("packages.remove_package_confirm", pkg = pkg_name.as_str())
                        .to_string(),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(if purge {
                            "install.purge_hint"
                        } else {
                            "install.remove_hint"
                        })),
                )
                .ok_text(tr("packages.yes_remove"))
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(tr("common.cancel"))
                .on_ok(move |_, window, cx| {
                    weak.update(cx, |view, cx| {
                        view.start_uninstall_by_name(name.clone(), purge, window, cx)
                    })
                    .ok();
                    true
                })
        });
    }

    /// Reuses the streaming uninstall UI for the package details action.
    pub fn start_uninstall_by_name(
        &mut self,
        pkg_name: String,
        purge: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_uninstall_by_names(vec![pkg_name], purge, window, cx);
    }

    /// Uninstalls the confirmed selection in a single apt transaction.
    pub fn start_uninstall_by_names(
        &mut self,
        pkg_names: Vec<String>,
        purge: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() || pkg_names.is_empty() {
            return;
        }
        self.cancel_auto_close();
        self.log_copied = false;
        self.state = InstallState::Uninstalling {
            pkg_names: pkg_names.clone(),
            log: Vec::new(),
        };
        self.open_target = None;
        self.log_list_state.reset(0);
        cx.notify();

        let on_installed = self.on_installed.clone();

        cx.spawn_in(window, async move |weak, cx| {
            let (log_tx, log_rx) = async_channel::unbounded::<String>();

            let names_bg = pkg_names.clone();
            let result_task = cx
                .background_executor()
                .spawn(async move { dpkg::remove_packages_streaming(names_bg, purge, log_tx) });

            while let Ok(line) = log_rx.recv().await {
                let l = line.clone();
                weak.update(cx, |view, cx| {
                    if let InstallState::Uninstalling { ref mut log, .. } = view.state {
                        log.push(l);
                        view.log_copied = false;
                        let last = log.len().saturating_sub(1);
                        view.log_list_state.splice(last..last, 1);
                        view.log_list_state.scroll_to_reveal_item(last);
                    }
                    cx.notify();
                })
                .ok();
            }

            let final_log = weak
                .read_with(cx, |view, _| match &view.state {
                    InstallState::Uninstalling { log, .. } => log.clone(),
                    _ => Vec::new(),
                })
                .unwrap_or_default();

            let result = result_task.await;

            match result {
                Ok(()) => {
                    weak.update(cx, |view, cx| {
                        view.state = InstallState::Done {
                            message: if pkg_names.len() == 1 {
                                rust_i18n::t!(
                                    "install.package_uninstalled",
                                    pkg = pkg_names[0].as_str()
                                )
                                .to_string()
                            } else {
                                rust_i18n::t!(
                                    "install.packages_uninstalled",
                                    count = pkg_names.len()
                                )
                                .to_string()
                            },
                            success: true,
                            log: final_log,
                        };
                        cx.notify();
                    })
                    .ok();
                }
                Err(e) => {
                    weak.update(cx, |view, cx| {
                        let mut log = final_log;
                        log.push(e.to_string());
                        view.log_list_state.reset(log.len());
                        view.log_list_state.scroll_to_reveal_item(log.len() - 1);
                        view.state = InstallState::Done {
                            message: tr("install.uninstall_failed"),
                            success: false,
                            log,
                        };
                        cx.notify();
                    })
                    .ok();
                }
            }
            // apt may remove some packages before reporting an error.
            if let Some(cb) = on_installed {
                cx.update(|window, cx| cb(window, cx)).ok();
            }
        })
        .detach();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_auto_close();
        self.state = InstallState::Idle;
        self.open_target = None;
        self.log_copied = false;
        self.log_list_state.reset(0);
        cx.notify();
        if let Some(cb) = self.on_reset.clone() {
            cb(window, cx);
        }
    }

    /// Opens the application the installed package provides.
    ///
    /// The system launcher is waited on to see whether it worked, so it runs on
    /// the background executor to keep the window responsive.
    fn open_installed_app(&mut self, target: &AppTarget, cx: &mut Context<Self>) {
        let target = target.clone();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { app_launch::launch(&target) })
                .await;
            let Err(err) = result else {
                return;
            };
            weak.update(cx, |view, cx| view.report_open_failure(err.to_string(), cx))
                .ok();
        })
        .detach();
    }

    /// Reports a failed launch through the output panel the user is looking at.
    fn report_open_failure(&mut self, message: String, cx: &mut Context<Self>) {
        self.log_copied = false;
        if let InstallState::Done { log, .. } = &mut self.state {
            log.push(rust_i18n::t!("install.open_app_failed", err = message).to_string());
            let last = log.len().saturating_sub(1);
            self.log_list_state.splice(last..last, 1);
            self.log_list_state.scroll_to_reveal_item(last);
        }
        cx.notify();
    }

    /// Returns Markdown content for the Dependencies tab.
    pub fn deps_markdown(&self) -> String {
        match &self.state {
            InstallState::FileSelected { info, .. } => {
                if info.depends.is_empty() {
                    tr("install.no_dependencies_markdown")
                } else {
                    info.depends
                        .iter()
                        .map(|d| format!("- {}", d))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            _ => tr("install.dependencies_hint_markdown"),
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for InstallView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .p_3()
            .gap_4()
            .child(match &self.state {
                InstallState::Idle => render_idle(self.selecting_file, cx),
                InstallState::LoadingInfo(path) => {
                    render_loading_info(path.to_string_lossy().to_string(), cx)
                }
                InstallState::FileSelected {
                    path,
                    info,
                    installed_version,
                    purge,
                } => {
                    let path_s = path.to_string_lossy().to_string();
                    let iv = installed_version.clone();
                    let auto_close = self.auto_close;
                    // SAFETY: borrowing different fields simultaneously is allowed
                    let inputs = &self.info_inputs;
                    render_file_selected(path_s, info, iv, auto_close, *purge, inputs, cx)
                }
                InstallState::Installing { info, log } => {
                    let list_state = self.log_list_state.clone();
                    render_with_log(
                        &rust_i18n::t!("install.installing", pkg = info.name.as_str()).to_string(),
                        log,
                        None,
                        None,
                        list_state,
                        self.log_copied,
                        cx,
                    )
                }
                InstallState::Uninstalling { pkg_names, log } => {
                    let list_state = self.log_list_state.clone();
                    let title = if pkg_names.len() == 1 {
                        rust_i18n::t!("install.uninstalling", pkg = pkg_names[0].as_str())
                            .to_string()
                    } else {
                        rust_i18n::t!("install.uninstalling_packages", count = pkg_names.len())
                            .to_string()
                    };
                    render_with_log(&title, log, None, None, list_state, self.log_copied, cx)
                }
                InstallState::Done {
                    message,
                    success,
                    log,
                } => {
                    let list_state = self.log_list_state.clone();
                    let open_target = self.open_target.clone();
                    render_with_log(
                        "",
                        log,
                        Some((*success, message.clone())),
                        open_target,
                        list_state,
                        self.log_copied,
                        cx,
                    )
                }
            })
    }
}

// ---------------------------------------------------------------------------
// Sub-renderers
// ---------------------------------------------------------------------------

fn is_deb_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("deb"))
}

fn render_idle(selecting: bool, cx: &mut Context<InstallView>) -> gpui_kit::AnyElement {
    v_flex()
        .id("package-drop-zone")
        .size_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap_5()
        .rounded_xl()
        .border_1()
        .border_dashed()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .drag_over::<ExternalPaths>(|style, _, _, cx| {
            style
                .bg(cx.theme().primary.opacity(0.06))
                .border_color(cx.theme().primary)
        })
        .on_drop(cx.listener(|view, paths: &ExternalPaths, window, cx| {
            if view.is_busy() {
                return;
            }
            if paths.paths().len() == 1 {
                let path = &paths.paths()[0];
                if is_deb_path(path) {
                    view.load_deb_file(path.clone(), window, cx);
                } else {
                    view.state = InstallState::Done {
                        message: tr("install.selected_not_deb"),
                        success: false,
                        log: Vec::new(),
                    };
                    cx.notify();
                }
            }
        }))
        .child(ui::icon_tile(IconName::PackageOpen, cx))
        .child(
            v_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_2xl()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(tr("install.welcome")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("install.idle_hint")),
                ),
        )
        .child(
            Button::new("select-file")
                .primary()
                .icon(IconName::FolderOpen)
                .label(tr("install.select_deb_file"))
                .loading(selecting)
                .on_click(cx.listener(|view, _, window, cx| view.select_file(window, cx))),
        )
        .child(
            h_flex()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::ShieldCheck).size_4())
                .child(tr("install.inspect_hint")),
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Shared async helper – used by both load_deb_file and select_file
// ---------------------------------------------------------------------------

async fn load_deb_async(
    path: PathBuf,
    path_str: String,
    weak: WeakEntity<InstallView>,
    cx: &mut AsyncWindowContext,
) {
    let result = cx
        .background_executor()
        .spawn(async move {
            let info = deb_reader::read_deb_info(&path)?;
            let installed = dpkg::installed_version(&info.name, &info.architecture);
            Ok::<_, anyhow::Error>((info, installed))
        })
        .await;

    match result {
        Ok((info, installed_version)) => {
            // Collect entity handles before mutating state
            let entities = weak
                .read_with(cx, |view, _| {
                    (
                        view.info_inputs.name.clone(),
                        view.info_inputs.version.clone(),
                        view.info_inputs.path.clone(),
                        view.info_inputs.description.clone(),
                        view.info_inputs.maintainer.clone(),
                        view.info_inputs.size.clone(),
                        view.info_inputs.section.clone(),
                        view.info_inputs.categories.clone(),
                    )
                })
                .ok();

            // Transition to FileSelected
            let loaded_path = weak
                .read_with(cx, |view, _| match &view.state {
                    InstallState::LoadingInfo(p) => p.clone(),
                    _ => PathBuf::new(),
                })
                .unwrap_or_default();

            let on_deb_loaded = weak
                .read_with(cx, |view, _| view.on_deb_loaded.clone())
                .ok()
                .flatten();

            weak.update(cx, |view, cx| {
                let p = match &view.state {
                    InstallState::LoadingInfo(p) => p.clone(),
                    _ => PathBuf::new(),
                };
                view.state = InstallState::FileSelected {
                    path: p,
                    info: info.clone(),
                    installed_version,
                    purge: false,
                };
                cx.notify();
            })
            .ok();

            // Fire on_deb_loaded callback
            if let Some(cb) = on_deb_loaded {
                cx.update(|window, cx| cb(loaded_path, window, cx)).ok();
            }

            // Populate the selectable input fields
            if let Some((ne, ve, pe, de, me, se, ste, ce)) = entities {
                let name_v = info.name.clone();
                let ver_v = format!("v{} ({})", info.version, info.architecture);
                let path_v = path_str;
                let desc_v = info.description.lines().next().unwrap_or("").to_string();
                let maint_v = info.maintainer.clone();
                let size_v = if info.installed_size_kb > 0 {
                    format!("{} KB", info.installed_size_kb)
                } else {
                    tr("install.unknown")
                };
                let sect_v = info
                    .section
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| tr("install.unknown"));

                cx.update_window_entity(&ne, |s, w, cx| s.set_value(name_v, w, cx))
                    .ok();
                cx.update_window_entity(&ve, |s, w, cx| s.set_value(ver_v, w, cx))
                    .ok();
                cx.update_window_entity(&pe, |s, w, cx| s.set_value(path_v, w, cx))
                    .ok();
                cx.update_window_entity(&de, |s, w, cx| s.set_value(desc_v, w, cx))
                    .ok();
                cx.update_window_entity(&me, |s, w, cx| s.set_value(maint_v, w, cx))
                    .ok();
                cx.update_window_entity(&se, |s, w, cx| s.set_value(size_v, w, cx))
                    .ok();
                cx.update_window_entity(&ste, |s, w, cx| s.set_value(sect_v, w, cx))
                    .ok();
                cx.update_window_entity(&ce, |s, w, cx| {
                    s.set_value(tr("files_preview.loading"), w, cx)
                })
                .ok();
            }
        }
        Err(e) => {
            weak.update(cx, |view, cx| {
                view.state = InstallState::Done {
                    message: rust_i18n::t!("install.read_deb_failed", err = e.to_string())
                        .to_string(),
                    success: false,
                    log: Vec::new(),
                };
                cx.notify();
            })
            .ok();
        }
    }
}

fn render_loading_info(path: String, cx: &App) -> gpui_kit::AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(Spinner::new().large().color(cx.theme().primary))
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(tr("install.reading_package_info")),
        )
        .child(
            div()
                .max_w_full()
                .truncate()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(path),
        )
        .into_any_element()
}

fn render_file_selected(
    _path: String,
    info: &DebInfo,
    installed_version: Option<String>,
    auto_close: bool,
    purge: bool,
    inputs: &InfoInputs,
    cx: &mut Context<InstallView>,
) -> gpui_kit::AnyElement {
    // Determine install status label and button label
    let (status_text, status_color, install_label) = match &installed_version {
        None => (
            tr("install.status.not_installed"),
            None,
            tr("install.action.install"),
        ),
        Some(v) => match crate::utils::dpkg::compare_versions(v, &info.version) {
            std::cmp::Ordering::Equal => (
                rust_i18n::t!("install.status.already_installed", version = v.as_str()).to_string(),
                Some("warning"),
                tr("install.action.reinstall"),
            ),
            std::cmp::Ordering::Less => (
                rust_i18n::t!(
                    "install.status.upgrade",
                    from = v.as_str(),
                    to = info.version.as_str()
                )
                .to_string(),
                Some("success"),
                tr("install.action.upgrade"),
            ),
            std::cmp::Ordering::Greater => (
                rust_i18n::t!(
                    "install.status.downgrade",
                    from = v.as_str(),
                    to = info.version.as_str()
                )
                .to_string(),
                Some("danger"),
                tr("install.action.downgrade"),
            ),
        },
    };

    let status_color = match status_color {
        Some("warning") => cx.theme().warning,
        Some("success") => cx.theme().success,
        Some("danger") => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };

    v_flex()
        .size_full()
        .min_h_0()
        .gap_2()
        .child(
            h_flex()
                .flex_shrink_0()
                .p_3()
                .gap_3()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    h_flex()
                        .size(px(32.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .bg(cx.theme().primary.opacity(0.10))
                        .text_color(cx.theme().primary)
                        .child(Icon::new(IconName::Package).size_5()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_lg()
                                .truncate()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(info.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} · {}", info.version, info.architecture)),
                        ),
                )
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .max_w(px(260.))
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(status_color.opacity(0.08))
                        .text_color(status_color)
                        .text_xs()
                        .child(Icon::new(IconName::Info).size_4())
                        .child(status_text),
                ),
        )
        .child(
            v_flex()
                .id("package-details")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .gap_1()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(info_row_input(
                    tr("install.field.package"),
                    &inputs.name,
                    cx,
                ))
                .child(info_row_input(
                    tr("install.field.version"),
                    &inputs.version,
                    cx,
                ))
                .child(info_row_input(tr("install.field.file"), &inputs.path, cx))
                .child(info_row_input(
                    tr("install.field.description"),
                    &inputs.description,
                    cx,
                ))
                .child(info_row_input(
                    tr("install.field.maintainer"),
                    &inputs.maintainer,
                    cx,
                ))
                .child(info_row_input(
                    tr("install.field.installed_size"),
                    &inputs.size,
                    cx,
                ))
                .child(info_row_input(
                    tr("install.field.section"),
                    &inputs.section,
                    cx,
                ))
                .child(info_row_input(
                    tr("install.field.categories"),
                    &inputs.categories,
                    cx,
                )),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .gap_3()
                .p_2()
                .rounded_lg()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Checkbox::new("auto-close-cb")
                                .label(tr("install.action.auto_close"))
                                .checked(auto_close)
                                .on_click(cx.listener(|view, checked: &bool, _, cx| {
                                    view.auto_close = *checked;
                                    let mut cfg = AppConfig::load();
                                    cfg.auto_close = *checked;
                                    cfg.save();
                                    cx.notify();
                                })),
                        )
                        .when(installed_version.is_some(), |el| {
                            el.child(
                                Checkbox::new("purge-config")
                                    .label(tr("common.purge_config"))
                                    .checked(purge)
                                    .on_click(cx.listener(|view, checked: &bool, _, cx| {
                                        if let InstallState::FileSelected { purge, .. } =
                                            &mut view.state
                                        {
                                            *purge = *checked;
                                        }
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_2()
                        .child(
                            Button::new("cancel-btn")
                                .ghost()
                                .icon(IconName::ArrowLeft)
                                .label(tr("install.action.cancel"))
                                .on_click(
                                    cx.listener(|view, _, window, cx| view.reset(window, cx)),
                                ),
                        )
                        .when(installed_version.is_some(), |el| {
                            el.child(
                                Button::new("uninstall-btn")
                                    .danger()
                                    .outline()
                                    .icon(IconName::Trash)
                                    .label(tr("install.action.uninstall"))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.uninstall_package(window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("install-btn")
                                .primary()
                                .icon(IconName::Download)
                                .label(install_label)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.install_package(window, cx)
                                })),
                        ),
                ),
        )
        .into_any_element()
}

/// Unified renderer for the in-progress and done states.
/// - `done = None`            → in-progress: shows title + live log, no Back button
/// - `done = Some((ok, msg))` → finished: swaps title for result banner, shows Back button
///
/// `open_target` is only set after a successful install that exposed an
/// application, and adds an "Open App" action to the result banner.
fn render_with_log(
    title: &str,
    log: &[String],
    done: Option<(bool, String)>,
    open_target: Option<AppTarget>,
    list_state: ListState,
    copied: bool,
    cx: &mut Context<InstallView>,
) -> gpui_kit::AnyElement {
    let is_done = done.is_some();

    // Build owned lines for the closure
    let lines: Vec<SharedString> = if log.is_empty() {
        Vec::new()
    } else {
        log.iter().map(|l| SharedString::from(l.clone())).collect()
    };

    let theme_fg = cx.theme().foreground;
    let theme_muted = cx.theme().muted_foreground;

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_3()
        // Header: result banner when done, plain title when in-progress
        .child(match done {
            Some((success, message)) => {
                let border_color = if success {
                    cx.theme().success
                } else {
                    cx.theme().danger
                };
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .rounded_lg()
                    .border_1()
                    .border_color(border_color.opacity(0.35))
                    .bg(border_color.opacity(0.08))
                    .child(
                        Icon::new(if success {
                            IconName::CircleCheck
                        } else {
                            IconName::CircleAlert
                        })
                        .size_6()
                        .text_color(border_color),
                    )
                    .child(div().flex_1().min_w_0().child(message))
                    .when_some(open_target, |el, target| {
                        el.child(
                            Button::new("open-app-btn")
                                .primary()
                                .icon(IconName::ExternalLink)
                                .label(tr("install.action.open_app"))
                                .on_click(cx.listener(move |view, _ev, _window, cx| {
                                    view.open_installed_app(&target, cx);
                                })),
                        )
                    })
                    .into_any_element()
            }
            None => h_flex()
                .flex_shrink_0()
                .p_4()
                .gap_3()
                .rounded_lg()
                .bg(cx.theme().primary.opacity(0.06))
                .child(Spinner::new().color(cx.theme().primary))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(title.to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme_muted)
                                .child(tr("install.progress_hint")),
                        ),
                )
                .into_any_element(),
        })
        // Log panel
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .overflow_hidden()
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .bg(cx.theme().muted.opacity(0.5))
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::Terminal).size_4())
                        .child(tr("install.output"))
                        .child(div().flex_1())
                        .child(
                            Button::new("copy-output")
                                .ghost()
                                .small()
                                .icon(if copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                })
                                .label(tr(if copied {
                                    "common.copied"
                                } else {
                                    "install.copy_output"
                                }))
                                .disabled(log.is_empty())
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    let output = match &view.state {
                                        InstallState::Installing { log, .. }
                                        | InstallState::Uninstalling { log, .. }
                                        | InstallState::Done { log, .. } => log.join("\n"),
                                        _ => String::new(),
                                    };
                                    cx.write_to_clipboard(ClipboardItem::new_string(output));
                                    view.log_copied = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .when(log.is_empty(), |el| {
                            el.child(div().p_4().text_sm().text_color(theme_muted).child(tr(
                                if is_done {
                                    "install.no_output"
                                } else {
                                    "install.waiting_auth"
                                },
                            )))
                        })
                        .when(!log.is_empty(), |el| {
                            el.child(
                                list(list_state, move |ix, _window, _cx| {
                                    let line = lines.get(ix).cloned().unwrap_or_default();
                                    div()
                                        .px_3()
                                        .py(px(1.))
                                        .text_sm()
                                        .font_family("monospace")
                                        .text_color(if line.is_empty() {
                                            theme_muted
                                        } else {
                                            theme_fg
                                        })
                                        .child(line)
                                        .into_any_element()
                                })
                                .flex_1()
                                .w_full()
                                .h_full(),
                            )
                        }),
                ),
        )
        // Back button appears only after completion
        .when(is_done, |el| {
            el.child(
                Button::new("reset-btn")
                    .icon(IconName::ArrowLeft)
                    .label(tr("install.action.back"))
                    .on_click(cx.listener(|view, _ev, window, cx| {
                        view.reset(window, cx);
                    })),
            )
        })
        .into_any_element()
}

/// Renders a label + selectable Input value row.
/// The Input is read-only and has no visible appearance,
/// so it looks like plain text but supports mouse selection and Ctrl+C.
fn info_row_input(
    label: String,
    input: &Entity<InputState>,
    cx: &mut Context<InstallView>,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_center()
        .text_sm()
        .child(
            div()
                .w(gpui_kit::px(120.))
                .text_color(cx.theme().muted_foreground)
                .flex_shrink_0()
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(input).small().readonly(true).appearance(false)),
        )
}
