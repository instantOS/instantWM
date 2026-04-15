use std::time::{Duration, Instant};

use crate::animation::WindowAnimation;
use crate::constants::animation::{
    DISTANCE_THRESHOLD, FRAME_SLEEP_MICROS, X11_ANIM_FULL_THRESHOLD, X11_ANIM_REDUCE_THRESHOLD,
};
use crate::contexts::WmCtx;
use crate::types::{Rect, WindowId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveResizeMode {
    AnimateTo,
    Immediate,
    AnimateFrom(Rect),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeHintPolicy {
    Ignore,
    Respect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundsPolicy {
    Unchanged,
    Layout,
    Interactive,
}

#[derive(Clone, Copy, Debug)]
pub struct MoveResizeOptions {
    pub mode: MoveResizeMode,
    pub frames: i32,
    pub size_hints: SizeHintPolicy,
    pub bounds: BoundsPolicy,
}

impl MoveResizeOptions {
    pub fn immediate() -> Self {
        Self {
            mode: MoveResizeMode::Immediate,
            frames: 0,
            size_hints: SizeHintPolicy::Ignore,
            bounds: BoundsPolicy::Unchanged,
        }
    }

    pub fn animate_to(frames: i32) -> Self {
        Self {
            mode: MoveResizeMode::AnimateTo,
            frames,
            size_hints: SizeHintPolicy::Ignore,
            bounds: BoundsPolicy::Unchanged,
        }
    }

    pub fn animate_from(from: Rect, frames: i32) -> Self {
        Self {
            mode: MoveResizeMode::AnimateFrom(from),
            frames,
            size_hints: SizeHintPolicy::Ignore,
            bounds: BoundsPolicy::Unchanged,
        }
    }

    pub fn with_size_hints(mut self) -> Self {
        self.size_hints = SizeHintPolicy::Respect;
        self
    }

    pub fn with_layout_bounds(mut self) -> Self {
        self.bounds = BoundsPolicy::Layout;
        self
    }

    pub fn with_interactive_bounds(mut self) -> Self {
        self.bounds = BoundsPolicy::Interactive;
        self
    }

    pub fn hinted_immediate(interactive: bool) -> Self {
        let options = Self::immediate().with_size_hints();
        if interactive {
            options.with_interactive_bounds()
        } else {
            options.with_layout_bounds()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GeometryApplyMode {
    Logical,
    VisualOnly,
}

fn current_client_rect(ctx: &WmCtx<'_>, win: WindowId) -> Option<Rect> {
    ctx.core()
        .globals()
        .clients
        .get(&win)
        .map(|c| if c.geo.is_valid() { c.geo } else { c.old_geo })
}

fn monitor_size_for_client(ctx: &WmCtx<'_>, win: WindowId) -> (i32, i32) {
    ctx.core()
        .globals()
        .clients
        .get(&win)
        .and_then(|c| ctx.core().globals().monitors.get(c.monitor_id))
        .map(|m| (m.monitor_rect.w, m.monitor_rect.h))
        .unwrap_or((
            ctx.core().globals().cfg.screen_width,
            ctx.core().globals().cfg.screen_height,
        ))
}

fn final_rect_for_target(target: Rect, mon_w: i32, mon_h: i32) -> Rect {
    Rect {
        x: target.x,
        y: target.y,
        w: target.w.min(mon_w).max(1),
        h: target.h.min(mon_h).max(1),
    }
}

fn animation_duration(frames: i32) -> Duration {
    Duration::from_micros(FRAME_SLEEP_MICROS * frames.max(0) as u64)
}

fn effective_animation_frames(active_count: usize, frames: i32) -> i32 {
    if active_count >= X11_ANIM_REDUCE_THRESHOLD {
        0
    } else if active_count >= X11_ANIM_FULL_THRESHOLD {
        (frames / 2).max(2)
    } else {
        frames
    }
}

fn active_animation_count(ctx: &WmCtx<'_>) -> usize {
    match ctx {
        WmCtx::X11(x11) => x11.x11_runtime.window_animations.len(),
        WmCtx::Wayland(wl) => wl
            .wayland
            .backend
            .with_state(|state| state.active_window_animation_count())
            .unwrap_or(0),
    }
}

fn enqueue_window_animation(ctx: &mut WmCtx<'_>, win: WindowId, from: Rect, to: Rect, frames: i32) {
    let duration = animation_duration(frames);
    match ctx {
        WmCtx::X11(x11) => {
            let mut wmctx = crate::contexts::WmCtx::X11(x11.reborrow());
            wmctx.set_geometry_impl(win, from, GeometryApplyMode::VisualOnly);
            x11.x11_runtime.insert_or_replace_window_animation(
                win,
                WindowAnimation {
                    from,
                    to,
                    started_at: Instant::now(),
                    duration,
                },
            );
        }
        WmCtx::Wayland(wl) => {
            let _ = wl.wayland.backend.with_state(|state| {
                state.set_window_target_rect(
                    win,
                    to,
                    crate::backend::wayland::compositor::window::animations::WindowMoveMode::AnimateFrom {
                        from,
                        duration,
                    },
                );
            });
        }
    }
}

fn should_preserve_inflight_animation(ctx: &WmCtx<'_>, win: WindowId, target: Rect) -> bool {
    match ctx {
        WmCtx::X11(x11) => x11
            .x11_runtime
            .window_animations
            .get(&win)
            .is_some_and(|anim| anim.to == target),
        WmCtx::Wayland(wl) => {
            let border_width = wl
                .core
                .globals()
                .clients
                .get(&win)
                .map(|c| c.border_width())
                .unwrap_or(0);
            let requested_surface_target = Rect {
                x: target.x + border_width,
                y: target.y + border_width,
                w: target.w,
                h: target.h,
            };
            wl.wayland
                .backend
                .with_state(|state| {
                    state
                        .window_animation_target(win)
                        .is_some_and(|anim| anim.to == requested_surface_target)
                })
                .unwrap_or(false)
        }
    }
}

fn apply_resize_policies(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    target: Rect,
    options: MoveResizeOptions,
) -> Option<Rect> {
    if options.size_hints == SizeHintPolicy::Ignore {
        return Some(target);
    }

    let mut adjusted = target;
    let interact = options.bounds == BoundsPolicy::Interactive;
    let changed = match ctx {
        WmCtx::X11(x11_ctx) => crate::client::geometry::apply_size_hints(
            &mut x11_ctx.core,
            Some(&x11_ctx.x11),
            win,
            &mut adjusted,
            interact,
        ),
        WmCtx::Wayland(wl_ctx) => crate::client::geometry::apply_size_hints(
            &mut wl_ctx.core,
            None,
            win,
            &mut adjusted,
            interact,
        ),
    };

    let client_count = ctx.core().globals().clients.len();
    if changed || client_count == 1 {
        Some(adjusted)
    } else {
        None
    }
}

pub(crate) fn move_resize(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    target: Rect,
    options: MoveResizeOptions,
) {
    if options.size_hints == SizeHintPolicy::Ignore && !target.is_valid() {
        return;
    }

    let Some(target) = apply_resize_policies(ctx, win, target, options) else {
        return;
    };
    if !target.is_valid() {
        return;
    }

    let (mon_w, mon_h) = monitor_size_for_client(ctx, win);
    let final_rect = final_rect_for_target(target, mon_w, mon_h);

    match options.mode {
        MoveResizeMode::Immediate => {
            if !should_preserve_inflight_animation(ctx, win, final_rect) {
                crate::animation::cancel_animation(ctx, win);
            }
            ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
        }
        MoveResizeMode::AnimateTo | MoveResizeMode::AnimateFrom(_) => {
            crate::animation::cancel_animation(ctx, win);

            let from = match options.mode {
                MoveResizeMode::AnimateTo => current_client_rect(ctx, win),
                MoveResizeMode::Immediate => unreachable!(),
                MoveResizeMode::AnimateFrom(from) => Some(from),
            };
            let Some(from) = from else {
                return;
            };
            if !from.is_valid() {
                return;
            }

            if from == final_rect {
                if ctx.client(win).is_some_and(|c| c.geo != final_rect) {
                    ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
                }
                return;
            }

            let animated = ctx.core().globals().behavior.animated;
            let effective_frames =
                effective_animation_frames(active_animation_count(ctx), options.frames);

            if !animated || effective_frames <= 0 {
                ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
                return;
            }

            let dist_moved = (final_rect.x - from.x).abs() > DISTANCE_THRESHOLD
                || (final_rect.y - from.y).abs() > DISTANCE_THRESHOLD
                || (final_rect.w - from.w).abs() > DISTANCE_THRESHOLD
                || (final_rect.h - from.h).abs() > DISTANCE_THRESHOLD;
            if !dist_moved {
                ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
                return;
            }

            if options.mode == MoveResizeMode::AnimateTo {
                crate::client::sync_client_geometry(ctx.core_mut().globals_mut(), win, final_rect);
            }

            enqueue_window_animation(ctx, win, from, final_rect, effective_frames);
        }
    }
}
