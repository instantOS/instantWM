mod args;
mod behavior;

#[cfg(test)]
mod tests;

use super::apply_config_effect;
use args::ActionArgs;
pub use args::ConfigAssignment;
use behavior::{
    DEFAULT_GAP_STEP, adjust_gaps, edge_scratchpad_set_direction, focus_horizontal, focus_vertical,
    key_resize_or_tree, move_horizontal, move_vertical, validate_mode_name, with_selected_win,
};

use crate::actions::ActionInfo;
use crate::client::fullscreen::toggle_fake_fullscreen;
use crate::client::{kill_client, shut_kill, zoom};
use crate::contexts::WmCtx;
use crate::floating::scratchpad::DEFAULT_SCRATCHPAD_NAME;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, center_window, distribute_clients, edge_scratchpad_create,
    scratchpad_create, scratchpad_hide_name, scratchpad_restore, scratchpad_show_name,
    scratchpad_toggle, toggle_floating,
};
use crate::focus::{focus_last_client, focus_stack};
use crate::ipc_types::ScratchpadInitialStatus;
use crate::keyboard::alt_tab_key;
use crate::layouts::tree::Side;
use crate::layouts::{
    LayoutCommand, begin_tree_placement, center_keyboard_tree_placement,
    cycle_keyboard_tree_placement, cycle_layout_direction, finish_keyboard_tree_placement,
    inc_master_count_by, reset_active_layout, resize_keyboard_tree_placement, resize_tree_smart,
    set_layout, step_keyboard_tree_placement, swap_keyboard_tree_placement, swap_tree_neighbor,
    toggle_floating_presentation, toggle_tiling_maximized,
};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow};
use crate::mouse::draw_window;
use crate::overview::ActionTransition;
use crate::tags::{
    cancel_overview, follow_view, last_view, move_client_follow_view, send_to_monitor, shift_tag,
    shift_view, toggle_overview, win_view,
};
use crate::toggles::{toggle_bar, toggle_mode, toggle_sticky, unhide_all};
use crate::types::{
    EdgeDirection, HorizontalDirection, MonitorDirection, StackDirection, TagMask, TagSelection,
    ToggleAction, VerticalDirection,
};
use crate::util::spawn;

/// Declares every named action once: its name, overview policy (default:
/// confirm, so a new mutating action never operates on the overview
/// projection), documentation, typed argument and behaviour.
macro_rules! define_named_actions {
    (@transition) => { ActionTransition::Confirm };
    (@transition $transition:ident) => { ActionTransition::$transition };
    (@usage) => { None };
    (@usage $arg:ty) => { Some(<$arg as ActionArgs>::usage()) };
    (@render) => { Vec::new() };
    (@render $value:ident) => { ActionArgs::render($value) };
    (@parse $name:literal, $variant:ident, $args:ident) => {
        if $args.is_empty() {
            Self::$variant
        } else {
            return Err(format!("action '{}' takes no arguments", $name));
        }
    };
    (@parse $name:literal, $variant:ident, $args:ident, $arg:ty) => {
        Self::$variant(
            <$arg as ActionArgs>::parse($args)
                .map_err(|error| format!("action '{}': {error}", $name))?,
        )
    };
    ($(
        $variant:ident $(($arg:ty))? => {
            name: $name:literal,
            $(overview: $transition:ident,)?
            doc: $doc:literal,
            run: |$ctx:ident $(, $value:ident)?| $body:block
        }
    ),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq)]
        pub enum NamedAction {
            $($variant $(($arg))?,)+
        }

        impl NamedAction {
            pub const fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $name,)+
                }
            }

            /// Parse an action from its name and textual arguments.
            pub fn parse(name: &str, args: &[String]) -> Result<Self, String> {
                Ok(match name {
                    $($name => define_named_actions!(@parse $name, $variant, args $(, $arg)?),)+
                    _ => return Err(format!("unknown action '{name}'")),
                })
            }

            /// The textual arguments [`Self::parse`] accepts for this action.
            pub fn args(&self) -> Vec<String> {
                match self {
                    $(Self::$variant $(($value))? => define_named_actions!(@render $($value)?),)+
                }
            }

            pub(crate) fn overview_transition(&self) -> ActionTransition {
                match self {
                    $(Self::$variant { .. } => define_named_actions!(@transition $($transition)?),)+
                }
            }

            pub(crate) fn execute(&self, ctx: &mut WmCtx<'_>) -> Result<(), String> {
                crate::overview::prepare_named_action(ctx, self);
                match self {
                    $(Self::$variant $(($value))? => {
                        let $ctx = &mut *ctx;
                        $body
                    })+
                }
                Ok(())
            }
        }

        /// Every named action with its documentation, sorted by name.
        pub fn action_infos() -> Vec<ActionInfo> {
            let mut infos = vec![
                $(ActionInfo {
                    name: $name,
                    description: $doc,
                    arg_example: define_named_actions!(@usage $($arg)?),
                }),+
            ];
            infos.sort_by_key(|info| info.name);
            infos
        }
    };
}

