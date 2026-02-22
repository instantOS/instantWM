use crate::bar::draw_bar;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt;

pub const CMD_ARG_NONE: u32 = 0;
pub const CMD_ARG_TOGGLE: u32 = 1;
pub const CMD_ARG_TAG: u32 = 2;
pub const CMD_ARG_STRING: u32 = 3;
pub const CMD_ARG_INT: u32 = 4;

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

        let arg = if fcursor.is_empty() {
            cmd.arg
        } else {
            if fcursor[0] != b';' {
                continue;
            }
            fcursor = &fcursor[1..];

            let arg_str = std::str::from_utf8(fcursor).unwrap_or("");

            match cmd.cmd_type {
                CMD_ARG_NONE => cmd.arg,
                CMD_ARG_TOGGLE => {
                    let argnum = arg_str.parse::<u32>().unwrap_or(0);
                    if argnum != 0 || fcursor.starts_with(b"0") {
                        Arg {
                            ui: argnum,
                            ..Default::default()
                        }
                    } else {
                        cmd.arg
                    }
                }
                CMD_ARG_TAG => {
                    let argnum = arg_str.parse::<u32>().unwrap_or(0);
                    if argnum != 0 || fcursor.starts_with(b"0") {
                        Arg {
                            ui: 1 << (argnum.saturating_sub(1)),
                            ..Default::default()
                        }
                    } else {
                        cmd.arg
                    }
                }
                CMD_ARG_STRING => {
                    let ptr = fcursor.as_ptr() as usize;
                    Arg {
                        v: Some(ptr),
                        ..Default::default()
                    }
                }
                CMD_ARG_INT => {
                    if !fcursor.is_empty() {
                        let val = arg_str.parse::<i32>().unwrap_or(0);
                        Arg {
                            i: val,
                            ..Default::default()
                        }
                    } else {
                        cmd.arg
                    }
                }
                _ => cmd.arg,
            }
        };

        if let Some(func) = cmd.func {
            func(&arg);
        }

        return 1;
    }

    0
}

pub fn set_special_next(arg: &Arg) {
    let globals = get_globals_mut();
    globals.specialnext = match arg.ui {
        0 => SpecialNext::None,
        _ => SpecialNext::Float,
    };
}

pub fn command_prefix(arg: &Arg) {
    let globals = get_globals_mut();
    globals.tags.prefix = arg.ui != 0;

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
            func: Some(crate::layouts::command_layout),
            arg: Arg::default(),
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "tag",
            func: Some(crate::tags::command_tag),
            arg: Arg::default(),
            cmd_type: CMD_ARG_TAG,
        },
        XCommand {
            cmd: "view",
            func: Some(crate::tags::command_view),
            arg: Arg::default(),
            cmd_type: CMD_ARG_TAG,
        },
        XCommand {
            cmd: "toggleview",
            func: Some(crate::tags::command_toggle_view),
            arg: Arg::default(),
            cmd_type: CMD_ARG_TAG,
        },
        XCommand {
            cmd: "toggletag",
            func: Some(crate::tags::command_toggle_tag),
            arg: Arg::default(),
            cmd_type: CMD_ARG_TAG,
        },
        XCommand {
            cmd: "togglebar",
            func: Some(crate::bar::toggle_bar),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "focusmon",
            func: Some(crate::monitor::focus_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "tagmon",
            func: Some(crate::monitor::tag_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "focusstack",
            func: Some(crate::focus::focus_stack),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "incnmaster",
            func: Some(crate::layouts::inc_nmaster),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "setmfact",
            func: Some(crate::layouts::set_mfact),
            arg: Arg {
                f: 0.05,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "zoom",
            func: Some(crate::tags::zoom),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "killclient",
            func: Some(crate::client::kill_client),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "setlayout",
            func: Some(crate::layouts::set_layout),
            arg: Arg {
                v: Some(0),
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "cyclelayout",
            func: Some(crate::layouts::cycle_layout),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "togglefloating",
            func: Some(crate::floating::toggle_floating),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "togglesticky",
            func: Some(crate::toggles::toggle_sticky),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "togglescratchpad",
            func: Some(crate::scratchpad::scratchpad_toggle),
            arg: Arg::default(),
            cmd_type: CMD_ARG_STRING,
        },
        XCommand {
            cmd: "showscratchpad",
            func: Some(crate::scratchpad::scratchpad_show),
            arg: Arg::default(),
            cmd_type: CMD_ARG_STRING,
        },
        XCommand {
            cmd: "hidescratchpad",
            func: Some(crate::scratchpad::scratchpad_hide),
            arg: Arg::default(),
            cmd_type: CMD_ARG_STRING,
        },
        XCommand {
            cmd: "makescratchpad",
            func: Some(crate::scratchpad::scratchpad_make),
            arg: Arg::default(),
            cmd_type: CMD_ARG_STRING,
        },
        XCommand {
            cmd: "unmakescratchpad",
            func: Some(crate::scratchpad::scratchpad_unmake),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "scratchpadstatus",
            func: Some(crate::scratchpad::scratchpad_status),
            arg: Arg::default(),
            cmd_type: CMD_ARG_STRING,
        },
        XCommand {
            cmd: "setoverlay",
            func: Some(crate::overlay::set_overlay),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "showoverlay",
            func: Some(crate::overlay::show_overlay),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "hideoverlay",
            func: Some(crate::overlay::hide_overlay),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "setoverlaymode",
            func: Some(crate::overlay::set_overlay_mode_cmd),
            arg: Arg {
                i: 0,
                ..Default::default()
            },
            cmd_type: CMD_ARG_INT,
        },
        XCommand {
            cmd: "togglealttag",
            func: Some(crate::toggles::toggle_alt_tag),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: CMD_ARG_TOGGLE,
        },
        XCommand {
            cmd: "toggleanimated",
            func: Some(crate::toggles::toggle_animated),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: CMD_ARG_TOGGLE,
        },
        XCommand {
            cmd: "togglefocusfollowsmouse",
            func: Some(crate::toggles::toggle_focus_follows_mouse),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: CMD_ARG_TOGGLE,
        },
        XCommand {
            cmd: "togglelocked",
            func: Some(crate::toggles::toggle_locked),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "pushup",
            func: Some(crate::push::push_up),
            arg: Arg {
                f: 0.0,
                ..Default::default()
            },
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "pushdown",
            func: Some(crate::push::push_down),
            arg: Arg {
                f: 0.0,
                ..Default::default()
            },
            cmd_type: CMD_ARG_NONE,
        },
        XCommand {
            cmd: "quit",
            func: Some(crate::tags::quit),
            arg: Arg::default(),
            cmd_type: CMD_ARG_NONE,
        },
    ]
}
