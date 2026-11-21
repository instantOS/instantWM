//! Backend-neutral pointer motion policy.
//!
//! X11 and Wayland resolve native hit testing differently, but the ordering
//! and meaning of compositor-owned hover targets is the same: bar, then
//! sidebar, then resize offer, then ordinary hover focus. Keeping that policy
//! here prevents the two adapters from drifting into separate state machines.
//!
//! Backends supply a normalized [`MotionPolicyInput`] and keep native
//! transport (X11 event replay, Smithay pointer focus, constraints, caching)
//! in their own modules. The returned [`MotionOutcome`] tells the adapter
//! which part of the pointer path owns the motion.

use crate::contexts::WmCtx;
use crate::types::{BarPosition, HoverFocusTrigger, Point, WindowId};

/// Where the pointer sits relative to window-manager-owned chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionBand {
    /// A visible window-manager bar owns this point.
    Bar,
    /// A foreground shell/menu surface is above the bar and owns this point.
    AboveBar,
    /// Ordinary desktop content.
    Content,
}

/// Normalized facts a backend must resolve before applying motion policy.
#[derive(Clone, Copy, Debug)]
pub struct MotionPolicyInput {
    /// Root-space pointer position.
    pub root: Point,
    /// Managed window under the pointer, as resolved by the backend's hit test.
    pub hovered: Option<WindowId>,
    /// Bar/above-bar/content classification.
    pub band: MotionBand,
    /// Why hover focus was recomputed.
    pub trigger: HoverFocusTrigger,
    /// Let bar hit-testing select the monitor under the pointer.
    pub sync_selected_monitor: bool,
    /// A tag/title-reorder capture owns the bar hover until release.
    pub bar_hover_blocked: bool,
    /// Any captured interaction owns ordinary hover ownership.
    pub capture_active: bool,
    /// A surface, window, or overlay already owns the point, so the sidebar
    /// must not arm a gesture.
    pub sidebar_blocked: bool,
    /// Keep the current focus (for example the guard strip below the bar).
    pub suppress_hover_focus: bool,
    /// The backend already selected the monitor under the pointer.
    pub monitor_already_selected: bool,
    /// Resize offers may move focus (X11's historical behavior).
    pub focus_resize_offer: bool,
    /// An exclusive layer owns focus; do not apply hover focus at all.
    pub preserve_focus: bool,
}

/// The compositor-owned target that won this motion sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionOutcome {
    /// The bar owns the pointer. Native motion may still be forwarded.
    Bar,
    /// A sidebar gesture owns the pointer. Focus must not change.
    Sidebar,
    /// A resize offer owns the pointer. Focus changes only if the backend
    /// opted into `focus_resize_offer`.
    Resize { win: WindowId },
    /// Normal desktop content; hover focus was already reconciled.
    Content,
    /// Native motion should be forwarded without changing focus.
    PreserveFocus,
}