impl NamedAction {
    /// Human-readable form, e.g. `spawn ins settings --gui`.
    pub fn describe(&self) -> String {
        std::iter::once(self.name().to_string())
            .chain(self.args())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

define_named_actions!(
    Zoom => { name: "zoom", doc: "zoom client into master area", run: |ctx| { zoom(ctx); } },
    Kill => { name: "kill", doc: "close focused window gracefully", run: |ctx| { with_selected_win(ctx, kill_client); } },
    ShutKill => { name: "shut_kill", doc: "force kill focused window", run: |ctx| { shut_kill(ctx); } },
    Quit => { name: "quit", doc: "quit instantwm", run: |ctx| { ctx.core_mut().quit(); } },
    FocusNext => { name: "focus_next", overview: Preserve, doc: "focus next window in stack", run: |ctx| { focus_stack(ctx, StackDirection::Next); } },
    FocusPrev => { name: "focus_prev", overview: Preserve, doc: "focus previous window in stack", run: |ctx| { focus_stack(ctx, StackDirection::Previous); } },
    FocusLast => { name: "focus_last", overview: Cancel, doc: "focus last focused window", run: |ctx| { focus_last_client(ctx); } },
    FocusUp => { name: "focus_up", overview: Preserve, doc: "focus above; at the boundary follow focus.vertical_edge", run: |ctx| { focus_vertical(ctx, VerticalDirection::Up); } },
    FocusDown => { name: "focus_down", overview: Preserve, doc: "focus below; at the boundary follow focus.vertical_edge", run: |ctx| { focus_vertical(ctx, VerticalDirection::Down); } },
    FocusLeft => { name: "focus_left", overview: Preserve, doc: "focus left, or move backward through bar order in maximized presentation; at the boundary follow focus.horizontal_edge", run: |ctx| { focus_horizontal(ctx, HorizontalDirection::Left); } },
    FocusRight => { name: "focus_right", overview: Preserve, doc: "focus right, or move forward through bar order in maximized presentation; at the boundary follow focus.horizontal_edge", run: |ctx| { focus_horizontal(ctx, HorizontalDirection::Right); } },
    DownKey => { name: "down_key", overview: Preserve, doc: "alt-tab forward", run: |ctx| { alt_tab_key(ctx, VerticalDirection::Down); } },
    UpKey => { name: "up_key", overview: Preserve, doc: "alt-tab backward", run: |ctx| { alt_tab_key(ctx, VerticalDirection::Up); } },
    LayoutFloat => { name: "layout_float", doc: "toggle floating layout presentation without changing per-window floating state", run: |ctx| { toggle_floating_presentation(ctx); } },
    ToggleTilingMaximized => { name: "toggle_tiling_maximized", doc: "toggle maximized-stack presentation, or restore manual tiling from floating layout", run: |ctx| { toggle_tiling_maximized(ctx); } },
    CycleLayoutNext => { name: "cycle_layout_next", doc: "cycle to next layout", run: |ctx| { cycle_layout_direction(ctx, true); } },
    CycleLayoutPrev => { name: "cycle_layout_prev", doc: "cycle to previous layout", run: |ctx| { cycle_layout_direction(ctx, false); } },
    IncMasterCount(Option<i32>) => { name: "inc_master_count", doc: "change the master window count (default +1)", run: |ctx, delta| { inc_master_count_by(ctx, delta.unwrap_or(1)); } },
    IncGaps(Option<i32>) => { name: "inc_gaps", doc: "increase tiled inner and outer gaps", run: |ctx, delta| { adjust_gaps(ctx, delta.unwrap_or(DEFAULT_GAP_STEP)); } },
    DecGaps(Option<i32>) => { name: "dec_gaps", doc: "decrease tiled inner and outer gaps", run: |ctx, delta| { adjust_gaps(ctx, -delta.unwrap_or(DEFAULT_GAP_STEP)); } },
    CenterWindow => { name: "center_window", doc: "center focused window", run: |ctx| { with_selected_win(ctx, center_window); } },
    DistributeClients => { name: "distribute_clients", doc: "distribute windows evenly", run: |ctx| { distribute_clients(ctx); } },
    KeyResizeUp => { name: "key_resize_up", doc: "grow a tiled window vertically or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Top, VerticalDirection::Up.into()); } },
    KeyResizeDown => { name: "key_resize_down", doc: "shrink a tiled window vertically or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Bottom, VerticalDirection::Down.into()); } },
    KeyResizeLeft => { name: "key_resize_left", doc: "shrink a tiled window horizontally or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Left, HorizontalDirection::Left.into()); } },
    KeyResizeRight => { name: "key_resize_right", doc: "grow a tiled window horizontally or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Right, HorizontalDirection::Right.into()); } },
    KeyMoveUp => { name: "key_move_up", doc: "move toward the previous maximized title, swap a tiled window upward, or move a floating window", run: |ctx| { move_vertical(ctx, VerticalDirection::Up); } },
    KeyMoveDown => { name: "key_move_down", doc: "move toward the next maximized title, swap a tiled window downward, or move a floating window", run: |ctx| { move_vertical(ctx, VerticalDirection::Down); } },
    KeyMoveLeft => { name: "key_move_left", doc: "move toward the previous maximized title or move left, carrying the window to the adjacent tag at the boundary", run: |ctx| { move_horizontal(ctx, HorizontalDirection::Left); } },
    KeyMoveRight => { name: "key_move_right", doc: "move toward the next maximized title or move right, carrying the window to the adjacent tag at the boundary", run: |ctx| { move_horizontal(ctx, HorizontalDirection::Right); } },
    TreeGrow => { name: "tree_grow", doc: "grow the focused window along its most local split", run: |ctx| { resize_tree_smart(ctx, true); } },
    TreeShrink => { name: "tree_shrink", doc: "shrink the focused window along its most local split", run: |ctx| { resize_tree_smart(ctx, false); } },
    PushUp => { name: "push_up", doc: "swap a tiled window upward (legacy action)", run: |ctx| { swap_tree_neighbor(ctx, Side::Top); } },
    PushDown => { name: "push_down", doc: "swap a tiled window downward (legacy action)", run: |ctx| { swap_tree_neighbor(ctx, Side::Bottom); } },
    LastView => { name: "last_view", overview: Cancel, doc: "view previously viewed tags", run: |ctx| { last_view(ctx); } },
    FollowView => { name: "follow_view", doc: "follow client to its tags", run: |ctx| { follow_view(ctx); } },
    WinView => { name: "win_view", doc: "view tags of focused client", run: |ctx| { win_view(ctx); } },
    ScrollLeft => { name: "scroll_left", overview: Cancel, doc: "scroll tags left", run: |ctx| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Left); } },
    ScrollRight => { name: "scroll_right", overview: Cancel, doc: "scroll tags right", run: |ctx| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Right); } },
    MoveClientLeft => { name: "move_client_left", doc: "move client to tag on left", run: |ctx| { move_client_follow_view(ctx, HorizontalDirection::Left); } },
    MoveClientRight => { name: "move_client_right", doc: "move client to tag on right", run: |ctx| { move_client_follow_view(ctx, HorizontalDirection::Right); } },
    ShiftTagLeft => { name: "shift_tag_left", doc: "shift client to tag on left", run: |ctx| { shift_tag(ctx, HorizontalDirection::Left); } },
    ShiftTagRight => { name: "shift_tag_right", doc: "shift client to tag on right", run: |ctx| { shift_tag(ctx, HorizontalDirection::Right); } },
    ShiftViewLeft => { name: "shift_view_left", overview: Cancel, doc: "shift view to tag on left", run: |ctx| { shift_view(ctx, HorizontalDirection::Left); } },
    ShiftViewRight => { name: "shift_view_right", overview: Cancel, doc: "shift view to tag on right", run: |ctx| { shift_view(ctx, HorizontalDirection::Right); } },
    ViewAll => { name: "view_all", overview: Cancel, doc: "view all tags", run: |ctx| { crate::tags::view::view_selection(ctx, TagSelection::All); } },
    TagAll => { name: "tag_all", doc: "tag client with all tags", run: |ctx| { with_selected_win(ctx, |ctx, win| crate::tags::client_tags::set_client_tag(ctx, win, TagMask::ALL_BITS)); } },
    ToggleOverview => { name: "toggle_overview", overview: Preserve, doc: "toggle overview mode", run: |ctx| { toggle_overview(ctx, TagMask::ALL_BITS); } },
    CancelOverview => { name: "cancel_overview", overview: Preserve, doc: "leave overview and restore previous view", run: |ctx| { cancel_overview(ctx, TagMask::ALL_BITS); } },
    EdgeScratchpadToggle => { name: "edge_scratchpad_toggle", overview: Preserve, doc: "toggle the default edge scratchpad", run: |ctx| { scratchpad_toggle(ctx, Some(DEFAULT_EDGE_SCRATCHPAD_NAME)); } },
    EdgeScratchpadCreate => { name: "edge_scratchpad_create", doc: "toggle the default edge scratchpad (create from the focused window, or restore if it exists)", run: |ctx| { edge_scratchpad_create(ctx); } },
    EdgeScratchpadShow => { name: "edge_scratchpad_show", overview: Preserve, doc: "show the default edge scratchpad", run: |ctx| { let _ = scratchpad_show_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME); } },
    EdgeScratchpadHide => { name: "edge_scratchpad_hide", overview: Preserve, doc: "hide the default edge scratchpad", run: |ctx| { scratchpad_hide_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME); } },
    EdgeScratchpadDirectionUp => { name: "edge_scratchpad_direction_up", overview: Preserve, doc: "set default edge scratchpad direction to top", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Top); } },
    EdgeScratchpadDirectionDown => { name: "edge_scratchpad_direction_down", overview: Preserve, doc: "set default edge scratchpad direction to bottom", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Bottom); } },
    EdgeScratchpadDirectionLeft => { name: "edge_scratchpad_direction_left", overview: Preserve, doc: "set default edge scratchpad direction to left", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Left); } },
    EdgeScratchpadDirectionRight => { name: "edge_scratchpad_direction_right", overview: Preserve, doc: "set default edge scratchpad direction to right", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Right); } },
    ScratchpadToggle => {
        name: "scratchpad_toggle",
        doc: "toggle scratchpad, creating it from current window if it doesn't exist",
        run: |ctx| {
            if ctx.core().model().scratchpad_find(DEFAULT_SCRATCHPAD_NAME).is_some() {
                scratchpad_toggle(ctx, Some(DEFAULT_SCRATCHPAD_NAME));
            } else {
                let _ = scratchpad_create(ctx, DEFAULT_SCRATCHPAD_NAME, None, None, ScratchpadInitialStatus::Shown);
            }
        }
    },
    ScratchpadRestore => { name: "scratchpad_restore", doc: "restore the focused scratchpad as an ordinary window", run: |ctx| { let _ = scratchpad_restore(ctx, None, None); } },
    ToggleBar => { name: "toggle_bar", overview: Preserve, doc: "toggle status bar", run: |ctx| { toggle_bar(ctx); } },
    ToggleBottomBar(Option<ToggleAction>) => { name: "toggle_bottom_bar", overview: Preserve, doc: "toggle or set bottom bar visibility", run: |ctx, action| {
        let mut shown = ctx.core().model().expect_selected_monitor().shows_bottom_bar();
        action.unwrap_or_default().apply(&mut shown);
        crate::toggles::set_bottom_bar_shown(ctx, shown);
    } },
    ToggleFloating => { name: "toggle_floating", doc: "toggle focused window between tiled and floating", run: |ctx| { toggle_floating(ctx); } },
    ToggleSticky => { name: "toggle_sticky", doc: "toggle sticky (visible on all tags)", run: |ctx| { with_selected_win(ctx, toggle_sticky); } },
    ConfigSet(ConfigAssignment) => { name: "config_set", overview: Preserve, doc: "set a runtime config value (e.g. config_set layout.inner_gap 12)", run: |ctx, assignment| { let effect = crate::config::runtime::set_runtime_field(ctx.core_mut().state_mut(), &assignment.key, assignment.value.clone())?; apply_config_effect(ctx, effect); } },
    ConfigToggle(String) => { name: "config_toggle", overview: Preserve, doc: "flip a boolean runtime config value (e.g. config_toggle window.decor_hints)", run: |ctx, key| { let (effect, _) = crate::config::runtime::toggle_runtime_field(ctx.core_mut().state_mut(), key)?; apply_config_effect(ctx, effect); } },
    ModeToggle(String) => { name: "mode_toggle", doc: "toggle a mode (enter if not active, else return to default)", run: |ctx, mode| { validate_mode_name(&ctx.core().config().bindings.modes, mode)?; toggle_mode(ctx, mode); } },
    UnhideAll => { name: "unhide_all", doc: "show all hidden windows", run: |ctx| { unhide_all(ctx); } },
    Hide => { name: "hide", doc: "minimize focused window or hide the visible scratchpad", run: |ctx| { with_selected_win(ctx, crate::client::hide_for_user); } },
    ToggleFakeFullscreen => { name: "toggle_fake_fullscreen", doc: "toggle fake fullscreen", run: |ctx| { toggle_fake_fullscreen(ctx); } },
    DrawWindow => { name: "draw_window", doc: "start dragging/resizing window", run: |ctx| { draw_window(ctx); } },
    BeginTreePlacement => { name: "begin_tree_placement", doc: "place the focused tiled window within its layout tree", run: |ctx| { let _ = begin_tree_placement(ctx); } },
    PlacementLeft => { name: "placement_left", doc: "select the placement target to the left", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementRight => { name: "placement_right", doc: "select the placement target to the right", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementUp => { name: "placement_up", doc: "select the placement target above", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementDown => { name: "placement_down", doc: "select the placement target below", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementSwapLeft => { name: "placement_swap_left", doc: "swap the armed window with its left neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementSwapRight => { name: "placement_swap_right", doc: "swap the armed window with its right neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementSwapUp => { name: "placement_swap_up", doc: "swap the armed window with its upper neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementSwapDown => { name: "placement_swap_down", doc: "swap the armed window with its lower neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementResizeLeft => { name: "placement_resize_left", doc: "resize the armed window at its left edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementResizeRight => { name: "placement_resize_right", doc: "resize the armed window at its right edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementResizeUp => { name: "placement_resize_up", doc: "resize the armed window at its upper edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementResizeDown => { name: "placement_resize_down", doc: "resize the armed window at its lower edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementNext => { name: "placement_next", doc: "select the next placement target", run: |ctx| { cycle_keyboard_tree_placement(ctx, false); } },
    PlacementPrevious => { name: "placement_previous", doc: "select the previous placement target", run: |ctx| { cycle_keyboard_tree_placement(ctx, true); } },
    PlacementCenter => { name: "placement_center", doc: "select the center replacement target", run: |ctx| { center_keyboard_tree_placement(ctx); } },
    PlacementApply => { name: "placement_apply", doc: "apply the pending tree placement", run: |ctx| { finish_keyboard_tree_placement(ctx, true); } },
    PlacementCancel => { name: "placement_cancel", doc: "cancel the pending tree placement", run: |ctx| { finish_keyboard_tree_placement(ctx, false); } },
    NextKeyboardLayout => { name: "next_keyboard_layout", overview: Preserve, doc: "cycle to next keyboard layout", run: |ctx| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, StackDirection::Next); } },
    PrevKeyboardLayout => { name: "prev_keyboard_layout", overview: Preserve, doc: "cycle to previous keyboard layout", run: |ctx| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, StackDirection::Previous); } },
    KeyboardLayout(String) => { name: "keyboard_layout", overview: Preserve, doc: "set keyboard layout, e.g. us(intl)", run: |ctx, layout| { crate::keyboard_layout::set_keyboard_layout_by_name(ctx, layout); } },
    SetMode(String) => { name: "set_mode", doc: "set WM mode (sway-like modes)", run: |ctx, mode| { validate_mode_name(&ctx.core().config().bindings.modes, mode)?; ctx.set_current_mode(mode.clone()); } },
    Spawn(Vec<String>) => { name: "spawn", overview: Preserve, doc: "spawn a command without shell expansion", run: |ctx, argv| { spawn(ctx, argv)?; } },
    SetLayout(LayoutCommand) => { name: "set_layout", doc: "set layout", run: |ctx, layout| { set_layout(ctx, *layout); } },
    ResetLayout => { name: "reset_layout", doc: "reset the active layout to stock geometry", run: |ctx| { reset_active_layout(ctx); } },
    FocusStack(StackDirection) => { name: "focus_stack", overview: Preserve, doc: "focus stack direction", run: |ctx, direction| { focus_stack(ctx, *direction); } },
    ViewTag(u32) => { name: "view_tag", overview: Cancel, doc: "view a tag by its 1-based number", run: |ctx, number| {
        let number = *number as usize;
        if number == 0 || number > ctx.core().model().tags.num_tags {
            return Err(format!("tag number {number} is out of range"));
        }
        let mask = TagMask::from_index(number - 1).ok_or_else(|| format!("tag number {number} is out of range"))?;
        crate::tags::view::view_tags(ctx, mask);
    } },
    WarpFocus => { name: "warp_focus", overview: Preserve, doc: "warp the pointer to the focused window", run: |ctx| { crate::mouse::warp::warp_to_focus(ctx); } },
    FocusMon(MonitorDirection) => { name: "focus_mon", overview: Cancel, doc: "focus another monitor, warping the pointer to it", run: |ctx, direction| { focus_monitor(ctx, *direction); } },
    SendMon(MonitorDirection) => { name: "send_mon", doc: "move the focused client to another monitor without following", run: |ctx, direction| { send_to_monitor(ctx, *direction); } },
    FollowMon(MonitorDirection) => { name: "follow_mon", doc: "move the focused client to another monitor and follow", run: |ctx, direction| { move_to_monitor_and_follow(ctx, *direction); } },
    SetBorder(Option<u32>) => { name: "set_border", doc: "set the focused window border width", run: |ctx, width| {
        let width = match width {
            Some(width) => i32::try_from(*width).map_err(|_| format!("border width {width} is too large"))?,
            None => crate::config::mod_consts::BORDER_PX,
        };
        with_selected_win(ctx, |ctx, win| ctx.set_border(win, width));
    } }
);
