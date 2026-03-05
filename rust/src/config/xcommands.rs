//! IPC / socket command dispatch table (`instantwmctl` commands).

use crate::commands::{command_prefix, set_special_next};
use crate::focus::warp_to_focus_x11;
use crate::layouts::command_layout;
use crate::monitor::{focus_mon, focus_n_mon, follow_mon};
use crate::overlay::set_overlay;
use crate::scratchpad::{
    scratchpad_hide_name, scratchpad_make, scratchpad_show_name, scratchpad_status,
    scratchpad_toggle, scratchpad_unmake,
};
use crate::tags::send_to_monitor;
use crate::tags::{name_tag, reset_name_tag, view};
use crate::toggles::{
    alt_tab_free, set_border_width, toggle_alt_tag, toggle_animated,
    toggle_focus_follows_float_mouse, toggle_focus_follows_mouse, toggle_show_tags,
};
use crate::types::MonitorDirection;
use crate::types::TagMask;
use crate::types::{ToggleAction, XCommand};

use super::mod_consts::BORDERPX;

pub fn get_xcommands() -> Vec<XCommand> {
    vec![
        XCommand {
            cmd: "overlay",
            action: |ctx, _arg| set_overlay(ctx),
        },
        XCommand {
            cmd: "warpfocus",
            action: |ctx, _arg| warp_to_focus_x11(ctx, &ctx.x11),
        },
        XCommand {
            cmd: "tag",
            action: |ctx, arg| {
                let tag_num = if arg.is_empty() {
                    2usize
                } else {
                    arg.parse().unwrap_or(2)
                };
                if let Some(mask) = TagMask::single(tag_num) {
                    view(&mut ctx.core, &ctx.x11, mask);
                }
            },
        },
        XCommand {
            cmd: "animated",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                toggle_animated(ctx, action);
            },
        },
        XCommand {
            cmd: "focusfollowsmouse",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                toggle_focus_follows_mouse(ctx, action);
            },
        },
        XCommand {
            cmd: "focusfollowsfloatmouse",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                toggle_focus_follows_float_mouse(ctx, action);
            },
        },
        XCommand {
            cmd: "alttab",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                alt_tab_free(ctx, &ctx.x11, action);
            },
        },
        XCommand {
            cmd: "alttag",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                toggle_alt_tag(ctx, &ctx.x11, action);
            },
        },
        XCommand {
            cmd: "hidetags",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                toggle_show_tags(ctx, &ctx.x11, action);
            },
        },
        XCommand {
            cmd: "layout",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                command_layout(ctx, val);
            },
        },
        XCommand {
            cmd: "prefix",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    1u32
                } else {
                    arg.parse().unwrap_or(1)
                };
                command_prefix(ctx, &ctx.x11, val);
            },
        },
        XCommand {
            cmd: "border",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    BORDERPX
                } else {
                    arg.parse().unwrap_or(BORDERPX)
                };
                if let Some(win) = ctx.selected_client() {
                    set_border_width(ctx, win, val);
                }
            },
        },
        XCommand {
            cmd: "specialnext",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                set_special_next(ctx, val);
            },
        },
        XCommand {
            cmd: "tagmon",
            action: |ctx, arg| {
                let direction = arg
                    .parse::<i32>()
                    .map(MonitorDirection::from)
                    .unwrap_or(MonitorDirection::NEXT);
                send_to_monitor(&mut ctx.core, &ctx.x11, direction);
            },
        },
        XCommand {
            cmd: "followmon",
            action: |ctx, arg| {
                let direction = arg
                    .parse::<i32>()
                    .map(MonitorDirection::from)
                    .unwrap_or(MonitorDirection::NEXT);
                follow_mon(ctx, direction);
            },
        },
        XCommand {
            cmd: "focusmon",
            action: |ctx, arg| {
                let direction = arg
                    .parse::<i32>()
                    .map(MonitorDirection::from)
                    .unwrap_or(MonitorDirection::NEXT);
                focus_mon(ctx, direction);
            },
        },
        XCommand {
            cmd: "focusnmon",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    0i32
                } else {
                    arg.parse().unwrap_or(0)
                };
                focus_n_mon(ctx, val);
            },
        },
        XCommand {
            cmd: "nametag",
            action: |ctx, arg| name_tag(ctx, &ctx.x11, arg),
        },
        XCommand {
            cmd: "resetnametag",
            action: |ctx, _arg| reset_name_tag(ctx, &ctx.x11),
        },
        XCommand {
            cmd: "scratchpad-make",
            action: |ctx, arg| scratchpad_make(ctx, Some(arg)),
        },
        XCommand {
            cmd: "scratchpad-unmake",
            action: |ctx, _arg| scratchpad_unmake(ctx),
        },
        XCommand {
            cmd: "scratchpad-toggle",
            action: |ctx, arg| scratchpad_toggle(ctx, Some(arg)),
        },
        XCommand {
            cmd: "scratchpad-show",
            action: |ctx, arg| scratchpad_show_name(ctx, arg),
        },
        XCommand {
            cmd: "scratchpad-hide",
            action: |ctx, arg| scratchpad_hide_name(ctx, arg),
        },
        XCommand {
            cmd: "scratchpad-status",
            action: |ctx, arg| scratchpad_status(ctx, arg),
        },
    ]
}
