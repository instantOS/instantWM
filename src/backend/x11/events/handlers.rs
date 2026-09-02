use crate::backend::x11::lifecycle::unmanage;
use crate::backend::x11::systray::XEmbedMessage;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::{BarPosition, Gesture, MouseButton, Point, Rect, TagMask, WindowId};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xinput::{ConnectionExt as XInputConnectionExt, EventMode, TouchBeginEvent};
use x11rb::protocol::xproto::*;

use super::query_manageable_window_geometry;
use super::setup::SYSTEM_TRAY_REQUEST_DOCK;

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

/// Handle incoming X11 client messages.
pub fn client_message(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let showsystray = ctx.core.config().systray.show;
    let systray_win = ctx.xembed_tray.as_ref().map(|s| s.win).unwrap_or_default();
    let net_system_tray_op = ctx.x11_runtime.netatom.system_tray_op;
    let net_wm_state = ctx.x11_runtime.netatom.wm_state;
    let net_active_window = ctx.x11_runtime.netatom.active_window;
    let net_current_desktop = ctx.x11_runtime.netatom.current_desktop;
    let net_wm_desktop = ctx.x11_runtime.netatom.wm_desktop;
    let event_win = WindowId::from(e.window);

    if showsystray && event_win == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(ctx, e);
        }
        return;
    };

    if e.type_ == net_current_desktop {
        handle_current_desktop(ctx, e);
        return;
    }

    if ctx.core.model().client(event_win).is_none() {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(ctx, e, event_win);
    } else if e.type_ == net_active_window {
        handle_active_window(ctx, event_win);
    } else if e.type_ == net_wm_desktop {
        handle_wm_desktop(ctx, e, event_win);
    };
}

pub fn configure_notify(ctx: &mut WmCtxX11<'_>, e: &ConfigureNotifyEvent) {
    let event_win = WindowId::from(e.window);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        return;
    };

    ctx.core.derived_mut().display.width = e.width as i32;
    ctx.core.derived_mut().display.height = e.height as i32;

    crate::monitor::refresh_monitor_layout(&mut WmCtx::X11(ctx.reborrow()));
    crate::backend::x11::update_ewmh_desktop_props(ctx.core.state, &ctx.x11, ctx.x11_runtime);
    crate::focus::focus(&mut WmCtx::X11(ctx.reborrow()), None);
    ctx.core.queue_layout_for_all_monitors_urgent();
}

/// Reconcile logical monitors after an output/CRTC change that did not
/// necessarily resize the X11 root window.
pub fn randr_notify(ctx: &mut WmCtxX11<'_>) {
    refresh_randr_topology(ctx, None);
}

/// RandR screen-change events carry the new framebuffer dimensions and are
/// more authoritative than waiting for a separate root ConfigureNotify.
pub fn randr_screen_change_notify(
    ctx: &mut WmCtxX11<'_>,
    event: &x11rb::protocol::randr::ScreenChangeNotifyEvent,
) {
    refresh_randr_topology(ctx, Some((event.width, event.height)));
}

fn refresh_randr_topology(ctx: &mut WmCtxX11<'_>, size: Option<(u16, u16)>) {
    if let Some((width, height)) = size {
        ctx.core.derived_mut().display.width = i32::from(width);
        ctx.core.derived_mut().display.height = i32::from(height);
    }
    let monitor_config = ctx.core.config().monitors.clone();
    crate::backend::x11::randr::configure_connected_outputs(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        &monitor_config,
    );
    crate::monitor::apply_monitor_config(&mut WmCtx::X11(ctx.reborrow()));
    crate::backend::x11::randr::compact_automatic_output_layout(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        &monitor_config,
    );
    crate::backend::x11::randr::fit_framebuffer_to_active_outputs(
        ctx.x11.conn,
        ctx.x11_runtime.root,
    );
    crate::monitor::refresh_monitor_layout(&mut WmCtx::X11(ctx.reborrow()));
    crate::backend::x11::update_ewmh_desktop_props(ctx.core.state, &ctx.x11, ctx.x11_runtime);
    crate::focus::focus(&mut WmCtx::X11(ctx.reborrow()), None);
    ctx.core.queue_layout_for_all_monitors_urgent();
}

