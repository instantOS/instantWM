//! Geometry types for window positioning and sizing.
//!
//! Provides types for rectangles, size hints, and geometric calculations.

/// Parsed monitor position configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorPosition {
    Absolute {
        x: i32,
        y: i32,
    },
    Relative {
        relation: RelativePosition,
        output: String,
    },
}

/// Relative placement of one monitor against another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativePosition {
    LeftOf,
    RightOf,
    Above,
    Below,
}

impl MonitorPosition {
    /// Parse monitor placement syntax.
    ///
    /// Supported formats:
    /// - `"X,Y"`
    /// - `"left-of:DP-1"`
    /// - `"right-of:DP-1"`
    /// - `"above:DP-1"`
    /// - `"below:DP-1"`
    pub fn parse(value: &str) -> Option<Self> {
        if let Some((x_str, y_str)) = value.split_once(',') {
            let x = x_str.trim().parse().ok()?;
            let y = y_str.trim().parse().ok()?;
            return Some(Self::Absolute { x, y });
        }

        let (relation, output) = value.split_once(':')?;
        let relation = match relation.trim().to_ascii_lowercase().as_str() {
            "left-of" => RelativePosition::LeftOf,
            "right-of" => RelativePosition::RightOf,
            "above" => RelativePosition::Above,
            "below" => RelativePosition::Below,
            _ => return None,
        };
        let output = output.trim();
        if output.is_empty() {
            return None;
        }

        Some(Self::Relative {
            relation,
            output: output.to_string(),
        })
    }

    /// Resolve the configured position against known output geometries.
    pub fn resolve<'a>(
        &self,
        current_size: (i32, i32),
        outputs: impl IntoIterator<Item = (&'a str, Rect)>,
    ) -> Option<(i32, i32)> {
        match self {
            Self::Absolute { x, y } => Some((*x, *y)),
            Self::Relative { relation, output } => {
                let reference = outputs
                    .into_iter()
                    .find_map(|(name, rect)| (name == output).then_some(rect))?;

                Some(match relation {
                    RelativePosition::LeftOf => (reference.x - current_size.0, reference.y),
                    RelativePosition::RightOf => (reference.x + reference.w, reference.y),
                    RelativePosition::Above => (reference.x, reference.y - current_size.1),
                    RelativePosition::Below => (reference.x, reference.y + reference.h),
                })
            }
        }
    }
}

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

    /// Calculate the intersection rectangle with another.
    /// Returns `None` if the rectangles don't intersect.
    #[inline]
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.w).min(other.x + other.w);
        let y2 = (self.y + self.h).min(other.y + other.h);
        if x2 <= x1 || y2 <= y1 {
            return None;
        }
        Some(Rect {
            x: x1,
            y: y1,
            w: x2 - x1,
            h: y2 - y1,
        })
    }

    /// Subtract another rectangle from this one, returning the remaining parts.
    /// Returns a vector of rectangles that cover `self` minus `other`.
    pub fn subtract(&self, other: &Rect) -> Vec<Rect> {
        if self.w <= 0 || self.h <= 0 {
            return Vec::new();
        }
        let Some(i) = self.intersection(other) else {
            return vec![*self];
        };

        let mut out = Vec::with_capacity(4);
        if i.y > self.y {
            out.push(Rect {
                x: self.x,
                y: self.y,
                w: self.w,
                h: i.y - self.y,
            });
        }
        let self_bottom = self.y + self.h;
        let i_bottom = i.y + i.h;
        if i_bottom < self_bottom {
            out.push(Rect {
                x: self.x,
                y: i_bottom,
                w: self.w,
                h: self_bottom - i_bottom,
            });
        }
        if i.x > self.x {
            out.push(Rect {
                x: self.x,
                y: i.y,
                w: i.x - self.x,
                h: i.h,
            });
        }
        let self_right = self.x + self.w;
        let i_right = i.x + i.w;
        if i_right < self_right {
            out.push(Rect {
                x: i_right,
                y: i.y,
                w: self_right - i_right,
                h: i.h,
            });
        }
        out.into_iter().filter(|r| r.w > 0 && r.h > 0).collect()
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

    /// Check if a point is in the resize-border zone around this rectangle.
    ///
    /// The zone is a `border_zone`-pixel band around the outside of the
    /// rectangle. Points inside the rectangle content are not considered part
    /// of the resize border.
    #[inline]
    pub fn contains_resize_border_point(&self, x: i32, y: i32, border_zone: i32) -> bool {
        if x > self.x && x < self.x + self.w && y > self.y && y < self.y + self.h {
            return false;
        }
        if y < self.y - border_zone
            || x < self.x - border_zone
            || y > self.y + self.h + border_zone
            || x > self.x + self.w + border_zone
        {
            return false;
        }
        true
    }

    /// `true` when (`root_x`, `root_y`) lies on the top-middle segment of the resize border
    /// — used to treat a click as *move* rather than resize.
    #[inline]
    pub fn is_at_top_middle_edge(&self, root_x: i32, root_y: i32, border_zone: i32) -> bool {
        let at_top = root_y >= self.y - border_zone && root_y < self.y + border_zone;
        let in_middle_third = root_x >= self.x + self.w / 3 && root_x <= self.x + 2 * self.w / 3;
        at_top && in_middle_third
    }
}

#[cfg(test)]
mod tests {
    use super::{MonitorPosition, Rect, RelativePosition};

    #[test]
    fn parses_absolute_monitor_position() {
        assert_eq!(
            MonitorPosition::parse("1920,0"),
            Some(MonitorPosition::Absolute { x: 1920, y: 0 })
        );
    }

    #[test]
    fn parses_relative_monitor_position() {
        assert_eq!(
            MonitorPosition::parse("left-of:DP-1"),
            Some(MonitorPosition::Relative {
                relation: RelativePosition::LeftOf,
                output: "DP-1".to_string(),
            })
        );
    }

    #[test]
    fn resolves_relative_monitor_position() {
        let pos = MonitorPosition::parse("below:DP-1").unwrap();
        let outputs = [("DP-1", Rect::new(1920, 0, 2560, 1440))];

        assert_eq!(pos.resolve((1920, 1080), outputs), Some((1920, 1440)));
    }

    #[test]
    fn rejects_unknown_monitor_position_syntax() {
        assert_eq!(MonitorPosition::parse("diagonal-of:DP-1"), None);
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
