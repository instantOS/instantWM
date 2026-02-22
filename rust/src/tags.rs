use crate::bar::{draw_bars, text_width};
use crate::client::set_client_tag_prop;
use crate::floating::{restore_all_floating, save_all_floating};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::{arrange, dir_to_mon, send_mon};
use crate::toggles::{alt_tab_free, toggle_alt_tag, toggle_show_tags};
use crate::types::*;
use std::sync::atomic::Ordering;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

const MAX_TAGLEN: usize = 16;

const DIR_LEFT: i32 = 0;
const DIR_RIGHT: i32 = 1;

pub fn compute_prefix(arg: &Arg) -> u32 {
    let tagprefix = get_globals().tags.prefix;
    if tagprefix && arg.ui != 0 {
        let globals = get_globals_mut();
        globals.tags.prefix = false;
        arg.ui << 10
    } else {
        arg.ui
    }
}

pub fn name_tag(arg: &Arg) {
    let name_ptr = arg.v;
    let name = unsafe { std::ffi::CStr::from_ptr(name_ptr.unwrap() as *const i8) };
    let name_bytes = name.to_bytes();

    if name_bytes.len() >= MAX_TAGLEN {
        return;
    }

    let (numtags, current_tag) = {
        let globals = get_globals();
        let numtags = globals.tags.count();
        let current_tag = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize]);
        (numtags, current_tag)
    };

    let Some(tagset) = current_tag else {
        return;
    };

    for i in 0..numtags {
        if i >= MAX_TAGS {
            break;
        }
        if (tagset & (1 << i)) != 0 {
            let globals = get_globals_mut();
            if !name_bytes.is_empty() {
                if i < globals.tags.tags.len() {
                    globals.tags.tags[i].name = String::from_utf8_lossy(name_bytes).into_owned();
                }
            } else {
                let default_tag = if i == 8 {
                    "9".to_string()
                } else {
                    ((b'1' + i as u8) as char).to_string()
                };
                if i < globals.tags.tags.len() {
                    globals.tags.tags[i].name = default_tag;
                }
            }
        }
    }

    {
        let mut globals = get_globals_mut();
        globals.tags.width = get_tag_width();
    }
    draw_bars();
}

pub fn reset_name_tag(_arg: &Arg) {
    let globals = get_globals_mut();
    for i in 0..globals.tags.count() {
        if i >= MAX_TAGS {
            break;
        }
        if i < globals.tags.tags.len() {
            globals.tags.tags[i].name = format!("{}", i + 1);
        }
    }
    globals.tags.width = get_tag_width();
    draw_bars();
}

pub fn get_tag_width() -> i32 {
    let mut x = 0;
    let mut occupied_tags: u32 = 0;

    let globals = get_globals();

    let mut current = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

    while let Some(c_win) = current {
        if let Some(c) = globals.clients.get(&c_win) {
            if c.tags != 255 {
                occupied_tags |= c.tags;
            }
            current = c.next;
        } else {
            break;
        }
    }

    let start_menu_size = globals.startmenusize;
    let numtags = globals.tags.count();
    let lrpad = globals.lrpad;
    let showalttag = globals.tags.show_alt;

    for i in 0..numtags {
        if i >= 9 {
            continue;
        }

        let showtags = globals
            .monitors
            .get(globals.selmon)
            .map_or(0, |mon| mon.showtags);

        let current_tagset = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize]);

        if showtags != 0 {
            let occupied = (occupied_tags & (1 << i)) != 0;
            let selected = current_tagset.map_or(false, |t| (t & (1 << i)) != 0);
            if !occupied && !selected {
                continue;
            }
        }

        let tag_name = globals
            .tags
            .tags
            .get(i)
            .map(|t| t.name.as_str())
            .unwrap_or("");
        let display_name = if showalttag {
            globals
                .tags
                .tags
                .get(i)
                .map(|t| t.alt_name)
                .unwrap_or(tag_name)
        } else {
            tag_name
        };
        x += text_width(display_name) + lrpad;
    }

    x + start_menu_size
}

