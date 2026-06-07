//! Single source of truth for emoji icons used in printed output.
//!
//! Defining the literals here keeps the renderer, its unit tests, and the
//! CLI integration tests agreed on the exact code points, so a change to
//! an icon only needs to be made in one place.

/// Globe glyph (U+1F30D) prefixing a comment permalink.
pub const ICON_PERMALINK: &str = "\u{1f30d}";

/// Document glyph (U+1F4C4) prefixing a file path.
pub const ICON_FILE: &str = "\u{1f4c4}";

/// Speech-balloon glyph (U+1F4AC) prefixing a comment author banner.
pub const ICON_COMMENT: &str = "\u{1f4ac}";

/// Memo glyph (U+1F4DD) prefixing a review author banner.
pub const ICON_REVIEW: &str = "\u{1f4dd}";
