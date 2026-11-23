use crate::backend::BackendKind;
use crate::contexts::WmCtxX11;
use crate::mouse::motion::{MotionBand, MotionPolicyInput};
use crate::types::{ModMask, MouseButton, Point, WindowId};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xinput::{ConnectionExt as XInputConnectionExt, EventMode, TouchBeginEvent};
use x11rb::protocol::xproto::*;

/// Focus a client touched through instantWM's XI2 passive touch grab, then
/// reject ownership so the native touch history continues to the application.
/// This keeps core Button1 click-to-focus from converting browser scrolling
/// into a mouse drag.
pub fn touch_begin(ctx: &mut WmCtxX11<'_>, e: &TouchBeginEvent) {
    let touched_window = WindowId::from(e.event);
    if ctx.core.state.model.client(touched_window).is_some()
        && ctx.core.state.model.selected_win() != Some(touched_window)
    {
        crate::focus::focus(&mut ctx.wm_ctx(), Some(touched_window));
    }

    let _ = ctx.x11.conn.xinput_xi_allow_events(
        e.time,
        e.deviceid,
        EventMode::REJECT_TOUCH,
        e.detail,
        e.event,
    );
    let _ = ctx.x11.conn.flush();
}

/// Release a synchronous passive-grab freeze without replaying the press,
/// before an interaction's active grab takes over.
fn thaw_pointer_grab(ctx: &WmCtxX11<'_>) {
    let _ = ctx
        .x11
        .conn
        .allow_events(Allow::ASYNC_POINTER, CURRENT_TIME);
    let _ = ctx.x11.conn.flush();
}

pub fn button_press(ctx: &mut WmCtxX11<'_>, e: &ButtonPressEvent) {
    let event_win = WindowId::from(e.event);
    let numlockmask = ctx.x11_runtime().numlockmask;
    let root = Point::new(e.root_x as i32, e.root_y as i32);
    let clean_state = ModMask::new(e.state.bits()).cleaned(numlockmask);

    let is_managed = ctx.core.state.model.client(event_win).is_some();
    let target_window = is_managed.then_some(event_win);

    let button = MouseButton::from_x11_detail(e.detail);

    let input = crate::mouse::press::PressInput {
        root,
        button,
        raw_button: e.detail,
        modifiers: clean_state,
        clicked_window: target_window,
        source: crate::types::InteractionSource::Pointer,
        time_msec: e.time,
    };

    let outcome = {
        let mut wm_ctx = ctx.wm_ctx();
        crate::mouse::press::dispatch_press_policy(&mut wm_ctx, input)
    };

    match outcome {
        crate::mouse::press::PressOutcome::CapturedInteraction { button } => {
            thaw_pointer_grab(ctx);
            let _ = crate::backend::x11::grab::begin_wm_interaction(ctx, button);
        }
        crate::mouse::press::PressOutcome::Consumed => {
            let conn = ctx.x11.conn;
            let _ = conn.allow_events(Allow::ASYNC_POINTER, CURRENT_TIME);
            let _ = conn.flush();
        }
        crate::mouse::press::PressOutcome::SystrayIconPress {
            index,
            button,
            root,
        } => {
            crate::systray::press_icon(ctx.core, index, button, root);
            let conn = ctx.x11.conn;
            let _ = conn.allow_events(Allow::ASYNC_POINTER, CURRENT_TIME);
            let _ = conn.flush();
        }
        crate::mouse::press::PressOutcome::ReplayToClient { .. } => {
            let conn = ctx.x11.conn;
            let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
            let _ = conn.flush();
        }
    }
}

/// Crossing events represent scene changes rather than physical pointer
/// motion. They therefore affect focus only in `force` mode.
pub fn enter_notify(ctx: &mut WmCtxX11<'_>, e: &EnterNotifyEvent) {
    let entering_root = e.event == ctx.x11_runtime.root;
    if (e.mode != NotifyMode::NORMAL || e.detail == NotifyDetail::INFERIOR) && !entering_root {
        return;
    }
    let root = Point::new(e.root_x as i32, e.root_y as i32);
    let hovered = crate::backend::x11::mouse::managed_window(&ctx.core.state, e.event)
        .or_else(|| crate::backend::x11::mouse::managed_window(&ctx.core.state, e.child));
    crate::focus::apply_hover_focus(
        &mut ctx.wm_ctx(),
        hovered,
        entering_root,
        Some(root),
        crate::types::HoverFocusTrigger::SceneChange,
    );
}

pub fn leave_notify(ctx: &mut WmCtxX11<'_>, _e: &LeaveNotifyEvent) {
    crate::bar::clear_hover(&mut ctx.wm_ctx());
}

/// Core-motion fallback for X servers without XI2 raw motion support.
pub fn motion_notify(ctx: &mut WmCtxX11<'_>, e: &MotionNotifyEvent) {
    let hovered = crate::backend::x11::mouse::managed_window(&ctx.core.state, e.event)
        .or_else(|| crate::backend::x11::mouse::managed_window(&ctx.core.state, e.child));
    physical_pointer_motion(ctx, Point::new(e.root_x as i32, e.root_y as i32), hovered);
}

/// XI2 raw motion is the authoritative physical-motion signal on X11.
/// Querying the root position here converts the device-independent signal into
/// the same coordinates consumed by the backend-neutral hover policy.
pub fn raw_motion_notify(ctx: &mut WmCtxX11<'_>) {
    let Some(snapshot) = crate::backend::x11::mouse::pointer_snapshot(
        &ctx.core.state,
        ctx.x11.conn,
        ctx.x11_runtime.root,
    ) else {
        return;
    };
    physical_pointer_motion(ctx, snapshot.root, snapshot.child);
}

fn physical_pointer_motion(ctx: &mut WmCtxX11<'_>, root: Point, hovered: Option<WindowId>) {
    // Select the monitor before evaluating the desktop hover targets.
    let monitor_changed = ctx
        .core
        .state
        .behavior
        .current_mode
        .tree_placement()
        .is_none()
        && ctx
            .core
            .state
            .config
            .window
            .focus_follows_mouse
            .is_enabled()
        && crate::focus::select_monitor_at_pointer(&mut ctx.wm_ctx(), root);

    if crate::mouse::update_overlay_hot_corner(&mut ctx.wm_ctx(), root) {
        crate::mouse::clear_hover_offer(&mut ctx.wm_ctx());
        return;
    }

    let (monitor_y, bar_height) = {
        let mon = ctx.core.state.model.expect_selected_monitor();
        (mon.monitor_rect.y, mon.bar_height)
    };
    let band = if root.y < monitor_y + bar_height {
        MotionBand::Bar
    } else {
        MotionBand::Content
    };

    let _ = crate::mouse::motion::update_motion_policy(
        &mut ctx.wm_ctx(),
        MotionPolicyInput {
            backend: BackendKind::X11,
            root,
            hovered,
            band,
            trigger: crate::types::HoverFocusTrigger::PointerMotion,
            sync_selected_monitor: false,
            bar_hover_blocked: false,
            capture_active: false,
            sidebar_blocked: hovered.is_some(),
            suppress_hover_focus: false,
            monitor_already_selected: monitor_changed,
            preserve_focus: false,
        },
    );
}
