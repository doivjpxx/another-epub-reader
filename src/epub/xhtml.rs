//! XHTML → [`crate::epub::model::BlockModel`] walker.
//!
//! Pipeline: `XHTML → scraper HTML DOM → collect <style>/<link> CSS → cascade → walk body`.
//! The walker is deliberately forgiving: unsupported elements are skipped (their text is
//! dropped rather than rendered raw), unsupported CSS is ignored, and no input can panic.

use crate::epub::Book;
use crate::epub::css::{ComputedStyle, Stylesheet};
use crate::epub::model::{
    Block, BlockModel, BlockStyle, ContainerBlock, ImageBlock, Inline, InlineStyle, ListBlock, ListItem,
    TableBlock, TableCell, TableRow, TextAlignment,
};
use crate::theme::CONTENT_DEFAULT_FONT_SIZE;
use scraper::{ElementRef, Html, Node, Selector as ScraperSelector};
use std::collections::HashMap;

/// Elements whose whole subtree is dropped (no text render).
const SKIP_TAGS: &[&str] = &[
    "script",
    "style",
    "head",
    "title",
    "meta",
    "link",
    "base",
    "noscript",
    "template",
    "svg",
    "math",
    "audio",
    "video",
    "iframe",
    "object",
    "embed",
    "canvas",
    "param",
    "source",
    "track",
];

/// Block-level elements: they interrupt inline runs and become their own blocks.
const BLOCK_TAGS: &[&str] = &[
    "address", "article", "aside", "blockquote", "body", "center", "dd", "div", "dl", "dt",
    "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6",
    "header", "hr", "html", "li", "main", "nav", "ol", "p", "pre", "section", "table", "tbody",
    "td", "tfoot", "th", "thead", "tr", "ul",
];

struct Ctx<'a> {
    sheet: &'a Stylesheet,
    base: &'a ComputedStyle,
}

/// The result of walking one XHTML document: rendered blocks plus fragment anchors.
struct WalkResult {
    blocks: Vec<Block>,
    anchors: HashMap<String, usize>,
    body_style: BlockStyle,
}

/// Accumulates emitted blocks while associating fragment anchors with them.
///
/// Anchors (`id` attributes and legacy `<a name>`) are queued as *pending* and bound to the next
/// emitted block, which gives the semantics the reader needs:
/// - a block element's own `id` maps to that block,
/// - `<a id/name>` inside a block maps to the paragraph/heading that contains it,
/// - anchors between blocks map to the next block.
///
/// Duplicate keys are first-wins; empty keys are ignored.
#[derive(Default)]
struct WalkOutput {
    blocks: Vec<Block>,
    anchors: HashMap<String, usize>,
    pending: Vec<String>,
}

impl WalkOutput {
    /// Queues the `id` of `el` (and `<a name>` for legacy anchors).
    fn note_element(&mut self, el: &ElementRef) {
        if let Some(id) = el.attr("id") {
            self.note(id);
        }
        if el.value().name() == "a"
            && let Some(name) = el.attr("name")
        {
            self.note(name);
        }
    }

    fn note(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty() || self.anchors.contains_key(key) || self.pending.iter().any(|p| p == key)
        {
            return;
        }
        self.pending.push(key.to_string());
    }

    /// Emits a top-level block, binding all queued anchors to it.
    fn push_block(&mut self, block: Block) {
        let index = self.blocks.len();
        for key in self.pending.drain(..) {
            self.anchors.entry(key).or_insert(index);
        }
        self.blocks.push(block);
    }

    /// Queues every anchor discovered while walking nested content (quote/list/table interiors),
    /// so they resolve to the enclosing top-level block.
    fn absorb(&mut self, nested: &WalkOutput) {
        for key in nested.anchors.keys().chain(nested.pending.iter()) {
            self.note(key);
        }
    }

    fn finish(self, body_style: BlockStyle) -> WalkResult {
        WalkResult {
            blocks: self.blocks,
            anchors: self.anchors,
            body_style,
        }
    }
}

