//! Settings popover: font size and theme controls.
//!
//! Parity values (design spec §6.1): anchored under the gear, min-width 160 px, `#fafafa`
//! background, 1 px `rgba(0,0,0,0.07)` border, shadow `0 6px 12px rgba(0,0,0,0.175)`, 8 px caret,
//! row 1 font-size buttons at 50% each (`A` 14 px / `A` 20 px), row 2 themes at 33% each, hover text
//! `#444`, disabled color `#d0d0d0`. Closes on outside click (the reader also wires Esc).

use crate::theme::{FONT_SIZE_MAX, FONT_SIZE_MIN, FONT_SIZE_STEP, Settings, Theme};
use gpui::prelude::*;
use gpui::{
    AnyElement, BoxShadow, ClickEvent, Hsla, IntoElement, MouseDownEvent, Window, div, hsla, point,
    px, rgb,
};
use std::rc::Rc;

/// Popover surface (design spec §6.1 parity value).
const POPOVER_BG: u32 = 0xfafafa;
/// Popover border color (parity).
const POPOVER_BORDER_ALPHA: f32 = 0.07;
/// Hover text color (parity).
const HOVER_TEXT: u32 = 0x444444;
/// Disabled control color (parity).
const DISABLED: u32 = 0xd0d0d0;
/// Normal control text color.
const CONTROL_TEXT: u32 = 0x333333;

/// Build the settings popover.
///
/// - `on_size` receives the *target* font size (already stepped/clamped by the button that fired).
/// - `on_theme` receives the theme to apply.
/// - `on_outside` is called on mouse-down outside the popover (close it).
pub fn popover(
    settings: &Settings,
    on_size: impl Fn(f32, &ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_theme: impl Fn(Theme, &ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_outside: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let on_size: Rc<dyn Fn(f32, &ClickEvent, &mut Window, &mut gpui::App)> = Rc::new(on_size);
    let on_theme: Rc<dyn Fn(Theme, &ClickEvent, &mut Window, &mut gpui::App)> = Rc::new(on_theme);

    let current = settings.font_size;
    let can_shrink = current > FONT_SIZE_MIN;
    let can_grow = current < FONT_SIZE_MAX;

    let size_row = div()
        .id("font-size-row")
        .w_full()
        .flex()
        .flex_row()
        .child(font_button(
            "font-smaller",
            "A",
            14.0,
            can_shrink,
            stepped_size(current, -FONT_SIZE_STEP),
            on_size.clone(),
        ))
        .child(font_button(
            "font-larger",
            "A",
            20.0,
            can_grow,
            stepped_size(current, FONT_SIZE_STEP),
            on_size.clone(),
        ));

    let theme_row = div()
        .id("theme-row")
        .w_full()
        .flex()
        .flex_row()
        .children(
            [
                (Theme::White, "White", "theme-white"),
                (Theme::Sepia, "Sepia", "theme-sepia"),
                (Theme::Night, "Night", "theme-night"),
            ]
            .into_iter()
            .map(|(theme, label, id)| {
                theme_button(theme, label, id, settings.theme == theme, on_theme.clone())
            }),
        );

    div()
        .id("settings-popover")
        .relative()
        .w(px(160.0))
        .min_w(px(160.0))
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap_1()
        .bg(rgb(POPOVER_BG))
        .border_1()
        .border_color(hsla(0.0, 0.0, 0.0, POPOVER_BORDER_ALPHA))
        .rounded_md()
        .shadow(vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.175),
            offset: point(px(0.0), px(6.0)),
            blur_radius: px(12.0),
            spread_radius: px(0.0),
        }])
        .on_mouse_down_out(on_outside)
        .child(caret())
        .child(size_row)
        .child(theme_row)
        .into_any_element()
}

/// 8 px caret pointing up at the gear side of the popover (design spec §6.1: "8 px caret").
///
/// Drawn as a filled triangle with a canvas path — no asset/icon font needed. The reader anchors
/// the popover at `left(-42px)` on the 40 px gear hit target, so the gear center is 20 + 42 = 62 px
/// from the popover's left edge; the caret is centered on it.
fn caret() -> AnyElement {
    const CARET_W: f32 = 10.0;
    const CARET_H: f32 = 6.0;
    const GEAR_CENTER_FROM_POPOVER_LEFT: f32 = 62.0;
    const CARET_LEFT: f32 = GEAR_CENTER_FROM_POPOVER_LEFT - CARET_W / 2.0;
    gpui::canvas(
        |_, _, _| {},
        move |bounds: gpui::Bounds<gpui::Pixels>,
              _: (),
              window: &mut Window,
              _cx: &mut gpui::App| {
            let left = bounds.origin.x;
            let top = bounds.origin.y;
            let bottom = top + px(CARET_H);
            let mut path = gpui::PathBuilder::fill();
            path.move_to(gpui::point(left, bottom));
            path.line_to(gpui::point(left + px(CARET_W / 2.0), top));
            path.line_to(gpui::point(left + px(CARET_W), bottom));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(POPOVER_BG));
            }
        },
    )
    .absolute()
    .top(px(-CARET_H))
    .left(px(CARET_LEFT))
    .w(px(CARET_W))
    .h(px(CARET_H))
    .into_any_element()
}

fn font_button(
    id: &'static str,
    label: &'static str,
    glyph_size: f32,
    enabled: bool,
    target_size: f32,
    on_size: Rc<dyn Fn(f32, &ClickEvent, &mut Window, &mut gpui::App)>,
) -> AnyElement {
    let color = if enabled {
        rgb(CONTROL_TEXT)
    } else {
        rgb(DISABLED)
    };
    let mut button = div()
        .id(id)
        .flex_1()
        .h(px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .text_size(px(glyph_size))
        .text_color(color)
        .child(label);
    if enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(rgb(POPOVER_BG)).text_color(rgb(HOVER_TEXT)))
            .on_click(move |event, window, cx| on_size(target_size, event, window, cx));
    }
    button.into_any_element()
}

fn theme_button(
    theme: Theme,
    label: &'static str,
    id: &'static str,
    active: bool,
    on_theme: Rc<dyn Fn(Theme, &ClickEvent, &mut Window, &mut gpui::App)>,
) -> AnyElement {
    let color = if active {
        rgb(CONTROL_TEXT)
    } else {
        rgb(HOVER_TEXT)
    };
    let mut button = div()
        .id(id)
        .flex_1()
        .px(px(4.0))
        .py(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .text_size(px(13.0))
        .text_color(color)
        .child(label);
    if active {
        button = button
            .font_weight(gpui::FontWeight::BOLD)
            .underline();
    }
    button
        .cursor_pointer()
        .hover(|style| style.text_color(rgb(HOVER_TEXT)))
        .on_click(move |event, window, cx| on_theme(theme, event, window, cx))
        .into_any_element()
}

/// Compute the next font size for a ± step, clamped to the 14..30 range.
pub fn stepped_size(current: f32, delta: f32) -> f32 {
    (current + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX)
}

/// Text color used by the popover for a control in the given state (exposed for tests).
pub fn popover_text_color(active: bool) -> Hsla {
    if active {
        rgb(CONTROL_TEXT).into()
    } else {
        rgb(HOVER_TEXT).into()
    }
}
