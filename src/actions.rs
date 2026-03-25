use crate::animation;
use crate::client::{close_win, kill_client, shut_kill, toggle_fake_fullscreen_x11, zoom};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::floating::{
    center_window, create_overlay, distribute_clients, hide_overlay, key_resize, scratchpad_toggle,
    set_overlay, show_overlay, toggle_floating, toggle_maximized,
};
use crate::focus::{direction_focus, focus_last_client, focus_stack};
use crate::keyboard::{down_key, up_key};
use crate::layouts::{
    LayoutKind, cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, toggle_layout,
};
use crate::monitor::{Direction as PushDirection, focus_monitor, move_to_monitor_and_follow, reorder_client};
use crate::mouse::{
    begin_keyboard_move, drag_tag, draw_window, gesture_mouse, resize_aspect_mouse,
    resize_mouse_from_cursor, window_title_mouse_handler,
};
use crate::tags::{
    follow_view, last_view, move_client, quit, shift_tag, shift_view, toggle_fullscreen_overview,
    toggle_overview, win_view,
};
use crate::toggles::{
    toggle_alt_tag, toggle_animated, toggle_bar, toggle_double_draw, toggle_locked, toggle_mode,
    toggle_show_tags, toggle_sticky, unhide_all,
};
use crate::types::{
    BarPosition, ButtonArg, Direction, MonitorDirection, StackDirection, TagMask, ToggleAction,
};
use crate::util::spawn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedAction {
    Zoom,
    None,
    Kill,
    ShutKill,
    Quit,
    FocusNext,
    FocusPrev,
    FocusLast,
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    DownKey,
    UpKey,
    ToggleLayout,
    LayoutTile,
    LayoutFloat,
    LayoutMonocle,
    LayoutGrid,
    CycleLayoutNext,
    CycleLayoutPrev,
    IncNmaster,
    DecNmaster,
    MfactGrow,
    MfactShrink,
    SetMfact,
    CenterWindow,
    ToggleMaximized,
    DistributeClients,
    KeyResizeUp,
    KeyResizeDown,
    KeyResizeLeft,
    KeyResizeRight,
    PushUp,
    PushDown,
    LastView,
    FollowView,
    WinView,
    ScrollLeft,
    ScrollRight,
    MoveClientLeft,
    MoveClientRight,
    ShiftTagLeft,
    ShiftTagRight,
    ShiftViewLeft,
    ShiftViewRight,
    ViewAll,
    TagAll,
    ToggleOverview,
    ToggleFullscreenOverview,
    FocusMonPrev,
    FocusMonNext,
    FollowMonPrev,
    FollowMonNext,
    SetOverlay,
    CreateOverlay,
    ScratchpadToggle,
    ToggleBar,
    ToggleSticky,
    ToggleAltTag,
    ToggleAnimated,
    ToggleShowTags,
    ToggleDoubleDraw,
    ModeToggle,
    TogglePrefix,
    UnhideAll,
    Hide,
    ToggleFakeFullscreen,
    DrawWindow,
    BeginKeyboardMove,
    NextKeyboardLayout,
    PrevKeyboardLayout,
    KeyboardLayout,
    SetMode,
    Spawn,
    SetLayout,
    FocusStack,
}

#[derive(Debug, Clone)]
pub enum KeyAction {
    Named {
        action: NamedAction,
        args: Vec<String>,
    },
    ViewTag {
        tag_idx: usize,
    },
    ToggleViewTag {
        tag_idx: usize,
    },
    SetClientTag {
        tag_idx: usize,
    },
    FollowClientTag {
        tag_idx: usize,
    },
    ToggleClientTag {
        tag_idx: usize,
    },
    SwapTags {
        tag_idx: usize,
    },
}

#[derive(Debug, Clone)]
pub enum ButtonAction {
    Named {
        action: NamedAction,
        args: Vec<String>,
    },
    WindowTitleMouseHandler,
    CloseClickedTitleWindow,
    DragTagBegin,
    ToggleClickedViewTag,
    SetSelectedClientClickedTag,
    ToggleSelectedClientClickedTag,
    FollowSelectedClientClickedTag,
    ClientMoveDrag,
    ResizeSelectedAspect,
    KillSelectedClient,
    ToggleLockSelectedClient,
    GestureMouse,
    ReorderSelected {
        up: bool,
    },
    ScaleSelected {
        percent: i32,
    },
    HideOverlay,
    ShowOverlay,
    ToggleFloatingSelected,
    ResizeMouseFromCursor,
}

