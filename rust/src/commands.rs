use crate::bar::draw_bar;
use crate::contexts::WmCtx;
use crate::types::*;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt;

const COMMAND_INDICATOR: &[u8] = b"c;:;";

pub fn x_command(ctx: &mut WmCtx) -> i32 {
    let conn = ctx.x11.conn;

    let root = ctx.g.cfg.root;

    let Ok(cookie) = conn.get_property::<u32, u8>(
        false,
        root,
        AtomEnum::WM_NAME.into(),
        AtomEnum::STRING.into(),
        0,
        256,
    ) else {
        return 0;
    };

    let Ok(reply) = cookie.reply() else {
        return 0;
    };

    let command_bytes: Vec<u8> = reply.value8().map(|v| v.collect()).unwrap_or_default();
    if command_bytes.len() < COMMAND_INDICATOR.len() {
        return 0;
    }

    if &command_bytes[..COMMAND_INDICATOR.len()] != COMMAND_INDICATOR {
        return 0;
    }

    let fcursor = &command_bytes[COMMAND_INDICATOR.len()..];

    let commands = ctx.g.cfg.commands.clone();

    for cmd in &commands {
        if fcursor.len() < cmd.cmd.len() {
            continue;
        }

        if &fcursor[..cmd.cmd.len()] != cmd.cmd.as_bytes() {
            continue;
        }

        let mut fcursor = &fcursor[cmd.cmd.len()..];

        let arg_str = if fcursor.is_empty() {
            ""
        } else {
            if fcursor[0] != b';' {
                continue;
            }
            fcursor = &fcursor[1..];
            std::str::from_utf8(fcursor).unwrap_or("")
        };

        (cmd.action)(ctx, arg_str);

        return 1;
    }

    0
}

pub fn set_special_next(ctx: &mut WmCtx, value: u32) {
    ctx.g.specialnext = match value {
        0 => SpecialNext::None,
        _ => SpecialNext::Float,
    };
}

pub fn command_prefix(ctx: &mut WmCtx, value: u32) {
    ctx.g.tags.prefix = value != 0;

    let selmon_id = ctx.g.selmon;
    if let Some(mon) = ctx.g.monitors.get_mut(selmon_id) {
        draw_bar(ctx, mon);
    }
}

