use crate::backend::BackendKind;
use crate::bar::draw_bar;
use crate::contexts::WmCtx;
use crate::types::*;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt;

const COMMAND_INDICATOR: &[u8] = b"c;:;";

pub fn x_command(ctx: &mut WmCtx) -> i32 {
    if ctx.backend_kind() == BackendKind::Wayland {
        return 0;
    }
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return 0;
    };

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

    let selmon_id = ctx.g.selected_monitor_id();
    draw_bar(ctx, selmon_id);
}
