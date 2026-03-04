use crate::bar::draw_bar;
use crate::contexts::WmCtx;
use crate::types::*;

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