#[derive(Debug, Clone, Copy)]
pub struct ActionMeta {
    pub name: &'static str,
    pub doc: &'static str,
    pub arg_example: Option<&'static str>,
}

const ACTION_METADATA: &[(NamedAction, ActionMeta)] = &[
    (NamedAction::Zoom, ActionMeta { name: "zoom", doc: "zoom client into master area", arg_example: None }),
    (NamedAction::None, ActionMeta { name: "none", doc: "explicitly unbind/ignore this key combination", arg_example: None }),
    (NamedAction::Kill, ActionMeta { name: "kill", doc: "close focused window gracefully", arg_example: None }),
    (NamedAction::ShutKill, ActionMeta { name: "shut_kill", doc: "force kill focused window", arg_example: None }),
    (NamedAction::Quit, ActionMeta { name: "quit", doc: "quit instantwm", arg_example: None }),
    (NamedAction::FocusNext, ActionMeta { name: "focus_next", doc: "focus next window in stack", arg_example: None }),
    (NamedAction::FocusPrev, ActionMeta { name: "focus_prev", doc: "focus previous window in stack", arg_example: None }),
    (NamedAction::FocusLast, ActionMeta { name: "focus_last", doc: "focus last focused window", arg_example: None }),
    (NamedAction::FocusUp, ActionMeta { name: "focus_up", doc: "focus window above", arg_example: None }),
    (NamedAction::FocusDown, ActionMeta { name: "focus_down", doc: "focus window below", arg_example: None }),
    (NamedAction::FocusLeft, ActionMeta { name: "focus_left", doc: "focus window to left", arg_example: None }),
    (NamedAction::FocusRight, ActionMeta { name: "focus_right", doc: "focus window to right", arg_example: None }),
    (NamedAction::DownKey, ActionMeta { name: "down_key", doc: "alt-tab forward", arg_example: None }),
    (NamedAction::UpKey, ActionMeta { name: "up_key", doc: "alt-tab backward", arg_example: None }),
    (NamedAction::ToggleLayout, ActionMeta { name: "toggle_layout", doc: "toggle layout", arg_example: None }),
    (NamedAction::LayoutTile, ActionMeta { name: "layout_tile", doc: "set tile layout", arg_example: None }),
    (NamedAction::LayoutFloat, ActionMeta { name: "layout_float", doc: "set floating layout", arg_example: None }),
    (NamedAction::LayoutMonocle, ActionMeta { name: "layout_monocle", doc: "set monocle layout (fullscreen)", arg_example: None }),
    (NamedAction::LayoutGrid, ActionMeta { name: "layout_grid", doc: "set grid layout", arg_example: None }),
    (NamedAction::CycleLayoutNext, ActionMeta { name: "cycle_layout_next", doc: "cycle to next layout", arg_example: None }),
    (NamedAction::CycleLayoutPrev, ActionMeta { name: "cycle_layout_prev", doc: "cycle to previous layout", arg_example: None }),
    (NamedAction::IncNmaster, ActionMeta { name: "inc_nmaster", doc: "increase master window count", arg_example: Some("1") }),
    (NamedAction::DecNmaster, ActionMeta { name: "dec_nmaster", doc: "decrease master window count", arg_example: None }),
    (NamedAction::MfactGrow, ActionMeta { name: "mfact_grow", doc: "increase master area width", arg_example: None }),
    (NamedAction::MfactShrink, ActionMeta { name: "mfact_shrink", doc: "decrease master area width", arg_example: None }),
    (NamedAction::SetMfact, ActionMeta { name: "set_mfact", doc: "set master factor", arg_example: Some("0.05") }),
    (NamedAction::CenterWindow, ActionMeta { name: "center_window", doc: "center focused window", arg_example: None }),
    (NamedAction::ToggleMaximized, ActionMeta { name: "toggle_maximized", doc: "toggle maximized state", arg_example: None }),
    (NamedAction::DistributeClients, ActionMeta { name: "distribute_clients", doc: "distribute windows evenly", arg_example: None }),
    (NamedAction::KeyResizeUp, ActionMeta { name: "key_resize_up", doc: "resize floating window up", arg_example: None }),
    (NamedAction::KeyResizeDown, ActionMeta { name: "key_resize_down", doc: "resize floating window down", arg_example: None }),
    (NamedAction::KeyResizeLeft, ActionMeta { name: "key_resize_left", doc: "resize floating window left", arg_example: None }),
    (NamedAction::KeyResizeRight, ActionMeta { name: "key_resize_right", doc: "resize floating window right", arg_example: None }),
    (NamedAction::PushUp, ActionMeta { name: "push_up", doc: "push window up in stack", arg_example: None }),
    (NamedAction::PushDown, ActionMeta { name: "push_down", doc: "push window down in stack", arg_example: None }),
    (NamedAction::LastView, ActionMeta { name: "last_view", doc: "view previously viewed tags", arg_example: None }),
    (NamedAction::FollowView, ActionMeta { name: "follow_view", doc: "follow client to its tags", arg_example: None }),
    (NamedAction::WinView, ActionMeta { name: "win_view", doc: "view tags of focused client", arg_example: None }),
    (NamedAction::ScrollLeft, ActionMeta { name: "scroll_left", doc: "scroll tags left", arg_example: None }),
    (NamedAction::ScrollRight, ActionMeta { name: "scroll_right", doc: "scroll tags right", arg_example: None }),
    (NamedAction::MoveClientLeft, ActionMeta { name: "move_client_left", doc: "move client to tag on left", arg_example: None }),
    (NamedAction::MoveClientRight, ActionMeta { name: "move_client_right", doc: "move client to tag on right", arg_example: None }),
    (NamedAction::ShiftTagLeft, ActionMeta { name: "shift_tag_left", doc: "shift client to tag on left", arg_example: None }),
    (NamedAction::ShiftTagRight, ActionMeta { name: "shift_tag_right", doc: "shift client to tag on right", arg_example: None }),
    (NamedAction::ShiftViewLeft, ActionMeta { name: "shift_view_left", doc: "shift view to tag on left", arg_example: None }),
    (NamedAction::ShiftViewRight, ActionMeta { name: "shift_view_right", doc: "shift view to tag on right", arg_example: None }),
    (NamedAction::ViewAll, ActionMeta { name: "view_all", doc: "view all tags", arg_example: None }),
    (NamedAction::TagAll, ActionMeta { name: "tag_all", doc: "tag client with all tags", arg_example: None }),
    (NamedAction::ToggleOverview, ActionMeta { name: "toggle_overview", doc: "toggle overview mode", arg_example: None }),
    (NamedAction::ToggleFullscreenOverview, ActionMeta { name: "toggle_fullscreen_overview", doc: "toggle fullscreen overview", arg_example: None }),
    (NamedAction::FocusMonPrev, ActionMeta { name: "focus_mon_prev", doc: "focus previous monitor", arg_example: None }),
    (NamedAction::FocusMonNext, ActionMeta { name: "focus_mon_next", doc: "focus next monitor", arg_example: None }),
    (NamedAction::FollowMonPrev, ActionMeta { name: "follow_mon_prev", doc: "move client to prev monitor and follow", arg_example: None }),
    (NamedAction::FollowMonNext, ActionMeta { name: "follow_mon_next", doc: "move client to next monitor and follow", arg_example: None }),
    (NamedAction::SetOverlay, ActionMeta { name: "set_overlay", doc: "set overlay", arg_example: None }),
    (NamedAction::CreateOverlay, ActionMeta { name: "create_overlay", doc: "create overlay from focused client", arg_example: None }),
    (NamedAction::ScratchpadToggle, ActionMeta { name: "scratchpad_toggle", doc: "toggle scratchpad", arg_example: None }),
    (NamedAction::ToggleBar, ActionMeta { name: "toggle_bar", doc: "toggle status bar", arg_example: None }),
    (NamedAction::ToggleSticky, ActionMeta { name: "toggle_sticky", doc: "toggle sticky (visible on all tags)", arg_example: None }),
    (NamedAction::ToggleAltTag, ActionMeta { name: "toggle_alt_tag", doc: "toggle alt-tag mode", arg_example: None }),
    (NamedAction::ToggleAnimated, ActionMeta { name: "toggle_animated", doc: "toggle window animations", arg_example: None }),
    (NamedAction::ToggleShowTags, ActionMeta { name: "toggle_show_tags", doc: "show/hide tag bar", arg_example: None }),
    (NamedAction::ToggleDoubleDraw, ActionMeta { name: "toggle_double_draw", doc: "toggle double draw mode", arg_example: None }),
    (NamedAction::ModeToggle, ActionMeta { name: "mode_toggle", doc: "toggle a mode (enter if not active, else return to default)", arg_example: Some("mode_name") }),
    (NamedAction::TogglePrefix, ActionMeta { name: "toggle_prefix", doc: "toggle prefix mode (legacy alias for mode_toggle prefix)", arg_example: None }),
    (NamedAction::UnhideAll, ActionMeta { name: "unhide_all", doc: "show all hidden windows", arg_example: None }),
    (NamedAction::Hide, ActionMeta { name: "hide", doc: "hide focused window", arg_example: None }),
    (NamedAction::ToggleFakeFullscreen, ActionMeta { name: "toggle_fake_fullscreen", doc: "toggle fake fullscreen (X11)", arg_example: None }),
    (NamedAction::DrawWindow, ActionMeta { name: "draw_window", doc: "start dragging/resizing window", arg_example: None }),
    (NamedAction::BeginKeyboardMove, ActionMeta { name: "begin_keyboard_move", doc: "move window with keyboard", arg_example: None }),
    (NamedAction::NextKeyboardLayout, ActionMeta { name: "next_keyboard_layout", doc: "cycle to next keyboard layout", arg_example: None }),
    (NamedAction::PrevKeyboardLayout, ActionMeta { name: "prev_keyboard_layout", doc: "cycle to previous keyboard layout", arg_example: None }),
    (NamedAction::KeyboardLayout, ActionMeta { name: "keyboard_layout", doc: "set keyboard layout", arg_example: Some("us(intl)") }),
    (NamedAction::SetMode, ActionMeta { name: "set_mode", doc: "set WM mode (sway-like modes)", arg_example: Some("resize") }),
    (NamedAction::Spawn, ActionMeta { name: "spawn", doc: "spawn command", arg_example: Some("kitty") }),
    (NamedAction::SetLayout, ActionMeta { name: "set_layout", doc: "set layout", arg_example: Some("tile") }),
    (NamedAction::FocusStack, ActionMeta { name: "focus_stack", doc: "focus stack direction", arg_example: Some("next") }),
];

