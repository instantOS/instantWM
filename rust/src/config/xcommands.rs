//! IPC / socket command dispatch table (`instantwmctl` commands).
//!
//! These are the commands handled when another process sends a message to the
//! running WM via the control socket (e.g. `instantwmctl tag 3`).
//!
//! # `cmd_type` values
//!
//! The `cmd_type` field controls how the argument passed over the socket is
//! interpreted before the handler function is called:
//!
//! | Value | Meaning                                        |
//! |-------|------------------------------------------------|
//! | 0     | No argument — function called with stored arg  |
//! | 1     | Toggle: 0 = off, 1 = on, 2 = toggle           |
//! | 3     | Unsigned integer argument from socket          |
//! | 4     | String argument from socket (e.g. scratchpad name) |
//! | 5     | Integer argument from socket                   |

use crate::commands::{command_prefix, set_special_next};
use crate::focus::warp_to_focus;
use crate::keyboard::focus_nmon;
use crate::layouts::command_layout;
use crate::monitor::{focus_mon, follow_mon};
use crate::overlay::set_overlay;
use crate::scratchpad::{
    scratchpad_hide, scratchpad_make, scratchpad_show, scratchpad_status, scratchpad_toggle,
    scratchpad_unmake,
};
use crate::tags::{name_tag, reset_name_tag, tag_mon, view};
use crate::toggles::{
    alt_tab_free, set_border_width, toggle_alt_tag, toggle_animated,
    toggle_focus_follows_float_mouse, toggle_focus_follows_mouse, toggle_show_tags,
};
use crate::types::{Arg, XCommand};

use super::commands::Cmd;
use super::mod_consts::BORDERPX;

/// Build the IPC command dispatch table.
pub fn get_xcommands() -> Vec<XCommand> {
    vec![
        // --- Overlay ---
        xc("overlay", set_overlay, Arg::default(), 0),
        // --- Focus ---
        xc("warpfocus", warp_to_focus, Arg::default(), 0),
        // --- Tag control ---
        xc(
            "tag",
            view,
            Arg {
                ui: 2,
                ..Default::default()
            },
            3,
        ),
        // --- Toggles ---
        xc(
            "animated",
            toggle_animated,
            Arg {
                ui: 2,
                ..Default::default()
            },
            1,
        ),
        xc(
            "focusfollowsmouse",
            toggle_focus_follows_mouse,
            Arg {
                ui: 2,
                ..Default::default()
            },
            1,
        ),
        xc(
            "focusfollowsfloatmouse",
            toggle_focus_follows_float_mouse,
            Arg {
                ui: 2,
                ..Default::default()
            },
            1,
        ),
        xc(
            "alttab",
            alt_tab_free,
            Arg {
                ui: 2,
                ..Default::default()
            },
            1,
        ),
        xc(
            "alttag",
            toggle_alt_tag,
            Arg {
                ui: 0,
                ..Default::default()
            },
            1,
        ),
        xc(
            "hidetags",
            toggle_show_tags,
            Arg {
                ui: 0,
                ..Default::default()
            },
            1,
        ),
        xc(
            "layout",
            command_layout,
            Arg {
                ui: 0,
                ..Default::default()
            },
            1,
        ),
        xc(
            "prefix",
            command_prefix,
            Arg {
                ui: 1,
                ..Default::default()
            },
            1,
        ),
        // --- Border width (integer arg from socket) ---
        xc(
            "border",
            set_border_width,
            Arg {
                i: BORDERPX,
                ..Default::default()
            },
            5,
        ),
        // --- Special next window ---
        xc(
            "specialnext",
            set_special_next,
            Arg {
                ui: 0,
                ..Default::default()
            },
            3,
        ),
        // --- Monitor commands ---
        xc(
            "tagmon",
            tag_mon,
            Arg {
                i: 1,
                ..Default::default()
            },
            0,
        ),
        xc(
            "followmon",
            follow_mon,
            Arg {
                i: 1,
                ..Default::default()
            },
            0,
        ),
        xc(
            "focusmon",
            focus_mon,
            Arg {
                i: 1,
                ..Default::default()
            },
            0,
        ),
        xc(
            "focusnmon",
            focus_nmon,
            Arg {
                i: 0,
                ..Default::default()
            },
            5,
        ),
        // --- Tag naming ---
        xc(
            "nametag",
            name_tag,
            Arg {
                v: Some(Cmd::Tag as usize),
                ..Default::default()
            },
            4,
        ),
        xc("resetnametag", reset_name_tag, Arg::default(), 0),
        // --- Scratchpad ---
        xc("scratchpad-make", scratchpad_make, Arg::default(), 4),
        xc("scratchpad-unmake", scratchpad_unmake, Arg::default(), 0),
        xc("scratchpad-toggle", scratchpad_toggle, Arg::default(), 4),
        xc("scratchpad-show", scratchpad_show, Arg::default(), 4),
        xc("scratchpad-hide", scratchpad_hide, Arg::default(), 4),
        xc("scratchpad-status", scratchpad_status, Arg::default(), 4),
    ]
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Shorthand constructor for [`XCommand`].
#[inline]
fn xc(cmd: &'static str, func: fn(&Arg), arg: Arg, cmd_type: u32) -> XCommand {
    XCommand {
        cmd,
        func: Some(func),
        arg,
        cmd_type,
    }
}
