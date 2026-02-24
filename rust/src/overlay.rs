use crate::animation::animate_client_rect;
use crate::client::save_border_width;
use crate::client::{attach, attach_stack, detach, detach_stack, resize};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

//TODO: maybe overlay should be a struct with the overlay relevant state kept
//there

pub fn overlay_exists() -> bool {
    let globals = get_globals();

    let overlay_win = match globals.monitors.get(globals.selmon).and_then(|m| m.overlay) {
        Some(w) => w,
        None => return false,
    };

    globals.clients.contains_key(&overlay_win)
}

pub fn create_overlay() {
    let (sel_win, sel_overlay, sel_fullscreen) = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        let sel_win = match mon.sel {
            Some(w) => w,
            None => return,
        };
        let sel_overlay = mon.overlay;
        let sel_fullscreen = globals
            .clients
            .get(&sel_win)
            .map(|c| c.is_fullscreen && !c.isfakefullscreen)
            .unwrap_or(false);
        (sel_win, sel_overlay, sel_fullscreen)
    };

    if sel_fullscreen {
        crate::floating::temp_fullscreen();
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

    save_border_width(temp_client);

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
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            (mon.overlaymode, mon.work_rect.w, mon.work_rect.h)
        } else {
            (OverlayMode::default(), 0, 0)
        }
    };

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&temp_client) {
            if overlay_mode.is_vertical() {
                client.geo.h = mon_wh / 3;
            } else {
                client.geo.w = mon_ww / 3;
            }
        }
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _globals = get_globals();
        let _ = conn.configure_window(
            temp_client,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }

    show_overlay();
}

pub fn reset_overlay() {
    if !overlay_exists() {
        return;
    }

    let overlay_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.overlay)
    };

    let overlay_win = match overlay_win {
        Some(w) => w,
        None => return,
    };

    let selmon = {
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

    arrange(Some(selmon));

    focus(Some(overlay_win));
}

pub fn show_overlay() {
    if !overlay_exists() {
        return;
    }

    let globals = get_globals();

    if globals.monitors.is_empty() {
        return;
    }

    let selmon_id = globals.selmon;

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
            OverlayMode::Top => {
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
            OverlayMode::Right => {
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
            OverlayMode::Bottom => {
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
            OverlayMode::Left => {
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
        }
    }

    {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            if let Some(_client) = globals.clients.get(&overlay_win) {
                let tags = mon.tagset[mon.seltags as usize];
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
                    OverlayMode::Top => (overlay_win as i32, mon.monitor_rect.y + yoffset),
                    OverlayMode::Right => (
                        mon.monitor_rect.x + mon.monitor_rect.w - client_w,
                        mon.monitor_rect.y + 40,
                    ),
                    OverlayMode::Bottom => (
                        mon.monitor_rect.x + 20,
                        mon.monitor_rect.y + mon.monitor_rect.h - client_h,
                    ),
                    OverlayMode::Left => (mon.monitor_rect.x, mon.monitor_rect.y + 40),
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

pub fn hide_overlay() {
    if !overlay_exists() {
        return;
    }

    let globals = get_globals();

    if globals.monitors.is_empty() {
        return;
    }

    let selmon_id = globals.selmon;

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
        crate::floating::temp_fullscreen();
    }

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&overlay_win) {
            client.issticky = false;
        }
    }

    if is_locked {
        match overlay_mode {
            OverlayMode::Top => {
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
            OverlayMode::Right => {
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
            OverlayMode::Bottom => {
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
            OverlayMode::Left => {
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

pub fn set_overlay() {
    if !overlay_exists() {
        return;
    }

    let (overlaystatus, overlay_visible, _mon_tags) = {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            let overlay_win = match mon.overlay {
                Some(w) => w,
                None => return,
            };

            let visible = if let Some(c) = globals.clients.get(&overlay_win) {
                c.is_visible() || c.issticky
            } else {
                false
            };

            (mon.overlaystatus, visible, mon.tagset[mon.seltags as usize])
        } else {
            return;
        }
    };

    if overlaystatus == 0 {
        show_overlay();
    } else if overlay_visible {
        hide_overlay();
    } else {
        show_overlay();
    }
}

pub fn set_overlay_mode(mode: OverlayMode) {
    {
        let globals = get_globals_mut();
        for mon in &mut globals.monitors {
            mon.overlaymode = mode;
        }
    }

    let (has_overlay, mon_wh, mon_ww, overlaystatus) = {
        let globals = get_globals();
        let mon = globals.monitors.get(globals.selmon);
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
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            if let Some(overlay_win) = mon.overlay {
                if let Some(client) = globals.clients.get_mut(&overlay_win) {
                    if mode.is_vertical() {
                        client.geo.h = mon_wh / 3;
                    } else {
                        client.geo.w = mon_ww / 3;
                    }
                }
            }
        }
    }

    if overlaystatus != 0 {
        hide_overlay();
        show_overlay();
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

pub fn set_overlay_mode_cmd(mode: i32) {
    let mode = OverlayMode::from_i32(mode).unwrap_or_default();
    set_overlay_mode(mode);
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
        if let Some(mon) = globals.monitors.get(globals.selmon) {
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
        globals.monitors.get(globals.selmon).and_then(|m| m.overlay)
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
        OverlayMode::Top => {
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
        OverlayMode::Right => {
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
        OverlayMode::Bottom => {
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
        OverlayMode::Left => {
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
    }
}
