use gpui::{
    App, Context, FontWeight, IntoElement, ParentElement, Render, Styled, WeakEntity, Window, div,
    prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Size, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use std::sync::Arc;

use crate::models::{db, package::InstalledPackage};
use crate::views::install::InstallView;

pub struct PackagesView {
    rows: Vec<InstalledPackage>,
    /// Package name pending uninstall confirmation.
    confirm_target: Option<String>,
    status_msg: Option<String>,
    /// Weak handle to InstallView, used to delegate the uninstall operation.
    pub install_view: Option<WeakEntity<InstallView>>,
    /// Called before uninstall starts — used by AppView to switch to the Install tab.
    pub on_tab_switch: Option<Arc<dyn Fn(&mut App) + 'static>>,
}

impl PackagesView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            rows: db::load_packages().unwrap_or_default(),
            confirm_target: None,
            status_msg: None,
            install_view: None,
            on_tab_switch: None,
        }
    }

    /// Reload the package list from disk.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.rows = db::load_packages().unwrap_or_default();
        cx.notify();
    }

    fn start_uninstall(&mut self, name: String, cx: &mut Context<Self>) {
        self.confirm_target = Some(name);
        cx.notify();
    }

    fn confirm_uninstall(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = match self.confirm_target.take() {
            Some(n) => n,
            None => return,
        };
        cx.notify();

        // Switch to the Install tab so the user sees the streaming log.
        if let Some(ref f) = self.on_tab_switch {
            f(cx);
        }

        // Delegate to InstallView's streaming uninstall logic.
        if let Some(ref install_weak) = self.install_view {
            install_weak
                .update(cx, |install, install_cx| {
                    install.start_uninstall_by_name(name, window, install_cx);
                })
                .ok();
        }
    }

    fn cancel_confirm(&mut self, cx: &mut Context<Self>) {
        self.confirm_target = None;
        cx.notify();
    }
}

impl Render for PackagesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let rows_count = rows.len();
        let confirm_target = self.confirm_target.clone();
        let status_msg = self.status_msg.clone();

        // Build uninstall buttons
        let uninstall_buttons: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(i, pkg)| {
                let name = pkg.name.clone();
                Button::new(("uninstall", i))
                    .danger()
                    .with_size(Size::Small)
                    .label("Uninstall")
                    .on_click(cx.listener(move |view, _ev, _window, cx| {
                        view.start_uninstall(name.clone(), cx);
                    }))
            })
            .collect();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            // Status message
            .when_some(status_msg, |el, msg| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(cx.theme().list)
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_color(cx.theme().muted_foreground)
                        .child(msg),
                )
            })
            // Confirm dialog (inline overlay)
            .when_some(confirm_target, |el, name| {
                let name_confirm = name.clone();
                el.child(
                    h_flex()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(cx.theme().danger.opacity(0.1))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .child(
                            div()
                                .flex_1()
                                .text_color(cx.theme().foreground)
                                .child(format!("Remove package '{}'?", name)),
                        )
                        .child(
                            Button::new("confirm-yes")
                                .danger()
                                .with_size(Size::Small)
                                .label("Yes, Remove")
                                .on_click(cx.listener(move |view, _ev, window, cx| {
                                    view.confirm_target = Some(name_confirm.clone());
                                    view.confirm_uninstall(window, cx);
                                })),
                        )
                        .child(
                            Button::new("confirm-no")
                                .with_size(Size::Small)
                                .label("Cancel")
                                .on_click(cx.listener(|view, _ev, _window, cx| {
                                    view.cancel_confirm(cx);
                                })),
                        ),
                )
            })
            // Header row
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(cx.theme().tab_bar)
                    .text_color(cx.theme().muted_foreground)
                    .font_weight(FontWeight::BOLD)
                    .child(div().w(gpui::px(200.)).child("Package"))
                    .child(div().w(gpui::px(120.)).child("Version"))
                    .child(div().flex_1().child("Installed"))
                    .child(div().w(gpui::px(100.)).child("Action")),
            )
            // Empty state
            .when(rows_count == 0, |el| {
                el.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_color(cx.theme().muted_foreground)
                        .child("No packages installed yet."),
                )
            })
            // Data rows
            .children(
                rows.into_iter()
                    .zip(uninstall_buttons.into_iter())
                    .map(|(pkg, btn)| {
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().list)
                            .child(div().w(gpui::px(200.)).child(pkg.name.clone()))
                            .child(div().w(gpui::px(120.)).child(pkg.version.clone()))
                            .child(
                                div()
                                    .flex_1()
                                    .child(pkg.install_date.format("%Y-%m-%d %H:%M").to_string()),
                            )
                            .child(div().w(gpui::px(100.)).child(btn))
                    }),
            )
    }
}
