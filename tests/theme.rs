use epub_reader::theme::{CONTENT_DEFAULT_FONT_SIZE, Theme, clamp_font_size};

#[test]
fn theme_tokens_match_source_css() {
    assert_eq!(Theme::White.content_bg(), gpui::rgb(0xffffff).into());
    assert_eq!(Theme::White.content_text(), gpui::rgb(0x000000).into());
    assert_eq!(Theme::Sepia.content_bg(), gpui::rgb(0xf4eacd).into());
    assert_eq!(Theme::Sepia.content_text(), gpui::rgb(0x704214).into());
    assert_eq!(Theme::Night.content_bg(), gpui::rgb(0x1b1f2a).into());
    assert_eq!(Theme::Night.content_text(), gpui::rgb(0xbdcadb).into());
    assert_eq!(Theme::Sepia.sidebar_bg(), gpui::rgb(0x111111).into());
    assert_eq!(Theme::Night.sidebar_bg(), gpui::rgb(0x2c3142).into());
    assert_eq!(Theme::Night.sidebar_hover_bg(), Some(gpui::rgb(0x363d52).into()));
    assert_eq!(Theme::Night.gear_icon(), gpui::rgb(0xaab4cc).into());
    assert_eq!(Theme::White.arrow_icon(), gpui::rgb(0xc0c0c0).into());
    assert_eq!(Theme::White.selected_toc(), gpui::rgb(0x346cb9).into());
    assert_eq!(Theme::Sepia.selected_toc(), gpui::rgb(0xe0b06a).into());
    assert_eq!(Theme::Night.selected_toc(), gpui::rgb(0x8ab4f8).into());
}

#[test]
fn font_size_steps_and_clamps() {
    assert_eq!(clamp_font_size(20.0), 20.0);
    assert_eq!(clamp_font_size(31.0), 30.0);
    assert_eq!(clamp_font_size(13.0), 14.0);
    assert_eq!(clamp_font_size(19.0), 20.0);
    assert_eq!(clamp_font_size(21.0), 22.0);
    assert_eq!(CONTENT_DEFAULT_FONT_SIZE, 20.0);
}

#[test]
fn theme_cycle() {
    assert_eq!(Theme::White.next(), Theme::Sepia);
    assert_eq!(Theme::Sepia.next(), Theme::Night);
    assert_eq!(Theme::Night.next(), Theme::White);
}

#[test]
fn book_colors_are_adapted_for_readability() {
    // EPUBs hard-code black text / white callout backgrounds; these must not punch through the
    // Night/Sepia themes.
    let black: gpui::Hsla = gpui::rgb(0x000000).into();
    let white: gpui::Hsla = gpui::rgb(0xffffff).into();
    assert_eq!(Theme::Night.adapt_content_color(black), Theme::Night.content_text());
    assert_eq!(Theme::Sepia.adapt_content_color(black), Theme::Sepia.content_text());
    assert_eq!(Theme::White.adapt_content_color(black), black);
    assert!(Theme::Night.adapt_background(white).l < 0.3);
    assert!(Theme::Sepia.adapt_background(white).l > 0.8);
    assert_eq!(Theme::White.adapt_background(white), white);
    assert_ne!(Theme::Night.adapt_border(black), black);
}

