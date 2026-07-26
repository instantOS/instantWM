//! Client/window management types.
//!
//! Types for managed windows and client lists.

use std::collections::HashMap;

use crate::types::TagMask;
use crate::types::WindowId;
use crate::types::core::MonitorId;
use crate::types::geometry::{Rect, Size, SizeHints};
use crate::types::input::{EdgeDirection, SnapPosition};

/// Persistent placement policy, independent of temporary presentation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ClientPlacement {
    #[default]
    Tiling,
    Floating,
}

/// Why a client is maximized.
///
/// Client-requested maximization is projected to XDG/EWMH state. `Wm` is the
/// instantWM-only zoom operation and deliberately has no protocol meaning.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum MaximizedOrigin {
    Client,
    Wm,
}

/// Whether fullscreen changes geometry or only the protocol-visible state.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum FullscreenKind {
    True,
    Fake,
}

/// Presentation to restore when fullscreen ends.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum RestoredPresentation {
    #[default]
    Normal,
    Maximized(MaximizedOrigin),
}

/// Temporary presentation, independent of tiled/floating placement.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ClientPresentation {
    #[default]
    Normal,
    Maximized(MaximizedOrigin),
    Fullscreen {
        kind: FullscreenKind,
        restore: RestoredPresentation,
    },
}

/// Complete window mode with orthogonal placement and presentation axes.
///
/// Keeping these values in one immutable value makes snapshots and IPC simple,
/// while preventing a presentation transition from overwriting placement.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct ClientMode {
    placement: ClientPlacement,
    presentation: ClientPresentation,
}

impl Default for ClientMode {
    fn default() -> Self {
        Self::tiled()
    }
}

impl ClientMode {
    #[inline]
    pub(crate) const fn tiled() -> Self {
        Self::normal(ClientPlacement::Tiling)
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn floating() -> Self {
        Self::normal(ClientPlacement::Floating)
    }

    #[inline]
    pub(crate) const fn normal(placement: ClientPlacement) -> Self {
        Self {
            placement,
            presentation: ClientPresentation::Normal,
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn maximized(placement: ClientPlacement, origin: MaximizedOrigin) -> Self {
        Self {
            placement,
            presentation: ClientPresentation::Maximized(origin),
        }
    }

    #[inline]
    pub const fn presentation(self) -> ClientPresentation {
        self.presentation
    }

    #[inline]
    pub fn is_fullscreen(self) -> bool {
        matches!(self.presentation, ClientPresentation::Fullscreen { .. })
    }

    #[inline]
    pub fn is_true_fullscreen(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Fullscreen {
                kind: FullscreenKind::True,
                ..
            }
        )
    }

    #[inline]
    pub fn is_fake_fullscreen(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Fullscreen {
                kind: FullscreenKind::Fake,
                ..
            }
        )
    }

    /// Whether maximized geometry is the current presentation.
    #[inline]
    pub fn is_maximized(self) -> bool {
        matches!(self.presentation, ClientPresentation::Maximized(_))
    }

    #[inline]
    pub fn is_wm_maximized(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Maximized(MaximizedOrigin::Wm)
        )
    }

    #[inline]
    pub const fn maximized_origin(self) -> Option<MaximizedOrigin> {
        match self.presentation {
            ClientPresentation::Maximized(origin) => Some(origin),
            _ => None,
        }
    }

