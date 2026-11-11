//! Hand-built GPUI icon elements (no icon font dependency).
//!
//! Icons are rendered as system-font glyphs from `Segoe UI Symbol` (present on every supported
//! Windows install), sized via the caller. The design spec allows this approach and notes that the
//! glyphs can be replaced with vector shapes later if spacing ever looks wrong; positioning is done
//! by the surrounding 40×40 hit targets, so a glyph swap is a one-line change.

use gpui::prelude::*;
use gpui::{AnyElement, Hsla, div, px, relative};

/// Font family used for symbol glyphs (crisp on Windows and present for all icons below).
pub const SYMBOL_FONT: &str = "Segoe UI Symbol";

/// Build a centered single-glyph icon element.
pub fn glyph(text: &'static str, size: f32, color: Hsla) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size * 1.25))
        .text_size(px(size))
        .line_height(relative(1.0))
        .font_family(SYMBOL_FONT)
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// ☰ — TOC / sidebar toggle (top bar, always visible).
pub fn toc_icon(color: Hsla) -> AnyElement {
    glyph("☰", 22.0, color)
}

/// ⚙ — settings popover trigger.
pub fn gear_icon(color: Hsla) -> AnyElement {
    glyph("⚙", 22.0, color)
}

/// × — close book (returns to landing).
pub fn close_icon(color: Hsla) -> AnyElement {
    glyph("×", 26.0, color)
}

/// ‹ — previous chapter arrow gutter.
pub fn chevron_left(color: Hsla) -> AnyElement {
    glyph("‹", 34.0, color)
}

/// › — next chapter arrow gutter.
pub fn chevron_right(color: Hsla) -> AnyElement {
    glyph("›", 34.0, color)
}

/// Small × used by dismissible rows (recent books / error banner).
pub fn remove_icon(color: Hsla) -> AnyElement {
    glyph("×", 14.0, color)
}

/// Placeholder book glyph for cover slots.
pub fn book_placeholder(color: Hsla, size: f32) -> AnyElement {
    glyph("▥", size, color)
}
