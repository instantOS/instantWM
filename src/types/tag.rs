//! Tag-system types.

use crate::types::{TagMask, color::TagColorConfigs};

/// A single workspace tag.
#[derive(Debug, Clone, Default)]
pub struct Tag {
    /// Display name for the tag.
    pub name: String,
    /// Alternative name (shown when `show_alt` is true).
    pub alt_name: String,
}

impl Tag {
    /// Return the display name (regular or alternative).
    pub fn display_name(&self, show_alternative: bool) -> &str {
        if show_alternative && !self.alt_name.is_empty() {
            &self.alt_name
        } else {
            &self.name
        }
    }
}

/// Metadata shared by the per-monitor tag sets.
#[derive(Debug, Clone, Default)]
pub struct TagSet {
    pub num_tags: usize,
    pub colors: TagColorConfigs,
    pub show_alternative_names: bool,
    /// Cached width of the complete tag list in the bar.
    pub width: i32,
}

impl TagSet {
    #[inline]
    pub fn mask(&self) -> TagMask {
        TagMask::all(self.num_tags)
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.num_tags
    }
}
