use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon, Theme, h_flex, v_flex};
use gpui_kit::{
    App, IntoElement, ParentElement, Styled, div, img, prelude::FluentBuilder, px, rgb,
};

pub fn icon_tile(icon: IconName, cx: &App) -> impl IntoElement {
    h_flex()
        .size(px(48.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_xl()
        .bg(cx.theme().primary.opacity(0.10))
        .text_color(cx.theme().primary)
        .child(Icon::new(icon).size_6())
}

pub fn empty_state(
    icon: IconName,
    title: String,
    description: String,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .p_6()
        .gap_3()
        .child(icon_tile(icon, cx))
        .child(
            div()
                .text_lg()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(title),
        )
        .when(!description.is_empty(), |el| {
            el.child(
                div()
                    .max_w(px(420.))
                    .text_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(description),
            )
        })
}

/// Keep the app accent legible in both system appearances.
pub fn apply_theme(cx: &mut App) {
    let names = cx.text_system().all_font_names();
    let font = [
        "Noto Sans CJK SC",
        "Source Han Sans SC",
        "WenQuanYi Micro Hei",
        "Noto Sans",
        "DejaVu Sans",
    ]
    .into_iter()
    .find(|family| names.iter().any(|name| name == family));
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        theme.primary = rgb(if dark { 0xa5b4fc } else { 0x4f46e5 }).into();
        theme.primary_hover = rgb(if dark { 0xc7d2fe } else { 0x4338ca }).into();
        theme.primary_active = rgb(if dark { 0x818cf8 } else { 0x3730a3 }).into();
        theme.primary_foreground = rgb(if dark { 0x1e1b4b } else { 0xffffff }).into();
        theme.button_primary = theme.primary;
        theme.button_primary_hover = theme.primary_hover;
        theme.button_primary_active = theme.primary_active;
        theme.button_primary_foreground = theme.primary_foreground;
        theme.tab_active_foreground = theme.primary;
        theme.ring = theme.primary;
        if let Some(font) = font {
            theme.font_family = font.to_string().into();
        }
    });
}

/// Reuse the desktop artwork inside the app without an external file lookup.
pub fn brand_icon() -> impl IntoElement {
    img("brand/icon.svg").size(px(28.)).flex_shrink_0()
}
