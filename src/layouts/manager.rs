//! Stateful layout orchestration, split by responsibility.

mod arrange;
mod commands;
mod pointer;
mod z_order;

#[cfg(test)]
use arrange::clients_with_planned_borders;
pub use arrange::{arrange, arrange_monitor};
pub(crate) use commands::finish_layout_change;
#[cfg(test)]
use commands::shifted_master_count;
pub use commands::{
    apply_tree_preset, cycle_layout_direction, focus_tree_neighbor, inc_master_count_by,
    promote_tree, resize_tree, resize_tree_smart, set_layout, swap_tree_neighbor,
    toggle_floating_presentation, toggle_tiling_maximized,
};
pub(crate) use pointer::{
    PointerPlacementPreviewCache, PointerTreeResizeStart, apply_tree_target,
    pointer_tree_resize_start, preview_tree_target, tree_placement_targets,
    update_pointer_tree_resize, uses_manual_tree_pointer_interaction,
};
#[cfg(test)]
use pointer::{
    available_tree_resize_direction, manual_tree_pointer_interaction_allowed,
    pointer_tree_resize_allowed, selected_tiling_constraints,
};
pub use pointer::{place_tree_at_point, preview_tree_at_point};
#[cfg(test)]
use z_order::compute_monitor_z_order;
pub use z_order::sync_monitor_z_order;

#[cfg(test)]
mod tests;
