//! Scratchpad metadata and client-local role transitions.

use super::{Client, ClientMode, FloatingGeometryState};
use crate::types::{
    EdgeDirection, MonitorId, Rect, SCRATCHPAD_NAME_LEN, SnapPosition, TagMask, WindowId,
};

/// State restored when a scratchpad returns to being an ordinary window.
///
/// Scratchpad promotion is a reversible role transition. Keeping the complete
/// client-owned snapshot here prevents scratchpad presentation from becoming
/// an implicit contract between tags, sticky state, geometry, and placement.
/// Mapping and monitor-list ownership remain orchestration concerns: restoring
/// a scratchpad intentionally surfaces it and reattaches it atomically.
#[derive(Debug, Clone)]
struct ScratchpadRestoreState {
    tags: TagMask,
    monitor_id: MonitorId,
    mode: ClientMode,
    is_sticky: bool,
    is_locked: bool,
    snap_status: SnapPosition,
    geo: Rect,
    old_geo: Rect,
    border_width: i32,
    old_border_width: i32,
    floating_geometry: FloatingGeometryState,
}

/// Scratchpad-specific role data.
#[derive(Debug, Clone)]
pub struct ScratchpadData {
    name: String,
    direction: Option<EdgeDirection>,
    restore: ScratchpadRestoreState,
    /// Window that had focus when this scratchpad was shown.
    restore_focus: Option<WindowId>,
}

impl ScratchpadData {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn direction(&self) -> Option<EdgeDirection> {
        self.direction
    }

    pub(crate) fn set_direction(&mut self, direction: EdgeDirection) {
        self.direction = Some(direction);
    }

    pub fn original_tags(&self) -> TagMask {
        self.restore.tags
    }

    pub fn original_monitor(&self) -> MonitorId {
        self.restore.monitor_id
    }

    pub(crate) fn remember_focus(&mut self, window: Option<WindowId>) {
        self.restore_focus = window;
    }

    pub(crate) fn take_restore_focus(&mut self) -> Option<WindowId> {
        self.restore_focus.take()
    }
}

impl Client {
    pub fn scratchpad(&self) -> Option<&ScratchpadData> {
        self.scratchpad.as_ref()
    }

    pub(crate) fn scratchpad_mut(&mut self) -> Option<&mut ScratchpadData> {
        self.scratchpad.as_mut()
    }

    /// Whether this client currently owns the scratchpad role.
    ///
    /// Role identity is deliberately independent of tags, sticky state, and
    /// mapping state. Those values describe orthogonal window behavior and
    /// must never silently create or destroy a scratchpad.
    pub fn is_scratchpad(&self) -> bool {
        self.scratchpad.is_some()
    }

    pub fn is_scratchpad_visible(&self) -> bool {
        self.is_scratchpad() && !self.is_hidden
    }

    pub fn is_edge_scratchpad(&self) -> bool {
        self.scratchpad
            .as_ref()
            .is_some_and(|scratchpad| scratchpad.direction.is_some())
    }

    #[inline]
    pub fn is_minimized(&self) -> bool {
        self.is_hidden && !self.is_scratchpad()
    }

    /// Promote an ordinary client into a named scratchpad role.
    pub(crate) fn promote_to_scratchpad(
        &mut self,
        name: &str,
        direction: Option<EdgeDirection>,
        monitor_width: i32,
        monitor_height: i32,
    ) -> Result<(), String> {
        if name.is_empty() {
            return Err("scratchpad name cannot be empty".to_string());
        }
        if name.chars().count() > SCRATCHPAD_NAME_LEN {
            return Err(format!(
                "scratchpad name cannot exceed {} characters",
                SCRATCHPAD_NAME_LEN
            ));
        }
        if self.is_scratchpad() {
            return Err(format!("window {} is already a scratchpad", self.win.0));
        }

        let restore = ScratchpadRestoreState {
            tags: self.tags,
            monitor_id: self.monitor_id,
            mode: self.mode,
            is_sticky: self.is_sticky,
            is_locked: self.is_locked,
            snap_status: self.snap_status,
            geo: self.geo,
            old_geo: self.old_geo,
            border_width: self.border_width,
            old_border_width: self.old_border_width,
            floating_geometry: self.floating_geometry.clone(),
        };
        self.scratchpad = Some(ScratchpadData {
            name: name.to_string(),
            direction,
            restore,
            restore_focus: None,
        });

        // Scratchpads are free-positioned role windows parked outside the
        // ordinary tag space. Their visibility is owned by `is_hidden`, not by
        // pretending that they are ordinary sticky clients.
        self.tags = TagMask::SCRATCHPAD;
        self.is_sticky = false;
        self.snap_status = SnapPosition::None;
        self.set_placement(super::ClientPlacement::Floating);

        if let Some(direction) = direction {
            if direction.is_vertical() {
                self.geo.h = monitor_height / 3;
            } else {
                self.geo.w = monitor_width / 3;
            }
            self.border_width = 0;
            self.is_locked = true;
        }

        Ok(())
    }

    /// Remove the scratchpad role and restore the captured ordinary state.
    ///
    /// `tags` allows an explicit user tag assignment to restore the window
    /// directly onto the requested tag. Otherwise its pre-scratchpad tags are
    /// restored.
    pub(crate) fn restore_from_scratchpad(&mut self, tags: Option<TagMask>) -> Result<(), String> {
        let Some(scratchpad) = self.scratchpad.take() else {
            return Err(format!("window {} is not a scratchpad", self.win.0));
        };
        let restore = scratchpad.restore;

        self.tags = tags.unwrap_or(restore.tags);
        self.mode = restore.mode;
        self.is_sticky = restore.is_sticky;
        self.is_locked = restore.is_locked;
        self.snap_status = restore.snap_status;
        self.geo = restore.geo;
        self.old_geo = restore.old_geo;
        self.border_width = restore.border_width;
        self.old_border_width = restore.old_border_width;
        self.floating_geometry = restore.floating_geometry;

        Ok(())
    }
}
