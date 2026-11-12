//! Application root: screen state machine, book open/close flow, global actions.
//!
//! `Root` owns the persisted [`AppState`], the current [`Screen`], the open [`Book`] and the
//! `Reader`/`Landing` views. The open pipeline runs [`Book::open`] on the background executor;
//! the UI thread only swaps state once the parse finishes (or surfaces an error banner).

use crate::epub::model::Position;
use crate::persistence::AppState;
use crate::theme::{clamp_font_size, Settings, Theme};
use crate::ui::landing::Landing;
use crate::ui::reader::Reader;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Focusable, Hsla,
    Image, ImageFormat, Render, Window, div, linear_color_stop, linear_gradient, px, rgb,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

gpui::actions!(epub_reader_app, [PrevChapter, NextChapter, Dismiss, OpenBook]);

/// App icon (the PNG embedded as the executable's Windows resource) used by the About overlay.
const ABOUT_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Landing,
    Reader,
}

pub struct Root {
    pub state: AppState,
    pub screen: Screen,
    pub error: Option<String>,
    pub focus_handle: FocusHandle,
    pub reader: Option<Entity<Reader>>,
    pub landing: Option<Entity<Landing>>,
    pub book: Option<Arc<crate::epub::Book>>,
    /// Path of the currently open book. `Root` owns it because `Reader` only sees the `Book`.
    pub book_path: Option<PathBuf>,
    /// True while the background EPUB parse is in flight; the landing area shows a loading state.
    pub loading: bool,
    /// Last state-save failure (e.g. read-only data directory), shown on the landing screen so the
    /// user knows the library cannot be persisted. Cleared after a successful save.
    pub persist_error: Option<String>,
    /// Last title pushed to the platform window (avoids redundant platform calls per frame).
    window_title: Option<String>,
    /// True while the About overlay is visible (opened from the landing footer or the reader bar).
    about_open: bool,
    /// App icon shown in the About card; decoded once when the root is created.
    about_logo: Arc<Image>,
}

impl Root {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut state = AppState::load();
        state.settings.font_size = clamp_font_size(state.settings.font_size);

        let weak = cx.weak_entity();
        let landing = cx.new(move |_| Landing::new(weak));

        // Debug-only initial UI state hook, mirroring `ui::reader::debug_initial_ui_state`:
        // `EPUB_READER_UI_DEBUG=about` opens the About overlay on start so the scripted screenshot
        // verification can capture it without synthetic mouse input. No effect in release builds.
        #[cfg(debug_assertions)]
        let about_open = std::env::var("EPUB_READER_UI_DEBUG").ok().as_deref() == Some("about");
        #[cfg(not(debug_assertions))]
        let about_open = false;

