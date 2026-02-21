use crate::animation::animate_client_rect;
use crate::client::{attach, attach_stack, detach, detach_stack, is_visible, resize};
use crate::floating::save_bw_win;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub const OVERLAY_TOP: i32 = 0;
pub const OVERLAY_RIGHT: i32 = 1;
pub const OVERLAY_BOTTOM: i32 = 2;
pub const OVERLAY_LEFT: i32 = 3;

pub fn overlay_exists() -> bool {
    let globals = get_globals();

    let selmon_overlay = if let Some(selmon_id) = globals.selmon {
        globals.monitors.get(selmon_id).and_then(|m| m.overlay)
    } else {
        return false;
    };

    let overlay_win = match selmon_overlay {
        Some(w) => w,
        None => return false,
    };

    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if c_win == overlay_win {
                    return true;
                }
                current = c.next;
            } else {
                break;
            }
        }
    }

    false
}

pub fn create_overlay(_arg: &Arg) {
    let (sel_win, sel_overlay, sel_fullscreen) = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };

        let mon = match globals.monitors.get(selmon_id) {
            Some(m) => m,
            None => return,
        };

        let sel = match mon.sel {
            Some(w) => w,
            None => return,
        };

        let is_fullscreen = globals
            .clients
            .get(&sel)
            .map(|c| c.is_fullscreen && !c.isfakefullscreen)
            .unwrap_or(false);

        let is_overlay = mon.overlay == Some(sel);

        (sel, mon.overlay, is_fullscreen)
    };

    if sel_fullscreen {
        crate::floating::temp_fullscreen(&Arg::default());
    }

    if Some(sel_win) == sel_overlay {
        reset_overlay();
        {
            let globals = get_globals_mut();
            for mon in &mut globals.monitors {
                mon.overlay = None;
            }
        }
        return;
    }

    let temp_client = sel_win;

    reset_overlay();

    {
        let globals = get_globals_mut();
        for mon in &mut globals.monitors {
            mon.overlay = Some(temp_client);
            mon.overlaystatus = 0;
        }
    }

    save_bw_win(temp_client);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&temp_client) {
            client.border_width = 0;
            client.islocked = true;

            if !client.isfloating {
                client.isfloating = true;
            }
        }
    }

    let (overlay_mode, mon_ww, mon_wh) = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        if let Some(mon) = globals.monitors.get(selmon_id) {
            (mon.overlaymode, mon.work_rect.w, mon.work_rect.h)
        } else {
            (0, 0, 0)
        }
    };

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&temp_client) {
            if overlay_mode == OVERLAY_TOP || overlay_mode == OVERLAY_BOTTOM {
                client.geo.h = mon_wh / 3;
            } else {
                client.geo.w = mon_ww / 3;
            }
        }
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let _ = conn.configure_window(
            temp_client,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }

    show_overlay(&Arg::default());
}

pub fn reset_overlay() {
    if !overlay_exists() {
        return;
    }

    let overlay_win = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        globals.monitors.get(selmon_id).and_then(|m| m.overlay)
    };

    let overlay_win = match overlay_win {
        Some(w) => w,
        None => return,
    };

    let mon_id = {
        let globals = get_globals();
        globals.selmon
    };

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            client.border_width = client.old_border_width;
            client.issticky = false;
            client.islocked = false;
            client.isfloating = true;
        }
    }

    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }

    focus(Some(overlay_win));
}

