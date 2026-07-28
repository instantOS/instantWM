//! Client-owned geometry history and floating placement state.

use super::Client;
use crate::types::{Rect, Size};

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

impl Client {
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
        self.geo.with_borders(self.border_width)
    }

    /// Update border width while preserving the client's outer dimensions.
    pub fn set_border_width(&mut self, new_width: i32) {
        let outer_rect = self.geo.with_borders(self.border_width);
        self.border_width = new_width;
        self.update_geometry(outer_rect.without_borders(new_width));
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
}

#[cfg(test)]
mod tests {
    use super::Client;
    use crate::types::Rect;

    #[test]
    fn repeated_update_preserves_previous_distinct_rectangle() {
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
    fn placement_changes_do_not_discard_saved_floating_geometry() {
        let saved = Rect::new(100, 120, 640, 480);
        let mut client = Client {
            geo: Rect::new(0, 0, 1920, 1080),
            ..Client::default()
        };
        client.save_floating_placement(saved, Rect::new(0, 0, 1920, 1080));

        client.set_placement(crate::types::ClientPlacement::Tiling);

        assert_eq!(client.mode(), crate::types::ClientMode::tiled());
        assert_eq!(client.saved_floating_rect(), Some(saved));
    }

    #[test]
    fn changing_border_width_preserves_the_outer_rectangle() {
        let mut client = Client {
            geo: Rect::new(40, 60, 800, 600),
            border_width: 2,
            ..Client::default()
        };
        let outer = client.total_rect();

        client.set_border_width(5);

        assert_eq!(client.total_rect(), outer);
        assert_eq!(client.geo, Rect::new(40, 60, 794, 594));
    }
}
