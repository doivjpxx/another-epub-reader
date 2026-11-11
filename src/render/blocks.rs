//! Per-block GPUI element builders for the reader view.
//!
//! Parity styles come from the design spec §6.1/§6.2: body base size is `settings.font_size`
//! (14..30), unitless line-height defaults to 1.5, block spacing comes from the CSS/UA cascade in
//! the engine. `BlockStyle.font_size` and all `BlockStyle` lengths are absolute px computed at
//! `theme::CONTENT_DEFAULT_FONT_SIZE` (20.0), so the renderer scales them by
//! `settings.font_size / CONTENT_DEFAULT_FONT_SIZE` to make font-size changes affect the whole
//! content tree (including headings, quotes and indents).
//!
//! Known approximations (documented in the design spec):
//! - GPUI has no `text-indent`, so CSS `text-indent` becomes left padding on the paragraph.
//! - Per-run font size/family inside a paragraph is unsupported; `code` switches the whole
//!   paragraph to a monospace family when every run is code, `sub`/`sup` degrade to plain runs.
//! - Table `colspan`/`rowspan` are engine-side ignored (equal-width columns here).

use crate::epub::Book;
use crate::epub::model::{
    Block, BlockStyle, ListBlock, ListItem, TableBlock, TableCell, TextAlignment,
};
use crate::render::images::{self, ImageCache};
use crate::render::text;
use crate::theme::{self, Settings, Theme};
use gpui::prelude::*;
use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, InteractiveText, SharedString, Stateful,
    StyledText, TextAlign, div, px, relative,
};
use std::ops::Range;
use std::rc::Rc;

/// Callback invoked when an inline `<a href>` link inside rendered text is clicked. The href is
/// passed raw (it may be `#anchor`, `chapter.xhtml#anchor`, or a relative path); the reader
/// resolves it against the current chapter and navigates.
pub type LinkHandler = Rc<dyn Fn(&str, &mut gpui::Window, &mut gpui::App)>;

/// Render every block of a chapter into a flat element list.
///
/// `content_width` is the viewer width in px (window width minus sidebar and arrow gutters); it is
/// used to scale images down to the content column.
pub fn render_blocks(
    model: &crate::epub::model::BlockModel,
    book: &Book,
    chapter_index: usize,
    settings: &Settings,
    content_width: f32,
    cache: &mut ImageCache,
) -> Vec<AnyElement> {
    render_blocks_full(
        model,
        book,
        chapter_index,
        settings,
        content_width,
        cache,
        None,
        None,
    )
}

/// Like [`render_blocks`], but emits a zero-size [`crate::render::anchor::AnchorProbe`] before every
/// block so the reader can measure block positions for in-chapter TOC jumps.
///
/// The returned element list length is `2 * blocks` (probe + block) when probing is enabled. Callers
/// should clear the sink before each render pass so stale positions never leak between chapters.
pub fn render_blocks_with_anchors(
    model: &crate::epub::model::BlockModel,
    book: &Book,
    chapter_index: usize,
    settings: &Settings,
    content_width: f32,
    cache: &mut ImageCache,
    anchor_sink: Option<&crate::render::anchor::AnchorPositions>,
) -> Vec<AnyElement> {
    render_blocks_full(
        model,
        book,
        chapter_index,
        settings,
        content_width,
        cache,
        anchor_sink,
        None,
    )
}

/// Full renderer entry point: anchor probes + interactive inline links.
pub fn render_blocks_full(
    model: &crate::epub::model::BlockModel,
    book: &Book,
    chapter_index: usize,
    settings: &Settings,
    content_width: f32,
    cache: &mut ImageCache,
    anchor_sink: Option<&crate::render::anchor::AnchorPositions>,
    on_link: Option<&LinkHandler>,
) -> Vec<AnyElement> {
    let mut ctx = Ctx {
        book,
        chapter: chapter_index,
        settings,
        theme: settings.theme,
        content_width,
        cache,
        next_id: 0,
        on_link: on_link.cloned(),
    };
    let mut out = Vec::with_capacity(model.blocks.len() * 2);
    for (index, block) in model.blocks.iter().enumerate() {
        if let Some(sink) = anchor_sink {
            out.push(
                crate::render::anchor::AnchorProbe::new(index, sink).into_any_element(),
            );
        }
        out.push(ctx.block(block, index));
    }
    out
}

