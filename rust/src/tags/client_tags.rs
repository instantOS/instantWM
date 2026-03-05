//! Client-to-tag assignment.

use crate::contexts::{WmCtx, WmCtxX11};
use crate::layouts::arrange;
use crate::types::{TagMask, WindowId, SCRATCHPAD_MASK};

/// Set the selected client's tags using type-safe mask.
pub fn set_client_tag(ctx: &mut WmCtxX11, win: WindowId, mask: TagMask) {
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    set_client_tag_ctx(&mut wm_ctx, win, mask);
}

/// Tag all clients on current tag with the given mask.
pub fn tag_all(ctx: &mut WmCtxX11, mask: TagMask) {
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    tag_all_ctx(&mut wm_ctx, mask);
}

/// Toggle tags on the selected client.
pub fn toggle_tag(ctx: &mut WmCtxX11, win: WindowId, mask: TagMask) {
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    toggle_tag_ctx(&mut wm_ctx, win, mask);
}

/// Follow a tag (move client to tag and view it).
pub fn follow_tag(ctx: &mut WmCtxX11, win: WindowId, mask: TagMask) {
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    follow_tag_ctx(&mut wm_ctx, win, mask);
}

pub fn set_client_tag_ctx(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let selmon_id = ctx.g_mut().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.g().tags.mask());
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);
    if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
        if TagMask::from_bits(client.tags) == scratchpad {
            client.issticky = false;
        }
        client.tags = effective_mask.bits();
    } else {
        return;
    }

    if let WmCtx::X11(x11) = ctx {
        crate::client::set_client_tag_prop(&x11.core, &x11.x11, win);
    }
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn tag_all_ctx(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g_mut().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.g().tags.mask());
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let current_tag = ctx.g().selected_monitor().current_tag;
    if current_tag == 0 {
        return;
    }
    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);
    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    let m = ctx.g().selected_monitor();
    let clients_on_tag: Vec<_> = m
        .iter_clients(ctx.g().clients.map())
        .filter(|(_, c)| TagMask::from_bits(c.tags).intersects(current_tag_mask))
        .map(|(win, _)| win)
        .collect();

    for win in clients_on_tag {
        if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            if TagMask::from_bits(client.tags) == scratchpad {
                client.issticky = false;
            }
            client.tags = effective_mask.bits();
        }
    }

    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn follow_tag_ctx(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let had_prefix = ctx.g().tags.prefix;
    set_client_tag_ctx(ctx, win, mask);
    if had_prefix {
        ctx.g_mut().tags.prefix = true;
    }
    crate::tags::view::view_ctx(ctx, mask);
}

pub fn toggle_tag_ctx(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let tagmask = TagMask::from_bits(ctx.g().tags.mask());
    let current_tags = ctx
        .g()
        .clients
        .get(&win)
        .map_or(TagMask::EMPTY, |c| TagMask::from_bits(c.tags));
    if current_tags.bits() == SCRATCHPAD_MASK {
        set_client_tag_ctx(ctx, win, mask);
        return;
    }
    let new_tags = current_tags ^ (mask & tagmask);
    if new_tags.is_empty() {
        return;
    }
    set_client_tag_ctx(ctx, win, new_tags);
}
