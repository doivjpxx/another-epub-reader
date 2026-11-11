//! In-repo CSS subset: parser, selector matching, cascade and browser-like UA defaults.
//!
//! The subset is intentionally small and forgiving — EPUB stylesheets in the wild are full of
//! unsupported rules (`@media`, `@font-face`, pseudo-elements, floats, ...). Unparseable
//! selectors are dropped, unsupported properties are ignored, and unknown at-rules are skipped
//! wholesale; nothing here panics on malformed input.
//!
//! Cascade order (last write wins):
//! 1. UA defaults for the tag (specificity 0, order 0).
//! 2. Author rules, sorted by `(specificity, source order)`.
//! 3. Inline `style="..."` declarations.
//!
//! The same three steps are repeated for `!important` declarations, which outrank all normal
//! ones.

use crate::epub::model::{BlockStyle, FontFamily, TextAlignment};
use gpui::{Hsla, rgb, rgba};
use scraper::ElementRef;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompoundSelector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    /// Attribute selectors, raw: `"href"` or `"rel=stylesheet"`.
    pub attrs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub parts: Vec<CompoundSelector>,
    pub specificity: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub order: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    next_order: usize,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Removes `/* ... */` comments.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Skips every `@...` at-rule: `@media`/`@supports`-style blocks are removed with their body,
