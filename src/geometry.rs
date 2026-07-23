use std::time::{Duration, Instant};

use crate::animation::WindowAnimation;
use crate::constants::animation::{DISTANCE_THRESHOLD, FRAME_SLEEP_MICROS};
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
    /// Force the move to apply even when size-hint adjustment leaves the target
    /// unchanged or there are multiple clients.
    ///
    /// Used by floating transitions: the rectangle is already contained by
    /// [`crate::client::geometry::resolve_floating_transition`] and must reach
    /// the backend regardless of the deduplication heuristics below.
    FloatingTransition,
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

    pub fn for_floating_transition() -> Self {
        Self::immediate()
            .with_size_hints()
            .with_floating_transition_bounds()
    }

    fn with_floating_transition_bounds(mut self) -> Self {
        self.bounds = BoundsPolicy::FloatingTransition;
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClientGeometry {
    current_rect: Rect,
    monitor_rect: Rect,
}

/// Snapshot the client and assigned-monitor geometry needed by a resize.
///
/// Returning owned rectangles ends the model borrow before backend or
/// animation state is mutated. A stale monitor assignment is an invalid model
/// relationship, not a reason to treat the virtual display as one monitor.
fn client_geometry(model: &crate::model::WmModel, win: WindowId) -> Option<ClientGeometry> {
    let view = model.client_view(win)?;
    let current_rect = if view.client.geo.is_valid() {
        view.client.geo
    } else {
        view.client.old_geo
    };

    Some(ClientGeometry {
        current_rect,
        monitor_rect: view.monitor.monitor_rect,
    })
}

fn animation_duration(frames: i32) -> Duration {
    Duration::from_micros(FRAME_SLEEP_MICROS * frames.max(0) as u64)
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
            let _ = wl.wayland.with_state(|state| {
                if let Some(element) = state.find_window(win).cloned()
                    && let Some(surface) = element.x11_surface()
                {
                    let geometry = smithay::utils::Rectangle::new(
                        (to.x, to.y).into(),
                        (to.w.max(1), to.h.max(1)).into(),
                    );
                    let _ = surface.configure(Some(geometry));
                }
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
        WmCtx::Wayland(wl) => wl
            .wayland
            .with_state(|state| state.animation_targets_outer_rect(win, target))
            .unwrap_or(false),
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
        WmCtx::X11(x11_ctx) => {
            let outcome = crate::client::geometry::apply_size_hints(
                x11_ctx.core.model(),
                x11_ctx.core.config(),
                win,
                &mut adjusted,
                interact,
            );
            if outcome.should_apply_client_hints {
                crate::backend::x11::geometry::apply_icccm_size_hints(
                    x11_ctx.core.model_mut(),
                    &x11_ctx.x11,
                    win,
                    &mut adjusted,
                );
            }
            crate::client::geometry::size_hints_changed(x11_ctx.core.model(), win, &adjusted)
        }
        WmCtx::Wayland(wl_ctx) => {
            let outcome = crate::client::geometry::apply_size_hints(
                wl_ctx.core.model(),
                wl_ctx.core.config(),
                win,
                &mut adjusted,
                interact,
            );
            if outcome.should_apply_client_hints
                && let Some(client) = wl_ctx.core.model().client(win)
            {
                let constrained = client.size_hints.constrain_size(
                    adjusted.size(),
                    client.min_aspect,
                    client.max_aspect,
                );
                adjusted = adjusted.with_size(constrained);
            }
            crate::client::geometry::size_hints_changed(wl_ctx.core.model(), win, &adjusted)
        }
    };

    let client_count = ctx.core().model().clients.len();
    if changed || client_count == 1 || options.bounds == BoundsPolicy::FloatingTransition {
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

    let Some(client_geometry) = client_geometry(ctx.core().model(), win) else {
        return;
    };
    let final_rect = target.clamped_to_monitor(
        client_geometry.monitor_rect.w,
        client_geometry.monitor_rect.h,
    );

    match options.mode {
        MoveResizeMode::Immediate => {
            if !should_preserve_inflight_animation(ctx, win, final_rect) {
                crate::animation::cancel_animation(ctx, win);
            }
            ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
        }
        MoveResizeMode::AnimateTo | MoveResizeMode::AnimateFrom(_) => {
            let from = match options.mode {
                MoveResizeMode::AnimateTo => {
                    crate::animation::take_current_animation_rect(ctx, win, Instant::now())
                        .unwrap_or(client_geometry.current_rect)
                }
                MoveResizeMode::Immediate => unreachable!(),
                MoveResizeMode::AnimateFrom(from) => {
                    crate::animation::cancel_animation(ctx, win);
                    from
                }
            };
            if !from.is_valid() {
                return;
            }

            if from == final_rect {
                if ctx
                    .core()
                    .model()
                    .client(win)
                    .is_some_and(|client| client.geo != final_rect)
                {
                    ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
                }
                return;
            }

            let animated = ctx.core().behavior().animated;

            if !animated || options.frames <= 0 {
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
                crate::client::sync_client_geometry(ctx.core_mut().model_mut(), win, final_rect);
            }

            enqueue_window_animation(ctx, win, from, final_rect, options.frames);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::model::WmModel;
    use crate::types::{Client, Monitor};
    use crate::wm::Wm;

    #[test]
    fn client_geometry_uses_assigned_monitor_not_virtual_layout_extent() {
        let mut model = WmModel::new();
        let left = model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        model.monitors.push(Monitor {
            monitor_rect: Rect::new(1920, 0, 2560, 1440),
            ..Monitor::default()
        });
        let win = WindowId(11);
        model.insert_client(Client {
            win,
            monitor_id: left,
            geo: Rect::new(100, 100, 800, 600),
            ..Client::default()
        });

        let geometry = client_geometry(&model, win).expect("client geometry");

        assert_eq!(geometry.current_rect, Rect::new(100, 100, 800, 600));
        assert_eq!(geometry.monitor_rect, Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn client_geometry_falls_back_to_previous_valid_client_rect() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        let win = WindowId(12);
        model.insert_client(Client {
            win,
            monitor_id,
            geo: Rect::default(),
            old_geo: Rect::new(10, 20, 640, 480),
            ..Client::default()
        });

        let geometry = client_geometry(&model, win).expect("client geometry");

        assert_eq!(geometry.current_rect, Rect::new(10, 20, 640, 480));
    }

    #[test]
    fn client_geometry_rejects_stale_monitor_assignment() {
        let mut model = WmModel::new();
        let win = WindowId(13);
        model.insert_client(Client {
            win,
            monitor_id: crate::types::MonitorId::from_raw(1234),
            geo: Rect::new(10, 20, 640, 480),
            ..Client::default()
        });

        assert!(client_geometry(&model, win).is_none());
    }

    #[test]
    fn wayland_hinted_resize_applies_stored_protocol_maximum() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 500, 400),
            available_rect: Rect::new(0, 0, 500, 400),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let win = WindowId(14);
        let mut client = Client {
            win,
            monitor_id,
            geo: Rect::new(0, 0, 50, 50),
            ..Client::default()
        };
        client.size_hints.maxw = 120;
        client.size_hints.maxh = 90;
        wm.core.model.insert_client(client);

        wm.ctx().move_resize(
            win,
            Rect::new(0, 0, 300, 200),
            MoveResizeOptions::hinted_immediate(false),
        );

        assert_eq!(
            wm.core.model.client(win).unwrap().geo,
            Rect::new(0, 0, 120, 90)
        );
    }
}