pub fn get_action_metadata() -> Vec<ActionMeta> {
    ACTION_METADATA.iter().map(|(_, meta)| *meta).collect()
}

pub fn parse_named_action(name: &str) -> Option<NamedAction> {
    Some(match name.to_ascii_lowercase().as_str() {
        "zoom" => NamedAction::Zoom,
        "none" => NamedAction::None,
        "kill" => NamedAction::Kill,
        "shut_kill" => NamedAction::ShutKill,
        "quit" => NamedAction::Quit,
        "focus_next" => NamedAction::FocusNext,
        "focus_prev" => NamedAction::FocusPrev,
        "focus_last" => NamedAction::FocusLast,
        "focus_up" => NamedAction::FocusUp,
        "focus_down" => NamedAction::FocusDown,
        "focus_left" => NamedAction::FocusLeft,
        "focus_right" => NamedAction::FocusRight,
        "down_key" => NamedAction::DownKey,
        "up_key" => NamedAction::UpKey,
        "toggle_layout" => NamedAction::ToggleLayout,
        "layout_tile" => NamedAction::LayoutTile,
        "layout_float" => NamedAction::LayoutFloat,
        "layout_monocle" => NamedAction::LayoutMonocle,
        "layout_grid" => NamedAction::LayoutGrid,
        "cycle_layout_next" => NamedAction::CycleLayoutNext,
        "cycle_layout_prev" => NamedAction::CycleLayoutPrev,
        "inc_nmaster" => NamedAction::IncNmaster,
        "dec_nmaster" => NamedAction::DecNmaster,
        "mfact_grow" => NamedAction::MfactGrow,
        "mfact_shrink" => NamedAction::MfactShrink,
        "set_mfact" => NamedAction::SetMfact,
        "center_window" => NamedAction::CenterWindow,
        "toggle_maximized" => NamedAction::ToggleMaximized,
        "distribute_clients" => NamedAction::DistributeClients,
        "key_resize_up" => NamedAction::KeyResizeUp,
        "key_resize_down" => NamedAction::KeyResizeDown,
        "key_resize_left" => NamedAction::KeyResizeLeft,
        "key_resize_right" => NamedAction::KeyResizeRight,
        "push_up" => NamedAction::PushUp,
        "push_down" => NamedAction::PushDown,
        "last_view" => NamedAction::LastView,
        "follow_view" => NamedAction::FollowView,
        "win_view" => NamedAction::WinView,
        "scroll_left" => NamedAction::ScrollLeft,
        "scroll_right" => NamedAction::ScrollRight,
        "move_client_left" => NamedAction::MoveClientLeft,
        "move_client_right" => NamedAction::MoveClientRight,
        "shift_tag_left" => NamedAction::ShiftTagLeft,
        "shift_tag_right" => NamedAction::ShiftTagRight,
        "shift_view_left" => NamedAction::ShiftViewLeft,
        "shift_view_right" => NamedAction::ShiftViewRight,
        "view_all" => NamedAction::ViewAll,
        "tag_all" => NamedAction::TagAll,
        "toggle_overview" => NamedAction::ToggleOverview,
        "toggle_fullscreen_overview" => NamedAction::ToggleFullscreenOverview,
        "focus_mon_prev" => NamedAction::FocusMonPrev,
        "focus_mon_next" => NamedAction::FocusMonNext,
        "follow_mon_prev" => NamedAction::FollowMonPrev,
        "follow_mon_next" => NamedAction::FollowMonNext,
        "set_overlay" => NamedAction::SetOverlay,
        "create_overlay" => NamedAction::CreateOverlay,
        "scratchpad_toggle" => NamedAction::ScratchpadToggle,
        "toggle_bar" => NamedAction::ToggleBar,
        "toggle_sticky" => NamedAction::ToggleSticky,
        "toggle_alt_tag" => NamedAction::ToggleAltTag,
        "toggle_animated" => NamedAction::ToggleAnimated,
        "toggle_show_tags" => NamedAction::ToggleShowTags,
        "toggle_double_draw" => NamedAction::ToggleDoubleDraw,
        "mode_toggle" => NamedAction::ModeToggle,
        "toggle_prefix" => NamedAction::TogglePrefix,
        "unhide_all" => NamedAction::UnhideAll,
        "hide" => NamedAction::Hide,
        "toggle_fake_fullscreen" => NamedAction::ToggleFakeFullscreen,
        "draw_window" => NamedAction::DrawWindow,
        "begin_keyboard_move" => NamedAction::BeginKeyboardMove,
        "next_keyboard_layout" => NamedAction::NextKeyboardLayout,
        "prev_keyboard_layout" => NamedAction::PrevKeyboardLayout,
        "keyboard_layout" => NamedAction::KeyboardLayout,
        "set_mode" => NamedAction::SetMode,
        "spawn" => NamedAction::Spawn,
        "set_layout" => NamedAction::SetLayout,
        "focus_stack" => NamedAction::FocusStack,
        _ => return None,
    })
}

