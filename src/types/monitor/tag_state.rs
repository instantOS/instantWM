use std::collections::HashMap;

/// Runtime state restored when a tag mask is revisited.
/// Initialized with hardcoded defaults on first visit.
#[derive(Debug, Clone)]
pub struct PerTagState {
    pub master_count: usize,
    pub show_bar: bool,
    pub presentation: crate::layouts::PresentationMode,
    /// Live manual tiling topology of the *active* layout slot for this exact
    /// visible tag mask. Every tree edit applies here, whatever the layout.
    pub layout_tree: crate::layouts::tree::LayoutTree,
    /// Which layout slot `layout_tree` currently represents.
    pub active_preset: crate::layouts::tree::Preset,
    /// Whether the active slot was chosen by an explicit layout command.
    /// Until then the organically grown default tree belongs to no slot and
    /// merely seeds the first activated layout.
    pub preset_activated: bool,
    /// Remembered trees of inactive layout slots, including their manual
    /// edits. A missing entry was never activated and seeds from the live
    /// tree the next time its layout is selected.
    pub stored_trees: HashMap<crate::layouts::tree::Preset, crate::layouts::tree::LayoutTree>,
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
            show_bar,
            presentation: crate::layouts::PresentationMode::Tiled,
            layout_tree: crate::layouts::tree::LayoutTree::default(),
            active_preset: crate::layouts::tree::Preset::MasterStack,
            preset_activated: false,
            stored_trees: HashMap::new(),
        }
    }
}