pub fn configure_request(ctx: &mut WmCtxX11<'_>, e: &ConfigureRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(current_size) = ctx
        .xembed_tray
        .as_ref()
        .and_then(|tray| tray.icon(event_win))
        .map(|icon| icon.size)
    {
        let requested_size = crate::types::Size::new(
            if e.value_mask.contains(ConfigWindow::WIDTH) {
                e.width as i32
            } else {
                current_size.w
            },
            if e.value_mask.contains(ConfigWindow::HEIGHT) {
                e.height as i32
            } else {
                current_size.h
            },
        );
        crate::backend::x11::systray::update_systray_icon_geom(
            ctx.core.derived().bar_height,
            ctx.xembed_tray.as_mut(),
            event_win,
            requested_size,
        );
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.model().client(event_win).is_some() {
        crate::backend::x11::focus::configure(ctx.core.state, &ctx.x11, event_win);
    } else {
        let conn = ctx.x11.conn;
        let _ = conn.configure_window(
            e.window,
            &ConfigureWindowAux::new()
                .x(e.x as i32)
                .y(e.y as i32)
                .width(e.width as u32)
                .height(e.height as u32)
                .border_width(e.border_width as u32),
        );
        let _ = conn.flush();
    };
}

pub fn create_notify(_e: &CreateNotifyEvent) {}

pub fn destroy_notify(ctx: &mut WmCtxX11<'_>, e: &DestroyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        // Remove tray-owned state before recomputing the paired tray/bar
        // geometry so the destroyed icon no longer reserves a cell.
        crate::backend::x11::systray::remove_systray_icon(ctx.xembed_tray.as_mut(), event_win);
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.model().client(event_win).is_some() {
        let mut tmp = ctx.reborrow();
        unmanage(&mut tmp, event_win, true);
    };
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

pub fn expose(ctx: &mut WmCtxX11<'_>, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    let event_win = WindowId::from(e.window);
    if let Some(monitor_id) = ctx
        .core
        .state
        .model
        .monitors
        .find_monitor_for(event_win, &ctx.core.model().clients)
    {
        let is_bar_win = ctx
            .core
            .state
            .model
            .monitors
            .get(monitor_id)
            .is_some_and(|m| event_win == m.bar_win);
        if is_bar_win {
            ctx.core.bar.mark_dirty();
        }
    };
}

pub fn focus_in(ctx: &mut WmCtxX11<'_>, _e: &FocusInEvent) {
    if let Some(selected_window) = ctx.core.model().selected_win() {
        crate::backend::x11::focus::set_focus(
            ctx.core.state,
            &ctx.x11,
            ctx.x11_runtime,
            selected_window,
        );
    };
}

pub fn mapping_notify(ctx: &mut WmCtxX11<'_>, _e: &MappingNotifyEvent) {
    if !crate::backend::x11::keyboard::refresh_keyboard_mapping(&ctx.x11, ctx.x11_runtime) {
        log::warn!("X11 keyboard mapping refresh failed; preserving existing passive grabs");
        return;
    }
    crate::backend::x11::keyboard::grab_keys(ctx.core.state, &ctx.x11, ctx.x11_runtime);
}

pub fn map_request(ctx: &mut WmCtxX11<'_>, e: &MapRequestEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        return;
    };

    if ctx.core.model().client(event_win).is_none() {
        let Some(initial_geometry) = query_manageable_window_geometry(&ctx.x11, event_win) else {
            return;
        };
        let mut tmp = ctx.reborrow();
        crate::backend::x11::lifecycle::manage(
            &mut tmp,
            event_win,
            initial_geometry.rect,
            initial_geometry.border_width,
        );
    };
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

pub fn property_notify(ctx: &mut WmCtxX11<'_>, e: &PropertyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        if e.atom == ctx.x11_runtime.xatom.xembed_info {
            crate::backend::x11::systray::update_systray_icon_state(
                &ctx.x11,
                ctx.x11_runtime,
                ctx.xembed_tray.as_mut(),
                event_win,
                Some(e),
            );
        }
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        return;
    };

    if ctx.core.model().client(event_win).is_some() {
        match e.atom {
            x if x == ctx.x11_runtime.wmatom.protocols => {
                let protocols = crate::backend::x11::focus::read_wm_protocols(
                    ctx.x11.conn,
                    e.window,
                    ctx.x11_runtime.wmatom.protocols,
                )
                .map(crate::backend::x11::X11ClientProtocols::Known)
                .unwrap_or_default();
                ctx.x11_runtime
                    .client_protocols
                    .insert(event_win, protocols);
            }
            x if x == u32::from(AtomEnum::WM_NORMAL_HINTS) => {
                if let Some(c) = ctx.core.model_mut().client_mut(event_win) {
                    c.size_hints_valid = false;
                }
            }
            x if x == u32::from(AtomEnum::WM_HINTS) => {
                crate::backend::x11::update_wm_hints(ctx, event_win);
                ctx.core.bar.mark_dirty();
            }
            x if x == u32::from(AtomEnum::WM_TRANSIENT_FOR) => {
                let parent =
                    crate::backend::x11::lifecycle::get_transient_for_hint(&ctx.x11, event_win);
                let monitor_id = ctx
                    .core
                    .model()
                    .client(event_win)
                    .map(|client| client.monitor_id);
                let needs_float = ctx.core.model().client(event_win).is_some_and(|client| {
                    parent.is_some()
                        && client.placement() != crate::types::ClientPlacement::Floating
                });
                if let Some(client) = ctx.core.model_mut().client_mut(event_win) {
                    client.transient_for = parent;
                }
                if needs_float {
                    let _ = crate::floating::set_window_placement_from_policy(
                        &mut WmCtx::X11(ctx.reborrow()),
                        event_win,
                        crate::floating::WindowModeRequest::Floating(
                            crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
                        ),
                    );
                }
                if let Some(monitor_id) = monitor_id {
                    crate::layouts::arrange(&mut WmCtx::X11(ctx.reborrow()), Some(monitor_id));
                }
            }
            _ => {}
        }

        let net_wm_name = ctx.x11_runtime.netatom.wm_name;
        if e.atom == u32::from(AtomEnum::WM_NAME)
            || e.atom == net_wm_name
            || e.atom == u32::from(AtomEnum::WM_CLASS)
        {
            let props =
                crate::backend::x11::window_properties(&ctx.x11, ctx.x11_runtime, event_win);
            let previous_focus = ctx.core.model().selected_win();
            if crate::client::update_window_properties(&mut ctx.core, event_win, &props) {
                crate::focus::refresh_focus_after_selection(
                    &mut WmCtx::X11(ctx.reborrow()),
                    previous_focus,
                    None,
                );
            }
        }
    };
}

