//! Client-to-tag assignment.

use crate::contexts::WmCtx;
use crate::layouts::arrange;
use crate::types::{TagMask, WindowId};

pub fn set_client_tag_ctx(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.core().globals().tags.mask());
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        if TagMask::from_bits(client.tags).is_scratchpad_only() {
            client.issticky = false;
        }
        client.set_tag_mask(effective_mask);
    } else {
        return;
    }

    if let WmCtx::X11(x11) = ctx {
        crate::client::set_client_tag_prop(&x11.core, &x11.x11, x11.x11_runtime, win);
    }
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn tag_all_ctx(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.core().globals().tags.mask());
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let current_tag = ctx.core().globals().selected_monitor().current_tag;
    if current_tag == 0 {
        return;
    }
    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);

    let m = ctx.core().globals().selected_monitor();
    let clients_on_tag: Vec<_> = m
        .iter_clients(ctx.core().globals().clients.map())
        .filter(|(_, c)| TagMask::from_bits(c.tags).intersects(current_tag_mask))
        .map(|(win, _)| win)
        .collect();

    for win in clients_on_tag {
        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            if TagMask::from_bits(client.tags).is_scratchpad_only() {
                client.issticky = false;
            }
            client.set_tag_mask(effective_mask);
        }
    }

    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn follow_tag_ctx(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    set_client_tag_ctx(ctx, win, mask);
    crate::tags::view::view(ctx, mask);
}

pub fn toggle_tag_ctx(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let tagmask = TagMask::from_bits(ctx.core().globals().tags.mask());
    let current_tags = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map_or(TagMask::EMPTY, |c| TagMask::from_bits(c.tags));
    if current_tags.is_scratchpad_only() {
        set_client_tag_ctx(ctx, win, mask);
        return;
    }
    let new_tags = current_tags ^ (mask & tagmask);
    if new_tags.is_empty() {
        return;
    }
    set_client_tag_ctx(ctx, win, new_tags);
}
