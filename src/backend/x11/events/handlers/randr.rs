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
    crate::backend::x11::randr::refresh_topology(
        ctx.x11.conn,
        ctx.x11_runtime,
        &ctx.core.derived().monitor_policy,
    );
    crate::monitor::refresh_monitor_layout(&mut WmCtx::X11(ctx.reborrow()));
    crate::backend::x11::update_ewmh_desktop_props(ctx.core.state, &ctx.x11, ctx.x11_runtime);
    crate::focus::focus(&mut WmCtx::X11(ctx.reborrow()), None);
    ctx.core.queue_layout_for_all_monitors_urgent();
}