pub fn resize_request(ctx: &mut WmCtxX11<'_>, e: &ResizeRequestEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        crate::backend::x11::systray::update_systray_icon_geom(
            ctx.core.derived().bar_height,
            ctx.xembed_tray.as_mut(),
            event_win,
            crate::types::Size::new(e.width as i32, e.height as i32),
        );
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    };
}

pub fn unmap_notify(ctx: &mut WmCtxX11<'_>, e: &UnmapNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        // XEmbed icons remain owned by the tray while unmapped. Recompute the
        // paired tray/bar geometry; mapped state comes from _XEMBED_INFO.
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.model().client(event_win).is_some() {
        if e.response_type & 0x80 != 0 {
            crate::backend::x11::set_client_state(
                &ctx.x11,
                ctx.x11_runtime,
                event_win,
                crate::backend::x11::constants::WM_STATE_WITHDRAWN,
            );
        } else {
            let mut tmp = ctx.reborrow();
            unmanage(&mut tmp, event_win, false);
        }
    };
}

pub fn leave_notify(ctx: &mut WmCtxX11<'_>, _e: &LeaveNotifyEvent) {
    crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
}

fn handle_systray_dock_request(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = WindowId::from(data[2]);
    if icon_win == WindowId::default() {
        return;
    };

    let systray_win_opt = ctx.xembed_tray.as_ref().map(|s| s.win);
    let statusescheme_bg_pixel = ctx.x11_runtime.status_scheme.bg.color.pixel as u32;

    let Some(systray_win) = systray_win_opt else {
        return;
    };

    let geo = {
        let conn = ctx.x11.conn;
        let x11_icon_win: Window = icon_win.into();
        conn.get_geometry(x11_icon_win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| Rect {
                x: 0,
                y: 0,
                w: wa.width as i32,
                h: wa.height as i32,
            })
            .unwrap_or(Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            })
    };

    let mapped =
        crate::backend::x11::systray::xembed_wants_mapped(&ctx.x11, ctx.x11_runtime, icon_win);
    let Some(systray) = ctx.xembed_tray.as_mut() else {
        return;
    };
    if systray.icon(icon_win).is_some() {
        return;
    }
    systray.icons.insert(
        0,
        crate::types::XEmbedIcon {
            win: icon_win,
            size: geo.size(),
            mapped,
        },
    );

    crate::backend::x11::systray::update_systray_icon_geom(
        ctx.core.derived().bar_height,
        ctx.xembed_tray.as_mut(),
        icon_win,
        geo.size(),
    );

    let conn = ctx.x11.conn;
    let x11_icon_win: Window = icon_win.into();
    let x11_systray_win: Window = systray_win.into();

    let _ = conn.change_save_set(SetMode::INSERT, x11_icon_win);
    let _ = conn.configure_window(x11_icon_win, &ConfigureWindowAux::new().border_width(0));

    let mask =
        EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE | EventMask::RESIZE_REDIRECT;
    let _ = conn.change_window_attributes(
        x11_icon_win,
        &ChangeWindowAttributesAux::new().event_mask(mask),
    );

    let _ = conn.reparent_window(x11_icon_win, x11_systray_win, 0, 0);

    let _ = conn.change_window_attributes(
        x11_icon_win,
        &ChangeWindowAttributesAux::new().background_pixel(statusescheme_bg_pixel),
    );

    let _ = conn.flush();

    crate::backend::x11::systray::update_systray(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        ctx.xembed_tray,
    );
    crate::backend::x11::systray::send_xembed_message(
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        XEmbedMessage::EmbeddedNotify {
            embedder: systray_win,
        },
    );
    if mapped {
        crate::backend::x11::systray::send_xembed_message(
            &ctx.x11,
            ctx.x11_runtime,
            icon_win,
            XEmbedMessage::WindowActivate,
        );
        crate::backend::x11::set_client_state(&ctx.x11, ctx.x11_runtime, icon_win, 1);
    }
}

