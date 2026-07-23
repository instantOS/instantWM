//! Client/window management types.
//!
//! Types for managed windows and client lists.

use std::collections::HashMap;

use crate::types::TagMask;
use crate::types::WindowId;
use crate::types::core::MonitorId;
use crate::types::geometry::{Rect, SizeHints};
use crate::types::input::{EdgeDirection, SnapPosition};

/// Base mode to restore after temporary modes such as fullscreen or maximized.
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
pub enum BaseClientMode {
    #[default]
    Tiling,
    Floating,
}

/// Mutually exclusive client placement mode.
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
pub enum ClientMode {
    #[default]
    Tiling,
    Floating,
    TrueFullscreen {
        restore: BaseClientMode,
    },
    FakeFullscreen {
        restore: BaseClientMode,
    },
    Maximized {
        restore: BaseClientMode,
    },
}

impl ClientMode {
    #[inline]
    pub fn is_fullscreen(self) -> bool {
        matches!(
            self,
            Self::TrueFullscreen { .. } | Self::FakeFullscreen { .. }
        )
    }

    #[inline]
    pub fn is_true_fullscreen(self) -> bool {
        matches!(self, Self::TrueFullscreen { .. })
    }

    #[inline]
    pub fn is_fake_fullscreen(self) -> bool {
        matches!(self, Self::FakeFullscreen { .. })
    }

    #[inline]
    pub fn is_maximized(self) -> bool {
        matches!(self, Self::Maximized { .. })
    }

    #[inline]
    pub fn is_floating(self) -> bool {
        matches!(self, Self::Floating)
    }

    #[inline]
    pub fn is_tiling(self) -> bool {
        matches!(self, Self::Tiling)
    }

    #[inline]
    pub fn is_free_positioned(self) -> bool {
        matches!(self, Self::Floating | Self::Maximized { .. })
    }

    #[inline]
    pub fn restore_mode(self) -> Option<BaseClientMode> {
        match self {
            Self::Tiling | Self::Floating => None,
            Self::TrueFullscreen { restore }
            | Self::FakeFullscreen { restore }
            | Self::Maximized { restore } => Some(restore),
        }
    }

    #[inline]
    pub fn base_mode(self) -> BaseClientMode {
        match self {
            Self::Tiling => BaseClientMode::Tiling,
            Self::Floating => BaseClientMode::Floating,
            Self::TrueFullscreen { restore }
            | Self::FakeFullscreen { restore }
            | Self::Maximized { restore } => restore,
        }
    }

    /// Replace the persistent placement mode without discarding a temporary
    /// presentation mode.
    ///
    /// Rules and policy refreshes may change whether a client should restore
    /// to tiling or floating, but they do not own fullscreen/maximized state.
    #[inline]
    pub fn with_base_mode(self, base: BaseClientMode) -> Self {
        match self {
            Self::TrueFullscreen { .. } => Self::TrueFullscreen { restore: base },
            Self::FakeFullscreen { .. } => Self::FakeFullscreen { restore: base },
            Self::Maximized { .. } => Self::Maximized { restore: base },
            Self::Tiling | Self::Floating => match base {
                BaseClientMode::Tiling => Self::Tiling,
                BaseClientMode::Floating => Self::Floating,
            },
        }
    }

    #[inline]
    pub fn as_fullscreen(self) -> Self {
        Self::TrueFullscreen {
            restore: self.base_mode(),
        }
    }

    #[inline]
    pub fn as_fake_fullscreen(self) -> Self {
        Self::FakeFullscreen {
            restore: self.base_mode(),
        }
    }

    #[inline]
    pub fn as_maximized(self) -> Self {
        Self::Maximized {
            restore: self.base_mode(),
        }
    }