    /// Whether literal client-owned maximization is the current presentation
    /// or the presentation restored after fullscreen.
    ///
    /// This is intentionally not the protocol projection for tiled windows;
    /// that also depends on the monitor's layout presentation.
    #[inline]
    pub fn has_client_maximized_presentation(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Maximized(MaximizedOrigin::Client)
                | ClientPresentation::Fullscreen {
                    restore: RestoredPresentation::Maximized(MaximizedOrigin::Client),
                    ..
                }
        )
    }

    #[inline]
    pub fn is_normal_floating(self) -> bool {
        self.placement == ClientPlacement::Floating
            && matches!(self.presentation, ClientPresentation::Normal)
    }

    #[inline]
    pub fn is_normal_tiling(self) -> bool {
        self.placement == ClientPlacement::Tiling
            && matches!(self.presentation, ClientPresentation::Normal)
    }

    #[inline]
    pub fn is_free_positioned(self) -> bool {
        self.is_normal_floating() || self.is_maximized()
    }

    #[inline]
    pub const fn placement(self) -> ClientPlacement {
        self.placement
    }

    /// Replace the persistent placement mode without discarding a temporary
    /// presentation mode.
    ///
    /// Rules and policy refreshes may change whether a client should restore
    /// to tiling or floating, but they do not own fullscreen/maximized state.
    #[inline]
    pub(crate) fn with_placement(self, placement: ClientPlacement) -> Self {
        Self { placement, ..self }
    }

    #[inline]
    pub(crate) fn as_fullscreen(self) -> Self {
        let restore = match self.presentation {
            ClientPresentation::Maximized(origin) => RestoredPresentation::Maximized(origin),
            ClientPresentation::Fullscreen { restore, .. } => restore,
            ClientPresentation::Normal => RestoredPresentation::Normal,
        };
        Self {
            presentation: ClientPresentation::Fullscreen {
                kind: FullscreenKind::True,
                restore,
            },
            ..self
        }
    }

    #[inline]
    pub(crate) fn as_fake_fullscreen(self) -> Self {
        let restore = match self.presentation {
            ClientPresentation::Maximized(origin) => RestoredPresentation::Maximized(origin),
            ClientPresentation::Fullscreen { restore, .. } => restore,
            ClientPresentation::Normal => RestoredPresentation::Normal,
        };
        Self {
            presentation: ClientPresentation::Fullscreen {
                kind: FullscreenKind::Fake,
                restore,
            },
            ..self
        }
    }

    #[inline]
    pub(crate) fn with_maximized(self, maximized: bool, origin: MaximizedOrigin) -> Self {
        let presentation = match (self.presentation, maximized) {
            (ClientPresentation::Fullscreen { kind, .. }, true) => ClientPresentation::Fullscreen {
                kind,
                restore: RestoredPresentation::Maximized(origin),
            },
            (
                ClientPresentation::Fullscreen {
                    kind,
                    restore: RestoredPresentation::Maximized(current),
                },
                false,
            ) if current == origin => ClientPresentation::Fullscreen {
                kind,
                restore: RestoredPresentation::Normal,
            },
            (fullscreen @ ClientPresentation::Fullscreen { .. }, false) => fullscreen,
            (_, true) => ClientPresentation::Maximized(origin),
            (ClientPresentation::Maximized(current), false) if current == origin => {
                ClientPresentation::Normal
            }
            (presentation, false) => presentation,
        };
        Self {
            presentation,
            ..self
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn as_maximized(self) -> Self {
        self.with_maximized(true, MaximizedOrigin::Client)
    }

    #[inline]
    pub(crate) fn restored(self) -> Self {
        let presentation = match self.presentation {
            ClientPresentation::Fullscreen {
                restore: RestoredPresentation::Normal,
                ..
            }
            | ClientPresentation::Maximized(_) => ClientPresentation::Normal,
            ClientPresentation::Fullscreen {
                restore: RestoredPresentation::Maximized(origin),
                ..
            } => ClientPresentation::Maximized(origin),
            ClientPresentation::Normal => ClientPresentation::Normal,
        };
        Self {
            presentation,
            ..self
        }
    }
}

/// Scratchpad-specific state for a window.
///
/// Present only when the window is a scratchpad. Groups the name, tags to
/// restore on unmake, and optional edge-anchored direction into a single
/// `Option<ScratchpadData>` on `Client`.
#[derive(Debug, Clone, Default)]
pub struct ScratchpadData {
    /// Scratchpad name.
    pub name: String,
    /// Tags to restore when unhiding from scratchpad.
    pub restore_tags: TagMask,
    /// Edge direction for edge-anchored scratchpads (None for regular scratchpads).
    pub direction: Option<EdgeDirection>,
    /// Window that had focus when this scratchpad was shown.
    ///
    /// Scratchpads are temporary UI. Hiding one should return to the window
    /// the user was working in, rather than selecting whichever client happens
    /// to be highest in the persistent stack.
    pub restore_focus: Option<WindowId>,
}

impl ScratchpadData {
    pub fn set_direction(&mut self, direction: EdgeDirection) {
        self.direction = Some(direction);
    }

    pub fn remember_focus(&mut self, window: Option<WindowId>) {
        self.restore_focus = window;
    }

    pub fn take_restore_focus(&mut self) -> Option<WindowId> {
        self.restore_focus.take()
    }
}

/// A real floating placement previously chosen or accepted by the WM.
///
/// The reference work area makes the placement portable across output
/// resolution, scale, position, and bar/exclusive-zone changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedFloatingPlacement {
    pub rect: Rect,
    pub reference_work_area: Rect,
}

