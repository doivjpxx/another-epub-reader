//! Task 10 — inline-run mapping + block-element building tests.
//!
//! The element builders only construct GPUI elements (no window/app context is required for
//! `div()`/`StyledText`/`img()` construction), so they are exercised against a real fixture EPUB.

use epub_reader::epub::Book;
use epub_reader::epub::model::{BlockStyle, ImageBlock, Inline, InlineStyle};
use epub_reader::render::ImageCache;
use epub_reader::render::anchor_positions;
use epub_reader::render::images::image_element;
use epub_reader::render::{render_blocks, render_blocks_with_anchors};
use epub_reader::render::text::{
    all_code, build_styled_runs, build_styled_runs_with_theme, highlight_for, split_breaks,
};
use epub_reader::theme::{Settings, Theme};

#[test]
fn builds_contiguous_highlight_ranges() {
    let bold = InlineStyle {
        bold: true,
        ..Default::default()
    };
    let plain = InlineStyle::default();
    let runs = vec![
        Inline::Text {
            text: "a ".into(),
            style: plain,
        },
        Inline::Text {
            text: "bold".into(),
            style: bold,
        },
        Inline::Break,
        Inline::Text {
            text: " end".into(),
            style: plain,
        },
    ];
    let p = build_styled_runs(&runs);
    assert_eq!(p.text, "a bold\n end");
    assert_eq!(p.highlights.len(), 1);
    assert_eq!(p.highlights[0].0, 2..6);
    assert_eq!(p.breaks.len(), 1);
    assert_eq!(p.breaks[0], 6);
}

#[test]
fn merges_adjacent_identical_styles() {
    let style = InlineStyle {
        italic: true,
        ..Default::default()
    };
    let runs = vec![
        Inline::Text {
            text: "one ".into(),
            style,
        },
        Inline::Text {
            text: "two".into(),
            style,
        },
    ];
    let p = build_styled_runs(&runs);
    assert_eq!(p.text, "one two");
    assert_eq!(p.highlights.len(), 1);
    assert_eq!(p.highlights[0].0, 0..7);
    assert!(p.breaks.is_empty());
}

#[test]
fn highlights_map_to_theme_colors() {
    let link = InlineStyle {
        link: true,
        ..Default::default()
    };
    let h = highlight_for(link, Theme::Night);
    assert!(h.color.is_some());
    assert_eq!(h.color, Some(Theme::Night.link_color()));

    let bold = highlight_for(
        InlineStyle {
            bold: true,
            ..Default::default()
        },
        Theme::White,
    );
    assert!(bold.font_weight.is_some());

    // Themed variant re-maps link colors instead of baking in the white-theme color.
    let runs = vec![Inline::Text {
        text: "link".into(),
        style: link,
    }];
    let night = build_styled_runs_with_theme(&runs, Theme::Night);
    assert_eq!(night.highlights.len(), 1);
    assert_eq!(night.highlights[0].1.color, Some(Theme::Night.link_color()));
}

#[test]
fn breaks_split_into_stacked_segments() {
    let bold = InlineStyle {
        bold: true,
        ..Default::default()
    };
    let plain = InlineStyle::default();
    let runs = vec![
        Inline::Text {
            text: "first".into(),
            style: bold,
        },
        Inline::Break,
        Inline::Text {
            text: "second".into(),
            style: plain,
        },
    ];
    let p = build_styled_runs_with_theme(&runs, Theme::White);
    assert_eq!(p.text, "first\nsecond");
    assert_eq!(p.segments().len(), 2);
    let segments = p.segments();
    assert_eq!(segments[0].text, "first");
    assert_eq!(segments[1].text, "second");
    // Highlights are re-based onto each segment.
    assert_eq!(segments[0].highlights.len(), 1);
    assert_eq!(segments[0].highlights[0].0, 0..5);
    assert!(segments[1].highlights.is_empty());
}

#[test]
fn consecutive_breaks_keep_empty_segments() {
    let runs = vec![
        Inline::Text {
            text: "a".into(),
            style: InlineStyle::default(),
        },
        Inline::Break,
        Inline::Break,
        Inline::Text {
            text: "b".into(),
            style: InlineStyle::default(),
        },
    ];
    let p = build_styled_runs(&runs);
    assert_eq!(p.text, "a\n\nb");
    assert_eq!(p.breaks, vec![1, 2]);
    let segments = p.segments();
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].text, "a");
    assert_eq!(segments[1].text, "");
    assert_eq!(segments[2].text, "b");
    assert_eq!(
        split_breaks("a\n\nb", &[1, 2]),
        vec!["a".to_string(), String::new(), "b".to_string()]
    );
}

