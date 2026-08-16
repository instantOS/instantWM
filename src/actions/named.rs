use crate::actions::ActionMeta;
use crate::client::fullscreen::toggle_fake_fullscreen;
use crate::client::{kill_client, shut_kill, zoom};
use crate::config::ModeConfig;
use crate::contexts::WmCtx;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, center_window, distribute_clients, edge_scratchpad_create,
    key_move, key_resize, scratchpad_create, scratchpad_hide_name, scratchpad_restore,
    scratchpad_show_name, scratchpad_toggle, set_scratchpad_direction, toggle_floating,
};
use crate::focus::{direction_focus, focus_last_client, focus_stack, focus_stack_neighbor};
use crate::ipc_types::ScratchpadInitialStatus;
use crate::keyboard::{down_key, up_key};
use crate::layouts::tree::Side;
use crate::layouts::{
    LayoutCommand, MaximizedStackReorder, begin_tree_placement, center_keyboard_tree_placement,
    cycle_keyboard_tree_placement, cycle_layout_direction, finish_keyboard_tree_placement,
    focus_tree_neighbor, inc_master_count_by, reorder_maximized_stack,
    resize_keyboard_tree_placement, resize_tree, resize_tree_smart, set_layout,
    step_keyboard_tree_placement, swap_keyboard_tree_placement, swap_tree_neighbor,
    toggle_floating_presentation, toggle_tiling_maximized,
};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow};
use crate::mouse::draw_window;
use crate::tags::{
    cancel_overview, follow_view, last_view, move_client_follow_view, send_to_monitor, shift_tag,
    shift_view, toggle_overview, win_view,
};
use crate::toggles::{
    toggle_alt_tag, toggle_bar, toggle_hide_tags, toggle_mode, toggle_sticky, unhide_all,
};
use crate::types::{
    EdgeDirection, HorizontalDirection, MonitorDirection, StackDirection, TagMask, TagSelection,
    ToggleAction, VerticalDirection,
};
use crate::util::spawn;
use std::collections::HashMap;

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

        impl NamedAction {
            pub const fn name(self) -> &'static str {
                match self {
                    $(NamedAction::$variant => $name,)+
                }
            }
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

        pub fn execute_named_action(
            ctx: &mut WmCtx<'_>,
            action: NamedAction,
            args: &[String],
        ) -> Result<(), String> {
            validate_action_args(action, args)?;
            crate::overview::prepare_named_action(ctx, action);
            match action {
                $(NamedAction::$variant => {
                    let $ctx = ctx;
                    let $args = args;
                    $body;
                    Ok(())
                }),+
            }
        }
    };
}

fn expect_arg_count(name: &str, args: &[String], min: usize, max: usize) -> Result<(), String> {
    if (min..=max).contains(&args.len()) {
        Ok(())
    } else if min == max {
        Err(format!(
            "action '{name}' expects {min} argument(s), got {}",
            args.len()
        ))
    } else {
        Err(format!(
            "action '{name}' expects {min}..={max} arguments, got {}",
            args.len()
        ))
    }
}

fn parse_toggle_action(value: Option<&str>) -> Result<ToggleAction, String> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("toggle") => Ok(ToggleAction::Toggle),
        Some("on" | "true" | "1") => Ok(ToggleAction::SetTrue),
        Some("off" | "false" | "0") => Ok(ToggleAction::SetFalse),
        Some(value) => Err(format!(
            "invalid toggle action '{value}'; expected toggle, on, or off"
        )),
    }
}

fn parse_monitor_direction(value: &str) -> Result<MonitorDirection, String> {
    value
        .parse()
        .map_err(|()| format!("invalid monitor direction '{value}'; expected next or prev"))
}