fn tag_mask_from_idx(tag_idx: usize) -> Option<TagMask> {
    TagMask::single(tag_idx + 1)
}

fn tag_mask_from_pos(pos: BarPosition) -> Option<TagMask> {
    match pos {
        BarPosition::Tag(idx) => tag_mask_from_idx(idx),
        _ => None,
    }
}

pub fn execute_key_action(ctx: &mut WmCtx<'_>, action: &KeyAction) {
    match action {
        KeyAction::Named { action, args } => execute_named_action(ctx, *action, args),
        KeyAction::ViewTag { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::view(ctx, mask);
            }
        }
        KeyAction::ToggleViewTag { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::toggle_view_ctx(ctx, mask);
            }
        }
        KeyAction::SetClientTag { tag_idx } => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, mask);
            }
        }
        KeyAction::FollowClientTag { tag_idx } => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::follow_tag_ctx(ctx, win, mask);
            }
        }
        KeyAction::ToggleClientTag { tag_idx } => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::toggle_tag_ctx(ctx, win, mask);
            }
        }
        KeyAction::SwapTags { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::swap_tags_ctx(ctx, mask);
            }
        }
    }
}

pub fn execute_button_action(ctx: &mut WmCtx<'_>, action: &ButtonAction, arg: ButtonArg) {
    match action {
        ButtonAction::Named { action, args } => execute_named_action(ctx, *action, args),
        ButtonAction::WindowTitleMouseHandler => {
            let BarPosition::WinTitle(win) = arg.pos else {
                return;
            };
            window_title_mouse_handler(ctx, win, arg.btn, arg.rx, arg.ry);
        }
        ButtonAction::CloseClickedTitleWindow => {
            let BarPosition::WinTitle(win) = arg.pos else {
                return;
            };
            close_win(ctx, win);
        }
        ButtonAction::DragTagBegin => match ctx {
            WmCtx::X11(ctx_x11) => drag_tag(ctx_x11, arg.pos, arg.btn, arg.rx),
            WmCtx::Wayland(_) => {
                let _ = crate::mouse::drag::drag_tag_begin(ctx, arg.pos, arg.btn);
            }
        },
        ButtonAction::ToggleClickedViewTag => {
            if let BarPosition::Tag(idx) = arg.pos {
                crate::tags::view::toggle_view_tag(ctx, idx);
            }
        }
        ButtonAction::SetSelectedClientClickedTag => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_pos(arg.pos)
            {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, mask);
            }
        }
        ButtonAction::ToggleSelectedClientClickedTag => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_pos(arg.pos)
            {
                crate::tags::client_tags::toggle_tag_ctx(ctx, win, mask);
            }
        }
        ButtonAction::FollowSelectedClientClickedTag => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_pos(arg.pos)
            {
                crate::tags::client_tags::follow_tag_ctx(ctx, win, mask);
            }
        }
        ButtonAction::ClientMoveDrag => match ctx {
            WmCtx::X11(ctx_x11) => crate::backend::x11::mouse::move_mouse_x11(ctx_x11, arg.btn, None),
            WmCtx::Wayland(_) => {
                if let Some(win) = ctx.selected_client() {
                    crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.rx, arg.ry, false);
                }
            }
        },
        ButtonAction::ResizeSelectedAspect => {
            if let Some(win) = ctx.selected_client() {
                resize_aspect_mouse(ctx, win, arg.btn);
            }
        }
        ButtonAction::KillSelectedClient => {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win);
            }
        }
        ButtonAction::ToggleLockSelectedClient => {
            if let Some(win) = ctx.selected_client() {
                toggle_locked(ctx, win);
            }
        }
        ButtonAction::GestureMouse => gesture_mouse(ctx, arg.btn),
        ButtonAction::ReorderSelected { up } => {
            if let Some(win) = ctx.selected_client() {
                reorder_client(
                    ctx,
                    win,
                    if *up {
                        PushDirection::Up
                    } else {
                        PushDirection::Down
                    },
                );
            }
        }
        ButtonAction::ScaleSelected { percent } => {
            if let Some(win) = ctx.selected_client() {
                crate::client::geometry::scale_client(ctx, win, *percent);
            }
        }
        ButtonAction::HideOverlay => hide_overlay(ctx),
        ButtonAction::ShowOverlay => show_overlay(ctx),
        ButtonAction::ToggleFloatingSelected => toggle_floating(ctx),
        ButtonAction::ResizeMouseFromCursor => resize_mouse_from_cursor(ctx, arg.btn),
    }
}

