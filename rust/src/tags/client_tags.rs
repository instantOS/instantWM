//! Client-to-tag assignment.

use crate::client::set_client_tag_prop;
use crate::contexts::WmCtx;
use crate::focus::focus;
use crate::layouts::arrange;
use crate::types::{TagMask, SCRATCHPAD_MASK};
use x11rb::protocol::xproto::Window;

/// Set the selected client's tags using type-safe mask.
pub fn set_client_tag(ctx: &mut WmCtx, win: Window, mask: TagMask) {
    let selmon_id = ctx.g.selmon;
    let tagmask = TagMask::from_bits(ctx.g.tags.mask());
    let effective_mask = mask & tagmask;

    if effective_mask.is_empty() {
        return;
    }

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        if TagMask::from_bits(client.tags) == scratchpad {
            client.issticky = false;
        }
        client.tags = effective_mask.bits();
    }

    set_client_tag_prop(ctx, win);
    focus(ctx, None);
    arrange(ctx, Some(selmon_id));
}

/// Tag all clients on current tag with the given mask.
pub fn tag_all(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g.selmon;
    let tagmask = TagMask::from_bits(ctx.g.tags.mask());
    let effective_mask = mask & tagmask;

    if effective_mask.is_empty() {
        return;
    }

    let current_tag = ctx
        .g
        .monitors
        .get(selmon_id)
        .map(|m| m.current_tag)
        .unwrap_or(0);

    if current_tag == 0 {
        return;
    }

    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);
    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    let clients_on_tag: Vec<_> = {
        let mut result = Vec::new();
        let mut cursor = ctx.g.monitors.get(selmon_id).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match ctx.g.clients.get(&win) {
                Some(c) => {
                    if TagMask::from_bits(c.tags).intersects(current_tag_mask) {
                        result.push(win);
                    }
                    cursor = c.next;
                }
                None => break,
            }
        }
        result
    };

    for win in clients_on_tag {
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            if TagMask::from_bits(client.tags) == scratchpad {
                client.issticky = false;
            }
            client.tags = effective_mask.bits();
        }
    }

    focus(ctx, None);
    arrange(ctx, Some(selmon_id));
}

/// Toggle tags on the selected client.
pub fn toggle_tag(ctx: &mut WmCtx, win: Window, mask: TagMask) {
    let selmon_id = ctx.g.selmon;

    let tagmask = TagMask::from_bits(ctx.g.tags.mask());
    let _scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    let (current_tags, is_scratchpad) = ctx
        .g
        .clients
        .get(&win)
        .map_or((TagMask::EMPTY, false), |c| {
            (TagMask::from_bits(c.tags), c.tags == SCRATCHPAD_MASK)
        });

    if is_scratchpad {
        set_client_tag(ctx, win, mask);
        return;
    }

    let new_tags = current_tags ^ (mask & tagmask);

    if new_tags.is_empty() {
        return;
    }

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.tags = new_tags.bits();
    }

    set_client_tag_prop(win);
    focus(ctx, None);
    arrange(ctx, Some(selmon_id));
}

/// Follow a tag (move client to tag and view it).
pub fn follow_tag(ctx: &mut WmCtx, win: Window, mask: TagMask) {
    let had_prefix = ctx.g.tags.prefix;

    set_client_tag(ctx, win, mask);

    if had_prefix {
        ctx.g.tags.prefix = true;
    }

    crate::tags::view::view(ctx, mask);
}
