//! Inline runs → `StyledText` highlight ranges.
//!
//! GPUI's `StyledText` can style contiguous byte ranges of a single string, so a paragraph is
//! flattened to one string plus a list of highlight ranges. Per-run font size/family is not
//! supported by GPUI 0.2.2 (documented limitation in the design spec §6.2), so `font-size` and
//! `font-family` are applied at block level only. `<br/>` boundaries are recorded in `breaks` and
//! the renderer splits the paragraph into stacked segments there.
//!
//! Inline `code` and `sub`/`sup` degrade to plain runs (no per-run monospace / smaller size is
//! possible): the block renderer switches to a monospace family when *every* run of a paragraph is
//! code (`all_code`).

use crate::epub::model::{Inline, InlineStyle};
use crate::theme::Theme;
use gpui::{FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, UnderlineStyle};
use std::ops::Range;

/// One rendered paragraph: flattened text, style ranges, link ranges, and `<br/>` byte offsets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyledParagraph {
    pub text: String,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    /// Byte ranges of inline links (`<a href>`), each with its resolved-raw href target.
    pub links: Vec<(Range<usize>, String)>,
    /// Byte offsets into `text` where a `<br/>` occurred. The renderer splits the paragraph at
    /// these offsets (the newline itself is not part of any highlight).
    pub breaks: Vec<usize>,
}

/// One visual line of a paragraph after splitting at `<br/>` runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyledSegment {
    pub text: String,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    /// Link ranges rebased onto this segment's text.
    pub links: Vec<(Range<usize>, String)>,
}

impl StyledParagraph {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Split the paragraph into stacked segments at `<br/>` offsets, re-basing highlight ranges
    /// onto each segment.
    pub fn segments(&self) -> Vec<StyledSegment> {
        let mut segments = Vec::new();
        let mut start = 0usize;
        let mut bounds: Vec<usize> = self.breaks.clone();
        bounds.push(self.text.len());
        for raw_end in bounds {
            let end = raw_end.min(self.text.len()).max(start);
            let mut highlights = Vec::new();
            for (range, highlight) in &self.highlights {
                let lo = range.start.max(start);
                let hi = range.end.min(end);
                if lo < hi {
                    highlights.push((lo - start..hi - start, *highlight));
                }
            }
            let mut links = Vec::new();
            for (range, href) in &self.links {
                let lo = range.start.max(start);
                let hi = range.end.min(end);
                if lo < hi {
                    links.push((lo - start..hi - start, href.clone()));
                }
            }
            segments.push(StyledSegment {
                text: self.text[start..end].to_string(),
                highlights,
                links,
            });
            start = (end + 1).min(self.text.len() + 1); // skip the '\n' injected for the break
        }
        segments
    }
}

/// True when a run carries any visual style that `HighlightStyle` can express.
pub fn style_non_default(style: InlineStyle) -> bool {
    style.bold
        || style.italic
        || style.underline
        || style.strikethrough
        || style.code
        || style.superscript
        || style.subscript
        || style.link
}

/// True when every non-whitespace text run is inline code. The block renderer uses this to pick a
/// monospace family for the whole paragraph (the only available approximation).
pub fn all_code(runs: &[Inline]) -> bool {
    let mut saw_text = false;
    for run in runs {
        if let Some((text, style)) = run.text() {
            if text.trim().is_empty() {
                continue;
            }
            saw_text = true;
            if !style.code {
                return false;
            }
        }
    }
    saw_text
}

/// Map an inline style to the GPUI highlight it renders with.
pub fn highlight_for(style: InlineStyle, theme: Theme) -> HighlightStyle {
    let mut highlight = HighlightStyle::default();
    if style.bold {
        highlight.font_weight = Some(FontWeight::BOLD);
    }
    if style.italic {
        highlight.font_style = Some(FontStyle::Italic);
    }
    if style.underline {
        highlight.underline = Some(UnderlineStyle::default());
    }
    if style.strikethrough {
        highlight.strikethrough = Some(StrikethroughStyle::default());
    }
    if style.link {
        highlight.color = Some(theme.link_color());
    }
    // `code`, `superscript` and `subscript` have no per-run representation in `HighlightStyle`;
    // they degrade to plain runs (documented in the design spec).
    highlight
}

/// Structural variant: identical to [`build_styled_runs_with_theme`] with the default (white)
/// theme. Use the themed variant when rendering so link colors match the active theme.
pub fn build_styled_runs(runs: &[Inline]) -> StyledParagraph {
    build_styled_runs_with_theme(runs, Theme::White)
}

/// Flatten inline runs into a styled paragraph, merging adjacent runs with identical styles.
pub fn build_styled_runs_with_theme(runs: &[Inline], theme: Theme) -> StyledParagraph {
    let mut out = StyledParagraph::default();
    let mut current: Option<(InlineStyle, usize)> = None; // (style, start byte)

    for run in runs {
        match run {
            Inline::Break => {
                if let Some((style, start)) = current.take() {
                    if start < out.text.len() && style_non_default(style) {
                        out.highlights
                            .push((start..out.text.len(), highlight_for(style, theme)));
                    }
                }
                out.breaks.push(out.text.len());
                out.text.push('\n');
            }
            Inline::Text { text, style } | Inline::Link { text, style, .. } => {
                match current {
                    Some((current_style, _)) if current_style == *style => {}
                    Some((current_style, start)) => {
                        if start < out.text.len() && style_non_default(current_style) {
                            out.highlights
                                .push((start..out.text.len(), highlight_for(current_style, theme)));
                        }
                        current = Some((*style, out.text.len()));
                    }
                    None => current = Some((*style, out.text.len())),
                }
                let run_start = out.text.len();
                let rendered = if style.uppercase || style.letter_spacing > 0.0 {
                    let mut rendered = if style.uppercase {
                        text.to_uppercase()
                    } else {
                        text.to_string()
                    };
                    if style.letter_spacing > 0.0 && rendered.chars().count() > 1 {
                        // GPUI has no per-run letter-spacing: approximate with thin spaces between
                        // non-whitespace characters (used by small uppercase labels).
                        let mut spaced = String::with_capacity(rendered.len() * 2);
                        let mut previous_visible = false;
                        for character in rendered.chars() {
                            if previous_visible && !character.is_whitespace() {
                                spaced.push('\u{2009}');
                            }
                            previous_visible = !character.is_whitespace();
                            spaced.push(character);
                        }
                        rendered = spaced;
                    }
                    std::borrow::Cow::Owned(rendered)
                } else {
                    std::borrow::Cow::Borrowed(text.as_str())
                };
                out.text.push_str(&rendered);
                if let Inline::Link { href, .. } = run {
                    let run_end = out.text.len();
                    match out.links.last_mut() {
                        Some((range, previous_href))
                            if *previous_href == *href && range.end == run_start =>
                        {
                            range.end = run_end;
                        }
                        _ => out.links.push((run_start..run_end, href.clone())),
                    }
                }
            }
        }
    }

    if let Some((style, start)) = current {
        if start < out.text.len() && style_non_default(style) {
            out.highlights
                .push((start..out.text.len(), highlight_for(style, theme)));
        }
    }
    out
}

/// Split `text` at each `<br/>` offset into plain strings (used by preformatted/heading paths that
/// do not need highlights).
pub fn split_breaks(text: &str, breaks: &[usize]) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for end in breaks {
        let end = (*end).min(text.len());
        if start < end {
            out.push(text[start..end].to_string());
        } else {
            out.push(String::new());
        }
        start = (end + 1).min(text.len());
    }
    if start < text.len() {
        out.push(text[start..].to_string());
    }
    out
}