/// Get the tag index at a given X coordinate in the bar.
///
/// This function is used to determine which tag was clicked based on
/// the X position of the mouse cursor relative to the bar.
///
/// # Arguments
/// * `click_x` - The X coordinate (horizontal position) to check
///
/// # Returns
/// The tag index (0-based) if a tag is at that position, or -1 if not
pub fn get_tag_at_x(click_x: i32) -> i32 {
    let mut accumulated_width: i32;
    let mut occupied_tags: u32 = 0;

    let globals = get_globals();
    let start_menu_size = globals.startmenusize;
    accumulated_width = start_menu_size;

    let mut current = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

    while let Some(c_win) = current {
        if let Some(c) = globals.clients.get(&c_win) {
            if c.tags != 255 {
                occupied_tags |= c.tags;
            }
            current = c.next;
        } else {
            break;
        }
    }

    let numtags = globals.tags.count();
    let lrpad = globals.lrpad;
    let showalttag = globals.tags.show_alt;

    for i in 0..numtags {
        if i >= 9 {
            continue;
        }

        let showtags = globals
            .monitors
            .get(globals.selmon)
            .map_or(0, |mon| mon.showtags);

        let current_tagset = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize]);

        if showtags != 0 {
            let occupied = (occupied_tags & (1 << i)) != 0;
            let selected = current_tagset.map_or(false, |t| (t & (1 << i)) != 0);
            if !occupied && !selected {
                continue;
            }
        }

        let tag_name = globals
            .tags
            .tags
            .get(i)
            .map(|t| t.name.as_str())
            .unwrap_or("");
        let display_name = if showalttag {
            globals
                .tags
                .tags
                .get(i)
                .map(|t| t.alt_name)
                .unwrap_or(tag_name)
        } else {
            tag_name
        };
        accumulated_width += text_width(display_name) + lrpad;

        if accumulated_width >= click_x {
            return i as i32;
        }
    }

    -1
}

/// Set the tag(s) for the currently selected client.
///
/// This function assigns the specified tag(s) to the selected window,
/// replacing any existing tags. Use `toggle_tag` to add/remove tags.
///
/// # Arguments
/// * `arg` - Argument containing the tag bitmask in `ui`
pub fn set_client_tag(arg: &Arg) {
    let ui = compute_prefix(arg);
    set_client_tag_impl(ui);
}

/// Legacy alias for `set_client_tag`. Use `set_client_tag` for new code.
#[deprecated(since = "0.1.0", note = "Use set_client_tag instead")]
pub fn tag(arg: &Arg) {
    set_client_tag(arg);
}

fn set_client_tag_impl(tagmask_bits: u32) {
    let (sel_win, tagmask) = {
        let globals = get_globals();
        (
            globals.monitors.get(globals.selmon).and_then(|m| m.sel),
            globals.tags.mask(),
        )
    };

    let Some(win) = sel_win else { return };

    if tagmask_bits & tagmask == 0 {
        return;
    }

    let is_scratchpad = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.tags == SCRATCHPAD_MASK)
            .unwrap_or(false)
    };

    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if is_scratchpad {
            client.issticky = false;
        }
        client.tags = tagmask_bits & tagmask;
    }

    set_client_tag_prop(win);
    focus(None);

    arrange(Some(get_globals().selmon));
}

pub fn tag_all(arg: &Arg) {
    let ui = compute_prefix(arg);
    let globals = get_globals();

    let current_tag = globals
        .monitors
        .get(globals.selmon)
        .map(|mon| mon.current_tag)
        .unwrap_or(0);

    if current_tag == 0 {
        return;
    }

    let tagmask = globals.tags.mask();

    let clients_to_tag: Vec<Window> = {
        let mut result = Vec::new();
        let mut current = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if (c.tags & (1 << (current_tag - 1))) != 0 {
                    result.push(c_win);
                }
                current = c.next;
            } else {
                break;
            }
        }
        result
    };

    if ui & tagmask == 0 {
        return;
    }

    for win in clients_to_tag {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if client.tags == SCRATCHPAD_MASK {
                client.issticky = false;
            }
            client.tags = ui & tagmask;
        }
    }

    focus(None);

    arrange(Some(get_globals().selmon));
}

pub fn follow_tag(arg: &Arg) {
    let ui = compute_prefix(arg);
    let tagprefix = get_globals().tags.prefix;

    if get_globals()
        .monitors
        .get(get_globals().selmon)
        .and_then(|m| m.sel)
        .is_none()
    {
        return;
    }

    let a = Arg {
        ui,
        ..Default::default()
    };
    tag(&a);

    if tagprefix {
        let globals = get_globals_mut();
        globals.tags.prefix = true;
    }

    view(&a);
}

