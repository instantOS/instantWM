//! Fullscreen and fake-fullscreen state management.
//!
//! # Responsibilities
//!
//! * [`set_fullscreen`]         – interpret an application fullscreen request
//!   through the per-window fake-fullscreen policy, updating backend protocol
//!   state and animating the transition.
//! * [`toggle_fake_fullscreen`] – arm/disarm the per-window policy that lets
//!   an application believe it is fullscreen while the window stays in the
//!   normal layout.
//! * [`apply_client_maximize_intent`] – interpret an application's
//!   maximize/restore request.
//!
//! ## Real vs. fake fullscreen
//!
//! *Real* fullscreen:
//! the border is removed, the window is raised above everything else, and it
//! is resized to exactly the monitor rectangle.
//!
//! *Fake* fullscreen is an opt-in per-window policy. Arming it does not
//! change protocol state or geometry. When the application next requests
//! fullscreen, the fullscreen protocol signal is advertised
//! (`_NET_WM_STATE_FULLSCREEN` on X11, xdg_toplevel state on Wayland) so the
//! application believes it is fullscreen, but the window remains in the normal
//! layout stack with its border intact and is still movable/resizable.
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
use crate::types::WindowId;

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
    apply_fullscreen_transition(ctx, win, transition);
}

fn apply_fullscreen_transition(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    transition: crate::client::mode::FullscreenTransition,
) {
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
            ctx.core_mut()
                .bar
                .invalidate(crate::bar::BarChange::Presentation);
            sync_z_order(ctx);
        }
        FullscreenChange::Exited { restore_rect }
        | FullscreenChange::FakeEntered { restore_rect } => {
            ctx.set_client_fullscreen_signal(
                win,
                matches!(transition.change, FullscreenChange::FakeEntered { .. }),
            );
            ctx.apply_exited_fullscreen_effects(win);
            if let Some(rect) = restore_rect {
                ctx.move_resize(win, rect, MoveResizeOptions::immediate());
            }
            arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
        }
        FullscreenChange::PolicyChanged => {
            // No protocol state or geometry changed, but the bar title badge
            // and IPC-visible mode did.
            ctx.core_mut()
                .bar
                .invalidate(crate::bar::BarChange::Content);
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
/// No-op when the window is missing or is not currently maximized. Explicit
/// move and placement operations use this as their single maximization exit
/// path so client protocol state cannot be left behind.
pub(crate) fn leave_maximized(ctx: &mut WmCtx<'_>, win: WindowId) {
    let Some(left) = ctx.core_mut().state.model.leave_maximized(win) else {
        return;
    };

    // Project transition geometry first. Native Wayland can then advertise
    // the cleared state and restored size in one final configure instead of
    // briefly advertising an unmaximized, maximized-sized window.
    apply_maximized_change(ctx, win, left.monitor_id, left.change);

    sync_owned_client_maximized_signal(ctx, left.monitor_id, win);
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

/// Toggle the per-window fake-fullscreen policy for the selected client.
///
/// The policy is independent of whether the application is currently
/// fullscreen:
///
/// * Windowed: the toggle arms/disarms the policy without sending any
///   fullscreen state to the application.
/// * Real fullscreen: turning the policy on demotes the window to active fake
///   fullscreen, restoring its normal layout geometry and border.
/// * Active fake fullscreen: turning the policy off promotes the window back
///   to real fullscreen, so the application stays fullscreen.
///
/// While the policy is armed, the next application fullscreen request is
/// granted as fake fullscreen instead of real fullscreen.
pub fn toggle_fake_fullscreen(ctx: &mut WmCtx<'_>) {
    if let Some(win) = ctx.core().state.model.selected_win() {
        toggle_client_fake_fullscreen(ctx, win);
    }
}

pub(crate) fn toggle_client_fake_fullscreen(ctx: &mut WmCtx<'_>, win: WindowId) {
    let Some(transition) = ctx.core_mut().state.model.toggle_fake_fullscreen(win) else {
        return;
    };
    apply_fullscreen_transition(ctx, win, transition);
}

#[cfg(test)]
mod tests {
    use super::toggle_fake_fullscreen;
    use crate::test_support::TestWm as Wm;

    use crate::test_support::MonitorBuilder;
    use crate::types::{Client, Rect, TagMask, WindowId};

    #[test]
    fn fake_fullscreen_policy_is_shared_by_wayland() {
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
        assert!(client.mode().is_fake_armed());
        assert!(!client.mode().is_fullscreen());
        assert_eq!(client.border_width, 3);

        crate::client::fullscreen::set_fullscreen(&mut wm.test_ctx(), win, true);
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_fake_fullscreen());
        assert_eq!(client.border_width, 3);

        crate::client::fullscreen::set_fullscreen(&mut wm.test_ctx(), win, false);
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_fake_armed());
        assert!(!client.mode().is_fullscreen());
        // The app is windowed again, so normal single-window tiling rules own
        // the border width.
        assert_eq!(client.border_width, 0);

        toggle_fake_fullscreen(&mut wm.test_ctx());
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_normal_tiling());
    }
}
