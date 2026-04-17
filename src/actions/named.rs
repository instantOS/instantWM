use crate::actions::ActionMeta;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen_x11, zoom};
use crate::contexts::WmCtx;
use crate::floating::{
    OVERLAY_NAME, center_window, distribute_clients, key_resize, overlay_create, overlay_toggle,
    scratchpad_find, scratchpad_make, scratchpad_toggle, set_scratchpad_direction, toggle_floating,
    toggle_maximized,
};
use crate::focus::{direction_focus, focus_last_client, focus_stack};
use crate::ipc_types::ScratchpadInitialStatus;
use crate::keyboard::{down_key, up_key};
use crate::layouts::{
    LayoutKind, cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, toggle_layout,
};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow, reorder_client};
use crate::mouse::{begin_keyboard_move, draw_window};
use crate::tags::{
    follow_view, last_view, move_client, quit, shift_tag, shift_view, toggle_fullscreen_overview,
    toggle_overview, win_view,
};
use crate::toggles::{
    toggle_alt_tag, toggle_animated, toggle_bar, toggle_double_draw, toggle_mode, toggle_show_tags,
    toggle_sticky, unhide_all,
};
use crate::types::{
    EdgeDirection, HorizontalDirection, MonitorDirection, StackDirection, TagMask, ToggleAction,
    VerticalDirection,
};
use crate::util::spawn;

fn parse_layout_kind_name(name: &str) -> Option<LayoutKind> {
    Some(match name.to_ascii_lowercase().as_str() {
        "tile" | "tiling" => LayoutKind::Tile,
        "float" | "floating" => LayoutKind::Floating,
        "monocle" => LayoutKind::Monocle,
        "grid" => LayoutKind::Grid,
        _ => return None,
    })
}

fn parse_stack_direction_name(name: &str) -> Option<StackDirection> {
    Some(match name.to_ascii_lowercase().as_str() {
        "next" | "down" | "forward" => StackDirection::Next,
        "prev" | "previous" | "up" | "backward" => StackDirection::Previous,
        _ => return None,
    })
}

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