pub fn toggle_tag(arg: &Arg) {
    let ui = compute_prefix(arg);

    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

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
        tag(arg);
        return;
    }

    let (current_tags, tagmask) = {
        let globals = get_globals();
        let current = globals.clients.get(&win).map(|c| c.tags).unwrap_or(0);
        (current, globals.tags.mask())
    };

    let new_tags = current_tags ^ (ui & tagmask);
    if new_tags != 0 {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = new_tags;
        }
        set_client_tag_prop(win);
        focus(None);

        arrange(Some(get_globals().selmon));
    }
}

pub fn view(arg: &Arg) {
    let ui = compute_prefix(arg);
    let tagmask = get_globals().tags.mask();

    let mut globals = get_globals_mut();

    if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
        mon.seltags ^= 1;
    }

    if ui & tagmask == 0 {
        return;
    }

    if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
        mon.tagset[mon.seltags as usize] = ui & tagmask;
    }

    //TOD: this is a magic number
    if ui == !0u32 {
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.prev_tag = mon.current_tag;
            mon.current_tag = 0;
        }
    } else {
        let mut i = 0;
        while (ui & (1 << i)) == 0 {
            i += 1;
        }

        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            if i + 1 == mon.current_tag {
                return;
            }
            mon.prev_tag = mon.current_tag;
            mon.current_tag = i + 1;
        }
    }

    apply_pertag_settings(&mut globals);
    focus(None);

    arrange(Some(get_globals().selmon));
}

fn apply_pertag_settings(globals: &mut crate::globals::Globals) {
    let sel_mon_id = globals.selmon;
    let (nmaster, mfact) = {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            let current_tag = mon.current_tag;
            if current_tag > 0 && current_tag <= globals.tags.tags.len() {
                let tag = &globals.tags.tags[current_tag - 1];
                (tag.nmaster, tag.mfact)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
        mon.nmaster = nmaster;
        mon.mfact = mfact;
    }
}

pub fn toggle_view(arg: &Arg) {
    let tagmask = get_globals().tags.mask();
    let new_tagset = {
        let globals = get_globals();
        let current = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize]);
        current.unwrap_or(0) ^ (arg.ui & tagmask)
    };

    if new_tagset == 0 {
        return;
    }

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
        let mut i = 0;
        while (new_tagset & (1 << i)) == 0 {
            i += 1;
        }

        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            let current_tag = mon.current_tag;
            if (new_tagset & (1 << (current_tag - 1))) == 0 {
                mon.prev_tag = current_tag;
                mon.current_tag = i + 1;
            }
        }
    }

    apply_pertag_settings(&mut globals);
    focus(None);

    arrange(Some(get_globals().selmon));
}

pub fn tag_mon(arg: &Arg) {
    let (sel_win, has_multiple_mons) = {
        let globals = get_globals();
        (
            globals.monitors.get(globals.selmon).and_then(|m| m.sel),
            globals.monitors.len() > 1,
        )
    };

    let Some(win) = sel_win else { return };
    if !has_multiple_mons {
        return;
    }

    let target_mon = dir_to_mon(arg.i);
    let Some(target_id) = target_mon else { return };

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if is_floating {
        let (c_x, c_y, mon_mx, mon_my, mon_ww, mon_wh) = {
            let globals = get_globals();
            if let Some(c) = globals.clients.get(&win) {
                let mon = globals.monitors.get(globals.selmon);
                let (mx, my, ww, wh) = mon
                    .map(|m| {
                        (
                            m.monitor_rect.x,
                            m.monitor_rect.y,
                            m.work_rect.w,
                            m.work_rect.h,
                        )
                    })
                    .unwrap_or((0, 0, 0, 0));
                (c.geo.x, c.geo.y, mx, my, ww, wh)
            } else {
                return;
            }
        };

        let xfact = (c_x - mon_mx) as f32 / mon_ww as f32;
        let yfact = (c_y - mon_my) as f32 / mon_wh as f32;

        let (target_mx, target_my, target_ww, target_wh) = {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(target_id) {
                (
                    mon.monitor_rect.x,
                    mon.monitor_rect.y,
                    mon.work_rect.w,
                    mon.work_rect.h,
                )
            } else {
                (0, 0, 0, 0)
            }
        };

        send_mon(win, target_id);

        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.geo.x = target_mx + (target_ww as f32 * xfact) as i32;
            client.geo.y = target_my + (target_wh as f32 * yfact) as i32;
        }

        arrange(Some(globals.selmon));

        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = configure_window(
                conn,
                win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }
    } else {
        send_mon(win, target_id);
    }
}

