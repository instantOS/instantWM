use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::{BarPosition, Gesture, MouseButton, Point, WindowId};
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
    if ctx.core.model().clients.contains_key(&touched_window)
        && ctx.core.model().selected_win() != Some(touched_window)
    {
        crate::focus::focus(&mut WmCtx::X11(ctx.reborrow()), Some(touched_window));
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
    let clean_state = crate::util::clean_mask(e.state.into(), numlockmask);

    let target_window = ctx
        .core
        .state
        .model
        .clients
        .contains_key(&event_win)
        .then_some(event_win);

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
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        crate::mouse::press::dispatch_press_policy(&mut wm_ctx, input)
    };

    match outcome {
        crate::mouse::press::PressOutcome::CapturedInteraction { button } => {
            thaw_pointer_grab(ctx);
            let _ = crate::backend::x11::grab::drive_wm_interaction(ctx, button);
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
            crate::systray::press_icon(&mut ctx.core, index, button, root);
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
    let hovered = crate::backend::x11::mouse::managed_window(ctx.core.state, e.event)
        .or_else(|| crate::backend::x11::mouse::managed_window(ctx.core.state, e.child));
    crate::focus::apply_hover_focus(
        &mut WmCtx::X11(ctx.reborrow()),
        hovered,
        entering_root,
        Some(root),
        crate::types::HoverFocusTrigger::SceneChange,
    );
}

pub fn leave_notify(ctx: &mut WmCtxX11<'_>, _e: &LeaveNotifyEvent) {
    crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
}

/// Core-motion fallback for X servers without XI2 raw motion support.
pub fn motion_notify(ctx: &mut WmCtxX11<'_>, e: &MotionNotifyEvent) {
    let event_win = WindowId::from(e.event);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        return;
    }

    let hovered = crate::backend::x11::mouse::managed_window(ctx.core.state, e.child);
    physical_pointer_motion(ctx, Point::new(e.root_x as i32, e.root_y as i32), hovered);
}

/// XI2 raw motion is the authoritative physical-motion signal on X11.
/// Querying the root position here converts the device-independent signal into
/// the same coordinates consumed by the backend-neutral hover policy.
pub fn raw_motion_notify(ctx: &mut WmCtxX11<'_>) {
    let Some(snapshot) = crate::backend::x11::mouse::pointer_snapshot(
        ctx.core.state,
        ctx.x11.conn,
        ctx.x11_runtime.root,
    ) else {
        return;
    };
    physical_pointer_motion(ctx, snapshot.root, snapshot.child);
}

fn physical_pointer_motion(ctx: &mut WmCtxX11<'_>, root: Point, hovered: Option<WindowId>) {
    // Handle focus-follows-mouse monitor switching
    if ctx.core.behavior().current_mode.tree_placement().is_none()
        && ctx.core.behavior().focus_follows_mouse.is_enabled()
        && crate::focus::select_monitor_at_pointer(&mut WmCtx::X11(ctx.reborrow()), root)
    {
        return;
    }

    if crate::mouse::update_overlay_hot_corner(&mut WmCtx::X11(ctx.reborrow()), root) {
        return;
    }

    // Early-out: cursor is below the bar area.
    let (monitor_id, monitor_y, bar_height) = {
        let mon = ctx.core.model().expect_selected_monitor();
        (
            mon.monitor_id,
            mon.monitor_rect.y,
            ctx.core.derived().bar_height,
        )
    };
    let current_gesture = ctx.core.bar.hover.gesture_on(monitor_id);

    if root.y >= monitor_y + bar_height {
        // Overview owns pointer semantics wholesale; a border-zone offer
        // armed before entering it must not keep borrowing the pointer.
        if ctx.core.model().is_overview_active() {
            crate::mouse::clear_hover_offer(&mut WmCtx::X11(ctx.reborrow()));
        } else {
            if crate::mouse::update_sidebar_offer_at(
                &mut WmCtx::X11(ctx.reborrow()),
                root,
                hovered.is_some(),
            )
            .affects_pointer_handling()
            {
                return;
            }
            if crate::mouse::update_resize_offer_with_focus_at(
                &mut WmCtx::X11(ctx.reborrow()),
                root,
            ) {
                return;
            }
        }
        crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
        crate::focus::apply_hover_focus(
            &mut WmCtx::X11(ctx.reborrow()),
            hovered,
            false,
            Some(root),
            crate::types::HoverFocusTrigger::PointerMotion,
        );
        return;
    };

    // The bar owns the pointer for its whole band; release any offer that is
    // still armed from the desktop area below it (Wayland parity).
    crate::mouse::clear_hover_offer(&mut WmCtx::X11(ctx.reborrow()));
    let pos = crate::bar::update_hover(&mut WmCtx::X11(ctx.reborrow()), root, false, false);
    if matches!(pos, Some(BarPosition::Root) | None) && current_gesture != Gesture::None {
        crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
    }
}
