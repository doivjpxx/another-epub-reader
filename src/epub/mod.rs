//! EPUB loading and resource access, backed by the `rbook` crate.
//!
//! The public surface of this module is frozen for the UI/renderer lanes:
//! [`Book::open`], [`Book::metadata`], [`Book::spine`], [`Book::toc`],
//! [`Book::chapter_count`], [`Book::spine_href`], [`Book::cover_png`],
//! [`Book::chapter`], [`Book::image_bytes`], [`Book::resource_str`] and
//! [`derive_toc_from_spine`].

pub mod css;
pub mod model;
pub mod util;
pub mod xhtml;

pub use model::*;

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct BookMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SpineItem {
    /// Package-relative, percent-decoded, normalized path (`"Text/chapter1.xhtml"`).
    pub href: String,
    pub title_hint: String,
}

#[derive(Clone, Debug)]
pub struct TocItem {
    pub label: String,
    pub depth: usize,
    pub chapter_index: Option<usize>,
    /// Fragment inside the target document (`#section-2` → `section-2`), used for in-chapter
    /// jumps. `None` when the entry points at the chapter start or has no fragment.
    pub anchor: Option<String>,
}

/// An opened EPUB publication.
///
/// `Book` is `Send + Sync`; the underlying `rbook::Epub` supports random resource access and the
/// parsed chapter cache is behind a `Mutex`, so chapters can be parsed off the UI thread.
pub struct Book {    epub: rbook::Epub,
    metadata: BookMetadata,
    spine: Vec<SpineItem>,
    toc: Vec<TocItem>,
    cover_bytes: Option<Vec<u8>>,
    chapters: Mutex<HashMap<usize, Arc<BlockModel>>>,
}

impl std::fmt::Debug for Book {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Book")
            .field("title", &self.metadata.title)
            .field("authors", &self.metadata.authors)
            .field("chapters", &self.spine.len())
            .field("toc_entries", &self.toc.len())
            .finish()
    }
}

impl Book {
    /// Opens an EPUB file (or an unzipped EPUB directory, which `rbook` also accepts).
    ///
    /// # Errors
    /// Returns an error — never panics — for missing files, malformed ZIP/OPF content, EPUBs
    /// without a readable spine and DRM-protected publications.
    pub fn open(path: &Path) -> Result<Self> {
        let epub = rbook::Epub::options()
            .strict(false)
            .open(path)
            .with_context(|| format!("Cannot open EPUB file: {}", path.display()))?;

        let package_dir = epub.package().directory().as_str().to_string();
        validate_not_drm(&epub)?;

        let metadata = BookMetadata {
            title: epub
                .metadata()
                .title()
                .map(|title| title.value().trim().to_string())
                .filter(|title| !title.is_empty()),
            authors: epub
                .metadata()
                .creators()
                .map(|author| author.value().trim().to_string())
                .filter(|author| !author.is_empty())
                .collect(),
        };

        // Spine: canonical reading order, package-relative hrefs, only linear entries.
        let mut spine = Vec::new();
        for index in 0..epub.spine().len() {
            let Some(entry) = epub.spine().get(index) else {
                continue;
            };
            if !entry.is_linear() {
                continue;
            }
            let Some(manifest_entry) = entry.manifest_entry() else {
                continue;
            };
            let href = package_relative(&package_dir, manifest_entry.href().as_str());
            if href.is_empty() {
                continue;
            }
            spine.push(SpineItem {
                href,
                title_hint: String::new(),
            });
        }
        if spine.is_empty() {
            bail!("EPUB has an empty spine (no readable chapters)");
        }

        // TOC: EPUB3 nav or EPUB2 NCX, flattened depth-first. Entries that point outside the
        // spine keep `chapter_index: None` (the sidebar can still show them).
        let mut toc = Vec::new();
        if let Some(root) = epub.toc().contents() {
            for entry in root.flatten() {
                let label = entry.label().trim().to_string();
                if label.is_empty() {
                    continue;
                }
                let chapter_index = entry.resource().and_then(|resource| {
                    let key = resource.key().value()?;
                    let href = package_relative(&package_dir, key);
                    spine.iter().position(|item| item.href == href)
                });
                // In-chapter jump target: the `#fragment` of the resolved href (e.g. NCX
                // `c1.xhtml#s1` → `s1`), percent-decoded. `None` when the entry points at the
                // chapter start.
                let anchor = entry
                    .href()
                    .and_then(|href| href.fragment())
                    .map(util::percent_decode)
                    .map(|fragment| fragment.trim().to_string())
                    .filter(|fragment| !fragment.is_empty());
                toc.push(TocItem {
                    label,
                    // `flatten()` starts at the children of the synthetic root, so the
                    // first real entries report depth 1; normalize them to depth 0.
                    depth: entry.depth().saturating_sub(1),
                    chapter_index,
                    anchor,
                });
            }
        }
        // A TOC that only points outside the spine is useless for navigation: fall back to the
        // spine-derived chapter list so the sidebar is never inert.
        if toc.is_empty() || !toc.iter().any(|entry| entry.chapter_index.is_some()) {
            toc = derive_toc_from_spine(&spine);
        }

        let cover_bytes = epub
            .manifest()
            .cover_image()
            .and_then(|entry| entry.read_bytes().ok())
            .filter(|bytes| !bytes.is_empty());

        Ok(Self {
            epub,
            metadata,
            spine,
            toc,
            cover_bytes,
            chapters: Mutex::new(HashMap::new()),
        })
    }

