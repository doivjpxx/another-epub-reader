#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use epub_reader::app::{NextChapter, OpenBook, PrevChapter, Dismiss, Root};
use epub_reader::cli::{parse_args, ScreenHint};
use epub_reader::window::{default_bounds, min_window_size, window_bounds};
use gpui::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_args(&args);

    Application::new().run(move |cx: &mut App| {
        let mut state = epub_reader::persistence::AppState::load();
        if let Some(theme) = cli.theme {
            state.settings.theme = theme;
        }
        if let Some(size) = cli.font_size {
            state.settings.font_size = size;
        }
        let bounds = default_bounds(cx, &state);

        let root = cx.new(|cx| Root::new(cx));
        let root_for_window = root.clone();

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds(bounds)),
                    window_min_size: Some(min_window_size()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Another Epub Reader".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    window.focus(&root_for_window.read(cx).focus_handle);
                    root_for_window
                },
            )
            .expect("failed to open window");

        cx.bind_keys([
            KeyBinding::new("left", PrevChapter, None),
            KeyBinding::new("right", NextChapter, None),
            KeyBinding::new("escape", Dismiss, None),
            KeyBinding::new("ctrl-o", OpenBook, None),
        ]);

        let root_for_open = root.clone();
        let cli_for_open = cli.clone();
        root_for_open.update(cx, move |root, cx| {
            if let Some(theme) = cli_for_open.theme {
                root.set_theme(theme, cx);
            }
            if let Some(size) = cli_for_open.font_size {
                root.set_font_size(size, cx);
            }
            if cli_for_open.screen == Some(ScreenHint::Landing) {
                root.close_book(cx);
            } else if let Some(path) = cli_for_open.open.clone() {
                root.open_path(path, cx);
            }
        });

        let root_for_close = root.clone();
        let _ = window.update(cx, |_root, window, cx| {
            window.on_window_should_close(cx, move |window, cx| {
                let size = window.bounds().size;
                let window_size = (f32::from(size.width), f32::from(size.height));
                root_for_close.update(cx, |root, cx| {
                    root.state.window_size = window_size;
                    root.save_on_exit(cx);
                });
                true
            });
        });

        let _ = window;
        cx.activate(true);
    });
}