/// Geometry state which exists independently of the client's current mode.
///
/// `saved` is deliberately optional: a tiled window that has never floated is
/// different from one whose previous floating rectangle happened to resemble
/// its tiled slot. `preferred_size` captures useful pre-layout client size
/// without pretending that the client supplied a meaningful floating position.
#[derive(Debug, Clone, Default)]
pub struct FloatingGeometryState {
    saved: Option<SavedFloatingPlacement>,
    preferred_size: Option<Size>,
}

/// Represents a managed client window in the window manager.
///
/// This struct contains all state for a window managed by instantWM,
/// including geometry, tags, flags, and relationships to other clients.
#[derive(Debug, Clone, Default)]
pub struct Client {
    /// Window title/name displayed in the bar.
    pub name: String,
    /// Minimum aspect ratio constraint from WM_NORMAL_HINTS.
    pub min_aspect: f32,
    /// Maximum aspect ratio constraint from WM_NORMAL_HINTS.
    pub max_aspect: f32,
    /// Current geometry.
    pub geo: Rect,
    /// Saved and preferred floating geometry, distinct from current tiled geometry.
    #[cfg(not(test))]
    floating_geometry: FloatingGeometryState,
    /// Unit tests may construct clients with struct update syntax.
    #[cfg(test)]
    pub(crate) floating_geometry: FloatingGeometryState,
    /// Previous geometry (for restoring).
    pub old_geo: Rect,
    /// Size hints from WM_NORMAL_HINTS property.
    pub size_hints: SizeHints,

    /// Whether [`size_hints`](Self::size_hints) contains a current backend snapshot.
    pub size_hints_valid: bool,
    /// Current border width.
    pub border_width: i32,
    /// Previous border width.
    pub old_border_width: i32,
    /// Tags this client belongs to.
    pub tags: TagMask,
    /// Whether the window has fixed size.
    pub is_fixed_size: bool,
    /// Combined persistent placement and temporary presentation state.
    ///
    /// Kept private so callers cannot accidentally discard fullscreen or
    /// maximized state while changing the tiled/floating placement policy.
    #[cfg(not(test))]
    mode: ClientMode,
    /// Unit tests may construct exact state-machine positions as fixtures.
    /// Production builds keep the same field private.
    #[cfg(test)]
    pub(crate) mode: ClientMode,
    /// Whether the window has urgency hint.
    pub is_urgent: bool,
    /// Whether the window should never receive focus.
    pub never_focus: bool,
    /// Whether the window is locked (can't be closed accidentally).
    pub is_locked: bool,
    /// Whether the window is sticky (visible on all tags).
    pub is_sticky: bool,
    /// Whether the window is minimized/hidden.
    pub is_hidden: bool,
    /// Current snap position.
    pub snap_status: SnapPosition,
    /// Scratchpad state (None if not a scratchpad).
    pub scratchpad: Option<ScratchpadData>,
    /// Monitor this client is on.
    pub monitor_id: MonitorId,
    /// Managed toplevel this window is transient for, when advertised by the
    /// client protocol (`xdg_toplevel.set_parent` / `WM_TRANSIENT_FOR`).
    ///
    /// This relationship is backend-neutral because stacking policy needs to
    /// keep dialogs above ordinary windows regardless of the active backend.
    pub transient_for: Option<WindowId>,
    /// Window ID.
    pub win: WindowId,
}

