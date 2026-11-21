//! Hover-resize: cursor feedback and click-to-resize around floating windows
//! and in gaps between tiled windows.
//!
//! Moving from inside a floating window into its outer border band changes
//! the cursor to a resize shape.  A left-click then starts an interactive
//! resize (or move, when the cursor is at the window's top-middle edge);
//! a right-click always starts a move; a middle-click closes the window.
//! Moving beyond the band clears the offer until the pointer visits the
//! window interior again.
//! The offer is occlusion-aware: a border seam hidden beneath another
//! window's surface is never offered.
//! An adjustable inner tiling gap instead offers the corresponding tree seam;
//! dragging either primary button there has the same layout effect as a
//! Super+right-button resize on the adjacent tile.
//!
//! The hover-offer model is authoritative: cursor presentation and pointer
//! routing are derived from it and reconciled by the active backend.
//! The shared motion policy updates the offer; the shared press policy
//! commits it, so X11 and Wayland cannot drift apart.
//!
//! ## Entry points
//!
//! | Function                                      | Called from          | Purpose                                    |
//! |-----------------------------------------------|----------------------|--------------------------------------------|
//! | [`update_resize_offer_at`]                    | shared motion policy | Update resize offer + cursor               |
//! | [`focus_resize_offer`]                        | shared motion policy | Apply X11's historical resize focus rule   |

use crate::contexts::WmCtx;
use crate::core_state::HoverOffer;
use crate::model::WmModel;
use crate::types::{Client, HoverFocusTrigger, Monitor, Point, Rect, ResizeDirection, WindowId};

use super::constants::RESIZE_BORDER_ZONE;

// ── Hover offer helpers ──────────────────────────────────────────────────────
//
// Pure hover-offer state lives on [`crate::core_state::HoverOffer`] /
// [`crate::core_state::PointerInteractionState`]; these functions reconcile its derived
// presentation after each transition.

/// Window and direction selected by the resize-border hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverResizeHit {
    pub win: WindowId,
    pub dir: ResizeDirection,
    pub geo: Rect,
}

/// Update the passive resize offer. A floating border can only inherit
/// the pointer from that window's interior or its already active border band.
pub fn update_resize_offer_at(
    ctx: &mut WmCtx,
    root: Point,
    hovered: Option<WindowId>,
    trigger: HoverFocusTrigger,
) -> Option<WindowId> {
    let state = &ctx.core().state;
    let drag = &state.interaction.drag;
    if drag.has_capture() {
        return None;
    }

    let physical_motion = trigger == HoverFocusTrigger::PointerMotion;
    let origin = drag.resize_hover_origin();
    let candidate = drag
        .hover_offer()
        .resize_target()
        .map(|(win, _)| win)
        .or(origin.filter(|_| physical_motion));

    let mut next_origin = None;
    let (offer, target) =
        if let Some(hit) = candidate.and_then(|win| resize_target_at(&state.model, win, root)) {
            (
                HoverOffer::Resize {
                    win: hit.win,
                    dir: hit.dir,
                },
                Some(hit.win),
            )
        } else if let Some((win, dir)) =
            crate::layouts::manager::pointer_tree_gap_resize_target(state, root)
        {
            (HoverOffer::TreeResize { win, dir }, Some(win))
        } else {
            next_origin = hovered
                .filter(|&win| physical_motion || origin == Some(win))
                .filter(|&win| {
                    state.model.client_view(win).is_some_and(|view| {
                        accepts_floating_resize(view.client, view.monitor)
                            && view.client.geo.contains_point(root)
                            && !view
                                .client
                                .geo
                                .contains_resize_border_point(root, RESIZE_BORDER_ZONE)
                    })
                });
            (HoverOffer::None, None)
        };

    let changed = ctx.transition_pointer_interaction(|drag| {
        let changed = drag.set_hover_offer(offer);
        drag.set_resize_hover_origin(next_origin);
        changed
    });
    if !changed {
        // Retry native projection even when the offer is unchanged.
        ctx.sync_interaction_projection();
    }
    target
}

/// Clear any active hover offer and reconcile the resulting presentation.
pub fn clear_hover_offer(ctx: &mut WmCtx) {
    // Reconciliation is intentionally unconditional so native state can heal
    // even after a redundant logical clear.
    let changed = ctx.transition_pointer_interaction(|drag| drag.clear_hover_offer());
    if !changed {
        ctx.sync_interaction_projection();
    }
}