pub fn init_commands() -> Vec<XCommand> {
    vec![
        XCommand {
            cmd: "layout",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                crate::layouts::command_layout(ctx, val);
            },
        },
        XCommand {
            cmd: "tag",
            action: |ctx, arg| {
                let mask = if arg.is_empty() {
                    TagMask::EMPTY
                } else {
                    let argnum = arg.parse::<usize>().unwrap_or(0);
                    TagMask::single(argnum).unwrap_or(TagMask::EMPTY)
                };
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::tags::set_client_tag(ctx, win, mask);
                }
            },
        },
        XCommand {
            cmd: "view",
            action: |ctx, arg| {
                let mask = if arg.is_empty() {
                    TagMask::EMPTY
                } else {
                    let argnum = arg.parse::<usize>().unwrap_or(0);
                    TagMask::single(argnum).unwrap_or(TagMask::EMPTY)
                };
                crate::tags::view(ctx, mask);
            },
        },
        XCommand {
            cmd: "toggleview",
            action: |ctx, arg| {
                let mask = if arg.is_empty() {
                    TagMask::EMPTY
                } else {
                    let argnum = arg.parse::<usize>().unwrap_or(0);
                    TagMask::single(argnum).unwrap_or(TagMask::EMPTY)
                };
                crate::tags::toggle_view(ctx, mask);
            },
        },
        XCommand {
            cmd: "toggletag",
            action: |ctx, arg| {
                let mask = if arg.is_empty() {
                    TagMask::EMPTY
                } else {
                    let argnum = arg.parse::<usize>().unwrap_or(0);
                    TagMask::single(argnum).unwrap_or(TagMask::EMPTY)
                };
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::tags::toggle_tag(ctx, win, mask);
                }
            },
        },
        XCommand {
            cmd: "togglebar",
            action: |_ctx, _arg| crate::bar::x11::toggle_bar(),
        },
        XCommand {
            cmd: "focusmon",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::monitor::focus_mon(ctx, val);
            },
        },
        XCommand {
            cmd: "tagmon",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::tags::tag_mon(ctx, val);
            },
        },
        XCommand {
            cmd: "focusstack",
            action: |ctx, arg| {
                let direction = if arg.is_empty() {
                    StackDirection::default()
                } else {
                    StackDirection::from_i32(arg.parse().unwrap_or(1))
                };
                crate::focus::focus_stack(ctx, direction);
            },
        },
        XCommand {
            cmd: "incnmaster",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::layouts::inc_nmaster_by(ctx, val);
            },
        },
        XCommand {
            cmd: "setmfact",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    0.05f32
                } else {
                    arg.parse().unwrap_or(0.05)
                };
                crate::layouts::set_mfact(ctx, val);
            },
        },
        XCommand {
            cmd: "zoom",
            action: |ctx, _arg| crate::tags::zoom(ctx),
        },
        XCommand {
            cmd: "killclient",
            action: |ctx, _arg| {
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::client::kill_client(ctx, win);
                }
            },
        },
        XCommand {
            cmd: "setlayout",
            action: |ctx, arg| {
                if arg.is_empty() {
                    crate::layouts::toggle_layout(ctx);
                } else {
                    let val = arg.parse().unwrap_or(0);
                    crate::layouts::command_layout(ctx, val);
                }
            },
        },
        XCommand {
            cmd: "cyclelayout",
            action: |ctx, arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::layouts::cycle_layout_direction(ctx, val > 0);
            },
        },
        XCommand {
            cmd: "togglefloating",
            action: |ctx, _arg| crate::floating::toggle_floating(ctx),
        },
        XCommand {
            cmd: "togglesticky",
            action: |ctx, _arg| {
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::toggles::toggle_sticky(ctx, win);
                }
            },
        },
        XCommand {
            cmd: "togglescratchpad",
            action: |ctx, arg| crate::scratchpad::scratchpad_toggle(ctx, Some(arg)),
        },
        XCommand {
            cmd: "showscratchpad",
            action: |ctx, arg| crate::scratchpad::scratchpad_show_name(ctx, arg),
        },
        XCommand {
            cmd: "hidescratchpad",
            action: |ctx, arg| crate::scratchpad::scratchpad_hide_name(ctx, arg),
        },
        XCommand {
            cmd: "makescratchpad",
            action: |ctx, arg| crate::scratchpad::scratchpad_make(ctx, Some(arg)),
        },
        XCommand {
            cmd: "unmakescratchpad",
            action: |ctx, _arg| crate::scratchpad::scratchpad_unmake(ctx),
        },
        XCommand {
            cmd: "scratchpadstatus",
            action: |ctx, arg| crate::scratchpad::scratchpad_status(ctx, arg),
        },
        XCommand {
            cmd: "setoverlay",
            action: |ctx, _arg| crate::overlay::set_overlay(ctx),
        },
        XCommand {
            cmd: "showoverlay",
            action: |ctx, _arg| crate::overlay::show_overlay(ctx),
        },
        XCommand {
            cmd: "hideoverlay",
            action: |ctx, _arg| crate::overlay::hide_overlay(ctx),
        },
        XCommand {
            cmd: "setoverlaymode",
            action: |ctx, arg| {
                let mode = if arg.is_empty() {
                    OverlayMode::default()
                } else {
                    arg.parse::<i32>()
                        .ok()
                        .and_then(OverlayMode::from_i32)
                        .unwrap_or_default()
                };
                crate::overlay::set_overlay_mode(ctx, mode);
            },
        },
        XCommand {
            cmd: "togglealttag",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                crate::toggles::toggle_alt_tag(ctx, action);
            },
        },
        XCommand {
            cmd: "toggleanimated",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                crate::toggles::toggle_animated(ctx, action);
            },
        },
        XCommand {
            cmd: "togglefocusfollowsmouse",
            action: |ctx, arg| {
                let action = ToggleAction::from_arg(arg);
                crate::toggles::toggle_focus_follows_mouse(ctx, action);
            },
        },
        XCommand {
            cmd: "togglelocked",
            action: |ctx, _arg| {
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::toggles::toggle_locked(ctx, win);
                }
            },
        },
        XCommand {
            cmd: "pushup",
            action: |ctx, _arg| {
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::push::push_up(ctx, win);
                }
            },
        },
        XCommand {
            cmd: "pushdown",
            action: |ctx, _arg| {
                if let Some(win) = crate::client::selected_window(ctx) {
                    crate::push::push_down(ctx, win);
                }
            },
        },
        XCommand {
            cmd: "quit",
            action: |_ctx, _arg| crate::tags::quit(),
        },
    ]
}
