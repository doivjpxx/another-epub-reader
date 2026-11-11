use crate::theme::{clamp_font_size, Theme};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenHint {
    Landing,
    Reader,
}

#[derive(Clone, Debug, Default)]
pub struct CliArgs {
    pub open: Option<PathBuf>,
    pub theme: Option<Theme>,
    pub font_size: Option<f32>,
    pub screen: Option<ScreenHint>,
}

pub fn parse_args(args: &[String]) -> CliArgs {
    let mut cli = CliArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--open" if i + 1 < args.len() => {
                cli.open = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--theme" if i + 1 < args.len() => {
                cli.theme = match args[i + 1].as_str() {
                    "0" => Some(Theme::White),
                    "1" => Some(Theme::Sepia),
                    "2" => Some(Theme::Night),
                    _ => None,
                };
                i += 2;
            }
            "--font-size" if i + 1 < args.len() => {
                cli.font_size = args[i + 1].parse::<f32>().ok().map(clamp_font_size);
                i += 2;
            }
            "--screen" if i + 1 < args.len() => {
                cli.screen = match args[i + 1].as_str() {
                    "reader" => Some(ScreenHint::Reader),
                    "landing" => Some(ScreenHint::Landing),
                    _ => None,
                };
                i += 2;
            }
            _ => i += 1,
        }
    }
    cli
}
