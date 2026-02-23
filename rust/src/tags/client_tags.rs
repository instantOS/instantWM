//! Client-to-tag assignment.
//!
//! This module controls which tag(s) a client window belongs to.  It covers
//! three distinct use-cases:
//!
//! * **Assign** – replace a client's tags entirely ([`set_client_tag`]).
//! * **Bulk assign** – move every client on the current tag to a new tag
//!   ([`tag_all`]).
//! * **Toggle** – flip a single tag bit on a client ([`toggle_tag`]).
//! * **Follow** – send a client to a tag *and* switch the view to that tag
//!   ([`follow_tag`]).
//!
//! All four operations share the same low-level guard: the resulting tag
//! bitmask must be non-zero and must not exceed the monitor's tag mask.

use crate::client::set_client_tag_prop;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::monitor::arrange;
use crate::tags::view::view;
use crate::types::{Arg, SCRATCHPAD_MASK};
use crate::util::get_sel_win;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Assign `arg.ui` (after applying the prefix modifier) as the sole tag(s)
/// for the currently selected client.
///
/// The value is masked against the monitor's valid tag range, so stray bits
/// are silently dropped.  Use [`toggle_tag`] to *add* or *remove* a tag
/// without clearing the others.
pub fn set_client_tag(arg: &Arg) {
    let tagmask = crate::tags::compute_prefix(arg);
    set_client_tag_impl(tagmask);
}

/// Assign every client that currently belongs to the active tag to `arg.ui`.
///
/// This is a bulk version of [`set_client_tag`]: every window on the current
/// tag is atomically moved to the target tag.  The view does **not**
/// automatically follow – call [`view`] afterwards if that is desired.
pub fn tag_all(arg: &Arg) {
    let target_bits = crate::tags::compute_prefix(arg);

    let globals = get_globals();
    let tagmask = globals.tags.mask();

    // Nothing to do if the target tag is outside the valid range.
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

    // Collect every client on the current tag before borrowing mutably.
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

/// XOR `arg.ui` into the selected client's tag bitmask.
///
/// If the client is a scratchpad it is treated as a plain assignment instead
/// (scratchpads cannot span multiple tags).  The operation is a no-op if it
/// would result in a zero-tag client (i.e. the last remaining tag cannot be
/// toggled off).
pub fn toggle_tag(arg: &Arg) {
    let bits = crate::tags::compute_prefix(arg);

    let sel_win = get_sel_win();

    let Some(win) = sel_win else { return };

    // Scratchpads get a plain set instead of a toggle.
    let is_scratchpad = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.tags == SCRATCHPAD_MASK)
            .unwrap_or(false)
    };

    if is_scratchpad {
        set_client_tag(arg);
        return;
    }

    let (current_tags, tagmask) = {
        let globals = get_globals();
        let tags = globals.clients.get(&win).map(|c| c.tags).unwrap_or(0);
        (tags, globals.tags.mask())
    };

    let new_tags = current_tags ^ (bits & tagmask);

    // Guard: do not create a tag-less client.
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

/// Send the selected client to `arg.ui` **and** switch the view to that tag.
///
/// This combines [`set_client_tag`] + [`view`] in a single action so the user
/// follows the window they just moved.  If prefix mode was active it is
/// re-enabled after the view switch so chained prefix commands still work.
pub fn follow_tag(arg: &Arg) {
    // Remember prefix state before compute_prefix clears it.
    let had_prefix = get_globals().tags.prefix;

    let bits = crate::tags::compute_prefix(arg);

    // Bail early if nothing is selected.
    if get_globals()
        .monitors
        .get(get_globals().selmon)
        .and_then(|m| m.sel)
        .is_none()
    {
        return;
    }

    let tag_arg = Arg {
        ui: bits,
        ..Default::default()
    };
    set_client_tag(&tag_arg);

    // Restore prefix so the subsequent view() call honours it.
    if had_prefix {
        get_globals_mut().tags.prefix = true;
    }

    view(&tag_arg);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Core implementation shared by [`set_client_tag`].
///
/// Separated so that [`follow_tag`] can call [`set_client_tag`] after building
/// the correctly-prefixed bitmask without double-applying the prefix logic.
pub(super) fn set_client_tag_impl(tagmask_bits: u32) {
    let globals = get_globals();
    let tagmask = globals.tags.mask();
    let sel_win = get_sel_win();

    let Some(win) = sel_win else { return };

    // Reject out-of-range masks (e.g. 0 means "no tag" which is invalid).
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
                // Moving a scratchpad to a real tag un-stickies it.
                client.issticky = false;
            }
            client.tags = tagmask_bits & tagmask;
        }
    }

    set_client_tag_prop(win);
    focus(None);
    arrange(Some(get_globals().selmon));
}
