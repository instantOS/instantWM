use crate::actions::ActionMeta;
use crate::client::fullscreen::toggle_fake_fullscreen;
use crate::client::{kill_client, shut_kill, zoom};
use crate::contexts::WmCtx;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, center_window, distribute_clients, edge_scratchpad_create,
    key_resize, moveresize, scratchpad_hide_name, scratchpad_make, scratchpad_show_name,
    scratchpad_toggle, set_scratchpad_direction, toggle_floating,
};
use crate::focus::{direction_focus, focus_last_client, focus_stack, focus_stack_neighbor};
use crate::ipc_types::ScratchpadInitialStatus;
use crate::keyboard::{down_key, up_key};
use crate::layouts::tree::Side;
use crate::layouts::{
    LayoutCommand, begin_tree_placement, center_keyboard_tree_placement,
    cycle_keyboard_tree_placement, cycle_layout_direction, finish_keyboard_tree_placement,
    focus_tree_neighbor, inc_master_count_by, resize_keyboard_tree_placement, resize_tree,
    resize_tree_smart, set_layout, step_keyboard_tree_placement, swap_keyboard_tree_placement,
    swap_tree_neighbor, toggle_tiling_maximized,
};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow};
use crate::mouse::draw_window;
use crate::tags::{
    cancel_overview, follow_view, last_view, move_client_follow_view, quit, shift_tag, shift_view,
    toggle_overview, win_view,
};
use crate::toggles::{
    toggle_alt_tag, toggle_bar, toggle_mode, toggle_show_tags, toggle_sticky, unhide_all,
};
use crate::types::{
    EdgeDirection, HorizontalDirection, MonitorDirection, StackDirection, TagMask, TagSelection,
    ToggleAction, VerticalDirection,
};
use crate::util::spawn;

macro_rules! define_named_actions {
    ($(
        $variant:ident => {
            name: $name:literal,
            arg_example: $arg_example:expr,
            doc: $doc:literal,
            run: |$ctx:ident, $args:ident| $body:block
        }
    ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum NamedAction {
            $($variant,)+
        }

        pub fn get_action_metadata() -> Vec<ActionMeta> {
            vec![
                $(ActionMeta { name: $name, doc: $doc, arg_example: $arg_example }),+
            ]
        }

        pub fn parse_named_action(name: &str) -> Option<NamedAction> {
            Some(match name.to_ascii_lowercase().as_str() {
                $($name => NamedAction::$variant,)+
                _ => return None,
            })
        }

        pub fn execute_named_action(ctx: &mut WmCtx<'_>, action: NamedAction, args: &[String]) {
            match action {
                $(NamedAction::$variant => {
                    let $ctx = ctx;
                    let $args = args;
                    $body
                }),+
            }
        }
    };
}

fn focus_horizontal(ctx: &mut WmCtx<'_>, direction: HorizontalDirection) {
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }
    if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout()
    {
        let stack_direction = match direction {
            HorizontalDirection::Left => StackDirection::Previous,
            HorizontalDirection::Right => StackDirection::Next,
        };
        if !focus_stack_neighbor(ctx, stack_direction) {
            crate::animation::scroll_view_with_slide(ctx, direction);
        }
        return;
    }

    let side = match direction {
        HorizontalDirection::Left => Side::Left,
        HorizontalDirection::Right => Side::Right,
    };
    if !focus_tree_neighbor(ctx, side) && !direction_focus(ctx, direction.into()) {
        crate::animation::scroll_view_with_slide(ctx, direction);
    }
}

fn focus_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }
    if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout()
    {
        let stack_direction = match direction {
            VerticalDirection::Up => StackDirection::Previous,
            VerticalDirection::Down => StackDirection::Next,
        };
        focus_stack(ctx, stack_direction);
        return;
    }

    let side = match direction {
        VerticalDirection::Up => Side::Top,
        VerticalDirection::Down => Side::Bottom,
    };
    if !focus_tree_neighbor(ctx, side) && !direction_focus(ctx, direction.into()) {
        let stack_direction = match direction {
            VerticalDirection::Up => StackDirection::Previous,
            VerticalDirection::Down => StackDirection::Next,
        };
        focus_stack(ctx, stack_direction);
    }
}

