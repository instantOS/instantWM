mod color;
mod model;
mod paint;
mod renderer;
mod status;
mod theme;
pub mod wayland;
mod widgets;
pub mod x11;
mod x11_painter;

pub use model::{bar_position_at_x, bar_position_to_gesture};
pub use x11::{resize_bar_win, resize_bar_win_ctx};

use crate::backend::BackendKind;
use crate::contexts::WmCtx;
use crate::types::*;

#[derive(Default)]
pub struct BarState {
    pausedraw: bool,
    draw_bar_recursion: usize,
    pub command_offsets: [i32; 20],
}

impl BarState {
    pub fn pausedraw(&self) -> bool {
        self.pausedraw
    }

    pub fn set_pausedraw(&mut self, paused: bool) {
        self.pausedraw = paused;
    }

    fn recursion_enter(&mut self) {
        self.draw_bar_recursion += 1;
        if self.draw_bar_recursion > 50 {
            std::process::abort();
        }
    }

    fn recursion_exit(&mut self) {
        self.draw_bar_recursion = self.draw_bar_recursion.saturating_sub(1);
    }

    pub fn clear_command_offsets(&mut self) {
        self.command_offsets.fill(-1);
    }
}

pub fn text_width_ctx(ctx: &crate::contexts::WmCtx, text: &str) -> i32 {
    if let Some(mut drw) = ctx
        .g
        .cfg
        .drw
        .as_ref()
        .and_then(|drw| drw.has_display().then(|| drw.clone()))
    {
        return drw.fontset_getwidth(text) as i32;
    }
    ctx.bar_painter.measure_text_width(text)
}

pub(crate) fn layout_symbol(m: &Monitor) -> String {
    m.layout_symbol()
}

pub fn get_layout_symbol_width(ctx: &WmCtx, m: &Monitor) -> i32 {
    text_width_ctx(ctx, &layout_symbol(m)) + ctx.g.cfg.horizontal_padding
}

pub fn draw_bar(ctx: &mut WmCtx, mon_idx: usize) {
    if ctx.backend_kind() == BackendKind::Wayland {
        wayland::draw_bar_wayland(ctx, mon_idx);
        return;
    }
    if ctx.x11_conn().is_none() {
        return;
    }
    let barwin = ctx.g.monitor(mon_idx).map(|m| m.barwin).unwrap_or_default();
    if barwin == WindowId::default() {
        return;
    }
    let work_rect_w = match ctx.g.monitor(mon_idx) {
        Some(m) => m.work_rect.w,
        None => return,
    };
    let bh = ctx.g.cfg.bar_height;
    if work_rect_w <= 0 || bh <= 0 {
        return;
    }

    let drw = {
        let Some(drw) = ctx.g.cfg.drw.as_mut() else {
            return;
        };
        if !drw.has_display() {
            return;
        }
        drw.resize(work_rect_w as u32, bh as u32);
        drw.clone()
    };

    let mut painter = x11_painter::X11BarPainter::new(drw);

    renderer::draw_bar_common(ctx, mon_idx, &mut painter);

    painter.map(barwin, 0, 0, work_rect_w as u16, bh as u16);
}

pub fn draw_bars(ctx: &mut WmCtx) {
    if ctx.backend_kind() == BackendKind::Wayland {
        wayland::draw_bars_wayland(ctx);
        return;
    }
    if ctx.x11_conn().is_none() {
        return;
    }
    let indices: Vec<usize> = ctx.g.monitors_iter().map(|(i, _)| i).collect();
    for i in indices {
        draw_bar(ctx, i);
    }
}

pub fn reset_bar(ctx: &mut WmCtx) {
    if ctx.backend_kind() == BackendKind::Wayland {
        wayland::reset_bar_wayland(ctx);
        return;
    }
    if ctx.x11_conn().is_none() {
        return;
    }
    let selmon_idx = ctx.g.selmon_id();
    renderer::reset_bar_common(ctx);
    draw_bar(ctx, selmon_idx);
}

pub fn should_draw_bar(ctx: &WmCtx) -> bool {
    renderer::should_draw_bar_common(ctx)
}
