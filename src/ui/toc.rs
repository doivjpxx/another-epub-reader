//! TOC sidebar rows for the reader.
//!
//! Parity values (design spec §6.1/U5): flattened TOC with 25 px indent per level, selected row
//! `#346cb9` bold, hover text per theme. The sidebar itself (20% width, scroll) is built by
//! `ui::reader`; this module owns the list body so it can stay a pure builder.

use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Hsla, IntoElement, Window, div, px};

/// Sidebar list container: vertical stack with top/bottom padding.
pub fn toc_list(rows: Vec<AnyElement>, theme: Theme) -> AnyElement {
    div()
        .id("toc-list")
        .w_full()
        .flex()
        .flex_col()
        .text_color(theme.sidebar_text())
        .children(rows)
        .into_any_element()
}

/// One TOC row.
///
/// `index` keeps element ids unique even when two entries share a label; `on_click` is supplied by
/// the reader (a `cx.listener` closure). Rows whose TOC entry has no spine mapping still render,
/// but the reader passes a no-op click handler for them.
pub fn toc_row(
    index: usize,
    label: &str,
    depth: usize,
    selected: bool,
    theme: Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    // 25 px indent per nesting level (source parity), plus a 12 px base inset.
    let indent = 12.0 + 25.0 * depth as f32;
    let text_color: Hsla = if selected {
        theme.selected_toc()
    } else {
        theme.sidebar_text()
    };

    let mut row = div()
        .id(("toc-row", index))
        .w_full()
        .flex()
        .items_center()
        .pl(px(indent))
        .pr(px(15.0))
        .py(px(15.0))
        .border_t_1()
        .border_color(theme.border())
        .text_size(px(14.0))
        .line_height(gpui::relative(1.35))
        .text_color(text_color)
        // The label is allowed to shrink below its intrinsic width so long chapter titles wrap
        // (multiple lines, hard-breaking over-long words) instead of being clipped by the sidebar.
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(label.to_string()),
        )
        .on_click(on_click);

    if selected {
        row = row.font_weight(gpui::FontWeight::BOLD);
    }

    let hover_text = theme.sidebar_hover_text();
    let hover_bg = theme.sidebar_hover_bg();
    match hover_bg {
        Some(bg) => row.hover(move |style| style.bg(bg).text_color(hover_text)),
        None => row.hover(move |style| style.text_color(hover_text)),
    }
    .into_any_element()
}

/// Sidebar fallback: when the EPUB has no TOC, list the spine as `Chapter N` rows.
pub fn spine_fallback_labels(book: &crate::epub::Book) -> Vec<(String, usize)> {
    book.spine()
        .iter()
        .enumerate()
        .map(|(index, item)| {
            if item.title_hint.trim().is_empty() {
                (format!("Chapter {}", index + 1), index)
            } else {
                (item.title_hint.trim().to_string(), index)
            }
        })
        .collect()
}