impl Client {
    /// Create a client with the default tiled placement.
    ///
    /// Construction is centralized because mode is an invariant-bearing state
    /// machine rather than an independently assignable data field.
    pub fn new(win: WindowId) -> Self {
        Self {
            win,
            ..Self::default()
        }
    }

    /// Current placement/presentation state.
    #[inline]
    pub fn mode(&self) -> ClientMode {
        self.mode
    }

    /// Persistent placement policy, independent of temporary presentation.
    #[inline]
    pub fn placement(&self) -> ClientPlacement {
        self.mode.placement()
    }

    /// Change the persistent tiled/floating policy while preserving any
    /// temporary fullscreen or maximized presentation.
    #[inline]
    pub(crate) fn set_placement(&mut self, placement: ClientPlacement) {
        self.mode = self.mode.with_placement(placement);
    }

    /// Enter true fullscreen while remembering the current base placement.
    #[inline]
    pub(crate) fn enter_fullscreen(&mut self) {
        self.mode = self.mode().as_fullscreen();
    }

    /// Enter fake fullscreen while remembering the current base placement.
    #[inline]
    pub(crate) fn enter_fake_fullscreen(&mut self) {
        self.mode = self.mode().as_fake_fullscreen();
    }

    /// Set maximized presentation without changing persistent placement.
    #[inline]
    pub(crate) fn set_maximized_presentation(&mut self, maximized: bool, origin: MaximizedOrigin) {
        self.mode = self.mode().with_maximized(maximized, origin);
    }

    /// Leave a temporary presentation mode and restore its base placement.
    #[inline]
    pub(crate) fn restore_mode(&mut self) {
        self.mode = self.mode().restored();
    }

    /// Construct otherwise unreachable states in unit tests without exposing a
    /// production escape hatch around the transition API.
    #[cfg(test)]
    pub(crate) fn set_mode_for_test(&mut self, mode: ClientMode) {
        self.mode = mode;
    }

    /// Calculate total width including borders.
    pub fn total_width(&self) -> i32 {
        self.geo.total_width(self.border_width)
    }

    /// Calculate total height including borders.
    pub fn total_height(&self) -> i32 {
        self.geo.total_height(self.border_width)
    }

    /// Return the outer bounding box of the client, including borders.
    pub fn total_rect(&self) -> Rect {
        Rect::new(
            self.geo.x,
            self.geo.y,
            self.total_width(),
            self.total_height(),
        )
    }

    /// Update border width and adjust geometry accordingly.
    pub fn set_border_width(&mut self, new_bw: i32) {
        let old_bw = self.border_width;
        let d = old_bw - new_bw;
        self.border_width = new_bw;

        self.update_geometry(Rect {
            x: self.geo.x,
            y: self.geo.y,
            w: self.geo.w + 2 * d,
            h: self.geo.h + 2 * d,
        });
    }

    /// Check whether a proposed geometry is large enough and meaningfully
    /// different from the client's current geometry.
    pub fn accepts_distinct_rect(
        &self,
        rect: Rect,
        min_size: i32,
        margin: i32,
        min_delta: i32,
    ) -> bool {
        rect.w > min_size
            && rect.h > min_size
            && rect.x > -margin
            && rect.y > -margin
            && ((self.geo.w - rect.w).abs() > min_delta
                || (self.geo.h - rect.h).abs() > min_delta
                || (self.geo.x - rect.x).abs() > min_delta
                || (self.geo.y - rect.y).abs() > min_delta)
    }

    /// Check if this client is a scratchpad window.
    pub fn is_scratchpad(&self) -> bool {
        self.scratchpad.is_some()
            && (self.tags.is_scratchpad_only() || self.is_hidden || self.is_sticky)
    }

    /// Check if this client is an edge-anchored scratchpad (has a slide direction).
    pub fn is_edge_scratchpad(&self) -> bool {
        self.scratchpad
            .as_ref()
            .is_some_and(|s| s.direction.is_some())
    }

