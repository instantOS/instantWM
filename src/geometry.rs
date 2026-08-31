use std::time::{Duration, Instant};

use crate::constants::animation::DISTANCE_THRESHOLD;
use crate::contexts::WmCtx;
use crate::types::{Rect, WindowId};

/// Relationship between a backend geometry observation and the WM's latest
/// geometry request for that window.
///
/// The Wayland backend derives this from the xdg configure serial a surface
/// commit acknowledges. Backends without a request/acknowledgement transaction
/// simply report every observation as
/// [`Unsolicited`](Self::Unsolicited). This keeps protocol ordering out of
/// core policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GeometryResponse {
    /// The observation answers the latest outstanding WM request.
    Current,
    /// The observation answers an older request and must not supersede the
    /// latest WM intent.
    Stale,
    /// No WM geometry request is outstanding; the client/backend originated
    /// this observation.
    Unsolicited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GeometryCommitDecision {
    /// Whether this response completes the latest outstanding request.
    pub settle_request: bool,
    /// Whether its actual size may replace floating model geometry.
    pub accept_client_size: bool,
}

/// Define backend-neutral geometry-authority policy for client observations.
///
/// Stale responses may be presented by a backend, but never feed back into the
/// logical model. A current or unsolicited response becomes authoritative only
/// for client-sized windows (normal floating clients today); layout-owned
/// windows retain the WM target.
pub(crate) fn reconcile_geometry_commit(
    response: GeometryResponse,
    client_size_is_authoritative: bool,
) -> GeometryCommitDecision {
    match response {
        GeometryResponse::Current => GeometryCommitDecision {
            settle_request: true,
            accept_client_size: client_size_is_authoritative,
        },
        GeometryResponse::Stale => GeometryCommitDecision {
            settle_request: false,
            accept_client_size: false,
        },
        GeometryResponse::Unsolicited => GeometryCommitDecision {
            settle_request: false,
            accept_client_size: client_size_is_authoritative,
        },
    }
}

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
    pub duration: Duration,
    pub size_hints: SizeHintPolicy,
    pub bounds: BoundsPolicy,
}

impl MoveResizeOptions {
    pub fn immediate() -> Self {
        Self {
            mode: MoveResizeMode::Immediate,
            duration: Duration::ZERO,
            size_hints: SizeHintPolicy::Ignore,
            bounds: BoundsPolicy::Unchanged,
        }
    }

    pub fn animate_to(millis: u64) -> Self {
        Self {
            mode: MoveResizeMode::AnimateTo,
            duration: Duration::from_millis(millis),
            size_hints: SizeHintPolicy::Ignore,
            bounds: BoundsPolicy::Unchanged,
        }
    }

    pub fn animate_from(from: Rect, millis: u64) -> Self {
        Self {
            mode: MoveResizeMode::AnimateFrom(from),
            duration: Duration::from_millis(millis),
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
        Self::animate_to(crate::constants::animation::DEFAULT_ANIMATION_MILLIS)
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

fn animation_duration(
    config: crate::config::config_toml::AnimationConfig,
    duration: Duration,
) -> Duration {
    config.scale_duration(duration)
}

fn enqueue_window_animation(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    from: Rect,
    to: Rect,
    duration: Duration,
) {
    let duration = animation_duration(ctx.core().config().animations, duration);
    ctx.begin_window_animation(win, from, to, duration);
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
    let outcome = crate::client::geometry::apply_size_hints(
        ctx.core().model(),
        ctx.core().config(),
        ctx.core().derived(),
        win,
        &mut adjusted,
        interact,
    );
    ctx.refine_size_hints(win, outcome.should_apply_client_hints, &mut adjusted);
    let changed = crate::client::geometry::size_hints_changed(ctx.core().model(), win, &adjusted);

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
    let final_rect = target.clamped_size_to(
        client_geometry.monitor_rect.w,
        client_geometry.monitor_rect.h,
    );

    match options.mode {
        MoveResizeMode::Immediate => {
            if !ctx.has_inflight_animation_to(win, final_rect) {
                // The new immediate geometry supersedes the old target. Drop
                // its presentation state without configuring the obsolete
                // size first; set_geometry_impl applies the new target below.
                let _ = ctx.take_current_animation_rect(win, Instant::now());
            }
            ctx.set_geometry_impl(win, final_rect, GeometryApplyMode::Logical);
        }
        MoveResizeMode::AnimateTo | MoveResizeMode::AnimateFrom(_) => {
            let from = match options.mode {
                MoveResizeMode::AnimateTo => ctx
                    .take_current_animation_rect(win, Instant::now())
                    .unwrap_or(client_geometry.current_rect),
                MoveResizeMode::Immediate => unreachable!(),
                MoveResizeMode::AnimateFrom(from) => {
                    // The explicit starting frame supersedes any previous
                    // transition, so its target must not be committed first.
                    let _ = ctx.take_current_animation_rect(win, Instant::now());
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

            if !animated || options.duration.is_zero() {
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

            enqueue_window_animation(ctx, win, from, final_rect, options.duration);
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
    fn stale_geometry_response_cannot_supersede_the_latest_resize_intent() {
        assert_eq!(
            reconcile_geometry_commit(GeometryResponse::Stale, true),
            GeometryCommitDecision {
                settle_request: false,
                accept_client_size: false,
            }
        );
        assert_eq!(
            reconcile_geometry_commit(GeometryResponse::Current, true),
            GeometryCommitDecision {
                settle_request: true,
                accept_client_size: true,
            }
        );
    }

    #[test]
    fn current_constrained_response_is_authoritative_only_for_client_sized_windows() {
        assert!(reconcile_geometry_commit(GeometryResponse::Current, true).accept_client_size);
        assert!(!reconcile_geometry_commit(GeometryResponse::Current, false).accept_client_size);
        assert!(reconcile_geometry_commit(GeometryResponse::Unsolicited, true).accept_client_size);
        assert!(
            !reconcile_geometry_commit(GeometryResponse::Unsolicited, false).accept_client_size
        );
    }

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
        client.size_hints.max_width = 120;
        client.size_hints.max_height = 90;
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