define_named_actions!(
    Zoom => { name: "zoom", arg_example: None, doc: "zoom client into master area", run: |ctx, _args| { zoom(ctx); } },
    None => { name: "none", arg_example: None, doc: "explicitly unbind/ignore this key combination", run: |_ctx, _args| {} },
    Kill => { name: "kill", arg_example: None, doc: "close focused window gracefully", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { kill_client(ctx, win); } } },
    ShutKill => { name: "shut_kill", arg_example: None, doc: "force kill focused window", run: |ctx, _args| { shut_kill(ctx); } },
    Quit => { name: "quit", arg_example: None, doc: "quit instantwm", run: |_ctx, _args| { quit(); } },
    FocusNext => { name: "focus_next", arg_example: None, doc: "focus next window in stack", run: |ctx, _args| { focus_stack(ctx, StackDirection::Next); } },
    FocusPrev => { name: "focus_prev", arg_example: None, doc: "focus previous window in stack", run: |ctx, _args| { focus_stack(ctx, StackDirection::Previous); } },
    FocusLast => { name: "focus_last", arg_example: None, doc: "focus last focused window", run: |ctx, _args| { focus_last_client(ctx); } },
    FocusUp => { name: "focus_up", arg_example: None, doc: "focus window above", run: |ctx, _args| { direction_focus(ctx, VerticalDirection::Up.into()); } },
    FocusDown => { name: "focus_down", arg_example: None, doc: "focus window below", run: |ctx, _args| { direction_focus(ctx, VerticalDirection::Down.into()); } },
    FocusLeft => { name: "focus_left", arg_example: None, doc: "focus window to left", run: |ctx, _args| { direction_focus(ctx, HorizontalDirection::Left.into()); } },
    FocusRight => { name: "focus_right", arg_example: None, doc: "focus window to right", run: |ctx, _args| { direction_focus(ctx, HorizontalDirection::Right.into()); } },
    DownKey => { name: "down_key", arg_example: None, doc: "alt-tab forward", run: |ctx, _args| { down_key(ctx, StackDirection::Next); } },
    UpKey => { name: "up_key", arg_example: None, doc: "alt-tab backward", run: |ctx, _args| { up_key(ctx, StackDirection::Previous); } },
    ToggleLayout => { name: "toggle_layout", arg_example: None, doc: "toggle layout", run: |ctx, _args| { toggle_layout(ctx); } },
    LayoutTile => { name: "layout_tile", arg_example: None, doc: "set tile layout", run: |ctx, _args| { set_layout(ctx, LayoutKind::Tile); } },
    LayoutFloat => { name: "layout_float", arg_example: None, doc: "set floating layout", run: |ctx, _args| { set_layout(ctx, LayoutKind::Floating); } },
    LayoutMonocle => { name: "layout_monocle", arg_example: None, doc: "set monocle layout (fullscreen)", run: |ctx, _args| { set_layout(ctx, LayoutKind::Monocle); } },
    LayoutGrid => { name: "layout_grid", arg_example: None, doc: "set grid layout", run: |ctx, _args| { set_layout(ctx, LayoutKind::Grid); } },
    CycleLayoutNext => { name: "cycle_layout_next", arg_example: None, doc: "cycle to next layout", run: |ctx, _args| { cycle_layout_direction(ctx, true); } },
    CycleLayoutPrev => { name: "cycle_layout_prev", arg_example: None, doc: "cycle to previous layout", run: |ctx, _args| { cycle_layout_direction(ctx, false); } },
    IncNmaster => { name: "inc_nmaster", arg_example: Some("1"), doc: "increase master window count", run: |ctx, args| { inc_nmaster_by(ctx, args.first().and_then(|s| s.parse().ok()).unwrap_or(1)); } },
    DecNmaster => { name: "dec_nmaster", arg_example: None, doc: "decrease master window count", run: |ctx, _args| { inc_nmaster_by(ctx, -1); } },
    MfactGrow => { name: "mfact_grow", arg_example: None, doc: "increase master area width", run: |ctx, _args| { set_mfact(ctx, 0.05); } },
    MfactShrink => { name: "mfact_shrink", arg_example: None, doc: "decrease master area width", run: |ctx, _args| { set_mfact(ctx, -0.05); } },
    SetMfact => { name: "set_mfact", arg_example: Some("0.05"), doc: "set master factor", run: |ctx, args| { if let Some(delta) = args.first().and_then(|s| s.parse::<f32>().ok()) { set_mfact(ctx, delta); } } },
    CenterWindow => { name: "center_window", arg_example: None, doc: "center focused window", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { center_window(ctx, win); } } },
    ToggleMaximized => { name: "toggle_maximized", arg_example: None, doc: "toggle maximized state", run: |ctx, _args| { toggle_maximized(ctx); } },
    DistributeClients => { name: "distribute_clients", arg_example: None, doc: "distribute windows evenly", run: |ctx, _args| { distribute_clients(ctx); } },
    KeyResizeUp => { name: "key_resize_up", arg_example: None, doc: "resize floating window up", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { key_resize(ctx, win, VerticalDirection::Up.into()); } } },
    KeyResizeDown => { name: "key_resize_down", arg_example: None, doc: "resize floating window down", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { key_resize(ctx, win, VerticalDirection::Down.into()); } } },
    KeyResizeLeft => { name: "key_resize_left", arg_example: None, doc: "resize floating window left", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { key_resize(ctx, win, HorizontalDirection::Left.into()); } } },
    KeyResizeRight => { name: "key_resize_right", arg_example: None, doc: "resize floating window right", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { key_resize(ctx, win, HorizontalDirection::Right.into()); } } },
    PushUp => { name: "push_up", arg_example: None, doc: "push window up in stack", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { reorder_client(ctx, win, VerticalDirection::Up); } } },
    PushDown => { name: "push_down", arg_example: None, doc: "push window down in stack", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { reorder_client(ctx, win, VerticalDirection::Down); } } },
    LastView => { name: "last_view", arg_example: None, doc: "view previously viewed tags", run: |ctx, _args| { last_view(ctx); } },
    FollowView => { name: "follow_view", arg_example: None, doc: "follow client to its tags", run: |ctx, _args| { follow_view(ctx); } },
    WinView => { name: "win_view", arg_example: None, doc: "view tags of focused client", run: |ctx, _args| { win_view(ctx); } },
    ScrollLeft => { name: "scroll_left", arg_example: None, doc: "scroll tags left", run: |ctx, _args| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Left); } },
    ScrollRight => { name: "scroll_right", arg_example: None, doc: "scroll tags right", run: |ctx, _args| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Right); } },
    MoveClientLeft => { name: "move_client_left", arg_example: None, doc: "move client to tag on left", run: |ctx, _args| { move_client(ctx, HorizontalDirection::Left); } },
    MoveClientRight => { name: "move_client_right", arg_example: None, doc: "move client to tag on right", run: |ctx, _args| { move_client(ctx, HorizontalDirection::Right); } },
    ShiftTagLeft => { name: "shift_tag_left", arg_example: None, doc: "shift client to tag on left", run: |ctx, _args| { shift_tag(ctx, HorizontalDirection::Left.into(), 1); } },
    ShiftTagRight => { name: "shift_tag_right", arg_example: None, doc: "shift client to tag on right", run: |ctx, _args| { shift_tag(ctx, HorizontalDirection::Right.into(), 1); } },
    ShiftViewLeft => { name: "shift_view_left", arg_example: None, doc: "shift view to tag on left", run: |ctx, _args| { shift_view(ctx, HorizontalDirection::Left); } },
    ShiftViewRight => { name: "shift_view_right", arg_example: None, doc: "shift view to tag on right", run: |ctx, _args| { shift_view(ctx, HorizontalDirection::Right); } },
    ViewAll => { name: "view_all", arg_example: None, doc: "view all tags", run: |ctx, _args| { crate::tags::view::view_tags(ctx, TagMask::ALL_BITS); } },
    TagAll => { name: "tag_all", arg_example: None, doc: "tag client with all tags", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { crate::tags::client_tags::set_client_tag(ctx, win, TagMask::ALL_BITS); } } },
    ToggleOverview => { name: "toggle_overview", arg_example: None, doc: "toggle overview mode", run: |ctx, _args| { toggle_overview(ctx, TagMask::ALL_BITS); } },
    ToggleFullscreenOverview => { name: "toggle_fullscreen_overview", arg_example: None, doc: "toggle fullscreen overview", run: |ctx, _args| { toggle_fullscreen_overview(ctx, TagMask::ALL_BITS); } },
    FocusMonPrev => { name: "focus_mon_prev", arg_example: None, doc: "focus previous monitor", run: |ctx, _args| { focus_monitor(ctx, MonitorDirection::PREV); } },
    FocusMonNext => { name: "focus_mon_next", arg_example: None, doc: "focus next monitor", run: |ctx, _args| { focus_monitor(ctx, MonitorDirection::NEXT); } },
    FollowMonPrev => { name: "follow_mon_prev", arg_example: None, doc: "move client to prev monitor and follow", run: |ctx, _args| { move_to_monitor_and_follow(ctx, MonitorDirection::PREV); } },
    FollowMonNext => { name: "follow_mon_next", arg_example: None, doc: "move client to next monitor and follow", run: |ctx, _args| { move_to_monitor_and_follow(ctx, MonitorDirection::NEXT); } },
    OverlayToggle => { name: "overlay_toggle", arg_example: None, doc: "toggle overlay scratchpad visibility", run: |ctx, _args| { overlay_toggle(ctx); } },
    OverlayCreate => { name: "overlay_create", arg_example: None, doc: "create overlay scratchpad from focused window", run: |ctx, _args| { overlay_create(ctx); } },
    OverlayDirectionUp => { name: "overlay_direction_up", arg_example: None, doc: "set overlay direction to top", run: |ctx, _args| { overlay_set_direction(ctx, EdgeDirection::Top); } },
    OverlayDirectionDown => { name: "overlay_direction_down", arg_example: None, doc: "set overlay direction to bottom", run: |ctx, _args| { overlay_set_direction(ctx, EdgeDirection::Bottom); } },
    OverlayDirectionLeft => { name: "overlay_direction_left", arg_example: None, doc: "set overlay direction to left", run: |ctx, _args| { overlay_set_direction(ctx, EdgeDirection::Left); } },
    OverlayDirectionRight => { name: "overlay_direction_right", arg_example: None, doc: "set overlay direction to right", run: |ctx, _args| { overlay_set_direction(ctx, EdgeDirection::Right); } },
    ScratchpadToggle => {
        name: "scratchpad_toggle",
        arg_example: None,
        doc: "toggle scratchpad, creating it from current window if it doesn't exist",
        run: |ctx, _args| {
            const DEFAULT_NAME: &str = "instantwm_scratchpad";
            if scratchpad_find(ctx.core().globals(), DEFAULT_NAME).is_some() {
                scratchpad_toggle(ctx, Some(DEFAULT_NAME));
            } else {
                scratchpad_make(ctx, DEFAULT_NAME, None, None, ScratchpadInitialStatus::Shown);
            }
        }
    },
    ToggleBar => { name: "toggle_bar", arg_example: None, doc: "toggle status bar", run: |ctx, _args| { toggle_bar(ctx); } },
    ToggleFloating => { name: "toggle_floating", arg_example: None, doc: "toggle focused window between tiled and floating", run: |ctx, _args| { toggle_floating(ctx); } },
    ToggleSticky => { name: "toggle_sticky", arg_example: None, doc: "toggle sticky (visible on all tags)", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { toggle_sticky(ctx, win); } } },
    ToggleAltTag => { name: "toggle_alt_tag", arg_example: None, doc: "toggle alt-tag mode", run: |ctx, _args| { toggle_alt_tag(ctx, ToggleAction::Toggle); } },
    ToggleAnimated => { name: "toggle_animated", arg_example: None, doc: "toggle window animations", run: |ctx, _args| { ctx.with_behavior_mut(|behavior| toggle_animated(behavior, ToggleAction::Toggle)); } },
    ToggleShowTags => { name: "toggle_show_tags", arg_example: None, doc: "show/hide tag bar", run: |ctx, _args| { toggle_show_tags(ctx, ToggleAction::Toggle); } },
    ToggleDoubleDraw => { name: "toggle_double_draw", arg_example: None, doc: "toggle double draw mode", run: |ctx, _args| { ctx.with_behavior_mut(toggle_double_draw); } },
    ModeToggle => { name: "mode_toggle", arg_example: Some("mode_name"), doc: "toggle a mode (enter if not active, else return to default)", run: |ctx, args| { if let Some(name) = args.first() { toggle_mode(ctx, name); } } },
    TogglePrefix => { name: "toggle_prefix", arg_example: None, doc: "toggle prefix mode (legacy alias for mode_toggle prefix)", run: |ctx, _args| { toggle_mode(ctx, "prefix"); } },
    UnhideAll => { name: "unhide_all", arg_example: None, doc: "show all hidden windows", run: |ctx, _args| { unhide_all(ctx); } },
    Hide => { name: "hide", arg_example: None, doc: "minimize focused window or hide the visible scratchpad", run: |ctx, _args| { if let Some(win) = ctx.selected_client() { crate::client::hide_for_user(ctx, win); } } },
    ToggleFakeFullscreen => { name: "toggle_fake_fullscreen", arg_example: None, doc: "toggle fake fullscreen (X11)", run: |ctx, _args| { if let WmCtx::X11(ctx_x11) = ctx { toggle_fake_fullscreen_x11(ctx_x11); } } },
    DrawWindow => { name: "draw_window", arg_example: None, doc: "start dragging/resizing window", run: |ctx, _args| { draw_window(ctx); } },
    BeginKeyboardMove => { name: "begin_keyboard_move", arg_example: None, doc: "move window with keyboard", run: |ctx, _args| { begin_keyboard_move(ctx); } },
    NextKeyboardLayout => { name: "next_keyboard_layout", arg_example: None, doc: "cycle to next keyboard layout", run: |ctx, _args| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, true); } },
    PrevKeyboardLayout => { name: "prev_keyboard_layout", arg_example: None, doc: "cycle to previous keyboard layout", run: |ctx, _args| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, false); } },
    KeyboardLayout => { name: "keyboard_layout", arg_example: Some("us(intl)"), doc: "set keyboard layout", run: |ctx, args| { if let Some(name) = args.first() { crate::keyboard_layout::set_keyboard_layout_by_name(ctx, name); } } },
    SetMode => { name: "set_mode", arg_example: Some("resize"), doc: "set WM mode (sway-like modes)", run: |ctx, args| { if let Some(name) = args.first() { ctx.set_current_mode(name.clone()); ctx.request_bar_update(None); } } },
    Spawn => { name: "spawn", arg_example: Some("kitty"), doc: "spawn command", run: |ctx, args| { spawn(ctx, args); } },
    SetLayout => { name: "set_layout", arg_example: Some("tile"), doc: "set layout", run: |ctx, args| { if let Some(name) = args.first().and_then(|s| parse_layout_kind_name(s)) { set_layout(ctx, name); } } },
    FocusStack => { name: "focus_stack", arg_example: Some("next"), doc: "focus stack direction", run: |ctx, args| { if let Some(direction) = args.first().and_then(|s| parse_stack_direction_name(s)) { focus_stack(ctx, direction); } } }
);

fn overlay_set_direction(ctx: &mut WmCtx, dir: EdgeDirection) {
    if let Some(win) = scratchpad_find(ctx.core().globals(), OVERLAY_NAME) {
        set_scratchpad_direction(ctx, win, dir);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_layout_kind_name, parse_stack_direction_name};
    use crate::layouts::LayoutKind;
    use crate::types::StackDirection;

    #[test]
    fn parse_layout_kind_name_accepts_aliases() {
        assert_eq!(parse_layout_kind_name("tile"), Some(LayoutKind::Tile));
        assert_eq!(
            parse_layout_kind_name("floating"),
            Some(LayoutKind::Floating)
        );
        assert_eq!(parse_layout_kind_name("bad"), None);
    }

    #[test]
    fn parse_stack_direction_name_accepts_aliases() {
        assert_eq!(
            parse_stack_direction_name("next"),
            Some(StackDirection::Next)
        );
        assert_eq!(
            parse_stack_direction_name("backward"),
            Some(StackDirection::Previous)
        );
        assert_eq!(parse_stack_direction_name("bad"), None);
    }
}
