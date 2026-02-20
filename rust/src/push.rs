use crate::client::{is_visible, next_tiled, resize};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::monitor::arrange;
use crate::types::*;
use x11rb::protocol::xproto::Window;

pub fn next_c(c_win: Option<Window>, include_floating: bool) -> Option<Window> {
    if !include_floating {
        return next_tiled(c_win);
    }

    let mut current = c_win;
    let globals = get_globals();

    while let Some(win) = current {
        if let Some(c) = globals.clients.get(&win) {
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

pub fn prev_c(c_win: Window, include_floating: bool) -> Option<Window> {
    let globals = get_globals();

    let selmon_id = match globals.selmon {
        Some(id) => id,
        None => return None,
    };

    let mon = match globals.monitors.get(selmon_id) {
        Some(m) => m,
        None => return None,
    };

    let mut p: Option<Window> = None;
    let mut r: Option<Window> = None;

    let mut current = mon.clients;
    while let Some(win) = current {
        if win == c_win {
            break;
        }

        if let Some(c) = globals.clients.get(&win) {
            if (include_floating || !c.isfloating) && is_visible(c) {
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

pub fn client_count_mon(mon: &MonitorInner) -> i32 {
    let globals = get_globals();
    let mut n = 0;
    let mut current = next_tiled(mon.clients);

    while let Some(win) = current {
        n += 1;
        if let Some(c) = globals.clients.get(&win) {
            current = next_tiled(c.next);
        } else {
            break;
        }
    }

    n
}

pub fn client_count() -> i32 {
    let globals = get_globals();
    if let Some(selmon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(selmon_id) {
            return client_count_mon(mon);
        }
    }
    0
}

pub fn all_client_count() -> i32 {
    let globals = get_globals();
    let selmon_id = match globals.selmon {
        Some(id) => id,
        None => return 0,
    };

    let mon = match globals.monitors.get(selmon_id) {
        Some(m) => m,
        None => return 0,
    };

    let mut n = 0;
    let mut current = mon.clients;

    while let Some(win) = current {
        if let Some(c) = globals.clients.get(&win) {
            if Some(win) != mon.overlay {
                n += 1;
            }
            current = c.next;
        } else {
            break;
        }
    }

    n
}

pub fn client_distance(c1: &ClientInner, c2: &ClientInner) -> i32 {
    let x = ((c1.x + c1.w) / 2 - (c2.x + c2.w) / 2).abs();
    let y = ((c1.y + c1.h) / 2 - (c2.y + c2.h) / 2).abs();

    ((y * y + x * x) as f64).sqrt() as i32
}

pub fn push_up(arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    if client_count() < 2 {
        return;
    }

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if is_floating && !arg.f.is_sign_positive() {
        return;
    }

    let include_floating = arg.f.is_sign_positive();

    let selmon_id = {
        let globals = get_globals();
        globals.selmon.unwrap_or(0)
    };

    if let Some(prev) = prev_c(win, include_floating) {
        detach(win);

        {
            let mut globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(selmon_id) {
                if let Some(client) = globals.clients.get_mut(&win) {
                    client.next = Some(prev);
                }

                if mon.clients == Some(prev) {
                    mon.clients = Some(win);
                } else {
                    let mut current = mon.clients;
                    while let Some(c_win) = current {
                        if let Some(c) = globals.clients.get(&c_win) {
                            if c.next == Some(prev) {
                                if let Some(c) = globals.clients.get_mut(&c_win) {
                                    c.next = Some(win);
                                }
                                break;
                            }
                            current = c.next;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    } else {
        let mut last: Option<Window> = None;
        {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(selmon_id) {
                let mut current = mon.clients;
                while let Some(c_win) = current {
                    if let Some(c) = globals.clients.get(&c_win) {
                        last = Some(c_win);
                        current = c.next;
                    } else {
                        break;
                    }
                }
            }
        }

        detach(win);

        if let Some(last_win) = last {
            let mut globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&last_win) {
                client.next = Some(win);
            }
            if let Some(client) = globals.clients.get_mut(&win) {
                client.next = None;
            }
        }
    }

    focus(Some(win));
    arrange(Some(selmon_id));
}

pub fn push_down(arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    if client_count() < 2 {
        return;
    }

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if is_floating && !arg.f.is_sign_positive() {
        return;
    }

    let include_floating = arg.f.is_sign_positive();

    let selmon_id = {
        let globals = get_globals();
        globals.selmon.unwrap_or(0)
    };

    let next = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            next_c(c.next, include_floating)
        } else {
            None
        }
    };

    if let Some(next_win) = next {
        detach(win);

        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            if let Some(next_c) = globals.clients.get(&next_win) {
                client.next = next_c.next;
            } else {
                client.next = None;
            }
        }

        if let Some(next_c) = globals.clients.get_mut(&next_win) {
            next_c.next = Some(win);
        }
    } else {
        detach(win);
        attach(win);
    }

    focus(Some(win));
    arrange(Some(selmon_id));
}

fn attach(win: Window) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get(&win) {
        let mon_id = match client.mon_id {
            Some(id) => id,
            None => return,
        };

        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            if let Some(client) = globals.clients.get_mut(&win) {
                client.next = mon.clients;
            }
            mon.clients = Some(win);
        }
    }
}

fn detach(win: Window) {
    let mut globals = get_globals_mut();

    let mon_id = {
        if let Some(client) = globals.clients.get(&win) {
            client.mon_id
        } else {
            return;
        }
    };

    if let Some(mid) = mon_id {
        if let Some(mon) = globals.monitors.get_mut(mid) {
            let mut current = mon.clients;
            let mut prev: Option<Window> = None;

            while let Some(cur_win) = current {
                if cur_win == win {
                    if let Some(prev_win) = prev {
                        if let Some(prev_client) = globals.clients.get_mut(&prev_win) {
                            prev_client.next = if let Some(c) = globals.clients.get(&win) {
                                c.next
                            } else {
                                None
                            };
                        }
                    } else {
                        mon.clients = if let Some(c) = globals.clients.get(&win) {
                            c.next
                        } else {
                            None
                        };
                    }
                    return;
                }
                prev = Some(cur_win);
                current = if let Some(c) = globals.clients.get(&cur_win) {
                    c.next
                } else {
                    None
                };
            }
        }
    }
}
