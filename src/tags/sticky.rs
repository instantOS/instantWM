//! Sticky-client helpers.
//!
//! A "sticky" client is one that appears on every tag simultaneously.  When
//! such a client is moved to a specific tag (e.g. via a shift or monitor
//! transfer) it must lose its sticky status so it stops following every view.

use crate::types::Client;

impl Client {
    /// Resets sticky status for this client, moving it to the given tag.
    pub fn reset_sticky(&mut self, target_tag: Option<usize>) {
        if !self.is_sticky {
            return;
        }
        self.is_sticky = false;
        if let Some(tag) = target_tag {
            let tags = crate::types::TagMask::single(tag).unwrap_or_default();
            self.set_tag_mask(tags);
        }
    }
}