pub fn show_overlay(_arg: &Arg) {
    if !overlay_exists() {
        return;
    }

    let globals = get_globals();
    let selmon_id = match globals.selmon {
        Some(id) => id,
        None => return,
    };

    let mon = match globals.monitors.get(selmon_id) {
        Some(m) => m,
        None => return,
    };

    if mon.overlaystatus != 0 {
        return;
    }

    let bh = globals.bh;
    let mut yoffset = if mon.showbar { bh } else { 0 };

    let overlay_win = match mon.overlay {
        Some(w) => w,
        None => return,
    };

    let current_tag = mon.current_tag as u32;
    let mut current = mon.clients;
    while let Some(c_win) = current {
        if let Some(c) = globals.clients.get(&c_win) {
            if (c.tags & (1 << (current_tag - 1))) != 0 && c.is_fullscreen && !c.isfakefullscreen {
                yoffset = 0;
                break;
            }
            current = c.next;
        } else {
            break;
        }
    }

    drop(globals);

    {
        let globals = get_globals_mut();
        for mon in &mut globals.monitors {
            mon.overlaystatus = 1;
        }
    }

    detach(overlay_win);
    detach_stack(overlay_win);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            client.mon_id = Some(selmon_id);
            client.isfloating = true;
        }
    }

    attach(overlay_win);
    attach_stack(overlay_win);

    let (overlay_mode, mon_mx, mon_my, mon_mw, mon_mh, mon_ww) = {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            (
                mon.overlaymode,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                mon.work_rect.w,
            )
        } else {
            return;
        }
    };

    let (client_w, client_h, is_locked) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&overlay_win) {
            (c.geo.w, c.geo.h, c.islocked)
        } else {
            return;
        }
    };

    if is_locked {
        match overlay_mode {
            OVERLAY_TOP => {
                resize(
                    overlay_win,
                    &Rect {
                        x: mon_mx + 20,
                        y: mon_my + yoffset - client_h,
                        w: mon_ww - 40,
                        h: client_h,
                    },
                    true,
                );
            }
            OVERLAY_RIGHT => {
                resize(
                    overlay_win,
                    &Rect {
                        x: mon_mx + mon_mw - 20,
                        y: mon_my + 40,
                        w: client_w,
                        h: mon_mh - 80,
                    },
                    true,
                );
            }
            OVERLAY_BOTTOM => {
                resize(
                    overlay_win,
                    &Rect {
                        x: mon_mx + 20,
                        y: mon_my + mon_mh,
                        w: mon_ww - 40,
                        h: client_h,
                    },
                    true,
                );
            }
            OVERLAY_LEFT => {
                resize(
                    overlay_win,
                    &Rect {
                        x: mon_mx - client_w + 20,
                        y: mon_my + 40,
                        w: client_w,
                        h: mon_mh - 80,
                    },
                    true,
                );
            }
            _ => {}
        }
    }

    {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            if let Some(client) = globals.clients.get(&overlay_win) {
                let tags = mon.tagset[mon.seltags as usize];
                drop(globals);
                let globals = get_globals_mut();
                if let Some(c) = globals.clients.get_mut(&overlay_win) {
                    c.tags = tags;
                }
            }
        }
    }

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            if !client.isfloating {
                client.isfloating = true;
            }
            client.border_width = 0;
        }
    }

    if is_locked {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.configure_window(
                overlay_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }

        let (target_x, target_y) = {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(selmon_id) {
                match overlay_mode {
                    OVERLAY_TOP => (overlay_win as i32, mon.monitor_rect.y + yoffset),
                    OVERLAY_RIGHT => (
                        mon.monitor_rect.x + mon.monitor_rect.w - client_w,
                        mon.monitor_rect.y + 40,
                    ),
                    OVERLAY_BOTTOM => (
                        mon.monitor_rect.x + 20,
                        mon.monitor_rect.y + mon.monitor_rect.h - client_h,
                    ),
                    OVERLAY_LEFT => (mon.monitor_rect.x, mon.monitor_rect.y + 40),
                    _ => (0, 0),
                }
            } else {
                (0, 0)
            }
        };

        animate_client_rect(
            overlay_win,
            &Rect {
                x: target_x,
                y: target_y,
                w: 0,
                h: 0,
            },
            15,
            0,
        );

        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            client.issticky = true;
        }
    }

    focus(Some(overlay_win));

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(
            overlay_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }
}

pub fn hide_overlay(_arg: &Arg) {
    if !overlay_exists() {
        return;
    }

    let globals = get_globals();
    let selmon_id = match globals.selmon {
        Some(id) => id,
        None => return,
    };

    let mon = match globals.monitors.get(selmon_id) {
        Some(m) => m,
        None => return,
    };

    if mon.overlaystatus == 0 {
        return;
    }

    let overlay_win = match mon.overlay {
        Some(w) => w,
        None => return,
    };

    let (
        is_locked,
        overlay_mode,
        mon_mx,
        mon_my,
        mon_mw,
        mon_mh,
        client_x,
        client_h,
        client_w,
        is_fullscreen,
    ) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&overlay_win) {
            let is_fullscreen = Some(overlay_win) == mon.fullscreen;
            let is_locked = c.islocked;

            let mon = globals.monitors.get(selmon_id).unwrap();
            (
                is_locked,
                mon.overlaymode,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                c.geo.x,
                c.geo.h,
                c.geo.w,
                is_fullscreen,
            )
        } else {
            return;
        }
    };

    if is_fullscreen {
        crate::floating::temp_fullscreen(&Arg::default());
    }

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            client.issticky = false;
        }
    }

    if is_locked {
        match overlay_mode {
            OVERLAY_TOP => {
                animate_client_rect(
                    overlay_win,
                    &Rect {
                        x: client_x,
                        y: 0 - client_h,
                        w: 0,
                        h: 0,
                    },
                    15,
                    0,
                );
            }
            OVERLAY_RIGHT => {
                animate_client_rect(
                    overlay_win,
                    &Rect {
                        x: mon_mx + mon_mw,
                        y: mon_mx + mon_mw,
                        w: 0,
                        h: 0,
                    },
                    15,
                    0,
                );
            }
            OVERLAY_BOTTOM => {
                animate_client_rect(
                    overlay_win,
                    &Rect {
                        x: client_x,
                        y: mon_mh + mon_my,
                        w: 0,
                        h: 0,
                    },
                    15,
                    0,
                );
            }
            OVERLAY_LEFT => {
                animate_client_rect(
                    overlay_win,
                    &Rect {
                        x: mon_mx - client_w,
                        y: 40,
                        w: 0,
                        h: 0,
                    },
                    15,
                    0,
                );
            }
            _ => {}
        }
    }

    {
        let globals = get_globals_mut();
        for mon in &mut globals.monitors {
            mon.overlaystatus = 0;
        }

        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            client.tags = 0;
        }
    }

    focus(None);
    arrange(Some(selmon_id));
}

