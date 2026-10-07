//! Fullscreen and fake-fullscreen state management.
//!
//! # Responsibilities
//!
//! * [`set_fullscreen`]         – enter/exit real fullscreen, updating
//!   backend protocol state and animating the transition.
//! * [`toggle_fake_fullscreen`] – cycle "fake" fullscreen (window fills the
//!   monitor for the application but still participates in the layout).
//! * [`apply_client_maximize_intent`] – interpret an application's
//!   maximize/restore request.
//!
//! ## Real vs. fake fullscreen
//!
//! *Real* fullscreen:
//! the border is removed, the window is raised above everything else, and it
//! is resized to exactly the monitor rectangle.
//!
//! *Fake* fullscreen:
//! the fullscreen protocol signal is advertised (`_NET_WM_STATE_FULLSCREEN`
//! on X11, xdg_toplevel state on Wayland) so the application believes it is
//! fullscreen, but the window remains in the normal layout stack with its
//! border intact.
//!
//! All protocol projection flows through [`WmCtx`] delegation methods; this
//! module contains no backend imports and one shared policy for both
//! backends.

use crate::layouts::ArrangeAnimation;

use crate::client::mode::{
    ClientMaximizeIntentOutcome, FullscreenChange, FullscreenEntryProjection, MaximizedChange,
};
use crate::constants::animation::EMPHASIZED_ANIMATION_MILLIS;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::{arrange, sync_z_order};
use crate::types::{Rect, WindowId};

// ---------------------------------------------------------------------------
// Real fullscreen
// ---------------------------------------------------------------------------

/// Backend‑agnostic entry point: enter or exit real fullscreen for `win`.
///
/// Handles shared state (mode, layout, z‑order) as a pure model transition,
/// then projects backend effects through `WmCtx` methods.
///
/// Native Wayland protocol requests commit the same model transaction
/// synchronously through the compositor command bridge before acknowledging
/// client state.
pub fn set_fullscreen(ctx: &mut WmCtx<'_>, win: WindowId, fullscreen: bool) {
    let Some(transition) = ctx.core_mut().state.model.set_fullscreen(win, fullscreen) else {
        return;
    };
    let monitor_id = transition.monitor_id;

    match transition.change {
        FullscreenChange::Unchanged => {}
        FullscreenChange::Entered {
            monitor_rect,
            projection,
        } => {
            ctx.set_client_fullscreen_signal(win, true);
            if projection == FullscreenEntryProjection::Animated {
                ctx.move_resize(
                    win,
                    monitor_rect,
                    MoveResizeOptions::animate_to(EMPHASIZED_ANIMATION_MILLIS),
                );
            }
            ctx.apply_entered_fullscreen_effects(win, monitor_rect);
            ctx.request_bar_geometry_update(monitor_id);
            sync_z_order(ctx);
        }
        FullscreenChange::Exited { restore_rect } => {
            ctx.set_client_fullscreen_signal(win, false);
            ctx.apply_exited_fullscreen_effects(win);
            if let Some(rect) = restore_rect {
                ctx.move_resize(win, rect, MoveResizeOptions::immediate());
            }
            arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
        }
    }
}

/// Interpret and project an application's maximize/restore intent.
pub(crate) fn apply_client_maximize_intent(ctx: &mut WmCtx<'_>, win: WindowId, maximized: bool) {
    let Some(transition) = ctx
        .core_mut()
        .state
        .model
        .apply_client_maximize_intent(win, maximized)
    else {
        return;
    };
    apply_client_maximize_intent_transition(ctx, win, transition);
    sync_owned_client_maximized_signal(ctx, transition.monitor_id, win);
    if transition.entered_floating_presentation() {
        ctx.raise_client(win);
    }
}

/// Leave the active maximized presentation, regardless of who requested it.
///
/// Returns `false` when the window is missing or is not currently maximized.
/// Explicit move and placement operations use this as their single
/// maximization exit path so client protocol state cannot be left behind.
pub(crate) fn leave_maximized(ctx: &mut WmCtx<'_>, win: WindowId) -> bool {
    let Some(left) = ctx.core_mut().state.model.leave_maximized(win) else {
        return false;
    };

    // Project transition geometry first. Native Wayland can then advertise
    // the cleared state and restored size in one final configure instead of
    // briefly advertising an unmaximized, maximized-sized window.
    apply_maximized_change(ctx, win, left.monitor_id, left.change);

    sync_owned_client_maximized_signal(ctx, left.monitor_id, win);

    true
}

/// Project the application-visible maximized state of `win`.
///
/// Callers that already know the owning monitor use
/// [`sync_owned_client_maximized_signal`] instead of re-resolving the window.
pub(crate) fn sync_client_maximized_signal(ctx: &mut WmCtx<'_>, win: WindowId) {
    let Some(maximized) = ctx.core().state.model.client_protocol_maximized(win) else {
        return;
    };
    ctx.set_client_maximized_signal(win, maximized);
}

/// Project maximization after a transition on the known owning monitor.
fn sync_owned_client_maximized_signal(
    ctx: &mut WmCtx<'_>,
    monitor_id: crate::types::MonitorId,
    win: WindowId,
) {
    let Some(maximized) = ctx
        .core()
        .state
        .model
        .monitor(monitor_id)
        .and_then(|monitor| {
            monitor
                .client(win)
                .map(|client| crate::client::mode::protocol_maximized(monitor, client))
        })
    else {
        return;
    };
    ctx.set_client_maximized_signal(win, maximized);
}

