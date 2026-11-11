//! Reader screen: sidebar TOC (20%), 40 px top bar, 100 px arrow gutters, vertical scroll viewer.
//!
//! Parity values from the design spec §6.1: sidebar fixed 20% (collapsed = removed from layout, so
//! the reader occupies the full width), top bar 40 px with 40×40 icon targets and 24 px glyphs,
//! arrow gutters 100 px full-height, viewer bottom inset 20 px, settings popover anchored under the
//! gear, three themes, font size 14..30 step 2.
//!
//! The reader is a `Render` entity owned by `Root` (frozen constructor/`position()` API). Root owns
//! persistence; `position()` is always live (read from the `ScrollHandle`) so Root can persist on
//! close/book-switch without the reader having to know the book path.
//!
//! Documented deviations from the source app (spec §6.1/§6.4):
//! - Sidebar collapse/expand is instant instead of a 300 ms ease transition (layout parity is still
//!   exact: 20% ↔ 0).
//! - TOC highlight is chapter-level (one active row for the displayed spine item); intra-chapter
//!   scroll does not change the highlight, matching the source app.
//! - Scroll offset is restored after the first layout pass; position math lives in
//!   `ratio_from_offset`/`offset_from_ratio` and is unit-tested in `tests/position.rs`.

use crate::app::Root;
use crate::epub::model::{BlockModel, Position};
use crate::epub::{Book, TocItem};
use crate::render::{
    AnchorPositions, AnchorProbe, ImageCache, LinkHandler, anchor_positions, render_blocks_full,
};
use crate::theme::{FONT_SIZE_MAX, FONT_SIZE_MIN, Settings, Theme};
use crate::ui::{components, icons, settings as settings_popover, toc};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, DragMoveEvent, FocusHandle, Hsla, IntoElement,
    MouseButton, Render, ScrollHandle, WeakEntity, Window, div, point, px, relative,
};
use std::rc::Rc;
use std::sync::Arc;

/// Sidebar width as a fraction of the window by default (source parity: 20%).
const SIDEBAR_FRACTION: f32 = crate::theme::DEFAULT_SIDEBAR_WIDTH;
/// Top bar height in px (parity: 40 px).
const TOP_BAR_HEIGHT: f32 = 40.0;
/// Arrow gutter width in px (parity: 100 px each side).
const GUTTER_WIDTH: f32 = 100.0;
/// Bottom inset of the viewer in px (parity: 20 px).
const BOTTOM_INSET: f32 = 20.0;
/// Resizable sidebar bounds: at least 160 px, at most 45% of the window, and always leave at least
/// 360 px for the reading column (so small windows get a tighter maximum).
const SIDEBAR_MIN_PX: f32 = 160.0;
const SIDEBAR_MAX_FRACTION: f32 = 0.45;
const READER_MIN_CONTENT_PX: f32 = 360.0;
/// Width of the invisible drag handle between the sidebar and the reading area.
const SIDEBAR_HANDLE_WIDTH: f32 = 6.0;

/// Clamp a sidebar fraction for the current window width. Pure function (unit-tested).
pub fn clamp_sidebar_fraction(viewport_width: f32, fraction: f32) -> f32 {
    let viewport = viewport_width.max(1.0);
    let min_px = SIDEBAR_MIN_PX.min(viewport * 0.30);
    let max_px = (viewport - READER_MIN_CONTENT_PX - 2.0 * GUTTER_WIDTH).max(min_px);
    let max_px = max_px.min(viewport * SIDEBAR_MAX_FRACTION).max(min_px);
    let fraction = if fraction.is_finite() { fraction } else { SIDEBAR_FRACTION };
    let width_px = (fraction * viewport).clamp(min_px, max_px);
    width_px / viewport
}

/// Drag payload for the sidebar resize handle (the width itself is derived from the cursor x).
pub struct SidebarDrag;

/// Minimal ghost rendered under the cursor while dragging the sidebar handle.
struct SidebarDragPreview {
    color: Hsla,
}

impl Render for SidebarDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(2.0))
            .h(px(160.0))
            .rounded_full()
            .bg(self.color)
    }
}

/// Convert a positive scroll offset (px) into a clamped 0..1 ratio of the scrollable range.
pub fn ratio_from_offset(offset: f32, max: f32) -> f32 {
    if max <= 0.0 || !offset.is_finite() || !max.is_finite() {
        0.0
    } else {
        let ratio = (offset / max).clamp(0.0, 1.0);
        // Normalize `-0.0` (possible from negated scroll offsets) to `0.0`.
        if ratio == 0.0 { 0.0 } else { ratio }
    }
}

/// Convert a 0..1 ratio into a pixel scroll offset for the given scrollable range.
pub fn offset_from_ratio(ratio: f32, max: f32) -> f32 {
    ratio.clamp(0.0, 1.0) * max.max(0.0)
}

