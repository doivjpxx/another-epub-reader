//! Shared small UI components: error banner, toolbar icon buttons, spinner.
//!
//! The error banner signature is shared with the landing screen (`ui::landing`) so both screens
//! surface failures the same way (red `#c0392b` text on `#fdf2f2`, 13 px, dismiss ×).

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Div, ElementId, Hsla, IntoElement, Stateful, Window, div, px, rgb,
};

/// Page/error message color (design spec §7.1 error banner).
const ERROR_TEXT: u32 = 0xc0392b;
/// Error banner background (design spec §7.1).
const ERROR_BG: u32 = 0xfdf2f2;

/// Dismissible inline error banner used by the landing screen (and any reader error slot).
///
/// This is the exact signature the P3 landing lane codes against:
/// `error_banner(message, on_dismiss)`.
pub fn error_banner(
    message: impl Into<gpui::SharedString>,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let message = message.into();
    div()
        .id("error-banner")
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px(px(10.0))
        .py(px(8.0))
        .mb(px(6.0))
        .bg(rgb(ERROR_BG))
        .text_color(rgb(ERROR_TEXT))
        .text_size(px(13.0))
        .child(div().flex_1().child(message))
        .child(
            div()
                .id("error-banner-dismiss")
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.0))
                .text_size(px(16.0))
                .cursor_pointer()
                .child("×")
                .on_click(on_dismiss),
        )
        .into_any_element()
}

/// A 40×40 icon hit target for the reader top bar (design spec §6.1).
///
/// The caller supplies the glyph child and the interaction handler, which keeps this helper usable
/// for both click (`on_click`) and press-to-toggle (`on_mouse_down`) semantics.
pub fn toolbar_icon(id: impl Into<ElementId>, hover_bg: Option<Hsla>) -> Stateful<Div> {
    let mut button = div()
        .id(id)
        .size(px(40.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer();
    if let Some(hover_bg) = hover_bg {
        button = button.hover(move |style| style.bg(hover_bg));
    }
    button
}

/// Convenience: a 40×40 icon button that fires `on_click`.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: AnyElement,
    hover_bg: Option<Hsla>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    toolbar_icon(id, hover_bg)
        .child(icon)
        .on_click(on_click)
        .into_any_element()
}

/// A minimal static loading indicator (glyph based; no animation loop to avoid idle repaints).
pub fn spinner(color: Hsla) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .p(px(12.0))
        .text_size(px(16.0))
        .text_color(color)
        .child("Loading…")
        .into_any_element()
}