    #[inline]
    pub fn restored(self) -> Self {
        match self.restore_mode() {
            Some(BaseClientMode::Tiling) => Self::Tiling,
            Some(BaseClientMode::Floating) => Self::Floating,
            None => self,
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
}

impl ScratchpadData {
    pub fn set_direction(&mut self, direction: EdgeDirection) {
        self.direction = Some(direction);
    }
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
    /// Geometry when floating.
    pub float_geo: Rect,
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
    pub fn base_mode(&self) -> BaseClientMode {
        self.mode.base_mode()
    }

    /// Change the persistent tiled/floating policy while preserving any
    /// temporary fullscreen or maximized presentation.
    ///
    /// Use [`Self::replace_mode_with_base`] only when an explicit user action
    /// should also leave the current presentation mode.
    #[inline]
    pub fn set_base_mode(&mut self, base: BaseClientMode) {
        self.mode = self.mode.with_base_mode(base);
    }

    /// Replace the complete mode with a base tiled/floating mode.
    ///
    /// This deliberately exits fullscreen/maximized presentation and does not
    /// modify saved floating geometry. Policy refreshes should normally use
    /// [`Self::set_base_mode`] instead.
    #[inline]
    pub fn replace_mode_with_base(&mut self, base: BaseClientMode) {
        self.mode = match base {
            BaseClientMode::Tiling => ClientMode::Tiling,
            BaseClientMode::Floating => ClientMode::Floating,
        };
    }

    /// Enter true fullscreen while remembering the current base placement.
    #[inline]
    pub fn enter_fullscreen(&mut self) {
        self.mode = self.mode().as_fullscreen();
    }

    /// Enter fake fullscreen while remembering the current base placement.
    #[inline]
    pub fn enter_fake_fullscreen(&mut self) {
        self.mode = self.mode().as_fake_fullscreen();
    }

    /// Enter maximized presentation while remembering the current base placement.
    #[inline]
    pub fn enter_maximized(&mut self) {
        self.mode = self.mode().as_maximized();
    }

    /// Leave a temporary presentation mode and restore its base placement.
    #[inline]
    pub fn restore_mode(&mut self) {
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
        self.mode().is_tiling() && self.is_visible(selected_tags)
    }

    /// Clear the urgency flag for this client.
    pub fn clear_urgency(&mut self) {
        self.is_urgent = false;
    }

    /// Returns the floating geometry if valid, otherwise falls back to current geometry.
    ///
    /// When a window has never been floated, `float_geo` is zeroed. This method
    /// provides the correct dimensions to use for floating: saved float dimensions
    /// if available, otherwise the current tiled dimensions.
    pub fn effective_float_geo(&self) -> Rect {
        if self.float_geo.is_valid() {
            self.float_geo
        } else {
            self.geo
        }
    }

    /// Returns the geometry to use when restoring a window from tiled to floating.
    ///
    /// If the window is already floating, returns current geometry.
    /// Otherwise returns effective float geometry (saved float dims or current tiled dims).
    pub fn restore_geo_for_float(&self) -> Rect {
        if self.mode().is_floating() {
            self.geo
        } else {
            self.effective_float_geo()
        }
    }

    pub fn save_floating_geometry(&mut self) {
        self.float_geo = self.geo;
    }

    pub fn update_geometry(&mut self, rect: Rect) {
        self.old_geo = self.geo;
        self.geo = rect;
        if self.mode().is_floating() {
            self.float_geo = rect;
        }
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
        });
        self.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
        self.is_sticky = false;
        if !self.mode().is_floating() {
            self.mode = ClientMode::Floating;
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
        self.mode = ClientMode::Floating;
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
    use super::{BaseClientMode, Client, ClientMode, ScratchpadData};
    use crate::types::{Rect, SCRATCHPAD_MASK, TagMask};

    #[test]
    fn fullscreen_restores_previous_tiling_mode() {
        let mut client = Client::default();

        client.enter_fullscreen();
        assert!(client.mode().is_true_fullscreen());
        assert!(!client.mode().is_tiling());

        client.restore_mode();
        assert_eq!(client.mode(), ClientMode::Tiling);
    }

    #[test]
    fn fullscreen_restores_previous_floating_mode() {
        let mut client = Client::default();
        client.replace_mode_with_base(BaseClientMode::Floating);

        client.enter_fullscreen();
        assert!(client.mode().is_true_fullscreen());
        assert!(!client.mode().is_floating());

        client.restore_mode();
        assert_eq!(client.mode(), ClientMode::Floating);
    }

    #[test]
    fn maximized_restores_previous_regular_mode() {
        let mut client = Client::default();
        client.replace_mode_with_base(BaseClientMode::Floating);

        client.enter_maximized();
        assert!(client.mode().is_maximized());
        assert!(!client.mode().is_floating());

        client.restore_mode();
        assert_eq!(client.mode(), ClientMode::Floating);
    }

    #[test]
    fn changing_base_mode_preserves_temporary_presentation() {
        for mode in [
            ClientMode::Tiling.as_fullscreen(),
            ClientMode::Tiling.as_fake_fullscreen(),
            ClientMode::Tiling.as_maximized(),
        ] {
            let changed = mode.with_base_mode(BaseClientMode::Floating);
            assert_eq!(
                std::mem::discriminant(&changed),
                std::mem::discriminant(&mode)
            );
            assert_eq!(changed.restored(), ClientMode::Floating);
        }
    }

    #[test]
    fn replacing_mode_does_not_implicitly_replace_saved_floating_geometry() {
        let saved = Rect::new(100, 120, 640, 480);
        let mut client = Client {
            geo: Rect::new(0, 0, 1920, 1080),
            float_geo: saved,
            ..Client::default()
        };

        client.replace_mode_with_base(BaseClientMode::Tiling);

        assert_eq!(client.mode(), ClientMode::Tiling);
        assert_eq!(client.float_geo, saved);
    }

    fn sp_data(name: &str, restore_tags: TagMask) -> ScratchpadData {
        ScratchpadData {
            name: name.to_string(),
            restore_tags,
            ..ScratchpadData::default()
        }
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