/// Parses a standalone XHTML fragment (used by tests and for CSS-only content).
pub fn parse_html_fragment(html: &str, base_font_size: f32) -> anyhow::Result<BlockModel> {
    let doc = Html::parse_document(html);
    let mut sheet = Stylesheet::default();
    collect_styles(&doc, None, &mut sheet);
    let base = ComputedStyle::default_with(base_font_size);
    let walked = walk_document(&doc, &sheet, &base);
    Ok(BlockModel {
        blocks: walked.blocks,
        anchors: walked.anchors,
        body_style: walked.body_style,
    })
}

/// Parses one EPUB chapter resource, resolving `<link rel="stylesheet">` through the book.
///
/// The model is computed against [`CONTENT_DEFAULT_FONT_SIZE`]; see [`Book::chapter`].
pub fn parse_chapter(book: &Book, chapter_index: usize, xhtml: &str) -> anyhow::Result<BlockModel> {
    let doc = Html::parse_document(xhtml);
    let mut sheet = Stylesheet::default();
    collect_styles(&doc, Some((book, chapter_index)), &mut sheet);
    let base = ComputedStyle::default_with(CONTENT_DEFAULT_FONT_SIZE);
    let walked = walk_document(&doc, &sheet, &base);
    Ok(BlockModel {
        blocks: walked.blocks,
        anchors: walked.anchors,
        body_style: walked.body_style,
    })
}

// ---------------------------------------------------------------------------
// Stylesheets
// ---------------------------------------------------------------------------

