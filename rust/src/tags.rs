use crate::client::{set_client_tag_prop, unfocus_win};
use crate::floating::{restore_all_floating, save_all_floating};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::{arrange, dir_to_mon, send_mon};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

const MAX_TAGLEN: usize = 16;
const DIR_LEFT: i32 = 0;
const DIR_RIGHT: i32 = 1;

pub fn compute_prefix(arg: &Arg) -> u32 {
    let tagprefix = get_globals().tagprefix;
    if tagprefix && arg.ui != 0 {
        let mut globals = get_globals_mut();
        globals.tagprefix = false;
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
        let numtags = globals.numtags as usize;
        let current_tag = globals
            .selmon
            .and_then(|id| globals.monitors.get(id))
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
            let mut globals = get_globals_mut();
            if !name_bytes.is_empty() {
                globals.tags[i][..name_bytes.len()].copy_from_slice(name_bytes);
                globals.tags[i][name_bytes.len()..]
                    .iter_mut()
                    .for_each(|b| *b = 0);
            } else {
                let default_tag = match i {
                    8 => b"9".to_vec(),
                    _ => vec![b'1' + i as u8],
                };
                globals.tags[i][..default_tag.len()].copy_from_slice(&default_tag);
                globals.tags[i][default_tag.len()..]
                    .iter_mut()
                    .for_each(|b| *b = 0);
            }
        }
    }
}

pub fn reset_name_tag(_arg: &Arg) {
    let mut globals = get_globals_mut();
    for i in 0..globals.numtags as usize {
        if i >= MAX_TAGS {
            break;
        }
        let default = format!("{}\0", i + 1);
        let bytes = default.as_bytes();
        globals.tags[i][..bytes.len().min(16)].copy_from_slice(&bytes[..bytes.len().min(16)]);
    }
    globals.tagwidth = 0;
}

pub fn get_tag_width() -> i32 {
    let mut x = 0;
    let mut occupied_tags: u32 = 0;

    let globals = get_globals();

    let mut current = if let Some(sel_mon_id) = globals.selmon {
        globals.monitors.get(sel_mon_id).and_then(|m| m.clients)
    } else {
        None
    };

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

    let start_menu_size = globals.startmenusize as i32;
    let numtags = globals.numtags;
    let lrpad = globals.lrpad;

    for i in 0..numtags as usize {
        if i >= 9 {
            continue;
        }

        let showtags = if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.showtags
            } else {
                0
            }
        } else {
            0
        };

        let current_tagset = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .map(|m| m.tagset[m.seltags as usize])
        } else {
            None
        };

        if showtags != 0 {
            let occupied = (occupied_tags & (1 << i)) != 0;
            let selected = current_tagset.map_or(false, |t| (t & (1 << i)) != 0);
            if !occupied && !selected {
                continue;
            }
        }

        let tag_len = globals.tags[i].iter().position(|&b| b == 0).unwrap_or(0);
        x += lrpad * (tag_len as i32 + 2);
    }

    x + start_menu_size
}

pub fn get_tag_at_x(ix: i32) -> i32 {
    let mut x;
    let mut occupied_tags: u32 = 0;

    let globals = get_globals();
    let start_menu_size = globals.startmenusize as i32;
    x = start_menu_size;

    let mut current = if let Some(sel_mon_id) = globals.selmon {
        globals.monitors.get(sel_mon_id).and_then(|m| m.clients)
    } else {
        None
    };

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

    let numtags = globals.numtags;
    let lrpad = globals.lrpad;

    for i in 0..numtags as usize {
        if i >= 9 {
            continue;
        }

        let showtags = if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.showtags
            } else {
                0
            }
        } else {
            0
        };

        let current_tagset = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .map(|m| m.tagset[m.seltags as usize])
        } else {
            None
        };

        if showtags != 0 {
            let occupied = (occupied_tags & (1 << i)) != 0;
            let selected = current_tagset.map_or(false, |t| (t & (1 << i)) != 0);
            if !occupied && !selected {
                continue;
            }
        }

        let tag_len = globals.tags[i].iter().position(|&b| b == 0).unwrap_or(0);
        x += lrpad * (tag_len as i32 + 2);

        if x >= ix {
            return i as i32;
        }
    }

    -1
}

