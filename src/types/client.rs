//! Backend-neutral managed-client state.
//!
//! The `Client` aggregate remains here; invariant-bearing supporting types and
//! their transitions live in focused submodules.

mod geometry;
mod iter;
mod mode;
mod scratchpad;

pub use geometry::{FloatingGeometryState, SavedFloatingPlacement};
pub use iter::{OrderedClients, TiledClientInfo};
pub use mode::{
    ClientMode, ClientPlacement, ClientPresentation, FullscreenKind, MaximizedOrigin,
    RestoredPresentation,
};
pub use scratchpad::ScratchpadData;

use crate::types::core::MonitorId;
use crate::types::geometry::{Rect, SizeHints};
use crate::types::input::SnapPosition;
use crate::types::{TagMask, WindowId};

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
    /// Authoritative optional scratchpad role.
    #[cfg(not(test))]
    scratchpad: Option<ScratchpadData>,
    /// Tests may continue to use struct-update fixtures while production code
    /// can only change the role through explicit transitions.
    #[cfg(test)]
    pub(crate) scratchpad: Option<ScratchpadData>,
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

    /// Assign a new tag bitmask.
    pub(crate) fn set_tag_mask(&mut self, tags: TagMask) {
        debug_assert!(
            !self.is_scratchpad(),
            "scratchpad tags are role-owned; restore the client before assigning normal tags"
        );
        self.tags = tags;
    }

    /// Transform the tag bitmask in place.
    pub(crate) fn update_tag_mask(&mut self, f: impl FnOnce(TagMask) -> TagMask) {
        debug_assert!(
            !self.is_scratchpad(),
            "scratchpad tags are role-owned; restore the client before transforming tags"
        );
        self.tags = f(self.tags);
    }

    /// Check if the client is on the selected tags, ignoring hidden state.
    #[inline]
    pub fn is_on_selected_tags(&self, selected_tags: TagMask) -> bool {
        self.is_scratchpad_visible() || self.is_sticky || self.tags.intersects(selected_tags)
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
            self.is_scratchpad_visible()
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
}

#[cfg(test)]
mod tests {
    use super::{Client, ClientMode, ClientPlacement, MaximizedOrigin};
    use crate::types::{EdgeDirection, MonitorId, Rect, TagMask};

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
    fn scratchpad_focus_restore_is_consumed_once() {
        let mut client = Client::default();
        client
            .promote_to_scratchpad("menu", None, 1920, 1080)
            .unwrap();
        let scratchpad = client.scratchpad.as_mut().unwrap();
        scratchpad.remember_focus(Some(crate::types::WindowId(42)));

        assert_eq!(
            scratchpad.take_restore_focus(),
            Some(crate::types::WindowId(42))
        );
        assert_eq!(scratchpad.take_restore_focus(), None);
    }

    #[test]
    fn scratchpad_role_is_authoritative_and_independent_of_presentation_flags() {
        let mut client = Client {
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };
        client
            .promote_to_scratchpad("term", None, 1920, 1080)
            .unwrap();

        client.tags = TagMask::single(3).unwrap();
        client.is_sticky = true;
        assert!(client.is_sticky);
        assert!(client.is_scratchpad());
        client.is_sticky = false;
        client.is_hidden = true;
        assert!(client.is_scratchpad());
    }

    #[test]
    fn visible_scratchpad_visibility_does_not_depend_on_tags_or_sticky() {
        let mut client = Client {
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };
        client
            .promote_to_scratchpad("term", None, 1920, 1080)
            .unwrap();

        assert!(!client.is_sticky);
        assert_eq!(client.tags, TagMask::SCRATCHPAD);
        assert!(client.is_visible(TagMask::single(9).unwrap()));
        assert!(client.shows_in_bar(TagMask::single(9).unwrap()));
    }

    #[test]
    fn scratchpad_restore_recovers_complete_ordinary_window_state() {
        let mut client = Client {
            monitor_id: MonitorId::default(),
            tags: TagMask::single(2).unwrap(),
            is_sticky: true,
            is_locked: true,
            geo: Rect::new(10, 20, 800, 600),
            border_width: 3,
            ..Client::default()
        };
        client.set_placement(ClientPlacement::Tiling);
        client
            .promote_to_scratchpad("term", Some(EdgeDirection::Top), 1920, 1080)
            .unwrap();

        assert!(client.is_scratchpad());
        assert_eq!(client.tags, TagMask::SCRATCHPAD);
        assert!(!client.is_sticky);
        assert_eq!(client.border_width, 0);
        assert!(client.mode().is_normal_floating());

        client.restore_from_scratchpad(None).unwrap();

        assert!(!client.is_scratchpad());
        assert_eq!(client.tags, TagMask::single(2).unwrap());
        assert!(client.is_sticky);
        assert!(client.is_locked);
        assert_eq!(client.geo, Rect::new(10, 20, 800, 600));
        assert_eq!(client.border_width, 3);
        assert!(client.mode().is_normal_tiling());
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
        let mut client = Client::default();
        client
            .promote_to_scratchpad("term", None, 1920, 1080)
            .unwrap();
        client.is_hidden = true;

        assert!(client.is_scratchpad());
        assert!(!client.is_minimized());
        assert!(!client.shows_in_bar(TagMask::single(1).unwrap()));
    }
}
