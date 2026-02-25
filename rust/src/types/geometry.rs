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
    /// Calculate the area of this rectangle.
    #[inline]
    pub fn area(&self) -> i32 {
        self.w * self.h
    }

    /// Check if a point is contained within this rectangle.
    #[inline]
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
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
    pub min_aspect_n: i32,
    /// Minimum aspect ratio denominator.
    pub min_aspect_d: i32,
    /// Maximum aspect ratio numerator.
    pub max_aspect_n: i32,
    /// Maximum aspect ratio denominator.
    pub max_aspect_d: i32,
}
