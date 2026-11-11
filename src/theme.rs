use gpui::{Hsla, rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    White,
    Sepia,
    Night,
}

pub const CONTENT_DEFAULT_FONT_SIZE: f32 = 20.0;
pub const FONT_SIZE_MIN: f32 = 14.0;
pub const FONT_SIZE_MAX: f32 = 30.0;
pub const FONT_SIZE_STEP: f32 = 2.0;

/// Default TOC sidebar width as a fraction of the window (source parity: 20%).
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 0.20;
/// Coarse storage bounds for the resizable sidebar; the viewport-aware clamp lives in
/// `ui::reader::clamp_sidebar_fraction`.
pub const SIDEBAR_WIDTH_STORED_MIN: f32 = 0.08;
pub const SIDEBAR_WIDTH_STORED_MAX: f32 = 0.65;

pub fn default_sidebar_width() -> f32 {
    DEFAULT_SIDEBAR_WIDTH
}

/// Normalize a persisted sidebar fraction (used on load and before saving).
pub fn clamp_sidebar_width_stored(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(SIDEBAR_WIDTH_STORED_MIN, SIDEBAR_WIDTH_STORED_MAX)
    } else {
        DEFAULT_SIDEBAR_WIDTH
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub theme: Theme,
    pub font_size: f32,
    /// TOC sidebar width as a window fraction; persisted. Backward compatible: old state files
    /// without this field deserialize to the 20% default.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::White,
            font_size: CONTENT_DEFAULT_FONT_SIZE,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

impl Theme {
    pub fn content_text(self) -> Hsla {
        match self {
            Theme::White => rgb(0x000000).into(),
            Theme::Sepia => rgb(0x704214).into(),
            Theme::Night => rgb(0xbdcadb).into(),
        }
    }

    pub fn content_bg(self) -> Hsla {
        match self {
            Theme::White => rgb(0xffffff).into(),
            Theme::Sepia => rgb(0xf4eacd).into(),
            Theme::Night => rgb(0x1b1f2a).into(),
        }
    }

    pub fn sidebar_bg(self) -> Hsla {
        match self {
            Theme::White => rgb(0xfafafa).into(),
            Theme::Sepia => rgb(0x111111).into(),
            Theme::Night => rgb(0x2c3142).into(),
        }
    }

    pub fn sidebar_text(self) -> Hsla {
        match self {
            Theme::White => rgb(0x333333).into(),
            Theme::Sepia => rgb(0xafa790).into(),
            Theme::Night => rgb(0xf4f4f5).into(),
        }
    }

    pub fn sidebar_hover_text(self) -> Hsla {
        match self {
            Theme::White => rgb(0x346cb9).into(),
            Theme::Sepia => rgb(0xe0c69a).into(),
            Theme::Night => rgb(0xdbe2f0).into(),
        }
    }

    pub fn sidebar_hover_bg(self) -> Option<Hsla> {
        match self {
            Theme::White | Theme::Sepia => None,
            Theme::Night => Some(rgb(0x363d52).into()),
        }
    }

    pub fn gear_icon(self) -> Hsla {
        match self {
            Theme::White => rgb(0x666666).into(),
            Theme::Sepia => rgb(0xb9ad93).into(),
            Theme::Night => rgb(0xaab4cc).into(),
        }
    }

    pub fn arrow_icon(self) -> Hsla {
        rgb(0xc0c0c0).into()
    }

    pub fn selected_toc(self) -> Hsla {
        match self {
            Theme::White => rgb(0x346cb9).into(),
            Theme::Sepia => rgb(0xe0b06a).into(),
            Theme::Night => rgb(0x8ab4f8).into(),
        }
    }

    pub fn border(self) -> Hsla {
        match self {
            Theme::White => rgb(0xf1f1f1).into(),
            Theme::Sepia => rgb(0x2a2117).into(),
            Theme::Night => rgb(0x3b3f54).into(),
        }
    }

    pub fn link_color(self) -> Hsla {
        match self {
            Theme::White => rgb(0x346cb9).into(),
            Theme::Sepia => rgb(0x8a5a20).into(),
            Theme::Night => rgb(0x8ab4f8).into(),
        }
    }

    pub fn next(self) -> Theme {
        match self {
            Theme::White => Theme::Sepia,
            Theme::Sepia => Theme::Night,
            Theme::Night => Theme::White,
        }
    }

    /// Adapt a text colour taken from the book's own CSS to the active reader theme.
    ///
    /// EPUBs routinely hard-code `color: #000` / `#111`; painting that literally in Night or Sepia
    /// makes the text unreadable. Neutral colours fall back to the theme's content colour, while
    /// deliberately coloured text keeps its hue at a readable lightness.
    pub fn adapt_content_color(self, color: Hsla) -> Hsla {
        match self {
            Theme::White => color,
            Theme::Sepia => {
                if color.s < 0.15 {
                    self.content_text()
                } else {
                    Hsla {
                        h: color.h,
                        s: (color.s * 0.65).min(0.65),
                        l: 0.30,
                        a: color.a,
                    }
                }
            }
            Theme::Night => {
                if color.s < 0.15 {
                    self.content_text()
                } else {
                    Hsla {
                        h: color.h,
                        s: (color.s * 0.55).min(0.60),
                        l: 0.80,
                        a: color.a,
                    }
                }
            }
        }
    }

    /// Adapt a background colour from the book's CSS (e.g. `.callout { background: #fff }`) so it
    /// becomes a subtle raised panel instead of a glaring white box in Night/Sepia.
    pub fn adapt_background(self, color: Hsla) -> Hsla {
        match self {
            Theme::White => color,
            Theme::Sepia => Hsla {
                h: 0.11,
                s: 0.42,
                l: if color.l > 0.55 { 0.91 } else { 0.84 },
                a: color.a,
            },
            Theme::Night => Hsla {
                h: 0.62,
                s: 0.16,
                l: if color.l > 0.35 { 0.17 } else { 0.12 },
                a: color.a,
            },
        }
    }

    /// Adapt a border colour from the book's CSS (black rules are invisible on a dark theme).
    pub fn adapt_border(self, _color: Hsla) -> Hsla {
        match self {
            Theme::White => _color,
            Theme::Sepia => Hsla {
                h: 0.11,
                s: 0.28,
                l: 0.52,
                a: _color.a,
            },
            Theme::Night => Hsla {
                h: 0.62,
                s: 0.14,
                l: 0.33,
                a: _color.a.max(0.75),
            },
        }
    }
}

pub fn serif_font() -> &'static str {
    "Times New Roman"
}