/// Move the selected client's tag to the left (to a lower-numbered tag).
///
/// # Arguments
/// * `offset` - Number of tag positions to shift (default: 1)
pub fn tag_to_left_by(offset: i32) {
    shift_tag(Direction::Left, offset.max(1));
}

/// Move the selected client's tag to the right (to a higher-numbered tag).
///
/// # Arguments
/// * `offset` - Number of tag positions to shift (default: 1)
pub fn tag_to_right_by(offset: i32) {
    shift_tag(Direction::Right, offset.max(1));
}

/// Legacy wrapper for key bindings. Use `tag_to_left_by` for new code.
pub fn tag_to_left(arg: &Arg) {
    tag_to_left_by(arg.i);
}

/// Legacy wrapper for key bindings. Use `tag_to_right_by` for new code.
/// TODO: remove all legacy wrappers
pub fn tag_to_right(arg: &Arg) {
    tag_to_right_by(arg.i);
}

fn shift_tag(dir: Direction, offset: i32) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let (current_tag, overlay) = {
        let globals = get_globals();
        let current_tag = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.current_tag as u32);
        let overlay = globals.monitors.get(globals.selmon).and_then(|m| m.overlay);
        (current_tag, overlay)
    };

    let Some(current_tag) = current_tag else {
        return;
    };

    if Some(win) == overlay {
        let mode = match dir {
            Direction::Left => 3,
            Direction::Right => 1,
            Direction::Up => 0,
            Direction::Down => 2,
        };
        crate::overlay::set_overlay_mode(mode);
        return;
    }

    if dir == Direction::Left && current_tag == 1 {
        return;
    }
    if dir == Direction::Right && current_tag == 20 {
        return;
    }

    reset_sticky_client(win);

    let animated = get_globals().animated;
    if animated {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = configure_window(
                conn,
                win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }

        let (mon_mw, c_x, c_y) = {
            let globals = get_globals();
            let mon_mw = globals
                .monitors
                .get(globals.selmon)
                .map(|m| m.monitor_rect.w)
                .unwrap_or(0);
            let (c_x, c_y) = globals
                .clients
                .get(&win)
                .map(|c| (c.geo.x, c.geo.y))
                .unwrap_or((0, 0));
            (mon_mw, c_x, c_y)
        };

        let anim_offset = (mon_mw / 10)
            * match dir {
                Direction::Left => -1,
                Direction::Right => 1,
                Direction::Up => -1,
                Direction::Down => 1,
            };
        crate::animation::animate_client_rect(
            win,
            &Rect {
                x: c_x + anim_offset,
                y: c_y,
                w: 0,
                h: 0,
            },
            7,
            0,
        );
    }

    let (tagset, tagmask) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.tagset[mon.seltags as usize], globals.tags.mask())
    };

    let is_single_tag = (tagset & tagmask).count_ones() == 1;

    if is_single_tag {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            match dir {
                Direction::Left if tagset > 1 => {
                    client.tags >>= offset;
                    focus(None);
                    arrange(Some(get_globals().selmon));
                }
                Direction::Right if (tagset & (tagmask >> 1)) != 0 => {
                    client.tags <<= offset;
                    focus(None);
                    arrange(Some(get_globals().selmon));
                }
                _ => {}
            }
        }
    }
}

fn reset_sticky_client(win: Window) {
    let target_tags = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|mon| {
            if mon.current_tag > 0 {
                Some(1 << (mon.current_tag - 1))
            } else {
                None
            }
        })
    };

    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.issticky {
            client.issticky = false;
            if let Some(tags) = target_tags {
                client.tags = tags;
            }
        }
    }
}

pub fn view_to_left(_arg: &Arg) {
    view_scroll(Direction::Left);
}

pub fn view_to_right(_arg: &Arg) {
    view_scroll(Direction::Right);
}