/// Collects `<style>` element bodies and (when a book is supplied) linked stylesheets in
/// document order. Missing linked stylesheets are skipped (warn-by-omission).
fn collect_styles(doc: &Html, book: Option<(&Book, usize)>, sheet: &mut Stylesheet) {
    let Ok(selector) = ScraperSelector::parse("style, link") else {
        return;
    };
    for element in doc.select(&selector) {
        match element.value().name() {
            "style" => {
                let css: String = element.text().collect();
                if !css.trim().is_empty() {
                    sheet.extend(&css);
                }
            }
            "link" => {
                let Some((book, chapter_index)) = book else {
                    continue;
                };
                let rel = element.attr("rel").unwrap_or("").to_ascii_lowercase();
                if !rel.split_whitespace().any(|token| token == "stylesheet") {
                    continue;
                }
                let Some(href) = element.attr("href") else {
                    continue;
                };
                if let Ok(css) = book.resource_str(chapter_index, href) {
                    sheet.extend(&css);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Document walk
// ---------------------------------------------------------------------------

fn find_element<'a>(doc: &'a Html, tag: &str) -> Option<ElementRef<'a>> {
    let selector = ScraperSelector::parse(tag).ok()?;
    doc.select(&selector).next()
}

fn walk_document(doc: &Html, sheet: &Stylesheet, base: &ComputedStyle) -> WalkResult {
    let ctx = Ctx { sheet, base };
    let mut out = WalkOutput::default();
    let html = find_element(doc, "html");
    let mut document_style = base.clone();
    if let Some(html) = &html {
        document_style = sheet.cascade_for(html, html.attr("style"), base, base);
        out.note_element(html);
    }
    let Some(body) = find_element(doc, "body") else {
        // Degenerate documents (no <body>): walk whatever the root element holds.
        if let Some(html) = html {
            walk_children(&html, &ctx, &document_style, &mut out);
        }
        return out.finish(document_style.to_block_style());
    };
    let body_style = sheet.cascade_for(&body, body.attr("style"), &document_style, base);
    if body_style.display_none {
        return out.finish(document_style.to_block_style());
    }
    out.note_element(&body);
    walk_children(&body, &ctx, &body_style, &mut out);
    out.finish(body_style.to_block_style())
}

/// Walks the children of a container, interleaving inline runs (paragraphs) with block children.
fn walk_children(
    parent: &ElementRef,
    ctx: &Ctx,
    style: &ComputedStyle,
    out: &mut WalkOutput,
) {
    let mut runs: Vec<Inline> = Vec::new();
    let fallback = inline_style_from_computed(style);
    for node in parent.children() {
        match node.value() {
            Node::Text(text) => runs.push(Inline::Text {
                text: text.to_string(),
                style: fallback,
            }),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(node) else {
                    continue;
                };
                let tag = child.value().name();
                if tag == "br" {
                    runs.push(Inline::Break);
                } else if tag == "img" {
                    flush_runs(&mut runs, out, style);
                    out.note_element(&child);
                    if let Some(block) = image_block(&child, ctx, style) {
                        out.push_block(block);
                    }
                } else if SKIP_TAGS.contains(&tag) {
                    continue;
                } else if BLOCK_TAGS.contains(&tag) {
                    flush_runs(&mut runs, out, style);
                    walk_element(&child, ctx, style, out);
                } else if has_block_content(&child) {
                    // An inline element that wraps block children — e.g. the reconstructed
                    // `<a id="chap01"/>` html5ever creates for Gutenberg's self-closing heading
                    // anchors, which ends up parenting every `<p>` of the chapter. Treat it as
                    // a container instead of flattening the whole chapter into one paragraph.
                    flush_runs(&mut runs, out, style);
                    walk_element(&child, ctx, style, out);
                } else {
                    out.note_element(&child);
                    runs.extend(collect_inline_element(&child, ctx, style, fallback, out));
                }
            }
            _ => {}
        }
    }
    flush_runs(&mut runs, out, style);
}

/// Whether an element's subtree contains block-level content (or an image).
///
/// Used to distinguish a genuinely inline element from a container masquerading as one.
fn has_block_content(el: &ElementRef) -> bool {
    el.descendent_elements().any(|descendant| {
        let tag = descendant.value().name();
        BLOCK_TAGS.contains(&tag) || tag == "img"
    })
}

fn walk_element(
    el: &ElementRef,
    ctx: &Ctx,
    parent_style: &ComputedStyle,
    out: &mut WalkOutput,
) {
    let style = ctx
        .sheet
        .cascade_for(el, el.attr("style"), parent_style, ctx.base);
    walk_element_styled(el, ctx, &style, out);
}

fn walk_element_styled(
    el: &ElementRef,
    ctx: &Ctx,
    style: &ComputedStyle,
    out: &mut WalkOutput,
) {
    let tag = el.value().name();
    if SKIP_TAGS.contains(&tag) || style.display_none {
        return;
    }
    out.note_element(el);
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let fallback = inline_style_from_computed(style);
            let runs = collapse_runs(collect_inline_element(el, ctx, style, fallback, out));
            if runs.is_empty() {
                return;
            }
            let level = tag.as_bytes()[1].saturating_sub(b'0');
            out.push_block(Block::Heading {
                level,
                runs,
                style: style.to_block_style(),
            });
        }
        "ul" | "ol" => {
            let mut nested = WalkOutput::default();
            let list = walk_list(el, ctx, style, &mut nested);
            if list.items.is_empty() {
                return;
            }
            // Anchors inside list items resolve to the enclosing list block.
            out.absorb(&nested);
            out.push_block(Block::List(list));
        }
        "blockquote" => {
            let mut nested = WalkOutput::default();
            walk_children(el, ctx, style, &mut nested);
            if nested.blocks.is_empty() {
                return;
            }
            out.absorb(&nested);
            out.push_block(Block::Quote {
                blocks: nested.blocks,
                style: style.to_block_style(),
            });
        }
        "pre" => {
            let mut text: String = el.text().collect();
            // The newline immediately after `<pre>` is ignored by browsers; do the same.
            if let Some(stripped) = text.strip_prefix('\n') {
                text = stripped.to_string();
            }
            let text = text.trim_end_matches(['\n', '\r']).to_string();
            out.push_block(Block::Preformatted {
                text,
                style: style.to_block_style(),
            });
        }
        "hr" => out.push_block(Block::Rule),
        "img" => {
            if let Some(block) = image_block(el, ctx, style) {
                out.push_block(block);
            }
        }
        "table" => {
            let mut nested = WalkOutput::default();
            let table = walk_table(el, ctx, style, &mut nested);
            out.absorb(&nested);
            out.push_block(Block::Table(table));
        }
        "figcaption" => {
            let fallback = inline_style_from_computed(style);
            let runs = collapse_runs(collect_inline_element(el, ctx, style, fallback, out));
            if runs.is_empty() {
                return;
            }
            out.push_block(Block::Paragraph {
                runs,
                style: style.to_block_style(),
            });
        }
        _ => {
            if is_container_tag(tag) && has_visual_box(style) {
                let mut nested = WalkOutput::default();
                walk_children(el, ctx, style, &mut nested);
                if !nested.blocks.is_empty() {
                    // Anchors inside a visual box resolve to the enclosing top-level block.
                    out.absorb(&nested);
                    out.push_block(Block::Container(ContainerBlock {
                        blocks: nested.blocks,
                        style: style.to_block_style(),
                    }));
                }
            } else {
                walk_children(el, ctx, style, out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

fn walk_list(
    el: &ElementRef,
    ctx: &Ctx,
    style: &ComputedStyle,
    nested: &mut WalkOutput,
) -> ListBlock {
    let ordered = el.value().name() == "ol";
    let start = el
        .attr("start")
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(1);
    let mut items = Vec::new();
    let mut index = 0u32;
    for child in el.child_elements() {
        if child.value().name() != "li" {
            continue;
        }
        let item_style = ctx
            .sheet
            .cascade_for(&child, child.attr("style"), style, ctx.base);
        if item_style.display_none {
            continue;
        }
        let (runs, blocks) = collect_list_item(&child, ctx, &item_style, nested);
        let marker = if ordered {
            format!("{}.", start.saturating_add(index))
        } else {
            "\u{2022}".to_string()
        };
        items.push(ListItem {
            runs,
            blocks,
            marker,
            style: item_style.to_block_style(),
        });
        index += 1;
    }
    ListBlock {
        ordered,
        start,
        items,
        style: style.to_block_style(),
    }
}

/// Collects an `<li>`: direct inline content becomes `runs`, nested block content becomes
/// `blocks` (nested lists, paragraphs, ...).
fn collect_list_item(
    el: &ElementRef,
    ctx: &Ctx,
    style: &ComputedStyle,
    nested: &mut WalkOutput,
) -> (Vec<Inline>, Vec<Block>) {
    let mut runs = Vec::new();
    let fallback = inline_style_from_computed(style);
    let blocks_start = nested.blocks.len();
    nested.note_element(el);
    for node in el.children() {
        match node.value() {
            Node::Text(text) => runs.push(Inline::Text {
                text: text.to_string(),
                style: fallback,
            }),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(node) else {
                    continue;
                };
                let tag = child.value().name();
                if tag == "br" {
                    runs.push(Inline::Break);
                } else if SKIP_TAGS.contains(&tag) {
                    continue;
                } else if BLOCK_TAGS.contains(&tag) || tag == "img" || has_block_content(&child) {
                    walk_element(&child, ctx, style, nested);
                } else {
                    nested.note_element(&child);
                    runs.extend(collect_inline_element(&child, ctx, style, fallback, nested));
                }
            }
            _ => {}
        }
    }
    let blocks = nested.blocks.split_off(blocks_start);
    (collapse_runs(runs), blocks)
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

fn walk_table(
    el: &ElementRef,
    ctx: &Ctx,
    style: &ComputedStyle,
    anchors: &mut WalkOutput,
) -> TableBlock {
    let mut rows = Vec::new();
    for row_el in el
        .descendent_elements()
        .filter(|element| element.value().name() == "tr")
    {
        anchors.note_element(&row_el);
        let mut cells = Vec::new();
        for cell_el in row_el.child_elements().filter(|element| {
            matches!(element.value().name(), "td" | "th")
        }) {
            anchors.note_element(&cell_el);
            let cell_style = ctx
                .sheet
                .cascade_for(&cell_el, cell_el.attr("style"), style, ctx.base);
            let fallback = inline_style_from_computed(&cell_style);
            let runs = collapse_runs(collect_inline_element(
                &cell_el,
                ctx,
                &cell_style,
                fallback,
                anchors,
            ));
            let header = cell_el.value().name() == "th";
            let align = match cell_el.attr("align").map(str::to_ascii_lowercase).as_deref() {
                Some("center") => TextAlignment::Center,
                Some("right") => TextAlignment::End,
                Some("left") => TextAlignment::Start,
                _ => cell_style.align,
            };
            cells.push(TableCell {
                runs,
                header,
                align,
            });
        }
        if !cells.is_empty() {
            rows.push(TableRow { cells });
        }
    }
    TableBlock {
        rows,
        style: style.to_block_style(),
    }
}

// ---------------------------------------------------------------------------
// Inline content
// ---------------------------------------------------------------------------

fn inline_style_from_computed(style: &ComputedStyle) -> InlineStyle {
    InlineStyle {
        bold: style.bold,
        italic: style.italic,
        uppercase: style.uppercase,
        letter_spacing: style.letter_spacing,
        ..InlineStyle::default()
    }
}

/// Elements that preserve their visual box (border/background/padding/margin) as a
/// [`ContainerBlock`] instead of being flattened into their parent.
fn is_container_tag(tag: &str) -> bool {
    matches!(
        tag,
        "div"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "figure"
            | "main"
            | "fieldset"
            | "form"
            | "address"
            | "center"
    )
}

fn has_visual_box(style: &ComputedStyle) -> bool {
    style.background.is_some()
        || style.border_left.is_some()
        || style.border_top.is_some()
        || style.border_right.is_some()
        || style.border_bottom.is_some()
        || style.padding_left > 0.0
        || style.padding_right > 0.0
        || style.padding_top > 0.0
        || style.padding_bottom > 0.0
        || style.margin_top > 0.0
        || style.margin_bottom > 0.0
}

/// Collects the inline runs of an element's subtree, flattening nested block content.
fn collect_inline_element(
    el: &ElementRef,
    ctx: &Ctx,
    parent_style: &ComputedStyle,
    inherited: InlineStyle,
    out: &mut WalkOutput,
) -> Vec<Inline> {
    let tag = el.value().name();
    if SKIP_TAGS.contains(&tag) {
        return Vec::new();
    }
    out.note_element(el);
    let computed = ctx
        .sheet
        .cascade_for(el, el.attr("style"), parent_style, ctx.base);
    if computed.display_none {
        return Vec::new();
    }
    let mut inline = inherited;
    // CSS `font-weight`/`font-style` are inherited; reflect the computed values.
    inline.bold = computed.bold;
    inline.italic = computed.italic;
    inline.uppercase = computed.uppercase;
    inline.letter_spacing = computed.letter_spacing;
    match tag {
        "b" | "strong" => inline.bold = true,
        "i" | "em" | "cite" | "var" => inline.italic = true,
        "u" | "ins" => inline.underline = true,
        "s" | "strike" | "del" => inline.strikethrough = true,
        "code" | "kbd" | "samp" | "tt" => inline.code = true,
        "sup" => inline.superscript = true,
        "sub" => inline.subscript = true,
        "a" if el.attr("href").is_some() => inline.link = true,
        _ => {}
    }

    let mut runs = Vec::new();
    for node in el.children() {
        match node.value() {
            Node::Text(text) => runs.push(Inline::Text {
                text: text.to_string(),
                style: inline,
            }),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(node) else {
                    continue;
                };
                let child_tag = child.value().name();
                if child_tag == "br" {
                    runs.push(Inline::Break);
                } else if child_tag == "img" {
                    // Nested images (e.g. inside a link) are not represented inline; the
                    // block walker only emits top-level images (documented degrade).
                    continue;
                } else {
                    runs.extend(collect_inline_element(&child, ctx, &computed, inline, out));
                }
            }
            _ => {}
        }
    }
    // `<a href="...">` content becomes clickable link runs (same styling as the surrounding text,
    // plus the theme link color/underline); the reader resolves the target on click.
    if tag == "a"
        && let Some(href) = el.attr("href").map(str::trim).filter(|href| !href.is_empty())
    {
        let href = href.to_string();
        runs = runs
            .into_iter()
            .map(|run| match run {
                Inline::Text { text, style } => Inline::Link {
                    text,
                    style,
                    href: href.clone(),
                },
                other => other,
            })
            .collect();
    }
    runs
}

fn image_block(el: &ElementRef, ctx: &Ctx, parent_style: &ComputedStyle) -> Option<Block> {
    let src = el.attr("src")?.trim();
    if src.is_empty() {
        return None;
    }
    let style = ctx
        .sheet
        .cascade_for(el, el.attr("style"), parent_style, ctx.base);
    if style.display_none {
        return None;
    }
    Some(Block::Image(ImageBlock {
        src: src.to_string(),
        alt: el.attr("alt").unwrap_or("").trim().to_string(),
        width: parse_dimension(el.attr("width")),
        height: parse_dimension(el.attr("height")),
        style: style.to_block_style(),
    }))
}

fn parse_dimension(value: Option<&str>) -> Option<f32> {
    let value = value?.trim();
    let numeric = value.strip_suffix("px").unwrap_or(value).trim();
    numeric
        .parse::<f32>()
        .ok()
        .filter(|dimension| *dimension > 0.0)
}

// ---------------------------------------------------------------------------
// Paragraph flushing and whitespace collapsing
// ---------------------------------------------------------------------------

fn flush_runs(runs: &mut Vec<Inline>, out: &mut WalkOutput, style: &ComputedStyle) {
    let collected = std::mem::take(runs);
    let collapsed = if style.white_space_pre {
        collected
    } else {
        collapse_runs(collected)
    };
    if collapsed.is_empty() {
        return;
    }
    let has_content = collapsed.iter().any(|run| match run {
        Inline::Break => true,
        Inline::Text { text, .. } | Inline::Link { text, .. } => {
            text.chars().any(|ch| !ch.is_whitespace())
        }
    });
    if !has_content {
        return;
    }
    out.push_block(Block::Paragraph {
        runs: collapsed,
        style: style.to_block_style(),
    });
}

/// Collapses HTML whitespace: runs of spaces/tabs/newlines become one space, block-edge and
/// line-start spaces are dropped, and adjacent runs with identical styles are merged.
fn collapse_runs(runs: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut pending_space = false;
    let mut at_line_start = true;
    for run in runs {
        match run {
            Inline::Break => {
                trim_trailing_space(&mut out);
                out.push(Inline::Break);
                pending_space = false;
                at_line_start = true;
            }
            Inline::Text { text, style } => {
                collapse_into(
                    &mut out,
                    text,
                    style,
                    None,
                    &mut pending_space,
                    &mut at_line_start,
                );
            }
            Inline::Link { text, style, href } => {
                collapse_into(
                    &mut out,
                    text,
                    style,
                    Some(href),
                    &mut pending_space,
                    &mut at_line_start,
                );
            }
        }
    }
    trim_trailing_space(&mut out);
    out
}

fn collapse_into(
    out: &mut Vec<Inline>,
    text: String,
    style: InlineStyle,
    href: Option<String>,
    pending_space: &mut bool,
    at_line_start: &mut bool,
) {
    let mut built = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            *pending_space = true;
        } else {
            if *pending_space && !*at_line_start {
                built.push(' ');
            }
            *pending_space = false;
            *at_line_start = false;
            built.push(ch);
        }
    }
    // Whitespace between the preceding text and a link belongs to the plain text, not to the
    // clickable link range (so hit-testing/cursor covers only the visible link words).
    if href.is_some() && built.starts_with(' ') {
        push_merged(
            out,
            Inline::Text {
                text: " ".to_string(),
                style,
            },
        );
        built = built.trim_start_matches(' ').to_string();
    }
    if built.is_empty() {
        return;
    }
    let run = match href {
        Some(href) => Inline::Link {
            text: built,
            style,
            href,
        },
        None => Inline::Text { text: built, style },
    };
    push_merged(out, run);
}

fn push_merged(out: &mut Vec<Inline>, run: Inline) {
    match &run {
        Inline::Text { text, style } => {
            if let Some(Inline::Text {
                text: previous_text,
                style: previous_style,
            }) = out.last_mut()
                && *previous_style == *style
            {
                previous_text.push_str(text);
                return;
            }
        }
        Inline::Link { text, style, href } => {
            if let Some(Inline::Link {
                text: previous_text,
                style: previous_style,
                href: previous_href,
            }) = out.last_mut()
                && *previous_style == *style
                && *previous_href == *href
            {
                previous_text.push_str(text);
                return;
            }
        }
        Inline::Break => {}
    }
    out.push(run);
}

fn trim_trailing_space(out: &mut Vec<Inline>) {
    let text = match out.last_mut() {
        Some(Inline::Text { text, .. }) | Some(Inline::Link { text, .. }) => text,
        _ => return,
    };
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    if text.is_empty() {
        out.pop();
    }
}