        Self {
            state,
            screen: Screen::Landing,
            error: None,
            focus_handle: cx.focus_handle(),
            reader: None,
            landing: Some(landing),
            book: None,
            book_path: None,
            loading: false,
            persist_error: None,
            window_title: None,
            about_open,
            about_logo: Arc::new(Image::from_bytes(ImageFormat::Png, ABOUT_ICON_PNG.to_vec())),
        }
    }

    pub fn settings(&self) -> Settings {
        self.state.settings.clone()
    }

    pub fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        cx.notify();
    }

    /// Persist the library state. Failures are surfaced once on the landing screen instead of
    /// being silently ignored (the app still works read-only, e.g. when the data directory is
    /// locked down).
    fn save_state(&mut self) {
        match self.state.save() {
            Ok(()) => self.persist_error = None,
            Err(err) => {
                self.persist_error = Some(format!("Could not save library state: {err:#}"));
            }
        }
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.persist_error = None;
        cx.notify();
    }

    /// Show the About overlay (version / build toolchain / direct packages).
    ///
    /// Called from the landing footer and from the reader's ⓘ button. It must not update the
    /// `Reader` entity: when invoked from a reader click handler the reader is already leased,
    /// and a nested `Entity::update` on the same GPUI entity panics (the reader closes its own
    /// settings popover before calling this).
    pub fn show_about(&mut self, cx: &mut Context<Self>) {
        self.about_open = true;
        cx.notify();
    }

    /// Hide the About overlay (× button, backdrop click, or Esc).
    pub fn close_about(&mut self, cx: &mut Context<Self>) {
        if self.about_open {
            self.about_open = false;
            cx.notify();
        }
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.state.settings.theme = theme;
        self.save_state();
        cx.notify();
    }

    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.state.settings.font_size = clamp_font_size(size);
        self.save_state();
        cx.notify();
    }

    /// Persist the reader's TOC sidebar width (window fraction, already viewport-clamped by the
    /// reader). Saved immediately so a restart restores the user's chosen width.
    pub fn set_sidebar_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.state.settings.sidebar_width = crate::theme::clamp_sidebar_width_stored(width);
        self.save_state();
        cx.notify();
    }

    pub fn persist_position(
        &mut self,
        book_path: &PathBuf,
        position: Position,
        cx: &mut Context<Self>,
    ) {
        self.state.update_position(book_path, position);
        self.save_state();
        cx.notify();
    }

    pub fn remove_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.state.remove_recent(&path);
        self.save_state();
        cx.notify();
    }

    pub fn request_open(&mut self, cx: &mut Context<Self>) {
        let file = rfd::FileDialog::new()
            .add_filter("EPUB", &["epub"])
            .pick_file();
        if let Some(path) = file {
            self.open_path(path, cx);
        }
    }

    /// Open `path`: validates the file, parses it off the UI thread, then switches to the reader
    /// at the previously stored position (resume). Failures leave the app on the landing screen
    /// with a dismissible error banner; they never panic.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.error = None;
        if !path.is_file() {
            self.set_error(format!("File not found: {}", path.display()), cx);
            return;
        }

        // Recents are de-duplicated by path (spec §8): store absolute paths so an `--open`
        // relative path and the same file picked from the dialog map to one recent entry.
        let path = absolute_path(&path);

        self.loading = true;
        cx.notify();

        let initial_position = self
            .state
            .find_recent(&path)
            .map(|recent| recent.position)
            .unwrap_or_default();
        let settings = self.state.settings.clone();
        let path_for_task = path.clone();

        cx.spawn(async move |this, cx| {
            let parsed = cx
                .background_spawn(async move { crate::epub::Book::open(&path_for_task) })
                .await;
            this.update(cx, |root, cx| match parsed {
                Ok(book) => root.accept_book(path, book, settings, initial_position, cx),
                Err(err) => {
                    root.loading = false;
                    if root.screen == Screen::Reader {
                        // Surface open failures on the landing screen where the banner lives.
                        root.close_book(cx);
                    }
                    // `Book::open` already prefixes the path/context; avoid a duplicated
                    // "Cannot open EPUB: cannot open EPUB:" banner.
                    root.set_error(format!("{err:#}"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Finish a successful background parse on the UI thread.
    fn accept_book(
        &mut self,
        path: PathBuf,
        book: crate::epub::Book,
        settings: Settings,
        initial_position: Position,
        cx: &mut Context<Self>,
    ) {
        let chapter_count = book.chapter_count();
        if chapter_count == 0 {
            self.loading = false;
            self.set_error("Cannot open EPUB: the book has no readable chapters", cx);
            return;
        }

        // Register the book's embedded `@font-face` fonts before the reader renders it, so the
        // chapter typography matches the publication (e.g. Literata/Be Vietnam Pro) instead of
        // falling back to system fonts.
        register_book_fonts(&book, cx);

        let metadata = book.metadata().clone();
        let title = metadata
            .title
            .clone()
            .map(|title| title.trim().to_string())
            .filter(|title| !title.is_empty())
            .or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "Untitled".to_string());
        let author = metadata.authors.join(", ");
        let cover = book
            .cover_png()
            .and_then(|bytes| crate::persistence::save_cover_png(bytes, &path));
        let position =
            initial_position.clamped(chapter_count, initial_position.scroll_ratio);

        self.state
            .upsert_recent(path.clone(), &title, &author, cover, position);
        self.save_state();

        let book = Arc::new(book);
        self.book = Some(book.clone());
        self.book_path = Some(path);
        let root = cx.weak_entity();
        self.reader = Some(cx.new(|cx| Reader::new(book, settings, position, root, cx)));
        self.screen = Screen::Reader;
        self.loading = false;
        self.error = None;
        cx.notify();
    }

    /// Persist the live reading position, drop the book and show the landing screen again.
    pub fn close_book(&mut self, cx: &mut Context<Self>) {
        let position = self
            .reader
            .as_ref()
            .map(|reader| reader.read(cx).position());
        self.close_book_at(position, cx);
    }

    /// Close the book with an already-computed reading position. The reader's × button handler
    /// calls this while the `Reader` entity is mid-update; reading the reader from here would
    /// double-lease the GPUI entity and panic ("cannot read ... while it is already being updated").
    pub fn close_book_at(&mut self, position: Option<Position>, cx: &mut Context<Self>) {
        if let (Some(position), Some(path)) = (position, self.book_path.clone()) {
            self.persist_position(&path, position, cx);
        }
        self.book = None;
        self.reader = None;
        self.book_path = None;
        self.screen = Screen::Landing;
        self.loading = false;
        cx.notify();
    }

    /// Flush the latest reader position and settings before the window closes (called from the
    /// window `on_window_should_close` hook in `main.rs`). Never panics; state failures are logged
    /// by `AppState::save`'s ignored result.
    pub fn save_on_exit(&mut self, cx: &mut Context<Self>) {
        let position = self
            .reader
            .as_ref()
            .map(|reader| reader.read(cx).position());
        if let (Some(position), Some(path)) = (position, self.book_path.clone()) {
            self.state.update_position(&path, position);
        }
        self.save_state();
    }

    fn on_prev(&mut self, _: &PrevChapter, window: &mut Window, cx: &mut Context<Self>) {
        if self.about_open {
            return;
        }
        if let Some(reader) = self.reader.clone() {
            reader.update(cx, |reader, cx| reader.prev_chapter(window, cx));
            self.persist_reader_position(&reader, cx);
        }
    }

    fn on_next(&mut self, _: &NextChapter, window: &mut Window, cx: &mut Context<Self>) {
        if self.about_open {
            return;
        }
        if let Some(reader) = self.reader.clone() {
            reader.update(cx, |reader, cx| reader.next_chapter(window, cx));
            self.persist_reader_position(&reader, cx);
        }
    }

    fn persist_reader_position(&mut self, reader: &Entity<Reader>, cx: &mut Context<Self>) {
        let Some(path) = self.book_path.clone() else {
            return;
        };
        let position = reader.read(cx).position();
        self.persist_position(&path, position, cx);
    }

    fn on_dismiss(&mut self, _: &Dismiss, _window: &mut Window, cx: &mut Context<Self>) {
        // The About overlay is modal: Esc closes it before anything beneath it.
        if self.about_open {
            self.close_about(cx);
            return;
        }
        // Esc closes the settings popover; on the landing screen it dismisses the error banner.
        // The reader registers the same action while its own view is in the dispatch path, but
        // the root owns the focused element, so we forward it explicitly.
        if let Some(reader) = self.reader.clone() {
            reader.update(cx, |reader, cx| reader.dismiss_popover(cx));
        } else if self.error.is_some() {
            self.dismiss_error(cx);
        }
    }

    fn on_open(&mut self, _: &OpenBook, _window: &mut Window, cx: &mut Context<Self>) {
        // Global shortcuts stay inert while the About overlay is modal.
        if self.about_open {
            return;
        }
        self.request_open(cx);
    }

    fn on_file_drop(&mut self, paths: &ExternalPaths, _window: &mut Window, cx: &mut Context<Self>) {
        if self.about_open {
            return;
        }
        match paths
            .paths()
            .iter()
            .find(|path| is_epub(path))
            .cloned()
        {
            Some(path) => self.open_path(path, cx),
            None => self.set_error("Dropped file is not an EPUB", cx),
        }
    }

    fn render_landing(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.landing.is_none() {
            let weak = cx.weak_entity();
            self.landing = Some(cx.new(move |_| Landing::new(weak)));
        }
        self.landing
            .clone()
            .map(|landing| landing.into_any_element())
            .unwrap_or_else(|| div().size_full().bg(color(0xf0f0f0)).into_any_element())
    }

    fn sync_window_title(&mut self, window: &mut Window) {
        let book_title = match (&self.book, self.screen) {
            (Some(book), Screen::Reader) => book
                .metadata()
                .title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    self.book_path
                        .as_ref()
                        .and_then(|path| path.file_stem())
                        .and_then(|stem| stem.to_str())
                        .map(ToOwned::to_owned)
                }),
            _ => None,
        };
        let title = book_title.unwrap_or_else(|| "Another Epub Reader".to_string());
        if self.window_title.as_deref() != Some(title.as_str()) {
            window.set_window_title(&title);
            self.window_title = Some(title);
        }
    }
}

impl Focusable for Root {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_window_title(window);

        let content: AnyElement = if self.loading {
            loading_screen()
        } else {
            match self.screen {
                Screen::Reader => match self.reader.clone() {
                    Some(reader) => reader.into_any_element(),
                    None => self.render_landing(cx),
                },
                Screen::Landing => self.render_landing(cx),
            }
        };

        // Modal About overlay painted above whichever screen is active (landing or reader).
        let about = if self.about_open {
            Some(crate::ui::about::overlay(
                crate::about::info(),
                self.about_logo.clone(),
                cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.close_about(cx)
                }),
            ))
        } else {
            None
        };

        div()
            .id("root")
            .size_full()
            .relative()
            .key_context("Root")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_open))
            .on_drop(cx.listener(Self::on_file_drop))
            .child(content)
            .children(about)
    }
}