fn apply_client_maximize_intent_transition(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    transition: crate::client::mode::ClientMaximizeIntentTransition,
) {
    let monitor_id = transition.monitor_id;
    match transition.outcome {
        ClientMaximizeIntentOutcome::FloatingPresentation(change) => {
            apply_maximized_change(ctx, win, monitor_id, change);
        }
        ClientMaximizeIntentOutcome::Placement {
            placement,
            visible_restore_rect,
            ..
        } => {
            if placement == crate::types::ClientPlacement::Floating {
                ctx.apply_floating_border_scheme(win);
                if let Some(rect) = visible_restore_rect {
                    ctx.move_resize(win, rect, MoveResizeOptions::for_floating_transition());
                }
            }
            arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
        }
        ClientMaximizeIntentOutcome::Rejected => {}
    }
}

fn apply_maximized_change(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    monitor_id: crate::types::MonitorId,
    change: MaximizedChange,
) {
    match change {
        MaximizedChange::Entered { work_rect } => {
            ctx.move_resize(win, work_rect, MoveResizeOptions::immediate());
            arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
        }
        MaximizedChange::Exited { restore_rect } => {
            if let Some(rect) = restore_rect {
                ctx.move_resize(win, rect, MoveResizeOptions::immediate());
            }
            arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
        }
        MaximizedChange::Unchanged => {}
    }
}

// ---------------------------------------------------------------------------
// Fake fullscreen toggle
// ---------------------------------------------------------------------------

/// Cycle fake-fullscreen on the selected client.
///
/// The state machine is identical on both backends:
///
/// ```text
/// normal ──toggle──► fake ──toggle──► real ──toggle──► fake ──► ...
/// ```
///
/// * Entering fake advertises the fullscreen signal but keeps the window in
///   the layout with its border.
/// * Leaving fake promotes to *real* fullscreen: the window claims the whole
///   monitor immediately and drops its border. Use the regular fullscreen
///   toggle to leave real fullscreen again.
pub fn toggle_fake_fullscreen(ctx: &mut WmCtx<'_>) {
    let core_state = &ctx.core().state;
    let Some(win) = core_state.model.selected_win() else {
        return;
    };
    let Some(view) = core_state.model.client_view(win) else {
        return;
    };
    let monitor_id = view.monitor.id();
    let was_fake = view.client.mode().is_fake_fullscreen();
    let old_border_width = view.client.old_border_width;
    let promotion_monitor_rect = was_fake.then_some(view.monitor.monitor_rect);

    // Fake → real promotion: claim the monitor rectangle immediately so the
    // transition reads as a single step instead of waiting for the layout.
    if let Some(mon_rect) = promotion_monitor_rect {
        let border_px = core_state.config.window.border_width_px;
        ctx.move_resize(
            win,
            Rect {
                x: mon_rect.x + border_px,
                y: mon_rect.y + border_px,
                w: mon_rect.w - 2 * border_px,
                h: mon_rect.h - 2 * border_px,
            },
            MoveResizeOptions::immediate(),
        );
        ctx.raise_client(win);
    }

    if let Some(client) = ctx
        .core_mut()
        .state
        .model
        .monitor_mut(monitor_id)
        .and_then(|monitor| monitor.client_mut(win))
    {
        if was_fake {
            client.enter_fullscreen();
        } else {
            client.enter_fake_fullscreen();
        }
        // Real fullscreen strips the border; every other outcome restores
        // the width saved before fullscreen was entered.
        client.border_width = if client.mode().is_true_fullscreen() {
            0
        } else {
            old_border_width
        };
    }

    // Both real and fake fullscreen advertise the fullscreen signal; fake
    // fullscreen differs only in remaining part of the layout stack.
    ctx.set_client_fullscreen_signal(win, true);

    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

#[cfg(test)]
mod tests {
    use super::toggle_fake_fullscreen;
    use crate::test_support::TestWm as Wm;

    use crate::test_support::MonitorBuilder;
    use crate::types::{Client, Rect, TagMask, WindowId};

    #[test]
    fn fake_fullscreen_cycle_is_shared_by_wayland() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.config.window.border_width_px = 3;
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .rect(Rect::new(100, 50, 1200, 800), Rect::new(100, 50, 1200, 800))
                .tag_count(1)
                .selected_tags(tags)
                .build(),
        );

        let win = WindowId(7);
        assert!(wm.core.state.model.add_client(
            monitor_id,
            Client {
                win,
                tags,
                geo: Rect::new(200, 150, 500, 400),
                border_width: 3,
                old_border_width: 3,
                ..Client::default()
            }
        ));
        wm.core
            .state
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected(Some(win));

        toggle_fake_fullscreen(&mut wm.test_ctx());
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_fake_fullscreen());
        assert_eq!(client.border_width, 3);

        toggle_fake_fullscreen(&mut wm.test_ctx());
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_true_fullscreen());
        assert_eq!(client.border_width, 0);
        assert_eq!(client.geo, Rect::new(103, 53, 1194, 794));

        toggle_fake_fullscreen(&mut wm.test_ctx());
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_fake_fullscreen());
        assert_eq!(client.border_width, 3);
    }
}