/// Apply the shared pointer-motion policy.
///
/// This function mutates the shared WM hover state (bar hover, sidebar offer,
/// resize offer, hover focus) exactly once per motion sample. Backends call it
/// after native hit testing and before native pointer dispatch.
pub fn update_motion_policy(ctx: &mut WmCtx<'_>, input: MotionPolicyInput) -> MotionOutcome {
    // Foreground shell UI and menus own the whole band. They never arm a bar
    // hover, a resize offer, or a sidebar gesture, and they must not move
    // focus to a window behind the overlay.
    if input.band == MotionBand::AboveBar {
        crate::mouse::clear_hover_offer(ctx);
        if !input.bar_hover_blocked {
            crate::bar::clear_hover(ctx);
        }
        return MotionOutcome::PreserveFocus;
    }

    // Resolve bar ownership first. This also clears stale bar hover whenever
    // the pointer leaves the bar, so content motion does not need a separate
    // clear pass.
    let bar_position = if input.bar_hover_blocked {
        None
    } else {
        crate::bar::update_hover(ctx, input.root, false, input.sync_selected_monitor)
    };
    if !input.bar_hover_blocked
        && (input.band == MotionBand::Bar || bar_position.is_some())
        && !input.capture_active
    {
        crate::mouse::clear_hover_offer(ctx);
        if matches!(bar_position, Some(BarPosition::Root) | None) {
            crate::bar::clear_hover(ctx);
        }
        return MotionOutcome::Bar;
    }

    // Overview owns pointer semantics wholesale. A border-zone offer armed
    // before entering it must not keep borrowing the pointer.
    if ctx.core().state.model.is_overview_active() {
        crate::mouse::clear_hover_offer(ctx);
        if !input.monitor_already_selected && !input.preserve_focus {
            crate::focus::apply_hover_focus(
                ctx,
                input.hovered,
                false,
                Some(input.root),
                input.trigger,
            );
        }
        return MotionOutcome::Content;
    }

    if !input.capture_active {
        let target = if input.sidebar_blocked {
            None
        } else {
            crate::mouse::pointer::sidebar_target_at(&ctx.core().state.model, input.root)
        };
        if crate::mouse::set_sidebar_offer(ctx, target).affects_pointer_handling() {
            return MotionOutcome::Sidebar;
        }
    }

    if !input.capture_active
        && let Some(win) = crate::mouse::hover::update_resize_offer_at(
            ctx,
            input.root,
            input.hovered,
            input.trigger,
        )
    {
        if input.focus_resize_offer {
            crate::mouse::hover::focus_resize_offer(ctx, win);
        }
        return MotionOutcome::Resize { win };
    }

    if !input.monitor_already_selected && !input.suppress_hover_focus && !input.preserve_focus {
        crate::focus::apply_hover_focus(ctx, input.hovered, false, Some(input.root), input.trigger);
    }

    MotionOutcome::Content
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{MonitorBuilder, TestWm, add_client_with, add_selected_client_with};
    use crate::types::{ClientMode, Rect, TagMask};

    fn wm_with_monitor(rect: Rect) -> (TestWm, crate::types::MonitorId, TagMask) {
        let tags = TagMask::single(1).unwrap();
        let mut wm = TestWm::new(crate::backend::WaylandBackendData::default());
        let monitor = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(rect)
                .bar(20, true)
                .tag_count(1)
                .selected_tags(tags)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor);
        (wm, monitor, tags)
    }

    fn input(root: Point, band: MotionBand) -> MotionPolicyInput {
        MotionPolicyInput {
            root,
            hovered: None,
            band,
            trigger: HoverFocusTrigger::PointerMotion,
            sync_selected_monitor: false,
            bar_hover_blocked: false,
            capture_active: false,
            sidebar_blocked: false,
            suppress_hover_focus: false,
            monitor_already_selected: false,
            focus_resize_offer: false,
            preserve_focus: false,
        }
    }

    #[test]
    fn bar_band_owns_the_pointer_and_updates_bar_hover() {
        let (mut wm, monitor, _) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        crate::bar::render_hit_caches_for_test(&mut wm.core);

        let outcome = update_motion_policy(
            &mut wm.test_ctx(),
            input(Point::new(4, 10), MotionBand::Bar),
        );

        assert_eq!(outcome, MotionOutcome::Bar);
        assert_eq!(wm.core.bar.hover.monitor_id, Some(monitor));
    }

    #[test]
    fn above_bar_preserves_focus_and_clears_stale_offers() {
        let (mut wm, _, _) = wm_with_monitor(Rect::new(0, 0, 1200, 800));

        let outcome = update_motion_policy(
            &mut wm.test_ctx(),
            input(Point::new(500, 10), MotionBand::AboveBar),
        );

        assert_eq!(outcome, MotionOutcome::PreserveFocus);
        assert_eq!(wm.core.bar.hover, Default::default());
    }

    #[test]
    fn sidebar_owns_the_edge_when_no_surface_is_under_the_pointer() {
        let (mut wm, _, _) = wm_with_monitor(Rect::new(0, 0, 1200, 800));

        let outcome = update_motion_policy(
            &mut wm.test_ctx(),
            input(Point::new(1195, 400), MotionBand::Content),
        );

        assert_eq!(outcome, MotionOutcome::Sidebar);
    }

    #[test]
    fn sidebar_is_blocked_by_content_under_the_pointer() {
        let (mut wm, _, _) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        let mut policy = input(Point::new(1195, 400), MotionBand::Content);
        policy.sidebar_blocked = true;

        let outcome = update_motion_policy(&mut wm.test_ctx(), policy);

        assert_eq!(outcome, MotionOutcome::Content);
    }

    #[test]
    fn resize_offer_owns_the_floating_border() {
        let (mut wm, monitor, tags) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        add_selected_client_with(&mut wm.core.state.model, monitor, |client| {
            client.win = WindowId(1);
            client.tags = tags;
            client.geo = Rect::new(0, 0, 1200, 800);
            client.mode = ClientMode::tiled();
        });
        let floating = add_client_with(&mut wm.core.state.model, monitor, |client| {
            client.win = WindowId(2);
            client.tags = tags;
            client.geo = Rect::new(100, 100, 600, 400);
            client.mode = ClientMode::floating();
        });
        let mut inside = input(Point::new(690, 270), MotionBand::Content);
        inside.hovered = Some(floating);
        let _ = update_motion_policy(&mut wm.test_ctx(), inside);

        let mut border = input(Point::new(710, 270), MotionBand::Content);
        border.hovered = Some(floating);
        let outcome = update_motion_policy(&mut wm.test_ctx(), border);

        assert_eq!(outcome, MotionOutcome::Resize { win: floating });
    }

    #[test]
    fn captured_interaction_keeps_hover_ownership() {
        let (mut wm, monitor, tags) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        let win = add_selected_client_with(&mut wm.core.state.model, monitor, |client| {
            client.win = WindowId(8);
            client.tags = tags;
            client.geo = Rect::new(0, 0, 1200, 800);
            client.mode = ClientMode::tiled();
        });
        wm.core
            .state
            .interaction
            .drag
            .arm_title_drag(crate::core_state::ArmedDragStart {
                win,
                button: crate::types::MouseButton::Left,
                origin: crate::core_state::ArmedDragOrigin::Client,
                source: crate::types::InteractionSource::Pointer,
                start: Point::new(600, 400),
                restore_geometry: Rect::new(0, 0, 1200, 800),
                was_focused: true,
                was_hidden: false,
                suppress_click_action: false,
            })
            .unwrap();
        let mut policy = input(Point::new(50, 10), MotionBand::Bar);
        policy.capture_active = true;

        let outcome = update_motion_policy(&mut wm.test_ctx(), policy);

        assert_ne!(outcome, MotionOutcome::Bar);
        assert!(wm.core.state.interaction.drag.has_capture());
    }

    #[test]
    fn overview_does_not_arm_sidebar_or_resize_targets() {
        let (mut wm, monitor, tags) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        wm.core
            .state
            .model
            .monitor_mut(monitor)
            .unwrap()
            .overview_state = Some(crate::overview::OverviewState::new(
            tags,
            Vec::new(),
            Default::default(),
            None,
        ));
        let mut policy = input(Point::new(1195, 400), MotionBand::Content);
        policy.hovered = Some(WindowId(9));

        let outcome = update_motion_policy(&mut wm.test_ctx(), policy);

        assert_eq!(outcome, MotionOutcome::Content);
        assert!(!wm.core.state.interaction.drag.hover_offer().is_sidebar());
    }

    #[test]
    fn monitor_already_selected_keeps_focus() {
        let (mut wm, first, tags) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        let second = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(1200, 0, 1200, 800))
                .bar(20, true)
                .tag_count(1)
                .selected_tags(tags)
                .build(),
        );
        let win = add_selected_client_with(&mut wm.core.state.model, second, |client| {
            client.win = WindowId(9);
            client.tags = tags;
            client.geo = Rect::new(1300, 100, 400, 300);
            client.mode = ClientMode::floating();
        });
        let mut policy = input(Point::new(1300, 200), MotionBand::Content);
        policy.hovered = Some(win);
        policy.monitor_already_selected = true;

        let outcome = update_motion_policy(&mut wm.test_ctx(), policy);

        assert_eq!(outcome, MotionOutcome::Content);
        assert_eq!(wm.core.state.model.selected_monitor_id(), first);
    }

    #[test]
    fn preserve_focus_keeps_the_current_selection() {
        let (mut wm, first, tags) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        let second = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(1200, 0, 1200, 800))
                .bar(20, true)
                .tag_count(1)
                .selected_tags(tags)
                .build(),
        );
        let win = add_selected_client_with(&mut wm.core.state.model, second, |client| {
            client.win = WindowId(11);
            client.tags = tags;
            client.geo = Rect::new(1300, 100, 400, 300);
            client.mode = ClientMode::floating();
        });
        let mut policy = input(Point::new(1300, 200), MotionBand::Content);
        policy.hovered = Some(win);
        policy.preserve_focus = true;

        let outcome = update_motion_policy(&mut wm.test_ctx(), policy);

        assert_eq!(outcome, MotionOutcome::Content);
        assert_eq!(wm.core.state.model.selected_monitor_id(), first);
    }

    #[test]
    fn x11_style_resize_offer_can_move_focus() {
        let (mut wm, monitor, tags) = wm_with_monitor(Rect::new(0, 0, 1200, 800));
        add_selected_client_with(&mut wm.core.state.model, monitor, |client| {
            client.win = WindowId(1);
            client.tags = tags;
            client.geo = Rect::new(0, 0, 1200, 800);
            client.mode = ClientMode::tiled();
        });
        let floating = add_client_with(&mut wm.core.state.model, monitor, |client| {
            client.win = WindowId(2);
            client.tags = tags;
            client.geo = Rect::new(100, 100, 600, 400);
            client.mode = ClientMode::floating();
        });
        let mut inside = input(Point::new(690, 270), MotionBand::Content);
        inside.hovered = Some(floating);
        let _ = update_motion_policy(&mut wm.test_ctx(), inside);

        let mut border = input(Point::new(710, 270), MotionBand::Content);
        border.hovered = Some(floating);
        border.focus_resize_offer = true;
        let outcome = update_motion_policy(&mut wm.test_ctx(), border);

        assert_eq!(outcome, MotionOutcome::Resize { win: floating });
        assert_eq!(wm.core.state.model.selected_win(), Some(floating));
    }
}
