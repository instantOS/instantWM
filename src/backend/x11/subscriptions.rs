//! Owned root/client subscriptions and temporary notification suppression.

use std::collections::HashMap;
use x11rb::errors::ConnectionError;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt, EventMask, Window};
use x11rb::rust_connection::RustConnection;

#[derive(Default)]
pub(crate) struct EventSubscriptions {
    masks: HashMap<Window, EventMask>,
}

#[derive(Debug, thiserror::Error)]
#[error("X11 event subscription for window {0:#x} is not owned")]
pub(crate) struct UnknownSubscription(Window);

impl EventSubscriptions {
    pub(crate) fn select(
        &mut self,
        conn: &RustConnection,
        window: Window,
        mask: EventMask,
    ) -> Result<(), ConnectionError> {
        conn.change_window_attributes(window, &ChangeWindowAttributesAux::new().event_mask(mask))?;
        self.masks.insert(window, mask);
        Ok(())
    }

    pub(crate) fn release(&mut self, conn: &RustConnection, window: Window, destroyed: bool) {
        if !destroyed {
            let _ = conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );
        }
        self.masks.remove(&window);
    }

    pub(crate) fn suppress_unmap<'a>(
        &'a self,
        conn: &'a RustConnection,
        root: Window,
        window: Window,
    ) -> Result<UnmapSuppression<'a>, UnknownSubscription> {
        // Resolve both before sending any requests. An explicitly empty mask
        // is valid; absent ownership is an error, never an inferred empty mask.
        let root_mask = self.masks.get(&root).ok_or(UnknownSubscription(root))?;
        let window_mask = self.masks.get(&window).ok_or(UnknownSubscription(window))?;
        let guard = UnmapSuppression {
            conn,
            root,
            window,
            root_mask,
            window_mask,
        };
        let suppressed_root =
            EventMask::from(root_mask.bits() & !EventMask::SUBSTRUCTURE_NOTIFY.bits());
        let suppressed_window =
            EventMask::from(window_mask.bits() & !EventMask::STRUCTURE_NOTIFY.bits());
        let _ = conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(suppressed_root),
        );
        let _ = conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().event_mask(suppressed_window),
        );
        Ok(guard)
    }
}

pub(crate) struct UnmapSuppression<'a> {
    conn: &'a RustConnection,
    root: Window,
    window: Window,
    // Borrowing the owned masks prevents subscription changes until restore.
    root_mask: &'a EventMask,
    window_mask: &'a EventMask,
}

impl Drop for UnmapSuppression<'_> {
    fn drop(&mut self) {
        let _ = self.conn.change_window_attributes(
            self.root,
            &ChangeWindowAttributesAux::new().event_mask(*self.root_mask),
        );
        let _ = self.conn.change_window_attributes(
            self.window,
            &ChangeWindowAttributesAux::new().event_mask(*self.window_mask),
        );
        // The enclosing server grab releases and flushes the transaction.
    }
}
