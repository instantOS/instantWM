//! Scratchpad metadata and client-local scratchpad transitions.

use super::{Client, ClientPlacement};
use crate::types::{EdgeDirection, TagMask, WindowId};

/// Scratchpad-specific state for a window.
#[derive(Debug, Clone, Default)]
pub struct ScratchpadData {
    pub name: String,
    pub restore_tags: TagMask,
    pub direction: Option<EdgeDirection>,
    /// Window that had focus when this scratchpad was shown.
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

impl Client {
    /// Check if this client is a scratchpad window.
    pub fn is_scratchpad(&self) -> bool {
        self.scratchpad.is_some()
            && (self.tags.is_scratchpad_only() || self.is_hidden || self.is_sticky)
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

    pub fn clear_scratchpad_state(&mut self) {
        self.scratchpad = None;
        self.is_sticky = false;
    }

    pub fn sync_scratchpad_state(&mut self) {
        if self.scratchpad.is_some()
            && !self.tags.is_scratchpad_only()
            && !self.is_hidden
            && !self.is_sticky
        {
            self.clear_scratchpad_state();
        }
    }

    pub fn clear_sticky_if_scratchpad(&mut self) {
        if self.tags.is_scratchpad_only() {
            self.is_sticky = false;
        }
    }

    /// Apply all client-local state required when making a window a scratchpad.
    pub fn apply_scratchpad_state(
        &mut self,
        name: &str,
        direction: Option<EdgeDirection>,
        restore_tags: TagMask,
        monitor_width: i32,
        monitor_height: i32,
    ) {
        self.scratchpad = Some(ScratchpadData {
            name: name.to_string(),
            restore_tags,
            direction,
            restore_focus: None,
        });
        self.set_tag_mask(TagMask::SCRATCHPAD);
        self.is_sticky = false;
        self.set_placement(ClientPlacement::Floating);

        if let Some(direction) = direction {
            if direction.is_vertical() {
                self.geo.h = monitor_height / 3;
            } else {
                self.geo.w = monitor_width / 3;
            }
            self.save_border_width();
            self.border_width = 0;
            self.is_locked = true;
        }
    }

    pub fn exit_scratchpad_state(&mut self, restore_tags: TagMask, had_direction: bool) {
        self.set_tag_mask(restore_tags);
        if had_direction {
            self.restore_border_width();
            self.is_locked = false;
        }
    }

    pub fn show_as_scratchpad(&mut self, tags: TagMask, direction: Option<EdgeDirection>) {
        self.is_sticky = true;
        self.set_placement(ClientPlacement::Floating);
        if direction.is_some() {
            self.border_width = 0;
        }
        self.set_tag_mask(tags);
    }

    pub fn hide_as_scratchpad(&mut self) {
        self.is_sticky = false;
        self.set_tag_mask(TagMask::SCRATCHPAD);
    }
}
