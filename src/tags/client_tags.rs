//! Client-to-tag assignment.

use crate::contexts::WmCtx;
use crate::types::{TagMask, WindowId};

pub fn set_client_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
    let tagmask = ctx.core().globals().tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.clear_sticky_if_scratchpad();
        client.set_tag_mask(effective_mask);
    } else {
        return;
    }

    if let WmCtx::X11(x11) = ctx {
        crate::backend::x11::set_client_tag_prop(&x11.core, &x11.x11, x11.x11_runtime, win);
    }
    crate::focus::focus(ctx, None);
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selmon_id);
}

pub fn tag_all(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
    let tagmask = ctx.core().globals().tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let current_tag = ctx.core().globals().selected_monitor().current_tag_index();
    let Some(current_tag) = current_tag else {
        return;
    };
    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);

    let m = ctx.core().globals().selected_monitor();
    let clients_on_tag: Vec<_> = m
        .iter_clients(ctx.core().globals().clients.map())
        .filter(|(_, c)| c.tags.intersects(current_tag_mask))
        .map(|(win, _)| win)
        .collect();

    for win in clients_on_tag {
        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            client.clear_sticky_if_scratchpad();
            client.set_tag_mask(effective_mask);
        }
    }

    crate::focus::focus(ctx, None);
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selmon_id);
}

pub fn follow_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    set_client_tag(ctx, win, mask);
    crate::tags::view::view_tags(ctx, mask);
}

pub fn toggle_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let tagmask = ctx.core().globals().tags.mask();
    let current_tags = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map_or(TagMask::EMPTY, |c| c.tags);
    if current_tags.is_scratchpad_only() {
        set_client_tag(ctx, win, mask);
        return;
    }
    let new_tags = current_tags ^ (mask & tagmask);
    if new_tags.is_empty() {
        return;
    }
    set_client_tag(ctx, win, new_tags);
}