fn hover_background(theme: Theme) -> Option<gpui::Hsla> {
    Some(theme.sidebar_hover_bg().unwrap_or_else(|| theme.border()))
}

/// Debug-only initial UI state hook used by the scripted screenshot verification when the Windows
/// session is locked and synthetic mouse input cannot reach the app (keyboard input still works).
///
/// `EPUB_READER_UI_DEBUG=settings` opens the settings popover on start, `=no-sidebar` starts with
/// the TOC sidebar collapsed. No effect in release builds and none when the variable is unset.
fn debug_initial_ui_state() -> (bool, bool) {
    #[cfg(debug_assertions)]
    {
        match std::env::var("EPUB_READER_UI_DEBUG").ok().as_deref() {
            Some("settings") => (true, true),
            Some("no-sidebar") => (false, false),
            _ => (true, false),
        }
    }
    #[cfg(not(debug_assertions))]
    {
        (true, false)
    }
}

#[derive(Clone, Copy)]
enum ArrowSide {
    Prev,
    Next,
}

/// A pending in-chapter TOC jump: the fragment to resolve once block probes are available.
#[derive(Clone, Debug)]
struct PendingAnchor {
    anchor: String,
    /// Frames spent waiting for probe positions (bounded retry, never an endless notify loop).
    attempts: u8,
}

/// Reader view (sidebar + top bar + viewer + arrow gutters).
pub struct Reader {
    book: Arc<Book>,
    settings: Settings,
    chapter: usize,
    /// Last known scroll ratio; refreshed from the `ScrollHandle` each render and used as fallback
    /// when the handle has no scrollable range yet (e.g. before first layout).
    scroll_ratio: f32,
    root: WeakEntity<Root>,
    scroll_handle: ScrollHandle,
    focus_handle: FocusHandle,
    sidebar_open: bool,
    settings_open: bool,
    chapter_cache: Option<(usize, Arc<BlockModel>)>,
    image_cache: ImageCache,
    error: Option<String>,
    /// Ratio to apply once the newly rendered chapter has a measurable scroll range.
    restore_ratio: Option<f32>,
    restore_attempts: u8,
    /// TOC entry index currently highlighted (clicked sub-item or first entry of the chapter).
    selected_toc: Option<usize>,
    /// In-chapter jump waiting for the next layout pass to expose block positions.
    pending_anchor: Option<PendingAnchor>,
    /// Per-frame block positions recorded by `render::AnchorProbe` during prepaint.
    anchor_positions: AnchorPositions,
    /// Current TOC sidebar width as a window fraction (resizable by dragging the handle).
    sidebar_width: f32,
    /// True while the sidebar resize drag is in progress (Root settings must not override it).
    resizing_sidebar: bool,
    /// Timestamp of the last single click on the resize handle (manual double-click detection,
    /// independent of platform click counting).
    last_handle_click: Option<std::time::Instant>,
    /// Sidebar TOC scroll handle (auto-scrolled to keep the active section visible).
    toc_scroll_handle: ScrollHandle,
    /// Per-frame TOC row positions recorded by `AnchorProbe` (row index → laid-out y).
    toc_row_positions: AnchorPositions,
    /// Last content scroll offset used for TOC sync (avoids redundant polling work).
    last_reading_offset: f32,
    /// Sidebar viewport height captured during render (used by TOC auto-scroll).
    sidebar_viewport_height: f32,
}

impl Reader {
    /// Frozen constructor used by `Root` / the open flow.
    pub fn new(
        book: Arc<Book>,
        settings: Settings,
        initial: Position,
        root: WeakEntity<Root>,
        cx: &mut Context<Self>,
    ) -> Self {
        let chapter = initial
            .chapter_index
            .min(book.chapter_count().saturating_sub(1));
        let (sidebar_open, settings_open) = debug_initial_ui_state();
        let sidebar_width = crate::theme::clamp_sidebar_width_stored(settings.sidebar_width);
        let mut reader = Self {
            book,
            settings,
            chapter,
            scroll_ratio: initial.scroll_ratio.clamp(0.0, 1.0),
            root,
            scroll_handle: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            sidebar_open,
            settings_open,
            chapter_cache: None,
            image_cache: ImageCache::new(),
            error: None,
            restore_ratio: None,
            restore_attempts: 0,
            selected_toc: None,
            pending_anchor: None,
            anchor_positions: anchor_positions(),
            sidebar_width,
            resizing_sidebar: false,
            last_handle_click: None,
            toc_scroll_handle: ScrollHandle::new(),
            toc_row_positions: anchor_positions(),
            last_reading_offset: -1.0,
            sidebar_viewport_height: 1.0,
        };
        reader.load_chapter();
        if reader.scroll_ratio > 0.0005 {
            reader.restore_ratio = Some(reader.scroll_ratio);
        }
        reader.selected_toc = reader.first_toc_index_for(reader.chapter);
        reader.spawn_toc_sync(cx);
        reader
    }

