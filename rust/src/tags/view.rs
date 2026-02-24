//! View (workspace) navigation.

use crate::floating::{restore_all_floating, save_all_floating};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::types::{Direction, SCRATCHPAD_MASK};
use crate::util::get_sel_win;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

pub fn view(tag_bits: u32) {
    let bits = crate::tags::compute_prefix(tag_bits);
    let tagmask = get_globals().tags.mask();

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.seltags ^= 1;
        }
    }

    if bits & tagmask == 0 {
        return;
    }

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.tagset[mon.seltags as usize] = bits & tagmask;
        }
    }

    if bits == !0u32 {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.prev_tag = mon.current_tag;
            mon.current_tag = 0;
        }
    } else {
        let new_tag = lowest_set_bit(bits) + 1;

        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            if new_tag == mon.current_tag {
                mon.seltags ^= 1;
                return;
            }
            mon.prev_tag = mon.current_tag;
            mon.current_tag = new_tag;
        }
    }

    let mut globals = get_globals_mut();
    apply_pertag_settings(globals);
    focus(None);
    arrange(Some(get_globals().selmon));
}

pub fn toggle_view(tag_bits: u32) {
    let tagmask = get_globals().tags.mask();

    let new_tagset = {
        let globals = get_globals();
        let current = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize])
            .unwrap_or(0);
        current ^ (tag_bits & tagmask)
    };

    if new_tagset == 0 {
        return;
    }

    {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.tagset[mon.seltags as usize] = new_tagset;
        }

        if new_tagset == !0u32 {
            if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
                mon.prev_tag = mon.current_tag;
                mon.current_tag = 0;
            }
        } else {
            let new_tag = lowest_set_bit(new_tagset) + 1;
            if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
                let current_tag = mon.current_tag;
                if current_tag == 0 || (new_tagset & (1 << (current_tag - 1))) == 0 {
                    mon.prev_tag = current_tag;
                    mon.current_tag = new_tag;
                }
            }
        }

        apply_pertag_settings(globals);
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

pub fn view_to_left() {
    scroll_view(Direction::Left);
}

pub fn view_to_right() {
    scroll_view(Direction::Right);
}

pub fn shift_view_direction(direction: Direction) {
    let (tagset, numtags) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.tagset[mon.seltags as usize], globals.tags.count())
    };

    let mut next_tagset = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        next_tagset = match direction {
            Direction::Right | Direction::Down => {
                let shift = step as usize;
                (tagset << shift) | (tagset >> (numtags - shift))
            }
            Direction::Left | Direction::Up => {
                let rshift = step as usize;
                (tagset >> rshift) | (tagset << (numtags - rshift))
            }
        };

        let globals = get_globals();
        let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
                Some(c) => {
                    if (next_tagset & c.tags) != 0 {
                        found = true;
                        break;
                    }
                    cursor = c.next;
                }
                None => break,
            }
        }

        if found {
            break;
        }
    }

    if !found {
        return;
    }

    if (next_tagset & SCRATCHPAD_MASK) != 0 {
        next_tagset ^= SCRATCHPAD_MASK;
    }

    view(next_tagset);
}

pub fn shift_view(direction: Direction) {
    shift_view_direction(direction);
}

pub fn last_view() {
    let (current_tag, prev_tag) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.current_tag, mon.prev_tag)
    };

    if current_tag == prev_tag {
        crate::focus::focus_last_client();
        return;
    }

    view(1 << (prev_tag.saturating_sub(1)));
}

pub fn win_view() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let Ok(cookie) = conn.get_input_focus() else {
        return;
    };
    let reply = match cookie.reply() {
        Ok(r) => r,
        Err(_) => return,
    };
    let focused_win = reply.focus;

    let client_win = find_client_for_window(focused_win);

    let Some(win) = client_win else { return };

    let tags = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.tags).unwrap_or(1)
    };

    if tags == SCRATCHPAD_MASK {
        let current_tag = {
            let globals = get_globals();
            globals
                .monitors
                .get(globals.selmon)
                .map(|m| m.current_tag)
                .unwrap_or(1)
        };
        view(1 << (current_tag.saturating_sub(1)));
    } else {
        view(tags);
    }

    focus(Some(win));
}