pub fn set_overlay(_arg: &Arg) {
    if !overlay_exists() {
        return;
    }

    let (overlaystatus, overlay_visible, mon_tags) = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        if let Some(mon) = globals.monitors.get(selmon_id) {
            let overlay_win = match mon.overlay {
                Some(w) => w,
                None => return,
            };

            let visible = if let Some(c) = globals.clients.get(&overlay_win) {
                is_visible(c) || c.issticky
            } else {
                false
            };

            (mon.overlaystatus, visible, mon.tagset[mon.seltags as usize])
        } else {
            return;
        }
    };

    if overlaystatus == 0 {
        show_overlay(&Arg::default());
    } else {
        if overlay_visible {
            hide_overlay(&Arg::default());
        } else {
            show_overlay(&Arg::default());
        }
    }
}

pub fn set_overlay_mode(mode: i32) {
    {
        let globals = get_globals_mut();
        for mon in &mut globals.monitors {
            mon.overlaymode = mode;
        }
    }

    let (has_overlay, mon_wh, mon_ww, overlaystatus) = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        let mon = globals.monitors.get(selmon_id);
        (
            mon.and_then(|m| m.overlay).is_some(),
            mon.map(|m| m.work_rect.h).unwrap_or(0),
            mon.map(|m| m.work_rect.w).unwrap_or(0),
            mon.map(|m| m.overlaystatus).unwrap_or(0),
        )
    };

    if !has_overlay {
        return;
    }

    {
        let globals = get_globals_mut();
        let selmon_id = globals.selmon.unwrap_or(0);
        if let Some(mon) = globals.monitors.get(selmon_id) {
            if let Some(overlay_win) = mon.overlay {
                if let Some(client) = globals.clients.get_mut(&overlay_win) {
                    if mode == OVERLAY_TOP || mode == OVERLAY_BOTTOM {
                        client.geo.h = mon_wh / 3;
                    } else {
                        client.geo.w = mon_ww / 3;
                    }
                }
            }
        }
    }

    if overlaystatus != 0 {
        hide_overlay(&Arg::default());
        show_overlay(&Arg::default());
    }
}

pub fn is_overlay_window(win: Window) -> bool {
    let globals = get_globals();
    for mon in &globals.monitors {
        if mon.overlay == Some(win) {
            return true;
        }
    }
    false
}

pub fn set_overlay_mode_cmd(arg: &Arg) {
    set_overlay_mode(arg.i);
}

pub fn reset_overlay_size() {
    let (
        has_overlay,
        overlay_mode,
        mon_mx,
        mon_my,
        mon_mw,
        mon_mh,
        mon_ww,
        mon_wh,
        mon_showbar,
        bh,
    ) = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        if let Some(mon) = globals.monitors.get(selmon_id) {
            let overlay_win = mon.overlay;
            (
                overlay_win.is_some(),
                mon.overlaymode,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                mon.work_rect.w,
                mon.work_rect.h,
                mon.showbar,
                globals.bh,
            )
        } else {
            return;
        }
    };

    if !has_overlay {
        return;
    }

    let overlay_win = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        globals.monitors.get(selmon_id).and_then(|m| m.overlay)
    };

    let Some(win) = overlay_win else { return };

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isfloating = true;
        }
    }

    let yoffset = if mon_showbar { bh } else { 0 };

    match overlay_mode {
        OVERLAY_TOP => {
            resize(
                win,
                &Rect {
                    x: mon_mx + 20,
                    y: mon_my + yoffset,
                    w: mon_ww - 40,
                    h: mon_wh / 3,
                },
                true,
            );
        }
        OVERLAY_RIGHT => {
            let client_w = {
                let globals = get_globals();
                globals
                    .clients
                    .get(&win)
                    .map(|c| c.geo.w)
                    .unwrap_or(mon_mw / 3)
            };
            resize(
                win,
                &Rect {
                    x: mon_mx + mon_mw - client_w,
                    y: mon_my + 40,
                    w: mon_mw / 3,
                    h: mon_mh - 80,
                },
                true,
            );
        }
        OVERLAY_BOTTOM => {
            let client_h = {
                let globals = get_globals();
                globals
                    .clients
                    .get(&win)
                    .map(|c| c.geo.h)
                    .unwrap_or(mon_wh / 3)
            };
            resize(
                win,
                &Rect {
                    x: mon_mx + 20,
                    y: mon_my + mon_mh - client_h,
                    w: mon_ww - 40,
                    h: mon_wh / 3,
                },
                true,
            );
        }
        OVERLAY_LEFT => {
            resize(
                win,
                &Rect {
                    x: mon_mx,
                    y: mon_my + 40,
                    w: mon_mw / 3,
                    h: mon_mh - 80,
                },
                true,
            );
        }
        _ => {}
    }
}