    pub fn book(&self) -> &Arc<Book> {
        &self.book
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn chapter(&self) -> usize {
        self.chapter
    }

    pub fn sidebar_open(&self) -> bool {
        self.sidebar_open
    }

    pub fn settings_open(&self) -> bool {
        self.settings_open
    }

    /// Live reading position: current chapter + scroll ratio read from the scroll handle.
    pub fn position(&self) -> Position {
        Position {
            chapter_index: self.chapter,
            scroll_ratio: self.current_ratio(),
        }
    }

    fn current_ratio(&self) -> f32 {
        let max: f32 = self.scroll_handle.max_offset().height.into();
        if max > 0.0 {
            let offset: f32 = (-self.scroll_handle.offset().y).into();
            ratio_from_offset(offset, max)
        } else {
            self.scroll_ratio
        }
    }
    /// Switch to `index` (clamped to the spine), load it and jump to its start.
    pub fn switch_chapter(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        self.switch_chapter_inner(index, cx);
    }

    fn switch_chapter_inner(&mut self, index: usize, cx: &mut Context<Self>) {
        let count = self.book.chapter_count();
        if count == 0 {
            return;
        }
        let index = index.min(count - 1);
        let already_loaded =
            self.chapter_cache.as_ref().map(|(cached, _)| *cached == index) == Some(true);
        if index == self.chapter && already_loaded {
            return;
        }
        self.chapter = index;
        self.load_chapter();
        self.scroll_handle.set_offset(point(px(0.0), px(0.0)));
        self.scroll_ratio = 0.0;
        self.restore_ratio = None;
        self.restore_attempts = 0;
        self.last_reading_offset = -1.0;
        // Chapter-level navigation (arrows, TOC chapter rows) highlights the first entry that maps
        // to the new chapter; clicking a chapter sub-item overrides this afterwards.
        self.selected_toc = self.first_toc_index_for(index);
        self.pending_anchor = None;
        self.anchor_positions.borrow_mut().clear();
        cx.notify();
    }

    pub fn prev_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chapter > 0 {
            self.switch_chapter(self.chapter - 1, window, cx);
        }
    }

