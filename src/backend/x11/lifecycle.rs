//! Client lifecycle: adopting and releasing X11 windows.
//!
//! Note on title initialization: `update_title` writes into `globals.model.clients`,
//! so we cannot use it before the client is inserted.  Instead we call the
//! shared property reader (which returns a `String`) and store the
//! result directly on the local `Client` before insertion.
//!
//! # The two entry points
//!
//! * [`manage`]   – called when the WM first sees a window (either at startup
//!   via `QueryTree`, or at runtime via a `MapRequest` event).
//!   Builds a [`Client`], attaches it to the correct monitor and linked lists,
//!   applies rules/hints, and arranges the monitor.
//!
//! * [`unmanage`] – called when a window is destroyed or deliberately withdrawn.
//!   Detaches it from every list, optionally restores X11 state (border, event
//!   mask, WM_STATE), and re-focuses.
//!
//! # Monitor assignment
//!
//! A new window inherits its monitor from its transient-for parent when one
//! exists; otherwise it goes to the currently selected monitor. After
//! [`crate::client::apply_rules`] runs, the assignment may be
//! overridden again by a matching rule.
//!
//! # Animation
//!
//! When the global `animated` flag is set, newly managed windows slide in from
//! 70 px above their final position.  Fullscreen windows skip the animation.

