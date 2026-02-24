//! IPC / socket command dispatch table (`instantwmctl` commands).

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
use crate::types::XCommand;

use super::mod_consts::BORDERPX;

pub fn get_xcommands() -> Vec<XCommand> {
    vec![
        XCommand {
            cmd: "overlay",
            action: |_arg| set_overlay(),
        },
        XCommand {
            cmd: "warpfocus",
            action: |_arg| warp_to_focus(),
        },
        XCommand {
            cmd: "tag",
            action: |arg| {
                let tag_bits = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                view(tag_bits);
            },
        },
        XCommand {
            cmd: "animated",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                toggle_animated(val);
            },
        },
        XCommand {
            cmd: "focusfollowsmouse",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                toggle_focus_follows_mouse(val);
            },
        },
        XCommand {
            cmd: "focusfollowsfloatmouse",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                toggle_focus_follows_float_mouse(val);
            },
        },
        XCommand {
            cmd: "alttab",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                alt_tab_free(val);
            },
        },
        XCommand {
            cmd: "alttag",
            action: |arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                toggle_alt_tag(val);
            },
        },
        XCommand {
            cmd: "hidetags",
            action: |arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                toggle_show_tags(val);
            },
        },
        XCommand {
            cmd: "layout",
            action: |arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                command_layout(val);
            },
        },
        XCommand {
            cmd: "prefix",
            action: |arg| {
                let val = if arg.is_empty() {
                    1u32
                } else {
                    arg.parse().unwrap_or(1)
                };
                command_prefix(val);
            },
        },
        XCommand {
            cmd: "border",
            action: |arg| {
                let val = if arg.is_empty() {
                    BORDERPX
                } else {
                    arg.parse().unwrap_or(BORDERPX)
                };
                set_border_width(val);
            },
        },
        XCommand {
            cmd: "specialnext",
            action: |arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                set_special_next(val);
            },
        },
        XCommand {
            cmd: "tagmon",
            action: |_arg| tag_mon(1),
        },
        XCommand {
            cmd: "followmon",
            action: |_arg| follow_mon(1),
        },
        XCommand {
            cmd: "focusmon",
            action: |_arg| focus_mon(1),
        },
        XCommand {
            cmd: "focusnmon",
            action: |arg| {
                let val = if arg.is_empty() {
                    0i32
                } else {
                    arg.parse().unwrap_or(0)
                };
                focus_nmon(val);
            },
        },
        XCommand {
            cmd: "nametag",
            action: |arg| name_tag(arg),
        },
        XCommand {
            cmd: "resetnametag",
            action: |_arg| reset_name_tag(),
        },
        XCommand {
            cmd: "scratchpad-make",
            action: |arg| scratchpad_make(Some(arg)),
        },
        XCommand {
            cmd: "scratchpad-unmake",
            action: |_arg| scratchpad_unmake(),
        },
        XCommand {
            cmd: "scratchpad-toggle",
            action: |arg| scratchpad_toggle(Some(arg)),
        },
        XCommand {
            cmd: "scratchpad-show",
            action: |arg| scratchpad_show(arg),
        },
        XCommand {
            cmd: "scratchpad-hide",
            action: |arg| scratchpad_hide(arg),
        },
        XCommand {
            cmd: "scratchpad-status",
            action: |arg| scratchpad_status(arg),
        },
    ]
}
