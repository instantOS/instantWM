//! Hover-resize: cursor feedback and click-to-resize/move/close near floating
//! windows.
//!
//! When the pointer hovers just outside a floating window's border, the root
//! cursor changes to a resize shape.  A left-click then starts an interactive
//! resize (or move, when the cursor is at the window's top-middle edge);
//! a right-click always starts a move; a middle-click closes the window.
//! Moving further away deactivates the mode.
//!
//! ## Entry points
//!
//! | Function                                      | Called from          | Purpose                                    |
//! |-----------------------------------------------|----------------------|--------------------------------------------|
//! | [`update_floating_resize_offer_at`]           | `motion_notify`      | Update resize offer and cursor feedback    |
//! | [`update_selected_resize_offer_at`]           | Wayland motion       | Update selected-window resize offer        |
//! | [`commit_x11_hover_offer`]                    | X11 button press     | Commit current offer to move/resize        |

use crate::contexts::{WmCtx, WmCtxX11};
use crate::core_state::HoverOffer;
use crate::model::WmModel;
use crate::types::{AltCursor, MouseButton, Point, Rect, ResizeDirection, WindowId};

use super::constants::RESIZE_BORDER_ZONE;
use super::resize::resize_mouse_directional;

// ── Hover offer helpers ──────────────────────────────────────────────────────
//
// Pure hover-offer state lives on [`crate::core_state::HoverOffer`] /
// [`crate::core_state::DragState`]; these functions apply the matching cursor.

/// Window and direction selected by the resize-border hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverResizeTarget {
    pub win: WindowId,
    pub dir: ResizeDirection,
    pub geo: Rect,
}

/// Activate a resize hover offer and apply the matching cursor.
fn offer_hover_resize(ctx: &mut WmCtx, target: HoverResizeTarget) {
    ctx.core_mut()
        .state_mut()
        .drag
        .set_hover_offer(HoverOffer::Resize {
            win: target.win,
            dir: target.dir,
        });
    ctx.set_cursor_style(AltCursor::Resize(target.dir));
}

/// Clear any active hover offer and reset the cursor if the state changed.
pub fn clear_hover_offer(ctx: &mut WmCtx) {
    if ctx.core_mut().drag_state_mut().clear_hover_offer() {
        ctx.set_cursor_style(AltCursor::Default);
    }
}

/// Commit the current X11 resize offer to a move, resize, or close operation.
///
/// Returns `false` when there is no resize offer or the mouse button is not a
/// valid commit button for hover resize.
pub fn commit_x11_hover_offer(ctx: &mut WmCtxX11, btn: MouseButton) -> bool {
    let Some((win, dir)) = ctx.core.drag_state().hover_offer.resize_target() else {
        return false;
    };

    if btn == MouseButton::Middle {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        clear_hover_offer(&mut wm_ctx);
        if wm_ctx.core().model().selected_win() != Some(win) {
            crate::focus::focus(&mut wm_ctx, Some(win));
        }
        crate::client::kill::close_win(&mut wm_ctx, win);
        return true;
    }

    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }

    let move_from_top_middle = {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        clear_hover_offer(&mut wm_ctx);
        if wm_ctx.core().model().selected_win() != Some(win) {
            crate::focus::focus(&mut wm_ctx, Some(win));
        }

        let Some(c) = wm_ctx.core().model().client(win) else {
            return false;
        };
        wm_ctx
            .pointer_backend()
            .pointer_location()
            .map(|p| c.geo.is_at_top_middle_edge(p, RESIZE_BORDER_ZONE))
            .unwrap_or(dir == ResizeDirection::Top)
    };

    if btn == MouseButton::Right || move_from_top_middle {
        crate::backend::x11::mouse::move_mouse(ctx, btn, None);
    } else {
        resize_mouse_directional(ctx, Some(dir), btn);
    }

    true
}