fn move_horizontal(ctx: &mut WmCtx<'_>, direction: HorizontalDirection) {
    let side = match direction {
        HorizontalDirection::Left => Side::Left,
        HorizontalDirection::Right => Side::Right,
    };
    if swap_tree_neighbor(ctx, side) {
        return;
    }
    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };
    if !moveresize(ctx, win, direction.into()) {
        let _ = move_client_follow_view(ctx, direction);
    }
}

define_named_actions!(
    Zoom => { name: "zoom", arg_example: None, doc: "zoom client into master area", run: |ctx, _args| { zoom(ctx); } },
    None => { name: "none", arg_example: None, doc: "explicitly unbind/ignore this key combination", run: |_ctx, _args| {} },
    Kill => { name: "kill", arg_example: None, doc: "close focused window gracefully", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { kill_client(ctx, win); } } },
    ShutKill => { name: "shut_kill", arg_example: None, doc: "force kill focused window", run: |ctx, _args| { shut_kill(ctx); } },
    Quit => { name: "quit", arg_example: None, doc: "quit instantwm", run: |_ctx, _args| { quit(); } },
    FocusNext => { name: "focus_next", arg_example: None, doc: "focus next window in stack", run: |ctx, _args| { focus_stack(ctx, StackDirection::Next); } },
    FocusPrev => { name: "focus_prev", arg_example: None, doc: "focus previous window in stack", run: |ctx, _args| { focus_stack(ctx, StackDirection::Previous); } },
    FocusLast => { name: "focus_last", arg_example: None, doc: "focus last focused window", run: |ctx, _args| { focus_last_client(ctx); } },
    FocusUp => { name: "focus_up", arg_example: None, doc: "focus above; cycle backward in bar order when no window is above", run: |ctx, _args| { focus_vertical(ctx, VerticalDirection::Up); } },
    FocusDown => { name: "focus_down", arg_example: None, doc: "focus below; cycle forward in bar order when no window is below", run: |ctx, _args| { focus_vertical(ctx, VerticalDirection::Down); } },
    FocusLeft => { name: "focus_left", arg_example: None, doc: "focus left, or move backward through bar order in maximized presentation; switch tags at the boundary", run: |ctx, _args| { focus_horizontal(ctx, HorizontalDirection::Left); } },
    FocusRight => { name: "focus_right", arg_example: None, doc: "focus right, or move forward through bar order in maximized presentation; switch tags at the boundary", run: |ctx, _args| { focus_horizontal(ctx, HorizontalDirection::Right); } },
    DownKey => { name: "down_key", arg_example: None, doc: "alt-tab forward", run: |ctx, _args| { down_key(ctx, StackDirection::Next); } },
    UpKey => { name: "up_key", arg_example: None, doc: "alt-tab backward", run: |ctx, _args| { up_key(ctx, StackDirection::Previous); } },
    LayoutTile => { name: "layout_tile", arg_example: None, doc: "rewrite the manual tree as master-stack", run: |ctx, _args| { set_layout(ctx, LayoutCommand::Tile); } },
    LayoutFloat => { name: "layout_float", arg_example: None, doc: "set floating layout", run: |ctx, _args| { set_layout(ctx, LayoutCommand::Floating); } },
    LayoutMaximized => { name: "layout_maximized", arg_example: None, doc: "set maximized-stack presentation without changing the manual tree", run: |ctx, _args| { set_layout(ctx, LayoutCommand::Maximized); } },
    ToggleTilingMaximized => { name: "toggle_tiling_maximized", arg_example: None, doc: "toggle between manual tiling and maximized-stack presentation without changing the tree", run: |ctx, _args| { toggle_tiling_maximized(ctx); } },
    LayoutGrid => { name: "layout_grid", arg_example: None, doc: "rewrite the manual tree as a grid", run: |ctx, _args| { set_layout(ctx, LayoutCommand::Grid); } },
    LayoutBottomStack => { name: "layout_bottom_stack", arg_example: None, doc: "set bottom-stack layout", run: |ctx, _args| { set_layout(ctx, LayoutCommand::BottomStack); } },
    LayoutHorizGrid => { name: "layout_horiz_grid", arg_example: None, doc: "set horiz-grid layout", run: |ctx, _args| { set_layout(ctx, LayoutCommand::HorizGrid); } },
    LayoutBStackHoriz => { name: "layout_bstack_horiz", arg_example: None, doc: "set bstack-horiz layout", run: |ctx, _args| { set_layout(ctx, LayoutCommand::BStackHoriz); } },
    CycleLayoutNext => { name: "cycle_layout_next", arg_example: None, doc: "cycle to next layout", run: |ctx, _args| { cycle_layout_direction(ctx, true); } },
    CycleLayoutPrev => { name: "cycle_layout_prev", arg_example: None, doc: "cycle to previous layout", run: |ctx, _args| { cycle_layout_direction(ctx, false); } },
    IncMasterCount => { name: "inc_master_count", arg_example: Some("1"), doc: "increase master window count", run: |ctx, args| { inc_master_count_by(ctx, args.first().and_then(|s| s.parse().ok()).unwrap_or(1)); } },
    DecMasterCount => { name: "dec_master_count", arg_example: None, doc: "decrease master window count", run: |ctx, _args| { inc_master_count_by(ctx, -1); } },
    CenterWindow => { name: "center_window", arg_example: None, doc: "center focused window", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { center_window(ctx, win); } } },
    DistributeClients => { name: "distribute_clients", arg_example: None, doc: "distribute windows evenly", run: |ctx, _args| { distribute_clients(ctx); } },
    KeyResizeUp => { name: "key_resize_up", arg_example: None, doc: "grow a tiled window vertically or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Top) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, VerticalDirection::Up.into()); } } },
    KeyResizeDown => { name: "key_resize_down", arg_example: None, doc: "shrink a tiled window vertically or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Bottom) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, VerticalDirection::Down.into()); } } },
    KeyResizeLeft => { name: "key_resize_left", arg_example: None, doc: "shrink a tiled window horizontally or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Left) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, HorizontalDirection::Left.into()); } } },
    KeyResizeRight => { name: "key_resize_right", arg_example: None, doc: "grow a tiled window horizontally or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Right) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, HorizontalDirection::Right.into()); } } },
    KeyMoveUp => { name: "key_move_up", arg_example: None, doc: "swap a tiled window upward or move a floating window", run: |ctx, _args| { if !swap_tree_neighbor(ctx, Side::Top) && let Some(win) = ctx.core().model().selected_win() { moveresize(ctx, win, VerticalDirection::Up.into()); } } },
    KeyMoveDown => { name: "key_move_down", arg_example: None, doc: "swap a tiled window downward or move a floating window", run: |ctx, _args| { if !swap_tree_neighbor(ctx, Side::Bottom) && let Some(win) = ctx.core().model().selected_win() { moveresize(ctx, win, VerticalDirection::Down.into()); } } },
    KeyMoveLeft => { name: "key_move_left", arg_example: None, doc: "move a window left, carrying it to the adjacent tag at the screen edge", run: |ctx, _args| { move_horizontal(ctx, HorizontalDirection::Left); } },
    KeyMoveRight => { name: "key_move_right", arg_example: None, doc: "move a window right, carrying it to the adjacent tag at the screen edge", run: |ctx, _args| { move_horizontal(ctx, HorizontalDirection::Right); } },
    TreeGrow => { name: "tree_grow", arg_example: None, doc: "grow the focused window along its most local split", run: |ctx, _args| { resize_tree_smart(ctx, true); } },
    TreeShrink => { name: "tree_shrink", arg_example: None, doc: "shrink the focused window along its most local split", run: |ctx, _args| { resize_tree_smart(ctx, false); } },
    PushUp => { name: "push_up", arg_example: None, doc: "swap a tiled window upward (legacy action)", run: |ctx, _args| { swap_tree_neighbor(ctx, Side::Top); } },
    PushDown => { name: "push_down", arg_example: None, doc: "swap a tiled window downward (legacy action)", run: |ctx, _args| { swap_tree_neighbor(ctx, Side::Bottom); } },
    LastView => { name: "last_view", arg_example: None, doc: "view previously viewed tags", run: |ctx, _args| { last_view(ctx); } },
    FollowView => { name: "follow_view", arg_example: None, doc: "follow client to its tags", run: |ctx, _args| { follow_view(ctx); } },
    WinView => { name: "win_view", arg_example: None, doc: "view tags of focused client", run: |ctx, _args| { win_view(ctx); } },
    ScrollLeft => { name: "scroll_left", arg_example: None, doc: "scroll tags left", run: |ctx, _args| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Left); } },
    ScrollRight => { name: "scroll_right", arg_example: None, doc: "scroll tags right", run: |ctx, _args| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Right); } },
    MoveClientLeft => { name: "move_client_left", arg_example: None, doc: "move client to tag on left", run: |ctx, _args| { move_client_follow_view(ctx, HorizontalDirection::Left); } },
    MoveClientRight => { name: "move_client_right", arg_example: None, doc: "move client to tag on right", run: |ctx, _args| { move_client_follow_view(ctx, HorizontalDirection::Right); } },
    ShiftTagLeft => { name: "shift_tag_left", arg_example: None, doc: "shift client to tag on left", run: |ctx, _args| { shift_tag(ctx, HorizontalDirection::Left.into(), 1); } },
    ShiftTagRight => { name: "shift_tag_right", arg_example: None, doc: "shift client to tag on right", run: |ctx, _args| { shift_tag(ctx, HorizontalDirection::Right.into(), 1); } },
    ShiftViewLeft => { name: "shift_view_left", arg_example: None, doc: "shift view to tag on left", run: |ctx, _args| { shift_view(ctx, HorizontalDirection::Left); } },
    ShiftViewRight => { name: "shift_view_right", arg_example: None, doc: "shift view to tag on right", run: |ctx, _args| { shift_view(ctx, HorizontalDirection::Right); } },
    ViewAll => { name: "view_all", arg_example: None, doc: "view all tags", run: |ctx, _args| { crate::tags::tag_ops::view_selection(ctx, TagSelection::All); } },
    TagAll => { name: "tag_all", arg_example: None, doc: "tag client with all tags", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { crate::tags::client_tags::set_client_tag(ctx, win, TagMask::ALL_BITS); } } },
    ToggleOverview => { name: "toggle_overview", arg_example: None, doc: "toggle overview mode", run: |ctx, _args| { toggle_overview(ctx, TagMask::ALL_BITS); } },
    CancelOverview => { name: "cancel_overview", arg_example: None, doc: "leave overview and restore previous view", run: |ctx, _args| { cancel_overview(ctx, TagMask::ALL_BITS); } },
    FocusMonPrev => { name: "focus_mon_prev", arg_example: None, doc: "focus previous monitor", run: |ctx, _args| { focus_monitor(ctx, MonitorDirection::PREV); } },
    FocusMonNext => { name: "focus_mon_next", arg_example: None, doc: "focus next monitor", run: |ctx, _args| { focus_monitor(ctx, MonitorDirection::NEXT); } },
    FollowMonPrev => { name: "follow_mon_prev", arg_example: None, doc: "move client to prev monitor and follow", run: |ctx, _args| { move_to_monitor_and_follow(ctx, MonitorDirection::PREV); } },
    FollowMonNext => { name: "follow_mon_next", arg_example: None, doc: "move client to next monitor and follow", run: |ctx, _args| { move_to_monitor_and_follow(ctx, MonitorDirection::NEXT); } },
    EdgeScratchpadToggle => { name: "edge_scratchpad_toggle", arg_example: None, doc: "toggle the default edge scratchpad", run: |ctx, _args| { scratchpad_toggle(ctx, Some(DEFAULT_EDGE_SCRATCHPAD_NAME)); } },
    EdgeScratchpadCreate => { name: "edge_scratchpad_create", arg_example: None, doc: "create the default edge scratchpad from the focused window", run: |ctx, _args| { edge_scratchpad_create(ctx); } },
    EdgeScratchpadShow => { name: "edge_scratchpad_show", arg_example: None, doc: "show the default edge scratchpad", run: |ctx, _args| { let _ = scratchpad_show_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME); } },
    EdgeScratchpadHide => { name: "edge_scratchpad_hide", arg_example: None, doc: "hide the default edge scratchpad", run: |ctx, _args| { scratchpad_hide_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME); } },
    EdgeScratchpadDirectionUp => { name: "edge_scratchpad_direction_up", arg_example: None, doc: "set default edge scratchpad direction to top", run: |ctx, _args| { edge_scratchpad_set_direction(ctx, EdgeDirection::Top); } },
    EdgeScratchpadDirectionDown => { name: "edge_scratchpad_direction_down", arg_example: None, doc: "set default edge scratchpad direction to bottom", run: |ctx, _args| { edge_scratchpad_set_direction(ctx, EdgeDirection::Bottom); } },
    EdgeScratchpadDirectionLeft => { name: "edge_scratchpad_direction_left", arg_example: None, doc: "set default edge scratchpad direction to left", run: |ctx, _args| { edge_scratchpad_set_direction(ctx, EdgeDirection::Left); } },
    EdgeScratchpadDirectionRight => { name: "edge_scratchpad_direction_right", arg_example: None, doc: "set default edge scratchpad direction to right", run: |ctx, _args| { edge_scratchpad_set_direction(ctx, EdgeDirection::Right); } },
    ScratchpadToggle => {
        name: "scratchpad_toggle",
        arg_example: None,
        doc: "toggle scratchpad, creating it from current window if it doesn't exist",
        run: |ctx, _args| {
            const DEFAULT_NAME: &str = "instantwm_scratchpad";
            if ctx.core().model().scratchpad_find(DEFAULT_NAME).is_some() {
                scratchpad_toggle(ctx, Some(DEFAULT_NAME));
            } else {
                scratchpad_make(ctx, DEFAULT_NAME, None, None, ScratchpadInitialStatus::Shown);
            }
        }
    },
    ToggleBar => { name: "toggle_bar", arg_example: None, doc: "toggle status bar", run: |ctx, _args| { toggle_bar(ctx); } },
    ToggleFloating => { name: "toggle_floating", arg_example: None, doc: "toggle focused window between tiled and floating", run: |ctx, _args| { toggle_floating(ctx); } },
    ToggleSticky => { name: "toggle_sticky", arg_example: None, doc: "toggle sticky (visible on all tags)", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { toggle_sticky(ctx, win); } } },
    ToggleAltTag => { name: "toggle_alt_tag", arg_example: None, doc: "toggle alt-tag mode", run: |ctx, _args| { toggle_alt_tag(ctx, ToggleAction::Toggle); } },
    ToggleAnimated => { name: "toggle_animated", arg_example: None, doc: "toggle window animations", run: |ctx, _args| { ctx.with_behavior_mut(|behavior| behavior.toggle_animated(ToggleAction::Toggle)); } },
    ToggleShowTags => { name: "toggle_show_tags", arg_example: None, doc: "show/hide tag bar", run: |ctx, _args| { toggle_show_tags(ctx, ToggleAction::Toggle); } },
    ModeToggle => { name: "mode_toggle", arg_example: Some("mode_name"), doc: "toggle a mode (enter if not active, else return to default)", run: |ctx, args| { if let Some(name) = args.first() { toggle_mode(ctx, name); } } },
    TogglePrefix => { name: "toggle_prefix", arg_example: None, doc: "toggle prefix mode (legacy alias for mode_toggle prefix)", run: |ctx, _args| { toggle_mode(ctx, "prefix"); } },
    UnhideAll => { name: "unhide_all", arg_example: None, doc: "show all hidden windows", run: |ctx, _args| { unhide_all(ctx); } },
    Hide => { name: "hide", arg_example: None, doc: "minimize focused window or hide the visible scratchpad", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { crate::client::hide_for_user(ctx, win); } } },
    ToggleFakeFullscreen => { name: "toggle_fake_fullscreen", arg_example: None, doc: "toggle fake fullscreen", run: |ctx, _args| { toggle_fake_fullscreen(ctx); } },
    DrawWindow => { name: "draw_window", arg_example: None, doc: "start dragging/resizing window", run: |ctx, _args| { draw_window(ctx); } },
    BeginTreePlacement => { name: "begin_tree_placement", arg_example: None, doc: "place the focused tiled window within its layout tree", run: |ctx, _args| { let _ = begin_tree_placement(ctx); } },
    PlacementLeft => { name: "placement_left", arg_example: None, doc: "select the placement target to the left", run: |ctx, _args| { step_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementRight => { name: "placement_right", arg_example: None, doc: "select the placement target to the right", run: |ctx, _args| { step_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementUp => { name: "placement_up", arg_example: None, doc: "select the placement target above", run: |ctx, _args| { step_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementDown => { name: "placement_down", arg_example: None, doc: "select the placement target below", run: |ctx, _args| { step_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementSwapLeft => { name: "placement_swap_left", arg_example: None, doc: "swap the armed window with its left neighbour", run: |ctx, _args| { swap_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementSwapRight => { name: "placement_swap_right", arg_example: None, doc: "swap the armed window with its right neighbour", run: |ctx, _args| { swap_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementSwapUp => { name: "placement_swap_up", arg_example: None, doc: "swap the armed window with its upper neighbour", run: |ctx, _args| { swap_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementSwapDown => { name: "placement_swap_down", arg_example: None, doc: "swap the armed window with its lower neighbour", run: |ctx, _args| { swap_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementResizeLeft => { name: "placement_resize_left", arg_example: None, doc: "resize the armed window at its left edge", run: |ctx, _args| { resize_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementResizeRight => { name: "placement_resize_right", arg_example: None, doc: "resize the armed window at its right edge", run: |ctx, _args| { resize_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementResizeUp => { name: "placement_resize_up", arg_example: None, doc: "resize the armed window at its upper edge", run: |ctx, _args| { resize_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementResizeDown => { name: "placement_resize_down", arg_example: None, doc: "resize the armed window at its lower edge", run: |ctx, _args| { resize_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementNext => { name: "placement_next", arg_example: None, doc: "select the next placement target", run: |ctx, _args| { cycle_keyboard_tree_placement(ctx, false); } },
    PlacementPrevious => { name: "placement_previous", arg_example: None, doc: "select the previous placement target", run: |ctx, _args| { cycle_keyboard_tree_placement(ctx, true); } },
    PlacementCenter => { name: "placement_center", arg_example: None, doc: "select the center replacement target", run: |ctx, _args| { center_keyboard_tree_placement(ctx); } },
    PlacementApply => { name: "placement_apply", arg_example: None, doc: "apply the pending tree placement", run: |ctx, _args| { finish_keyboard_tree_placement(ctx, true); } },
    PlacementCancel => { name: "placement_cancel", arg_example: None, doc: "cancel the pending tree placement", run: |ctx, _args| { finish_keyboard_tree_placement(ctx, false); } },
    NextKeyboardLayout => { name: "next_keyboard_layout", arg_example: None, doc: "cycle to next keyboard layout", run: |ctx, _args| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, StackDirection::Next); } },
    PrevKeyboardLayout => { name: "prev_keyboard_layout", arg_example: None, doc: "cycle to previous keyboard layout", run: |ctx, _args| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, StackDirection::Previous); } },
    KeyboardLayout => { name: "keyboard_layout", arg_example: Some("us(intl)"), doc: "set keyboard layout", run: |ctx, args| { if let Some(name) = args.first() { crate::keyboard_layout::set_keyboard_layout_by_name(ctx, name); } } },
    SetMode => { name: "set_mode", arg_example: Some("resize"), doc: "set WM mode (sway-like modes)", run: |ctx, args| { if let Some(name) = args.first() && name != crate::core_state::TREE_PLACEMENT_MODE_NAME { ctx.set_current_mode(name.clone()); } } },
    Spawn => { name: "spawn", arg_example: Some("kitty"), doc: "spawn command", run: |ctx, args| { spawn(ctx, args); } },
    SetLayout => { name: "set_layout", arg_example: Some("tile"), doc: "set layout", run: |ctx, args| { if let Some(name) = args.first().and_then(|s| LayoutCommand::from_name(s)) { set_layout(ctx, name); } } },
    FocusStack => { name: "focus_stack", arg_example: Some("next"), doc: "focus stack direction", run: |ctx, args| { if let Some(direction) = args.first().and_then(|s| StackDirection::from_name(s)) { focus_stack(ctx, direction); } } }
);

fn edge_scratchpad_set_direction(ctx: &mut WmCtx, dir: EdgeDirection) {
    if let Some(win) = ctx
        .core()
        .model()
        .scratchpad_find(DEFAULT_EDGE_SCRATCHPAD_NAME)
    {
        set_scratchpad_direction(ctx, win, dir);
    }
}

#[cfg(test)]
mod tests {
    use super::{NamedAction, focus_vertical, move_horizontal, parse_named_action};
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::layouts::LayoutCommand;
    use crate::layouts::tree::Preset;
    use crate::types::{
        Client, ClientMode, HorizontalDirection, Monitor, Rect, StackDirection, TagMask,
        VerticalDirection, WindowId,
    };
    use crate::wm::Wm;

    #[test]
    fn layout_command_from_name_accepts_only_canonical_names() {
        assert_eq!(LayoutCommand::from_name("tile"), Some(LayoutCommand::Tile));
        assert_eq!(
            LayoutCommand::from_name("floating"),
            Some(LayoutCommand::Floating)
        );
        assert_eq!(
            LayoutCommand::from_name("horiz-grid"),
            Some(LayoutCommand::HorizGrid)
        );
        assert_eq!(
            LayoutCommand::from_name("bstack-horiz"),
            Some(LayoutCommand::BStackHoriz)
        );
        assert_eq!(
            LayoutCommand::from_name("maximized"),
            Some(LayoutCommand::Maximized)
        );
        for alias in ["tiling", "float", "monocle", "deck", "gaplessgrid"] {
            assert_eq!(LayoutCommand::from_name(alias), None);
        }
        assert_eq!(LayoutCommand::from_name("bad"), None);
    }

    #[test]
    fn stack_direction_from_name_accepts_aliases() {
        assert_eq!(
            StackDirection::from_name("next"),
            Some(StackDirection::Next)
        );
        assert_eq!(
            StackDirection::from_name("backward"),
            Some(StackDirection::Previous)
        );
        assert_eq!(StackDirection::from_name("bad"), None);
    }

    #[test]
    fn edge_scratchpad_actions_replace_legacy_overlay_actions() {
        assert_eq!(
            parse_named_action("edge_scratchpad_toggle"),
            Some(NamedAction::EdgeScratchpadToggle)
        );
        assert_eq!(
            parse_named_action("edge_scratchpad_direction_left"),
            Some(NamedAction::EdgeScratchpadDirectionLeft)
        );
        assert_eq!(parse_named_action("overlay_toggle"), None);
        assert_eq!(parse_named_action("overlay_direction_left"), None);
    }

    #[test]
    fn tiling_maximized_toggle_has_an_explicit_presentation_name() {
        assert_eq!(
            parse_named_action("toggle_tiling_maximized"),
            Some(NamedAction::ToggleTilingMaximized)
        );
        assert_eq!(parse_named_action("toggle_maximized_layout"), None);
    }

    #[test]
    fn tree_placement_action_does_not_alias_legacy_pointer_move() {
        assert_eq!(
            parse_named_action("begin_tree_placement"),
            Some(NamedAction::BeginTreePlacement)
        );
        assert_eq!(parse_named_action("begin_keyboard_move"), None);
    }

    #[test]
    fn horizontal_window_move_crosses_tags_only_at_the_tree_edge() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 3;
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);

        let left = WindowId(1);
        let right = WindowId(2);
        for win in [left, right] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags: tag1,
                mode: ClientMode::Tiling,
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag1);
        monitor.clients = vec![left, right];
        monitor.selected = Some(left);
        monitor
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::MasterStack, &[left, right], 1);

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        // The first press has a visual neighbour, so it only swaps the tree.
        assert_eq!(wm.core.model.client(left).unwrap().tags, tag1);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag1
        );

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        // The same client is now at the right edge, so the next press carries
        // it into the adjacent tag and follows it there.
        assert_eq!(wm.core.model.client(left).unwrap().tags, tag2);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag2
        );
        assert_eq!(wm.core.model.selected_win(), Some(left));
    }

    #[test]
    fn vertical_focus_falls_back_to_cycling_in_bar_order() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tag = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);

        let left = WindowId(1);
        let middle = WindowId(2);
        let right = WindowId(3);
        for win in [left, middle, right] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags: tag,
                mode: ClientMode::Tiling,
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag);
        monitor.clients = vec![left, middle, right];
        monitor.selected = Some(middle);
        monitor.per_tag_state().layout_tree.apply_preset(
            Preset::BottomStack,
            &[left, middle, right],
            0,
        );

        focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
        assert_eq!(wm.core.model.selected_win(), Some(right));

        focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
        assert_eq!(wm.core.model.selected_win(), Some(left));

        focus_vertical(&mut wm.ctx(), VerticalDirection::Up);
        assert_eq!(wm.core.model.selected_win(), Some(right));
    }
}
