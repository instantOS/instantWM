use crate::bar::draw_bar;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt;

const COMMAND_INDICATOR: &[u8] = b"c;:;";

pub fn x_command() -> i32 {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return 0 };

    let globals = get_globals();
    let root = globals.root;

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

    let commands = globals.commands.clone();

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

        (cmd.action)(arg_str);

        return 1;
    }

    0
}

pub fn set_special_next(value: u32) {
    let globals = get_globals_mut();
    globals.specialnext = match value {
        0 => SpecialNext::None,
        _ => SpecialNext::Float,
    };
}

pub fn command_prefix(value: u32) {
    let globals = get_globals_mut();
    globals.tags.prefix = value != 0;

    let selmon_id = globals.selmon;
    let globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(selmon_id) {
        draw_bar(mon);
    }
}

pub fn init_commands() -> Vec<XCommand> {
    vec![
        XCommand {
            cmd: "layout",
            action: |arg| {
                let val = if arg.is_empty() {
                    0u32
                } else {
                    arg.parse().unwrap_or(0)
                };
                crate::layouts::command_layout(val);
            },
        },
        XCommand {
            cmd: "tag",
            action: |arg| {
                let tag_bits = if arg.is_empty() {
                    0u32
                } else {
                    let argnum = arg.parse::<u32>().unwrap_or(0);
                    if argnum != 0 || arg.starts_with('0') {
                        1 << (argnum.saturating_sub(1))
                    } else {
                        0
                    }
                };
                crate::tags::set_client_tag(tag_bits);
            },
        },
        XCommand {
            cmd: "view",
            action: |arg| {
                let tag_bits = if arg.is_empty() {
                    0u32
                } else {
                    let argnum = arg.parse::<u32>().unwrap_or(0);
                    if argnum != 0 || arg.starts_with('0') {
                        1 << (argnum.saturating_sub(1))
                    } else {
                        0
                    }
                };
                crate::tags::view(tag_bits);
            },
        },
        XCommand {
            cmd: "toggleview",
            action: |arg| {
                let tag_bits = if arg.is_empty() {
                    0u32
                } else {
                    let argnum = arg.parse::<u32>().unwrap_or(0);
                    if argnum != 0 || arg.starts_with('0') {
                        1 << (argnum.saturating_sub(1))
                    } else {
                        0
                    }
                };
                crate::tags::toggle_view(tag_bits);
            },
        },
        XCommand {
            cmd: "toggletag",
            action: |arg| {
                let tag_bits = if arg.is_empty() {
                    0u32
                } else {
                    let argnum = arg.parse::<u32>().unwrap_or(0);
                    if argnum != 0 || arg.starts_with('0') {
                        1 << (argnum.saturating_sub(1))
                    } else {
                        0
                    }
                };
                crate::tags::toggle_tag(tag_bits);
            },
        },
        XCommand {
            cmd: "togglebar",
            action: |_arg| crate::bar::toggle_bar(),
        },
        XCommand {
            cmd: "focusmon",
            action: |arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::monitor::focus_mon(val);
            },
        },
        XCommand {
            cmd: "tagmon",
            action: |arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::monitor::tag_mon(val);
            },
        },
        XCommand {
            cmd: "focusstack",
            action: |arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::focus::focus_stack(val);
            },
        },
        XCommand {
            cmd: "incnmaster",
            action: |arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::layouts::inc_nmaster(val);
            },
        },
        XCommand {
            cmd: "setmfact",
            action: |arg| {
                let val = if arg.is_empty() {
                    0.05f32
                } else {
                    arg.parse().unwrap_or(0.05)
                };
                crate::layouts::set_mfact(val);
            },
        },
        XCommand {
            cmd: "zoom",
            action: |_arg| crate::tags::zoom(),
        },
        XCommand {
            cmd: "killclient",
            action: |_arg| crate::client::kill_client(),
        },
        XCommand {
            cmd: "setlayout",
            action: |arg| {
                let val = if arg.is_empty() {
                    None
                } else {
                    Some(arg.parse().unwrap_or(0))
                };
                crate::layouts::set_layout(val);
            },
        },
        XCommand {
            cmd: "cyclelayout",
            action: |arg| {
                let val = if arg.is_empty() {
                    1i32
                } else {
                    arg.parse().unwrap_or(1)
                };
                crate::layouts::cycle_layout(val);
            },
        },
        XCommand {
            cmd: "togglefloating",
            action: |_arg| crate::floating::toggle_floating(),
        },
        XCommand {
            cmd: "togglesticky",
            action: |_arg| crate::toggles::toggle_sticky(),
        },
        XCommand {
            cmd: "togglescratchpad",
            action: |arg| crate::scratchpad::scratchpad_toggle(Some(arg)),
        },
        XCommand {
            cmd: "showscratchpad",
            action: |arg| crate::scratchpad::scratchpad_show(arg),
        },
        XCommand {
            cmd: "hidescratchpad",
            action: |arg| crate::scratchpad::scratchpad_hide(arg),
        },
        XCommand {
            cmd: "makescratchpad",
            action: |arg| crate::scratchpad::scratchpad_make(Some(arg)),
        },
        XCommand {
            cmd: "unmakescratchpad",
            action: |_arg| crate::scratchpad::scratchpad_unmake(),
        },
        XCommand {
            cmd: "scratchpadstatus",
            action: |arg| crate::scratchpad::scratchpad_status(arg),
        },
        XCommand {
            cmd: "setoverlay",
            action: |_arg| crate::overlay::set_overlay(),
        },
        XCommand {
            cmd: "showoverlay",
            action: |_arg| crate::overlay::show_overlay(),
        },
        XCommand {
            cmd: "hideoverlay",
            action: |_arg| crate::overlay::hide_overlay(),
        },
        XCommand {
            cmd: "setoverlaymode",
            action: |arg| {
                let val = if arg.is_empty() {
                    0i32
                } else {
                    arg.parse().unwrap_or(0)
                };
                crate::overlay::set_overlay_mode_cmd(val);
            },
        },
        XCommand {
            cmd: "togglealttag",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                crate::toggles::toggle_alt_tag(val);
            },
        },
        XCommand {
            cmd: "toggleanimated",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                crate::toggles::toggle_animated(val);
            },
        },
        XCommand {
            cmd: "togglefocusfollowsmouse",
            action: |arg| {
                let val = if arg.is_empty() {
                    2u32
                } else {
                    arg.parse().unwrap_or(2)
                };
                crate::toggles::toggle_focus_follows_mouse(val);
            },
        },
        XCommand {
            cmd: "togglelocked",
            action: |_arg| crate::toggles::toggle_locked(),
        },
        XCommand {
            cmd: "pushup",
            action: |_arg| crate::push::push_up(),
        },
        XCommand {
            cmd: "pushdown",
            action: |_arg| crate::push::push_down(),
        },
        XCommand {
            cmd: "quit",
            action: |_arg| crate::tags::quit(),
        },
    ]
}