pub fn sans_font() -> &'static str {
    "Segoe UI"
}

pub fn mono_font() -> &'static str {
    "Cascadia Mono"
}

/// CSS `font-family` names declared by an opened book mapped to the loaded font's internal family
/// name (e.g. "Literata" → "Literata 12pt"). Populated when the reader registers embedded fonts.
static FONT_ALIASES: std::sync::RwLock<Option<std::collections::HashMap<String, String>>> =
    std::sync::RwLock::new(None);

pub fn set_font_aliases(aliases: std::collections::HashMap<String, String>) {
    if let Ok(mut slot) = FONT_ALIASES.write() {
        *slot = Some(aliases);
    }
}

fn font_alias(name: &str) -> Option<String> {
    FONT_ALIASES
        .read()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(|map| map.get(name).cloned()))
}

pub fn font_for(family: &crate::epub::model::FontFamily) -> String {
    use crate::epub::model::FontFamily;
    match family {
        FontFamily::Serif => serif_font().to_string(),
        FontFamily::SansSerif => sans_font().to_string(),
        FontFamily::Monospace => mono_font().to_string(),
        FontFamily::Named(name) => font_alias(name).unwrap_or_else(|| name.clone()),
    }
}

pub fn clamp_font_size(size: f32) -> f32 {
    (size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX) / FONT_SIZE_STEP).round() * FONT_SIZE_STEP
}