/// Load the publication's embedded fonts into GPUI's text system so CSS `font-family` names such
/// as Literata/Be Vietnam Pro resolve to the fonts shipped inside the EPUB.
fn register_book_fonts(book: &crate::epub::Book, cx: &mut App) {
    let fonts = book.embedded_fonts();

    if fonts.is_empty() {
        return;
    }
    let payload: Vec<Cow<'static, [u8]>> = fonts.into_iter().map(Cow::Owned).collect();
    match cx.text_system().add_fonts(payload) {
        Ok(()) => {
            // The CSS family name is not always the font's internal family name; alias them so the
            // renderer's `font_family` resolves to the registered font instead of a system fallback.
            let names = cx.text_system().all_font_names();
            let mut aliases = HashMap::new();
            for family in book.embedded_font_families() {
                if names.iter().any(|name| name.eq_ignore_ascii_case(&family)) {
                    continue;
                }
                let lower = family.to_lowercase();
                if let Some(name) = names
                    .iter()
                    .filter(|name| name.to_lowercase().starts_with(&lower))
                    .min_by_key(|name| name.len())
                {
                    aliases.insert(family, name.clone());
                }
            }

            crate::theme::set_font_aliases(aliases);
        }
        // Only debug builds report the failure, so the binding is underscore-prefixed to stay
        // warning-free in release builds, where the `eprintln!` is compiled out.
        Err(_err) => {
            #[cfg(debug_assertions)]
            eprintln!("embedded fonts add_fonts failed: {_err:?}");
        }
    }
}

fn is_epub(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
}

/// Make `path` absolute without resolving symlinks (avoids Windows `\\?\` verbatim prefixes,
/// which `std::fs::canonicalize` returns and which look wrong in persisted state/error text).
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn loading_screen() -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(color(0xd7d7d7), 0.),
            linear_color_stop(color(0xf0f0f0), 1.),
        ))
        .child(crate::ui::components::spinner(color(0x666666)))
        .child(
            div()
                .text_size(px(14.))
                .text_color(color(0x999999))
                .child("Opening book…"),
        )
        .into_any_element()
}







