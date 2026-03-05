//! Client-to-tag assignment.

use crate::contexts::{CoreCtx, X11Ctx};
use crate::layouts::arrange;
use crate::types::{TagMask, WindowId, SCRATCHPAD_MASK};

/// Set the selected client's tags using type-safe mask.
pub fn set_client_tag(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, mask: TagMask) {
    let selmon_id = core.g.selected_monitor_id();

    let tagmask = TagMask::from_bits(core.g.tags.mask());
    let effective_mask = mask & tagmask;

    if effective_mask.is_empty() {
        return;
    }

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    if let Some(client) = core.client_mut(win) {
        if TagMask::from_bits(client.tags) == scratchpad {
            client.issticky = false;
        }
        client.tags = effective_mask.bits();

        crate::client::set_client_tag_prop(core, x11, win);
        crate::focus::focus_soft_x11(core, x11, None);
        arrange(core, Some(selmon_id));
    }
}

/// Tag all clients on current tag with the given mask.
pub fn tag_all(core: &mut CoreCtx, x11: &X11Ctx, mask: TagMask) {
    let selmon_id = core.g.selected_monitor_id();
    let tagmask = TagMask::from_bits(core.g.tags.mask());
    let effective_mask = mask & tagmask;

    if effective_mask.is_empty() {
        return;
    }

    let current_tag = core.g.selected_monitor().current_tag;

    //TODO: what does 0 mean here? Magic number?
    if current_tag == 0 {
        return;
    }

    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);
    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    let m = core.g.selected_monitor();
    let clients_on_tag: Vec<_> = m
        .iter_clients(&*core.g.clients)
        .filter(|(_, c)| TagMask::from_bits(c.tags).intersects(current_tag_mask))
        .map(|(win, _)| win)
        .collect();

    for win in clients_on_tag {
        if let Some(client) = core.g.clients.get_mut(&win) {
            if TagMask::from_bits(client.tags) == scratchpad {
                client.issticky = false;
            }
            client.tags = effective_mask.bits();
        }
    }

    crate::focus::focus_soft_x11(core, x11, None);
    arrange(core, Some(selmon_id));
}

/// Toggle tags on the selected client.
pub fn toggle_tag(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, mask: TagMask) {
    let tagmask = TagMask::from_bits(core.g.tags.mask());

    let current_tags = core
        .client(win)
        .map_or(TagMask::EMPTY, |c| TagMask::from_bits(c.tags));

    if current_tags.bits() == SCRATCHPAD_MASK {
        set_client_tag(core, x11, win, mask);
        return;
    }

    let new_tags = current_tags ^ (mask & tagmask);

    if new_tags.is_empty() {
        return;
    }

    set_client_tag(core, x11, win, new_tags);
}

/// Follow a tag (move client to tag and view it).
pub fn follow_tag(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, mask: TagMask) {
    let had_prefix = core.g.tags.prefix;

    set_client_tag(core, x11, win, mask);

    if had_prefix {
        core.g.tags.prefix = true;
    }

    crate::tags::view::view(core, x11, mask);
}