pub fn tag(arg: &Arg) {
    let ui = compute_prefix(arg);
    set_client_tag_impl(ui);
}

fn set_client_tag_impl(tagmask_bits: u32) {
    let (sel_win, tagmask) = {
        let globals = get_globals();
        (
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel)),
            globals.tagmask,
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

    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if is_scratchpad {
            client.issticky = false;
        }
        client.tags = tagmask_bits & tagmask;
    }
    drop(globals);

    set_client_tag_prop(win);
    focus(None);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn tag_all(arg: &Arg) {
    let ui = compute_prefix(arg);
    let globals = get_globals();

    let current_tag = if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            if let Some(ref pertag) = mon.pertag {
                pertag.current_tag
            } else {
                return;
            }
        } else {
            return;
        }
    } else {
        return;
    };

    if current_tag == 0 {
        return;
    }

    let tagmask = globals.tagmask;

    let clients_to_tag: Vec<Window> = {
        let mut result = Vec::new();
        let mut current = if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.clients)
        } else {
            None
        };

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

    drop(globals);

    if ui & tagmask == 0 {
        return;
    }

    for win in clients_to_tag {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if client.tags == SCRATCHPAD_MASK {
                client.issticky = false;
            }
            client.tags = ui & tagmask;
        }
    }

    focus(None);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn follow_tag(arg: &Arg) {
    let ui = compute_prefix(arg);
    let tagprefix = get_globals().tagprefix;

    if get_globals()
        .selmon
        .and_then(|id| get_globals().monitors.get(id).and_then(|m| m.sel))
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
        let mut globals = get_globals_mut();
        globals.tagprefix = true;
    }

    view(&a);
}

pub fn toggle_tag(arg: &Arg) {
    let ui = compute_prefix(arg);

    let sel_win = {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
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
        (current, globals.tagmask)
    };

    let new_tags = current_tags ^ (ui & tagmask);
    if new_tags != 0 {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = new_tags;
        }
        drop(globals);
        set_client_tag_prop(win);
        focus(None);

        if let Some(sel_mon_id) = get_globals().selmon {
            arrange(Some(sel_mon_id));
        }
    }
}

pub fn view(arg: &Arg) {
    let ui = compute_prefix(arg);
    let tagmask = get_globals().tagmask;

    let mut globals = get_globals_mut();

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.seltags ^= 1;
        }
    }

    if ui & tagmask == 0 {
        return;
    }

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.tagset[mon.seltags as usize] = ui & tagmask;
        }
    }

    if ui == !0u32 {
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
                if let Some(ref mut pertag) = mon.pertag {
                    pertag.prevtag = pertag.current_tag;
                    pertag.current_tag = 0;
                }
            }
        }
    } else {
        let mut i = 0;
        while (ui & (1 << i)) == 0 {
            i += 1;
        }

        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
                if let Some(ref mut pertag) = mon.pertag {
                    if i + 1 == pertag.current_tag {
                        return;
                    }
                    pertag.prevtag = pertag.current_tag;
                    pertag.current_tag = i + 1;
                }
            }
        }
    }

    apply_pertag_settings(&mut globals);

    drop(globals);
    focus(None);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

fn apply_pertag_settings(globals: &mut crate::globals::Globals) {
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            if let Some(ref pertag) = mon.pertag {
                let current_tag = pertag.current_tag as usize;
                if current_tag < MAX_TAGS {
                    mon.nmaster = pertag.nmasters[current_tag];
                    mon.mfact = pertag.mfacts[current_tag];
                    mon.sellt = pertag.sellts[current_tag];

                    if let Some(lt_idx) = pertag.ltidxs[current_tag][mon.sellt as usize] {
                        mon.ltsymbol = globals
                            .layouts
                            .get(lt_idx)
                            .map(|l| {
                                let mut arr = [0u8; 16];
                                let bytes = l.symbol.as_bytes();
                                arr[..bytes.len().min(16)]
                                    .copy_from_slice(&bytes[..bytes.len().min(16)]);
                                arr
                            })
                            .unwrap_or([0u8; 16]);
                    }
                }
            }
        }
    }
}