#[test]
fn code_detection_ignores_whitespace_runs() {
    let code = InlineStyle {
        code: true,
        ..Default::default()
    };
    let runs = vec![
        Inline::Text {
            text: "let ".into(),
            style: code,
        },
        Inline::Text {
            text: "x".into(),
            style: code,
        },
    ];
    assert!(all_code(&runs));

    let mixed = vec![
        Inline::Text {
            text: "let ".into(),
            style: code,
        },
        Inline::Text {
            text: "x".into(),
            style: InlineStyle::default(),
        },
    ];
    assert!(!all_code(&mixed));
}

#[test]
fn renders_styled_fixture_into_elements() {
    let book = Book::open(std::path::Path::new("tests/fixtures/styled.epub")).expect("fixture opens");
    let model = book.chapter(0).expect("chapter 0 parses");
    assert!(!model.blocks.is_empty(), "fixture chapter has blocks");

    let mut cache = ImageCache::new();
    let elements = render_blocks(
        &model,
        &book,
        0,
        &Settings::default(),
        800.0,
        &mut cache,
    );
    assert_eq!(elements.len(), model.blocks.len());
    assert!(
        !cache.is_empty(),
        "styled fixture contains an image and it should be cached"
    );
}

#[test]
fn missing_image_degrades_to_placeholder_element() {
    let book = Book::open(std::path::Path::new("tests/fixtures/styled.epub")).expect("fixture opens");
    let block = ImageBlock {
        src: "does/not/exist.png".into(),
        alt: "missing".into(),
        width: None,
        height: None,
        style: BlockStyle::default(),
    };
    let mut cache = ImageCache::new();
    let element = image_element(&book, 0, &block, 600.0, &mut cache, Theme::Night);
    // A failed load degrades to a placeholder element and is remembered (never a panic).
    drop(element);
    assert_eq!(cache.len(), 1);
    // Second call is served from the negative cache entry.
    let again = image_element(&book, 0, &block, 600.0, &mut cache, Theme::Night);
    drop(again);
    assert_eq!(cache.len(), 1);
}

#[test]
fn probed_render_emits_one_probe_per_block() {
    let book = Book::open(std::path::Path::new("tests/fixtures/styled.epub")).expect("fixture opens");
    let model = book.chapter(0).expect("chapter 0 parses");
    let mut cache = ImageCache::new();
    let sink = anchor_positions();
    let elements = render_blocks_with_anchors(
        &model,
        &book,
        0,
        &Settings::default(),
        800.0,
        &mut cache,
        Some(&sink),
    );
    // One zero-size probe + one block element per block.
    assert_eq!(elements.len(), model.blocks.len() * 2);
    // Element construction does not lay out, so positions stay empty until prepaint runs.
    assert!(sink.borrow().is_empty());
}

#[test]
fn link_runs_become_clickable_ranges_and_rebase_in_segments() {
    let plain = InlineStyle::default();
    let link_style = InlineStyle {
        link: true,
        ..Default::default()
    };
    let runs = vec![
        Inline::Text {
            text: "Go ".into(),
            style: plain,
        },
        Inline::Link {
            text: "there".into(),
            style: link_style,
            href: "#target".into(),
        },
        Inline::Text {
            text: " now".into(),
            style: plain,
        },
        Inline::Break,
        Inline::Link {
            text: "next".into(),
            style: link_style,
            href: "ch2.xhtml#t2".into(),
        },
    ];
    let styled = build_styled_runs(&runs);
    assert_eq!(
        styled.links,
        vec![
            (3..8, "#target".to_string()),
            (13..17, "ch2.xhtml#t2".to_string())
        ]
    );
    let segments = styled.segments();
    assert_eq!(segments[0].links, vec![(3..8, "#target".to_string())]);
    assert_eq!(segments[1].links, vec![(0..4, "ch2.xhtml#t2".to_string())]);
    assert!(build_styled_runs(&[]).links.is_empty());
}