fn view_scroll(dir: Direction) {
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

    if dir == Direction::Left && current_tag == 1 {
        return;
    }
    if dir == Direction::Right && current_tag == 20 {
        return;
    }

    let is_single_tag = (tagset & tagmask).count_ones() == 1;
    if !is_single_tag {
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

    let mut globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
        mon.seltags ^= 1;
        mon.tagset[mon.seltags as usize] = new_tagset;

        mon.prev_tag = mon.current_tag;
        let mut i = 0;
        while (new_tagset & (1 << i)) == 0 {
            i += 1;
        }
        mon.current_tag = i + 1;
    }

    apply_pertag_settings(&mut globals);
    focus(None);

    arrange(Some(get_globals().selmon));
}

pub fn move_left(arg: &Arg) {
    tag_to_left(arg);
    view_to_left(arg);
}

pub fn move_right(arg: &Arg) {
    tag_to_right(arg);
    view_to_right(arg);
}

/// Shift view to the next/previous tag that has visible clients.
///
/// # Arguments
/// * `forward` - If true, shift to higher-numbered tags; if false, shift to lower.
pub fn shift_view_direction(forward: bool) {
    let direction: i32 = if forward { 1 } else { -1 };

    let (tagset, numtags) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.tagset[mon.seltags as usize], globals.tags.count())
    };

    let mut next_seltags = tagset;
    let mut visible = false;
    let mut count = 0;
    let mut shift = direction;

    while !visible && count < 10 {
        if direction > 0 {
            next_seltags = (tagset << shift) | (tagset >> (numtags - 1 - shift as usize));
        } else {
            next_seltags =
                (tagset >> (-shift) as usize) | (tagset << (numtags - 1 + shift as usize));
        }

        let globals = get_globals();
        let mut current = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if (next_seltags & c.tags) != 0 {
                    visible = true;
                    break;
                }
                current = c.next;
            } else {
                break;
            }
        }

        shift += direction;
        count += 1;
    }

    if count < 10 {
        if (next_seltags & SCRATCHPAD_MASK) != 0 {
            next_seltags ^= SCRATCHPAD_MASK;
        }
        let a = Arg {
            ui: next_seltags,
            ..Default::default()
        };
        view(&a);
    }
}

/// Legacy wrapper for key bindings. Use `shift_view_direction` for new code.
pub fn shift_view(arg: &Arg) {
    shift_view_direction(arg.i > 0);
}

pub fn swap_tags(arg: &Arg) {
    let ui = compute_prefix(arg);
    let newtag = ui & get_globals().tags.mask();

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

    let mut target_idx = 0;
    while (ui & (1 << target_idx)) == 0 {
        target_idx += 1;
    }

    let clients_to_swap: Vec<Window> = {
        let globals = get_globals();
        let mut result = Vec::new();
        let mut current = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if (c.tags & newtag) != 0 || (c.tags & current_tagset) != 0 {
                    result.push(c_win);
                }
                current = c.next;
            } else {
                break;
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

    let globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
        mon.tagset[mon.seltags as usize] = newtag;

        if mon.prev_tag == target_idx + 1 {
            mon.prev_tag = current_tag as usize;
        }
        mon.current_tag = target_idx + 1;
    }
    focus(None);

    let sel_mon_id = get_globals().selmon;
    arrange(Some(sel_mon_id));
}

pub fn follow_view(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let prevtag = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        mon.prev_tag
    };

    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.tags = 1 << (prevtag - 1);
    }

    let a = Arg {
        ui: 1 << (prevtag - 1),
        ..Default::default()
    };
    view(&a);
    focus(Some(win));

    let sel_mon_id = get_globals().selmon;
    arrange(Some(sel_mon_id));
}

pub fn reset_sticky(c: &mut Client) {
    if !c.issticky {
        return;
    }
    c.issticky = false;

    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(globals.selmon) {
        if mon.current_tag > 0 {
            c.tags = 1 << (mon.current_tag - 1);
        }
    }
}