fn handle_net_wm_state(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent, win: WindowId) {
    let data = e.data.as_data32();
    let action = data[0];
    let requested = |current: bool| match action {
        0 => Some(false),
        1 => Some(true),
        2 => Some(!current),
        _ => None,
    };
    let atoms = [data[1], data[2]];
    let netatom = ctx.x11_runtime.netatom;
    if atoms.contains(&netatom.wm_maximized_vert) || atoms.contains(&netatom.wm_maximized_horz) {
        let current = ctx
            .core
            .state
            .model
            .client_protocol_maximized(win)
            .unwrap_or(false);
        if let Some(maximized) = requested(current) {
            crate::client::fullscreen::apply_client_maximize_intent(
                &mut WmCtx::X11(ctx.reborrow()),
                win,
                maximized,
            );
        }
    }

    if atoms.contains(&netatom.wm_fullscreen) {
        let mode = ctx.core.state.model.client(win).map(|client| client.mode());
        let current = mode.is_some_and(|mode| mode.is_fullscreen());
        if let Some(fullscreen) = requested(current) {
            crate::client::set_fullscreen(&mut WmCtx::X11(ctx.reborrow()), win, fullscreen);
        }
    }
}

fn handle_current_desktop(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let desktop = e.data.as_data32()[0];
    let Some((monitor_id, tag_index)) =
        crate::backend::x11::properties::monitor_tag_for_desktop(ctx.core.model(), desktop)
    else {
        return;
    };
    let Some(mask) = TagMask::single(tag_index) else {
        return;
    };

    crate::overview::exit_overview(
        &mut WmCtx::X11(ctx.reborrow()),
        crate::overview::ExitMode::RestorePrevious,
    );
    crate::focus::select_monitor(&mut WmCtx::X11(ctx.reborrow()), monitor_id);
    crate::tags::view::view_tags(&mut WmCtx::X11(ctx.reborrow()), mask);
}