fn accepts_floating_resize(client: &Client, monitor: &Monitor) -> bool {
    client.is_visible(monitor.visible_tags())
        && (client.mode().is_normal_floating() || !monitor.is_tiling_layout())
}

/// Validate the tracked window's band against the pointer output and stacking.
pub fn resize_target_at(model: &WmModel, win: WindowId, root: Point) -> Option<HoverResizeHit> {
    let pointer_monitor = model
        .monitors
        .monitor_intersecting_rect(crate::mouse::pointer::point_rect(root))?;
    let view = model.client_view(win)?;
    let client = view.client;
    if pointer_monitor.bar_contains_y(root.y)
        || !accepts_floating_resize(client, view.monitor)
        || !client
            .geo
            .contains_resize_border_point(root, RESIZE_BORDER_ZONE)
        || point_occluded_above(model, win, root)
    {
        return None;
    }

    Some(HoverResizeHit {
        win,
        dir: ResizeDirection::from_hit(client.geo.size(), client.geo.local_point(root)),
        geo: client.geo,
    })
}

/// Check occlusion using the same global layer projection as rendering.
fn point_occluded_above(model: &WmModel, win: WindowId, point: Point) -> bool {
    crate::layouts::global_z_order(model)
        .into_iter()
        .rev()
        .take_while(|&above| above != win)
        .any(|above| {
            model.client_view(above).is_some_and(|view| {
                view.client.is_visible(view.monitor.visible_tags())
                    && view.client.total_rect().contains_point(point)
            })
        })
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(model: &WmModel) -> bool {
    let has_tiling = model.expect_selected_monitor().is_tiling_layout();
    let mon = model.expect_selected_monitor();
    let selected = mon.visible_tags();
    has_tiling
        && mon
            .iter_clients()
            .any(|(_, c)| c.is_visible(selected) && !c.mode().is_normal_floating())
}

// ── Motion-notify hook ───────────────────────────────────────────────────────

/// Apply the historical X11 rule for moving focus onto a resize offer.
///
/// The shared motion policy decides whether a resize offer owns the motion;
/// only X11 opts into this focus side effect. Wayland keeps focus independent
/// so its pointer focus and resize cursor cannot fight each other.
pub fn focus_resize_offer(ctx: &mut WmCtx, win: WindowId) {
    let should_focus = ctx
        .core()
        .state
        .config
        .window
        .focus_follows_mouse
        .allows(crate::types::HoverFocusTrigger::PointerMotion)
        && ctx.core().state.model.selected_win() != Some(win)
        && !has_visible_tiled_client(&ctx.core().state.model);

    if should_focus {
        crate::focus::focus(ctx, Some(win));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum SidebarOfferUpdate {
    None,
    Active,
    Cleared,
}

impl SidebarOfferUpdate {
    pub fn affects_pointer_handling(self) -> bool {
        !matches!(self, SidebarOfferUpdate::None)
    }
}

pub fn set_sidebar_offer(
    ctx: &mut WmCtx,
    target: Option<crate::types::SidebarTarget>,
) -> SidebarOfferUpdate {
    if let Some(target) = target {
        // Always reconcile. Gesture completion can leave the same logical
        // offer in place while ending a higher-priority captured interaction.
        let changed = ctx.transition_pointer_interaction(|drag| {
            drag.set_hover_offer(HoverOffer::Sidebar(target))
        });
        if !changed {
            ctx.sync_interaction_projection();
        }
        return SidebarOfferUpdate::Active;
    }

    if ctx.core().state.interaction.drag.hover_offer().is_sidebar() {
        clear_hover_offer(ctx);
        return SidebarOfferUpdate::Cleared;
    }

    SidebarOfferUpdate::None
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{MonitorBuilder, add_client_with, add_selected_client_with};
    use crate::types::{ClientMode, MonitorId, TagMask, WindowId};
    use crate::wm::WaylandWm as Wm;

    #[test]
    fn floating_resize_offer_requires_an_outward_crossing() {
        let mut wm =
            crate::test_support::TestWm::new(crate::backend::WaylandBackendData::default());
        let tags = TagMask::single(1).unwrap();
        let monitor = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(0, 0, 1200, 800))
                .bar(0, false)
                .tag_count(1)
                .selected_tags(tags)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor);
        let tile = add_selected_client_with(&mut wm.core.state.model, monitor, |client| {
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

        use crate::mouse::press::{PressInput, PressOutcome, dispatch_press_policy};
        use crate::types::{AltCursor, InteractionSource, ModMask, MouseButton};

        let inside = Point::new(690, 270);
        let border = Point::new(710, 270);
        let far = Point::new(750, 270);
        let motion = |wm: &mut crate::test_support::TestWm, point, hovered, trigger| {
            update_resize_offer_at(&mut wm.test_ctx(), point, Some(hovered), trigger)
        };
        let button = MouseButton::Left;
        let press = |wm: &mut crate::test_support::TestWm| {
            dispatch_press_policy(
                &mut wm.test_ctx(),
                PressInput {
                    root: border,
                    button: Some(button),
                    raw_button: 1,
                    modifiers: ModMask::NONE,
                    clicked_window: Some(tile),
                    source: InteractionSource::Pointer,
                    time_msec: 0,
                },
            )
        };
        let device = HoverFocusTrigger::PointerMotion;
        let refresh = HoverFocusTrigger::SceneChange;
        for (point, hovered, trigger, expected) in [
            (far, tile, device, None),
            (border, tile, device, None),
            (inside, floating, refresh, None),
            (border, tile, device, None),
            (inside, floating, device, None),
            (border, tile, device, Some(floating)),
            (Point::new(720, 270), tile, device, Some(floating)),
            (far, tile, device, None),
            (border, tile, device, None),
        ] {
            assert_eq!(
                motion(&mut wm, point, hovered, trigger),
                expected,
                "{point:?}"
            );
            let cursor = if expected.is_some() {
                AltCursor::Resize(ResizeDirection::Right)
            } else {
                AltCursor::Default
            };
            assert_eq!(wm.core.state.interaction.drag.projection().cursor, cursor);
        }
        assert_eq!(
            press(&mut wm),
            PressOutcome::ReplayToClient { window: Some(tile) }
        );
        assert_eq!(wm.core.state.model.selected_win(), Some(tile));

        motion(&mut wm, inside, floating, device);
        assert_eq!(motion(&mut wm, border, tile, device), Some(floating));
        assert_eq!(press(&mut wm), PressOutcome::CapturedInteraction { button });
        assert_eq!(
            wm.core
                .state
                .interaction
                .drag
                .projection()
                .active_resize_window,
            Some(floating)
        );
    }

    /// Push one taggable monitor for a hover fixture.
    ///
    /// The tag list must be non-empty before `set_selected_tags` can select
    /// anything, so the count is seeded here rather than at each call site.
    fn push_hover_monitor(wm: &mut Wm, rect: Rect, bar_shown: bool) -> MonitorId {
        wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(rect)
                .bar(0, bar_shown)
                .tag_count(9)
                .build(),
        )
    }

    #[test]
    fn overlapping_bands_keep_the_window_the_pointer_left() {
        let mut wm =
            crate::test_support::TestWm::new(crate::backend::WaylandBackendData::default());
        setup_stacked_floating_windows(
            &mut wm,
            Rect::new(100, 100, 600, 400),
            Rect::new(715, 100, 600, 400),
        );

        let bottom = WindowId(1);
        let device = HoverFocusTrigger::PointerMotion;
        update_resize_offer_at(
            &mut wm.test_ctx(),
            Point::new(690, 270),
            Some(bottom),
            device,
        );
        assert_eq!(
            update_resize_offer_at(&mut wm.test_ctx(), Point::new(710, 270), None, device),
            Some(bottom)
        );
    }

    #[test]
    fn resize_target_uses_the_monitor_under_the_pointer() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let tags = TagMask::single(1).unwrap();
        let left_id = push_hover_monitor(&mut wm, Rect::new(0, 0, 1920, 1080), true);
        let right_id = push_hover_monitor(&mut wm, Rect::new(1920, 0, 1920, 1080), true);
        wm.core.state.model.monitors.set_selected(left_id);
        wm.core
            .state
            .model
            .monitor_mut(right_id)
            .unwrap()
            .set_selected_tags(tags);

        let win = WindowId(3);
        add_client_with(&mut wm.core.state.model, right_id, |client| {
            client.win = win;
            client.tags = tags;
            client.geo = Rect::new(2000, 100, 600, 400);
            client.mode = ClientMode::floating();
            client.set_placement(crate::types::ClientPlacement::Floating);
        });

        let target = resize_target_at(&wm.core.state.model, win, Point::new(2200, 95));
        assert_eq!(target.map(|target| target.win), Some(win));
        assert_eq!(wm.core.state.model.selected_monitor_id(), left_id);
    }

    /// Insert two floating windows with `bottom` focused but stacked beneath
    /// `top`, mirroring the focus/stacking split of the overlap test above.
    fn setup_stacked_floating_windows(wm: &mut Wm, bottom_geo: Rect, top_geo: Rect) {
        let tags = TagMask::single(1).unwrap();
        let bottom = WindowId(1);
        let top = WindowId(2);

        let monitor_id = push_hover_monitor(wm, Rect::new(0, 0, 1920, 1080), false);
        wm.core
            .state
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected_tags(tags);
        wm.core.state.model.monitors.set_selected(monitor_id);

        // Adding `bottom` first stacks `top` above it in the persistent z-order.
        add_selected_client_with(&mut wm.core.state.model, monitor_id, |client| {
            client.win = bottom;
            client.tags = tags;
            client.geo = bottom_geo;
            client.mode = ClientMode::floating();
            client.set_placement(crate::types::ClientPlacement::Floating);
        });
        add_client_with(&mut wm.core.state.model, monitor_id, |client| {
            client.win = top;
            client.tags = tags;
            client.geo = top_geo;
            client.mode = ClientMode::floating();
            client.set_placement(crate::types::ClientPlacement::Floating);
        });
        // Focus order: `bottom` is the focused window.
        assert!(
            wm.core
                .state
                .model
                .monitor_mut(monitor_id)
                .unwrap()
                .set_focus_order(vec![bottom, top])
        );
    }

    /// A smaller floating window fully covered by a larger one must not offer
    /// its border seams through the covering window's surface.
    #[test]
    fn covered_window_borders_are_not_offered_through_the_covering_surface() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        setup_stacked_floating_windows(
            &mut wm,
            Rect::new(150, 130, 400, 300),
            Rect::new(100, 90, 600, 400),
        );

        // Inside the covered window's top border zone (y 100..130) but
        // strictly inside the covering window's surface.
        assert_eq!(
            resize_target_at(&wm.core.state.model, WindowId(1), Point::new(300, 110)),
            None
        );

        // The covering window's own border is exposed.
        let target = resize_target_at(&wm.core.state.model, WindowId(2), Point::new(300, 85));
        assert_eq!(target.map(|hit| hit.win), Some(WindowId(2)));
    }

    /// Occlusion must only suppress the hidden part of a seam: a covered
    /// window whose border pokes out beyond the covering window is still
    /// offered on its exposed side.
    #[test]
    fn exposed_border_of_a_covered_window_still_offers() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        setup_stacked_floating_windows(
            &mut wm,
            Rect::new(750, 130, 400, 300),
            Rect::new(100, 90, 600, 400),
        );

        // In the covered window's left border zone, outside the cover's
        // surface (the cover's band ends at x = 700 + 30).
        let target = resize_target_at(&wm.core.state.model, WindowId(1), Point::new(735, 200));
        assert_eq!(target.map(|hit| hit.win), Some(WindowId(1)));
    }
}