pub fn toggle_overview(_arg: &Arg) {
    let (has_clients, current_tag, prevtag) = {
        let globals = get_globals();
        let has_clients = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.clients.is_some())
            .unwrap_or(false);
        let current_tag = globals.monitors.get(globals.selmon).map(|m| m.current_tag);
        let prevtag = globals.monitors.get(globals.selmon).map(|m| m.prev_tag);
        (has_clients, current_tag, prevtag)
    };

    if !has_clients {
        if current_tag == Some(0) {
            last_view(&Arg::default());
        }
        return;
    }

    if let Some(current) = current_tag {
        if current == 0 {
            let pt = prevtag.unwrap_or(1);
            let sel_mon_id = get_globals().selmon;
            restore_all_floating(Some(sel_mon_id));
            win_view(&Arg::default());
        } else {
            let sel_mon_id = get_globals().selmon;
            save_all_floating(Some(sel_mon_id));
            view(&Arg {
                ui: !0,
                ..Default::default()
            });
        }
    }
}

//TODO: according to cargo check this is unused. what is up with that?
//also compare to C codebase
pub fn toggle_fullscreen_overview(_arg: &Arg) {
    let current_tag = {
        let globals = get_globals();
        let sel_mon_id = globals.selmon;
        globals.monitors.get(sel_mon_id).map(|m| m.current_tag)
    };

    if let Some(current) = current_tag {
        if current == 0 {
            win_view(&Arg::default());
        } else {
            view(&Arg {
                ui: !0,
                ..Default::default()
            });
        }
    }
}

pub fn last_view(_arg: &Arg) {
    let (current_tag, prevtag) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.current_tag, mon.prev_tag)
    };

    if current_tag == prevtag {
        focus_last_client(&Arg::default());
    } else {
        view(&Arg {
            ui: 1 << (prevtag - 1),
            ..Default::default()
        });
    }
}

pub fn focus_last_client(_arg: &Arg) {
    let last_client = crate::client::LAST_CLIENT.load(Ordering::Relaxed);
    if last_client != 0 {
        focus(Some(last_client));
    }
}

pub fn win_view(_arg: &Arg) {
    //TODO: this is very nested, search for architectural issues or how to do this cleaner
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let focus_win = conn.get_input_focus();
        if let Ok(cookie) = focus_win {
            if let Ok(reply) = cookie.reply() {
                let win = reply.focus;

                let client_win = {
                    let globals = get_globals();
                    if globals.clients.contains_key(&win) {
                        Some(win)
                    } else {
                        let mut current =
                            globals.monitors.get(globals.selmon).and_then(|m| m.clients);

                        let mut found = None;
                        while let Some(c_win) = current {
                            if let Some(c) = globals.clients.get(&c_win) {
                                if c.win == win {
                                    found = Some(c_win);
                                    break;
                                }
                                current = c.next;
                            } else {
                                break;
                            }
                        }
                        found
                    }
                };

                if let Some(c_win) = client_win {
                    let tags = {
                        let globals = get_globals();
                        globals.clients.get(&c_win).map(|c| c.tags).unwrap_or(1)
                    };

                    if tags == SCRATCHPAD_MASK {
                        let current_tag = {
                            let globals = get_globals();
                            let sel_mon_id = globals.selmon;
                            globals.monitors.get(sel_mon_id).map(|m| m.current_tag)
                        };
                        view(&Arg {
                            ui: 1 << (current_tag.unwrap_or(1) - 1),
                            ..Default::default()
                        });
                    } else {
                        view(&Arg {
                            ui: tags,
                            ..Default::default()
                        });
                    }
                    focus(Some(c_win));
                }
            }
        }
    }
}

pub fn desktop_set(_arg: &Arg) {}

//TODO: this is unused according to cargo check. what is up with that?
pub fn zoom(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    crate::client::pop(win);
}

pub fn quit(_arg: &Arg) {
    std::process::exit(0);
}

pub fn spawn(arg: &Arg) {
    if let Some(ptr) = arg.v {
        let cmd = unsafe {
            let ptr = ptr as *const u8;
            let len = (0..1024).find(|&i| *ptr.add(i) == 0).unwrap_or(1024);
            let slice = std::slice::from_raw_parts(ptr, len);
            String::from_utf8_lossy(slice).to_string()
        };

        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        let program = parts[0];
        let args: Vec<&str> = parts[1..].to_vec();

        let _ = std::process::Command::new(program).args(&args).spawn();
    }
}

pub fn command_tag(arg: &Arg) {
    tag(arg);
}

pub fn command_view(arg: &Arg) {
    view(arg);
}

pub fn command_toggle_view(arg: &Arg) {
    toggle_view(arg);
}

pub fn command_toggle_tag(arg: &Arg) {
    toggle_tag(arg);
}