pub fn execute_named_action(ctx: &mut WmCtx<'_>, action: NamedAction, args: &[String]) {
    match action {
        NamedAction::Zoom => zoom(ctx),
        NamedAction::None => {}
        NamedAction::Kill => {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win);
            }
        }
        NamedAction::ShutKill => shut_kill(ctx),
        NamedAction::Quit => quit(),
        NamedAction::FocusNext => focus_stack(ctx, StackDirection::Next),
        NamedAction::FocusPrev => focus_stack(ctx, StackDirection::Previous),
        NamedAction::FocusLast => focus_last_client(ctx),
        NamedAction::FocusUp => direction_focus(ctx, Direction::Up),
        NamedAction::FocusDown => direction_focus(ctx, Direction::Down),
        NamedAction::FocusLeft => direction_focus(ctx, Direction::Left),
        NamedAction::FocusRight => direction_focus(ctx, Direction::Right),
        NamedAction::DownKey => down_key(ctx, StackDirection::Next),
        NamedAction::UpKey => up_key(ctx, StackDirection::Previous),
        NamedAction::ToggleLayout => toggle_layout(ctx),
        NamedAction::LayoutTile => set_layout(ctx, LayoutKind::Tile),
        NamedAction::LayoutFloat => set_layout(ctx, LayoutKind::Floating),
        NamedAction::LayoutMonocle => set_layout(ctx, LayoutKind::Monocle),
        NamedAction::LayoutGrid => set_layout(ctx, LayoutKind::Grid),
        NamedAction::CycleLayoutNext => cycle_layout_direction(ctx, true),
        NamedAction::CycleLayoutPrev => cycle_layout_direction(ctx, false),
        NamedAction::IncNmaster => inc_nmaster_by(ctx, args.first().and_then(|s| s.parse().ok()).unwrap_or(1)),
        NamedAction::DecNmaster => inc_nmaster_by(ctx, -1),
        NamedAction::MfactGrow => set_mfact(ctx, 0.05),
        NamedAction::MfactShrink => set_mfact(ctx, -0.05),
        NamedAction::SetMfact => {
            if let Some(delta) = args.first().and_then(|s| s.parse::<f32>().ok()) {
                set_mfact(ctx, delta);
            }
        }
        NamedAction::CenterWindow => {
            if let Some(win) = ctx.selected_client() {
                center_window(ctx, win);
            }
        }
        NamedAction::ToggleMaximized => toggle_maximized(ctx),
        NamedAction::DistributeClients => distribute_clients(ctx),
        NamedAction::KeyResizeUp => {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Up);
            }
        }
        NamedAction::KeyResizeDown => {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Down);
            }
        }
        NamedAction::KeyResizeLeft => {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Left);
            }
        }
        NamedAction::KeyResizeRight => {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Right);
            }
        }
        NamedAction::PushUp => {
            if let Some(win) = ctx.selected_client() {
                reorder_client(ctx, win, PushDirection::Up);
            }
        }
        NamedAction::PushDown => {
            if let Some(win) = ctx.selected_client() {
                reorder_client(ctx, win, PushDirection::Down);
            }
        }
        NamedAction::LastView => last_view(ctx),
        NamedAction::FollowView => follow_view(ctx),
        NamedAction::WinView => win_view(ctx),
        NamedAction::ScrollLeft => animation::anim_scroll(ctx, Direction::Left),
        NamedAction::ScrollRight => animation::anim_scroll(ctx, Direction::Right),
        NamedAction::MoveClientLeft => move_client(ctx, Direction::Left),
        NamedAction::MoveClientRight => move_client(ctx, Direction::Right),
        NamedAction::ShiftTagLeft => shift_tag(ctx, Direction::Left, 1),
        NamedAction::ShiftTagRight => shift_tag(ctx, Direction::Right, 1),
        NamedAction::ShiftViewLeft => shift_view(ctx, Direction::Left),
        NamedAction::ShiftViewRight => shift_view(ctx, Direction::Right),
        NamedAction::ViewAll => crate::tags::view::view(ctx, TagMask::ALL_BITS),
        NamedAction::TagAll => {
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, TagMask::ALL_BITS);
            }
        }
        NamedAction::ToggleOverview => toggle_overview(ctx, TagMask::ALL_BITS),
        NamedAction::ToggleFullscreenOverview => toggle_fullscreen_overview(ctx, TagMask::ALL_BITS),
        NamedAction::FocusMonPrev => focus_monitor(ctx, MonitorDirection::PREV),
        NamedAction::FocusMonNext => focus_monitor(ctx, MonitorDirection::NEXT),
        NamedAction::FollowMonPrev => move_to_monitor_and_follow(ctx, MonitorDirection::PREV),
        NamedAction::FollowMonNext => move_to_monitor_and_follow(ctx, MonitorDirection::NEXT),
        NamedAction::SetOverlay => set_overlay(ctx),
        NamedAction::CreateOverlay => {
            if let Some(win) = ctx.selected_client() {
                create_overlay(ctx, win);
            }
        }
        NamedAction::ScratchpadToggle => scratchpad_toggle(ctx, None),
        NamedAction::ToggleBar => toggle_bar(ctx),
        NamedAction::ToggleSticky => {
            if let Some(win) = ctx.selected_client() {
                toggle_sticky(ctx, win);
            }
        }
        NamedAction::ToggleAltTag => toggle_alt_tag(ctx, ToggleAction::Toggle),
        NamedAction::ToggleAnimated => {
            toggle_animated(&mut ctx.core_mut().globals_mut().behavior, ToggleAction::Toggle);
        }
        NamedAction::ToggleShowTags => toggle_show_tags(ctx, ToggleAction::Toggle),
        NamedAction::ToggleDoubleDraw => toggle_double_draw(&mut ctx.core_mut().globals_mut().behavior),
        NamedAction::ModeToggle => {
            if let Some(name) = args.first() {
                toggle_mode(ctx, name);
            }
        }
        NamedAction::TogglePrefix => toggle_mode(ctx, "prefix"),
        NamedAction::UnhideAll => unhide_all(ctx),
        NamedAction::Hide => {
            if let Some(win) = ctx.selected_client() {
                crate::client::hide(ctx, win);
            }
        }
        NamedAction::ToggleFakeFullscreen => {
            if let WmCtx::X11(ctx_x11) = ctx {
                toggle_fake_fullscreen_x11(ctx_x11);
            }
        }
        NamedAction::DrawWindow => draw_window(ctx),
        NamedAction::BeginKeyboardMove => begin_keyboard_move(ctx),
        NamedAction::NextKeyboardLayout => {
            let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, true);
        }
        NamedAction::PrevKeyboardLayout => {
            let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, false);
        }
        NamedAction::KeyboardLayout => {
            if let Some(name) = args.first() {
                crate::keyboard_layout::set_keyboard_layout_by_name(ctx, name);
            }
        }
        NamedAction::SetMode => {
            if let Some(name) = args.first() {
                ctx.core_mut().globals_mut().behavior.current_mode = name.clone();
                ctx.request_bar_update(None);
            }
        }
        NamedAction::Spawn => spawn(ctx, args),
        NamedAction::SetLayout => {
            if let Some(name) = args.first() {
                let kind = match name.to_ascii_lowercase().as_str() {
                    "tile" | "tiling" => LayoutKind::Tile,
                    "float" | "floating" => LayoutKind::Floating,
                    "monocle" => LayoutKind::Monocle,
                    "grid" => LayoutKind::Grid,
                    _ => return,
                };
                set_layout(ctx, kind);
            }
        }
        NamedAction::FocusStack => {
            if let Some(dir) = args.first() {
                let direction = match dir.to_ascii_lowercase().as_str() {
                    "next" | "down" | "forward" => StackDirection::Next,
                    "prev" | "previous" | "up" | "backward" => StackDirection::Previous,
                    _ => return,
                };
                focus_stack(ctx, direction);
            }
        }
    }
}

pub fn execute_button_action_x11(ctx: &mut WmCtxX11<'_>, action: &ButtonAction, arg: ButtonArg) {
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    execute_button_action(&mut wm_ctx, action, arg);
}

pub fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}
