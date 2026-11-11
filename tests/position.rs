use epub_reader::epub::model::Position;
use epub_reader::ui::reader::{clamp_sidebar_fraction, offset_from_ratio, ratio_from_offset};

#[test]
fn ratio_math_is_clamped_and_safe() {
    assert_eq!(ratio_from_offset(0.0, 0.0), 0.0);
    assert_eq!(ratio_from_offset(50.0, 200.0), 0.25);
    assert_eq!(ratio_from_offset(-10.0, 200.0), 0.0);
    assert_eq!(ratio_from_offset(999.0, 200.0), 1.0);
    assert_eq!(offset_from_ratio(0.25, 200.0), 50.0);
    assert_eq!(offset_from_ratio(2.0, 200.0), 200.0);
    assert_eq!(offset_from_ratio(-1.0, 200.0), 0.0);
}

#[test]
fn position_clamp_respects_chapter_count() {
    let p = Position {
        chapter_index: 99,
        scroll_ratio: 1.7,
    }
    .clamped(5, 1.7);
    assert_eq!(p.chapter_index, 4);
    assert_eq!(p.scroll_ratio, 1.0);

    let empty = Position {
        chapter_index: 3,
        scroll_ratio: 0.5,
    }
    .clamped(0, 0.5);
    assert_eq!(empty, Position::default());
}

#[test]
fn sidebar_fraction_clamps_to_readable_bounds() {
    // 1280 px window: min 160 px, max 45% (576 px).
    assert!((clamp_sidebar_fraction(1280.0, 0.20) - 0.20).abs() < 1e-6);
    assert!((clamp_sidebar_fraction(1280.0, 0.0) - 160.0 / 1280.0).abs() < 1e-6);
    assert!((clamp_sidebar_fraction(1280.0, 1.0) - 0.45).abs() < 1e-6);

    // 900 px minimum window: the reading column keeps >= 360 px, so max is ~37.8% (340 px).
    let fraction = clamp_sidebar_fraction(900.0, 0.9);
    assert!((fraction - 340.0 / 900.0).abs() < 1e-6);
    let reading_column = 900.0 - fraction * 900.0 - 2.0 * 100.0;
    assert!(reading_column >= 360.0 - 1e-3);

    // Non-finite input falls back to the default.
    assert!((clamp_sidebar_fraction(1280.0, f32::NAN) - 0.20).abs() < 1e-6);
    // Degenerate window widths never produce NaN/panic.
    assert!(clamp_sidebar_fraction(0.0, 0.2).is_finite());
}
