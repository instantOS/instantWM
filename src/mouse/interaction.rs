//! Shared transport for compositor-owned pointer and touch interactions.
//!
//! Backends translate native input into [`InteractionEvent`]. This module is
//! the only place that decides which armed/active WM interaction receives an
//! update, release, or cancellation.

use crate::contexts::WmCtx;
use crate::core_state::DragCancelReason;
use crate::types::{MouseButton, Point, SidebarTarget};

pub use crate::types::InteractionSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionPhase {
    Update,
    End { button: MouseButton },
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
    ) -> Self {
        Self {
            source: InteractionSource::Pointer,
            phase: InteractionPhase::End { button },
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
        && ctx.core().drag_state().captured_source() != Some(event.source)
    {
        return InteractionOutcome::Ignored;
    }
    match event.phase {
        InteractionPhase::Update => update(ctx, event),
        InteractionPhase::End { button } => finish(ctx, event, button),
        InteractionPhase::Cancel { reason } => cancel(ctx, reason),
    }
}

fn update(ctx: &mut WmCtx<'_>, event: InteractionEvent) -> InteractionOutcome {
    if ctx.core().drag_state().active_interaction().is_some() {
        return if crate::mouse::drag::active_drag_motion(ctx, event.root) {
            InteractionOutcome::Captured
        } else {
            InteractionOutcome::Ignored
        };
    }
    if ctx.core().drag_state().tag.active {
        let drag = &mut ctx.core_mut().drag_state_mut().tag;
        drag.last_motion = Some((event.root, event.modifiers));
        let _ = crate::mouse::drag_tag_motion(ctx, event.root);
        return InteractionOutcome::Captured;
    }
    if ctx.core().drag_state().armed_interaction().is_some() {
        let _ = crate::mouse::title_drag_motion(
            ctx,
            match event.source {
                InteractionSource::Pointer => crate::mouse::DragInput::Pointer(event.root),
                InteractionSource::Touch(_) => crate::mouse::DragInput::Absolute(event.root),
            },
        );
        return InteractionOutcome::Captured;
    }
    if ctx.core().drag_state().sidebar_volume_active() {
        crate::mouse::update_sidebar_gesture(ctx, event.root.y);
        return InteractionOutcome::Captured;
    }
    InteractionOutcome::Ignored
}

fn finish(ctx: &mut WmCtx<'_>, event: InteractionEvent, button: MouseButton) -> InteractionOutcome {
    if crate::mouse::drag::active_drag_finish(ctx, button, event.modifiers) {
        return InteractionOutcome::Captured;
    }
    if ctx.core().drag_state().tag.active && ctx.core().drag_state().tag.button == button {
        crate::mouse::drag_tag_finish(ctx, event.modifiers);
        return InteractionOutcome::Captured;
    }
    if ctx
        .core()
        .drag_state()
        .armed_interaction()
        .is_some_and(|drag| drag.button() == button)
    {
        crate::mouse::title_drag_finish(ctx);
        return InteractionOutcome::Captured;
    }
    if crate::mouse::finish_sidebar_gesture(ctx, button, event.sidebar_hover) {
        return InteractionOutcome::Captured;
    }
    InteractionOutcome::Ignored
}

fn cancel(ctx: &mut WmCtx<'_>, reason: DragCancelReason) -> InteractionOutcome {
    let cancelled_interactive = match ctx {
        WmCtx::X11(x11) => {
            crate::mouse::drag::lifecycle::cancel(x11.core.drag_state_mut(), &x11.x11, reason)
        }
        WmCtx::Wayland(wayland) => crate::mouse::drag::lifecycle::cancel(
            wayland.core.drag_state_mut(),
            wayland.wayland,
            reason,
        ),
    }
    .is_some();
    let cancelled_tag = ctx.core().drag_state().tag.active;
    let cancelled_sidebar = ctx.core_mut().drag_state_mut().cancel_sidebar_volume();
    ctx.core_mut().drag_state_mut().tag = Default::default();
    if cancelled_interactive || cancelled_tag || cancelled_sidebar {
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
    use crate::types::{Client, ClientMode, Monitor, Rect, TagMask, WindowId};
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
        wm.core
            .drag
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
                .core
                .drag
                .active_interaction()
                .unwrap()
                .last_root_point(),
            touch_wm
                .core
                .drag
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
                    },
                    root: Point::new(700, 600),
                    modifiers: 0,
                    sidebar_hover: None,
                },
            ),
            InteractionOutcome::Ignored
        );
        assert!(wm.core.drag.active_interaction().is_some());
    }
}
