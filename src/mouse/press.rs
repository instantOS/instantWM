//! Shared button press policy dispatcher.
//!
//! Coordinates cross-backend press sequencing across X11, Wayland, and Touch:
//! monitor selection -> region classification -> overview card gesture ->
//! bar widgets and bottom-bar gestures -> edge sidebar gestures -> hover-resize ->
//! focus, raise, & overview exit -> configured binding dispatch -> fallthrough/replay.

use crate::contexts::WmCtx;
use crate::types::{BarPosition, ButtonTarget, InteractionSource, MouseButton, Point, WindowId};

/// Normalized button press input from a backend or touch adapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressInput {
    pub root: Point,
    pub button: Option<MouseButton>,
    pub raw_button: u8,
    pub modifiers: u32,
    pub clicked_window: Option<WindowId>,
    pub source: InteractionSource,
    pub time_msec: u32,
}

/// The outcome of evaluating the press policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressOutcome {
    /// A WM interaction or gesture was started (e.g. overview card drag,
    /// sidebar gesture, bottom bar swipe, hover resize/move drag, or a button binding
    /// that initiated an interaction).
    CapturedInteraction { button: MouseButton },
    /// The press was consumed by the window manager (e.g. bar widget, status text click,
    /// systray activation, or a configured binding without drag capture).
    Consumed,
    /// The shared hit policy selected a tray icon. Backends execute this
    /// effect so native menu toggle state can be resolved at the boundary.
    SystrayIconPress {
        index: usize,
        button: MouseButton,
        root: Point,
    },
    /// The press was processed for focus/raise, but no WM binding/gesture consumed it.
    /// It should be forwarded or replayed to the underlying client window.
    ReplayToClient { window: Option<WindowId> },
}

