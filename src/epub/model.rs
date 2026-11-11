use gpui::Hsla;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub link: bool,
    /// CSS `text-transform: uppercase` (rendered uppercase; the stored text keeps its case).
    pub uppercase: bool,
    /// CSS `letter-spacing` in px (0 = normal); rendered with thin spaces between letters.
    pub letter_spacing: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text {
        text: String,
        style: InlineStyle,
    },
    /// Inline text wrapped in an `<a href="...">`: clicking it navigates to the target.
    Link {
        text: String,
        style: InlineStyle,
        href: String,
    },
    Break,
}

impl Inline {
    pub fn text(&self) -> Option<(&str, InlineStyle)> {
        match self {
            Inline::Text { text, style } | Inline::Link { text, style, .. } => {
                Some((text.as_str(), *style))
            }
            Inline::Break => None,
        }
    }

    /// The raw `href` when this run is a hyperlink.
    pub fn href(&self) -> Option<&str> {
        match self {
            Inline::Link { href, .. } => Some(href.as_str()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Serif,
    SansSerif,
    Monospace,
    Named(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockStyle {
    pub font_size: f32,
    pub font_family: FontFamily,
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
    pub align: TextAlignment,
    pub indent: f32,
    pub space_before: f32,
    pub space_after: f32,
    pub line_height: f32,
    pub padding_left: f32,
    pub padding_right: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    /// Percentage horizontal paddings (resolved by the renderer relative to the available width,
    /// e.g. `body { margin: 0 5% }`).
    pub padding_left_percent: f32,
    pub padding_right_percent: f32,
    pub border_left: Option<Border>,
    pub border_top: Option<Border>,
    pub border_right: Option<Border>,
    pub border_bottom: Option<Border>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Hsla,
}

impl Default for BlockStyle {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            font_family: FontFamily::Serif,
            color: None,
            background: None,
            align: TextAlignment::Start,
            indent: 0.0,
            space_before: 0.0,
            space_after: 0.0,
            line_height: 1.5,
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left_percent: 0.0,
            padding_right_percent: 0.0,
            border_left: None,
            border_top: None,
            border_right: None,
            border_bottom: None,
        }
    }
}

/// A styled block-level container (`div`/`section`/`aside` with border/background/padding),
/// preserving the box around its child blocks (e.g. EPUB callouts and per-entry rules).
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerBlock {
    pub blocks: Vec<Block>,
    pub style: BlockStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        runs: Vec<Inline>,
        style: BlockStyle,
    },
    Paragraph {
        runs: Vec<Inline>,
        style: BlockStyle,
    },
    List(ListBlock),
    Quote {
        blocks: Vec<Block>,
        style: BlockStyle,
    },
    Container(ContainerBlock),
    Preformatted {
        text: String,
        style: BlockStyle,
    },
    Image(ImageBlock),
    Rule,
    Table(TableBlock),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListBlock {
    pub ordered: bool,
    pub start: u32,
    pub items: Vec<ListItem>,
    pub style: BlockStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub runs: Vec<Inline>,
    pub blocks: Vec<Block>,
    pub marker: String,
    pub style: BlockStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageBlock {
    pub src: String,
    pub alt: String,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub style: BlockStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableBlock {
    pub rows: Vec<TableRow>,
    pub style: BlockStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableCell {
    pub runs: Vec<Inline>,
    pub header: bool,
    pub align: TextAlignment,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BlockModel {
    pub blocks: Vec<Block>,
    /// Fragment anchors (`element id` / `<a name>`) mapped to the index of the block they belong
    /// to. Populated by the XHTML walker and used by the reader for in-chapter TOC jumps.
    pub anchors: std::collections::HashMap<String, usize>,
    /// Computed `<body>` style; the reader applies its horizontal padding/margins around the
    /// content column (e.g. `body { margin: 0 5% }`).
    pub body_style: BlockStyle,
}

impl BlockModel {
    /// Resolve a TOC fragment (`#section-2` or `section-2`) to a block index.
    pub fn anchor_block(&self, anchor: &str) -> Option<usize> {
        let key = anchor.strip_prefix('#').unwrap_or(anchor);
        self.anchors.get(key).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub chapter_index: usize,
    pub scroll_ratio: f32,
}

impl Position {
    pub fn clamped(mut self, chapter_count: usize, ratio: f32) -> Self {
        if chapter_count == 0 {
            return Position::default();
        }
        self.chapter_index = self.chapter_index.min(chapter_count - 1);
        let ratio = if ratio.is_finite() {
            ratio.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Normalize `-0.0` so persisted state stays canonical.
        self.scroll_ratio = if ratio == 0.0 { 0.0 } else { ratio };
        self
    }
}




