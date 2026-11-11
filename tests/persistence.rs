use epub_reader::epub::model::Position;
use epub_reader::persistence::{
    AppState, choose_writable_dir, cover_cache_path, data_dir, is_writable_dir, load_cover_image,
    relative_time, save_cover_png, save_cover_thumbnail,
};
use epub_reader::theme::{Settings, Theme};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A real 4x4 RGBA PNG (generated with GDI+) as hex; deterministic fixture for cover tests.
const PNG_4X4_HEX: &str = concat!(
    "89504e470d0a1a0a0000000d4948445200000004000000040806000000a9f19e7e",
    "000000017352474200aece1ce90000000467414d410000b18f0bfc61050000000970",
    "48597300000ec300000ec301c76fa8640000003149444154185715c83111003010c3",
    "b0000bb01f0d2afcdcab4625c1062fb860526cf18aeb8fc31edee1eec7b0c31b6ef8",
    "0039c8233128aed8210000000049454e44ae426082",
);

fn png_4x4() -> Vec<u8> {
    assert_eq!(PNG_4X4_HEX.len() % 2, 0);
    (0..PNG_4X4_HEX.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&PNG_4X4_HEX[offset..offset + 2], 16).expect("hex"))
        .collect()
}

const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn unique_temp_path(label: &str, extension: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "epub-reader-{label}-{}-{nanos}.{extension}",
        std::process::id()
    ))
}

fn assert_is_png_file(path: &Path) {
    assert!(path.exists(), "expected cover file at {}", path.display());
    let bytes = std::fs::read(path).expect("read cover file");
    assert!(
        bytes.starts_with(PNG_MAGIC),
        "saved cover is not a PNG: {:?}",
        &bytes[..bytes.len().min(8)]
    );
}

/// Serializes tests that temporarily point `persistence::data_dir()` at a temp directory.
static DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Run `f` with `EPUB_READER_DATA_DIR` pointing at a fresh temp directory, then clean up.
///
/// Serialized through `DATA_DIR_LOCK` because the env var is process-wide.
fn with_temp_data_dir<T>(f: impl FnOnce() -> T) -> T {
    let guard = DATA_DIR_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_temp_path("data-dir", "d");
    std::fs::create_dir_all(&dir).expect("create temp data dir");
    // SAFETY: all env mutation in this test binary is serialized by DATA_DIR_LOCK and no other
    // thread reads these variables.
    unsafe { std::env::set_var("EPUB_READER_DATA_DIR", &dir) };
    let result = f();
    unsafe { std::env::remove_var("EPUB_READER_DATA_DIR") };
    drop(guard);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn cover_cache_path_is_stable_and_per_book() {
    with_temp_data_dir(|| {
        let first = cover_cache_path(Path::new(r"C:\books\a.epub"));
        let second = cover_cache_path(Path::new(r"C:\books\a.epub"));
        let other = cover_cache_path(Path::new(r"C:\books\b.epub"));
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(first.starts_with(data_dir()));
        assert_eq!(first.extension().and_then(|ext| ext.to_str()), Some("png"));
    });
}

#[test]
fn cover_thumbnail_roundtrips_as_valid_png() {
    let dest = unique_temp_path("cover-thumb", "png");
    let _ = std::fs::remove_file(&dest);
    let written = save_cover_thumbnail(&png_4x4(), &dest).expect("thumbnail write");
    assert_eq!(written, dest);
    assert_is_png_file(&written);
    assert!(
        load_cover_image(&written).is_some(),
        "saved cover thumbnail must decode via image"
    );

    let dimension = load_cover_image(&written).expect("decode saved cover");
    assert!(dimension.width() <= 240 && dimension.height() <= 320);

    let _ = std::fs::remove_file(&written);
}

#[test]
fn save_cover_png_uses_the_cover_cache_directory() {
    with_temp_data_dir(|| {
        // Uses a unique book path so the hashed cache file cannot collide with a real entry.
        let book_path = unique_temp_path("cover-book", "epub");
        let cached = save_cover_png(&png_4x4(), &book_path).expect("cover cache write");
        assert_eq!(cached, cover_cache_path(&book_path));
        assert!(cached.starts_with(data_dir()));
        assert!(cached.starts_with(data_dir().join("covers")));
        assert_is_png_file(&cached);
    });
}

#[test]
fn corrupt_cover_bytes_are_rejected_without_panicking() {
    let dest = unique_temp_path("corrupt-cover", "png");
    assert!(save_cover_thumbnail(b"this is definitely not an image", &dest).is_none());
    assert!(!dest.exists());
    assert!(save_cover_thumbnail(&[], &dest).is_none());
}

#[test]
fn missing_cover_file_decodes_to_none() {
    assert!(load_cover_image(Path::new(r"C:\definitely\missing\cover.png")).is_none());
}

#[test]
fn relative_time_uses_coarse_buckets() {
    let now = 1_800_000_000u64;
    assert_eq!(relative_time(now, now), "just now");
    assert_eq!(relative_time(now, now + 120), "just now"); // future timestamps (clock skew)
    assert_eq!(relative_time(now, now - 30), "just now");
    assert_eq!(relative_time(now, now - 60), "1 min ago");
    assert_eq!(relative_time(now, now - 60 * 45), "45 min ago");
    assert_eq!(relative_time(now, now - 60 * 60 * 3), "3 hr ago");
    assert_eq!(relative_time(now, now - 60 * 60 * 24), "1 day ago");
    assert_eq!(relative_time(now, now - 60 * 60 * 24 * 3), "3 days ago");
    assert_eq!(relative_time(now, now - 60 * 60 * 24 * 21), "3 wk ago");
}

#[test]
fn roundtrip_and_defaults() {
    let mut state = AppState::default();
    assert_eq!(state.settings.theme, Theme::White);
    assert_eq!(state.settings.font_size, 20.0);
    state.upsert_recent(
        PathBuf::from(r"C:\books\a.epub"),
        "A",
        "Ann",
        None,
        Position {
            chapter_index: 2,
            scroll_ratio: 0.5,
        },
    );
    let json = serde_json::to_string_pretty(&state).unwrap();
    let back: AppState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.recent_books.len(), 1);
    assert_eq!(back.recent_books[0].position.chapter_index, 2);
    assert_eq!(back.settings, Settings::default());
}