    /// Check if this client is a normal minimized window rather than a hidden scratchpad.
    #[inline]
    pub fn is_minimized(&self) -> bool {
        self.is_hidden && !self.is_scratchpad()
    }

    /// Clear scratchpad-only metadata after the window has been moved to normal tags.
    pub fn clear_scratchpad_state(&mut self) {
        self.scratchpad = None;
        self.is_sticky = false;
    }

    /// Keep scratchpad metadata consistent with the current tag assignment.
    pub fn sync_scratchpad_state(&mut self) {
        if self.scratchpad.is_some()
            && !self.tags.is_scratchpad_only()
            && !self.is_hidden
            && !self.is_sticky
        {
            self.clear_scratchpad_state();
        }
    }

    /// Assign a new tag bitmask and normalize any dependent client state.
    pub fn set_tag_mask(&mut self, tags: TagMask) {
        self.tags = tags;
        self.sync_scratchpad_state();
    }

    /// Transform the tag bitmask in place and normalize dependent client state.
    pub fn update_tag_mask(&mut self, f: impl FnOnce(TagMask) -> TagMask) {
        self.tags = f(self.tags);
        self.sync_scratchpad_state();
    }

    /// Clear sticky status when moving a scratchpad client to real tags.
    ///
    /// A client on the scratchpad tag should lose its sticky flag when it is
    /// explicitly reassigned to a normal tag so that it stops following every
    /// view after the move.
    pub fn clear_sticky_if_scratchpad(&mut self) {
        if self.tags.is_scratchpad_only() {
            self.is_sticky = false;
        }
    }

    /// Check if the client is on the selected tags, ignoring hidden state.
    #[inline]
    pub fn is_on_selected_tags(&self, selected_tags: TagMask) -> bool {
        self.is_sticky || self.tags.intersects(selected_tags)
    }

    /// Check if the client is actually visible for the given tag-set.
    #[inline]
    pub fn is_visible(&self, selected_tags: TagMask) -> bool {
        self.is_on_selected_tags(selected_tags) && !self.is_hidden
    }

    /// Check if the client should keep a title entry in the bar.
    #[inline]
    pub fn shows_in_bar(&self, selected_tags: TagMask) -> bool {
        if self.is_scratchpad() {
            self.is_sticky && !self.is_hidden
        } else {
            self.is_on_selected_tags(selected_tags)
        }
    }

    /// Check if this client should be included in tiling calculations.
    #[inline]
    pub fn is_tiled(&self, selected_tags: TagMask) -> bool {
        self.mode().is_normal_tiling() && self.is_visible(selected_tags)
    }

    /// Whether this client owns a persistent leaf in the manual tiling tree.
    ///
    /// Temporary presentation modes must not remove a tiled client's leaf:
    /// doing so collapses the topology and reinserts the client elsewhere when
    /// fullscreen or maximized presentation ends.
    #[inline]
    pub fn is_tiling_tree_member(&self, selected_tags: TagMask) -> bool {
        self.placement() == ClientPlacement::Tiling && self.is_visible(selected_tags)
    }

    /// Clear the urgency flag for this client.
    pub fn clear_urgency(&mut self) {
        self.is_urgent = false;
    }

    pub fn saved_floating_placement(&self) -> Option<SavedFloatingPlacement> {
        self.floating_geometry.saved
    }

    pub fn saved_floating_rect(&self) -> Option<Rect> {
        self.saved_floating_placement()
            .map(|placement| placement.rect)
    }

    pub fn preferred_floating_size(&self) -> Option<Size> {
        self.floating_geometry.preferred_size
    }

    pub fn set_preferred_floating_size(&mut self, size: Size) {
        if size.is_positive() {
            self.floating_geometry.preferred_size = Some(size);
        }
    }

    pub fn save_floating_placement(&mut self, rect: Rect, reference_work_area: Rect) {
        if rect.is_valid() && reference_work_area.is_valid() {
            self.floating_geometry.saved = Some(SavedFloatingPlacement {
                rect,
                reference_work_area,
            });
        }
    }

