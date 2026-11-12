//! Landing screen: open-book card + recent books card.
//!
//! Parity values come from the design spec §7.1 (the source app's CSS): 950 px centered
//! container, two 454 px white cards, vertical `#d7d7d7 -> #f0f0f0` gradient background,
//! `#00c0b8` primary button, 128 px logo, 48x64 recent covers.
//!
//! The landing view is a thin presentation layer: it owns no book state. It reads the
//! `Root` entity (recents/error) at render time and dispatches [`LandingAction`]s back to
//! `Root`, which owns the open/remove/dismiss flows. That keeps the screen stateless enough
//! to re-render from `Root::state` whenever `Root` notifies.

use crate::app::Root;
use crate::persistence::{self, RecentBook};
use crate::ui::components::error_banner;
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, Hsla, Image, ImageFormat, ObjectFit, Render,
    SharedString, WeakEntity, Window, div, img, linear_color_stop, linear_gradient, px, rgb,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Width of the centered content container (source: `.container { width: 950px }`).
const CONTAINER_WIDTH: f32 = 950.0;
/// Width of each landing card (source: `.card { width: 454px }`).
const CARD_WIDTH: f32 = 454.0;
/// Gap left over by two 454 px cards inside the 950 px container.
const CARD_GAP: f32 = CONTAINER_WIDTH - 2.0 * CARD_WIDTH;
/// The recents policy caps the list at 8; the card shows them all (scrolls past ~6 rows).
const MAX_VISIBLE_RECENTS: usize = 8;
/// Cap the recent list height so the card still fits the 900x600 minimum window.
const RECENT_LIST_MAX_HEIGHT: f32 = 448.0;
/// Logo drawn from the same PNG that is embedded as the app icon.
const LOGO_PNG: &[u8] = include_bytes!("../../assets/icon.png");

/// User intent produced by the landing screen. `Root` owns the actual behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LandingAction {
    /// Open the native file dialog (primary button / drop zone click / Ctrl+O).
    OpenDialog,
    /// Open a recent book row.
    OpenRecent(PathBuf),
    /// Remove a recent book row (× button).
    RemoveRecent(PathBuf),
    /// Dismiss the shared error banner.
    DismissError,
    /// Show the About overlay (version / build toolchain / packages).
    ShowAbout,
}

pub struct Landing {
    root: WeakEntity<Root>,
    logo: Arc<Image>,
}

