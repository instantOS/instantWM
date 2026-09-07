use crate::contexts::{WmCtx, WmCtxX11};
use x11rb::protocol::randr::ScreenChangeNotifyEvent;

/// Reconcile logical monitors after an output/CRTC change that did not
/// necessarily resize the X11 root window.
pub fn randr_notify(ctx: &mut WmCtxX11<'_>) {
    refresh_randr_topology(ctx, None);
}

/// RandR screen-change events carry the new framebuffer dimensions and are
/// more authoritative than waiting for a separate root ConfigureNotify.
pub fn randr_screen_change_notify(ctx: &mut WmCtxX11<'_>, event: &ScreenChangeNotifyEvent) {
    refresh_randr_topology(ctx, Some((event.width, event.height)));
}

fn refresh_randr_topology(ctx: &mut WmCtxX11<'_>, size: Option<(u16, u16)>) {
    if let Some((width, height)) = size {
        ctx.core.derived_mut().display.width = i32::from(width);
        ctx.core.derived_mut().display.height = i32::from(height);
    }
    let monitor_config = ctx.core.config().monitors.clone();
    let connected =
        crate::backend::x11::randr::connected_output_names(ctx.x11.conn, ctx.x11_runtime.root);
    let active_before =
        crate::backend::x11::randr::active_output_names(ctx.x11.conn, ctx.x11_runtime.root);

    // A CRTC disappearing while its physical connector remains present is an
    // external disable, not a hot-plug. Relinquish automatic placement and do
    // not queue it for re-enabling.
    for name in ctx
        .x11_runtime
        .active_outputs
        .difference(&active_before)
        .filter(|name| connected.contains(*name))
    {
        ctx.x11_runtime.automatic_outputs.remove(name);
    }

    ctx.x11_runtime.pending_output_enable.retain(|name| {
        connected.contains(name)
            && !crate::backend::x11::randr::output_is_explicitly_disabled(&monitor_config, name)
    });
    ctx.x11_runtime
        .automatic_outputs
        .retain(|name| connected.contains(name));
    ctx.x11_runtime.pending_output_enable.extend(
        crate::backend::x11::randr::new_auto_enable_candidates(
            &ctx.x11_runtime.connected_outputs,
            &connected,
            &active_before,
            &monitor_config,
        ),
    );
    ctx.x11_runtime.connected_outputs = connected;

    let pending = ctx.x11_runtime.pending_output_enable.clone();
    let (activated, automatic) = crate::backend::x11::randr::configure_new_outputs(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        &monitor_config,
        &pending,
    );
    ctx.x11_runtime
        .pending_output_enable
        .retain(|name| !activated.contains(name));
    ctx.x11_runtime.automatic_outputs.extend(automatic);

    crate::backend::x11::randr::apply_monitor_configs(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        &monitor_config,
    );
    crate::backend::x11::randr::compact_automatic_output_layout(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        &monitor_config,
        &ctx.x11_runtime.automatic_outputs,
    );
    crate::backend::x11::randr::fit_framebuffer_to_active_outputs(
        ctx.x11.conn,
        ctx.x11_runtime.root,
    );
    crate::monitor::refresh_monitor_layout(&mut WmCtx::X11(ctx.reborrow()));
    let active_after =
        crate::backend::x11::randr::active_output_names(ctx.x11.conn, ctx.x11_runtime.root);
    ctx.x11_runtime
        .automatic_outputs
        .retain(|name| active_after.contains(name));
    ctx.x11_runtime.active_outputs = active_after;
    crate::backend::x11::update_ewmh_desktop_props(ctx.core.state, &ctx.x11, ctx.x11_runtime);
    crate::focus::focus(&mut WmCtx::X11(ctx.reborrow()), None);
    ctx.core.queue_layout_for_all_monitors_urgent();
}