    pub fn update_saved_floating_size(&mut self, size: Size) {
        if !size.is_positive() {
            return;
        }
        if let Some(placement) = self.floating_geometry.saved.as_mut() {
            placement.rect.w = size.w;
            placement.rect.h = size.h;
        }
    }

    pub fn update_geometry(&mut self, rect: Rect) {
        // Geometry synchronization can report the model's already-authoritative
        // rectangle (for example after a model-first fullscreen transition).
        // Keep `old_geo` as the previous distinct rectangle in that case.
        if self.geo == rect {
            return;
        }
        self.old_geo = self.geo;
        self.geo = rect;
    }

    pub fn save_border_width(&mut self) {
        if self.border_width != 0 {
            self.old_border_width = self.border_width;
        }
    }

    pub fn restore_border_width(&mut self) {
        if self.old_border_width != 0 {
            self.border_width = self.old_border_width;
        }
    }

    // -------------------------------------------------------------------------
    // Scratchpad state transitions
    // -------------------------------------------------------------------------

    /// Apply all client-local state changes required when making a window a scratchpad.
    ///
    /// Sets the scratchpad metadata, moves the client to the scratchpad tag, clears
    /// sticky, ensures floating mode, and — for edge-anchored scratchpads — also
    /// sizes the window, zeroes the border, and locks it.
    pub fn apply_scratchpad_state(
        &mut self,
        name: &str,
        direction: Option<EdgeDirection>,
        restore_tags: TagMask,
        mon_ww: i32,
        mon_wh: i32,
    ) {
        self.scratchpad = Some(ScratchpadData {
            name: name.to_string(),
            restore_tags,
            direction,
            restore_focus: None,
        });
        self.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
        self.is_sticky = false;
        if self.placement() != ClientPlacement::Floating {
            self.set_placement(ClientPlacement::Floating);
        }
        if let Some(dir) = direction {
            if dir.is_vertical() {
                self.geo.h = mon_wh / 3;
            } else {
                self.geo.w = mon_ww / 3;
            }
            self.save_border_width();
            self.border_width = 0;
            self.is_locked = true;
        }
    }

    /// Revert client-local state changes when removing scratchpad status.
    ///
    /// Assigns `restore_tags` (or the monitor's active tags when empty) and, for
    /// edge-anchored scratchpads, also restores the saved border width and unlocks.
    pub fn exit_scratchpad_state(&mut self, restore_tags: TagMask, had_direction: bool) {
        self.set_tag_mask(restore_tags);
        if had_direction {
            self.border_width = self.old_border_width;
            self.is_locked = false;
        }
    }

    /// Apply client-local state required to reveal a scratchpad window.
    ///
    /// Marks the client sticky, ensures floating mode, optionally zeroes the border
    /// for edge-anchored scratchpads, and updates the tag mask to the current tags.
    pub fn show_as_scratchpad(&mut self, tags: TagMask, direction: Option<EdgeDirection>) {
        self.is_sticky = true;
        self.set_placement(ClientPlacement::Floating);
        if direction.is_some() {
            self.border_width = 0;
        }
        self.set_tag_mask(tags);
    }

    /// Apply client-local state required to hide a scratchpad window.
    ///
    /// Clears sticky and moves the client back to the scratchpad tag.
    pub fn hide_as_scratchpad(&mut self) {
        self.is_sticky = false;
        self.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
    }