impl Landing {
    pub fn new(root: WeakEntity<Root>) -> Self {
        Self {
            root,
            // Decode the embedded PNG once; `Arc<Image>` keeps the asset cache key stable.
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, LOGO_PNG.to_vec())),
        }
    }

    /// Dispatch a landing action to `Root`. No-op if the root entity is gone.
    pub fn handle_action(&mut self, action: LandingAction, cx: &mut Context<Self>) {
        let Some(root) = self.root.upgrade() else {
            return;
        };
        match action {
            LandingAction::OpenDialog => {
                root.update(cx, |root, cx| root.request_open(cx));
            }
            LandingAction::OpenRecent(path) => {
                root.update(cx, |root, cx| root.open_path(path, cx));
            }
            LandingAction::RemoveRecent(path) => {
                root.update(cx, |root, cx| root.remove_recent(path, cx));
            }
            LandingAction::DismissError => {
                root.update(cx, |root, cx| root.dismiss_error(cx));
            }
            LandingAction::ShowAbout => {
                root.update(cx, |root, cx| root.show_about(cx));
            }
        }
    }

    fn on_action_click(
        &mut self,
        action: LandingAction,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_action(action, cx);
    }

    fn on_remove_click(
        &mut self,
        action: LandingAction,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click on the × must not also open the book via the row's click handler.
        cx.stop_propagation();
        self.handle_action(action, cx);
    }

    fn logo_element(&self) -> AnyElement {
        img(self.logo.clone())
            .w(px(128.))
            .h(px(128.))
            .object_fit(ObjectFit::Contain)
            .mb(px(25.))
            .into_any_element()
    }

    fn primary_button(&self, cx: &Context<Self>) -> AnyElement {
        let action = LandingAction::OpenDialog;
        div()
            .id("landing-open-button")
            .mt(px(20.))
            .w(px(300.))
            .h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .bg(color(0x00c0b8))
            .text_color(color(0xffffff))
            .text_size(px(16.))
            .font_weight(FontWeight::BOLD)
            .cursor_pointer()
            .hover(|style| style.bg(color(0x00aea7)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_action_click(action.clone(), event, window, cx)
            }))
            .child("Open EPUB book")
            .into_any_element()
    }

    fn drop_zone(&self, cx: &Context<Self>) -> AnyElement {
        let action = LandingAction::OpenDialog;
        div()
            .id("landing-drop-zone")
            .mt(px(12.))
            .w(px(300.))
            .h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(2.))
            .border_2()
            .border_color(color(0x999999))
            .text_color(color(0x999999))
            .text_size(px(14.))
            .cursor_pointer()
            .hover(|style| style.border_dashed().bg(color(0xfafafa)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_action_click(action.clone(), event, window, cx)
            }))
            .child("or drop an EPUB here")
            .into_any_element()
    }

    fn left_card(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .id("landing-open-card")
            .w(px(CARD_WIDTH))
            .bg(color(0xffffff))
            .rounded(px(5.))
            .shadow_sm()
            .pt(px(20.))
            .px(px(30.))
            .pb(px(30.))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .child(self.logo_element())
            .child(
                div()
                    .text_size(px(18.))
                    .line_height(px(22.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(color(0x333333))
                    .child("Hello, Welcome to Another Epub Reader"),
            )
            .child(
                div()
                    .pt(px(10.))
                    .text_size(px(16.))
                    .text_color(color(0x666666))
                    .child("Click button to select your book"),
            )
            .child(self.primary_button(cx))
            .child(
                div()
                    .pt(px(10.))
                    .text_size(px(14.))
                    .text_color(color(0x999999))
                    .child("A native, offline reading experience. No browser, no account."),
            )
            .child(self.drop_zone(cx))
            .child(div().flex_1().min_h(px(24.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .text_color(color(0x999999))
                            .child("Another Epub Reader"),
                    )
                    .child(
                        div()
                            .id("landing-about")
                            .text_size(px(13.))
                            .text_color(color(0x00c0b8))
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                                this.on_action_click(LandingAction::ShowAbout, event, window, cx)
                            }))
                            .child("About"),
                    ),
            )
            .into_any_element()
    }

    fn right_card(
        &self,
        recents: Vec<RecentBook>,
        error: Option<SharedString>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let now = persistence::now_unix();
        let mut card = div()
            .id("recent-books-card")
            .w(px(CARD_WIDTH))
            .bg(color(0xffffff))
            .rounded(px(5.))
            .shadow_sm()
            .flex()
            .flex_col();

        if let Some(message) = error {
            card = card.child(div().px(px(20.)).pt(px(16.)).child(error_banner(
                banner_message(message),
                cx.listener(|this, event: &ClickEvent, window, cx| {
                    this.on_action_click(LandingAction::DismissError, event, window, cx)
                }),
            )));
        }

        card = card.child(
            div()
                .pt(px(20.))
                .px(px(30.))
                .pb(px(8.))
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .text_color(color(0x333333))
                .child("Recent books"),
        );

        if recents.is_empty() {
            card = card.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(30.))
                    .pb(px(40.))
                    .text_size(px(14.))
                    .text_color(color(0x999999))
                    .text_center()
                    .child("No books yet. Open an EPUB to get started."),
            );
        } else {
            let rows: Vec<AnyElement> = recents
                .into_iter()
                .take(MAX_VISIBLE_RECENTS)
                .enumerate()
                .map(|(index, book)| self.recent_row(index, book, now, cx))
                .collect();
            card = card.child(
                div()
                    .id("recent-books-list")
                    .flex()
                    .flex_col()
                    .px(px(16.))
                    .pb(px(16.))
                    .max_h(px(RECENT_LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(rows),
            );
        }

        card.into_any_element()
    }

    fn recent_row(
        &self,
        index: usize,
        book: RecentBook,
        now: u64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let missing = !book.path.is_file();
        let open_action = LandingAction::OpenRecent(book.path.clone());
        let remove_action = LandingAction::RemoveRecent(book.path.clone());
        let group = SharedString::from(format!("recent-row-{index}"));
        let author = if book.author.trim().is_empty() {
            "Unknown author".to_string()
        } else {
            book.author.clone()
        };

        let status: AnyElement = if missing {
            div()
                .text_size(px(12.))
                .text_color(color(0xc0392b))
                .child("File not found — use × to remove")
                .into_any_element()
        } else {
            div()
                .text_size(px(12.))
                .text_color(color(0xaaaaaa))
                .child(persistence::relative_time(now, book.last_opened_unix))
                .into_any_element()
        };

        let remove_group = group.clone();
        let remove_color = if missing { color(0xc0392b) } else { color(0xd0d0d0) };
        let row = div()
            .id(("recent-row", index))
            .group(group)
            .flex()
            .items_center()
            .gap(px(12.))
            .p(px(8.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|style| style.bg(color(0xf7f7f7)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_action_click(open_action.clone(), event, window, cx)
            }))
            .child(cover_thumb(&book))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(if missing { color(0x777777) } else { color(0x333333) })
                            .line_clamp(2)
                            .child(book.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(color(0x999999))
                            .text_ellipsis()
                            .child(author),
                    )
                    .child(status),
            )
            .child(
                div()
                    .id(("recent-remove", index))
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .text_size(px(16.))
                    .text_color(remove_color)
                    .cursor_pointer()
                    .group_hover(remove_group, |style| style.text_color(color(0x666666)))
                    .hover(|style| style.bg(color(0xeaeaea)))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.on_remove_click(remove_action.clone(), event, window, cx)
                    }))
                    .child("×"),
            );
        row.into_any_element()
    }
}

