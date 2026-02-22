use crate::animation::animate_client_rect;
use crate::bar::draw_bar;
use crate::client::{client_height, client_width, next_tiled, resize, save_border_width};
use crate::floating::restore_border_width_win;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use crate::util::{max, min};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Layout trait implementations ─────────────────────────────────────────────

#[derive(Debug)]
pub struct TileLayout;
pub static TILE_LAYOUT: TileLayout = TileLayout;
impl Layout for TileLayout {
    fn symbol(&self) -> &'static str {
        "+"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        tile(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct GridLayout;
pub static GRID_LAYOUT: GridLayout = GridLayout;
impl Layout for GridLayout {
    fn symbol(&self) -> &'static str {
        "#"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        grid(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct FloatingLayout;
pub static FLOATING_LAYOUT: FloatingLayout = FloatingLayout;
impl Layout for FloatingLayout {
    fn symbol(&self) -> &'static str {
        "-"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct MonocleLayout;
pub static MONOCLE_LAYOUT: MonocleLayout = MonocleLayout;
impl Layout for MonocleLayout {
    fn symbol(&self) -> &'static str {
        "[M]"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        monocle(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
    fn is_monocle(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct VertLayout;
pub static VERT_LAYOUT: VertLayout = VertLayout;
impl Layout for VertLayout {
    fn symbol(&self) -> &'static str {
        "|||"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct DeckLayout;
pub static DECK_LAYOUT: DeckLayout = DeckLayout;
impl Layout for DeckLayout {
    fn symbol(&self) -> &'static str {
        "H[]"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        deck(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct OverviewLayout;
pub static OVERVIEW_LAYOUT: OverviewLayout = OverviewLayout;
impl Layout for OverviewLayout {
    fn symbol(&self) -> &'static str {
        "O"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
    fn is_overview(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct BstackLayout;
pub static BSTACK_LAYOUT: BstackLayout = BstackLayout;
impl Layout for BstackLayout {
    fn symbol(&self) -> &'static str {
        "TTT"
    }
    fn arrange(&self, m: &mut MonitorInner) {
        bstack(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct HorizLayout;
pub static HORIZ_LAYOUT: HorizLayout = HorizLayout;
impl Layout for HorizLayout {
    fn symbol(&self) -> &'static str {
        "==="
    }
    fn arrange(&self, m: &mut MonitorInner) {
        floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
}

/// Returns the currently active layout for the given monitor.
pub fn get_current_layout(m: &MonitorInner) -> &'static dyn Layout {
    let g = get_globals();
    let idx = get_current_layout_idx(m).unwrap_or(0);
    g.layouts.get(idx).copied().unwrap_or(&TILE_LAYOUT)
}

// ─────────────────────────────────────────────────────────────────────────────

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
            (m.mfact * m.work_rect.w as f32) as i32
        } else {
            0
        };
    } else {
        mw = m.work_rect.w;
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
            let h = (m.work_rect.h - my as i32) / (min(n, m.nmaster as u32) - i) as i32;

            if n == 2 {
                animate_client_rect(
                    win,
                    &Rect {
                        x: m.work_rect.x,
                        y: m.work_rect.y + my as i32,
                        w: mw - 2 * border_width,
                        h: h - 2 * border_width,
                    },
                    0,
                    0,
                );
            } else {
                animate_client_rect(
                    win,
                    &Rect {
                        x: m.work_rect.x,
                        y: m.work_rect.y + my as i32,
                        w: mw - 2 * border_width,
                        h: h - 2 * border_width,
                    },
                    framecount,
                    0,
                );
                if m.nmaster == 1 && n > 1 {
                    let g = get_globals();
                    if let Some(c) = g.clients.get(&win) {
                        mw = c.geo.w + c.border_width * 2;
                    }
                }
            }

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if my as i32 + client_height(c) < m.work_rect.h {
                    my += client_height(c) as u32;
                }
            }
        } else {
            let h = (m.work_rect.h - ty as i32) / (n - i) as i32;
            animate_client_rect(
                win,
                &Rect {
                    x: m.work_rect.x + mw,
                    y: m.work_rect.y + ty as i32,
                    w: m.work_rect.w - mw - 2 * border_width,
                    h: h - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if ty as i32 + client_height(c) < m.work_rect.h {
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

    if g.animated && !g.monitors.is_empty() {
        if let Some(mon) = g.monitors.get(g.selmon) {
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

    let mut c_win = m.clients;
    while let Some(win) = c_win {
        let g = get_globals();
        if let Some(c) = g.clients.get(&win) {
            if c.is_visible() {
                n += 1;
            }
            c_win = c.next;
        } else {
            break;
        }
    }

    let g = get_globals();
    let animated = g.animated;
    let sel_win = g.monitors.get(g.selmon).and_then(|mon| mon.sel);

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
        animate_client_rect(
            win,
            &Rect {
                x: m.work_rect.x,
                y: m.work_rect.y,
                w: m.work_rect.w - 2 * border_width,
                h: m.work_rect.h - 2 * border_width,
            },
            frames,
            0,
        );

        c_win = next_tiled(next_client);
    }
}

pub fn grid(m: &mut MonitorInner) {
    let g = get_globals();
    if m.clientcount <= 2 && m.monitor_rect.w > m.monitor_rect.h {
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

    let ch = m.work_rect.h / if rows > 0 { rows } else { 1 };
    let cw = m.work_rect.w / if cols > 0 { cols as i32 } else { 1 };

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

        let cx = m.work_rect.x + (i / rows) * cw;
        let cy = m.work_rect.y + (i % rows) * ch;

        let ah = if (i + 1) % rows == 0 {
            m.work_rect.h - ch * rows
        } else {
            0
        };
        let aw = if i >= rows * (cols as i32 - 1) {
            m.work_rect.w - cw * cols as i32
        } else {
            0
        };

        animate_client_rect(
            win,
            &Rect {
                x: cx,
                y: cy,
                w: cw - 2 * border_width + aw,
                h: ch - 2 * border_width + ah,
            },
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

    let mw: u32 = if n > m.nmaster as u32 {
        if m.nmaster > 0 {
            (m.mfact * m.work_rect.w as f32) as u32
        } else {
            0
        }
    } else {
        m.work_rect.w as u32
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
            let h = (m.work_rect.h - my as i32) / (min(n, m.nmaster as u32) - i) as i32;
            resize(
                win,
                &Rect {
                    x: m.work_rect.x,
                    y: m.work_rect.y + my as i32,
                    w: mw as i32 - 2 * border_width,
                    h: h - 2 * border_width,
                },
                false,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                my += client_height(c) as u32;
            }
        } else {
            resize(
                win,
                &Rect {
                    x: m.work_rect.x + mw as i32,
                    y: m.work_rect.y,
                    w: m.work_rect.w - mw as i32 - 2 * border_width,
                    h: m.work_rect.h - 2 * border_width,
                },
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
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        tw = m.work_rect.w / (n - m.nmaster as u32) as i32;
        ty = m.work_rect.y + mh;
    } else {
        mh = m.work_rect.h;
        tw = m.work_rect.w;
        ty = m.work_rect.y;
    }

    let mut mx: i32 = 0;
    let mut tx: i32 = m.work_rect.x;
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
            let w = (m.work_rect.w - mx) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client_rect(
                win,
                &Rect {
                    x: m.work_rect.x + mx,
                    y: m.work_rect.y,
                    w: w - 2 * border_width,
                    h: mh - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                mx += client_width(c);
            }
        } else {
            let h = m.work_rect.h - mh;
            animate_client_rect(
                win,
                &Rect {
                    x: tx,
                    y: ty,
                    w: tw - 2 * border_width,
                    h: h - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if tw != m.work_rect.w {
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

    let (mh, th, mut ty) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let th = (m.work_rect.h - mh) / (n - m.nmaster as u32) as i32;
        let ty = m.work_rect.y + mh;
        (mh, th, ty)
    } else {
        (m.work_rect.h, m.work_rect.h, m.work_rect.y)
    };

    let mut mx: i32 = 0;
    let tx: i32 = m.work_rect.x;
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
            let w = (m.work_rect.w - mx) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client_rect(
                win,
                &Rect {
                    x: m.work_rect.x + mx,
                    y: m.work_rect.y,
                    w: w - 2 * border_width,
                    h: mh - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                mx += client_width(c);
            }
        } else {
            animate_client_rect(
                win,
                &Rect {
                    x: tx,
                    y: ty,
                    w: m.work_rect.w - 2 * border_width,
                    h: th - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if th != m.work_rect.h {
                    let mut new_ty = ty;
                    new_ty += client_height(c);
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

    let mw = (m.mfact * m.work_rect.w as f32) as i32;
    let sw = (m.work_rect.w - mw) / 2;

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
        &Rect {
            x: if n < 3 {
                m.work_rect.x
            } else {
                m.work_rect.x + sw
            },
            y: m.work_rect.y,
            w: if n == 1 {
                m.work_rect.w - bdw
            } else {
                mw - bdw
            },
            h: m.work_rect.h - bdw,
        },
        false,
    );

    if n <= 1 {
        return;
    }
    let n = n - 1;

    let w = (m.work_rect.w - mw) / if n > 1 { 2 } else { 1 };

    let mut c_win = next_tiled(first_next);
    if n > 1 {
        let x = m.work_rect.x + mw + sw;
        let mut y = m.work_rect.y;
        let h = m.work_rect.h / (n / 2) as i32;

        let bh = {
            let g = get_globals();
            g.bh
        };

        let actual_h = if h < bh { m.work_rect.h } else { h };

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
                m.work_rect.y + m.work_rect.h - y - 2 * border_width
            } else {
                actual_h - 2 * border_width
            };
            resize(
                win,
                &Rect {
                    x,
                    y,
                    w: w - 2 * border_width,
                    h: rh,
                },
                false,
            );

            if actual_h != m.work_rect.h {
                let g = get_globals();
                if let Some(c) = g.clients.get(&win) {
                    y = c.geo.y + client_height(c);
                }
            }

            i += 1;
            c_win = next_tiled(next_client);
        }
    }

    let x = if (n + 1) / 2 == 1 {
        mw + m.work_rect.x
    } else {
        m.work_rect.x
    };
    let mut y = m.work_rect.y;
    let h = m.work_rect.h / ((n + 1) / 2) as i32;

    let bh = {
        let g = get_globals();
        g.bh
    };

    let actual_h = if h < bh { m.work_rect.h } else { h };

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
            m.work_rect.y + m.work_rect.h - y - 2 * border_width
        } else {
            actual_h - 2 * border_width
        };
        resize(
            win,
            &Rect {
                x,
                y,
                w: w - 2 * border_width,
                h: rh,
            },
            false,
        );

        if actual_h != m.work_rect.h {
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                y = c.geo.y + client_height(c);
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
        if g.monitors.is_empty() {
            return;
        }
        if let Some(mon) = g.monitors.get(g.selmon) {
            (
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.work_rect.h,
                mon.work_rect.w,
                mon.showbar,
                mon.barwin,
            )
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

                let (cw, ch, is_floating) = (c.geo.w, c.geo.h, c.isfloating);
                let next_client = c.next;

                if is_floating {
                    save_floating(win);
                }

                resize(
                    win,
                    &Rect {
                        x: tmpx,
                        y: tmpy,
                        w: cw,
                        h: ch,
                    },
                    false,
                );

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

    {
        let g = get_globals_mut();
        g.animated = false;
    }

    let mut c_win = m.clients;
    while let Some(win) = c_win {
        let g = get_globals();
        if let Some(c) = g.clients.get(&win) {
            if !c.is_visible() {
                c_win = c.next;
                continue;
            }

            let snapstatus = c.snapstatus;
            let next_client = c.next;

            if snapstatus != SnapPosition::None {
                apply_snap_for_window(win, m);
            }

            c_win = next_client;
        } else {
            break;
        }
    }

    if !get_globals().monitors.is_empty() {
        let g = get_globals_mut();
        if let Some(mon) = g.monitors.get_mut(g.selmon) {
            restack(mon);
        }
    }

    {
        let g = get_globals();
        if !g.monitors.is_empty() {
            if let Some(mon) = g.monitors.get(g.selmon) {
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
        let g = get_globals_mut();
        g.animated = true;
    }
}

fn apply_snap_for_window(win: Window, m: &MonitorInner) {
    let g = get_globals();
    if let Some(c) = g.clients.get(&win) {
        let snapstatus = c.snapstatus;
        let border_width = c.border_width;

        let (x, y, w, h) = match snapstatus {
            SnapPosition::Top => (
                m.work_rect.x,
                m.work_rect.y,
                m.work_rect.w - 2 * border_width,
                m.work_rect.h / 2 - 2 * border_width,
            ),
            SnapPosition::TopRight => (
                m.work_rect.x + m.work_rect.w / 2,
                m.work_rect.y,
                m.work_rect.w / 2 - 2 * border_width,
                m.work_rect.h / 2 - 2 * border_width,
            ),
            SnapPosition::Right => (
                m.work_rect.x + m.work_rect.w / 2,
                m.work_rect.y,
                m.work_rect.w / 2 - 2 * border_width,
                m.work_rect.h - 2 * border_width,
            ),
            SnapPosition::BottomRight => (
                m.work_rect.x + m.work_rect.w / 2,
                m.work_rect.y + m.work_rect.h / 2,
                m.work_rect.w / 2 - 2 * border_width,
                m.work_rect.h / 2 - 2 * border_width,
            ),
            SnapPosition::Bottom => (
                m.work_rect.x,
                m.work_rect.y + m.work_rect.h / 2,
                m.work_rect.w - 2 * border_width,
                m.work_rect.h / 2 - 2 * border_width,
            ),
            SnapPosition::BottomLeft => (
                m.work_rect.x,
                m.work_rect.y + m.work_rect.h / 2,
                m.work_rect.w / 2 - 2 * border_width,
                m.work_rect.h / 2 - 2 * border_width,
            ),
            SnapPosition::Left => (
                m.work_rect.x,
                m.work_rect.y,
                m.work_rect.w / 2 - 2 * border_width,
                m.work_rect.h - 2 * border_width,
            ),
            SnapPosition::TopLeft => (
                m.work_rect.x,
                m.work_rect.y,
                m.work_rect.w / 2 - 2 * border_width,
                m.work_rect.h / 2 - 2 * border_width,
            ),
            SnapPosition::Maximized => (
                m.work_rect.x,
                m.work_rect.y,
                m.work_rect.w - 2 * border_width,
                m.work_rect.h - 2 * border_width,
            ),
            SnapPosition::None => return,
        };

        resize(win, &Rect { x, y, w, h }, false);
    }
}

fn save_floating(win: Window) {
    let g = get_globals_mut();
    if let Some(c) = g.clients.get_mut(&win) {
        c.float_geo = c.geo;
    }
}

pub fn arrange(mon_id: Option<MonitorId>) {
    reset_cursor();

    if let Some(id) = mon_id {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(id) {
            let stack = m.stack;
            show_hide(stack);
        }
        let g = get_globals_mut();
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

        let g = get_globals_mut();
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

                if let Some(mid) = mon_id {
                    if let Some(mon) = g.monitors.get(mid) {
                        let clientcount = mon.clientcount;
                        let layout_idx = get_current_layout_idx(mon).unwrap_or(0);
                        let has_arrange = g.layouts.get(layout_idx).map_or(true, |l| l.is_tiling());
                        let is_monocle =
                            g.layouts.get(layout_idx).map_or(false, |l| l.is_monocle());
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

        {
            let g = get_globals_mut();
            if let Some(_c) = g.clients.get_mut(&win) {
                if !is_floating
                    && !is_fullscreen
                    && ((mon_clientcount == 1 && has_arrange) || is_monocle)
                {
                    save_border_width(win);
                    let g = get_globals_mut();
                    if let Some(c) = g.clients.get_mut(&win) {
                        c.border_width = 0;
                    }
                } else {
                    restore_border_width_win(win);
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

    let layout = get_current_layout(m);
    layout.arrange(m);

    let g = get_globals_mut();
    let showbar = crate::monitor::get_current_showbar(m, &g.tags);
    if let Some(ref fullscreen_win) = m.overlay {
        if let Some(c) = g.clients.get_mut(fullscreen_win) {
            let tbw = c.border_width;
            if c.isfloating {
                save_floating(*fullscreen_win);
            }
            let showbar_offset = if showbar { g.bh } else { 0 };
            resize(
                *fullscreen_win,
                &Rect {
                    x: m.monitor_rect.x,
                    y: m.monitor_rect.y + showbar_offset,
                    w: m.monitor_rect.w - 2 * tbw,
                    h: m.monitor_rect.h - showbar_offset - 2 * tbw,
                },
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

    let has_arrange = get_current_layout(m).is_tiling();

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let g = get_globals();
        let is_floating = g
            .clients
            .get(&sel_win)
            .map(|c| c.isfloating)
            .unwrap_or(false);

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
                    let visible = c.is_visible();
                    let snext = c.snext;

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
    }
}

fn is_overview_layout(m: &MonitorInner) -> bool {
    get_current_layout(m).is_overview()
}

pub fn set_layout(arg: &Arg) {
    let mut multimon = false;
    let tagprefix = {
        let g = get_globals();
        g.tags.prefix
    };

    if tagprefix {
        // When tagprefix is set, apply layout changes to ALL tags on ALL monitors
        let layout_idx = arg.v;
        {
            let g = get_globals_mut();

            // Update all tags
            for tag in g.tags.tags.iter_mut() {
                if arg.v.is_none() {
                    tag.sellt ^= 1;
                }
                if let Some(idx) = layout_idx {
                    tag.ltidxs[tag.sellt as usize] = Some(idx);
                }
            }

            g.tags.prefix = false;
        }
        // Recursively call set_layout to arrange the current monitor
        set_layout(arg);
        return;
    } else {
        let layout_idx = arg.v;
        {
            let g = get_globals_mut();
            if !g.monitors.is_empty() {
                if let Some(m) = g.monitors.get_mut(g.selmon) {
                    let current_tag = m.current_tag;

                    if current_tag > 0 && current_tag <= g.tags.tags.len() {
                        let tag = &mut g.tags.tags[current_tag - 1];

                        if arg.v.is_none() || layout_idx != get_current_layout_idx(m) {
                            tag.sellt ^= 1;
                        }
                        if let Some(idx) = layout_idx {
                            tag.ltidxs[tag.sellt as usize] = Some(idx);
                        }
                    }
                }
            }
        }

        let selmon_sel = {
            let g = get_globals();
            g.monitors.get(g.selmon).and_then(|m| m.sel)
        };

        if selmon_sel.is_some() {
            let g = get_globals();
            arrange(Some(g.selmon));
        } else {
            let g = get_globals_mut();
            if let Some(m) = g.monitors.get_mut(g.selmon) {
                draw_bar(m);
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
            if mon_id != g.selmon {
                let g = get_globals_mut();
                g.selmon = mon_id;
                set_layout(arg);
            }
        }

        let g = get_globals_mut();
        g.selmon = tmpmon;
        crate::focus::focus(None);
    }
}

fn get_current_layout_idx(m: &MonitorInner) -> Option<usize> {
    let g = get_globals();
    let current_tag = m.current_tag;
    if current_tag > 0 && current_tag <= g.tags.tags.len() {
        let tag = &g.tags.tags[current_tag - 1];
        tag.ltidxs[tag.sellt as usize]
    } else {
        None
    }
}

fn get_current_layout_symbol() -> Option<&'static str> {
    let g = get_globals();
    if !g.monitors.is_empty() {
        if let Some(m) = g.monitors.get(g.selmon) {
            let idx = get_current_layout_idx(m);
            if let Some(i) = idx {
                if i < g.layouts.len() {
                    return Some(g.layouts[i].symbol());
                }
            }
        }
    }
    g.layouts.first().map(|l| l.symbol())
}

/// Cycle to the next or previous layout.
///
/// # Arguments
/// * `forward` - If true, cycle forward; if false, cycle backward.
pub fn cycle_layout_direction(forward: bool) {
    let current_idx = {
        let g = get_globals();
        if g.monitors.is_empty() {
            None
        } else if let Some(m) = g.monitors.get(g.selmon) {
            get_current_layout_idx(m)
        } else {
            None
        }
    };

    let g = get_globals();
    let layouts_len = g.layouts.len();

    if layouts_len == 0 {
        return;
    }

    let current = current_idx.unwrap_or(0);

    let new_idx = if forward {
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
        g.layouts.get(new_idx).map_or(false, |l| l.is_overview())
    };

    let final_idx = if skip_overview {
        if forward {
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

    set_layout_by_index(final_idx);
}

/// Set the layout by its index in the layout list.
pub fn set_layout_by_index(layout_idx: usize) {
    set_layout(&Arg {
        v: Some(layout_idx),
        ..Default::default()
    });
}

/// Legacy wrapper for key bindings. Use `cycle_layout_direction` for new code.
pub fn cycle_layout(arg: &Arg) {
    cycle_layout_direction(arg.i > 0);
}

/// Adjust the number of master clients in the layout.
///
/// # Arguments
/// * `delta` - The amount to add (positive) or subtract (negative) from nmaster.
///             Use `inc_nmaster_by(1)` to increase, `inc_nmaster_by(-1)` to decrease.
pub fn inc_nmaster_by(delta: i32) {
    let ccount = client_count();

    {
        let g = get_globals_mut();
        if !g.monitors.is_empty() {
            if let Some(m) = g.monitors.get_mut(g.selmon) {
                if delta > 0 && m.nmaster >= ccount as i32 {
                    m.nmaster = ccount as i32;
                    return;
                }

                let new_nmaster = max(m.nmaster + delta, 0);
                m.nmaster = new_nmaster;

                let current_tag = m.current_tag;
                if current_tag > 0 && current_tag <= g.tags.tags.len() {
                    g.tags.tags[current_tag - 1].nmaster = new_nmaster;
                }
            }
        }
    }

    let g = get_globals();
    if !g.monitors.is_empty() {
        arrange(Some(g.selmon));
    }
}

/// Legacy wrapper for key bindings. Use `inc_nmaster_by` for new code.
pub fn inc_nmaster(arg: &Arg) {
    inc_nmaster_by(arg.i);
}

pub fn set_mfact(arg: &Arg) {
    if arg.f == 0.0 {
        return;
    }

    let has_arrange = {
        let g = get_globals();
        if g.monitors.is_empty() {
            false
        } else if let Some(m) = g.monitors.get(g.selmon) {
            let idx = get_current_layout_idx(m).unwrap_or(0);
            g.layouts.get(idx).map_or(true, |l| l.is_tiling())
        } else {
            false
        }
    };

    if !has_arrange {
        return;
    }

    let g = get_globals();
    let current_mfact = if g.monitors.is_empty() {
        0.55
    } else if let Some(m) = g.monitors.get(g.selmon) {
        m.mfact
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

    let animated = {
        let g = get_globals();
        g.animated && client_count() > 2
    };

    if animated {
        let g = get_globals_mut();
        g.animated = false;
    }

    {
        let g = get_globals_mut();
        if !g.monitors.is_empty() {
            if let Some(m) = g.monitors.get_mut(g.selmon) {
                m.mfact = f;
                let current_tag = m.current_tag;
                if current_tag > 0 && current_tag <= g.tags.tags.len() {
                    g.tags.tags[current_tag - 1].mfact = f;
                }
            }
        }
    }

    let g = get_globals();
    if !g.monitors.is_empty() {
        arrange(Some(g.selmon));
    }

    if animated {
        let g = get_globals_mut();
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
    crate::mouse::reset_cursor();
}

fn show_hide(win: Option<Window>) {
    crate::client::show_hide(win);
}

pub fn client_count() -> i32 {
    let g = get_globals();
    let mut count = 0;

    if !g.monitors.is_empty() {
        if let Some(mon) = g.monitors.get(g.selmon) {
            let mut c_win = mon.clients;
            while let Some(win) = c_win {
                if let Some(c) = g.clients.get(&win) {
                    if c.is_visible() && !c.isfloating {
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
            if c.is_visible() && !c.isfloating {
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

pub fn find_visible_client(start_win: Option<Window>) -> Option<Window> {
    let mut current = start_win;
    let g = get_globals();

    while let Some(win) = current {
        if let Some(c) = g.clients.get(&win) {
            if c.is_visible() {
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

    let mut x = m.work_rect.x;
    let mut y = m.work_rect.y;
    let mut w = m.work_rect.w;
    let mut h = m.work_rect.h;

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

        resize(
            win,
            &Rect {
                x,
                y,
                w: w - 2 * border_width,
                h: h - 2 * border_width,
            },
            false,
        );

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
        let cw = m.work_rect.w / cols as i32;

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

                let ch = m.work_rect.h / cn as i32;
                let cx = m.work_rect.x + col as i32 * cw;
                let cy = m.work_rect.y + row as i32 * ch;

                let aw = if col == cols - 1 {
                    m.work_rect.w - cols as i32 * cw + cw
                } else {
                    0
                };

                animate_client_rect(
                    win,
                    &Rect {
                        x: cx,
                        y: cy,
                        w: cw - 2 * border_width + aw,
                        h: ch - 2 * border_width,
                    },
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