use crate::backend::WindowOps;
use crate::backend::x11::X11BackendRef;
use crate::backend::x11::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL, WM_STATE_WITHDRAWN};
use crate::backend::x11::focus::grab_buttons;
use crate::backend::x11::{
    X11RuntimeConfig, set_client_state, set_client_tag_prop, update_motif_hints,
    update_window_type, update_wm_hints,
};
use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::geometry::{GeometryApplyMode, MoveResizeOptions};
// focus() is used via focus_soft() in this module
use crate::focus::focus;
use crate::layouts::arrange;
use crate::types::{BaseClientMode, Client, Rect, TagMask, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

// ---------------------------------------------------------------------------
// manage
// ---------------------------------------------------------------------------

pub fn manage(
    ctx: &mut WmCtxX11,
    window: WindowId,
    initial_geometry: Rect,
    original_border_width: u32,
) {
    let transient_for = get_transient_for_hint(&ctx.x11, window);
    let x11_runtime = &*ctx.x11_runtime;
    let mut client = build_initial_client(&ctx.x11, x11_runtime, window, initial_geometry);
    client.transient_for = transient_for;
    let launch_context = read_launch_context(ctx.core.pending_launches_mut(), &ctx.x11, window);
    if !assign_initial_monitor_and_tags(
        ctx.core.state_mut(),
        &mut client,
        transient_for,
        launch_context,
    ) {
        return;
    }
    let Some(rule_placement) = insert_client_and_apply_rules(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        window,
        client,
        launch_context,
    ) else {
        return;
    };
    ctx.x11_runtime
        .original_border_widths
        .insert(window, original_border_width);

    let border_px = ctx.core.config().window.border_width_px;
    apply_default_border(ctx.core.model_mut(), border_px, window);
    let (monitor_work_rect, monitor_rect) = monitor_rects_for_client(ctx.core.model(), window);
    clamp_client_to_work_area(ctx.core.model_mut(), window, monitor_work_rect);
    let is_maximized = is_maximized_on_client_monitor(ctx.core.model(), window);
    let bar_height = ctx.core.config().derived.bar_height;
    configure_client_border(
        ctx.core.model_mut(),
        bar_height,
        &ctx.x11,
        ctx.x11_runtime,
        window,
        border_px,
        monitor_rect,
        is_maximized,
    );

    let hinted_position_is_explicit = apply_manage_hints(ctx, window);
    let position_is_explicit = match rule_placement {
        crate::client::InitialRulePlacement::Default => hinted_position_is_explicit,
        crate::client::InitialRulePlacement::Center => false,
        crate::client::InitialRulePlacement::Preserve => true,
    };
    subscribe_manage_events(&ctx.x11, window);
    grab_buttons(ctx.core.state(), &ctx.x11, ctx.x11_runtime, window, false);

    if initialize_floating_state(ctx.core.model_mut(), window, transient_for.is_some()) {
        if let Some(rect) = crate::client::sane_floating_spawn_rect(
            ctx.core.model(),
            window,
            transient_for,
            position_is_explicit,
        ) {
            crate::client::sync_client_geometry(ctx.core.model_mut(), window, rect);
        }
        ctx.x11.raise_window_visual_only(window);
        ctx.x11.flush();
    }

    let attached = ctx.core.model_mut().attach_client(window);
    debug_assert!(attached, "managed X11 client must have a valid monitor");

    register_client_root(&ctx.x11, ctx.x11_runtime, window);

    move_client_offscreen_before_arrange(&mut WmCtx::X11(ctx.reborrow()), window);
    let initially_hidden = prepare_visibility(&mut WmCtx::X11(ctx.reborrow()), window);
    let animated = ctx.core.behavior().animated;
    let client =
        arrange_map_focus_and_snapshot(&mut WmCtx::X11(ctx.reborrow()), window, initially_hidden);

    run_manage_animation(
        &mut WmCtx::X11(ctx.reborrow()),
        window,
        &client,
        monitor_rect,
        animated,
    );
}

fn build_initial_client(
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    window: WindowId,
    initial_geometry: Rect,
) -> Client {
    let mut client = Client::new(window);
    client.geo = initial_geometry;
    client.old_geo = client.geo;
    client.set_preferred_floating_size(initial_geometry.size());
    client.name = crate::backend::x11::properties::read_window_title(x11, x11_runtime, window);
    client
}

fn assign_initial_monitor_and_tags(
    state: &mut crate::core_state::CoreState,
    client: &mut Client,
    transient_for: Option<WindowId>,
    launch_context: Option<crate::client::LaunchContext>,
) -> bool {
    if let Some(view) = transient_for.and_then(|window| state.model.client_view(window)) {
        client.monitor_id = view.monitor.id();
        client.set_tag_mask(view.client.tags);
        return true;
    }
    if let Some(launch_context) = launch_context
        && state.model.monitor(launch_context.monitor_id).is_some()
    {
        client.monitor_id = launch_context.monitor_id;
        client.set_tag_mask(launch_context.tags);
        client.set_base_mode(if launch_context.is_floating {
            BaseClientMode::Floating
        } else {
            BaseClientMode::Tiling
        });
        return true;
    }
    let Some(selected_monitor) = state.model.selected_monitor() else {
        return false;
    };
    client.monitor_id = selected_monitor.id();
    client.set_tag_mask(selected_monitor.selected_tags());
    true
}

fn insert_client_and_apply_rules(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    window: WindowId,
    mut client: Client,
    launch_context: Option<crate::client::LaunchContext>,
) -> Option<crate::client::InitialRulePlacement> {
    client.is_hidden =
        crate::backend::x11::visibility::get_state(x11, x11_runtime.wmatom.state, window)
            == crate::backend::x11::constants::WM_STATE_ICONIC;
    if !core.model_mut().insert_client(client) {
        return None;
    }
    let properties = crate::backend::x11::window_properties(x11, x11_runtime, window);
    let outcome =
        crate::client::apply_initial_rules(core.state_mut(), window, &properties, launch_context);
    if outcome.changed {
        core.queue_layout_for_client(window);
    }
    Some(outcome.placement)
}

fn read_launch_context(
    pending_launches: &mut std::collections::VecDeque<crate::client::PendingLaunch>,
    x11: &X11BackendRef<'_>,
    window: WindowId,
) -> Option<crate::client::LaunchContext> {
    let startup_id = read_string_prop(x11, window, "_NET_STARTUP_ID");
    let pid = read_u32_prop(x11, window, "_NET_WM_PID");
    crate::client::take_pending_launch(pending_launches, pid, startup_id.as_deref())
}

fn read_string_prop(x11: &X11BackendRef<'_>, window: WindowId, atom_name: &str) -> Option<String> {
    let atom = x11
        .conn
        .intern_atom(false, atom_name.as_bytes())
        .ok()?
        .reply()
        .ok()?
        .atom;
    if atom == 0 {
        return None;
    }
    let x11_window: Window = window.into();
    let reply = x11
        .conn
        .get_property(false, x11_window, atom, AtomEnum::ANY, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if reply.format != 8 || reply.value.is_empty() {
        return None;
    }
    let len = reply
        .value
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(reply.value.len());
    Some(String::from_utf8_lossy(&reply.value[..len]).into_owned()).filter(|s| !s.is_empty())
}

fn read_u32_prop(x11: &X11BackendRef<'_>, window: WindowId, atom_name: &str) -> Option<u32> {
    let atom = x11
        .conn
        .intern_atom(false, atom_name.as_bytes())
        .ok()?
        .reply()
        .ok()?
        .atom;
    if atom == 0 {
        return None;
    }
    let x11_window: Window = window.into();
    x11.conn
        .get_property(false, x11_window, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}

fn apply_default_border(model: &mut crate::model::WmModel, border_px: i32, window: WindowId) {
    if let Some(client) = model.client_mut(window) {
        client.border_width = border_px;
        client.old_border_width = border_px;
    }
}

fn monitor_rects_for_client(model: &crate::model::WmModel, window: WindowId) -> (Rect, Rect) {
    let view = model
        .client_view(window)
        .expect("newly managed client must have an assigned monitor");
    (view.monitor.work_rect(), view.monitor.monitor_rect)
}

fn clamp_client_to_work_area(
    model: &mut crate::model::WmModel,
    window: WindowId,
    monitor_work_rect: Rect,
) {
    if let Some(client) = model.client_mut(window) {
        client
            .geo
            .clamp_position(&monitor_work_rect, client.total_width(), client.total_height());
    }
}

fn is_maximized_on_client_monitor(model: &crate::model::WmModel, window: WindowId) -> bool {
    model
        .client_view(window)
        .is_some_and(|view| view.monitor.is_maximized_layout())
}

fn configure_client_border(
    model: &mut crate::model::WmModel,
    bar_height: i32,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    window: WindowId,
    border_px: i32,
    monitor_rect: Rect,
    is_maximized: bool,
) {
    let Some(client) = model.client_mut(window) else {
        return;
    };

    let border_width = if client.mode().is_tiling()
        && is_maximized
        && client.geo.w > monitor_rect.w - 30
        && client.geo.h > monitor_rect.h - 30 - bar_height
    {
        0
    } else {
        border_px
    };

    client.border_width = border_width;

    let x11_window: Window = window.into();
    let pixel = x11_runtime.border_scheme.normal.bg.pixel();
    let _ = x11.conn.change_window_attributes(
        x11_window,
        &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
    );
    let _ = x11.conn.flush();
}

fn apply_manage_hints(ctx_x11: &mut WmCtxX11<'_>, window: WindowId) -> bool {
    crate::backend::x11::focus::configure(ctx_x11.core.state(), &ctx_x11.x11, window);
    update_window_type(ctx_x11, window);
    let size_hints =
        crate::backend::x11::update_size_hints(ctx_x11.core.model_mut(), &ctx_x11.x11, window);
    update_wm_hints(ctx_x11, window);
    read_client_info(
        ctx_x11.core.model_mut(),
        &ctx_x11.x11,
        ctx_x11.x11_runtime,
        window,
    );
    read_wm_desktop_hint(
        ctx_x11.core.model_mut(),
        &ctx_x11.x11,
        ctx_x11.x11_runtime,
        window,
    );
    set_client_tag_prop(
        ctx_x11.core.state(),
        &ctx_x11.x11,
        ctx_x11.x11_runtime,
        window,
    );
    update_motif_hints(ctx_x11, window);
    size_hints.is_some_and(|hints| hints.position.is_some())
}

fn subscribe_manage_events(x11: &X11BackendRef, window: WindowId) {
    let mask = EventMask::ENTER_WINDOW
        | EventMask::FOCUS_CHANGE
        | EventMask::PROPERTY_CHANGE
        | EventMask::STRUCTURE_NOTIFY;
    let x11_window: Window = window.into();
    let _ = x11.conn.change_window_attributes(
        x11_window,
        &ChangeWindowAttributesAux::new().event_mask(mask),
    );
}

fn initialize_floating_state(
    model: &mut crate::model::WmModel,
    window: WindowId,
    has_transient_parent: bool,
) -> bool {
    if let Some(client) = model.client_mut(window) {
        if client.base_mode() != BaseClientMode::Floating {
            client.set_base_mode(if has_transient_parent || client.is_fixed_size {
                BaseClientMode::Floating
            } else {
                BaseClientMode::Tiling
            });
        }
        client.base_mode() == BaseClientMode::Floating
    } else {
        false
    }
}

fn register_client_root(x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig, window: WindowId) {
    let x11_window: Window = window.into();
    let _ = x11.conn.change_property32(
        PropMode::APPEND,
        x11_runtime.root,
        x11_runtime.netatom.client_list,
        AtomEnum::WINDOW,
        &[x11_window],
    );
    let _ = x11.conn.flush();
}

fn move_client_offscreen_before_arrange(ctx: &mut WmCtx, window: WindowId) {
    let (screen_width, client_x, client_y, client_width, client_height) = ctx
        .core()
        .state()
        .model
        .client(window)
        .map(|client| {
            (
                ctx.core().config().derived.display.width,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
            )
        })
        .unwrap_or((0, 0, 0, 0, 0));

    ctx.set_geometry_impl(
        window,
        Rect {
            x: client_x + 2 * screen_width,
            y: client_y,
            w: client_width,
            h: client_height,
        },
        GeometryApplyMode::VisualOnly,
    );
}

fn prepare_visibility(ctx: &mut WmCtx, window: WindowId) -> bool {
    let initially_hidden = ctx
        .core()
        .state()
        .model
        .client(window)
        .map(|client| client.is_hidden)
        .unwrap_or(false);
    if !initially_hidden && let WmCtx::X11(ctx_x11) = ctx {
        set_client_state(&ctx_x11.x11, ctx_x11.x11_runtime, window, WM_STATE_NORMAL);
    }
    initially_hidden
}

fn arrange_map_focus_and_snapshot(
    ctx: &mut WmCtx,
    window: WindowId,
    initially_hidden: bool,
) -> Client {
    let mut client = ctx
        .core()
        .state()
        .model
        .client(window)
        .cloned()
        .expect("managed client must exist before arrange");
    let monitor_id = client.monitor_id;
    arrange(ctx, Some(monitor_id));
    if !initially_hidden {
        ctx.window_backend().map_window(window);
        ctx.window_backend().flush();
    }
    // Route initial selection through the normal focus transaction. Passing
    // the managed window explicitly ensures backend focus, histories and
    // persistent z-order are updated together. Hidden windows are rejected by
    // focus target resolution and fall back to the previous visible target.
    focus(ctx, Some(window));
    client = ctx
        .core()
        .state()
        .model
        .client(window)
        .cloned()
        .expect("managed client must exist after arrange");
    client
}

fn run_manage_animation(
    ctx: &mut WmCtx,
    window: WindowId,
    client: &Client,
    monitor_rect: Rect,
    animated: bool,
) {
    if !animated || client.mode().is_fullscreen() {
        return;
    }

    ctx.move_resize(
        window,
        client.geo,
        MoveResizeOptions::animate_from(
            Rect {
                x: client.geo.x,
                y: monitor_rect.y - client.geo.h - client.border_width * 2,
                w: client.geo.w,
                h: client.geo.h,
            },
            DEFAULT_FRAME_COUNT,
        ),
    );

    let is_tiling = ctx
        .core()
        .model()
        .client_view(window)
        .is_some_and(|view| view.monitor.is_tiling_layout());

    if !is_tiling {
        ctx.window_backend().raise_window_visual_only(window);
        ctx.window_backend().flush();
    } else if client.geo.w > monitor_rect.w - 30 || client.geo.h > monitor_rect.h - 30 {
        arrange(ctx, Some(client.monitor_id));
    }
}

// ---------------------------------------------------------------------------
// unmanage
// ---------------------------------------------------------------------------

/// Release a window from WM management.
///
/// `destroyed` should be `true` when this is called in response to a
/// `DestroyNotify` event (the X server has already destroyed the window; any
/// attempt to configure it will fail).  When `false` (e.g. a `UnmapNotify`
/// from a deliberately withdrawn window) we restore the border width and clear
/// the event mask / WM_STATE.
///
pub fn unmanage(ctx: &mut WmCtxX11, window: WindowId, destroyed: bool) {
    let original_border_width = ctx.x11_runtime.original_border_widths.remove(&window);

    if !destroyed {
        let x11_window: Window = window.into();
        {
            let _grab = crate::backend::x11::ServerGrab::new(ctx.x11.conn);
            if let Some(border_width) = original_border_width {
                let _ = ctx.x11.conn.configure_window(
                    x11_window,
                    &ConfigureWindowAux::new().border_width(border_width),
                );
            }
            let _ = ctx.x11.conn.change_window_attributes(
                x11_window,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );
            let _ =
                ctx.x11
                    .conn
                    .ungrab_button(ButtonIndex::from(0u8), x11_window, ModMask::from(0u16));
            let _ = ctx
                .x11
                .conn
                .delete_property(x11_window, ctx.x11_runtime.netatom.wm_desktop);

            set_client_state(&ctx.x11, ctx.x11_runtime, window, WM_STATE_WITHDRAWN);
        }
    }

    let mut tmp = WmCtx::X11(ctx.reborrow());
    crate::client::lifecycle::remove_managed_client(&mut tmp, window);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the `_NET_CLIENT_INFO` property and restore the window's tags and monitor.
///
/// This is used to persist client state across WM restarts: when the WM starts
/// up it re-manages all existing windows, and this call recovers the tag
/// assignment and monitor that were set in the previous session.
fn read_client_info(
    model: &mut crate::model::WmModel,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    window: WindowId,
) {
    let x11_window: Window = window.into();
    let client_info_atom = x11_runtime.netatom.client_info;

    let Ok(cookie) = x11.conn.get_property(
        false,
        x11_window,
        client_info_atom,
        AtomEnum::CARDINAL,
        0,
        2,
    ) else {
        return;
    };
    let Ok(reply) = cookie.reply() else { return };
    let Some(mut data) = reply.value32() else {
        return;
    };

    let tags = data.next().unwrap_or(0);
    let monitor_number = data.next().unwrap_or(0);

    let target_monitor = model
        .monitors_iter()
        .find(|(_id, monitor)| monitor.num as u32 == monitor_number)
        .map(|(monitor_id, _monitor)| monitor_id);

    if let Some(client) = model.client_mut(window) {
        client.set_tag_mask(crate::types::TagMask::from_bits(tags));
        if let Some(monitor_id) = target_monitor {
            client.monitor_id = monitor_id;
        }
    }
}

/// Read the standard `_NET_WM_DESKTOP` hint and apply it to the
/// just-managed client. This covers clients that request a desktop before map.
fn read_wm_desktop_hint(
    model: &mut crate::model::WmModel,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    window: WindowId,
) {
    let x11_window: Window = window.into();
    let wm_desktop_atom = x11_runtime.netatom.wm_desktop;

    let Ok(cookie) =
        x11.conn
            .get_property(false, x11_window, wm_desktop_atom, AtomEnum::CARDINAL, 0, 1)
    else {
        return;
    };
    let Ok(reply) = cookie.reply() else { return };
    let Some(mut data) = reply.value32() else {
        return;
    };
    let Some(desktop) = data.next() else { return };

    if desktop == u32::MAX {
        if let Some(client) = model.client_mut(window) {
            client.is_sticky = true;
        }
        return;
    }

    let Some((monitor_id, tag_index)) =
        crate::backend::x11::properties::monitor_tag_for_desktop(model, desktop)
    else {
        return;
    };
    let Some(tags) = TagMask::single(tag_index) else {
        return;
    };

    if let Some(client) = model.client_mut(window) {
        client.monitor_id = monitor_id;
        client.is_sticky = false;
        client.clear_sticky_if_scratchpad();
        client.set_tag_mask(tags);
    }
}

pub(crate) fn get_transient_for_hint(x11: &X11BackendRef, window: WindowId) -> Option<WindowId> {
    let x11_window: Window = window.into();

    x11.conn
        .get_property(
            false,
            x11_window,
            AtomEnum::WM_TRANSIENT_FOR,
            AtomEnum::WINDOW,
            0,
            1,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
        .map(WindowId::from)
}

use crate::backend::x11::ServerGrab;
use crate::wm::Wm;
use x11rb::protocol::xproto::{ConfigureWindowAux, Window};

pub fn cleanup(wm: &mut Wm) {
    let x11_data = match wm.backend.x11_data_mut() {
        Some(data) => data,
        None => return,
    };
    let conn = &x11_data.conn;
    let x11_runtime = &mut x11_data.x11_runtime;

    let _grab = ServerGrab::new(conn);

    for (_monitor_id, monitor) in wm.core.monitors_iter() {
        for (window, _client) in monitor.iter_clients(&wm.core.model.clients) {
            let Some(&original_border_width) = x11_runtime.original_border_widths.get(&window)
            else {
                continue;
            };
            let x11_window: Window = window.into();
            let _ = conn.configure_window(
                x11_window,
                &ConfigureWindowAux::new().border_width(original_border_width),
            );
        }
    }

    let wm_check_window = x11_runtime.wm_check_win;
    if wm_check_window != 0 {
        let _ = conn.destroy_window(wm_check_window);
    }

    let root = x11_runtime.root;
    let _ = conn.delete_property(root, x11_runtime.netatom.supported);
    let _ = conn.delete_property(root, x11_runtime.netatom.wm_check);

    if let Some(ref drawing_context) = x11_runtime.draw {
        for cursor in x11_runtime.cursors.iter().flatten() {
            drawing_context.cur_free(cursor);
        }
    }

    let _ = conn.flush();
}

pub fn is_window_iconic(
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    window: WindowId,
) -> bool {
    let x11_window: Window = window.into();

    let state_atom = x11_runtime.wmatom.state;
    let Ok(cookie) = x11
        .conn
        .get_property(false, x11_window, state_atom, state_atom, 0, 2)
    else {
        return false;
    };
    let Ok(reply) = cookie.reply() else {
        return false;
    };

    reply
        .value32()
        .and_then(|mut it| it.next())
        .map(|state| state as i32 == WM_STATE_ICONIC)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::initialize_floating_state;
    use crate::model::WmModel;
    use crate::types::{BaseClientMode, Client, ClientMode, WindowId};

    #[test]
    fn transient_policy_changes_fullscreen_restore_mode_without_exiting() {
        let mut model = WmModel::default();
        let win = WindowId(71);
        let mut client = Client::new(win);
        client.enter_fullscreen();
        model.insert_client(client);

        assert!(initialize_floating_state(&mut model, win, true));

        let client = model.client(win).unwrap();
        assert!(client.mode().is_true_fullscreen());
        assert_eq!(client.base_mode(), BaseClientMode::Floating);
        assert_eq!(client.mode().restored(), ClientMode::Floating);
    }
}