impl Render for Landing {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (recents, error) = match self.root.upgrade() {
            Some(root) => {
                let root = root.read(cx);
                (
                    root.state.recent_books.clone(),
                    root.error
                        .clone()
                        .or_else(|| root.persist_error.clone())
                        .map(SharedString::from),
                )
            }
            None => (Vec::new(), None),
        };

        let stacked = window.bounds().size.width < px(CONTAINER_WIDTH);
        if stacked {
            // Narrow windows stack the cards; the two cards are taller than the 900x600 minimum
            // window, so the stack scrolls vertically instead of clipping.
            div()
                .id("landing-scroll")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(20.))
                .py(px(20.))
                .bg(landing_gradient())
                .child(self.left_card(cx))
                .child(self.right_card(recents, error, cx))
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(landing_gradient())
                .child(
                    div().w(px(CONTAINER_WIDTH)).child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(CARD_GAP))
                            .child(self.left_card(cx))
                            .child(self.right_card(recents, error, cx)),
                    ),
                )
                .into_any_element()
        }
    }
}

fn landing_gradient() -> gpui::Background {
    // Source uses a 6-stop CSS gradient; GPUI 0.2.2 only supports two stops, so we
    // interpolate between the first and last source stops (documented deviation).
    linear_gradient(
        180.,
        linear_color_stop(color(0xd7d7d7), 0.),
        linear_color_stop(color(0xf0f0f0), 1.),
    )
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Max characters per error-banner line. The right card is 454px wide with 20px insets and the
/// banner has 10px padding, so ~52 characters of 13px text fit before the card edge.
pub const BANNER_LINE_CHARS: usize = 52;

/// Format an open-error message so it never paints outside the 454px card.
///
/// DirectWrite word-wraps at spaces, and a Windows path is a single unbreakable token, so a raw
/// long path in the banner was clipped mid-character at the card edge. `File not found` errors are
/// rendered as explicit lines (prefix, file name, middle-elided path); other messages get newline
/// break opportunities injected into over-long tokens.
pub fn banner_message(message: SharedString) -> SharedString {
    let text = message.as_ref();
    if let Some(path) = text.strip_prefix("File not found: ") {
        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(path);
        return SharedString::from(format!(
            "File not found\n{}\n{}",
            clip_middle(file_name, BANNER_LINE_CHARS),
            clip_middle(path, BANNER_LINE_CHARS),
        ));
    }
    SharedString::from(break_long_tokens(text, BANNER_LINE_CHARS))
}

/// Keep a string readable within `max_chars` by eliding its middle (head/tail are preserved so the
/// drive/root and the file name stay visible).
fn clip_middle(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    let head = max_chars.saturating_sub(1) / 2;
    let tail = max_chars.saturating_sub(1 + head);
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(chars[chars.len() - tail..].iter());
    out
}

/// Insert newlines so no line is longer than `max_chars`; tokens are first tried whole (word
/// wrap), then hard-split (Windows paths have no break opportunities for DirectWrite).
fn break_long_tokens(text: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(1);
    let mut out = String::with_capacity(text.len() + 16);
    let mut line_len = 0usize;
    for (index, token) in text.split(' ').enumerate() {
        let token_chars: Vec<char> = token.chars().collect();
        if index > 0 {
            if line_len + 1 + token_chars.len() > max_chars {
                out.push('\n');
                line_len = 0;
            } else {
                out.push(' ');
                line_len += 1;
            }
        }
        let mut cursor = 0;
        while cursor < token_chars.len() {
            let space = max_chars - line_len;
            if space == 0 {
                out.push('\n');
                line_len = 0;
                continue;
            }
            let take = space.min(token_chars.len() - cursor);
            out.extend(&token_chars[cursor..cursor + take]);
            cursor += take;
            line_len += take;
            if cursor < token_chars.len() {
                out.push('\n');
                line_len = 0;
            }
        }
    }
    out
}

fn cover_thumb(book: &RecentBook) -> AnyElement {
    match book.cover.as_ref().filter(|path| path.is_file()) {
        Some(path) => div()
            .w(px(48.))
            .h(px(64.))
            .flex_shrink_0()
            .rounded(px(2.))
            // GPUI 0.2.2 paints `ObjectFit::Cover` images centred on their natural ratio and
            // does not clip them to the element bounds, so clip through a wrapper.
            .overflow_hidden()
            .child(
                img(path.clone())
                    .w(px(48.))
                    .h(px(64.))
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element(),
        None => placeholder_cover().into_any_element(),
    }
}

/// Drawn book glyph used when a recent entry has no cached cover (no icon font available).
fn placeholder_cover() -> gpui::Div {
    div()
        .w(px(48.))
        .h(px(64.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .bg(color(0xececec))
        .border_1()
        .border_color(color(0xd8d8d8))
        .child(
            div()
                .w(px(20.))
                .h(px(26.))
                .flex()
                .rounded(px(1.))
                .border_1()
                .border_color(color(0xb0b0b0))
                .child(div().w(px(3.)).h_full().bg(color(0xb0b0b0)))
                .child(
                    div()
                        .flex_1()
                        .px(px(2.))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(px(3.))
                        .child(div().w_full().h(px(2.)).bg(color(0xd0d0d0)))
                        .child(div().w_full().h(px(2.)).bg(color(0xd0d0d0)))
                        .child(div().w(px(8.)).h(px(2.)).bg(color(0xe0e0e0))),
                ),
        )
}
