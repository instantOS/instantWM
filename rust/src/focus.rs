use crate::client::set_focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::types::*;
use x11rb::protocol::xproto::Window;

pub fn focus(win: Option<Window>) {
    let globals = get_globals();

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            let current_sel = mon.sel;

            drop(globals);

            if win == current_sel {
                return;
            }

            if let Some(cur_win) = current_sel {
                crate::client::unfocus_win(cur_win, true);
            }
        }
    }

    let mut globals = get_globals_mut();

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.sel = win;
        }
    }

    if let Some(w) = win {
        if let Some(_client) = globals.clients.get(&w) {
            drop(globals);
            set_focus(w);
        }
    }
}

pub fn unfocus(_win: Window, _set_focus: bool) {}

pub fn set_focus_win(_win: Window) {}

pub fn direction_focus(_arg: &Arg) {}
