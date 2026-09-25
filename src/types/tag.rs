//! Tag-system types.

use crate::types::{TagMask, color::TagColorConfigs};

/// Maximum byte-length (excluding the NUL terminator) accepted for a tag
/// label, shared by the config schema and runtime renames.
pub const MAX_TAG_NAME_BYTES: usize = 16;

/// Default number of tag cells in the bar (`bar.tag_slots`).
pub const DEFAULT_TAG_SLOTS: u32 = 9;

/// A single workspace tag.
#[derive(Debug, Clone, Default)]
pub struct Tag {
    /// Display name for the tag.
    pub name: String,
    /// Icon shown in place of the name while icon mode is active. Empty
    /// means "no icon"; the name is displayed instead.
    pub icon: String,
}

impl Tag {
    /// Return the display label: the icon while icon mode is on and an icon
    /// is set, otherwise the name.
    pub fn display_label(&self, show_icons: bool) -> &str {
        if show_icons && !self.icon.is_empty() {
            &self.icon
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tag() -> Tag {
        Tag {
            name: "web".into(),
            icon: "W".into(),
        }
    }

    #[test]
    fn display_label_prefers_the_icon_only_while_icon_mode_is_on() {
        let tag = tag();
        assert_eq!(tag.display_label(false), "web");
        assert_eq!(tag.display_label(true), "W");
    }

    #[test]
    fn display_label_falls_back_to_the_name_without_an_icon() {
        let tag = Tag {
            name: "web".into(),
            icon: String::new(),
        };
        assert_eq!(tag.display_label(true), "web");
        assert_eq!(tag.display_label(false), "web");
    }
}
