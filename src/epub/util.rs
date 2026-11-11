pub fn strip_fragment(href: &str) -> &str {
    href.split('#').next().unwrap_or(href)
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn normalize_package_path(base_dir: &str, href: &str) -> String {
    let href = percent_decode(strip_fragment(href));
    let mut parts: Vec<&str> = if href.starts_with('/') {
        Vec::new()
    } else {
        base_dir
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect()
    };
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Decodes EPUB text resources to a `String` without ever failing.
///
/// EPUB 3 requires UTF-8, but real-world EPUB 2 books routinely ship legacy encodings.
/// Order of attempts:
/// 1. Byte-order marks (UTF-8, UTF-16 LE/BE).
/// 2. Strict UTF-8.
/// 3. The encoding declared in the XML prolog (`encoding="..."`) or an HTML
///    `<meta charset>`/`<meta http-equiv>` tag.
/// 4. Windows-1252 as the last resort (a superset of ISO-8859-1 and the historical
///    web default), decoded lossily so malformed bytes cannot panic.
pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return decode_utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return decode_utf16(rest, false);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let head = &bytes[..bytes.len().min(4096)];
    if let Some(label) = sniff_declared_encoding(head)
        && let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes())
    {
        let (text, _, _) = encoding.decode(bytes);
        return text.into_owned();
    }
    // NOTE: `decode` never fails and replaces invalid sequences; `WINDOWS_1252` maps
    // every byte to a character, which keeps legacy books readable.
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    text.into_owned()
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Best-effort extraction of a declared character encoding from the head of a document.
fn sniff_declared_encoding(head: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(head);
    let lower = text.to_ascii_lowercase();
    for marker in ["charset=", "encoding="] {
        let mut search_from = 0;
        while let Some(index) = lower[search_from..].find(marker) {
            let start = search_from + index + marker.len();
            let label: String = text[start..]
                .trim_start()
                .trim_start_matches(['"', '\''])
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                .collect();
            if label.len() >= 2 && !label.eq_ignore_ascii_case("utf-8") {
                return Some(label);
            }
            search_from = start;
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    pub family: String,
    pub url: String,
}

/// Extracts `@font-face { font-family: …; src: url(…) }` declarations from a stylesheet.
///
/// Only the first `url(...)` is used (EPUB e-ink stylesheets commonly list several fallbacks) and
/// families are unquoted/first-of-list. Malformed blocks are skipped rather than failing the book;
/// this is deliberately a small tolerant scanner, not a full CSS parser.
pub fn parse_font_faces(css: &str) -> Vec<FontFace> {
    let mut faces = Vec::new();
    let mut rest = css;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find("@font-face") else {
            break;
        };
        rest = &rest[start + "@font-face".len()..];
        let Some(open) = rest.find('{') else {
            break;
        };
        let Some(close) = rest[open + 1..].find('}') else {
            break;
        };
        let block = &rest[open + 1..open + 1 + close];
        rest = &rest[open + 1 + close + 1..];
        let Some(family) = css_property(block, "font-family") else {
            continue;
        };
        let Some(src) = css_property(block, "src") else {
            continue;
        };
        let Some(url) = first_url(src) else {
            continue;
        };
        let family = first_family(family);
        if !family.is_empty() && !url.is_empty() {
            faces.push(FontFace { family, url });
        }
    }
    faces
}

fn css_property<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    for declaration in block.split(';') {
        let Some((key, value)) = declaration.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(value.trim());
        }
    }
    None
}

fn first_family(value: &str) -> String {
    let family = value.split(',').next().unwrap_or(value).trim();
    family
        .trim_matches(['"', '\''])
        .trim()
        .to_string()
}

fn first_url(src: &str) -> Option<String> {
    let lower = src.to_ascii_lowercase();
    let start = lower.find("url(")? + 4;
    let end = src[start..].find(')')? + start;
    let url = src[start..end].trim();
    Some(url.trim_matches(['"', '\'']).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_utf8() {
        assert_eq!(decode_text("Tiếng Việt — ắ ằ".as_bytes()), "Tiếng Việt — ắ ằ");
    }

    #[test]
    fn parses_font_faces_from_stylesheet() {
        let css = r#"
            @font-face { font-family: "Literata"; src: url("fonts/Literata-Regular.ttf"); font-weight: 400; }
            @font-face { font-family: 'Be Vietnam Pro'; src: url(fonts/BeVietnamPro-SemiBold.ttf) format("truetype"); }
            @font-face { font-family: Broken; }
        "#;
        let faces = parse_font_faces(css);
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].family, "Literata");
        assert_eq!(faces[0].url, "fonts/Literata-Regular.ttf");
        assert_eq!(faces[1].family, "Be Vietnam Pro");
        assert_eq!(faces[1].url, "fonts/BeVietnamPro-SemiBold.ttf");
    }

    #[test]
    fn font_face_parser_is_tolerant_and_case_insensitive() {
        let css = "@FONT-FACE { FONT-FAMILY: Arial, sans-serif; SRC: URL(x.otf) }";
        let faces = parse_font_faces(css);
        assert_eq!(
            faces,
            vec![FontFace {
                family: "Arial".to_string(),
                url: "x.otf".to_string()
            }]
        );
        assert!(parse_font_faces("@font-face { font-family: X }").is_empty());
        assert!(parse_font_faces("body{}").is_empty());
    }

    #[test]
    fn decodes_windows_1252_without_declaration() {
        assert_eq!(decode_text(b"caf\xE9 na\xEFve"), "café naïve");
    }

    #[test]
    fn decodes_declared_charset() {
        let bytes = b"<meta charset=\"windows-1252\"><p>caf\xE9</p>";
        assert!(decode_text(bytes).contains("café"));
    }

    #[test]
    fn decodes_utf8_bom() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhello"), "hello");
    }

    #[test]
    fn decodes_utf16le_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "hi".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_text(&bytes), "hi");
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(decode_text(b""), "");
    }
}
