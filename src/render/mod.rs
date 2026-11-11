//! BlockModel → GPUI element rendering.
//!
//! `text` maps inline runs to `StyledText` highlights, `blocks` builds one element per block type
//! using the parity styles from the design spec §6.1/§6.2, `images` decodes EPUB image bytes into
//! `gpui::Image` elements with a per-chapter cache, and `anchor` provides the zero-size probe
//! element used to measure block positions for in-chapter TOC jumps.

pub mod anchor;
pub mod blocks;
pub mod images;
pub mod text;

pub use anchor::{AnchorPositions, AnchorProbe, anchor_positions};
pub use blocks::{LinkHandler, render_blocks, render_blocks_full, render_blocks_with_anchors};
pub use images::ImageCache;