    pub fn metadata(&self) -> &BookMetadata {
        &self.metadata
    }

    pub fn spine(&self) -> &[SpineItem] {
        &self.spine
    }

    pub fn toc(&self) -> &[TocItem] {
        &self.toc
    }

    pub fn chapter_count(&self) -> usize {
        self.spine.len()
    }

    pub fn spine_href(&self, index: usize) -> &str {
        self.spine
            .get(index)
            .map(|item| item.href.as_str())
            .unwrap_or("")
    }

    /// Resolve an in-content hyperlink (`href` in an `<a>` tag) relative to `chapter_index`.
    ///
    /// Handles the three common EPUB forms: pure fragments (`#note1`), chapter-relative paths
    /// (`../Text/ch2.xhtml#note1`) and absolute-ish paths (`/OEBPS/Text/ch2.xhtml`). Returns the
    /// target spine index plus the decoded fragment (if any); external URLs (`http:`, `mailto:`)
    /// return `None` and are ignored by the reader.
    pub fn resolve_href(
        &self,
        chapter_index: usize,
        href: &str,
    ) -> Option<(usize, Option<String>)> {
        let href = href.trim();
        if href.is_empty() || href.starts_with("http://") || href.starts_with("https://")
            || href.starts_with("mailto:") || href.starts_with("tel:")
        {
            return None;
        }
        let fragment = href
            .split_once('#')
            .map(|(_, fragment)| util::percent_decode(fragment))
            .filter(|fragment| !fragment.trim().is_empty());
        let path = util::strip_fragment(href).trim();
        if path.is_empty() {
            return Some((chapter_index, fragment));
        }
        let resolved = self.resolve_chapter_relative(chapter_index, path).ok()?;
        let target = util::percent_decode(util::strip_fragment(&resolved));
        let target = target.trim_start_matches('/');
        self.spine
            .iter()
            .position(|item| {
                util::percent_decode(util::strip_fragment(&item.href))
                    .trim_start_matches('/')
                    == target
            })
            .map(|index| (index, fragment))
    }

    /// Raw cover image bytes (PNG or JPEG, as shipped inside the EPUB), if the book declares one.
    pub fn cover_png(&self) -> Option<&[u8]> {
        self.cover_bytes.as_deref()
    }

    /// Fonts embedded in the publication (TTF/OTF manifest entries), ready to hand to GPUI's text
    /// system. The CSS font-family names in the book match the fonts' internal family names, so
    /// registering the bytes is enough for `font-family: Literata` etc. to resolve.
    ///
    /// WOFF/WOFF2 and legacy bitmap fonts are skipped: DirectWrite cannot load them directly.
    pub fn embedded_fonts(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for entry in self.epub.manifest().fonts() {
            let kind = entry.kind().as_str().to_ascii_lowercase();
            let supported = kind.contains("ttf")
                || kind.contains("otf")
                || kind.contains("sfnt")
                || kind.contains("opentype");
            if !supported {
                continue;
            }
            if let Ok(bytes) = entry.read_bytes()
                && !bytes.is_empty()
            {
                out.push(bytes);
            }
        }
        out
    }

    /// CSS family names declared by `@font-face` in the book's stylesheets.
    ///
    /// The font files' internal family names often differ slightly from the CSS name (e.g. the
    /// "Literata" CSS family ships as "Literata 12pt"), so the reader uses this list to build a
    /// CSS-name → loaded-family alias table after registering the fonts.
    pub fn embedded_font_families(&self) -> Vec<String> {
        let mut families = Vec::new();
        for entry in self.epub.manifest().styles() {
            let Ok(css) = entry.read_str() else {
                continue;
            };
            for face in util::parse_font_faces(&css) {
                if !families.iter().any(|known| known == &face.family) {
                    families.push(face.family);
                }
            }
        }
        families
    }

    /// Parses (and caches) one spine item into a [`BlockModel`].
    ///
    /// The model is computed against the default reader font size
    /// ([`crate::theme::CONTENT_DEFAULT_FONT_SIZE`]); renderers scale the absolute px sizes by
    /// `settings.font_size / CONTENT_DEFAULT_FONT_SIZE`.
    ///
    /// # Errors
    /// Returns an error for out-of-range indices or unreadable/malformed chapter resources.
    pub fn chapter(&self, index: usize) -> Result<Arc<BlockModel>> {
        {
            let cache = self
                .chapters
                .lock()
                .map_err(|_| anyhow::anyhow!("chapter cache is poisoned"))?;
            if let Some(model) = cache.get(&index) {
                return Ok(Arc::clone(model));
            }
        }
        let source = self.chapter_text(index)?;
        let model = Arc::new(xhtml::parse_chapter(self, index, &source)?);
        let mut cache = self
            .chapters
            .lock()
            .map_err(|_| anyhow::anyhow!("chapter cache is poisoned"))?;
        Ok(Arc::clone(cache.entry(index).or_insert(model)))
    }

