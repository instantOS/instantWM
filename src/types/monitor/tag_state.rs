use super::*;

/// Runtime state restored when a tag mask is revisited.
/// Initialized with hardcoded defaults on first visit.
#[derive(Debug, Clone)]
pub struct PerTagState {
    pub master_count: i32,
    pub master_factor: f32,
    pub show_bar: bool,
    pub layouts: TagLayouts,
    /// Persistent manual tiling topology for this exact visible tag mask.
    pub layout_tree: crate::layouts::tree::LayoutTree,
    /// Last one-shot tree preset command, used only as the cycle cursor.
    pub last_tree_layout: crate::layouts::tree::Preset,
}

impl Default for PerTagState {
    fn default() -> Self {
        Self::new(true)
    }
}

impl PerTagState {
    pub fn new(show_bar: bool) -> Self {
        Self {
            master_count: 1,
            master_factor: 0.55,
            show_bar,
            layouts: TagLayouts::default(),
            layout_tree: crate::layouts::tree::LayoutTree::default(),
            last_tree_layout: crate::layouts::tree::Preset::MasterStack,
        }
    }
}

/// Per-tag name data. No runtime layout state.
#[derive(Debug, Clone, Default)]
pub struct TagNames {
    pub name: String,
    pub alt_name: String,
}
