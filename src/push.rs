use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module
use crate::layouts::arrange;
use crate::types::WindowId;

pub enum Direction {
    Up,
    Down,
}

pub fn push(ctx: &mut WmCtx, win: WindowId, direction: Direction) {
    let tiled_count = {
        let g = ctx.core_mut().globals_mut();
        g.selected_monitor().tiled_client_count(g.clients.map())
    };
    if tiled_count < 2 {
        return;
    }

    let is_floating = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| c.is_floating)
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();

    if let Some(mon) = ctx.core_mut().globals_mut().monitors.get_mut(selmon_id)
        && let Some(pos) = mon.clients.iter().position(|&w| w == win)
    {
        match direction {
            Direction::Up => {
                if pos > 0 {
                    mon.clients.swap(pos, pos - 1);
                } else {
                    let last = mon.clients.pop();
                    if let Some(last_win) = last {
                        mon.clients.insert(1, last_win);
                    }
                }
            }
            Direction::Down => {
                if pos + 1 < mon.clients.len() {
                    mon.clients.swap(pos, pos + 1);
                } else {
                    let first = mon.clients.remove(0);
                    mon.clients.push(first);
                }
            }
        }
    }

    crate::focus::focus_soft(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}
