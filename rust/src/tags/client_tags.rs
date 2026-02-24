//! Client-to-tag assignment.

use crate::client::set_client_tag_prop;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::monitor::arrange;
use crate::tags::view::view;
use crate::types::SCRATCHPAD_MASK;
use crate::util::get_sel_win;

pub fn set_client_tag(tagmask: u32) {
    let tagmask = crate::tags::compute_prefix(tagmask);
    set_client_tag_impl(tagmask);
}

pub fn tag_all(tag_bits: u32) {
    let target_bits = crate::tags::compute_prefix(tag_bits);

    let globals = get_globals();
    let tagmask = globals.tags.mask();

    if target_bits & tagmask == 0 {
        return;
    }

    let current_tag = globals
        .monitors
        .get(globals.selmon)
        .map(|m| m.current_tag)
        .unwrap_or(0);

    if current_tag == 0 {
        return;
    }

    let clients_on_tag: Vec<_> = {
        let mut result = Vec::new();
        let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
                Some(c) => {
                    if (c.tags & (1 << (current_tag - 1))) != 0 {
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
            if client.tags == SCRATCHPAD_MASK {
                client.issticky = false;
            }
            client.tags = target_bits & tagmask;
        }
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

pub fn toggle_tag(tag_bits: u32) {
    let bits = crate::tags::compute_prefix(tag_bits);

    let sel_win = get_sel_win();

    let Some(win) = sel_win else { return };

    let is_scratchpad = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.tags == SCRATCHPAD_MASK)
            .unwrap_or(false)
    };

    if is_scratchpad {
        set_client_tag(bits);
        return;
    }

    let (current_tags, tagmask) = {
        let globals = get_globals();
        let tags = globals.clients.get(&win).map(|c| c.tags).unwrap_or(0);
        (tags, globals.tags.mask())
    };

    let new_tags = current_tags ^ (bits & tagmask);

    if new_tags == 0 {
        return;
    }

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = new_tags;
        }
    }

    set_client_tag_prop(win);
    focus(None);
    arrange(Some(get_globals().selmon));
}

pub fn follow_tag(tag_bits: u32) {
    let had_prefix = get_globals().tags.prefix;

    let bits = crate::tags::compute_prefix(tag_bits);

    if get_globals()
        .monitors
        .get(get_globals().selmon)
        .and_then(|m| m.sel)
        .is_none()
    {
        return;
    }

    set_client_tag(bits);

    if had_prefix {
        get_globals_mut().tags.prefix = true;
    }

    view(bits);
}

pub(super) fn set_client_tag_impl(tagmask_bits: u32) {
    let globals = get_globals();
    let tagmask = globals.tags.mask();
    let sel_win = get_sel_win();

    let Some(win) = sel_win else { return };

    if tagmask_bits & tagmask == 0 {
        return;
    }

    let is_scratchpad = globals
        .clients
        .get(&win)
        .map(|c| c.tags == SCRATCHPAD_MASK)
        .unwrap_or(false);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if is_scratchpad {
                client.issticky = false;
            }
            client.tags = tagmask_bits & tagmask;
        }
    }

    set_client_tag_prop(win);
    focus(None);
    arrange(Some(get_globals().selmon));
}
