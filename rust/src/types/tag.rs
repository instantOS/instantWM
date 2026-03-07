//! Tag system types.
//!
//! Types for workspace tags, layouts, and tag management.

use crate::layouts::LayoutKind;
use crate::types::color::TagColorConfigs;

/// Identifies which layout slot (primary or secondary) is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutSlot {
    /// The primary layout slot (index 0).
    #[default]
    Primary,
    /// The secondary layout slot (index 1).
    Secondary,
}

impl LayoutSlot {
    /// Convert to a usize index (0 for Primary, 1 for Secondary).
    pub const fn as_index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Secondary => 1,
        }
    }

    /// Create a LayoutSlot from a usize index.
    ///
    /// Returns `None` if the index is not 0 or 1.
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Primary),
            1 => Some(Self::Secondary),
            _ => None,
        }
    }

    /// Toggle between Primary and Secondary.
    pub const fn toggle(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Primary,
        }
    }
}

/// Stores layout state for a tag with last-used tracking.
///
/// Each tag maintains its current layout and remembers the previously used layout,
/// enabling `restore_last_layout()` functionality.
#[derive(Debug, Clone, Copy)]
pub struct TagLayouts {
    primary: LayoutKind,
    secondary: LayoutKind,
    active_slot: LayoutSlot,
    last_layout: Option<LayoutKind>,
}

impl Default for TagLayouts {
    fn default() -> Self {
        Self {
            primary: LayoutKind::Tile,
            secondary: LayoutKind::Floating,
            active_slot: LayoutSlot::default(),
            last_layout: None,
        }
    }
}

impl TagLayouts {
    /// Get the currently active layout.
    pub fn get_layout(self) -> LayoutKind {
        match self.active_slot {
            LayoutSlot::Primary => self.primary,
            LayoutSlot::Secondary => self.secondary,
        }
    }

    /// Set a new layout on the active slot, saving the current one to `last_layout`.
    ///
    /// If the new layout matches the current one, this is a no-op.
    pub fn set_layout(&mut self, layout: LayoutKind) {
        let current = self.get_layout();
        if current == layout {
            return;
        }
        self.last_layout = Some(current);
        match self.active_slot {
            LayoutSlot::Primary => self.primary = layout,
            LayoutSlot::Secondary => self.secondary = layout,
        }
    }

    /// Swap the current layout with the last used layout.
    ///
    /// Returns `true` if a swap occurred, `false` if no last layout was stored.
    pub fn restore_last_layout(&mut self) -> bool {
        let current = self.get_layout();
        let last = self.last_layout.take();

        match last {
            Some(last) => {
                self.last_layout = Some(current);
                match self.active_slot {
                    LayoutSlot::Primary => self.primary = last,
                    LayoutSlot::Secondary => self.secondary = last,
                }
                true
            }
            None => false,
        }
    }

    /// Returns true if the current layout is a tiling layout.
    pub fn is_tiling(self) -> bool {
        self.get_layout().is_tiling()
    }

    /// Returns true if the current layout is a monocle layout.
    pub fn is_monocle(self) -> bool {
        self.get_layout().is_monocle()
    }

    /// Returns true if the current layout is an overview layout.
    pub fn is_overview(self) -> bool {
        self.get_layout().is_overview()
    }

    /// Get the symbol of the current layout.
    pub fn symbol(self) -> &'static str {
        self.get_layout().symbol()
    }

    /// Toggle between primary and secondary slots.
    ///
    /// Saves current layout to `last_layout` before toggling.
    pub fn toggle_slot(&mut self) {
        self.last_layout = Some(self.get_layout());
        self.active_slot = self.active_slot.toggle();
    }
}

/// A single workspace tag.
#[derive(Debug, Clone)]
pub struct Tag {
    /// Display name for the tag.
    pub name: String,
    /// Alternative name (shown when `show_alt` is true).
    pub alt_name: String,
    /// Number of clients in the master area for tiling layouts.
    pub nmaster: i32,
    /// Master factor for tiling layouts (0.0 to 1.0).
    pub mfact: f32,
    /// Whether to show the bar on this tag.
    pub showbar: bool,
    /// The layouts for this tag (primary and secondary).
    pub layouts: TagLayouts,
}

impl Default for Tag {
    fn default() -> Self {
        Self {
            name: String::new(),
            alt_name: String::new(),
            nmaster: 1,
            mfact: 0.55,
            showbar: true,
            layouts: TagLayouts::default(),
        }
    }
}

impl Tag {
    /// Return the display name (regular or alt name).
    pub fn display_name(&self, show_alternative: bool) -> &str {
        if show_alternative && !self.alt_name.is_empty() {
            &self.alt_name
        } else {
            &self.name
        }
    }
}

/// All tag-related configuration and runtime state, grouped in one place.
///
/// Tag data (names, layouts, nmaster, mfact, showbar) lives on each
/// `Monitor` in its `tags: Vec<Tag>` field. `TagSet` only holds the
/// metadata that is shared across all monitors.
#[derive(Debug, Clone, Default)]
pub struct TagSet {
    /// Number of tags configured. Each monitor owns its own `Vec<Tag>` of
    /// this length; this value is used for mask/count helpers that don't
    /// have a monitor reference handy.
    pub num_tags: usize,
    /// Raw colour strings from config.
    pub colors: TagColorConfigs,
    /// Whether to display `alt_names` instead of `names`.
    pub show_alternative_names: bool,
    /// Prefix-key mode: next tag key toggles rather than views.
    pub prefix: bool,
    /// Cached pixel width of the tag strip in the bar.
    pub width: i32,
}

impl TagSet {
    /// Bitmask covering all active tags: `(1 << count) - 1`.
    #[inline]
    pub fn mask(&self) -> u32 {
        (1u32 << self.num_tags).wrapping_sub(1)
    }

    /// Number of active tags.
    #[inline]
    pub fn count(&self) -> usize {
        self.num_tags
    }
}
