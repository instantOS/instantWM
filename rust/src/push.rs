use crate::client::list::{attach, detach};
use crate::client::next_tiled;
use crate::contexts::WmCtx;
use crate::focus::focus;
use crate::layouts::arrange;
pub use crate::layouts::query::client_count;
use x11rb::protocol::xproto::Window;

pub fn next_c(ctx: &WmCtx, c_win: Option<Window>, include_floating: bool) -> Option<Window> {
    if !include_floating {
        return next_tiled(c_win);
    }

    let mut current = c_win;
    let selected = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| m.selected_tags())
        .unwrap_or(0);

    while let Some(win) = current {
        if let Some(c) = ctx.g.clients.get(&win) {
            if c.is_visible_on_tags(selected) {
                return Some(win);
            }
            current = c.next;
        } else {
            break;
        }
    }
    None
}

pub fn prev_c(ctx: &WmCtx, c_win: Window, include_floating: bool) -> Option<Window> {
    if ctx.g.monitors.is_empty() {
        return None;
    }

    let mon = ctx.g.monitors.get(ctx.g.selmon)?;
    let selected = mon.selected_tags();

    let mut p: Option<Window> = None;
    let mut r: Option<Window> = None;

    let mut current = mon.clients;
    while let Some(win) = current {
        if win == c_win {
            break;
        }

        if let Some(c) = ctx.g.clients.get(&win) {
            if (include_floating || !c.isfloating) && c.is_visible_on_tags(selected) {
                r = Some(win);
            }
            p = Some(win);
            current = c.next;
        } else {
            break;
        }
    }

    r
}

pub fn push_up(ctx: &mut WmCtx, win: Window) {
    if client_count(ctx.g) < 2 {
        return;
    }

    let is_floating = ctx
        .g
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let include_floating = true;

    let selmon_id = ctx.g.selmon;

    if let Some(prev) = prev_c(ctx, win, include_floating) {
        detach(win);

        {
            let clients = &mut ctx.g.clients;
            let monitors = &mut ctx.g.monitors;
            if let Some(client) = clients.get_mut(&win) {
                client.next = Some(prev);
            }

            if let Some(mon) = monitors.get_mut(selmon_id) {
                if mon.clients == Some(prev) {
                    mon.clients = Some(win);
                } else {
                    let target_c_win = mon.iter_clients(clients).find_map(|(c_win, c)| {
                        if c.next == Some(prev) {
                            Some(c_win)
                        } else {
                            None
                        }
                    });
                    if let Some(t_win) = target_c_win {
                        if let Some(c) = clients.get_mut(&t_win) {
                            c.next = Some(win);
                        }
                    }
                }
            }
        }
    } else {
        let mut last: Option<Window> = None;
        if let Some(mon) = ctx.g.monitors.get(selmon_id) {
            for (c_win, _c) in mon.iter_clients(&ctx.g.clients) {
                last = Some(c_win);
            }
        }

        detach(win);

        if let Some(last_win) = last {
            if let Some(client) = ctx.g.clients.get_mut(&last_win) {
                client.next = Some(win);
            }
            if let Some(client) = ctx.g.clients.get_mut(&win) {
                client.next = None;
            }
        }
    }

    focus(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}

pub fn push_down(ctx: &mut WmCtx, win: Window) {
    if client_count(ctx.g) < 2 {
        return;
    }

    let is_floating = ctx
        .g
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let include_floating = true;

    let selmon_id = ctx.g.selmon;

    let next = ctx
        .g
        .clients
        .get(&win)
        .and_then(|c| next_c(ctx, c.next, include_floating));

    if let Some(next_win) = next {
        detach(win);

        let next_c_next = ctx.g.clients.get(&next_win).and_then(|c| c.next);
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.next = next_c_next;
        }

        if let Some(next_c) = ctx.g.clients.get_mut(&next_win) {
            next_c.next = Some(win);
        }
    } else {
        detach(win);
        attach(win);
    }

    focus(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}
