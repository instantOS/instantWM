use crate::animation::animate_client;
use crate::bar::draw_bar;
use crate::client::{client_height, client_width, is_visible, next_tiled, resize, resize_client};
use crate::floating::{apply_snap, restore_border_width, save_bw};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange as arrange_mon;
use crate::types::*;
use crate::util::{max, min};
use x11rb::protocol::xproto::*;

pub fn tile(m: &mut MonitorInner) {
    let framecount = {
        let g = get_globals();
        if g.animated && client_count() > 5 {
            4
        } else {
            7
        }
    };

    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let mut mw: i32;
    if n > m.nmaster as u32 {
        mw = if m.nmaster > 0 {
            (m.mfact * m.ww as f32) as i32
        } else {
            0
        };
    } else {
        mw = m.ww;
        if n > 1 && n < m.nmaster as u32 {
            m.nmaster = n as i32;
            tile(m);
            return;
        }
    }

    let mut my: u32 = 0;
    let mut ty: u32 = 0;
    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        if i < m.nmaster as u32 {
            let h = (m.wh - my as i32) / (min(n, m.nmaster as u32) - i) as i32;

            if n == 2 {
                animate_client(
                    win,
                    m.wx,
                    m.wy + my as i32,
                    mw - 2 * border_width,
                    h - 2 * border_width,
                    0,
                    0,
                );
            } else {
                animate_client(
                    win,
                    m.wx,
                    m.wy + my as i32,
                    mw - 2 * border_width,
                    h - 2 * border_width,
                    framecount,
                    0,
                );
                if m.nmaster == 1 && n > 1 {
                    let g = get_globals();
                    if let Some(c) = g.clients.get(&win) {
                        mw = c.w + c.border_width * 2;
                    }
                }
            }

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if my as i32 + client_height(c) < m.wh {
                    my += client_height(c) as u32;
                }
            }
        } else {
            let h = (m.wh - ty as i32) / (n - i) as i32;
            animate_client(
                win,
                m.wx + mw,
                m.wy + ty as i32,
                m.ww - mw - 2 * border_width,
                h - 2 * border_width,
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if ty as i32 + client_height(c) < m.wh {
                    ty += client_height(c) as u32;
                }
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn monocle(m: &mut MonitorInner) {
    let mut n: u32 = 0;
    let g = get_globals();

    if g.animated {
        if let Some(selmon_id) = g.selmon {
            if let Some(mon) = g.monitors.get(selmon_id) {
                if let Some(sel_win) = mon.sel {
                    let x11 = get_x11();
                    if let Some(ref conn) = x11.conn {
                        let _ = configure_window(
                            conn,
                            sel_win,
                            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                        );
                        let _ = conn.flush();
                    }
                }
            }
        }
    }

    let mut c_win = m.clients;
    while let Some(win) = c_win {
        let g = get_globals();
        if let Some(c) = g.clients.get(&win) {
            if is_visible(c) {
                n += 1;
            }
            c_win = c.next;
        } else {
            break;
        }
    }

    if n > 0 {
        let symbol = format!("[{}]", n);
        let bytes = symbol.as_bytes();
        let len = bytes.len().min(15);
        m.ltsymbol[..len].copy_from_slice(&bytes[..len]);
        if len < 16 {
            m.ltsymbol[len] = 0;
        }
    }

    let g = get_globals();
    let animated = g.animated;
    let sel_win = g
        .selmon
        .and_then(|id| g.monitors.get(id).and_then(|mon| mon.sel));

    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        let frames = if animated && Some(win) == sel_win {
            7
        } else {
            0
        };
        animate_client(
            win,
            m.wx,
            m.wy,
            m.ww - 2 * border_width,
            m.wh - 2 * border_width,
            frames,
            0,
        );

        c_win = next_tiled(next_client);
    }
}

pub fn grid(m: &mut MonitorInner) {
    let g = get_globals();
    if m.clientcount <= 2 && m.mw > m.mh {
        tile(m);
        return;
    }

    let framecount = if g.animated && client_count() > 5 {
        3
    } else {
        6
    };

    let mut n: i32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let mut rows: i32 = 0;
    for r in 0..=n / 2 {
        if r * r >= n {
            rows = r;
            break;
        }
    }

    let cols: u32 = if rows > 0 && (rows - 1) * rows >= n {
        (rows - 1) as u32
    } else {
        rows as u32
    };

    let ch = m.wh / if rows > 0 { rows } else { 1 };
    let cw = m.ww / if cols > 0 { cols as i32 } else { 1 };

    let mut i: i32 = 0;
    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        let cx = m.wx + (i / rows) * cw;
        let cy = m.wy + (i % rows) * ch;

        let ah = if (i + 1) % rows == 0 {
            m.wh - ch * rows
        } else {
            0
        };
        let aw = if i >= rows * (cols as i32 - 1) {
            m.ww - cw * cols as i32
        } else {
            0
        };

        animate_client(
            win,
            cx,
            cy,
            cw - 2 * border_width + aw,
            ch - 2 * border_width + ah,
            framecount,
            0,
        );

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn deck(m: &mut MonitorInner) {
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let dn = n as i32 - m.nmaster;
    if dn > 0 {
        let symbol = format!("D {}", dn);
        let bytes = symbol.as_bytes();
        let len = bytes.len().min(15);
        m.ltsymbol[..len].copy_from_slice(&bytes[..len]);
        if len < 16 {
            m.ltsymbol[len] = 0;
        }
    }

    let mw: u32 = if n > m.nmaster as u32 {
        if m.nmaster > 0 {
            (m.mfact * m.ww as f32) as u32
        } else {
            0
        }
    } else {
        m.ww as u32
    };

    let mut my: u32 = 0;
    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        if i < m.nmaster as u32 {
            let h = (m.wh - my as i32) / (min(n, m.nmaster as u32) - i) as i32;
            resize(
                win,
                m.wx,
                m.wy + my as i32,
                mw as i32 - 2 * border_width,
                h - 2 * border_width,
                false,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                my += client_height(c) as u32;
            }
        } else {
            resize(
                win,
                m.wx + mw as i32,
                m.wy,
                m.ww - mw as i32 - 2 * border_width,
                m.wh - 2 * border_width,
                false,
            );
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn bstack(m: &mut MonitorInner) {
    let framecount = {
        let g = get_globals();
        if g.animated && client_count() > 4 {
            4
        } else {
            7
        }
    };

    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let mh: i32;
    let tw: i32;
    let ty: i32;

    if n > m.nmaster as u32 {
        mh = if m.nmaster > 0 {
            (m.mfact * m.wh as f32) as i32
        } else {
            0
        };
        tw = m.ww / (n - m.nmaster as u32) as i32;
        ty = m.wy + mh;
    } else {
        mh = m.wh;
        tw = m.ww;
        ty = m.wy;
    }

    let mut mx: i32 = 0;
    let mut tx: i32 = m.wx;
    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        if i < m.nmaster as u32 {
            let w = (m.ww - mx) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client(
                win,
                m.wx + mx,
                m.wy,
                w - 2 * border_width,
                mh - 2 * border_width,
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                mx += client_width(c);
            }
        } else {
            let h = m.wh - mh;
            animate_client(
                win,
                tx,
                ty,
                tw - 2 * border_width,
                h - 2 * border_width,
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if tw != m.ww {
                    tx += client_width(c);
                }
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn bstackhoriz(m: &mut MonitorInner) {
    let framecount = {
        let g = get_globals();
        if g.animated && client_count() > 4 {
            4
        } else {
            7
        }
    };

    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let mh: i32;
    let th: i32;
    let ty: i32;

    if n > m.nmaster as u32 {
        mh = if m.nmaster > 0 {
            (m.mfact * m.wh as f32) as i32
        } else {
            0
        };
        th = (m.wh - mh) / (n - m.nmaster as u32) as i32;
        ty = m.wy + mh;
    } else {
        th = m.wh;
        mh = m.wh;
        ty = m.wy;
    }

    let mut mx: i32 = 0;
    let tx: i32 = m.wx;
    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        if i < m.nmaster as u32 {
            let w = (m.ww - mx) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client(
                win,
                m.wx + mx,
                m.wy,
                w - 2 * border_width,
                mh - 2 * border_width,
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                mx += client_width(c);
            }
        } else {
            animate_client(
                win,
                tx,
                ty,
                m.ww - 2 * border_width,
                th - 2 * border_width,
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if th != m.wh {
                    let mut new_ty = ty;
                    new_ty += client_height(c);
                    drop(g);
                    ty = new_ty;
                }
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn tcl(m: &mut MonitorInner) {
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let first_win = next_tiled(m.clients);
    if first_win.is_none() {
        return;
    }

    let mw = (m.mfact * m.ww as f32) as i32;
    let sw = (m.ww - mw) / 2;

    let (bdw, first_next) = {
        let g = get_globals();
        if let Some(c) = g.clients.get(&first_win.unwrap()) {
            (2 * c.border_width, c.next)
        } else {
            (0, None)
        }
    };

    resize(
        first_win.unwrap(),
        if n < 3 { m.wx } else { m.wx + sw },
        m.wy,
        if n == 1 { m.ww - bdw } else { mw - bdw },
        m.wh - bdw,
        false,
    );

    if n <= 1 {
        return;
    }
    let n = n - 1;

    let w = (m.ww - mw) / if n > 1 { 2 } else { 1 };

    let mut c_win = next_tiled(first_next);
    if n > 1 {
        let x = m.wx + mw + sw;
        let mut y = m.wy;
        let h = m.wh / (n / 2);

        let bh = {
            let g = get_globals();
            g.bh
        };

        let actual_h = if h < bh { m.wh } else { h };

        let mut i: u32 = 0;
        while c_win.is_some() && i < n / 2 {
            let win = c_win.unwrap();
            let (border_width, next_client) = {
                let g = get_globals();
                if let Some(c) = g.clients.get(&win) {
                    (c.border_width, c.next)
                } else {
                    (0, None)
                }
            };

            let rh = if i + 1 == n / 2 {
                m.wy + m.wh - y - 2 * border_width
            } else {
                actual_h - 2 * border_width
            };
            resize(win, x, y, w - 2 * border_width, rh, false);

            if actual_h != m.wh {
                let g = get_globals();
                if let Some(c) = g.clients.get(&win) {
                    y = c.y + client_height(c);
                }
            }

            i += 1;
            c_win = next_tiled(next_client);
        }
    }

    let x = if (n + 1) / 2 == 1 { mw + m.wx } else { m.wx };
    let mut y = m.wy;
    let h = m.wh / ((n + 1) / 2);

    let bh = {
        let g = get_globals();
        g.bh
    };

    let actual_h = if h < bh { m.wh } else { h };

    let mut i: u32 = 0;
    while c_win.is_some() {
        let win = c_win.unwrap();
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        let rh = if i + 1 == (n + 1) / 2 {
            m.wy + m.wh - y - 2 * border_width
        } else {
            actual_h - 2 * border_width
        };
        resize(win, x, y, w - 2 * border_width, rh, false);

        if actual_h != m.wh {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                y = c.y + client_height(c);
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn overviewlayout(m: &mut MonitorInner) {
    let n = all_client_count();
    if n == 0 {
        return;
    }

    let mut gridwidth = 1;
    while gridwidth * gridwidth < n {
        gridwidth += 1;
    }

    let (selmon_mx, selmon_my, selmon_wh, selmon_ww, selmon_showbar, selmon_barwin) = {
        let g = get_globals();
        if let Some(selmon_id) = g.selmon {
            if let Some(mon) = g.monitors.get(selmon_id) {
                (mon.mx, mon.my, mon.wh, mon.ww, mon.showbar, mon.barwin)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let bh = {
        let g = get_globals();
        g.bh
    };

    let mut tmpx = selmon_mx;
    let mut tmpy = selmon_my + if selmon_showbar { bh } else { 0 };
    let lineheight = selmon_wh / gridwidth;
    let colwidth = selmon_ww / gridwidth;

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let mut wc = ConfigureWindowAux::new();
        wc = wc.stack_mode(StackMode::ABOVE).sibling(selmon_barwin);

        let mut c_win = m.clients;
        while let Some(win) = c_win {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                let is_hidden = c.oldstate != 0;
                let is_overlay = m.overlay == Some(win);

                if is_hidden || is_overlay {
                    c_win = c.next;
                    continue;
                }

                let (cw, ch, is_floating) = (c.w, c.h, c.isfloating);
                let next_client = c.next;
                drop(g);

                if is_floating {
                    save_floating(win);
                }

                resize(win, tmpx, tmpy, cw, ch, false);

                let _ = configure_window(
                    conn,
                    win,
                    &ConfigureWindowAux::new()
                        .stack_mode(StackMode::ABOVE)
                        .sibling(selmon_barwin),
                );

                if tmpx + colwidth < selmon_mx + selmon_ww {
                    tmpx += colwidth;
                } else {
                    tmpx = selmon_mx;
                    tmpy += lineheight;
                }

                c_win = next_client;
            } else {
                break;
            }
        }

        let _ = conn.flush();
    }
}

pub fn floatl(m: &mut MonitorInner) {
    let g = get_globals();
    let animated_store = g.animated;

    drop(g);
    {
        let mut g = get_globals_mut();
        g.animated = false;
    }

    let mut c_win = m.clients;
    while let Some(win) = c_win {
        let g = get_globals();
        if let Some(c) = g.clients.get(&win) {
            if !is_visible(c) {
                c_win = c.next;
                continue;
            }

            let snapstatus = c.snapstatus;
            let next_client = c.next;
            drop(g);

            if snapstatus != SnapPosition::None {
                apply_snap_for_window(win, m);
            }

            c_win = next_client;
        } else {
            break;
        }
    }

    let g = get_globals();
    let selmon_id = g.selmon;
    drop(g);

    if let Some(id) = selmon_id {
        let mut g = get_globals_mut();
        if let Some(mon) = g.monitors.get_mut(id) {
            restack(mon);
        }
    }

    {
        let g = get_globals();
        if let Some(selmon_id) = g.selmon {
            if let Some(mon) = g.monitors.get(selmon_id) {
                if let Some(sel_win) = mon.sel {
                    let x11 = get_x11();
                    if let Some(ref conn) = x11.conn {
                        let _ = configure_window(
                            conn,
                            sel_win,
                            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                        );
                        let _ = conn.flush();
                    }
                }
            }
        }
    }

    if animated_store {
        let mut g = get_globals_mut();
        g.animated = true;
    }
}

fn apply_snap_for_window(win: Window, m: &MonitorInner) {
    let g = get_globals();
    if let Some(c) = g.clients.get(&win) {
        let snapstatus = c.snapstatus;
        let border_width = c.border_width;

        drop(g);

        let (x, y, w, h) = match snapstatus {
            SnapPosition::Top => (
                m.wx,
                m.wy,
                m.ww - 2 * border_width,
                m.wh / 2 - 2 * border_width,
            ),
            SnapPosition::TopRight => (
                m.wx + m.ww / 2,
                m.wy,
                m.ww / 2 - 2 * border_width,
                m.wh / 2 - 2 * border_width,
            ),
            SnapPosition::Right => (
                m.wx + m.ww / 2,
                m.wy,
                m.ww / 2 - 2 * border_width,
                m.wh - 2 * border_width,
            ),
            SnapPosition::BottomRight => (
                m.wx + m.ww / 2,
                m.wy + m.wh / 2,
                m.ww / 2 - 2 * border_width,
                m.wh / 2 - 2 * border_width,
            ),
            SnapPosition::Bottom => (
                m.wx,
                m.wy + m.wh / 2,
                m.ww - 2 * border_width,
                m.wh / 2 - 2 * border_width,
            ),
            SnapPosition::BottomLeft => (
                m.wx,
                m.wy + m.wh / 2,
                m.ww / 2 - 2 * border_width,
                m.wh / 2 - 2 * border_width,
            ),
            SnapPosition::Left => (
                m.wx,
                m.wy,
                m.ww / 2 - 2 * border_width,
                m.wh - 2 * border_width,
            ),
            SnapPosition::TopLeft => (
                m.wx,
                m.wy,
                m.ww / 2 - 2 * border_width,
                m.wh / 2 - 2 * border_width,
            ),
            SnapPosition::Maximized => {
                (m.wx, m.wy, m.ww - 2 * border_width, m.wh - 2 * border_width)
            }
            SnapPosition::None => return,
        };

        resize(win, x, y, w, h, false);
    }
}

fn save_floating(win: Window) {
    let mut g = get_globals_mut();
    if let Some(c) = g.clients.get_mut(&win) {
        c.saved_float_x = c.x;
        c.saved_float_y = c.y;
        c.saved_float_width = c.w;
        c.saved_float_height = c.h;
    }
}

pub fn arrange(mon_id: Option<MonitorId>) {
    reset_cursor();

    if let Some(id) = mon_id {
        let mut g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(id) {
            let stack = m.stack;
            drop(g);
            show_hide(stack);
        }
        let mut g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(id) {
            arrange_monitor(m);
            restack(m);
        }
    } else {
        let stack_list: Vec<Option<Window>> = {
            let g = get_globals();
            g.monitors.iter().map(|m| m.stack).collect()
        };

        for stack in stack_list {
            show_hide(stack);
        }

        let mut g = get_globals_mut();
        for m in g.monitors.iter_mut() {
            arrange_monitor(m);
        }
    }
}

pub fn arrange_monitor(m: &mut MonitorInner) {
    m.clientcount = client_count_mon(m) as u32;

    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        let win = c_win.unwrap();
        let g = get_globals();

        let (is_floating, is_fullscreen, mon_clientcount, has_arrange, is_monocle) = {
            if let Some(c) = g.clients.get(&win) {
                let mon_id = c.mon_id;
                let is_floating = c.isfloating;
                let is_fullscreen = c.is_fullscreen;
                let border_width = c.border_width;

                if let Some(mid) = mon_id {
                    if let Some(mon) = g.monitors.get(mid) {
                        let clientcount = mon.clientcount;
                        let has_arrange = mon.sellt == 0;
                        let is_monocle = false;
                        (
                            is_floating,
                            is_fullscreen,
                            clientcount,
                            has_arrange,
                            is_monocle,
                        )
                    } else {
                        (is_floating, is_fullscreen, 0, true, false)
                    }
                } else {
                    (is_floating, is_fullscreen, 0, true, false)
                }
            } else {
                c_win = None;
                continue;
            }
        };

        drop(g);

        {
            let mut g = get_globals_mut();
            if let Some(c) = g.clients.get_mut(&win) {
                if !is_floating
                    && !is_fullscreen
                    && ((mon_clientcount == 1 && has_arrange) || is_monocle)
                {
                    save_bw(c);
                    c.border_width = 0;
                } else {
                    restore_border_width(c);
                }
            }
        }

        let g = get_globals();
        if let Some(c) = g.clients.get(&win) {
            c_win = next_tiled(c.next);
        } else {
            break;
        }
    }

    let g = get_globals();
    if let Some(symbol) = get_layout_symbol(m) {
        let bytes = symbol.as_bytes();
        let len = bytes.len().min(15);
        m.ltsymbol[..len].copy_from_slice(&bytes[..len]);
        if len < 16 {
            m.ltsymbol[len] = 0;
        }
    }

    let arrange_func = get_tiling_layout_func(m);
    drop(g);

    if let Some(func) = arrange_func {
        func(m);
    } else {
        floatl(m);
    }

    let mut g = get_globals_mut();
    if let Some(ref fullscreen_win) = m.overlay {
        if let Some(c) = g.clients.get_mut(fullscreen_win) {
            let tbw = c.border_width;
            if c.isfloating {
                save_floating(*fullscreen_win);
            }
            let showbar_offset = if m.showbar { g.bh } else { 0 };
            resize(
                *fullscreen_win,
                m.mx,
                m.my + showbar_offset,
                m.mw - 2 * tbw,
                m.mh - showbar_offset - 2 * tbw,
                false,
            );
        }
    }
}

pub fn restack(m: &mut MonitorInner) {
    let is_overview = is_overview_layout(m);
    if is_overview {
        return;
    }

    draw_bar(m);

    let sel_win = m.sel;
    if sel_win.is_none() {
        return;
    }
    let sel_win = sel_win.unwrap();

    let g = get_globals();
    let has_arrange = m.sellt == 0;
    drop(g);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let g = get_globals();
        let is_floating = g
            .clients
            .get(&sel_win)
            .map(|c| c.isfloating)
            .unwrap_or(false);
        drop(g);

        if is_floating || !has_arrange {
            let _ = configure_window(
                conn,
                sel_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
        }

        if has_arrange {
            let mut wc = ConfigureWindowAux::new();
            wc = wc.stack_mode(StackMode::BELOW).sibling(m.barwin);

            let mut s_win = m.stack;
            while let Some(win) = s_win {
                let g = get_globals();
                if let Some(c) = g.clients.get(&win) {
                    let is_floating = c.isfloating;
                    let visible = is_visible(c);
                    let snext = c.snext;
                    drop(g);

                    if !is_floating && visible {
                        let _ = configure_window(conn, win, &wc);
                        wc = ConfigureWindowAux::new()
                            .stack_mode(StackMode::ABOVE)
                            .sibling(win);
                    }

                    s_win = snext;
                } else {
                    break;
                }
            }
        }

        let _ = conn.flush();

        let mut ev: Option<GenericEvent> = None;
        while let Ok(event) = conn.poll_for_event() {
            ev = event;
            if ev.is_none() {
                break;
            }
        }
    }
}

fn is_overview_layout(_m: &MonitorInner) -> bool {
    false
}

fn get_layout_symbol(_m: &MonitorInner) -> Option<&'static str> {
    let g = get_globals();
    if g.layouts.is_empty() {
        return None;
    }
    Some(g.layouts.first()?.symbol)
}

fn get_tiling_layout_func(_m: &MonitorInner) -> Option<fn(&mut MonitorInner)> {
    let g = get_globals();
    if g.layouts.is_empty() {
        return None;
    }
    Some(g.layouts.first()?.arrange)
}

pub fn set_layout(arg: &Arg) {
    let mut multimon = false;
    let tagprefix = {
        let g = get_globals();
        g.tagprefix
    };

    if tagprefix {
        multimon = true;
        let layout_idx = arg.v;
        {
            let mut g = get_globals_mut();
            for m in g.monitors.iter_mut() {
                for i in 0..20 {
                    if arg.v.is_none() || layout_idx != get_current_layout_idx(m) {
                        if let Some(ref mut pertag) = m.pertag {
                            pertag.sellts[i] ^= 1;
                        }
                    }
                    if let Some(idx) = layout_idx {
                        if let Some(ref mut pertag) = m.pertag {
                            pertag.ltidxs[i][pertag.sellts[i] as usize] = Some(idx);
                        }
                    }
                }
            }
            g.tagprefix = false;
        }
        set_layout(arg);
    } else {
        let layout_idx = arg.v;
        {
            let mut g = get_globals_mut();
            if let Some(selmon_id) = g.selmon {
                if let Some(m) = g.monitors.get_mut(selmon_id) {
                    if arg.v.is_none() || layout_idx != get_current_layout_idx(m) {
                        if let Some(ref mut pertag) = m.pertag {
                            let tag = pertag.current_tag as usize;
                            pertag.sellts[tag] ^= 1;
                        }
                    }
                    if let Some(idx) = layout_idx {
                        if let Some(ref mut pertag) = m.pertag {
                            let tag = pertag.current_tag as usize;
                            pertag.ltidxs[tag][pertag.sellts[tag] as usize] = Some(idx);
                        }
                    }
                }
            }
        }

        let (selmon_sel, ltsymbol) = {
            let g = get_globals();
            let sel = g
                .selmon
                .and_then(|id| g.monitors.get(id).and_then(|m| m.sel));
            let symbol = get_current_layout_symbol();
            (sel, symbol)
        };

        {
            let mut g = get_globals_mut();
            if let Some(selmon_id) = g.selmon {
                if let Some(m) = g.monitors.get_mut(selmon_id) {
                    if let Some(symbol) = ltsymbol {
                        let bytes = symbol.as_bytes();
                        let len = bytes.len().min(15);
                        m.ltsymbol[..len].copy_from_slice(&bytes[..len]);
                        if len < 16 {
                            m.ltsymbol[len] = 0;
                        }
                    }
                }
            }
        }

        if selmon_sel.is_some() {
            let g = get_globals();
            if let Some(selmon_id) = g.selmon {
                arrange(Some(selmon_id));
            }
        } else {
            let mut g = get_globals_mut();
            if let Some(selmon_id) = g.selmon {
                if let Some(m) = g.monitors.get_mut(selmon_id) {
                    draw_bar(m);
                }
            }
        }
    }

    if multimon {
        let tmpmon = {
            let g = get_globals();
            g.selmon
        };

        let monitors: Vec<MonitorId> = {
            let g = get_globals();
            g.monitors.iter().enumerate().map(|(i, _)| i).collect()
        };

        for mon_id in monitors {
            let g = get_globals();
            if Some(mon_id) != g.selmon {
                drop(g);
                let mut g = get_globals_mut();
                g.selmon = Some(mon_id);
                drop(g);
                set_layout(arg);
            }
        }

        let mut g = get_globals_mut();
        g.selmon = tmpmon;
        drop(g);
        crate::focus::focus(None);
    }
}

fn get_current_layout_idx(m: &MonitorInner) -> Option<usize> {
    if let Some(ref pertag) = m.pertag {
        let tag = pertag.current_tag as usize;
        pertag.ltidxs[tag][pertag.sellts[tag] as usize]
    } else {
        None
    }
}

fn get_current_layout_symbol() -> Option<&'static str> {
    let g = get_globals();
    if let Some(selmon_id) = g.selmon {
        if let Some(m) = g.monitors.get(selmon_id) {
            let idx = get_current_layout_idx(m);
            if let Some(i) = idx {
                if i < g.layouts.len() {
                    return Some(g.layouts[i].symbol);
                }
            }
        }
    }
    g.layouts.first().map(|l| l.symbol)
}

pub fn cycle_layout(arg: &Arg) {
    let current_idx = {
        let g = get_globals();
        if let Some(selmon_id) = g.selmon {
            if let Some(m) = g.monitors.get(selmon_id) {
                get_current_layout_idx(m)
            } else {
                None
            }
        } else {
            None
        }
    };

    let g = get_globals();
    let layouts_len = g.layouts.len();
    drop(g);

    if layouts_len == 0 {
        return;
    }

    let current = current_idx.unwrap_or(0);

    let new_idx = if arg.i > 0 {
        let next = current + 1;
        if next >= layouts_len {
            0
        } else {
            next
        }
    } else {
        if current == 0 {
            layouts_len - 1
        } else {
            current - 1
        }
    };

    let skip_overview = {
        let g = get_globals();
        if new_idx < g.layouts.len() {
            g.layouts[new_idx].symbol == "O"
        } else {
            false
        }
    };

    let final_idx = if skip_overview {
        if arg.i > 0 {
            let next = new_idx + 1;
            if next >= layouts_len {
                0
            } else {
                next
            }
        } else {
            if new_idx == 0 {
                layouts_len - 1
            } else {
                new_idx - 1
            }
        }
    } else {
        new_idx
    };

    set_layout(&Arg {
        v: Some(final_idx),
        ..Default::default()
    });
}

pub fn inc_nmaster(arg: &Arg) {
    let ccount = client_count();

    {
        let mut g = get_globals_mut();
        if let Some(selmon_id) = g.selmon {
            if let Some(m) = g.monitors.get_mut(selmon_id) {
                if arg.i > 0 && m.nmaster >= ccount as i32 {
                    m.nmaster = ccount as i32;
                    return;
                }

                let new_nmaster = max(m.nmaster + arg.i, 0);
                m.nmaster = new_nmaster;

                if let Some(ref mut pertag) = m.pertag {
                    let tag = pertag.current_tag as usize;
                    pertag.nmasters[tag] = new_nmaster;
                }
            }
        }
    }

    let g = get_globals();
    if let Some(selmon_id) = g.selmon {
        arrange(Some(selmon_id));
    }
}

pub fn set_mfact(arg: &Arg) {
    if arg.f == 0.0 {
        return;
    }

    let g = get_globals();
    let has_arrange = if let Some(selmon_id) = g.selmon {
        if let Some(m) = g.monitors.get(selmon_id) {
            m.sellt == 0
        } else {
            false
        }
    } else {
        false
    };

    if !has_arrange {
        return;
    }

    let current_mfact = if let Some(selmon_id) = g.selmon {
        if let Some(m) = g.monitors.get(selmon_id) {
            m.mfact
        } else {
            0.55
        }
    } else {
        0.55
    };

    let f = if arg.f < 1.0 {
        arg.f + current_mfact
    } else {
        arg.f - 1.0
    };

    if f < 0.05 || f > 0.95 {
        return;
    }

    drop(g);

    let animated = {
        let g = get_globals();
        g.animated && client_count() > 2
    };

    if animated {
        let mut g = get_globals_mut();
        g.animated = false;
    }

    {
        let mut g = get_globals_mut();
        if let Some(selmon_id) = g.selmon {
            if let Some(m) = g.monitors.get_mut(selmon_id) {
                m.mfact = f;
                if let Some(ref mut pertag) = m.pertag {
                    let tag = pertag.current_tag as usize;
                    pertag.mfacts[tag] = f;
                }
            }
        }
    }

    let g = get_globals();
    if let Some(selmon_id) = g.selmon {
        arrange(Some(selmon_id));
    }

    if animated {
        let mut g = get_globals_mut();
        g.animated = true;
    }
}

pub fn command_layout(arg: &Arg) {
    let layout_number = if arg.ui > 0
        && (arg.ui as usize) < {
            let g = get_globals();
            g.layouts.len()
        } {
        arg.ui as usize
    } else {
        0
    };

    set_layout(&Arg {
        v: Some(layout_number),
        ..Default::default()
    });
}

fn reset_cursor() {
    // TODO: implement cursor reset
}

fn show_hide(win: Option<Window>) {
    crate::client::show_hide(win);
}

pub fn client_count() -> i32 {
    let g = get_globals();
    let mut count = 0;

    if let Some(selmon_id) = g.selmon {
        if let Some(mon) = g.monitors.get(selmon_id) {
            let mut c_win = mon.clients;
            while let Some(win) = c_win {
                if let Some(c) = g.clients.get(&win) {
                    if is_visible(c) && !c.isfloating {
                        count += 1;
                    }
                    c_win = c.next;
                } else {
                    break;
                }
            }
        }
    }

    count
}

pub fn client_count_mon(m: &MonitorInner) -> i32 {
    let g = get_globals();
    let mut count = 0;

    let mut c_win = m.clients;
    while let Some(win) = c_win {
        if let Some(c) = g.clients.get(&win) {
            if is_visible(c) && !c.isfloating {
                count += 1;
            }
            c_win = c.next;
        } else {
            break;
        }
    }

    count
}

pub fn all_client_count() -> i32 {
    let g = get_globals();
    g.clients.len() as i32
}

pub fn get_tiling_layout_func_for_mon(m: &MonitorInner) -> Option<fn(&mut MonitorInner)> {
    get_tiling_layout_func(m)
}

pub fn find_visible_client(start_win: Option<Window>) -> Option<Window> {
    let mut current = start_win;
    let g = get_globals();

    while let Some(win) = current {
        if let Some(c) = g.clients.get(&win) {
            if is_visible(c) {
                return Some(win);
            }
            current = c.next;
        } else {
            break;
        }
    }

    None
}

pub fn fibonacci(m: &mut MonitorInner, spiral: bool) {
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let mut x = m.wx;
    let mut y = m.wy;
    let mut w = m.ww;
    let mut h = m.wh;

    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                (c.border_width, c.next)
            } else {
                (0, None)
            }
        };

        if i > 0 {
            if i % 2 == 0 {
                h /= 2;
                if spiral {
                    y += h;
                }
            } else {
                w /= 2;
                if !spiral {
                    x += w;
                }
            }
        }

        resize(win, x, y, w - 2 * border_width, h - 2 * border_width, false);

        if i % 2 == 0 {
            if !spiral {
                y += h;
            }
        } else {
            if spiral {
                x += w;
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

pub fn spiral(m: &mut MonitorInner) {
    fibonacci(m, true);
}

pub fn dwindle(m: &mut MonitorInner) {
    fibonacci(m, false);
}

pub fn horizgrid(m: &mut MonitorInner) {
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = if let Some(win) = c_win {
            if let Some(c) = g.clients.get(&win) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
    }

    if n == 0 {
        return;
    }

    let framecount = {
        let g = get_globals();
        if g.animated && client_count() > 5 {
            3
        } else {
            6
        }
    };

    let cols = ((n as f32).sqrt() + 0.5) as u32;

    for col in 0..cols {
        let cn = if col == cols - 1 {
            n - (n / cols) * (cols - 1)
        } else {
            n / cols
        };
        let cw = m.ww / cols as i32;

        let mut c_win = next_tiled(m.clients);
        let mut count = 0;
        while count < col * (n / cols) {
            if let Some(win) = c_win {
                let g = get_globals();
                if let Some(c) = g.clients.get(&win) {
                    c_win = next_tiled(c.next);
                } else {
                    break;
                }
            } else {
                break;
            }
            count += 1;
        }

        for row in 0..cn {
            if let Some(win) = c_win {
                let (border_width, next_client) = {
                    let g = get_globals();
                    if let Some(c) = g.clients.get(&win) {
                        (c.border_width, c.next)
                    } else {
                        (0, None)
                    }
                };

                let ch = m.wh / cn as i32;
                let cx = m.wx + col as i32 * cw;
                let cy = m.wy + row as i32 * ch;

                let aw = if col == cols - 1 {
                    m.ww - cols as i32 * cw + cw
                } else {
                    0
                };

                animate_client(
                    win,
                    cx,
                    cy,
                    cw - 2 * border_width + aw,
                    ch - 2 * border_width,
                    framecount,
                    0,
                );

                c_win = next_tiled(next_client);
            }
        }
    }
}

pub fn gaplessgrid(m: &mut MonitorInner) {
    grid(m);
}
