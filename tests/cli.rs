use epub_reader::cli::{ScreenHint, parse_args};
use epub_reader::theme::Theme;
use std::path::PathBuf;

#[test]
fn parses_all_flags() {
    let args: Vec<String> = [
        "epub-reader",
        "--open",
        r"C:\books\a.epub",
        "--theme",
        "2",
        "--font-size",
        "26",
        "--screen",
        "reader",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let cli = parse_args(&args);
    assert_eq!(cli.open, Some(PathBuf::from(r"C:\books\a.epub")));
    assert_eq!(cli.theme, Some(Theme::Night));
    assert_eq!(cli.font_size, Some(26.0));
    assert_eq!(cli.screen, Some(ScreenHint::Reader));
}

#[test]
fn ignores_unknown_and_invalid_values() {
    let args: Vec<String> = ["epub-reader", "--nope", "--theme", "9", "--font-size", "99"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let cli = parse_args(&args);
    assert_eq!(cli.theme, None);
    assert_eq!(cli.font_size, Some(30.0));
    assert_eq!(cli.screen, None);
    assert_eq!(cli.open, None);
}