pub fn toggle_view(arg: &Arg) {
    let tagmask = get_globals().tagmask;
    let new_tagset = {
        let globals = get_globals();
        let current = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .map(|m| m.tagset[m.seltags as usize])
        } else {
            None
        };
        current.unwrap_or(0) ^ (arg.ui & tagmask)
    };

    if new_tagset == 0 {
        return;
    }

    let mut globals = get_globals_mut();
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.tagset[mon.seltags as usize] = new_tagset;
        }
    }

    if new_tagset == !0u32 {
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
                if let Some(ref mut pertag) = mon.pertag {
                    pertag.prevtag = pertag.current_tag;
                    pertag.current_tag = 0;
                }
            }
        }
    } else {
        let mut i = 0;
        while (new_tagset & (1 << i)) == 0 {
            i += 1;
        }

        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
                if let Some(ref mut pertag) = mon.pertag {
                    let current_tag = pertag.current_tag;
                    if (new_tagset & (1 << (current_tag - 1))) == 0 {
                        pertag.prevtag = current_tag;
                        pertag.current_tag = i + 1;
                    }
                }
            }
        }
    }

    apply_pertag_settings(&mut globals);

    drop(globals);
    focus(None);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn tag_mon(arg: &Arg) {
    let (sel_win, has_multiple_mons) = {
        let globals = get_globals();
        (
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel)),
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
                let mon = if let Some(sel_mon_id) = globals.selmon {
                    globals.monitors.get(sel_mon_id)
                } else {
                    None
                };
                let (mx, my, ww, wh) = mon
                    .map(|m| (m.mx, m.my, m.ww, m.wh))
                    .unwrap_or((0, 0, 0, 0));
                (c.x, c.y, mx, my, ww, wh)
            } else {
                return;
            }
        };

        let xfact = (c_x - mon_mx) as f32 / mon_ww as f32;
        let yfact = (c_y - mon_my) as f32 / mon_wh as f32;

        let (target_mx, target_my, target_ww, target_wh) = {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(target_id) {
                (mon.mx, mon.my, mon.ww, mon.wh)
            } else {
                (0, 0, 0, 0)
            }
        };

        send_mon(win, target_id);

        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.x = target_mx + (target_ww as f32 * xfact) as i32;
            client.y = target_my + (target_wh as f32 * yfact) as i32;
        }

        if let Some(sel_mon_id) = globals.selmon {
            arrange(Some(sel_mon_id));
        }

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

pub fn tag_to_left(arg: &Arg) {
    let offset = arg.i.max(1);
    shift_tag(DIR_LEFT, offset);
}

pub fn tag_to_right(arg: &Arg) {
    let offset = arg.i.max(1);
    shift_tag(DIR_RIGHT, offset);
}