pub fn swap_tags(tag_bits: u32) {
    let bits = crate::tags::compute_prefix(tag_bits);
    let tagmask = get_globals().tags.mask();
    let newtag = bits & tagmask;

    let (current_tag, current_tagset) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.current_tag as u32, mon.tagset[mon.seltags as usize])
    };

    if newtag == current_tagset
        || current_tagset == 0
        || (current_tagset & (current_tagset - 1)) != 0
    {
        return;
    }

    let target_idx = lowest_set_bit(bits);

    let clients_to_swap: Vec<Window> = {
        let globals = get_globals();
        let mut result = Vec::new();
        let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
                Some(c) => {
                    if (c.tags & newtag) != 0 || (c.tags & current_tagset) != 0 {
                        result.push(win);
                    }
                    cursor = c.next;
                }
                None => break,
            }
        }
        result
    };

    for win in clients_to_swap {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags ^= current_tagset ^ newtag;
            if client.tags == 0 {
                client.tags = newtag;
            }
        }
    }

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.tagset[mon.seltags as usize] = newtag;
            if mon.prev_tag == target_idx + 1 {
                mon.prev_tag = current_tag as usize;
            }
            mon.current_tag = target_idx + 1;
        }
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

pub fn follow_view() {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let prev_tag = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        mon.prev_tag
    };

    if prev_tag == 0 {
        return;
    }

    let target_bits = 1u32 << (prev_tag - 1);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = target_bits;
        }
    }

    view(target_bits);
    focus(Some(win));
    arrange(Some(get_globals().selmon));
}

pub fn toggle_overview(tag_bits: u32) {
    let (has_clients, current_tag) = {
        let globals = get_globals();
        let has_clients = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.clients.is_some())
            .unwrap_or(false);
        let current_tag = globals.monitors.get(globals.selmon).map(|m| m.current_tag);
        (has_clients, current_tag)
    };

    if !has_clients {
        if current_tag == Some(0) {
            last_view();
        }
        return;
    }

    match current_tag {
        Some(0) => {
            let sel_mon_id = get_globals().selmon;
            restore_all_floating(Some(sel_mon_id));
            win_view();
        }
        Some(_) => {
            let sel_mon_id = get_globals().selmon;
            save_all_floating(Some(sel_mon_id));
            view(tag_bits);
        }
        None => {}
    }
}

pub fn toggle_fullscreen_overview(tag_bits: u32) {
    let current_tag = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).map(|m| m.current_tag)
    };

    match current_tag {
        Some(0) => win_view(),
        Some(_) => view(tag_bits),
        None => {}
    }
}

pub(super) fn apply_pertag_settings(globals: &mut crate::globals::Globals) {
    let sel_mon_id = globals.selmon;

    let (nmaster, mfact) = {
        let Some(mon) = globals.monitors.get(sel_mon_id) else {
            return;
        };
        let current_tag = mon.current_tag;
        if current_tag == 0 || current_tag > globals.tags.tags.len() {
            return;
        }
        let tag = &globals.tags.tags[current_tag - 1];
        (tag.nmaster, tag.mfact)
    };

    if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
        mon.nmaster = nmaster;
        mon.mfact = mfact;
    }
}

fn scroll_view(dir: Direction) {
    let (current_tag, tagset, tagmask) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (
            mon.current_tag as u32,
            mon.tagset[mon.seltags as usize],
            globals.tags.mask(),
        )
    };

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= crate::constants::animation::MAX_TAG_NUMBER {
        return;
    }

    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    let new_tagset = match dir {
        Direction::Left => {
            if tagset <= 1 {
                return;
            }
            tagset >> 1
        }
        Direction::Right => {
            if (tagset & (tagmask >> 1)) == 0 {
                return;
            }
            tagset << 1
        }
        Direction::Up => {
            if tagset <= 1 {
                return;
            }
            tagset >> 1
        }
        Direction::Down => {
            if (tagset & (tagmask >> 1)) == 0 {
                return;
            }
            tagset << 1
        }
    };

    let new_tag = lowest_set_bit(new_tagset) + 1;

    {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.seltags ^= 1;
            mon.tagset[mon.seltags as usize] = new_tagset;
            mon.prev_tag = mon.current_tag;
            mon.current_tag = new_tag;
        }
        apply_pertag_settings(globals);
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

#[inline]
fn lowest_set_bit(bits: u32) -> usize {
    bits.trailing_zeros() as usize
}

fn find_client_for_window(win: Window) -> Option<Window> {
    let globals = get_globals();

    if globals.clients.contains_key(&win) {
        return Some(win);
    }

    let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

    while let Some(c_win) = cursor {
        match globals.clients.get(&c_win) {
            Some(c) => {
                if c.win == win {
                    return Some(c_win);
                }
                cursor = c.next;
            }
            None => break,
        }
    }

    None
}