    /// Raw resource bytes for a chapter-relative `src` (e.g. an `<img src>` value).
    ///
    /// The `src` is resolved relative to the chapter's resource path, percent-decoded and
    /// normalized (`./`, `..`, fragments); leading `/` paths are container-absolute.
    ///
    /// # Errors
    /// Returns an error when the chapter index is invalid or the resource is missing/unreadable.
    pub fn image_bytes(&self, chapter_index: usize, src: &str) -> Result<Vec<u8>> {
        let path = self.resolve_chapter_relative(chapter_index, src)?;
        self.epub
            .read_resource_bytes(path.as_str())
            .with_context(|| format!("image not found: {src}"))
    }

    /// String content of a chapter-relative resource (e.g. a linked stylesheet), decoded with
    /// the EPUB fallback encoding (see [`util::decode_text`]).
    pub fn resource_str(&self, chapter_index: usize, href: &str) -> Result<String> {
        let path = self.resolve_chapter_relative(chapter_index, href)?;
        let bytes = self
            .epub
            .read_resource_bytes(path.as_str())
            .with_context(|| format!("resource not found: {href}"))?;
        Ok(util::decode_text(&bytes))
    }

    fn chapter_text(&self, index: usize) -> Result<String> {
        let href = self
            .spine
            .get(index)
            .map(|item| item.href.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "chapter index {index} out of range ({} chapters)",
                    self.spine.len()
                )
            })?;
        let bytes = self
            .epub
            .read_resource_bytes(href)
            .with_context(|| format!("chapter {index} ({href}) unreadable"))?;
        Ok(util::decode_text(&bytes))
    }

    fn resolve_chapter_relative(&self, chapter_index: usize, href: &str) -> Result<String> {
        let chapter_href = self
            .spine
            .get(chapter_index)
            .map(|item| item.href.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "chapter index {chapter_index} out of range ({} chapters)",
                    self.spine.len()
                )
            })?;
        if href.trim().is_empty() {
            bail!("empty resource path");
        }
        let base_dir = chapter_href
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let normalized = util::normalize_package_path(base_dir, href);
        if normalized.is_empty() {
            bail!("unresolvable resource path: {href}");
        }
        // A leading `/` in the source href addresses the container root, not the package
        // directory, so keep it absolute when handing the path to `rbook`.
        let decoded = util::percent_decode(util::strip_fragment(href));
        let path = if decoded.starts_with('/') {
            format!("/{normalized}")
        } else {
            normalized
        };
        Ok(path)
    }
}

/// TOC fallback used whenever an EPUB ships no usable navigation document: one flat entry per
/// spine item (`Chapter 1`, `Chapter 2`, …), unless the spine carries a title hint.
pub fn derive_toc_from_spine(spine: &[SpineItem]) -> Vec<TocItem> {
    spine
        .iter()
        .enumerate()
        .map(|(i, item)| TocItem {
            label: if item.title_hint.is_empty() {
                format!("Chapter {}", i + 1)
            } else {
                item.title_hint.clone()
            },
            depth: 0,
            chapter_index: Some(i),
            anchor: None,
        })
        .collect()
}

/// Converts an `rbook` container-absolute href (`/OEBPS/text/c1.xhtml`) to a package-relative,
/// percent-decoded path (`text/c1.xhtml`).
fn package_relative(package_dir: &str, absolute: &str) -> String {
    let path = util::percent_decode(util::strip_fragment(absolute));
    let dir = package_dir.trim_end_matches('/');
    let relative = if dir.is_empty() {
        path.as_str()
    } else {
        path.strip_prefix(dir).unwrap_or(&path)
    };
    relative.trim_start_matches('/').to_string()
}

/// Rejects DRM-protected publications up front (spec §9): an `encryption.xml` that encrypts
/// content resources is not readable by this app. Fonts-only obfuscation (a common, benign
/// case) is tolerated.
fn validate_not_drm(epub: &rbook::Epub) -> Result<()> {
    let Ok(encryption) = epub.read_resource_str("/META-INF/encryption.xml") else {
        return Ok(());
    };
    let lower = encryption.to_ascii_lowercase();
    const CONTENT_MARKERS: [&str; 6] = [
        ".xhtml",
        ".html",
        ".opf",
        ".ncx",
        ".css",
        "application/xhtml+xml",
    ];
    if CONTENT_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        bail!("This EPUB is DRM-protected and cannot be opened");
    }
    Ok(())
}