    pub fn set_tags(
        &mut self,
        mask: crate::types::TagMask,
        core: &mut crate::contexts::CoreCtx,
        x11: &crate::backend::x11::X11BackendRef,
        x11_runtime: &mut crate::backend::x11::X11RuntimeConfig,
    ) {
        let tag_mask = core.model().tags.mask();
        let effective_mask = mask & tag_mask;

        if effective_mask.is_empty() {
            return;
        }

        self.clear_sticky_if_scratchpad();
        self.set_tag_mask(effective_mask);

        crate::backend::x11::set_client_tag_prop(core.state(), x11, x11_runtime, self.win);
        crate::backend::x11::focus::focus_soft(core, x11, x11_runtime, None);
        let monitor_id = core.model().selected_monitor_id();
        core.queue_layout_for_monitor_urgent(monitor_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{Client, ClientMode, ClientPlacement, MaximizedOrigin, ScratchpadData};
    use crate::types::{Rect, SCRATCHPAD_MASK, TagMask};

    #[test]
    fn repeated_geometry_update_preserves_the_previous_distinct_rectangle() {
        let previous = Rect::new(0, 0, 1920, 1080);
        let current = Rect::new(200, 150, 900, 600);
        let next = Rect::new(240, 180, 800, 500);
        let mut client = Client {
            geo: current,
            old_geo: previous,
            ..Client::default()
        };

        client.update_geometry(current);
        assert_eq!(client.geo, current);
        assert_eq!(client.old_geo, previous);

        client.update_geometry(next);
        assert_eq!(client.geo, next);
        assert_eq!(client.old_geo, current);
    }

    #[test]
    fn fullscreen_restores_previous_tiling_mode() {
        let mut client = Client::default();

        client.enter_fullscreen();
        assert!(client.mode().is_true_fullscreen());
        assert!(!client.mode().is_normal_tiling());

        client.restore_mode();
        assert_eq!(client.mode(), ClientMode::tiled());
    }

    #[test]
    fn fullscreen_restores_previous_floating_mode() {
        let mut client = Client::default();
        client.set_placement(ClientPlacement::Floating);

        client.enter_fullscreen();
        assert!(client.mode().is_true_fullscreen());
        assert!(!client.mode().is_normal_floating());

        client.restore_mode();
        assert_eq!(client.mode(), ClientMode::floating());
    }

    #[test]
    fn maximized_restores_previous_regular_mode() {
        let mut client = Client::default();
        client.set_placement(ClientPlacement::Floating);

        client.set_maximized_presentation(true, MaximizedOrigin::Client);
        assert!(client.mode().is_maximized());
        assert!(!client.mode().is_normal_floating());

        client.restore_mode();
        assert_eq!(client.mode(), ClientMode::floating());
    }

    #[test]
    fn changing_placement_preserves_temporary_presentation() {
        for mode in [
            ClientMode::tiled().as_fullscreen(),
            ClientMode::tiled().as_fake_fullscreen(),
            ClientMode::tiled().as_maximized(),
        ] {
            let changed = mode.with_placement(ClientPlacement::Floating);
            assert_eq!(changed.presentation(), mode.presentation());
            assert_eq!(changed.restored(), ClientMode::floating());
        }
    }

    #[test]
    fn replacing_mode_does_not_implicitly_replace_saved_floating_geometry() {
        let saved = Rect::new(100, 120, 640, 480);
        let mut client = Client {
            geo: Rect::new(0, 0, 1920, 1080),
            ..Client::default()
        };
        client.save_floating_placement(saved, Rect::new(0, 0, 1920, 1080));

        client.set_placement(ClientPlacement::Tiling);

        assert_eq!(client.mode(), ClientMode::tiled());
        assert_eq!(client.saved_floating_rect(), Some(saved));
    }

    fn sp_data(name: &str, restore_tags: TagMask) -> ScratchpadData {
        ScratchpadData {
            name: name.to_string(),
            restore_tags,
            ..ScratchpadData::default()
        }
    }

    #[test]
    fn scratchpad_focus_restore_is_consumed_once() {
        let mut scratchpad = sp_data("menu", TagMask::EMPTY);
        scratchpad.remember_focus(Some(crate::types::WindowId(42)));

        assert_eq!(
            scratchpad.take_restore_focus(),
            Some(crate::types::WindowId(42))
        );
        assert_eq!(scratchpad.take_restore_focus(), None);
    }

    #[test]
    fn scratchpad_requires_scratchpad_tag() {
        let client = Client {
            scratchpad: Some(sp_data("term", TagMask::EMPTY)),
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        assert!(!client.is_scratchpad());
    }

    #[test]
    fn sync_clears_stale_scratchpad_metadata() {
        let mut client = Client {
            scratchpad: Some(sp_data("term", TagMask::single(2).unwrap())),
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert!(client.scratchpad.is_none());
        assert!(!client.is_sticky);
    }

    #[test]
    fn sync_keeps_valid_scratchpad_metadata() {
        let mut client = Client {
            scratchpad: Some(sp_data("term", TagMask::single(2).unwrap())),
            is_sticky: true,
            tags: TagMask::from_bits(SCRATCHPAD_MASK),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert_eq!(client.scratchpad.as_ref().unwrap().name, "term");
        assert_eq!(
            client.scratchpad.as_ref().unwrap().restore_tags,
            TagMask::single(2).unwrap()
        );
        assert!(client.is_sticky);
        assert!(client.is_scratchpad());
    }

    #[test]
    fn sync_keeps_hidden_scratchpad_metadata_off_scratchpad_tag() {
        let mut client = Client {
            scratchpad: Some(sp_data("term", TagMask::single(2).unwrap())),
            is_hidden: true,
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert_eq!(client.scratchpad.as_ref().unwrap().name, "term");
        assert!(client.is_scratchpad());
    }

    #[test]
    fn sync_keeps_sticky_scratchpad_metadata_off_scratchpad_tag() {
        let mut client = Client {
            scratchpad: Some(sp_data("term", TagMask::single(2).unwrap())),
            is_sticky: true,
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert_eq!(client.scratchpad.as_ref().unwrap().name, "term");
        assert!(client.is_scratchpad());
    }

    #[test]
    fn minimized_normal_window_stays_in_bar() {
        let client = Client {
            is_hidden: true,
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        assert!(client.is_minimized());
        assert!(client.shows_in_bar(TagMask::single(1).unwrap()));
    }

    #[test]
    fn hidden_scratchpad_does_not_stay_in_bar() {
        let client = Client {
            scratchpad: Some(sp_data("term", TagMask::single(2).unwrap())),
            is_hidden: true,
            tags: TagMask::SCRATCHPAD,
            ..Client::default()
        };

        assert!(client.is_scratchpad());
        assert!(!client.is_minimized());
        assert!(!client.shows_in_bar(TagMask::single(1).unwrap()));
    }
}

/// Lightweight snapshot of a tiled client for layout calculations.
///
/// Layout algorithms collect these once and then work purely with
/// geometry — no further access to the model's client map needed.
#[derive(Debug, Clone, Copy)]
pub struct TiledClientInfo {
    pub win: WindowId,
    pub border_width: i32,
}

/// Iterator over a monitor's client list (focus order).
///
/// Yields `(Window, &Client)` pairs so call-sites keep the window id and the
/// corresponding client tightly coupled.
pub struct ClientListIter<'a> {
    iter: std::slice::Iter<'a, WindowId>,
    clients: &'a HashMap<WindowId, Client>,
}

impl<'a> ClientListIter<'a> {
    #[inline]
    pub fn new(clients: &'a [WindowId], map: &'a HashMap<WindowId, Client>) -> Self {
        Self {
            iter: clients.iter(),
            clients: map,
        }
    }
}

impl<'a> Iterator for ClientListIter<'a> {
    type Item = (WindowId, &'a Client);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let win = match self.iter.next() {
                Some(&w) => w,
                None => return None,
            };
            if let Some(c) = self.clients.get(&win) {
                return Some((win, c));
            }
        }
    }
}

/// Iterator over a monitor's persistent z-order.
///
/// Yields `(Window, &Client)` pairs so z-order/showhide style logic can use the
/// correct ordering while keeping the window id available.
///
/// This uses the same implementation as [`ClientListIter`] — the distinction
/// is semantic (stacking order vs focus order).
pub struct ClientStackIter<'a>(ClientListIter<'a>);

impl<'a> ClientStackIter<'a> {
    #[inline]
    pub fn new(stack: &'a [WindowId], map: &'a HashMap<WindowId, Client>) -> Self {
        Self(ClientListIter::new(stack, map))
    }
}

impl<'a> Iterator for ClientStackIter<'a> {
    type Item = <ClientListIter<'a> as Iterator>::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}