    pub fn next_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chapter + 1 < self.book.chapter_count() {
            self.switch_chapter(self.chapter + 1, window, cx);
        }
    }

    /// × button: close is Root's job (it owns the book path). Snapshot the position here — this
    /// handler runs while the `Reader` entity is already being updated, so Root must not read the
    /// reader (GPUI double-lease panic).
    pub fn close_book(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let position = self.position();
        let _ = self
            .root
            .update(cx, |root, cx| root.close_book_at(Some(position), cx));
    }

    /// Close the settings popover (wired to Esc by `Root::on_dismiss`).
    pub fn dismiss_popover(&mut self, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_open = false;
            cx.notify();
        }
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        cx.notify();
    }

    fn load_chapter(&mut self) {
        match self.book.chapter(self.chapter) {
            Ok(model) => {
                self.chapter_cache = Some((self.chapter, model));
                self.error = None;
            }
            Err(error) => {
                self.chapter_cache = None;
                self.error = Some(format!(
                    "Could not load chapter {}: {error}",
                    self.chapter + 1
                ));
            }
        }
    }

    fn ensure_chapter(&mut self) {
        let loaded = self.chapter_cache.as_ref().map(|(index, _)| *index);
        if loaded != Some(self.chapter) {
            self.load_chapter();
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_chapter();
        cx.notify();
    }

    /// Root's settings are authoritative (theme/font-size persistence lives there); pick them up.
    fn sync_settings(&mut self, cx: &mut Context<Self>) {
        let root_settings = match self.root.upgrade() {
            Some(root) => root.read(cx).settings(),
            None => return,
        };
        if root_settings.font_size != self.settings.font_size {
            // Preserve the reading ratio across font-size changes (the scroll range changes).
            let ratio = self.current_ratio();
            self.restore_ratio = Some(ratio);
            self.restore_attempts = 0;
        }
        if !self.resizing_sidebar {
            self.sidebar_width = crate::theme::clamp_sidebar_width_stored(root_settings.sidebar_width);
        }
        self.settings = root_settings;
    }

    /// Viewport-clamped sidebar fraction actually used for layout.
    fn effective_sidebar_width(&self, window: &Window) -> f32 {
        let viewport: f32 = window.viewport_size().width.into();
        clamp_sidebar_fraction(viewport, self.sidebar_width)
    }

    /// Invisible 6 px drag handle between the TOC sidebar and the reading area.
    fn sidebar_resize_handle(&self, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.settings.theme.selected_toc();
        div()
            .id("sidebar-resize-handle")
            .w(px(SIDEBAR_HANDLE_WIDTH))
            .h_full()
            .flex_shrink_0()
            .cursor(CursorStyle::ResizeLeftRight)
            .bg(self.settings.theme.content_bg())
            .hover(move |style| style.bg(accent.opacity(0.22)))
            .on_drag(SidebarDrag, move |_, _position, _, cx| {
                cx.new(|_| SidebarDragPreview { color: accent })
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::on_sidebar_handle_mouse_down),
            )
            .into_any_element()
    }

    fn on_sidebar_drag_move(
        &mut self,
        event: &DragMoveEvent<SidebarDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let viewport: f32 = window.viewport_size().width.into();
        let fraction = f32::from(event.event.position.x) / viewport.max(1.0);
        self.sidebar_width = clamp_sidebar_fraction(viewport, fraction);
        self.resizing_sidebar = true;
        cx.notify();
    }

    fn on_sidebar_drop(
        &mut self,
        _: &SidebarDrag,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resizing_sidebar = false;
        let width = self.sidebar_width;
        let _ = self.root.update(cx, |root, cx| root.set_sidebar_width(width, cx));
    }

    /// Double-click the handle resets the sidebar to the default 20%.
    ///
    /// Detection happens on mouse-down with our own 400 ms window, because the element owns a drag
    /// listener (so `on_click` is not delivered) and synthetic/automated input may not expose the
    /// platform click count.
    fn on_sidebar_handle_mouse_down(
        &mut self,
        _event: &gpui::MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        let elapsed_ms = self
            .last_handle_click
            .map(|last| now.duration_since(last).as_millis())
            .unwrap_or(u128::MAX);
        let is_double_click = elapsed_ms < 600;
        self.last_handle_click = if is_double_click { None } else { Some(now) };
        if !is_double_click {
            return;
        }
        self.sidebar_width = SIDEBAR_FRACTION;
        let width = self.sidebar_width;
        let _ = self.root.update(cx, |root, cx| root.set_sidebar_width(width, cx));
        cx.notify();
    }

    fn sync_scroll_ratio(&mut self) {
        self.scroll_ratio = self.current_ratio();
    }

    /// Apply a pending scroll ratio once the chapter layout exposes a scroll range.
    ///
    /// `max_offset` is only known after a layout pass, so a failed attempt schedules itself after
    /// the current frame via `cx.defer_in` (bounded to avoid an endless defer chain).
    fn apply_restore_ratio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ratio) = self.restore_ratio else {
            return;
        };
        let max: f32 = self.scroll_handle.max_offset().height.into();
        if max > 1.0 {
            let offset = offset_from_ratio(ratio, max);
            self.scroll_handle.set_offset(point(px(0.0), px(-offset)));
            self.restore_ratio = None;
            self.restore_attempts = 0;
            self.scroll_ratio = ratio.clamp(0.0, 1.0);
            cx.notify();
        } else if self.restore_attempts < 8 {
            self.restore_attempts += 1;
            cx.defer_in(window, |reader, window, cx| {
                reader.apply_restore_ratio(window, cx);
            });
        } else {
            self.restore_ratio = None;
            self.restore_attempts = 0;
        }
    }

    /// TOC entries used by the sidebar: engine TOC when present, spine fallback otherwise.
    fn toc_entries(&self) -> Vec<TocItem> {
        if self.book.toc().is_empty() {
            toc::spine_fallback_labels(&self.book)
                .into_iter()
                .map(|(label, chapter_index)| TocItem {
                    label,
                    depth: 0,
                    chapter_index: Some(chapter_index),
                    anchor: None,
                })
                .collect()
        } else {
            self.book.toc().to_vec()
        }
    }

    /// First TOC entry mapping to `chapter` (used for chapter-level highlight sync).
    fn first_toc_index_for(&self, chapter: usize) -> Option<usize> {
        self.toc_entries()
            .iter()
            .position(|entry| entry.chapter_index == Some(chapter))
    }

    /// Background poller that keeps the TOC highlight + scroll position in sync with the reading
    /// position. GPUI scroll containers repaint without notifying the parent view, so a lightweight
    /// 120 ms poll is the least invasive way to observe the scroll offset.
    fn spawn_toc_sync(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| {
            loop {
                smol::Timer::after(std::time::Duration::from_millis(120)).await;
                if weak
                    .update(cx, |reader, cx| reader.sync_toc_to_reading_position(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Map the current scroll offset to the TOC entry whose anchor block is nearest above the
    /// viewport top; update the highlight and auto-scroll the sidebar to keep that row visible.
    fn sync_toc_to_reading_position(&mut self, cx: &mut Context<Self>) -> bool {
        // Do not fight programmatic jumps: wait until the anchor/restore pass settles.
        if self.pending_anchor.is_some() || self.restore_ratio.is_some() {
            self.last_reading_offset = -1.0;
            return false;
        }
        let max: f32 = self.scroll_handle.max_offset().height.into();
        if max <= 1.0 {
            return false;
        }
        let offset: f32 = (-self.scroll_handle.offset().y).into();
        let offset = offset.clamp(0.0, max);
        if (offset - self.last_reading_offset).abs() < 0.5 {
            return false;
        }
        self.last_reading_offset = offset;

        let Some((cached, model)) = &self.chapter_cache else {
            return false;
        };
        if *cached != self.chapter {
            return false;
        }
        let positions = self.anchor_positions.borrow();
        let Some(&origin) = positions.get(&0) else {
            return false;
        };
        let mut target_block = 0usize;
        for (&block, &y) in positions.iter() {
            if y - origin <= offset + 6.0 && block >= target_block {
                target_block = block;
            }
        }
        drop(positions);

        let candidates: Vec<(usize, usize)> = {
            let entries = self.toc_entries();
            let mut candidates: Vec<(usize, usize)> = entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    if entry.chapter_index != Some(self.chapter) {
                        return None;
                    }
                    let block = entry
                        .anchor
                        .as_deref()
                        .and_then(|anchor| model.anchor_block(anchor))
                        .unwrap_or(0);
                    Some((block, index))
                })
                .collect();
            candidates.sort_by_key(|(block, _)| *block);
            candidates
        };
        let selected = candidates
            .iter()
            .rev()
            .find(|(block, _)| *block <= target_block)
            .map(|(_, index)| *index)
            .or_else(|| candidates.first().map(|(_, index)| *index));        let Some(selected) = selected else {
            return false;
        };

        let mut changed = false;
        if self.selected_toc != Some(selected) {
            self.selected_toc = Some(selected);
            changed = true;
        }
        if self.sidebar_open {
            let rows = self.toc_row_positions.borrow();
            if let (Some(&row_origin), Some(&row_y)) = (rows.get(&0), rows.get(&selected)) {
                let row_content = (row_y - row_origin).max(0.0);
                let toc_offset: f32 = (-self.toc_scroll_handle.offset().y).into();
                let toc_offset = toc_offset.max(0.0);
                let height = self.sidebar_viewport_height.max(1.0);
                let row_view_y = row_content - toc_offset;
                let desired = if row_view_y < 24.0 {
                    (row_content - 12.0).max(0.0)
                } else if row_view_y > height - 24.0 {
                    (row_content - height + 48.0).max(0.0)
                } else {
                    toc_offset
                };
                let toc_max: f32 = self.toc_scroll_handle.max_offset().height.into();
                let desired = desired.clamp(0.0, toc_max.max(0.0));
                if (desired - toc_offset).abs() > 0.5 {
                    self.toc_scroll_handle
                        .set_offset(point(px(0.0), px(-desired)));
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
        changed
    }

    /// Navigate an in-content `<a href>`: resolve relative path + fragment, switch chapter when
    /// needed, then reuse the pending-anchor jump machinery used by TOC clicks.
    fn navigate_href(&mut self, href: &str, cx: &mut Context<Self>) {
        let Some((target_chapter, anchor)) = self.book.resolve_href(self.chapter, href) else {
            return;
        };

        if target_chapter != self.chapter {
            self.switch_chapter_inner(target_chapter, cx);
        }
        if let Some(anchor) = anchor.as_deref()
            && let Some(index) = self.toc_entries().iter().position(|entry| {
                entry.chapter_index == Some(target_chapter)
                    && entry.anchor.as_deref() == Some(anchor)
            })
        {
            self.selected_toc = Some(index);
        }
        self.pending_anchor = anchor
            .filter(|anchor| !anchor.trim().is_empty())
            .map(|anchor| PendingAnchor {
                anchor,
                attempts: 0,
            });
        self.restore_ratio = None;
        self.restore_attempts = 0;
        self.last_reading_offset = -1.0;
        if self.pending_anchor.is_none() {
            self.scroll_handle.set_offset(point(px(0.0), px(0.0)));
            self.scroll_ratio = 0.0;
        }
        // Note: do NOT clear `anchor_positions` here. For same-chapter link jumps the map from the
        // previous frame is exactly what `apply_pending_anchor` needs; chapter switches already
        // clear it inside `switch_chapter_inner`.
        cx.notify();
    }

    /// Handle a TOC row click: chapter switch (when needed) + in-chapter jump to the row fragment.
    fn on_toc_click(
        &mut self,
        row: usize,
        chapter_index: Option<usize>,
        anchor: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_chapter) = chapter_index else {
            // Entry points outside the spine (or has no resolvable chapter): keep the reader as-is.
            return;
        };
        if target_chapter != self.chapter {
            self.switch_chapter(target_chapter, window, cx);
        }
        // The clicked row stays highlighted, including nested sub-items inside the current chapter.
        self.selected_toc = Some(row);

        match anchor.filter(|anchor| !anchor.trim().is_empty()) {
            Some(anchor) => {
                self.pending_anchor = Some(PendingAnchor {
                    anchor,
                    attempts: 0,
                });
                self.restore_ratio = None;
                self.restore_attempts = 0;
            }
            None => {
                // No fragment: jump to chapter start.
                self.pending_anchor = None;
                self.scroll_handle.set_offset(point(px(0.0), px(0.0)));
                self.scroll_ratio = 0.0;
            }
        }
        cx.notify();
    }

    /// Apply a pending in-chapter jump once the layout pass has recorded block positions.
    ///
    /// The positions map is filled during *prepaint*, so the first render after a click can only
    /// read positions from the previous frame (or none at all for a freshly switched chapter). The
    /// jump is therefore retried for at most two more frames; a bounded counter plus `cx.notify`
    /// keeps this from becoming a render loop.
    fn apply_pending_anchor(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_anchor.clone() else {
            return;
        };
        let Some((cached, model)) = &self.chapter_cache else {
            self.pending_anchor = None;
            return;
        };
        if *cached != self.chapter {
            self.pending_anchor = None;
            return;
        }
        let Some(target_block) = model.anchor_block(&pending.anchor) else {
            // Fragment is not in this chapter (engine may not have captured it): stop trying, but
            // keep the clicked TOC row highlighted so the user still sees the selection.
            self.pending_anchor = None;
            return;
        };

        let positions = self.anchor_positions.borrow();
        let origin = positions.get(&0).copied();
        let target = positions.get(&target_block).copied();
        drop(positions);

        let (Some(origin), Some(target)) = (origin, target) else {
            let attempts = self
                .pending_anchor
                .as_mut()
                .map(|pending| {
                    pending.attempts += 1;
                    pending.attempts
                })
                .unwrap_or(0);
            if attempts > 2 {
                self.pending_anchor = None;
            } else {
                // Another frame is needed for the probes to lay out.
                cx.notify();
            }
            return;
        };

        let relative = (target - origin).max(0.0);
        let max: f32 = self.scroll_handle.max_offset().height.into();
        let offset = relative.min(max.max(0.0));

        self.scroll_handle.set_offset(point(px(0.0), px(-offset)));
        self.scroll_ratio = ratio_from_offset(offset, max);
        self.pending_anchor = None;
    }

    fn content_width(&self, window: &Window) -> f32 {
        let viewport: f32 = window.viewport_size().width.into();
        let sidebar = if self.sidebar_open {
            viewport * self.effective_sidebar_width(window)
        } else {
            0.0
        };
        (viewport - sidebar - 2.0 * GUTTER_WIDTH).max(200.0)
    }

    fn chapter_content(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.settings.theme;
        if let Some((cached, model)) = &self.chapter_cache {
            if *cached == self.chapter {
                let width = self.content_width(window);
                // Fresh probe map for this render pass; prepaint refills it for every block.
                self.anchor_positions.borrow_mut().clear();
                let link_handler: LinkHandler = {
                    let entity = cx.entity();
                    Rc::new(move |href: &str, _window: &mut Window, app: &mut gpui::App| {
                        let href = href.to_string();
                        entity.update(app, |reader, cx| reader.navigate_href(&href, cx));
                    })
                };
                let blocks = render_blocks_full(
                    model.as_ref(),
                    self.book.as_ref(),
                    self.chapter,
                    &self.settings,
                    width,
                    &mut self.image_cache,
                    Some(&self.anchor_positions),
                    Some(&link_handler),
                );
                // Apply the book's `<body>` horizontal margins/padding around the reading column
                // (source CSS commonly uses `body { margin: 0 5% }`). Percentages resolve against
                // the viewer width via GPUI `relative`, matching the browser layout.
                let body = model.body_style.clone();
                let scale =
                    (self.settings.font_size / crate::theme::CONTENT_DEFAULT_FONT_SIZE).clamp(0.4, 3.0);
                let mut content = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .text_color(theme.content_text());
                if body.padding_left_percent > 0.0 {
                    content = content.pl(relative(body.padding_left_percent / 100.0));
                } else if body.padding_left > 0.0 {
                    content = content.pl(px(body.padding_left * scale));
                }
                if body.padding_right_percent > 0.0 {
                    content = content.pr(relative(body.padding_right_percent / 100.0));
                } else if body.padding_right > 0.0 {
                    content = content.pr(px(body.padding_right * scale));
                }
                return content.children(blocks).into_any_element();
            }
        }
        // Error panel: the reader stays usable; Retry re-parses while arrows/TOC keep working.
        let message = self
            .error
            .clone()
            .unwrap_or_else(|| "This chapter has no content.".to_string());
        let retry = div()
            .id("chapter-retry")
            .mt(px(8.0))
            .px(px(12.0))
            .py(px(6.0))
            .rounded_sm()
            .border_1()
            .border_color(theme.border())
            .text_size(px(13.0))
            .cursor_pointer()
            .child("Retry")
            .on_click(cx.listener(|reader: &mut Reader, _event, _window, cx| {
                reader.reload(cx);
            }));
        div()
            .w_full()
            .p(px(24.0))
            .flex()
            .flex_col()
            .gap_2()
            .text_color(theme.content_text())
            .child(div().text_size(px(15.0)).child(message))
            .child(retry)
            .into_any_element()
    }

    fn settings_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.clone();
        let theme = settings.theme;
        let entity = cx.entity();

        let on_size = {
            let entity = entity.clone();
            move |target: f32, _event: &ClickEvent, _window: &mut Window, app: &mut gpui::App| {
                let target = target.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
                entity.update(app, |reader, cx| {
                    reader.settings.font_size = target;
                    let root = reader.root.clone();
                    let _ = root.update(cx, |root, cx| root.set_font_size(target, cx));
                    cx.notify();
                });
            }
        };
        let on_theme = {
            let entity = entity.clone();
            move |next: Theme, _event: &ClickEvent, _window: &mut Window, app: &mut gpui::App| {
                entity.update(app, |reader, cx| {
                    reader.settings.theme = next;
                    let root = reader.root.clone();
                    let _ = root.update(cx, |root, cx| root.set_theme(next, cx));
                    cx.notify();
                });
            }
        };
        let on_outside = {
            let entity = entity.clone();
            move |_event: &gpui::MouseDownEvent, _window: &mut Window, app: &mut gpui::App| {
                entity.update(app, |reader, cx| {
                    if reader.settings_open {
                        reader.settings_open = false;
                        cx.notify();
                    }
                });
            }
        };

        let popover = settings_popover::popover(&settings, on_size, on_theme, on_outside);

        let gear = components::toolbar_icon("settings-toggle", hover_background(theme))
            .child(icons::gear_icon(theme.gear_icon()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|reader: &mut Reader, _event, _window, cx| {
                    reader.toggle_settings(cx);
                }),
            );

        div()
            .relative()
            .child(gear)
            .when(self.settings_open, |this| {
                this.child(gpui::deferred(
                    div()
                        .absolute()
                        .top(px(TOP_BAR_HEIGHT - 2.0))
                        .left(px(-42.0))
                        .child(popover),
                ))
            })
            .into_any_element()
    }

    fn toc_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.settings.theme;
        let mut rows: Vec<AnyElement> = Vec::new();

        let entries = self.toc_entries();

        // Highlight the clicked TOC row (including sub-items inside the current chapter). On
        // chapter changes `switch_chapter` already resets this to the first entry of the chapter.
        let selected_index = self
            .selected_toc
            .filter(|index| *index < entries.len())
            .or_else(|| entries.iter().position(|entry| entry.chapter_index == Some(self.chapter)));

        self.toc_row_positions.borrow_mut().clear();
        for (index, entry) in entries.iter().enumerate() {
            let chapter_index = entry.chapter_index;
            let anchor = entry.anchor.clone();
            let selected = selected_index == Some(index);
            let label = entry.label.clone();
            let depth = entry.depth;
            let on_click = cx.listener(
                move |reader: &mut Reader, _event: &ClickEvent, window: &mut Window, cx| {
                    reader.on_toc_click(index, chapter_index, anchor.clone(), window, cx);
                },
            );
            // Probe before the row records its laid-out top for TOC auto-scroll.
            rows.push(
                AnchorProbe::new(index, &self.toc_row_positions).into_any_element(),
            );
            rows.push(toc::toc_row(index, &label, depth, selected, theme, on_click));
        }

        div()
            .id("toc-sidebar")
            .w(relative(self.effective_sidebar_width(window)))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(theme.sidebar_bg())
            .border_r_1()
            .border_color(theme.border())
            .overflow_y_scroll()
            .track_scroll(&self.toc_scroll_handle)
            .child(toc::toc_list(rows, theme))
            .into_any_element()
    }

    fn top_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.settings.theme;

        let toc_toggle = components::icon_button(
            "toc-toggle",
            icons::toc_icon(theme.gear_icon()),
            hover_background(theme),
            cx.listener(|reader: &mut Reader, _event: &ClickEvent, _window, cx| {
                reader.toggle_sidebar(cx);
            }),
        );

        let close_button = components::icon_button(
            "close-book",
            icons::close_icon(theme.gear_icon()),
            hover_background(theme),
            cx.listener(|reader: &mut Reader, _event: &ClickEvent, window, cx| {
                reader.close_book(window, cx);
            }),
        );

        let settings_button = self.settings_button(cx);

        div()
            .id("reader-top-bar")
            .w_full()
            .h(px(TOP_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .px(px(10.0))
            .bg(theme.content_bg())
            .child(toc_toggle)
            .child(settings_button)
            .child(div().flex_1())
            .child(close_button)
            .into_any_element()
    }

    fn arrow_gutter(
        &mut self,
        side: ArrowSide,
        enabled: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let glyph = match side {
            ArrowSide::Prev => icons::chevron_left(theme.arrow_icon()),
            ArrowSide::Next => icons::chevron_right(theme.arrow_icon()),
        };
        let id: &'static str = match side {
            ArrowSide::Prev => "gutter-prev",
            ArrowSide::Next => "gutter-next",
        };
        let mut gutter = div()
            .id(id)
            .absolute()
            .top(px(0.0))
            .bottom(px(0.0))
            .w(px(GUTTER_WIDTH))
            .flex()
            .items_center()
            .justify_center();
        gutter = match side {
            ArrowSide::Prev => gutter.left(px(0.0)),
            ArrowSide::Next => gutter.right(px(0.0)),
        };
        if enabled {
            let hover_bg = theme.arrow_icon().opacity(0.06);
            gutter = gutter
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .on_click(
                    cx.listener(
                        move |reader: &mut Reader, _event: &ClickEvent, window, cx| match side {
                            ArrowSide::Prev => reader.prev_chapter(window, cx),
                            ArrowSide::Next => reader.next_chapter(window, cx),
                        },
                    ),
                );
        }
        gutter.child(glyph).into_any_element()
    }

    fn viewer(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.settings.theme;
        let content = self.chapter_content(window, cx);
        div()
            .id("chapter-viewer")
            .absolute()
            .left(px(GUTTER_WIDTH))
            .right(px(GUTTER_WIDTH))
            .top(px(0.0))
            .bottom(px(BOTTOM_INSET))
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .text_color(theme.content_text())
            .child(content)
            .into_any_element()
    }
}

impl Render for Reader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sidebar_viewport_height =
            (f32::from(window.viewport_size().height) - TOP_BAR_HEIGHT).max(1.0);
        self.sync_settings(cx);
        self.ensure_chapter();
        self.sync_scroll_ratio();
        // In-chapter TOC jumps are applied before layout/paint of this frame, using the block
        // positions recorded by the previous frame's probes; retried (bounded) for fresh chapters.
        self.apply_pending_anchor(cx);
        self.apply_restore_ratio(window, cx);

        // Esc is handled by `Root::on_dismiss`, which forwards to `Reader::dismiss_popover` (the
        // Root view owns focus, so a per-view action listener here would not be in the dispatch
        // path; registering a window action from a child view's render also panics in GPUI 0.2.2).

        let theme = self.settings.theme;
        let chapter_count = self.book.chapter_count();
        let can_prev = self.chapter > 0;
        let can_next = chapter_count > 0 && self.chapter + 1 < chapter_count;

        let sidebar = if self.sidebar_open {
            Some(self.toc_sidebar(window, cx))
        } else {
            None
        };
        let resize_handle = if self.sidebar_open {
            Some(self.sidebar_resize_handle(cx))
        } else {
            None
        };
        let top_bar = self.top_bar(cx);
        let prev = self.arrow_gutter(ArrowSide::Prev, can_prev, theme, cx);
        let next = self.arrow_gutter(ArrowSide::Next, can_next, theme, cx);
        let viewer = self.viewer(window, cx);

        let main = div()
            .flex_1()
            .relative()
            .h_full()
            .child(prev)
            .child(viewer)
            .child(next);

        let body = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .child(top_bar)
            .child(main);

        div()
            .size_full()
            .flex()
            .flex_row()
            .key_context("Reader")
            .track_focus(&self.focus_handle)
            .bg(theme.content_bg())
            .on_drag_move::<SidebarDrag>(cx.listener(Self::on_sidebar_drag_move))
            .on_drop(cx.listener(Self::on_sidebar_drop))
            .children(sidebar)
            .children(resize_handle)
            .child(body)
    }
}

/// Handle type kept for the open-flow lane (`Entity<Reader>`).
pub type ReaderEntity = gpui::Entity<Reader>;