fn shift_tag(dir: i32, offset: i32) {
    let sel_win = {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
    };

    let Some(win) = sel_win else { return };

    let (current_tag, overlay) = {
        let globals = get_globals();
        let current_tag = if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.pertag.as_ref().map(|p| p.current_tag).unwrap_or(0)
            } else {
                return;
            }
        } else {
            return;
        };
        let overlay = if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.overlay)
        } else {
            None
        };
        (current_tag, overlay)
    };

    if Some(win) == overlay {
        let mode = if dir == DIR_LEFT { 3 } else { 1 };
        crate::overlay::set_overlay_mode(mode);
        return;
    }

    if dir == DIR_LEFT && current_tag == 1 {
        return;
    }
    if dir == DIR_RIGHT && current_tag == 20 {
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
            let mon_mw = if let Some(sel_mon_id) = globals.selmon {
                globals.monitors.get(sel_mon_id).map(|m| m.mw).unwrap_or(0)
            } else {
                0
            };
            let (c_x, c_y) = globals
                .clients
                .get(&win)
                .map(|c| (c.x, c.y))
                .unwrap_or((0, 0));
            (mon_mw, c_x, c_y)
        };

        let anim_offset = (mon_mw / 10) * if dir == DIR_LEFT { -1 } else { 1 };
        crate::animation::animate_client(win, c_x + anim_offset, c_y, 0, 0, 7, 0);
    }

    let (tagset, tagmask) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (mon.tagset[mon.seltags as usize], globals.tagmask)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let is_single_tag = (tagset & tagmask).count_ones() == 1;

    if is_single_tag {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if dir == DIR_LEFT && tagset > 1 {
                client.tags >>= offset;
                drop(globals);
                focus(None);
                if let Some(sel_mon_id) = get_globals().selmon {
                    arrange(Some(sel_mon_id));
                }
            } else if dir == DIR_RIGHT && (tagset & (tagmask >> 1)) != 0 {
                client.tags <<= offset;
                drop(globals);
                focus(None);
                if let Some(sel_mon_id) = get_globals().selmon {
                    arrange(Some(sel_mon_id));
                }
            }
        }
    }
}

fn reset_sticky_client(win: Window) {
    let target_tags = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.pertag.as_ref().map(|p| 1 << (p.current_tag - 1))
            } else {
                None
            }
        } else {
            None
        }
    };

    let mut globals = get_globals_mut();
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
    view_scroll(DIR_LEFT);
}

pub fn view_to_right(_arg: &Arg) {
    view_scroll(DIR_RIGHT);
}

fn view_scroll(dir: i32) {
    let (current_tag, tagset, tagmask) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                let current_tag = mon.pertag.as_ref().map(|p| p.current_tag).unwrap_or(0);
                (
                    current_tag,
                    mon.tagset[mon.seltags as usize],
                    globals.tagmask,
                )
            } else {
                return;
            }
        } else {
            return;
        }
    };

    if dir == DIR_LEFT && current_tag == 1 {
        return;
    }
    if dir == DIR_RIGHT && current_tag == 20 {
        return;
    }

    let is_single_tag = (tagset & tagmask).count_ones() == 1;
    if !is_single_tag {
        return;
    }

    let new_tagset = if dir == DIR_LEFT {
        if tagset <= 1 {
            return;
        }
        tagset >> 1
    } else {
        if (tagset & (tagmask >> 1)) == 0 {
            return;
        }
        tagset << 1
    };

    let mut globals = get_globals_mut();
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.seltags ^= 1;
            mon.tagset[mon.seltags as usize] = new_tagset;

            if let Some(ref mut pertag) = mon.pertag {
                pertag.prevtag = pertag.current_tag;
                let mut i = 0;
                while (new_tagset & (1 << i)) == 0 {
                    i += 1;
                }
                pertag.current_tag = i + 1;
            }
        }
    }

    apply_pertag_settings(&mut globals);

    drop(globals);
    focus(None);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn move_left(arg: &Arg) {
    tag_to_left(arg);
    view_to_left(arg);
}

pub fn move_right(arg: &Arg) {
    tag_to_right(arg);
    view_to_right(arg);
}

pub fn shift_view(arg: &Arg) {
    let direction = arg.i;

    let (tagset, numtags) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (mon.tagset[mon.seltags as usize], globals.numtags)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let mut next_seltags = tagset;
    let mut visible = false;
    let mut count = 0;
    let mut shift = direction;

    while !visible && count < 10 {
        if direction > 0 {
            next_seltags = (tagset << shift) | (tagset >> (numtags as usize - 1 - shift as usize));
        } else {
            next_seltags =
                (tagset >> (-shift) as usize) | (tagset << (numtags as usize - 1 + shift as usize));
        }

        let globals = get_globals();
        let mut current = if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.clients)
        } else {
            None
        };

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