#[cfg(test)]
mod cross_output_tests {
    use super::*;
    use crate::test_support::{MonitorBuilder, add_client_with};
    use crate::types::{ClientMode, TagMask};

    #[test]
    fn spanning_float_border_wins_over_destination_tiles() {
        let mut wm =
            crate::test_support::TestWm::new(crate::backend::WaylandBackendData::default());
        let tags = TagMask::single(1).unwrap();
        let monitors: Vec<_> = [0, 1000]
            .into_iter()
            .map(|x| {
                wm.core.state.model.monitors.push(
                    MonitorBuilder::new()
                        .monitor_rect(Rect::new(x, 0, 1000, 800))
                        .bar(0, false)
                        .tag_count(1)
                        .selected_tags(tags)
                        .build(),
                )
            })
            .collect();
        let floating = WindowId(1);
        let tile = WindowId(2);
        add_client_with(&mut wm.core.state.model, monitors[0], |client| {
            client.win = floating;
            client.tags = tags;
            client.mode = ClientMode::floating();
            client.geo = Rect::new(800, 100, 400, 200);
        });
        add_client_with(&mut wm.core.state.model, monitors[1], |client| {
            client.win = tile;
            client.tags = tags;
            client.mode = ClientMode::tiled();
            client.geo = Rect::new(1000, 0, 1000, 800);
        });
        let point = Point::new(1100, 95);
        assert_eq!(
            resize_target_at(&wm.core.state.model, floating, point).map(|hit| hit.win),
            Some(floating)
        );
        assert!(!point_occluded_above(&wm.core.state.model, floating, point));
    }
}
