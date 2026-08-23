//! Shared transport for compositor-owned pointer and touch interactions.
//!
//! Backends translate native input into [`InteractionEvent`]. This module is
//! the only place that decides which armed/active WM interaction receives an
//! update, release, or cancellation.

use crate::contexts::WmCtx;
use crate::core_state::{CapturedInteraction, DragCancelReason, WindowDragState};
use crate::types::{MouseButton, Point, SidebarTarget};

pub use crate::types::InteractionSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionPhase {
    Update,
    End { button: MouseButton, time_msec: u32 },
    Cancel { reason: DragCancelReason },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteractionEvent {
    pub source: InteractionSource,
    pub phase: InteractionPhase,
    pub root: Point,
    pub modifiers: u32,
    /// Sidebar offer to restore after release, already resolved by the input
    /// adapter after accounting for higher-priority compositor UI.
    pub sidebar_hover: Option<SidebarTarget>,
}

impl InteractionEvent {
    pub fn pointer_update(root: Point, modifiers: u32) -> Self {
        Self {
            source: InteractionSource::Pointer,
            phase: InteractionPhase::Update,
            root,
            modifiers,
            sidebar_hover: None,
        }
    }

    pub fn pointer_end(
        root: Point,
        button: MouseButton,
        modifiers: u32,
        sidebar_hover: Option<SidebarTarget>,
        time_msec: u32,
    ) -> Self {
        Self {
            source: InteractionSource::Pointer,
            phase: InteractionPhase::End { button, time_msec },
            root,
            modifiers,
            sidebar_hover,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionOutcome {
    Ignored,
    Captured,
}

impl InteractionOutcome {
    pub fn captured(self) -> bool {
        matches!(self, Self::Captured)
    }
}

pub fn handle(ctx: &mut WmCtx<'_>, event: InteractionEvent) -> InteractionOutcome {
    if !matches!(event.phase, InteractionPhase::Cancel { .. })
        && ctx.core().interaction().drag.captured_source() != Some(event.source)
    {
        return InteractionOutcome::Ignored;
    }
    match event.phase {
        InteractionPhase::Update => update(ctx, event),
        InteractionPhase::End { button, time_msec } => finish(ctx, event, button, time_msec),
        InteractionPhase::Cancel { reason } => cancel(ctx, reason),
    }
}

fn update(ctx: &mut WmCtx<'_>, event: InteractionEvent) -> InteractionOutcome {
    match ctx.core().interaction().drag.capture() {
        Some(CapturedInteraction::OverviewCard(_)) => {
            let _ = crate::overview::update_card_gesture(ctx, event.root);
        }
        Some(CapturedInteraction::Window(WindowDragState::Active(_))) => {
            let _ = crate::mouse::drag::apply_active_drag_motion(ctx, event.root);
        }
        Some(CapturedInteraction::Window(WindowDragState::Armed(_))) => {
            let _ = crate::mouse::process_title_drag_motion(
                ctx,
                match event.source {
                    InteractionSource::Pointer => crate::mouse::DragInput::Pointer(event.root),
                    InteractionSource::Touch(_) => crate::mouse::DragInput::Absolute(event.root),
                },
            );
        }
        Some(CapturedInteraction::Window(WindowDragState::Reordering(..))) => {
            let _ = crate::mouse::process_title_reorder_motion(ctx, event.root);
        }
        Some(CapturedInteraction::Tag(_)) => {
            ctx.core_mut()
                .interaction_mut().drag
                .tag_drag_mut()
                .expect("tag capture remained active")
                .last_motion = Some((event.root, event.modifiers));
            let _ = crate::mouse::apply_drag_tag_motion(ctx, event.root);
        }
        Some(CapturedInteraction::SidebarVolume(_)) => {
            crate::mouse::update_sidebar_gesture(ctx, event.root.y);
        }
        Some(CapturedInteraction::BottomBar(_)) => {
            crate::mouse::update_bottom_bar_gesture(ctx, event.root);
        }
        None => return InteractionOutcome::Ignored,
    }
    InteractionOutcome::Captured
}

fn finish(
    ctx: &mut WmCtx<'_>,
    event: InteractionEvent,
    button: MouseButton,
    time_msec: u32,
) -> InteractionOutcome {
    if ctx.core().interaction().drag.captured_button() != Some(button) {
        return InteractionOutcome::Ignored;
    }
    match ctx.core().interaction().drag.capture() {
        Some(CapturedInteraction::OverviewCard(_)) => {
            let _ = crate::overview::finish_card_gesture(ctx, button);
        }
        Some(CapturedInteraction::Window(WindowDragState::Active(_))) => {
            let _ = crate::mouse::drag::active_drag_finish(ctx, button, event.modifiers);
        }
        Some(CapturedInteraction::Window(WindowDragState::Armed(_))) => {
            crate::mouse::title_drag_finish(ctx);
        }
        Some(CapturedInteraction::Window(WindowDragState::Reordering(..))) => {
            crate::mouse::title_reorder_finish(ctx);
        }
        Some(CapturedInteraction::Tag(_)) => {
            crate::mouse::drag_tag_finish(ctx, event.modifiers);
        }
        Some(CapturedInteraction::SidebarVolume(_)) => {
            let _ = crate::mouse::sidebar_gesture_finish(ctx, button, event.sidebar_hover);
        }
        Some(CapturedInteraction::BottomBar(_)) => {
            let _ = crate::mouse::bottom_bar_gesture_finish(ctx, button, event.root, time_msec);
        }
        None => return InteractionOutcome::Ignored,
    }
    InteractionOutcome::Captured
}

fn cancel(ctx: &mut WmCtx<'_>, reason: DragCancelReason) -> InteractionOutcome {
    let cancelled_interactive = match ctx {
        WmCtx::X11(x11) => {
            crate::mouse::drag::lifecycle::cancel(&mut x11.core.interaction_mut().drag, &x11.x11, reason)
        }
        WmCtx::Wayland(wayland) => crate::mouse::drag::lifecycle::cancel(&mut wayland.core.interaction_mut().drag,
            wayland.wayland,
            reason,
        ),
    }
    .is_some();
    let cancelled_other = ctx.core_mut().interaction_mut().drag.cancel_capture().is_some();
    if cancelled_interactive || cancelled_other {
        ctx.core_mut().bar.hover.clear();
        ctx.set_cursor_style(crate::types::AltCursor::Default);
        ctx.update_layout_preview(None);
        ctx.request_bar_update();
        InteractionOutcome::Captured
    } else {
        InteractionOutcome::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, ClientMode, Monitor, MonitorId, Rect, TagMask, WindowId};
    use crate::wm::Wm;

    fn floating_drag_fixture(source: InteractionSource) -> (Wm, WindowId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let win = WindowId(7);
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            show_bar: false,
            ..Monitor::default()
        });
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::floating(),
            geo: Rect::new(100, 100, 500, 300),
            ..Client::default()
        });
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = vec![win];
        monitor.selected = Some(win);
        wm.core.interaction.drag
            .begin_move(
                win,
                MouseButton::Left,
                source,
                Point::new(150, 150),
                Rect::new(100, 100, 500, 300),
            )
            .unwrap();
        (wm, win)
    }

    fn update(source: InteractionSource, root: Point) -> InteractionEvent {
        InteractionEvent {
            source,
            phase: InteractionPhase::Update,
            root,
            modifiers: 0,
            sidebar_hover: None,
        }
    }

    #[test]
    fn pointer_and_touch_apply_identical_active_drag_motion() {
        let (mut pointer_wm, win) = floating_drag_fixture(InteractionSource::Pointer);
        let (mut touch_wm, _) = floating_drag_fixture(InteractionSource::Touch(4));
        let root = Point::new(350, 275);

        assert_eq!(
            handle(
                &mut pointer_wm.ctx(),
                update(InteractionSource::Pointer, root)
            ),
            InteractionOutcome::Captured
        );
        assert_eq!(
            handle(
                &mut touch_wm.ctx(),
                update(InteractionSource::Touch(4), root)
            ),
            InteractionOutcome::Captured
        );

        assert_eq!(
            pointer_wm.core.model.client(win).unwrap().geo,
            touch_wm.core.model.client(win).unwrap().geo
        );
        assert_eq!(
            pointer_wm
                .core.interaction.drag
                .active_interaction()
                .unwrap()
                .last_root_point(),
            touch_wm
                .core.interaction.drag
                .active_interaction()
                .unwrap()
                .last_root_point()
        );
    }

    #[test]
    fn uncaptured_samples_are_explicitly_ignored_for_client_forwarding() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Pointer, Point::new(10, 20))
            ),
            InteractionOutcome::Ignored
        );
        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Touch(2), Point::new(10, 20))
            ),
            InteractionOutcome::Ignored
        );
    }

    fn bottom_bar_fixture() -> (Wm, MonitorId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 9;
        let tags = TagMask::single(2).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            show_bar: true,
            show_bottom_bar: true,
            bottom_bar_height: 30,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        // Add a client so overview-style actions can activate.
        let win = WindowId(7);
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            geo: Rect::new(100, 100, 500, 300),
            ..Client::default()
        });
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = vec![win];
        monitor.selected = Some(win);
        (wm, monitor_id)
    }

    fn begin_bottom_bar_drag(wm: &mut Wm, monitor_id: MonitorId, root: Point) {
        let actions = crate::core_state::BottomBarActions {
            left: Box::new(crate::actions::ButtonAction::named(
                crate::actions::NamedAction::ScrollLeft,
            )),
            right: Box::new(crate::actions::ButtonAction::named(
                crate::actions::NamedAction::ScrollRight,
            )),
            up: Box::new(crate::actions::ButtonAction::named(
                crate::actions::NamedAction::ToggleOverview,
            )),
            click: Box::new(crate::actions::ButtonAction::named(
                crate::actions::NamedAction::CancelOverview,
            )),
            hold: Box::new(crate::actions::ButtonAction::named(
                crate::actions::NamedAction::CancelOverview,
            )),
        };
        assert!(crate::mouse::drag::bottom_bar_gesture_begin(
            &mut wm.ctx(),
            MouseButton::Left,
            InteractionSource::Pointer,
            monitor_id,
            root,
            0,
            actions,
        ));
    }

    fn end_bottom_bar_drag(wm: &mut Wm, root: Point) {
        end_bottom_bar_drag_at(wm, root, 0);
    }

    fn end_bottom_bar_drag_at(wm: &mut Wm, root: Point, time_msec: u32) {
        assert_eq!(
            handle(
                &mut wm.ctx(),
                InteractionEvent {
                    source: InteractionSource::Pointer,
                    phase: InteractionPhase::End {
                        button: MouseButton::Left,
                        time_msec,
                    },
                    root,
                    modifiers: 0,
                    sidebar_hover: None,
                }
            ),
            InteractionOutcome::Captured
        );
        assert!(!wm.core.interaction.drag.bottom_bar_gesture_active());
    }

    #[test]
    fn bottom_bar_swipe_switches_exactly_one_adjacent_tag_on_release() {
        let (mut wm, monitor_id) = bottom_bar_fixture();
        let begin_root = Point::new(100, 1060);
        assert_eq!(
            crate::mouse::pointer::bottom_bar_monitor_at(&wm.core.model, begin_root),
            Some(monitor_id)
        );

        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);

        // Threshold is 1920 / 30 = 64. Crossing it latches right; motion alone
        // must not change the view (action fires on release).
        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Pointer, Point::new(164, 1060))
            ),
            InteractionOutcome::Captured
        );
        // Dragging far beyond the threshold still produces only one action.
        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Pointer, Point::new(1500, 1060))
            ),
            InteractionOutcome::Captured
        );
        assert_eq!(
            wm.core.model.monitor(monitor_id).unwrap().selected_tags(),
            TagMask::single(2).unwrap()
        );

        // Release fires the right (next-tag) action exactly once: tag 2 -> 3.
        end_bottom_bar_drag(&mut wm, Point::new(1500, 1060));
        assert_eq!(
            wm.core.model.monitor(monitor_id).unwrap().selected_tags(),
            TagMask::single(3).unwrap()
        );

        // A left swipe switches back exactly one tag: tag 3 -> 2.
        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);
        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Pointer, Point::new(36, 1060))
            ),
            InteractionOutcome::Captured
        );
        end_bottom_bar_drag(&mut wm, Point::new(36, 1060));
        assert_eq!(
            wm.core.model.monitor(monitor_id).unwrap().selected_tags(),
            TagMask::single(2).unwrap()
        );

        // A plain click (release without crossing the threshold) does nothing.
        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);
        end_bottom_bar_drag(&mut wm, begin_root);
        assert_eq!(
            wm.core.model.monitor(monitor_id).unwrap().selected_tags(),
            TagMask::single(2).unwrap()
        );
    }

    #[test]
    fn bottom_bar_click_and_hold_fire_on_release_without_a_swipe() {
        let (mut wm, monitor_id) = bottom_bar_fixture();
        let begin_root = Point::new(100, 1060);

        // A quick release (no movement, short duration) fires `click` and
        // completes the gesture.
        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);
        end_bottom_bar_drag_at(&mut wm, begin_root, 0);
        assert!(!wm.core.interaction.drag.bottom_bar_gesture_active());

        // A long hold (no movement, duration >= 400ms) fires `hold`.
        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);
        end_bottom_bar_drag_at(&mut wm, begin_root, 500);
        assert!(!wm.core.interaction.drag.bottom_bar_gesture_active());

        // A swipe still takes precedence over click/hold regardless of duration:
        // even after holding 600ms, the latched direction wins.
        let tags_before = wm.core.model.monitor(monitor_id).unwrap().selected_tags();
        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);
        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Pointer, Point::new(164, 1060))
            ),
            InteractionOutcome::Captured
        );
        end_bottom_bar_drag_at(&mut wm, Point::new(164, 1060), 600);
        // The right-swipe action (ScrollRight) advanced exactly one tag.
        assert_ne!(
            wm.core.model.monitor(monitor_id).unwrap().selected_tags(),
            tags_before
        );
    }

    #[test]
    fn bottom_bar_up_swipe_leaves_the_strip_and_triggers_once_on_release() {
        let (mut wm, monitor_id) = bottom_bar_fixture();
        let begin_root = Point::new(100, 1060);

        begin_bottom_bar_drag(&mut wm, monitor_id, begin_root);

        // Drag up well past the threshold and off the strip into the desktop.
        // Motion alone must not fire anything (overview stays inactive).
        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Pointer, Point::new(100, 500))
            ),
            InteractionOutcome::Captured
        );
        assert!(!wm.core.model.is_overview_active());

        // Release fires the up (overview) action exactly once.
        end_bottom_bar_drag(&mut wm, Point::new(100, 500));
        assert!(wm.core.model.is_overview_active());
    }

    #[test]
    fn another_input_source_cannot_move_or_release_a_captured_interaction() {
        let (mut wm, win) = floating_drag_fixture(InteractionSource::Pointer);
        let original = wm.core.model.client(win).unwrap().geo;

        assert_eq!(
            handle(
                &mut wm.ctx(),
                update(InteractionSource::Touch(9), Point::new(700, 600)),
            ),
            InteractionOutcome::Ignored
        );
        assert_eq!(wm.core.model.client(win).unwrap().geo, original);

        assert_eq!(
            handle(
                &mut wm.ctx(),
                InteractionEvent {
                    source: InteractionSource::Touch(9),
                    phase: InteractionPhase::End {
                        button: MouseButton::Left,
                        time_msec: 0,
                    },
                    root: Point::new(700, 600),
                    modifiers: 0,
                    sidebar_hover: None,
                },
            ),
            InteractionOutcome::Ignored
        );
        assert!(wm.core.interaction.drag.active_interaction().is_some());
    }
}