pub fn swap_tags(arg: &Arg) {
    let ui = compute_prefix(arg);
    let newtag = ui & get_globals().tagmask;

    let (current_tag, current_tagset) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                let current_tag = mon.pertag.as_ref().map(|p| p.current_tag).unwrap_or(0);
                (current_tag, mon.tagset[mon.seltags as usize])
            } else {
                return;
            }
        } else {
            return;
        }
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
        let mut current = if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.clients)
        } else {
            None
        };

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
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags ^= current_tagset ^ newtag;
            if client.tags == 0 {
                client.tags = newtag;
            }
        }
    }

    let mut globals = get_globals_mut();
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.tagset[mon.seltags as usize] = newtag;
            if let Some(ref mut pertag) = mon.pertag {
                if pertag.prevtag == target_idx + 1 {
                    pertag.prevtag = current_tag;
                }
                pertag.current_tag = target_idx + 1;
            }
        }
    }

    drop(globals);
    focus(None);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn follow_view(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
    };

    let Some(win) = sel_win else { return };

    let prevtag = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.pertag.as_ref().map(|p| p.prevtag).unwrap_or(1)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.tags = 1 << (prevtag - 1);
    }

    drop(globals);

    let a = Arg {
        ui: 1 << (prevtag - 1),
        ..Default::default()
    };
    view(&a);
    focus(Some(win));

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn reset_sticky(c: &mut ClientInner) {
    if !c.issticky {
        return;
    }
    c.issticky = false;

    let globals = get_globals();
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            if let Some(ref pertag) = mon.pertag {
                c.tags = 1 << (pertag.current_tag - 1);
            }
        }
    }
}

pub fn toggle_overview(_arg: &Arg) {
    let (has_clients, current_tag, prevtag) = {
        let globals = get_globals();
        let has_clients = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .map(|m| m.clients.is_some())
                .unwrap_or(false)
        } else {
            false
        };
        let current_tag = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .and_then(|m| m.pertag.as_ref().map(|p| p.current_tag))
        } else {
            None
        };
        let prevtag = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .and_then(|m| m.pertag.as_ref().map(|p| p.prevtag))
        } else {
            None
        };
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
            if let Some(sel_mon_id) = get_globals().selmon {
                restore_all_floating(Some(sel_mon_id));
            }
            win_view(&Arg::default());
        } else {
            if let Some(sel_mon_id) = get_globals().selmon {
                save_all_floating(Some(sel_mon_id));
            }
            view(&Arg {
                ui: !0,
                ..Default::default()
            });
        }
    }
}

pub fn toggle_fullscreen_overview(_arg: &Arg) {
    let current_tag = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .and_then(|m| m.pertag.as_ref().map(|p| p.current_tag))
        } else {
            None
        }
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
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                if let Some(ref pertag) = mon.pertag {
                    (pertag.current_tag, pertag.prevtag)
                } else {
                    return;
                }
            } else {
                return;
            }
        } else {
            return;
        }
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
    let last_client = unsafe { crate::client::LAST_CLIENT };
    if let Some(win) = last_client {
        focus(Some(win));
    }
}

pub fn win_view(_arg: &Arg) {
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
                        let mut current = if let Some(sel_mon_id) = globals.selmon {
                            globals.monitors.get(sel_mon_id).and_then(|m| m.clients)
                        } else {
                            None
                        };

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
                            if let Some(sel_mon_id) = globals.selmon {
                                globals
                                    .monitors
                                    .get(sel_mon_id)
                                    .and_then(|m| m.pertag.as_ref().map(|p| p.current_tag))
                            } else {
                                None
                            }
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

pub fn toggle_show_tags(_arg: &Arg) {}

pub fn toggle_alt_tag(_arg: &Arg) {}

pub fn alt_tab_free(_arg: &Arg) {}

pub fn zoom(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };
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
