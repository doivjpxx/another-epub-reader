//! EPUB image resources → `gpui::Image` elements.
//!
//! Bytes are fetched from the engine (`Book::image_bytes`) and cached per (chapter, src). The
//! format is detected from magic bytes; natural pixel dimensions come from decoding the header
//! with the `image` crate so block layout can size images before GPUI paints them.

use crate::epub::Book;
use crate::epub::model::ImageBlock;
use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{AnyElement, Image as GpuiImage, ImageFormat, div, px};
use std::collections::HashMap;
use std::sync::Arc;

/// A decoded image plus the metadata the block renderer needs for layout.
#[derive(Clone, Debug)]
pub struct LoadedImage {
    pub image: Arc<GpuiImage>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}

impl LoadedImage {
    pub fn aspect_ratio(&self) -> f32 {
        if self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

/// Per-chapter cache of decoded images. `None` values remember failed loads so a broken image is
/// only fetched once per chapter render.
#[derive(Default)]
pub struct ImageCache {
    cache: HashMap<(usize, String), Option<LoadedImage>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all remembered images (used when switching chapters or books).
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn get_or_load(
        &mut self,
        book: &Book,
        chapter: usize,
        src: &str,
    ) -> Option<LoadedImage> {
        let key = (chapter, src.to_string());
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let loaded = load_from_book(book, chapter, src);
        self.cache.insert(key, loaded.clone());
        loaded
    }
}

fn load_from_book(book: &Book, chapter: usize, src: &str) -> Option<LoadedImage> {
    let bytes = book.image_bytes(chapter, src).ok()?;
    if bytes.is_empty() {
        return None;
    }
    load_from_bytes(bytes)
}

/// Build a [`LoadedImage`] from raw resource bytes (also used by tests).
pub fn load_from_bytes(bytes: Vec<u8>) -> Option<LoadedImage> {
    let format = detect_format(&bytes);
    let (width, height) = natural_size(&bytes).unwrap_or((0, 0));
    let image = Arc::new(GpuiImage::from_bytes(format, bytes));
    Some(LoadedImage {
        image,
        width,
        height,
        format,
    })
}

/// Detect the image format from its magic bytes, defaulting to PNG (matching GPUI's convention).
pub fn detect_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ImageFormat::Gif
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else if bytes.starts_with(b"BM") {
        ImageFormat::Bmp
    } else if looks_like_svg(bytes) {
        ImageFormat::Svg
    } else {
        ImageFormat::Png
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head).trim_start().to_ascii_lowercase();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

/// Natural pixel size from the image header. `None` for vector/unsupported payloads.
pub fn natural_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| (image.width(), image.height()))
}

/// Render one [`ImageBlock`].
///
/// Sizing: explicit attribute/CSS width/height when present, otherwise natural size scaled down to
/// [`max_width`] (CSS `max-width: 100%`); unresolvable images degrade to a bordered placeholder
/// carrying the alt text.
pub fn image_element(
    book: &Book,
    chapter: usize,
    block: &ImageBlock,
    max_width: f32,
    cache: &mut ImageCache,
    theme: Theme,
) -> AnyElement {
    let max_width = if max_width.is_finite() && max_width > 1.0 {
        max_width
    } else {
        640.0
    };

    match cache.get_or_load(book, chapter, &block.src) {
        Some(loaded) => {
            let (width, height) = fit_size(block, &loaded, max_width);
            div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(4.0))
                .child(
                    gpui::img(loaded.image.clone())
                        .w(px(width))
                        .h(px(height)),
                )
                .into_any_element()
        }
        None => placeholder(block, max_width, theme),
    }
}

/// Compute the painted pixel size of an image: explicit dimensions win, then the natural aspect
/// ratio is preserved and the result is clamped to the content width.
fn fit_size(block: &ImageBlock, loaded: &LoadedImage, max_width: f32) -> (f32, f32) {
    let natural_w = if loaded.width > 0 {
        loaded.width as f32
    } else {
        max_width
    };
    let natural_h = if loaded.height > 0 {
        loaded.height as f32
    } else {
        natural_w
    };
    let aspect = (natural_w / natural_h).clamp(0.01, 100.0);

    let explicit_w = block.width.filter(|w| w.is_finite() && *w > 1.0);
    let explicit_h = block.height.filter(|h| h.is_finite() && *h > 1.0);

    let (mut width, mut height) = match (explicit_w, explicit_h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w / aspect),
        (None, Some(h)) => (h * aspect, h),
        (None, None) => (natural_w, natural_h),
    };

    if width > max_width {
        let scale = max_width / width;
        width *= scale;
        height *= scale;
    }
    (width.max(1.0), height.max(1.0))
}

fn placeholder(block: &ImageBlock, max_width: f32, theme: Theme) -> AnyElement {
    let width = block
        .width
        .filter(|w| w.is_finite() && *w > 1.0)
        .unwrap_or(max_width.min(320.0))
        .min(max_width);
    let height = block
        .height
        .filter(|h| h.is_finite() && *h > 1.0)
        .unwrap_or(120.0);
    let label: String = if block.alt.trim().is_empty() {
        "Image unavailable".to_string()
    } else {
        block.alt.trim().to_string()
    };
    div()
        .w_full()
        .flex()
        .justify_center()
        .py(px(4.0))
        .child(
            div()
                .w(px(width))
                .h(px(height))
                .border_1()
                .border_color(theme.border())
                .bg(theme.content_bg())
                .flex()
                .items_center()
                .justify_center()
                .p(px(6.0))
                .text_size(px(12.0))
                .text_color(theme.content_text().opacity(0.55))
                .child(label),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats_from_magic_bytes() {
        assert_eq!(detect_format(b"\x89PNG\r\n\x1a\nrest"), ImageFormat::Png);
        assert_eq!(detect_format(&[0xff, 0xd8, 0xff, 0xe0]), ImageFormat::Jpeg);
        assert_eq!(detect_format(b"GIF89a...."), ImageFormat::Gif);
        assert_eq!(detect_format(b"RIFF1234WEBPxxxx"), ImageFormat::Webp);
        assert_eq!(detect_format(b"BM...."), ImageFormat::Bmp);
        assert_eq!(detect_format(b"<svg xmlns=\"...\"/>"), ImageFormat::Svg);
        // Unknown payloads default to PNG (GPUI's convention) and fail to decode to a placeholder.
        assert_eq!(detect_format(b"not an image"), ImageFormat::Png);
    }

    #[test]
    fn fits_images_to_content_width() {
        let loaded = LoadedImage {
            image: Arc::new(GpuiImage::empty()),
            width: 2000,
            height: 1000,
            format: ImageFormat::Png,
        };
        let block = ImageBlock {
            src: "a.png".into(),
            alt: String::new(),
            width: None,
            height: None,
            style: crate::epub::model::BlockStyle::default(),
        };
        let (w, h) = fit_size(&block, &loaded, 800.0);
        assert!((w - 800.0).abs() < 0.01);
        assert!((h - 400.0).abs() < 0.01);

        let small = ImageBlock {
            width: Some(120.0),
            height: None,
            ..block
        };
        let (w, h) = fit_size(&small, &loaded, 800.0);
        assert!((w - 120.0).abs() < 0.01);
        assert!((h - 60.0).abs() < 0.01);
    }
}