struct Ctx<'a> {
    book: &'a Book,
    chapter: usize,
    settings: &'a Settings,
    theme: Theme,
    content_width: f32,
    cache: &'a mut ImageCache,
    next_id: usize,
    on_link: Option<LinkHandler>,
}

impl Ctx<'_> {
    fn scale(&self) -> f32 {
        (self.settings.font_size / theme::CONTENT_DEFAULT_FONT_SIZE).clamp(0.4, 3.0)
    }

    /// Base body font size in px for this render pass.
    fn base_font_size(&self) -> f32 {
        self.settings.font_size.clamp(theme::FONT_SIZE_MIN, theme::FONT_SIZE_MAX)
    }

    fn font_size(&self, style: &BlockStyle) -> f32 {
        (style.font_size * self.scale()).clamp(6.0, 96.0)
    }

    fn length(&self, value: f32) -> f32 {
        (value * self.scale()).max(0.0)
    }

    fn font_family(&self, style: &BlockStyle, runs: &[crate::epub::model::Inline]) -> SharedString {
        if text::all_code(runs) {
            theme::mono_font().to_string().into()
        } else {
            theme::font_for(&style.font_family).into()
        }
    }

    fn text_color(&self, style: &BlockStyle) -> Hsla {
        style
            .color
            .map(|color| self.theme.adapt_content_color(color))
            .unwrap_or_else(|| self.theme.content_text())
    }

    fn next_id(&mut self, prefix: &str) -> ElementId {
        let id = format!("{prefix}-{}", self.next_id);
        self.next_id += 1;
        ElementId::Name(id.into())
    }

    fn block(&mut self, block: &Block, index: usize) -> AnyElement {
        match block {
            Block::Heading { level, runs, style } => {
                self.text_block(runs, style, true, *level, index)
            }
            Block::Paragraph { runs, style } => self.text_block(runs, style, false, 0, index),
            Block::List(list) => self.list(list, index),
            Block::Quote { blocks, style } => self.quote(blocks, style, index),
            Block::Container(container) => self.styled_container(container, index),
            Block::Preformatted { text, style } => self.preformatted(text, style, index),
            Block::Image(image) => {
                let element = images::image_element(
                    self.book,
                    self.chapter,
                    image,
                    self.content_width,
                    self.cache,
                    self.theme,
                );
                wrapper_with_spacing(self, &image.style, index).child(element).into_any_element()
            }
            Block::Rule => {
                let margin = self.base_font_size() * 0.5;
                div()
                    .w_full()
                    .h(px(1.0))
                    .my(px(margin))
                    .bg(self.theme.border())
                    .into_any_element()
            }
            Block::Table(table) => self.table(table, index),
        }
    }

    /// Heading / paragraph: one `StyledText` per `<br/>`-separated segment, stacked vertically.
    fn text_block(
        &mut self,
        runs: &[crate::epub::model::Inline],
        style: &BlockStyle,
        bold: bool,
        level: u8,
        index: usize,
    ) -> AnyElement {
        let styled = text::build_styled_runs_with_theme(runs, self.theme);
        let container = wrapper_with_spacing(self, style, index);
        if styled.is_empty() {
            return container.into_any_element();
        }

        let font_size = self.font_size(style);
        let font_family = self.font_family(style, runs);
        let color = self.text_color(style);
        let line_height = if style.line_height.is_finite() && style.line_height > 0.0 {
            style.line_height
        } else {
            1.5
        };
        let align = match style.align {
            TextAlignment::Center => TextAlign::Center,
            TextAlignment::End => TextAlign::Right,
            TextAlignment::Start => TextAlign::Left,
        };

        let mut text_column = div()
            .w_full()
            .flex()
            .flex_col()
            .text_size(px(font_size))
            .font_family(font_family)
            .text_color(color)
            .line_height(relative(line_height))
            .text_align(align);
        if bold {
            text_column = text_column.font_weight(FontWeight::BOLD);
        }
        if style.indent > 0.0 {
            // GPUI has no real text-indent; paragraphs get a first-line indent by prefixing
            // em/en spaces to the first segment below (visual parity with the source CSS).
        }
        if style.padding_left > 0.0 {
            text_column = text_column.pl(px(self.length(style.padding_left)));
        }
        if style.padding_right > 0.0 {
            text_column = text_column.pr(px(self.length(style.padding_right)));
        }
        // Heading level is not needed for styling (font-size/weight already come from the engine's
        // UA cascade) but is kept in the model for future anchor jumps; mark the element id so
        // debug tooling and future in-chapter anchors can find it.
        let _ = level;

        for (segment_index, mut segment) in styled.segments().into_iter().enumerate() {
            if !bold && segment_index == 0 && style.indent > 0.0 {
                self.indent_first_line(&mut segment, font_size, style.indent);
            }
            let child = self.styled_segment(segment);
            text_column = text_column.child(child);
        }
        container.child(text_column).into_any_element()
    }

    /// Approximate CSS `text-indent` by prefixing em/en spaces to the first visual line. Ranges
    /// for highlights/links are shifted so link hit-testing keeps working.
    fn indent_first_line(&self, segment: &mut text::StyledSegment, font_size: f32, indent: f32) {
        let em = (self.length(indent) / font_size.max(1.0)).max(0.0);
        let mut pad = String::new();
        for _ in 0..em.floor() as usize {
            pad.push('\u{2003}'); // em space
        }
        if em.fract() >= 0.4 {
            pad.push('\u{2002}'); // en space (~0.5em)
        }
        if pad.is_empty() {
            return;
        }
        let shift = pad.len();
        segment.text.insert_str(0, &pad);
        for (range, _) in &mut segment.highlights {
            range.start += shift;
            range.end += shift;
        }
        for (range, _) in &mut segment.links {
            range.start += shift;
            range.end += shift;
        }
    }

    /// Styled block-level container (`div` with border/background/padding), preserving the box.
    fn styled_container(
        &mut self,
        container: &crate::epub::model::ContainerBlock,
        index: usize,
    ) -> AnyElement {
        let style = &container.style;
        let mut el = wrapper_with_spacing(self, style, index)
            .w_full()
            .flex()
            .flex_col();
        if let Some(border) = style.border_left {
            el = el
                .border_l(px((border.width * self.scale()).max(1.0)))
                .border_color(self.theme.adapt_border(border.color));
        }
        if let Some(border) = style.border_top {
            el = el
                .border_t(px((border.width * self.scale()).max(1.0)))
                .border_color(self.theme.adapt_border(border.color));
        }
        if let Some(border) = style.border_right {
            el = el
                .border_r(px((border.width * self.scale()).max(1.0)))
                .border_color(self.theme.adapt_border(border.color));
        }
        if let Some(border) = style.border_bottom {
            el = el
                .border_b(px((border.width * self.scale()).max(1.0)))
                .border_color(self.theme.adapt_border(border.color));
        }
        if let Some(background) = style.background {
            el = el.bg(self.theme.adapt_background(background));
        }
        if style.padding_left > 0.0 {
            el = el.pl(px(self.length(style.padding_left)));
        }
        if style.padding_right > 0.0 {
            el = el.pr(px(self.length(style.padding_right)));
        }
        if style.padding_top > 0.0 {
            el = el.pt(px(self.length(style.padding_top)));
        }
        if style.padding_bottom > 0.0 {
            el = el.pb(px(self.length(style.padding_bottom)));
        }
        for (child_index, block) in container.blocks.iter().enumerate() {
            el = el.child(self.block(block, index * 100 + child_index));
        }
        el.into_any_element()
    }

    /// Build one segment's text element; segments containing `<a href>` runs become
    /// `InteractiveText` with byte-range hit-testing so clicks trigger the link handler.
    fn styled_segment(&mut self, segment: text::StyledSegment) -> AnyElement {
        let styled = if segment.highlights.is_empty() {
            StyledText::new(segment.text)
        } else {
            StyledText::new(segment.text).with_highlights(segment.highlights)
        };
        let (Some(handler), false) = (self.on_link.clone(), segment.links.is_empty()) else {
            return styled.into_any_element();
        };
        let ranges: Vec<Range<usize>> = segment.links.iter().map(|(range, _)| range.clone()).collect();
        let hrefs: Vec<String> = segment.links.into_iter().map(|(_, href)| href).collect();
        let id = self.next_id("link-text");
        InteractiveText::new(id, styled)
            .on_click(ranges, move |range_ix, window, cx| {
                if let Some(href) = hrefs.get(range_ix) {
                    handler(href, window, cx);
                }
            })
            .into_any_element()
    }

    fn list(&mut self, list: &ListBlock, index: usize) -> AnyElement {
        let mut container = wrapper_with_spacing(self, &list.style, index)
            .flex()
            .flex_col()
            .w_full();
        if list.style.padding_left > 0.0 {
            container = container.pl(px(self.length(list.style.padding_left)));
        }
        for (item_index, item) in list.items.iter().enumerate() {
            container =
                container.child(self.list_item(list, item, item_index, index));
        }
        container.into_any_element()
    }

    fn list_item(
        &mut self,
        list: &ListBlock,
        item: &ListItem,
        item_index: usize,
        index: usize,
    ) -> AnyElement {
        let font_size = self.font_size(&item.style);
        let marker = if item.marker.trim().is_empty() {
            if list.ordered {
                format!("{}.", list.start + item_index as u32)
            } else {
                "•".to_string()
            }
        } else {
            item.marker.clone()
        };
        let marker_width = (2.0 * font_size).max(16.0);
        let mut marker_el = div()
            .w(px(marker_width))
            .flex_shrink_0()
            .text_right()
            .pr(px(8.0))
            .text_size(px(font_size))
            .text_color(self.text_color(&item.style));
        marker_el = marker_el.child(marker.clone());

        let styled = text::build_styled_runs_with_theme(&item.runs, self.theme);
        let mut content = div().flex_1().flex_col();
        if !styled.is_empty() {
            let mut text_column = div()
                .w_full()
                .flex()
                .flex_col()
                .text_size(px(font_size))
                .font_family(self.font_family(&item.style, &item.runs))
                .text_color(self.text_color(&item.style))
                .line_height(relative(if item.style.line_height.is_finite()
                    && item.style.line_height > 0.0
                {
                    item.style.line_height
                } else {
                    1.5
                }));
            for segment in styled.segments() {
                let child = self.styled_segment(segment);
                text_column = text_column.child(child);
            }
            content = content.child(text_column);
        }
        for (nested_index, nested) in item.blocks.iter().enumerate() {
            content = content.child(self.block(nested, index * 100 + nested_index));
        }

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .child(marker_el)
            .child(content)
            .into_any_element()
    }

    fn quote(&mut self, blocks: &[Block], style: &BlockStyle, index: usize) -> AnyElement {
        let mut container = wrapper_with_spacing(self, style, index).flex().flex_col().w_full();
        if let Some(border) = style.border_left {
            container = container
                .border_l(px((border.width * self.scale()).max(1.0)))
                .border_color(self.theme.adapt_border(border.color));
        } else {
            container = container.border_l_3().border_color(self.theme.border());
        }
        let padding = if style.padding_left > 0.0 {
            self.length(style.padding_left)
        } else {
            self.length(24.0)
        };
        container = container.pl(px(padding));
        if let Some(background) = style.background {
            container = container.bg(self.theme.adapt_background(background));
        }
        for (nested_index, nested) in blocks.iter().enumerate() {
            container = container.child(self.block(nested, index * 100 + nested_index));
        }
        container.into_any_element()
    }

    fn preformatted(&mut self, text: &str, style: &BlockStyle, index: usize) -> AnyElement {
        let font_size = self.font_size(style);
        let line_height = if style.line_height.is_finite() && style.line_height > 0.0 {
            style.line_height
        } else {
            1.4
        };
        let family: SharedString = theme::font_for(&style.font_family).into();
        let color = self.text_color(style);

        let mut lines = div().flex().flex_col();
        for line in text.split('\n') {
            // Keep blank lines and leading whitespace visible.
            let line = if line.is_empty() { " " } else { line };
            lines = lines.child(
                div()
                    .text_size(px(font_size))
                    .font_family(family.clone())
                    .text_color(color)
                    .line_height(relative(line_height))
                    .whitespace_nowrap()
                    .child(line.to_string()),
            );
        }

        wrapper_with_spacing(self, style, index)
            .w_full()
            .child(
                div()
                    .id(self.next_id("pre"))
                    .w_full()
                    .overflow_x_scroll()
                    .pl(px(self.length(style.padding_left)))
                    .pr(px(self.length(style.padding_right)))
                    .py(px(self.length(6.0)))
                    .when(style.background.is_some(), |this| {
                        this.bg(style.background.unwrap_or_else(|| self.theme.content_bg()))
                    })
                    .child(lines),
            )
            .into_any_element()
    }

    fn table(&mut self, table: &TableBlock, index: usize) -> AnyElement {
        let mut container = wrapper_with_spacing(self, &table.style, index)
            .w_full()
            .flex()
            .flex_col()
            .border_1()
            .border_color(self.theme.border());
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut row_el = div()
                .w_full()
                .flex()
                .flex_row()
                .when(row_index > 0, |this| this.border_t_1().border_color(self.theme.border()));
            for (cell_index, cell) in row.cells.iter().enumerate() {
                row_el = row_el.child(self.table_cell(cell, cell_index));
            }
            container = container.child(row_el);
        }
        container.into_any_element()
    }

    fn table_cell(&mut self, cell: &TableCell, cell_index: usize) -> AnyElement {
        let font_size = self.base_font_size();
        let styled = text::build_styled_runs_with_theme(&cell.runs, self.theme);
        let align = match cell.align {
            TextAlignment::Center => TextAlign::Center,
            TextAlignment::End => TextAlign::Right,
            TextAlignment::Start => TextAlign::Left,
        };
        let mut inner = div()
            .w_full()
            .flex()
            .flex_col()
            .text_size(px(font_size))
            .font_family(theme::serif_font().to_string())
            .text_color(self.theme.content_text())
            .text_align(align);
        if cell.header {
            inner = inner.font_weight(FontWeight::BOLD);
        }
        for segment in styled.segments() {
            let child: AnyElement = if segment.highlights.is_empty() {
                StyledText::new(segment.text).into_any_element()
            } else {
                StyledText::new(segment.text)
                    .with_highlights(segment.highlights)
                    .into_any_element()
            };
            inner = inner.child(child);
        }
        div()
            .flex_1()
            .min_w(px(16.0))
            .p(px(6.0))
            .when(cell_index > 0, |this| this.border_l_1().border_color(self.theme.border()))
            .child(inner)
            .into_any_element()
    }
}

/// Outer wrapper carrying CSS margins, background and text color for a block.
fn wrapper_with_spacing(ctx: &Ctx<'_>, style: &BlockStyle, index: usize) -> Stateful<Div> {
    let mut wrapper = div().id(ElementId::Name(format!("block-{index}").into())).w_full();
    if style.space_before > 0.0 {
        wrapper = wrapper.mt(px(ctx.length(style.space_before)));
    }
    if style.space_after > 0.0 {
        wrapper = wrapper.mb(px(ctx.length(style.space_after)));
    }
    if let Some(color) = style.color {
        wrapper = wrapper.text_color(color);
    } else {
        wrapper = wrapper.text_color(ctx.theme.content_text());
    }
    if let Some(background) = style.background {
        wrapper = wrapper.bg(background);
    }
    wrapper
}

