use crate::contexts::{CoreCtx, WmCtx};
use crate::types::*;

pub fn set_special_next(core: &mut CoreCtx, value: u32) {
    core.g.specialnext = match value {
        0 => SpecialNext::None,
        _ => SpecialNext::Float,
    };
}

pub fn command_prefix(ctx: &mut WmCtx, value: u32) {
    ctx.g_mut().tags.prefix = value != 0;

    let selmon_id = ctx.g().selected_monitor_id();
    ctx.request_bar_update(Some(selmon_id));
}