fn resize_target_for_window(
    model: &WmModel,
    win: WindowId,
    root: Point,
) -> Option<HoverResizeTarget> {
    let c = model.client(win)?;
    let mon = model.expect_selected_monitor();
    let selected_tags = mon.selected_tags();
    let has_tiling = mon.is_tiling_layout();

    if !c.is_visible(selected_tags) {
        return None;
    }
    if !c.mode().is_floating() && has_tiling {
        return None;
    }
    if !c.geo.contains_resize_border_point(root, RESIZE_BORDER_ZONE) {
        return None;
    }

    let hit = c.geo.local_point(root);
    Some(HoverResizeTarget {
        win,
        dir: ResizeDirection::from_hit(c.geo.size(), hit),
        geo: c.geo,
    })
}

fn pointer_in_bar(model: &WmModel, root_y: i32) -> bool {
    let mon = model.expect_selected_monitor();
    mon.bar_contains_y(&model.clients, root_y)
}

// ── Border detection ─────────────────────────────────────────────────────────

/// Return the floating window + direction currently targeted by hover-resize.
fn hover_resize_target_at(model: &WmModel, root: Point) -> Option<HoverResizeTarget> {
    if pointer_in_bar(model, root.y) {
        return None;
    }
    let mon = model.expect_selected_monitor();
    mon.iter_clients(&model.clients)
        .find_map(|(win, _)| resize_target_for_window(model, win, root))
}

pub fn selected_hover_resize_target_at(
    model: &WmModel,
    position: Point,
) -> Option<HoverResizeTarget> {
    let win = model.selected_win()?;
    if pointer_in_bar(model, position.y) {
        return None;
    }
    resize_target_for_window(model, win, position)
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(model: &WmModel) -> bool {
    let has_tiling = model.expect_selected_monitor().is_tiling_layout();
    let mon = model.expect_selected_monitor();
    let selected = mon.selected_tags();
    has_tiling
        && mon
            .iter_clients(&model.clients)
            .any(|(_, c)| c.is_visible(selected) && !c.mode().is_floating())
}

// ── Motion-notify hook ───────────────────────────────────────────────────────

/// Updates the resize offer when the pointer is in a floating window border.
///
/// Returns `true` when the pointer is over a resize offer zone and the caller
/// should stop processing the motion event.
pub fn update_floating_resize_offer_at(ctx: &mut WmCtx, root: Point) -> bool {
    if let Some(target) = hover_resize_target_at(ctx.core().model(), root) {
        offer_hover_resize(ctx, target);
        // This function is only entered from physical X11 motion. The shared
        // mode policy still decides whether the resize offer may move focus.
        // Otherwise the motion handler resolves the actual window beneath the
        // pointer after the resize-offer check.
        let should_focus = ctx
            .core()
            .behavior()
            .focus_follows_mouse
            .allows(crate::types::HoverFocusTrigger::PointerMotion)
            && ctx.core().model().selected_win() != Some(target.win)
            && !has_visible_tiled_client(ctx.core().model());

        if should_focus {
            crate::focus::focus(ctx, Some(target.win));
        }
        return true;
    }

    clear_hover_offer(ctx);
    false
}

/// Update the resize offer for the currently selected window.
///
/// This is the backend-neutral hover-resize path used by Wayland motion events.
pub fn update_selected_resize_offer_at(ctx: &mut WmCtx, position: Point) -> Option<WindowId> {
    let Some(target) = selected_hover_resize_target_at(ctx.core().model(), position) else {
        clear_hover_offer(ctx);
        return None;
    };
    offer_hover_resize(ctx, target);
    Some(target.win)
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

pub fn update_sidebar_offer_at(ctx: &mut WmCtx, root: crate::types::Point) -> SidebarOfferUpdate {
    if let Some(target) = crate::mouse::pointer::sidebar_target_at(ctx.core().model(), root) {
        if ctx.core().drag_state().hover_offer != HoverOffer::Sidebar(target) {
            ctx.core_mut()
                .state_mut()
                .drag
                .set_hover_offer(HoverOffer::Sidebar(target));
            ctx.set_cursor_style(AltCursor::Resize(ResizeDirection::Left));
        }
        return SidebarOfferUpdate::Active;
    }

    if ctx.core().drag_state().hover_offer.is_sidebar() {
        clear_hover_offer(ctx);
        return SidebarOfferUpdate::Cleared;
    }

    SidebarOfferUpdate::None
}