/// statement at-rules (`@import`, `@charset`, ...) are removed up to their `;`.
fn strip_at_rules(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let chars: Vec<char> = css.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut depth = 0usize;
            let mut j = i;
            while j < chars.len() {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    ';' if depth == 0 => {
                        j += 1;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Splits a declaration block on `;`, respecting quotes and parentheses.
fn split_statements(body: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for ch in body.chars() {
        match quote {
            Some(q) => {
                current.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    current.push(ch);
                }
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ';' if depth <= 0 => statements.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            },
        }
    }
    if !current.trim().is_empty() {
        statements.push(current);
    }
    statements
}

fn split_important(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    // CSS allows `!important` and `! important`.
    if let Some(index) = lower.rfind('!') {
        let rest = lower[index + 1..].trim();
        if rest == "important" {
            return (trimmed[..index].trim().to_string(), true);
        }
    }
    (trimmed.to_string(), false)
}

/// Parses a `property: value` list (a rule body or an inline `style` attribute).
pub fn parse_declarations(body: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    for statement in split_statements(body) {
        let Some((name, value)) = statement.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let (value, important) = split_important(value);
        if value.is_empty() {
            continue;
        }
        declarations.push(Declaration {
            name,
            value,
            important,
        });
    }
    declarations
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

fn parse_compound(text: &str) -> Option<CompoundSelector> {
    // Pseudo-classes/elements are approximate: `p:first-child` matches every `p`, and
    // `::before`/`::after` cannot be generated, so their content is dropped.
    let text = match text.find(':') {
        Some(index) => &text[..index],
        None => text,
    };
    let mut compound = CompoundSelector::default();
    let mut rest = text;
    if rest.is_empty() {
        // A bare pseudo selector (e.g. `::before`) still parses as a universal compound.
        return Some(compound);
    }
    if let Some(stripped) = rest.strip_prefix('*') {
        rest = stripped;
    } else if rest.chars().next().is_some_and(|c| c.is_alphabetic() || c == '-') {
        let end = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
        compound.tag = Some(rest[..end].to_ascii_lowercase());
        rest = &rest[end..];
    }
    while !rest.is_empty() {
        let mut chars = rest.chars();
        match chars.next() {
            Some('.') => {
                let after = chars.as_str();
                let end = after.find(|c: char| !is_ident_char(c)).unwrap_or(after.len());
                if end == 0 {
                    return None;
                }
                compound.classes.push(after[..end].to_string());
                rest = &after[end..];
            }
            Some('#') => {
                let after = chars.as_str();
                let end = after.find(|c: char| !is_ident_char(c)).unwrap_or(after.len());
                if end == 0 {
                    return None;
                }
                if compound.id.is_some() {
                    return None;
                }
                compound.id = Some(after[..end].to_string());
                rest = &after[end..];
            }
            Some('[') => {
                let after = chars.as_str();
                let end = after.find(']')?;
                compound.attrs.push(after[..end].trim().to_string());
                rest = &after[end + 1..];
            }
            _ => {
                // Unsupported syntax (e.g. `+`, `~`, attribute operators we do not model):
                // keep the part parsed so far and ignore the remainder.
                break;
            }
        }
    }
    Some(compound)
}

impl Selector {
    /// Parses a single (non-list) selector such as `div.chapter p.lead`.
    pub fn parse(text: &str) -> Option<Selector> {
        let normalized = text.trim().replace('>', " ");
        let mut parts = Vec::new();
        for token in normalized.split_whitespace() {
            let compound = parse_compound(token)?;
            parts.push(compound);
        }
        if parts.is_empty() {
            return None;
        }
        let mut ids = 0u32;
        let mut classes = 0u32;
        let mut tags = 0u32;
        for part in &parts {
            if part.id.is_some() {
                ids += 1;
            }
            classes += part.classes.len() as u32 + part.attrs.len() as u32;
            if part.tag.is_some() {
                tags += 1;
            }
        }
        Some(Selector {
            parts,
            specificity: ids * 100 + classes * 10 + tags,
        })
    }

    /// CSS specificity: `#id` = 100, `.class`/`[attr]` = 10, tag = 1.
    pub fn specificity(&self) -> u32 {
        self.specificity
    }

    /// Matches an element against this selector (descendant combinators only).
    pub fn matches(&self, el: &ElementRef) -> bool {
        match self.parts.len() {
            0 => false,
            last => self.matches_from(el, last - 1),
        }
    }

    fn matches_from(&self, el: &ElementRef, index: usize) -> bool {
        if !compound_matches(&self.parts[index], el) {
            return false;
        }
        if index == 0 {
            return true;
        }
        let mut ancestor = el.parent();
        while let Some(node) = ancestor {
            if let Some(parent) = ElementRef::wrap(node)
                && self.matches_from(&parent, index - 1)
            {
                return true;
            }
            ancestor = node.parent();
        }
        false
    }
}

fn compound_matches(compound: &CompoundSelector, el: &ElementRef) -> bool {
    if let Some(tag) = &compound.tag
        && !el.value().name().eq_ignore_ascii_case(tag)
    {
        return false;
    }
    if let Some(id) = &compound.id
        && el.value().id() != Some(id.as_str())
    {
        return false;
    }
    for class in &compound.classes {
        if !el.value().classes().any(|candidate| candidate == class) {
            return false;
        }
    }
    for attr in &compound.attrs {
        match attr.split_once('=') {
            Some((name, value)) => {
                let value = value.trim().trim_matches(['"', '\'']);
                if el.attr(name.trim()) != Some(value) {
                    return false;
                }
            }
            None => {
                if el.attr(attr.trim()).is_none() {
                    return false;
                }
            }
        }
    }
    true
}

impl Stylesheet {
    /// Parses an author stylesheet. Malformed constructs are skipped, never rejected.
    pub fn parse(css: &str) -> Self {
        let cleaned = strip_at_rules(&strip_comments(css));
        let mut rules = Vec::new();
        let mut order = 0usize;
        let mut cursor = 0usize;
        while let Some(open_rel) = cleaned[cursor..].find('{') {
            let open = cursor + open_rel;
            let prelude = cleaned[cursor..open].trim();
            let Some(close_rel) = find_matching_brace(&cleaned[open..]) else {
                break;
            };
            let close = open + close_rel;
            let body = &cleaned[open + 1..close];
            cursor = close + 1;

            let selectors: Vec<Selector> = prelude
                .split(',')
                .filter_map(|part| Selector::parse(part.trim()))
                .collect();
            if selectors.is_empty() {
                continue;
            }
            let declarations = parse_declarations(body);
            if declarations.is_empty() {
                continue;
            }
            rules.push(Rule {
                selectors,
                declarations,
                order,
            });
            order += 1;
        }
        Self {
            rules,
            next_order: order,
        }
    }

    /// Appends another stylesheet, preserving source order after the existing rules.
    pub fn extend(&mut self, css: &str) {
        let parsed = Self::parse(css);
        for mut rule in parsed.rules {
            rule.order = self.next_order;
            self.next_order += 1;
            self.rules.push(rule);
        }
    }

    /// Pushes every rule with at least one selector matching `el`, with its best specificity.
    pub fn matching_rules<'a>(&'a self, el: &ElementRef, out: &mut Vec<(&'a Rule, u32)>) {
        for rule in &self.rules {
            let mut best = None;
            for selector in &rule.selectors {
                if selector.matches(el) {
                    best = Some(best.map_or(selector.specificity, |current: u32| {
                        current.max(selector.specificity)
                    }));
                }
            }
            if let Some(specificity) = best {
                out.push((rule, specificity));
            }
        }
    }

    /// Computes the style of one element.
    ///
    /// `parent` is the computed style of the parent element (inherited properties come from it),
    /// `base` is the document root style used for font-size keywords, and `inline` is the raw
    /// `style="..."` attribute, if any.
    pub fn cascade_for(
        &self,
        node: &ElementRef,
        inline: Option<&str>,
        parent: &ComputedStyle,
        base: &ComputedStyle,
    ) -> ComputedStyle {
        let mut style = parent.inherited(base);
        apply_declarations(&mut style, parent, base, &ua_default_for_tag(node.value().name()));

        let inline_declarations = inline.map(parse_declarations).unwrap_or_default();
        let mut matched: Vec<(&Rule, u32)> = Vec::new();
        self.matching_rules(node, &mut matched);
        matched.sort_by_key(|(rule, specificity)| (*specificity, rule.order));

        // Normal declarations: author rules first (low to high specificity), then inline.
        for (rule, _) in &matched {
            apply_important_phase(&mut style, parent, base, &rule.declarations, false);
        }
        apply_important_phase(&mut style, parent, base, &inline_declarations, false);

        // `!important` declarations: author rules, then inline (which wins).
        for (rule, _) in &matched {
            apply_important_phase(&mut style, parent, base, &rule.declarations, true);
        }
        apply_important_phase(&mut style, parent, base, &inline_declarations, true);
        style
    }
}

fn find_matching_brace(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn apply_important_phase(
    style: &mut ComputedStyle,
    parent: &ComputedStyle,
    base: &ComputedStyle,
    declarations: &[Declaration],
    important: bool,
) {
    let filtered: Vec<Declaration> = declarations
        .iter()
        .filter(|declaration| declaration.important == important)
        .cloned()
        .collect();
    apply_declarations(style, parent, base, &filtered);
}

// ---------------------------------------------------------------------------
// Computed style
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub font_size: f32,
    pub font_family: FontFamily,
    pub bold: bool,
    pub italic: bool,
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
    pub align: TextAlignment,
    pub indent: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub padding_left: f32,
    pub padding_right: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    pub margin_left_percent: f32,
    pub margin_right_percent: f32,
    pub padding_left_percent: f32,
    pub padding_right_percent: f32,
    /// Multiplier of the font size (unitless CSS `line-height`); default browser value 1.5.
    pub line_height: f32,
    pub display_none: bool,
    pub white_space_pre: bool,
    pub uppercase: bool,
    pub letter_spacing: f32,
    pub border_left: Option<(f32, Hsla)>,
    pub border_top: Option<(f32, Hsla)>,
    pub border_right: Option<(f32, Hsla)>,
    pub border_bottom: Option<(f32, Hsla)>,
}

impl ComputedStyle {
    /// Document root style: serif, 1.5 line height, no colors (theme supplies them).
    pub fn default_with(base_font_size: f32) -> Self {
        Self {
            font_size: base_font_size.clamp(6.0, 96.0),
            font_family: FontFamily::Serif,
            bold: false,
            italic: false,
            color: None,
            background: None,
            align: TextAlignment::Start,
            indent: 0.0,
            margin_top: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            margin_left_percent: 0.0,
            margin_right_percent: 0.0,
            padding_left_percent: 0.0,
            padding_right_percent: 0.0,
            line_height: 1.5,
            display_none: false,
            white_space_pre: false,
            uppercase: false,
            letter_spacing: 0.0,
            border_left: None,
            border_top: None,
            border_right: None,
            border_bottom: None,
        }
    }

    /// Creates the child style, inheriting only the CSS-inherited properties.
    pub fn inherited(&self, _base: &ComputedStyle) -> ComputedStyle {
        Self {
            font_size: self.font_size.clamp(6.0, 96.0),
            font_family: self.font_family.clone(),
            bold: self.bold,
            italic: self.italic,
            color: self.color,
            background: None,
            align: self.align,
            indent: self.indent,
            margin_top: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            margin_left_percent: 0.0,
            margin_right_percent: 0.0,
            padding_left_percent: self.padding_left_percent,
            padding_right_percent: self.padding_right_percent,
            line_height: self.line_height,
            display_none: false,
            white_space_pre: self.white_space_pre,
            uppercase: self.uppercase,
            letter_spacing: self.letter_spacing,
            border_left: None,
            border_top: None,
            border_right: None,
            border_bottom: None,
        }
    }

    /// Flattens onto the renderer's block style.
    ///
    /// `BlockStyle` has no left/right margins (frozen type), so horizontal margins are folded
    /// into padding — a documented approximation that keeps indented quotes/lists visible.
    pub fn to_block_style(&self) -> BlockStyle {
        BlockStyle {
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            color: self.color,
            background: self.background,
            align: self.align,
            indent: self.indent,
            space_before: self.margin_top,
            space_after: self.margin_bottom,
            line_height: self.line_height,
            padding_left: self.padding_left + self.margin_left.max(0.0),
            padding_right: self.padding_right + self.margin_right.max(0.0),
            padding_top: self.padding_top,
            padding_bottom: self.padding_bottom,
            padding_left_percent: self.padding_left_percent + self.margin_left_percent,
            padding_right_percent: self.padding_right_percent + self.margin_right_percent,
            border_left: self
                .border_left
                .map(|(width, color)| crate::epub::model::Border { width, color }),
            border_top: self
                .border_top
                .map(|(width, color)| crate::epub::model::Border { width, color }),
            border_right: self
                .border_right
                .map(|(width, color)| crate::epub::model::Border { width, color }),
            border_bottom: self
                .border_bottom
                .map(|(width, color)| crate::epub::model::Border { width, color }),
        }
    }
}

fn length_px(value: &str, font_size: f32) -> Option<f32> {
    let value = value.trim();
    let (number, unit) = split_number_unit(value)?;
    let px = match unit.as_str() {
        "px" => number,
        "em" => number * font_size,
        "rem" => number * font_size,
        "pt" => number * 4.0 / 3.0,
        "pc" => number * 16.0,
        "%" => number * font_size / 100.0,
        "ex" | "ch" => number * font_size * 0.5,
        "in" => number * 96.0,
        "cm" => number * 96.0 / 2.54,
        "mm" => number * 96.0 / 25.4,
        "" => {
            // Unitless is only valid for line-height; elsewhere treat as px.
            number
        }
        _ => return None,
    };
    Some(px)
}

fn split_number_unit(value: &str) -> Option<(f32, String)> {
    let value = value.trim();
    let end = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(value.len());
    let number: f32 = value[..end].trim().parse().ok()?;
    let unit = value[end..].trim().to_ascii_lowercase();
    Some((number, unit))
}

fn apply_font_size(style: &mut ComputedStyle, parent: &ComputedStyle, base: &ComputedStyle, value: &str) {
    let value = value.trim().to_ascii_lowercase();
    let parent_size = parent.font_size.max(1.0);
    let size = match value.as_str() {
        "xx-small" => base.font_size * 0.6,
        "x-small" => base.font_size * 0.75,
        "small" => base.font_size * 0.83,
        "medium" => base.font_size,
        "large" => base.font_size * 1.2,
        "x-large" => base.font_size * 1.5,
        "xx-large" => base.font_size * 2.0,
        "smaller" => parent_size * 0.83,
        "larger" => parent_size * 1.2,
        _ => match split_number_unit(&value) {
            Some((number, unit)) if unit == "em" || unit == "rem" => number * parent_size,
            Some((number, unit)) if unit == "%" => number * parent_size / 100.0,
            Some((number, unit)) if unit == "pt" => number * 4.0 / 3.0,
            Some((number, unit)) if unit == "pc" => number * 16.0,
            _ => match length_px(&value, parent_size) {
                Some(px) => px,
                None => return,
            },
        },
    };
    style.font_size = size.clamp(6.0, 96.0);
}

fn first_font_family(value: &str) -> FontFamily {
    let first = value.split(',').next().unwrap_or(value).trim();
    let unquoted = first.trim_matches(['"', '\'']).trim();
    let lower = unquoted.to_ascii_lowercase();
    if lower.contains("monospace")
        || lower.contains("courier")
        || lower.contains("consolas")
        || lower.contains("cascadia")
    {
        FontFamily::Monospace
    } else if lower.contains("sans-serif") || lower.contains("arial") || lower.contains("segoe") {
        FontFamily::SansSerif
    } else if lower.contains("serif")
        || lower.contains("times")
        || lower.contains("georgia")
        || unquoted.is_empty()
    {
        FontFamily::Serif
    } else {
        FontFamily::Named(unquoted.to_string())
    }
}

fn apply_declarations(
    style: &mut ComputedStyle,
    parent: &ComputedStyle,
    base: &ComputedStyle,
    declarations: &[Declaration],
) {
    // Font size first: other `em`-based properties resolve against the element's own font size.
    for declaration in declarations {
        if declaration.name == "font-size" {
            apply_font_size(style, parent, base, &declaration.value);
        }
    }
    for declaration in declarations {
        let value = declaration.value.trim();
        let lower = value.to_ascii_lowercase();
        let font_size = style.font_size.max(1.0);
        match declaration.name.as_str() {
            "font-size" => {}
            "font-family" => style.font_family = first_font_family(value),
            "font-weight" => {
                style.bold = match lower.as_str() {
                    "bold" | "bolder" => true,
                    "normal" | "lighter" => false,
                    _ => lower
                        .parse::<u32>()
                        .map(|weight| weight >= 600)
                        .unwrap_or(style.bold),
                };
            }
            "font-style" => {
                style.italic = matches!(lower.as_str(), "italic" | "oblique");
            }
            "color" => {
                if let Some(color) = parse_color(value) {
                    style.color = Some(color);
                }
            }
            "background-color" | "background" => {
                if let Some(color) = parse_color(value) {
                    style.background = Some(color);
                }
            }
            "text-align" => {
                style.align = match lower.as_str() {
                    "center" => TextAlignment::Center,
                    "right" | "end" => TextAlignment::End,
                    _ => TextAlignment::Start,
                };
            }
            "text-indent" => {
                if let Some(px) = length_px(value, font_size) {
                    style.indent = px.max(0.0);
                }
            }
            "line-height" => {
                if lower == "normal" {
                    style.line_height = 1.5;
                } else if let Some((number, unit)) = split_number_unit(value) {
                    style.line_height = match unit.as_str() {
                        "" => number,
                        "%" => number / 100.0,
                        _ => match length_px(value, font_size) {
                            Some(px) if font_size > 0.0 => px / font_size,
                            _ => style.line_height,
                        },
                    }
                    .clamp(0.5, 6.0);
                }
            }
            "margin" => {
                if let Some((top, right, bottom, left)) = shorthands(value, font_size) {
                    style.margin_top = top;
                    style.margin_right = right;
                    style.margin_bottom = bottom;
                    style.margin_left = left;
                }
                let (_, right, _, left) = shorthand_percents(value);
                style.margin_left_percent = left;
                style.margin_right_percent = right;
            }
            "margin-top" => {
                if let Some(px) = length_px(value, font_size) {
                    style.margin_top = px;
                }
            }
            "margin-right" => {
                if let Some(px) = length_px(value, font_size) {
                    style.margin_right = px;
                }
            }
            "margin-bottom" => {
                if let Some(px) = length_px(value, font_size) {
                    style.margin_bottom = px;
                }
            }
            "margin-left" => {
                if let Some(px) = length_px(value, font_size) {
                    style.margin_left = px;
                }
            }
            "padding" => {
                if let Some((top, right, bottom, left)) = shorthands(value, font_size) {
                    style.padding_top = top.max(0.0);
                    style.padding_right = right.max(0.0);
                    style.padding_bottom = bottom.max(0.0);
                    style.padding_left = left.max(0.0);
                }
                let (_, right, _, left) = shorthand_percents(value);
                style.padding_left_percent = left;
                style.padding_right_percent = right;
            }
            "padding-top" => {
                if let Some(px) = length_px(value, font_size) {
                    style.padding_top = px.max(0.0);
                }
            }
            "padding-bottom" => {
                if let Some(px) = length_px(value, font_size) {
                    style.padding_bottom = px.max(0.0);
                }
            }
            "padding-left" => {
                if let Some(px) = length_px(value, font_size) {
                    style.padding_left = px.max(0.0);
                }
            }
            "padding-right" => {
                if let Some(px) = length_px(value, font_size) {
                    style.padding_right = px.max(0.0);
                }
            }
            "text-transform" => {
                style.uppercase = lower == "uppercase";
            }
            "letter-spacing" => {
                style.letter_spacing = if lower == "normal" {
                    0.0
                } else {
                    length_px(value, font_size).unwrap_or(0.0).max(0.0)
                };
            }
            "display" => {
                style.display_none = lower == "none";
            }
            "white-space" => {
                style.white_space_pre = matches!(lower.as_str(), "pre" | "pre-wrap" | "pre-line");
            }
            "border-left" => {
                if lower == "none" || lower == "0" || lower.is_empty() {
                    style.border_left = None;
                } else if let Some(border) = parse_border(value) {
                    style.border_left = Some(border);
                }
            }
            "border" => {
                if lower == "none" || lower == "0" || lower.is_empty() {
                    style.border_left = None;
                    style.border_top = None;
                    style.border_right = None;
                    style.border_bottom = None;
                } else if let Some(border) = parse_border(value) {
                    style.border_left = Some(border);
                    style.border_top = Some(border);
                    style.border_right = Some(border);
                    style.border_bottom = Some(border);
                }
            }
            "border-top" => {
                if lower == "none" || lower == "0" || lower.is_empty() {
                    style.border_top = None;
                } else if let Some(border) = parse_border(value) {
                    style.border_top = Some(border);
                }
            }
            "border-right" => {
                if lower == "none" || lower == "0" || lower.is_empty() {
                    style.border_right = None;
                } else if let Some(border) = parse_border(value) {
                    style.border_right = Some(border);
                }
            }
            "border-bottom" => {
                if lower == "none" || lower == "0" || lower.is_empty() {
                    style.border_bottom = None;
                } else if let Some(border) = parse_border(value) {
                    style.border_bottom = Some(border);
                }
            }
            "border-left-width" => {
                if let Some(px) = length_px(value, font_size) {
                    let color = style
                        .border_left
                        .map(|(_, color)| color)
                        .unwrap_or_else(|| rgb(0x000000).into());
                    style.border_left = Some((px, color));
                }
            }
            _ => {}
        }
    }
}

/// Expands a CSS box shorthand (1-4 lengths) into `(top, right, bottom, left)`.
/// Parses percentage components of a CSS shorthand (`margin: 0 5%` → right/left 5).
fn shorthand_percents(value: &str) -> (f32, f32, f32, f32) {
    fn pct(token: &str) -> f32 {
        token
            .trim()
            .strip_suffix('%')
            .and_then(|number| number.trim().parse::<f32>().ok())
            .unwrap_or(0.0)
            .max(0.0)
    }
    let tokens: Vec<&str> = value.split_whitespace().filter(|t| !t.is_empty()).collect();
    let Some(first) = tokens.first() else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let top = pct(first);
    let right = tokens.get(1).map_or(top, |t| pct(t));
    let bottom = tokens.get(2).map_or(top, |t| pct(t));
    let left = tokens.get(3).map_or(right, |t| pct(t));
    (top, right, bottom, left)
}

fn shorthands(value: &str, font_size: f32) -> Option<(f32, f32, f32, f32)> {
    let values: Vec<&str> = value.split_whitespace().collect();
    let resolved: Vec<f32> = values
        .iter()
        .map(|value| length_px(value, font_size))
        .collect::<Option<Vec<f32>>>()?;
    match resolved.as_slice() {
        [all] => Some((*all, *all, *all, *all)),
        [vertical, horizontal] => Some((*vertical, *horizontal, *vertical, *horizontal)),
        [top, horizontal, bottom] => Some((*top, *horizontal, *bottom, *horizontal)),
        [top, right, bottom, left, ..] => Some((*top, *right, *bottom, *left)),
        [] => None,
    }
}

/// Parses `border`/`border-left`: `<width> <style> <color>` in any order (style optional).
fn parse_border(value: &str) -> Option<(f32, Hsla)> {
    let mut width = 1.0f32;
    let mut color = None;
    let mut has_style = false;
    for token in value.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
        ) {
            has_style = true;
        } else if lower == "none" || lower == "hidden" {
            return None;
        } else if let Some(px) = length_px(&lower, 16.0) {
            width = px;
        } else if let Some(parsed) = parse_color(token) {
            color = Some(parsed);
        }
    }
    if width <= 0.0 {
        return None;
    }
    let color = color.unwrap_or_else(|| rgb(0x000000).into());
    if has_style || !value.trim().is_empty() {
        Some((width, color))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

/// Parses CSS colors: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()` and a small named set.
///
/// `currentcolor` intentionally returns `None` (the renderer resolves it from the theme).
pub fn parse_color(value: &str) -> Option<Hsla> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let lower = value.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
    {
        let inner = inner.strip_suffix(')').unwrap_or(inner);
        return parse_rgb_function(inner);
    }
    if lower == "transparent" {
        return Some(rgba(0x0000_0000).into());
    }
    if lower == "currentcolor" {
        return None;
    }
    named_color(&lower).map(|hex| rgb(hex).into())
}

fn parse_hex_color(hex: &str) -> Option<Hsla> {
    let digits: Vec<char> = hex.chars().collect();
    let expand = |chars: &[char]| -> Option<u8> {
        let text: String = chars.iter().collect();
        let value = u8::from_str_radix(&text, 16).ok()?;
        Some(value * 16 + value)
    };
    match digits.len() {
        3 => Some(
            rgb(expand(&digits[0..1])? as u32 * 0x10000
                + expand(&digits[1..2])? as u32 * 0x100
                + expand(&digits[2..3])? as u32)
                .into(),
        ),
        6 => {
            let hex = u32::from_str_radix(hex, 16).ok()?;
            Some(rgb(hex).into())
        }
        8 => {
            let hex = u32::from_str_radix(hex, 16).ok()?;
            Some(rgba(hex).into())
        }
        _ => None,
    }
}

fn parse_rgb_function(inner: &str) -> Option<Hsla> {
    let normalized = inner.replace('/', " ");
    let tokens: Vec<String> = normalized
        .split([',', ' '])
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.len() < 3 {
        return None;
    }
    let red = parse_color_channel(&tokens[0])?;
    let green = parse_color_channel(&tokens[1])?;
    let blue = parse_color_channel(&tokens[2])?;
    let alpha = match tokens.get(3) {
        Some(token) => parse_color_alpha(token)?,
        None => 1.0,
    };
    let hex = ((red as u32) << 16) | ((green as u32) << 8) | blue as u32;
    if (alpha - 1.0).abs() < f32::EPSILON {
        Some(rgb(hex).into())
    } else {
        let alpha_byte = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
        // `gpui::rgba` expects `0xRRGGBBAA`.
        Some(rgba((hex << 8) | alpha_byte).into())
    }
}

fn parse_color_channel(token: &str) -> Option<u8> {
    let token = token.trim();
    if let Some(percent) = token.strip_suffix('%') {
        let value: f32 = percent.trim().parse().ok()?;
        Some((value * 255.0 / 100.0).clamp(0.0, 255.0).round() as u8)
    } else {
        let value: f32 = token.parse().ok()?;
        Some(value.clamp(0.0, 255.0).round() as u8)
    }
}

fn parse_color_alpha(token: &str) -> Option<f32> {
    let token = token.trim();
    if let Some(percent) = token.strip_suffix('%') {
        let value: f32 = percent.trim().parse().ok()?;
        Some((value / 100.0).clamp(0.0, 1.0))
    } else {
        let value: f32 = token.parse().ok()?;
        Some(value.clamp(0.0, 1.0))
    }
}

fn named_color(name: &str) -> Option<u32> {
    Some(match name {
        "black" => 0x000000,
        "white" => 0xffffff,
        "red" => 0xff0000,
        "green" => 0x008000,
        "blue" => 0x0000ff,
        "gray" | "grey" => 0x808080,
        "silver" => 0xc0c0c0,
        "maroon" => 0x800000,
        "olive" => 0x808000,
        "lime" => 0x00ff00,
        "aqua" | "cyan" => 0x00ffff,
        "teal" => 0x008080,
        "navy" => 0x000080,
        "fuchsia" | "magenta" => 0xff00ff,
        "purple" => 0x800080,
        "orange" => 0xffa500,
        "yellow" => 0xffff00,
        "pink" => 0xffc0cb,
        "brown" => 0xa52a2a,
        "gold" => 0xffd700,
        "indigo" => 0x4b0082,
        "violet" => 0xee82ee,
        "beige" => 0xf5f5dc,
        "ivory" => 0xfffff0,
        "khaki" => 0xf0e68c,
        "salmon" => 0xfa8072,
        "tan" => 0xd2b48c,
        "chocolate" => 0xd2691e,
        "crimson" => 0xdc143c,
        "darkblue" => 0x00008b,
        "darkgreen" => 0x006400,
        "darkred" => 0x8b0000,
        "darkgray" | "darkgrey" => 0xa9a9a9,
        "lightgray" | "lightgrey" => 0xd3d3d3,
        "lightblue" => 0xadd8e6,
        "lightgreen" => 0x90ee90,
        "rebeccapurple" => 0x663399,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// UA defaults
// ---------------------------------------------------------------------------

/// Browser-like UA declarations for a tag, applied before author rules at specificity 0/order 0.
pub fn ua_default_for_tag(tag: &str) -> Vec<Declaration> {
    ua_declarations(&tag.to_ascii_lowercase())
        .iter()
        .map(|(name, value)| Declaration {
            name: (*name).to_string(),
            value: (*value).to_string(),
            important: false,
        })
        .collect()
}

fn ua_declarations(tag: &str) -> Vec<(&'static str, &'static str)> {
    match tag {
        "html" => vec![("display", "block")],
        "body" => vec![("margin", "0"), ("padding", "0")],
        "p" => vec![("margin", "1em 0")],
        "h1" => vec![
            ("font-size", "2em"),
            ("font-weight", "bold"),
            ("margin", "0.67em 0"),
        ],
        "h2" => vec![
            ("font-size", "1.5em"),
            ("font-weight", "bold"),
            ("margin", "0.83em 0"),
        ],
        "h3" => vec![
            ("font-size", "1.17em"),
            ("font-weight", "bold"),
            ("margin", "1em 0"),
        ],
        "h4" => vec![
            ("font-size", "1em"),
            ("font-weight", "bold"),
            ("margin", "1.33em 0"),
        ],
        "h5" => vec![
            ("font-size", "0.83em"),
            ("font-weight", "bold"),
            ("margin", "1.67em 0"),
        ],
        "h6" => vec![
            ("font-size", "0.67em"),
            ("font-weight", "bold"),
            ("margin", "2.33em 0"),
        ],
        "blockquote" => vec![("margin", "1em 40px")],
        "figure" => vec![("margin", "1em 40px")],
        "ul" | "ol" => vec![("padding-left", "40px"), ("margin", "1em 0")],
        "li" => vec![("display", "list-item")],
        "dd" => vec![("margin-left", "40px")],
        "pre" => vec![
            ("margin", "1em 0"),
            ("font-family", "monospace"),
            ("white-space", "pre"),
        ],
        "code" | "kbd" | "samp" | "tt" => vec![("font-family", "monospace")],
        "strong" | "b" => vec![("font-weight", "bold")],
        "em" | "i" | "cite" | "var" => vec![("font-style", "italic")],
        "small" => vec![("font-size", "0.83em")],
        "big" => vec![("font-size", "1.2em")],
        "sup" | "sub" => vec![("font-size", "0.83em")],
        "center" => vec![("text-align", "center")],
        "a" => vec![],
        _ => vec![],
    }
}




