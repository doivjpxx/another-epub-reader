use crate::epub::model::Position;
use crate::theme::{Settings, Theme};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const STATE_VERSION: u32 = 1;
const MAX_RECENTS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentBook {
    pub path: PathBuf,
    pub title: String,
    pub author: String,
    pub cover: Option<PathBuf>,
    pub position: Position,
    pub last_opened_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppState {
    pub version: u32,
    pub settings: Settings,
    pub window_size: (f32, f32),
    pub recent_books: Vec<RecentBook>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            settings: Settings::default(),
            window_size: (1280.0, 800.0),
            recent_books: Vec::new(),
        }
    }
}

/// Root directory for app data (`state.json`, `covers/`).
///
/// Defaults to `dirs::data_dir()/epub-reader` (design spec §8). The `EPUB_READER_DATA_DIR`
/// environment variable overrides it; this is a test/portable-mode hook that keeps integration
/// tests hermetic and lets sandboxed UI-verification runs isolate state from the real user
/// profile. Default behaviour is unchanged.
///
/// If the default location is not writable (portable/USB installs, locked-down profiles,
/// sandboxed runs), a writable fallback is resolved once: a portable `epub-reader-data/` folder
/// next to the executable, then the local app-data dir, then the temp dir. Recent books are
/// therefore still remembered instead of silently disappearing.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EPUB_READER_DATA_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    default_data_dir().clone()
}

fn default_data_dir() -> &'static PathBuf {
    static RESOLVED: OnceLock<PathBuf> = OnceLock::new();
    RESOLVED.get_or_init(|| {
        let primary = dirs::data_dir()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("epub-reader");
        let mut fallbacks = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            fallbacks.push(dir.join("epub-reader-data"));
        }
        if let Some(local) = dirs::data_local_dir() {
            fallbacks.push(local.join("epub-reader"));
        }
        fallbacks.push(std::env::temp_dir().join("epub-reader-data"));
        choose_writable_dir(primary, fallbacks)
    })
}

/// Prefer `primary` when writable, otherwise the first writable fallback (or `primary` when none
/// is writable so error messages still point at the intended location). Pure enough to unit-test.
pub fn choose_writable_dir(primary: PathBuf, fallbacks: Vec<PathBuf>) -> PathBuf {
    if is_writable_dir(&primary) {
        return primary;
    }
    for fallback in fallbacks {
        if is_writable_dir(&fallback) {
            eprintln!(
                "epub-reader: data dir {} is not writable; using {}",
                primary.display(),
                fallback.display()
            );
            return fallback;
        }
    }
    eprintln!(
        "epub-reader: no writable data directory found (tried {}); library state will not persist",
        primary.display()
    );
    primary
}

/// Whether `dir` can be created and written to. Uses a short-lived probe file so read-only or
/// permission-denied locations are detected before we rely on them for `state.json`.
pub fn is_writable_dir(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".epub-reader-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

pub fn cover_cache_path(path: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    data_dir().join("covers").join(format!("{hex}.png"))
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl AppState {
    pub fn load() -> AppState {
        Self::load_from(&state_path())
    }

    pub fn load_from(path: &Path) -> AppState {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<AppState>(&text) {
                Ok(mut state) => {
                    state.version = STATE_VERSION;
                    state.settings.font_size =
                        crate::theme::clamp_font_size(state.settings.font_size);
                    state
                        .recent_books
                        .retain(|r| r.position.scroll_ratio.is_finite());
                    state
                }
                Err(_) => AppState::default(),
            },
            Err(_) => AppState::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create data dir {}", dir.display()))?;
        let tmp = dir.join("state.json.tmp");
        let target = state_path();
        let text = serde_json::to_string_pretty(self).context("serialize app state")?;
        std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &target).with_context(|| format!("replace {}", target.display()))?;
        Ok(())
    }

    pub fn upsert_recent(
        &mut self,
        path: PathBuf,
        title: &str,
        author: &str,
        cover: Option<PathBuf>,
        position: Position,
    ) {
        self.recent_books.retain(|b| b.path != path);
        self.recent_books.insert(
            0,
            RecentBook {
                path,
                title: title.to_string(),
                author: author.to_string(),
                cover,
                position,
                last_opened_unix: now_unix(),
            },
        );
        self.recent_books.truncate(MAX_RECENTS);
    }

    pub fn remove_recent(&mut self, path: &Path) {
        self.recent_books.retain(|b| b.path != path);
    }

    pub fn find_recent(&self, path: &Path) -> Option<&RecentBook> {
        self.recent_books.iter().find(|b| b.path == path)
    }

    pub fn update_position(&mut self, path: &Path, position: Position) {
        if let Some(book) = self.recent_books.iter_mut().find(|b| b.path == path) {
            book.position = position;
            book.last_opened_unix = now_unix();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppStateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Decode arbitrary cover bytes and write a max 240x320 PNG thumbnail to `dest`.
///
/// Returns the path written on success, `None` for corrupt/unsupported image data or
/// unwritable destinations. Never panics on malformed input.
pub fn save_cover_thumbnail(bytes: &[u8], dest: &Path) -> Option<PathBuf> {
    let img = image::load_from_memory(bytes).ok()?;
    let thumb = img.thumbnail(240, 320);
    std::fs::create_dir_all(dest.parent()?).ok()?;
    thumb.save(dest).ok()?;
    Some(dest.to_path_buf())
}

/// Decode cover bytes and store a max 240x320 PNG thumbnail in the per-book cover cache.
pub fn save_cover_png(bytes: &[u8], book_path: &Path) -> Option<PathBuf> {
    save_cover_thumbnail(bytes, &cover_cache_path(book_path))
}

pub fn load_cover_image(path: &Path) -> Option<image::DynamicImage> {
    image::open(path).ok()
}

/// Human readable "last opened" text for the recent-books card.
///
/// `now_unix` and `last_opened_unix` are seconds since the unix epoch; a timestamp in the
/// future (clock skew) is reported as "just now". Pure function so it can be unit-tested.
pub fn relative_time(now_unix: u64, last_opened_unix: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    let delta = now_unix.saturating_sub(last_opened_unix);
    match delta {
        0..=59 => "just now".to_string(),
        MINUTE..=3599 => format!("{} min ago", delta / MINUTE),
        HOUR..=86399 => format!("{} hr ago", delta / HOUR),
        DAY..=604_799 => {
            let days = delta / DAY;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
        _ => {
            let weeks = delta / WEEK;
            format!("{weeks} wk ago")
        }
    }
}

#[allow(dead_code)]
fn _assert_theme_serializable() {
    let _ = Theme::White;
}
