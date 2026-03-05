use crate::bar::draw_bar;
use crate::contexts::{CoreCtx, X11Ctx};
use crate::types::*;

pub fn set_special_next(core: &mut CoreCtx, value: u32) {
    core.g.specialnext = match value {
        0 => SpecialNext::None,
        _ => SpecialNext::Float,
    };
}

pub fn command_prefix(core: &mut CoreCtx, x11: &X11Ctx, value: u32) {
    core.g.tags.prefix = value != 0;

    let selmon_id = core.g.selected_monitor_id();
    draw_bar(core, x11, selmon_id);
}
