//! Geometry types for window positioning and sizing.
//!
//! Provides types for rectangles, size hints, and geometric calculations.

/// A rectangle representing window geometry or screen areas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// X coordinate (horizontal position).
    pub x: i32,
    /// Y coordinate (vertical position).
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl Rect {
    /// Create a new Rect with the given dimensions.
    #[inline]
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Calculate the area of this rectangle.
    #[inline]
    pub fn area(&self) -> i32 {
        self.w * self.h
    }

    /// Check if a point is contained within this rectangle.
    #[inline]
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// Check if this rectangle intersects with another.
    #[inline]
    pub fn intersects_other(&self, other: &Rect) -> bool {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.w).min(other.x + other.w);
        let y2 = (self.y + self.h).min(other.y + other.h);
        x1 < x2 && y1 < y2
    }

    /// Get the center point of this rectangle.
    #[inline]
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// Calculate total width including borders.
    #[inline]
    pub fn total_width(&self, border_width: i32) -> i32 {
        self.w + 2 * border_width
    }

    /// Calculate total height including borders.
    #[inline]
    pub fn total_height(&self, border_width: i32) -> i32 {
        self.h + 2 * border_width
    }

    /// Convert to a 4-tuple (x, y, w, h).
    #[inline]
    pub fn as_tuple(&self) -> (i32, i32, i32, i32) {
        (self.x, self.y, self.w, self.h)
    }

    /// Create a Rect from a 4-tuple.
    #[inline]
    pub fn from_tuple((x, y, w, h): (i32, i32, i32, i32)) -> Self {
        Self { x, y, w, h }
    }

    /// Create a new Rect with adjusted position.
    #[inline]
    pub fn with_pos(&self, x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            w: self.w,
            h: self.h,
        }
    }

    /// Create a new Rect with adjusted size.
    #[inline]
    pub fn with_size(&self, w: i32, h: i32) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w,
            h,
        }
    }

    /// Create a new Rect with borders subtracted from size.
    #[inline]
    pub fn without_borders(&self, border_width: i32) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w: self.w - 2 * border_width,
            h: self.h - 2 * border_width,
        }
    }

    /// Create a new Rect with borders added to size.
    #[inline]
    pub fn with_borders(&self, border_width: i32) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w: self.w + 2 * border_width,
            h: self.h + 2 * border_width,
        }
    }

    /// Check if this rect has valid positive dimensions.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0
    }

    /// Clamp position to keep the window within the given bounds.
    ///
    /// Ensures the window doesn't escape the usable area by adjusting
    /// x and y so at least part of the window remains visible.
    #[inline]
    pub fn clamp_position(&mut self, bounds: &Rect, total_w: i32, total_h: i32) {
        let right_bound = bounds.x + bounds.w;
        let bottom_bound = bounds.y + bounds.h;

        if self.x > right_bound {
            self.x = right_bound - total_w;
        }
        if self.y > bottom_bound {
            self.y = bottom_bound - total_h;
        }
        if self.x + total_w < bounds.x {
            self.x = bounds.x;
        }
        if self.y + total_h < bounds.y {
            self.y = bounds.y;
        }
    }

    /// Ensure minimum dimensions.
    #[inline]
    pub fn enforce_minimum(&mut self, min_w: i32, min_h: i32) {
        self.w = self.w.max(min_w);
        self.h = self.h.max(min_h);
    }

    /// Check if this rect differs from another.
    #[inline]
    pub fn differs_from(&self, other: &Rect) -> bool {
        self.x != other.x || self.y != other.y || self.w != other.w || self.h != other.h
    }
}

/// Size hints for a client window (from WM_NORMAL_HINTS).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizeHints {
    /// Base width for size calculations.
    pub basew: i32,
    /// Base height for size calculations.
    pub baseh: i32,
    /// Width increment for sizing steps.
    pub incw: i32,
    /// Height increment for sizing steps.
    pub inch: i32,
    /// Maximum allowed width.
    pub maxw: i32,
    /// Maximum allowed height.
    pub maxh: i32,
    /// Minimum allowed width.
    pub minw: i32,
    /// Minimum allowed height.
    pub minh: i32,
    /// Minimum aspect ratio numerator.
    pub min_aspect_num: i32,
    /// Minimum aspect ratio denominator.
    pub min_aspect_denom: i32,
    /// Maximum aspect ratio numerator.
    pub max_aspect_num: i32,
    /// Maximum aspect ratio denominator.
    pub max_aspect_denom: i32,
}

impl SizeHints {
    /// Check if base size equals min size.
    #[inline]
    pub fn base_is_min(&self) -> bool {
        self.basew == self.minw && self.baseh == self.minh
    }

    /// Apply size constraints to the given dimensions.
    ///
    /// Returns the constrained (width, height) after applying:
    /// - Base size subtraction/addition
    /// - Aspect ratio constraints
    /// - Resize increments
    /// - Min/max bounds
    pub fn constrain_size(
        &self,
        mut w: i32,
        mut h: i32,
        min_aspect: f32,
        max_aspect: f32,
    ) -> (i32, i32) {
        let base_is_min = self.base_is_min();

        // Step 1: subtract base size before aspect / increment checks.
        if !base_is_min {
            w -= self.basew;
            h -= self.baseh;
        }

        // Step 2: enforce aspect ratio.
        if min_aspect > 0.0 && max_aspect > 0.0 {
            let current_aspect = w as f32 / h as f32;
            if max_aspect < current_aspect {
                w = (h as f32 * max_aspect + 0.5) as i32;
            } else if min_aspect < (h as f32) / (w as f32) {
                h = (w as f32 * min_aspect + 0.5) as i32;
            }
        }

        // Step 3: when base == min, subtract base *after* the aspect check.
        if base_is_min {
            w -= self.basew;
            h -= self.baseh;
        }

        // Step 4: snap to resize increments.
        if self.incw != 0 {
            w -= w % self.incw;
        }
        if self.inch != 0 {
            h -= h % self.inch;
        }

        // Step 5: re-add base and clamp to [min, max].
        w = (w + self.basew).max(self.minw);
        h = (h + self.baseh).max(self.minh);

        if self.maxw != 0 {
            w = w.min(self.maxw);
        }
        if self.maxh != 0 {
            h = h.min(self.maxh);
        }

        (w, h)
    }

    /// Check if this represents a fixed-size window (max == min != 0).
    #[inline]
    pub fn is_fixed(&self) -> bool {
        self.maxw != 0 && self.maxh != 0 && self.maxw == self.minw && self.maxh == self.minh
    }
}