/// Dispatch a normalized button press through the unified window manager policy pipeline.
pub fn dispatch_press_policy(ctx: &mut WmCtx<'_>, input: PressInput) -> PressOutcome {
    // 1. Monitor Selection:
    // First select the monitor containing the press coordinates.
    if let Some(monitor_id) = ctx
        .core()
        .model()
        .monitors
        .id_intersecting_rect(crate::mouse::pointer::point_rect(input.root))
    {
        crate::focus::select_monitor(ctx, monitor_id);
    }
    // If clicking a client window, select the monitor that client belongs to.
    if let Some(win) = input.clicked_window {
        crate::focus::select_monitor_for_client(ctx, win);
    }

    // 2. Pointer Region Classification:
    let region =
        crate::mouse::pointer::button_region_at(ctx.core_mut(), input.root, input.clicked_window);

    // 3. Overview Card Gesture:
    if let (crate::mouse::pointer::PointerRegion::Client(window), Some(MouseButton::Left)) =
        (region, input.button)
        && ctx.core().model().is_overview_active()
        && crate::overview::begin_card_gesture(
            ctx,
            window,
            MouseButton::Left,
            input.source,
            input.root,
        )
    {
        return PressOutcome::CapturedInteraction {
            button: MouseButton::Left,
        };
    }

    // 4. Bar and BottomBar handling:
    match region {
        crate::mouse::pointer::PointerRegion::Bar { pos, .. } => {
            if let BarPosition::SystrayMenuItem(idx) = pos {
                if input.button == Some(MouseButton::Left) {
                    crate::systray::activate_menu_entry(ctx.core_mut(), idx);
                }
                return PressOutcome::Consumed;
            }

            crate::systray::close_menu(ctx.core_mut());

            if let BarPosition::SystrayItem(idx) = pos {
                if let Some(btn) = input.button {
                    return PressOutcome::SystrayIconPress {
                        index: idx,
                        button: btn,
                        root: input.root,
                    };
                }
                return PressOutcome::Consumed;
            }

            if pos == BarPosition::StatusText {
                crate::bar::handle_status_text_click(
                    ctx,
                    input.root,
                    input.raw_button,
                    input.modifiers,
                );
                return PressOutcome::Consumed;
            }

            if let Some(btn) = input.button {
                let numlockmask = ctx.numlock_mask();
                crate::mouse::bindings::run_first_matching(
                    ctx,
                    crate::mouse::bindings::ButtonBindingEvent {
                        target: ButtonTarget::Bar(pos),
                        window: None,
                        button: btn,
                        source: input.source,
                        root: input.root,
                        clean_state: input.modifiers,
                        time_msec: input.time_msec,
                    },
                    numlockmask,
                );
            }
            return PressOutcome::Consumed;
        }
        crate::mouse::pointer::PointerRegion::BottomBar { .. } => {
            if let Some(btn) = input.button {
                let numlockmask = ctx.numlock_mask();
                crate::mouse::bindings::run_first_matching(
                    ctx,
                    crate::mouse::bindings::ButtonBindingEvent {
                        target: ButtonTarget::BottomBar,
                        window: None,
                        button: btn,
                        source: input.source,
                        root: input.root,
                        clean_state: input.modifiers,
                        time_msec: input.time_msec,
                    },
                    numlockmask,
                );
                if ctx.core().interaction().drag.captured_source() == Some(input.source) {
                    return PressOutcome::CapturedInteraction { button: btn };
                }
            }
            return PressOutcome::Consumed;
        }
        crate::mouse::pointer::PointerRegion::Client(_)
        | crate::mouse::pointer::PointerRegion::Root { .. } => {}
    }

    // 5. Sidebar Gesture (on Root):
    if matches!(region, crate::mouse::pointer::PointerRegion::Root { .. })
        && input.modifiers == 0
        && let Some(btn @ MouseButton::Left) = input.button
        && let Some(target) =
            crate::mouse::pointer::sidebar_target_at(ctx.core().model(), input.root)
        && crate::mouse::sidebar_gesture_begin(ctx, btn, input.source, target, input.root)
    {
        return PressOutcome::CapturedInteraction { button: btn };
    }

    // 6. Hover Resize / Decoration Border Gesture:
    if input.source == InteractionSource::Pointer
        && input.modifiers == 0
        && let Some(btn) = input.button
        && crate::mouse::drag::hover_drag_begin(ctx, input.root, btn, input.source)
    {
        if ctx.core().interaction().drag.captured_source() == Some(input.source) {
            return PressOutcome::CapturedInteraction { button: btn };
        } else {
            return PressOutcome::Consumed;
        }
    }

    // 7. Focus, Raise & Overview Dismissal:
    if input.button == Some(MouseButton::Left) {
        let exit_mode = if input.clicked_window.is_some() {
            crate::overview::ExitMode::ToSelectedWindow
        } else {
            crate::overview::ExitMode::RestorePrevious
        };
        crate::overview::exit_overview(ctx, exit_mode);
    }

    if let Some(win) = input.clicked_window {
        if ctx.core().model().selected_win() != Some(win) {
            crate::focus::focus(ctx, Some(win));
        }
        if let Some(btn) = input.button {
            crate::focus::raise_floating_on_client_click(ctx, win, btn);
        }
    } else {
        crate::focus::focus(ctx, None);
    }

    // 8. Binding Dispatch / Replay decision:
    let mut binding_matched = false;
    // Touch contacts participate only in explicitly touch-capable regions
    // handled above (overview, sidebar, bar, and bottom bar). Root/client
    // button bindings are pointer chords and must not steal native touch.
    if input.source == InteractionSource::Pointer
        && let (Some(target), Some(btn)) = (region.binding_target(), input.button)
    {
        let numlockmask = ctx.numlock_mask();
        binding_matched = crate::mouse::bindings::run_first_matching(
            ctx,
            crate::mouse::bindings::ButtonBindingEvent {
                target,
                window: input.clicked_window,
                button: btn,
                source: input.source,
                root: input.root,
                clean_state: input.modifiers,
                time_msec: input.time_msec,
            },
            numlockmask,
        );
        if ctx.core().interaction().drag.captured_source() == Some(input.source) {
            return PressOutcome::CapturedInteraction { button: btn };
        }
    }

    if binding_matched {
        PressOutcome::Consumed
    } else if matches!(region, crate::mouse::pointer::PointerRegion::Client(_)) {
        PressOutcome::ReplayToClient {
            window: input.clicked_window,
        }
    } else {
        PressOutcome::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{ButtonAction, NamedAction};
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Button, Client, ClientMode, Monitor, MonitorId, Rect, TagMask, WindowId};
    use crate::wm::Wm;

    fn setup_wm() -> (Wm, WindowId, MonitorId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 4;
        let tags = TagMask::single(1).unwrap();
        let win = WindowId(10);
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            show_bar: false,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
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
        monitor.z_order.attach_top(win);
        monitor.selected = Some(win);
        (wm, win, monitor_id)
    }

    #[test]
    fn unbound_client_click_focuses_and_replays() {
        let (mut wm, win, _) = setup_wm();
        let input = PressInput {
            root: Point::new(150, 150),
            button: Some(MouseButton::Left),
            raw_button: 1,
            modifiers: 0,
            clicked_window: Some(win),
            source: InteractionSource::Pointer,
            time_msec: 100,
        };

        let outcome = dispatch_press_policy(&mut wm.ctx(), input);
        assert_eq!(outcome, PressOutcome::ReplayToClient { window: Some(win) });
        assert_eq!(wm.core.model.selected_win(), Some(win));
    }

    #[test]
    fn empty_root_click_is_consumed() {
        let (mut wm, win, _) = setup_wm();
        crate::focus::focus(&mut wm.ctx(), Some(win));
        assert_eq!(wm.core.model.selected_win(), Some(win));

        let input = PressInput {
            root: Point::new(800, 800),
            button: Some(MouseButton::Left),
            raw_button: 1,
            modifiers: 0,
            clicked_window: None,
            source: InteractionSource::Pointer,
            time_msec: 100,
        };

        let outcome = dispatch_press_policy(&mut wm.ctx(), input);
        assert_eq!(outcome, PressOutcome::Consumed);
    }

    #[test]
    fn overview_active_client_click_captures_interaction() {
        let (mut wm, win, _) = setup_wm();
        crate::overview::toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
        assert!(wm.core.model.is_overview_active());

        let input = PressInput {
            root: Point::new(150, 150),
            button: Some(MouseButton::Left),
            raw_button: 1,
            modifiers: 0,
            clicked_window: Some(win),
            source: InteractionSource::Pointer,
            time_msec: 100,
        };

        let outcome = dispatch_press_policy(&mut wm.ctx(), input);
        assert_eq!(
            outcome,
            PressOutcome::CapturedInteraction {
                button: MouseButton::Left
            }
        );
    }

    #[test]
    fn overview_active_root_click_exits_overview_and_consumes() {
        let (mut wm, _, _) = setup_wm();
        crate::overview::toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
        assert!(wm.core.model.is_overview_active());

        let input = PressInput {
            root: Point::new(800, 800),
            button: Some(MouseButton::Left),
            raw_button: 1,
            modifiers: 0,
            clicked_window: None,
            source: InteractionSource::Pointer,
            time_msec: 100,
        };

        let outcome = dispatch_press_policy(&mut wm.ctx(), input);
        assert_eq!(outcome, PressOutcome::Consumed);
        assert!(!wm.core.model.is_overview_active());
    }

    fn overview_binding() -> Button {
        Button {
            target: ButtonTarget::Root,
            mask: 0,
            button: MouseButton::Left,
            action: ButtonAction::named(NamedAction::ToggleOverview),
        }
    }

    #[test]
    fn duplicate_pointer_chords_execute_only_the_first_binding() {
        let (mut wm, _, _) = setup_wm();
        wm.core.config.bindings.buttons = vec![overview_binding(), overview_binding()];

        let outcome = dispatch_press_policy(
            &mut wm.ctx(),
            PressInput {
                root: Point::new(800, 800),
                button: Some(MouseButton::Left),
                raw_button: 1,
                modifiers: 0,
                clicked_window: None,
                source: InteractionSource::Pointer,
                time_msec: 100,
            },
        );

        assert_eq!(outcome, PressOutcome::Consumed);
        assert!(wm.core.model.is_overview_active());
    }

    #[test]
    fn touch_does_not_execute_root_mouse_bindings() {
        let (mut wm, _, _) = setup_wm();
        wm.core.config.bindings.buttons = vec![overview_binding()];

        let outcome = dispatch_press_policy(
            &mut wm.ctx(),
            PressInput {
                root: Point::new(800, 800),
                button: Some(MouseButton::Left),
                raw_button: 1,
                modifiers: 0,
                clicked_window: None,
                source: InteractionSource::Touch(1),
                time_msec: 100,
            },
        );

        assert_eq!(outcome, PressOutcome::Consumed);
        assert!(!wm.core.model.is_overview_active());
    }

    #[test]
    fn touch_on_a_floating_border_does_not_start_pointer_resize() {
        let (mut wm, win, _) = setup_wm();
        wm.core.config.bindings.buttons.clear();
        let border = Point::new(100, 150);

        let outcome = dispatch_press_policy(
            &mut wm.ctx(),
            PressInput {
                root: border,
                button: Some(MouseButton::Left),
                raw_button: 1,
                modifiers: 0,
                clicked_window: Some(win),
                source: InteractionSource::Touch(1),
                time_msec: 100,
            },
        );

        assert_eq!(outcome, PressOutcome::ReplayToClient { window: Some(win) });
        assert_eq!(wm.core.interaction.drag.captured_source(), None);

        let (mut pointer_wm, pointer_win, _) = setup_wm();
        pointer_wm.core.config.bindings.buttons.clear();
        let pointer_outcome = dispatch_press_policy(
            &mut pointer_wm.ctx(),
            PressInput {
                root: border,
                button: Some(MouseButton::Left),
                raw_button: 1,
                modifiers: 0,
                clicked_window: Some(pointer_win),
                source: InteractionSource::Pointer,
                time_msec: 100,
            },
        );
        assert_eq!(
            pointer_outcome,
            PressOutcome::CapturedInteraction {
                button: MouseButton::Left
            }
        );
    }

    #[test]
    fn middle_click_close_clears_the_hover_offer_first() {
        let (mut wm, win, _) = setup_wm();
        wm.core.config.bindings.buttons.clear();
        let border = Point::new(100, 150);
        assert_eq!(
            crate::mouse::update_resize_offer_at(&mut wm.ctx(), border),
            Some(win)
        );

        let outcome = dispatch_press_policy(
            &mut wm.ctx(),
            PressInput {
                root: border,
                button: Some(MouseButton::Middle),
                raw_button: 2,
                modifiers: 0,
                clicked_window: Some(win),
                source: InteractionSource::Pointer,
                time_msec: 100,
            },
        );

        assert_eq!(outcome, PressOutcome::Consumed);
        assert_eq!(
            wm.core.interaction.drag.hover_offer(),
            crate::core_state::HoverOffer::None
        );
    }
}
