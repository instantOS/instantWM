//! Stateful layout orchestration, split by responsibility.

mod arrange;
mod commands;
mod pointer;
mod z_order;

pub use arrange::{arrange, arrange_monitor};
pub(crate) use commands::finish_layout_change;
#[cfg(test)]
use commands::shifted_master_count;
pub use commands::{
    MaximizedStackReorder, apply_tree_preset, cycle_layout_direction, focus_tree_neighbor,
    inc_master_count_by, promote_tree, reorder_maximized_stack, reset_active_layout, resize_tree,
    resize_tree_smart, set_layout, swap_bar_titles, swap_tree_neighbor,
    toggle_floating_presentation, toggle_tiling_maximized,
};
#[cfg(test)]
use pointer::available_tree_resize_direction;
pub(crate) use pointer::{
    PointerPlacementPreviewCache, PointerTreeResizeStart, pointer_tree_gap_resize_start,
    pointer_tree_resize_start, selected_tiling, update_pointer_tree_resize,
    uses_manual_tree_pointer_interaction,
};
pub use pointer::{place_tree_at_point, preview_tree_at_point};
#[cfg(test)]
use z_order::compute_monitor_z_order;
pub use z_order::sync_monitor_z_order;

#[cfg(test)]
mod tests;
