//! Client-to-tag assignment.

use crate::client::set_client_tag_prop;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::layouts::arrange;
use crate::tags::view::view;
use crate::types::{TagMask, SCRATCHPAD_MASK};
use crate::util::get_sel_win;

/// Set the selected client's tags using type-safe mask.
pub fn set_client_tag(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let tagmask = TagMask::from_bits(get_globals().tags.mask());
    let effective_mask = mask & tagmask;

    if effective_mask.is_empty() {
        return;
    }

    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if TagMask::from_bits(client.tags) == scratchpad {
                client.issticky = false;
            }
            client.tags = effective_mask.bits();
        }
    }

    set_client_tag_prop(win);
    focus(None);
    arrange(Some(selmon_id));
}

/// Tag all clients on current tag with the given mask.
pub fn tag_all(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let tagmask = TagMask::from_bits(get_globals().tags.mask());
    let effective_mask = mask & tagmask;

    if effective_mask.is_empty() {
        return;
    }

    let current_tag = {
        let globals = get_globals();
        globals
            .monitors
            .get(selmon_id)
            .map(|m| m.current_tag)
            .unwrap_or(0)
    };

    if current_tag == 0 {
        return;
    }

    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);
    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    let clients_on_tag: Vec<_> = {
        let globals = get_globals();
        let mut result = Vec::new();
        let mut cursor = globals.monitors.get(selmon_id).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
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
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if TagMask::from_bits(client.tags) == scratchpad {
                client.issticky = false;
            }
            client.tags = effective_mask.bits();
        }
    }

    focus(None);
    arrange(Some(selmon_id));
}

/// Toggle tags on the selected client.
pub fn toggle_tag(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let tagmask = TagMask::from_bits(get_globals().tags.mask());
    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);

    let (current_tags, is_scratchpad) = {
        let globals = get_globals();
        let client = globals.clients.get(&win);
        (
            client.map(|c| TagMask::from_bits(c.tags)).unwrap_or(TagMask::EMPTY),
            client.map(|c| c.tags == SCRATCHPAD_MASK).unwrap_or(false),
        )
    };

    if is_scratchpad {
        set_client_tag(mask);
        return;
    }

    let new_tags = current_tags ^ (mask & tagmask);

    if new_tags.is_empty() {
        return;
    }

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = new_tags.bits();
        }
    }

    set_client_tag_prop(win);
    focus(None);
    arrange(Some(selmon_id));
}

/// Follow a tag (move client to tag and view it).
pub fn follow_tag(mask: TagMask) {
    let had_prefix = get_globals().tags.prefix;
    let selmon_id = get_globals().selmon;

    let has_selection = {
        let globals = get_globals();
        globals
            .monitors
            .get(selmon_id)
            .and_then(|m| m.sel)
            .is_some()
    };

    if !has_selection {
        return;
    }

    set_client_tag(mask);

    if had_prefix {
        get_globals_mut().tags.prefix = true;
    }

    view(mask);
}