#[test]
fn recents_dedupe_newest_first_max_eight() {
    let mut state = AppState::default();
    for i in 0..10 {
        state.upsert_recent(
            PathBuf::from(format!(r"C:\books\{i}.epub")),
            &format!("T{i}"),
            "A",
            None,
            Position::default(),
        );
    }
    assert_eq!(state.recent_books.len(), 8);
    assert_eq!(state.recent_books[0].title, "T9");

    state.upsert_recent(
        PathBuf::from(r"C:\books\3.epub"),
        "T3-updated",
        "A",
        None,
        Position {
            chapter_index: 4,
            scroll_ratio: 0.1,
        },
    );
    assert_eq!(state.recent_books.len(), 8);
    assert_eq!(state.recent_books[0].title, "T3-updated");
    assert_eq!(state.recent_books[0].position.chapter_index, 4);
}

#[test]
fn remove_recent_works() {
    let mut state = AppState::default();
    state.upsert_recent(
        PathBuf::from(r"C:\books\a.epub"),
        "A",
        "",
        None,
        Position::default(),
    );
    state.remove_recent(std::path::Path::new(r"C:\books\a.epub"));
    assert!(state.recent_books.is_empty());
}

#[test]
fn corrupt_state_falls_back_to_defaults() {
    let tmp = std::env::temp_dir().join("epub-reader-corrupt-state.json");
    std::fs::write(&tmp, b"{not json").unwrap();
    let state = AppState::load_from(&tmp);
    assert_eq!(state.settings.theme, Theme::White);
    assert!(state.recent_books.is_empty());
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn position_update_is_stored() {
    let mut state = AppState::default();
    let path = PathBuf::from(r"C:\books\b.epub");
    state.upsert_recent(path.clone(), "B", "", None, Position::default());
    state.update_position(
        &path,
        Position {
            chapter_index: 7,
            scroll_ratio: 0.25,
        },
    );
    assert_eq!(state.find_recent(&path).unwrap().position.chapter_index, 7);
}

// Landing error-banner formatting lives in src/ui/landing.rs; covered here because this is the
// only persistence-side test binary that already exercises non-GPUI helpers.
#[test]
fn error_banner_message_wraps_long_missing_paths() {
    use epub_reader::ui::landing::{BANNER_LINE_CHARS, banner_message};

    let message = r"File not found: C:\Users\example\Documents\projects\epub-reader\test-artifacts\a-rather-long-missing-book-name.epub";
    let formatted = banner_message(message.into());
    let lines: Vec<&str> = formatted.as_ref().lines().collect();

    assert_eq!(lines.len(), 3, "prefix, file name and elided path: {formatted:?}");
    assert_eq!(lines[0], "File not found");
    for line in &lines {
        assert!(
            line.chars().count() <= BANNER_LINE_CHARS,
            "line {line:?} exceeds {BANNER_LINE_CHARS} chars"
        );
    }
    assert!(lines[1].ends_with(".epub"), "file name is shown: {formatted:?}");
    assert!(lines[2].contains('…'), "path is middle-elided: {formatted:?}");
}

#[test]
fn error_banner_message_breaks_generic_long_tokens() {
    use epub_reader::ui::landing::{BANNER_LINE_CHARS, banner_message};

    let message = "Cannot open EPUB: C:\\a\\very\\long\\path\\without\\any\\spaces\\at\\all\\that\\keeps\\going.epub";
    let formatted = banner_message(message.into());
    assert!(
        formatted
            .as_ref()
            .lines()
            .all(|line| line.chars().count() <= BANNER_LINE_CHARS),
        "over-long tokens get newline breaks: {formatted:?}"
    );
}

#[test]
fn old_state_without_sidebar_width_defaults_to_twenty_percent() {
    // Forward compatibility: state files written before the resizable sidebar existed must load
    // with the source-parity 20% default instead of failing deserialization.
    let settings: Settings = serde_json::from_str(r#"{"theme":"white","font_size":20.0}"#).unwrap();
    assert!((settings.sidebar_width - 0.20).abs() < 1e-6);

    let state: AppState = serde_json::from_str(
        r#"{
            "version": 1,
            "settings": {"theme": "sepia", "font_size": 22.0},
            "window_size": [1280.0, 800.0],
            "recent_books": []
        }"#,
    )
    .unwrap();
    assert_eq!(state.settings.theme, Theme::Sepia);
    assert!((state.settings.sidebar_width - 0.20).abs() < 1e-6);
}

#[test]
fn sidebar_width_roundtrips_in_state() {
    let mut state = AppState::default();
    state.settings.sidebar_width = 0.31;
    let json = serde_json::to_string(&state).unwrap();
    let back: AppState = serde_json::from_str(&json).unwrap();
    assert!((back.settings.sidebar_width - 0.31).abs() < 1e-6);
}

#[test]
fn missing_recent_file_is_kept_until_user_removes_it() {
    with_temp_data_dir(|| {
        let missing = unique_temp_path("vanished-book", "epub");
        assert!(!missing.exists(), "fixture path must not exist");

        let mut state = AppState::default();
        state.upsert_recent(missing.clone(), "Gone", "Author", None, Position::default());
        state.save().expect("state saves with missing path");

        let mut loaded = AppState::load();
        assert_eq!(loaded.recent_books.len(), 1);
        assert_eq!(loaded.recent_books[0].path, missing);
        assert!(!loaded.recent_books[0].path.exists());

        loaded.remove_recent(&missing);
        loaded.save().expect("state saves after removal");
        assert!(AppState::load().recent_books.is_empty());
    });
}

#[test]
fn writable_dir_selection_skips_unwritable_primary() {
    let base = unique_temp_path("writable-dir", "d");
    std::fs::create_dir_all(&base).expect("create base dir");
    // A file can never be used as the data directory, so the primary is effectively unwritable.
    let primary = base.join("primary-is-a-file");
    std::fs::write(&primary, b"not a directory").expect("write file");
    let fallback = base.join("fallback");

    let chosen = choose_writable_dir(primary.clone(), vec![fallback.clone()]);
    assert_eq!(chosen, fallback);
    assert!(is_writable_dir(&chosen));

    // With no writable fallback, the primary is returned so error text points at the real target.
    let unusable = primary.join("nested");
    assert_eq!(choose_writable_dir(primary.clone(), vec![unusable]), primary);
    let _ = std::fs::remove_dir_all(&base);
}