fn validate_action_args(action: NamedAction, args: &[String]) -> Result<(), String> {
    use NamedAction::*;

    match action {
        Spawn if args.is_empty() => Err("action 'spawn' expects a command".to_string()),
        Spawn => Ok(()),
        IncMasterCount => {
            expect_arg_count("inc_master_count", args, 0, 1)?;
            if let Some(value) = args.first() {
                value.parse::<i32>().map_err(|_| {
                    format!("invalid master-count delta '{value}'; expected an integer")
                })?;
            }
            Ok(())
        }
        KeyboardLayout | SetMode | ModeToggle | SetLayout | FocusStack | ViewTag | FocusMon
        | TagMon | FollowMon | SetFocusFollowsMouse => expect_arg_count(action.name(), args, 1, 1),
        SetBorder => {
            expect_arg_count("set_border", args, 0, 1)?;
            if let Some(value) = args.first() {
                let width = value.parse::<i32>().map_err(|_| {
                    format!("invalid border width '{value}'; expected a non-negative integer")
                })?;
                if width < 0 {
                    return Err(format!(
                        "invalid border width '{value}'; expected a non-negative integer"
                    ));
                }
            }
            Ok(())
        }
        ToggleAltTag
        | ToggleAnimated
        | ToggleHideTags
        | ToggleBottomBar
        | ToggleFocusFollowsFloatMouse => {
            expect_arg_count(action.name(), args, 0, 1)?;
            parse_toggle_action(args.first().map(String::as_str)).map(|_| ())
        }
        _ => expect_arg_count(action.name(), args, 0, 0),
    }
}

fn validate_mode_name(
    configured_modes: &HashMap<String, ModeConfig>,
    name: &str,
) -> Result<(), String> {
    if name == crate::core_state::TREE_PLACEMENT_MODE_NAME {
        return Err("mode 'placement' can only be entered by begin_tree_placement".to_string());
    }
    if configured_modes.contains_key(name)
        || matches!(
            crate::core_state::ActiveWmMode::from_name(name),
            crate::core_state::ActiveWmMode::Default | crate::core_state::ActiveWmMode::Overview
        )
    {
        Ok(())
    } else {
        Err(format!("mode '{name}' not found"))
    }
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
    let stack_direction = match direction {
        HorizontalDirection::Left => StackDirection::Previous,
        HorizontalDirection::Right => StackDirection::Next,
    };
    match reorder_maximized_stack(ctx, stack_direction) {
        MaximizedStackReorder::Reordered | MaximizedStackReorder::ReconcileRequired => return,
        MaximizedStackReorder::Boundary => {
            let _ = move_client_follow_view(ctx, direction);
            return;
        }
        MaximizedStackReorder::NotApplicable => {}
    }

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
    if !key_move(ctx, win, direction.into()) {
        let _ = move_client_follow_view(ctx, direction);
    }
}

fn move_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    let stack_direction = match direction {
        VerticalDirection::Up => StackDirection::Previous,
        VerticalDirection::Down => StackDirection::Next,
    };
    if !matches!(
        reorder_maximized_stack(ctx, stack_direction),
        MaximizedStackReorder::NotApplicable
    ) {
        return;
    }

    let side = match direction {
        VerticalDirection::Up => Side::Top,
        VerticalDirection::Down => Side::Bottom,
    };
    if !swap_tree_neighbor(ctx, side)
        && let Some(win) = ctx.core().model().selected_win()
    {
        key_move(ctx, win, direction.into());
    }
}

