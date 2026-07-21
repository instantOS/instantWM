use super::*;

/// Persistent per-monitor client z-order.
///
/// The stored order is bottom-to-top. Layout policy may project this into a
/// different backend order temporarily (for example, maximized presentation promotes the
/// focused client visually), but focus changes alone should not mutate this
/// persistent order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientZOrder {
    bottom_to_top: Vec<WindowId>,
}

impl ClientZOrder {
    pub fn as_slice(&self) -> &[WindowId] {
        &self.bottom_to_top
    }

    pub fn attach_top(&mut self, win: WindowId) {
        self.remove(win);
        self.bottom_to_top.push(win);
    }

    pub fn attach_bottom(&mut self, win: WindowId) {
        self.remove(win);
        self.bottom_to_top.insert(0, win);
    }

    pub fn remove(&mut self, win: WindowId) -> bool {
        let old_len = self.bottom_to_top.len();
        self.bottom_to_top.retain(|&w| w != win);
        self.bottom_to_top.len() != old_len
    }

    pub fn raise(&mut self, win: WindowId) -> bool {
        if !self.remove(win) {
            return false;
        }
        self.bottom_to_top.push(win);
        true
    }

    pub fn lower(&mut self, win: WindowId) -> bool {
        if !self.remove(win) {
            return false;
        }
        self.bottom_to_top.insert(0, win);
        true
    }

    pub fn iter_bottom_to_top(&self) -> impl DoubleEndedIterator<Item = WindowId> + '_ {
        self.bottom_to_top.iter().copied()
    }

    pub fn iter_top_to_bottom(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.bottom_to_top.iter().rev().copied()
    }
}