fn handle_wm_desktop(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent, win: WindowId) {
    let desktop = e.data.as_data32()[0];

    if desktop == u32::MAX {
        if ctx
            .core
            .model()
            .client(win)
            .is_some_and(|client| client.is_scratchpad())
        {
            crate::backend::x11::set_client_tag_prop(
                ctx.core.state,
                &ctx.x11,
                ctx.x11_runtime,
                win,
            );
            return;
        }
        if let Some(client) = ctx.core.model_mut().client_mut(win) {
            client.is_sticky = true;
        }
        crate::backend::x11::set_client_tag_prop(ctx.core.state, &ctx.x11, ctx.x11_runtime, win);
        ctx.core.queue_layout_for_all_monitors_urgent();
        return;
    }

    let Some((target_mon, tag_index)) =
        crate::backend::x11::properties::monitor_tag_for_desktop(ctx.core.model(), desktop)
    else {
        return;
    };
    let Some(target_tags) = TagMask::single(tag_index) else {
        return;
    };

    if ctx
        .core
        .model()
        .client(win)
        .is_some_and(|client| client.is_scratchpad())
    {
        let _ = crate::floating::scratchpad::scratchpad_restore_window(
            &mut WmCtx::X11(ctx.reborrow()),
            win,
            Some((target_mon, target_tags)),
        );
        return;
    }

    let old_mon = ctx.core.model().client(win).map(|client| client.monitor_id);
    let previous_focus = ctx.core.model().selected_win();
    let reassigned = ctx.core.mutate_selection(|model| {
        if let Some(client) = model.client_mut(win) {
            client.is_sticky = false;
            client.set_tag_mask(target_tags);
        } else {
            return false;
        }
        model.reassign_client_monitor(win, target_mon)
    });
    debug_assert!(reassigned, "validated EWMH monitor transfer must succeed");
    if !reassigned {
        return;
    }

    crate::backend::x11::set_client_tag_prop(ctx.core.state, &ctx.x11, ctx.x11_runtime, win);
    crate::focus::refresh_focus_after_selection(
        &mut WmCtx::X11(ctx.reborrow()),
        previous_focus,
        None,
    );

    if old_mon == Some(target_mon) {
        ctx.core.queue_layout_for_monitor_urgent(target_mon);
    } else {
        ctx.core.queue_layout_for_all_monitors_urgent();
    }
}

fn handle_active_window(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let is_hidden = ctx
        .core
        .model()
        .client(win)
        .is_some_and(|client| client.is_hidden);
    if is_hidden {
        crate::client::show_window(&mut WmCtx::X11(ctx.reborrow()), win);
    };

    let _ = crate::focus::activate_client(&mut WmCtx::X11(ctx.reborrow()), win);
}