define_named_actions!(
    Zoom => { name: "zoom", arg_example: None, doc: "zoom client into master area", run: |ctx, _args| { zoom(ctx); } },
    None => { name: "none", arg_example: None, doc: "explicitly unbind/ignore this key combination", run: |_ctx, _args| {} },
    Kill => { name: "kill", arg_example: None, doc: "close focused window gracefully", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { kill_client(ctx, win); } } },
    ShutKill => { name: "shut_kill", arg_example: None, doc: "force kill focused window", run: |ctx, _args| { shut_kill(ctx); } },
    Quit => { name: "quit", arg_example: None, doc: "quit instantwm", run: |ctx, _args| { ctx.quit(); } },
    FocusNext => { name: "focus_next", arg_example: None, doc: "focus next window in stack", run: |ctx, _args| { focus_stack(ctx, StackDirection::Next); } },
    FocusPrev => { name: "focus_prev", arg_example: None, doc: "focus previous window in stack", run: |ctx, _args| { focus_stack(ctx, StackDirection::Previous); } },
    FocusLast => { name: "focus_last", arg_example: None, doc: "focus last focused window", run: |ctx, _args| { focus_last_client(ctx); } },
    FocusUp => { name: "focus_up", arg_example: None, doc: "focus above; cycle backward in bar order when no window is above", run: |ctx, _args| { focus_vertical(ctx, VerticalDirection::Up); } },
    FocusDown => { name: "focus_down", arg_example: None, doc: "focus below; cycle forward in bar order when no window is below", run: |ctx, _args| { focus_vertical(ctx, VerticalDirection::Down); } },
    FocusLeft => { name: "focus_left", arg_example: None, doc: "focus left, or move backward through bar order in maximized presentation; switch tags at the boundary", run: |ctx, _args| { focus_horizontal(ctx, HorizontalDirection::Left); } },
    FocusRight => { name: "focus_right", arg_example: None, doc: "focus right, or move forward through bar order in maximized presentation; switch tags at the boundary", run: |ctx, _args| { focus_horizontal(ctx, HorizontalDirection::Right); } },
    DownKey => { name: "down_key", arg_example: None, doc: "alt-tab forward", run: |ctx, _args| { down_key(ctx, StackDirection::Next); } },
    UpKey => { name: "up_key", arg_example: None, doc: "alt-tab backward", run: |ctx, _args| { up_key(ctx, StackDirection::Previous); } },
    LayoutFloat => { name: "layout_float", arg_example: None, doc: "toggle floating layout presentation without changing per-window floating state", run: |ctx, _args| { toggle_floating_presentation(ctx); } },
    ToggleTilingMaximized => { name: "toggle_tiling_maximized", arg_example: None, doc: "toggle maximized-stack presentation, or restore manual tiling from floating layout", run: |ctx, _args| { toggle_tiling_maximized(ctx); } },
    CycleLayoutNext => { name: "cycle_layout_next", arg_example: None, doc: "cycle to next layout", run: |ctx, _args| { cycle_layout_direction(ctx, true); } },
    CycleLayoutPrev => { name: "cycle_layout_prev", arg_example: None, doc: "cycle to previous layout", run: |ctx, _args| { cycle_layout_direction(ctx, false); } },
    IncMasterCount => { name: "inc_master_count", arg_example: Some("1"), doc: "increase master window count", run: |ctx, args| { inc_master_count_by(ctx, args.first().and_then(|s| s.parse().ok()).unwrap_or(1)); } },
    CenterWindow => { name: "center_window", arg_example: None, doc: "center focused window", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { center_window(ctx, win); } } },
    DistributeClients => { name: "distribute_clients", arg_example: None, doc: "distribute windows evenly", run: |ctx, _args| { distribute_clients(ctx); } },
    KeyResizeUp => { name: "key_resize_up", arg_example: None, doc: "grow a tiled window vertically or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Top) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, VerticalDirection::Up.into()); } } },
    KeyResizeDown => { name: "key_resize_down", arg_example: None, doc: "shrink a tiled window vertically or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Bottom) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, VerticalDirection::Down.into()); } } },
    KeyResizeLeft => { name: "key_resize_left", arg_example: None, doc: "shrink a tiled window horizontally or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Left) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, HorizontalDirection::Left.into()); } } },
    KeyResizeRight => { name: "key_resize_right", arg_example: None, doc: "grow a tiled window horizontally or resize a floating window", run: |ctx, _args| { if !resize_tree(ctx, Side::Right) && let Some(win) = ctx.core().model().selected_win() { key_resize(ctx, win, HorizontalDirection::Right.into()); } } },
    KeyMoveUp => { name: "key_move_up", arg_example: None, doc: "move toward the previous maximized title, swap a tiled window upward, or move a floating window", run: |ctx, _args| { move_vertical(ctx, VerticalDirection::Up); } },
    KeyMoveDown => { name: "key_move_down", arg_example: None, doc: "move toward the next maximized title, swap a tiled window downward, or move a floating window", run: |ctx, _args| { move_vertical(ctx, VerticalDirection::Down); } },
    KeyMoveLeft => { name: "key_move_left", arg_example: None, doc: "move toward the previous maximized title or move left, carrying the window to the adjacent tag at the boundary", run: |ctx, _args| { move_horizontal(ctx, HorizontalDirection::Left); } },
    KeyMoveRight => { name: "key_move_right", arg_example: None, doc: "move toward the next maximized title or move right, carrying the window to the adjacent tag at the boundary", run: |ctx, _args| { move_horizontal(ctx, HorizontalDirection::Right); } },
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
    EdgeScratchpadToggle => { name: "edge_scratchpad_toggle", arg_example: None, doc: "toggle the default edge scratchpad", run: |ctx, _args| { scratchpad_toggle(ctx, Some(DEFAULT_EDGE_SCRATCHPAD_NAME)); } },
    EdgeScratchpadCreate => { name: "edge_scratchpad_create", arg_example: None, doc: "toggle the default edge scratchpad (create from the focused window, or restore if it exists)", run: |ctx, _args| { edge_scratchpad_create(ctx); } },
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
                let _ = scratchpad_create(ctx, DEFAULT_NAME, None, None, ScratchpadInitialStatus::Shown);
            }
        }
    },
    ScratchpadRestore => {
        name: "scratchpad_restore",
        arg_example: None,
        doc: "restore the focused scratchpad as an ordinary window",
        run: |ctx, _args| { let _ = scratchpad_restore(ctx, None, None); }
    },
    ToggleBar => { name: "toggle_bar", arg_example: None, doc: "toggle status bar", run: |ctx, _args| { toggle_bar(ctx); } },
    ToggleBottomBar => { name: "toggle_bottom_bar", arg_example: Some("[toggle|on|off]"), doc: "toggle or set bottom bar visibility", run: |ctx, args| {
        let action = parse_toggle_action(args.first().map(String::as_str))?;
        let mut shown = ctx.core().model().expect_selected_monitor().shows_bottom_bar();
        action.apply(&mut shown);
        crate::toggles::set_bottom_bar_shown(ctx, shown);
    } },
    ToggleFloating => { name: "toggle_floating", arg_example: None, doc: "toggle focused window between tiled and floating", run: |ctx, _args| { toggle_floating(ctx); } },
    ToggleSticky => { name: "toggle_sticky", arg_example: None, doc: "toggle sticky (visible on all tags)", run: |ctx, _args| { if let Some(win) = ctx.core().model().selected_win() { toggle_sticky(ctx, win); } } },
    ToggleAltTag => { name: "toggle_alt_tag", arg_example: Some("[toggle|on|off]"), doc: "toggle or set alt-tag mode", run: |ctx, args| { toggle_alt_tag(ctx, parse_toggle_action(args.first().map(String::as_str))?); } },
    ToggleAnimated => { name: "toggle_animated", arg_example: Some("[toggle|on|off]"), doc: "toggle or set window animations", run: |ctx, args| { let action = parse_toggle_action(args.first().map(String::as_str))?; ctx.with_behavior_mut(|behavior| behavior.toggle_animated(action)); } },
    ToggleHideTags => { name: "toggle_hide_tags", arg_example: Some("[toggle|on|off]"), doc: "toggle or set hiding empty tags in the bar", run: |ctx, args| { toggle_hide_tags(ctx, parse_toggle_action(args.first().map(String::as_str))?); } },
    ToggleFocusFollowsFloatMouse => { name: "toggle_focus_follows_float_mouse", arg_example: Some("[toggle|on|off]"), doc: "toggle or set focus-follows-mouse for floating windows", run: |ctx, args| { let action = parse_toggle_action(args.first().map(String::as_str))?; ctx.with_behavior_mut(|behavior| behavior.toggle_focus_follows_float_mouse(action)); } },
    SetFocusFollowsMouse => { name: "set_focus_follows_mouse", arg_example: Some("off|normal|force"), doc: "set focus-follows-mouse behavior", run: |ctx, args| {
        let mode = match args[0].to_ascii_lowercase().as_str() {
            "off" => crate::types::FocusFollowsMouseMode::Off,
            "normal" => crate::types::FocusFollowsMouseMode::Normal,
            "force" => crate::types::FocusFollowsMouseMode::Force,
            value => return Err(format!("invalid focus-follows-mouse mode '{value}'; expected off, normal, or force")),
        };
        ctx.with_behavior_mut(|behavior| behavior.set_focus_follows_mouse(mode));
    } },
    ModeToggle => { name: "mode_toggle", arg_example: Some("mode_name"), doc: "toggle a mode (enter if not active, else return to default)", run: |ctx, args| { validate_mode_name(&ctx.core().config().bindings.modes, &args[0])?; toggle_mode(ctx, &args[0]); } },
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
    SetMode => { name: "set_mode", arg_example: Some("resize"), doc: "set WM mode (sway-like modes)", run: |ctx, args| { validate_mode_name(&ctx.core().config().bindings.modes, &args[0])?; ctx.set_current_mode(args[0].clone()); } },
    Spawn => { name: "spawn", arg_example: Some("COMMAND [ARG ...]"), doc: "spawn a command without shell expansion", run: |ctx, args| { spawn(ctx, args)?; } },
    SetLayout => { name: "set_layout", arg_example: Some("tile"), doc: "set layout", run: |ctx, args| { let Some(layout) = LayoutCommand::from_name(&args[0]) else { return Err(format!("invalid layout '{}'", args[0])); }; set_layout(ctx, layout); } },
    FocusStack => { name: "focus_stack", arg_example: Some("next"), doc: "focus stack direction", run: |ctx, args| { let Some(direction) = StackDirection::from_name(&args[0]) else { return Err(format!("invalid stack direction '{}'", args[0])); }; focus_stack(ctx, direction); } },
    ViewTag => { name: "view_tag", arg_example: Some("NUMBER"), doc: "view a tag by its 1-based number", run: |ctx, args| {
        let number = args[0].parse::<usize>().map_err(|_| format!("invalid tag number '{}'", args[0]))?;
        let index = number.checked_sub(1).ok_or_else(|| "tag number must be at least 1".to_string())?;
        if number > ctx.core().model().tags.num_tags { return Err(format!("tag number {number} is out of range")); }
        let mask = TagMask::from_index(index).ok_or_else(|| format!("tag number {number} is out of range"))?;
        crate::tags::view::view_tags(ctx, mask);
    } },
    WarpFocus => { name: "warp_focus", arg_example: None, doc: "warp the pointer to the focused window", run: |ctx, _args| { crate::mouse::warp::warp_to_focus(ctx); } },
    FocusMon => { name: "focus_mon", arg_example: Some("next|prev"), doc: "focus another monitor", run: |ctx, args| { focus_monitor(ctx, parse_monitor_direction(&args[0])?); } },
    TagMon => { name: "tag_mon", arg_example: Some("next|prev"), doc: "move the focused tag view to another monitor", run: |ctx, args| { send_to_monitor(ctx, parse_monitor_direction(&args[0])?); } },
    FollowMon => { name: "follow_mon", arg_example: Some("next|prev"), doc: "move the focused client to another monitor and follow", run: |ctx, args| { move_to_monitor_and_follow(ctx, parse_monitor_direction(&args[0])?); } },
    SetBorder => { name: "set_border", arg_example: Some("[WIDTH]"), doc: "set the focused window border width", run: |ctx, args| {
        let width = args.first().map(|value| value.parse::<i32>()).transpose().map_err(|_| format!("invalid border width '{}'", args[0]))?.unwrap_or(crate::config::mod_consts::BORDER_PX);
        if let Some(win) = ctx.core().model().selected_win() { ctx.set_border(win, width); }
    } }
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
    use super::{
        NamedAction, execute_named_action, focus_vertical, move_horizontal, move_vertical,
        parse_named_action, validate_action_args,
    };
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::layouts::tree::Preset;

    use crate::layouts::{LayoutCommand, PresentationMode};
    use crate::types::{
        Client, ClientMode, HorizontalDirection, Monitor, Rect, StackDirection, TagMask,
        VerticalDirection, WindowId,
    };
    use crate::wm::Wm;

    fn maximized_tiled_wm(windows: &[WindowId], selected: WindowId) -> Wm {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 3;
        let tag = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        for &win in windows {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags: tag,
                mode: ClientMode::tiled(),
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag);
        monitor.clients = windows.to_vec();
        monitor.selected = Some(selected);
        monitor
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::Grid, windows, 1);
        monitor.per_tag_state().presentation = PresentationMode::Maximized;
        wm
    }

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
    fn action_arguments_are_validated_before_dispatch() {
        assert!(validate_action_args(NamedAction::ToggleAltTag, &[]).is_ok());
        assert!(validate_action_args(NamedAction::ToggleAltTag, &["on".to_string()]).is_ok());
        assert!(
            validate_action_args(NamedAction::ToggleAltTag, &["sometimes".to_string()]).is_err()
        );
        assert!(validate_action_args(NamedAction::FocusNext, &["unexpected".to_string()]).is_err());
        assert!(validate_action_args(NamedAction::SetLayout, &[]).is_err());
        assert!(validate_action_args(NamedAction::SetBorder, &["-1".to_string()]).is_err());
    }

    #[test]
    fn action_dispatch_reports_invalid_values_and_can_set_toggles_idempotently() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        execute_named_action(
            &mut wm.ctx(),
            NamedAction::ToggleAltTag,
            &["on".to_string()],
        )
        .unwrap();
        execute_named_action(
            &mut wm.ctx(),
            NamedAction::ToggleAltTag,
            &["on".to_string()],
        )
        .unwrap();
        assert!(wm.core.model.tags.show_alternative_names);

        let error = execute_named_action(
            &mut wm.ctx(),
            NamedAction::SetLayout,
            &["not-a-layout".to_string()],
        )
        .unwrap_err();
        assert!(error.contains("invalid layout"));
    }

    #[test]
    fn quit_action_uses_the_normal_wm_shutdown_flag() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        execute_named_action(&mut wm.ctx(), NamedAction::Quit, &[]).unwrap();
        assert!(!wm.running);
    }

    #[test]
    fn action_dispatch_rejects_unknown_and_interaction_owned_modes() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        let unknown = execute_named_action(
            &mut wm.ctx(),
            NamedAction::SetMode,
            &["does-not-exist".to_string()],
        )
        .unwrap_err();
        assert!(unknown.contains("not found"));

        let placement = execute_named_action(
            &mut wm.ctx(),
            NamedAction::SetMode,
            &[crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string()],
        )
        .unwrap_err();
        assert!(placement.contains("begin_tree_placement"));
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
                mode: ClientMode::tiled(),
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
    fn maximized_window_move_reorders_adjacent_titles_not_hidden_visual_neighbors() {
        let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
        let selected = WindowId(4);
        let mut wm = maximized_tiled_wm(&windows, selected);

        // In this grid, window 4's hidden visual neighbour to the left is
        // window 2. The exposed maximized order instead places window 3
        // immediately before it.
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .layout_tree
                .visual_neighbor(selected, crate::layouts::tree::Side::Left),
            Some(WindowId(2))
        );

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

        let monitor = wm.core.model.expect_selected_monitor();
        assert_eq!(
            monitor.per_tag().unwrap().layout_tree.leaves(),
            vec![WindowId(1), WindowId(2), WindowId(4), WindowId(3)]
        );
        assert_eq!(
            monitor.bar_client_order(&wm.core.model.clients),
            vec![WindowId(1), WindowId(2), WindowId(4), WindowId(3)]
        );
        assert_eq!(monitor.selected, Some(selected));
    }

    #[test]
    fn maximized_horizontal_move_crosses_tags_at_title_strip_boundary() {
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        let selected = WindowId(3);
        let mut wm = maximized_tiled_wm(&windows, selected);
        let tag2 = TagMask::single(2).unwrap();

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        assert_eq!(wm.core.model.client(selected).unwrap().tags, tag2);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag2
        );
        assert_eq!(wm.core.model.selected_win(), Some(selected));
    }

    #[test]
    fn maximized_vertical_move_stops_at_title_strip_boundary() {
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        let selected = WindowId(3);
        let mut wm = maximized_tiled_wm(&windows, selected);
        let tag1 = TagMask::single(1).unwrap();

        move_vertical(&mut wm.ctx(), VerticalDirection::Up);
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .layout_tree
                .leaves(),
            vec![WindowId(1), WindowId(3), WindowId(2)]
        );

        move_vertical(&mut wm.ctx(), VerticalDirection::Down);
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .layout_tree
                .leaves(),
            windows
        );

        move_vertical(&mut wm.ctx(), VerticalDirection::Down);

        let monitor = wm.core.model.expect_selected_monitor();
        assert_eq!(monitor.per_tag().unwrap().layout_tree.leaves(), windows);
        assert_eq!(monitor.selected_tags(), tag1);
        assert_eq!(monitor.selected, Some(selected));
    }

    #[test]
    fn maximized_move_does_not_treat_pending_tree_reconciliation_as_a_boundary() {
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        let selected = WindowId(3);
        let mut wm = maximized_tiled_wm(&windows, selected);
        let tag1 = TagMask::single(1).unwrap();
        assert!(
            wm.core
                .model
                .expect_selected_monitor_mut()
                .per_tag_state()
                .layout_tree
                .remove(selected)
        );

        // Title order defensively appends a newly managed tiled client before
        // the next arrange reconciles its leaf. Moving left during that window
        // must not fall through to an adjacent-tag transfer.
        move_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

        assert_eq!(wm.core.model.client(selected).unwrap().tags, tag1);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag1
        );
        assert_eq!(wm.core.model.selected_win(), Some(selected));
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
                mode: ClientMode::tiled(),
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
