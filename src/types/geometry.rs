//! Geometry types for window positioning and sizing.
//!
//! Provides types for rectangles, size hints, and geometric calculations.

use super::input::SnapPosition;

/// Parsed monitor position configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorPosition {
    Absolute(Point),
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
            return Some(Self::Absolute(Point::new(x, y)));
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
        current_size: Size,
        outputs: impl IntoIterator<Item = (&'a str, Rect)>,
    ) -> Option<Point> {
        match self {
            Self::Absolute(position) => Some(*position),
            Self::Relative { relation, output } => {
                let reference = outputs
                    .into_iter()
                    .find_map(|(name, rect)| (name == output).then_some(rect))?;

                Some(match relation {
                    RelativePosition::LeftOf => {
                        Point::new(reference.x - current_size.w, reference.y)
                    }
                    RelativePosition::RightOf => Point::new(reference.x + reference.w, reference.y),
                    RelativePosition::Above => {
                        Point::new(reference.x, reference.y - current_size.h)
                    }
                    RelativePosition::Below => Point::new(reference.x, reference.y + reference.h),
                })
            }
        }
    }
}

/// A point representing a coordinate in 2D space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    /// X coordinate.
    pub x: i32,
    /// Y coordinate.
    pub y: i32,
}

impl Point {
    /// Create a new point.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Return the absolute difference between X coordinates.
    pub fn abs_diff_x(&self, other: &Point) -> i32 {
        (self.x - other.x).abs()
    }

    /// Return the absolute difference between Y coordinates.
    pub fn abs_diff_y(&self, other: &Point) -> i32 {
        (self.y - other.y).abs()
    }

    /// Calculate the Manhattan distance between this point and another.
    pub fn manhattan_distance(&self, other: &Point) -> i32 {
        self.abs_diff_x(other) + self.abs_diff_y(other)
    }

    /// Calculate a weighted distance used for directional focus scoring.
    pub fn weighted_distance(&self, other: &Point, weight_x: i32, weight_y: i32) -> i32 {
        self.abs_diff_x(other) * weight_x + self.abs_diff_y(other) * weight_y
    }
}

/// Two-dimensional dimensions without an associated position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

/// Widths inset from the four edges of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Insets {
    #[inline]
    pub const fn new(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    #[inline]
    pub const fn horizontal(self) -> i32 {
        self.left + self.right
    }

    #[inline]
    pub const fn vertical(self) -> i32 {
        self.top + self.bottom
    }
}

impl Size {
    #[inline]
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }

    #[inline]
    pub const fn is_positive(self) -> bool {
        self.w > 0 && self.h > 0
    }
}

/// A rectangle representing window geometry or screen areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    #[inline]
    pub const fn from_position_and_size(position: Point, size: Size) -> Self {
        Self::new(position.x, position.y, size.w, size.h)
    }

    #[inline]
    pub const fn position(self) -> Point {
        Point::new(self.x, self.y)
    }

    #[inline]
    pub const fn size(self) -> Size {
        Size::new(self.w, self.h)
    }

    /// Calculate the area of this rectangle.
    #[inline]
    pub fn area(&self) -> i32 {
        self.w * self.h
    }

    /// Check if a point is contained within this rectangle.
    #[inline]
    pub fn contains_point(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x < self.x + self.w
            && point.y >= self.y
            && point.y < self.y + self.h
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
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2, self.y + self.h / 2)
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

    /// Create a new Rect with adjusted position.
    #[inline]
    pub fn with_position(&self, position: Point) -> Self {
        Self {
            x: position.x,
            y: position.y,
            w: self.w,
            h: self.h,
        }
    }

    /// Create a new Rect with adjusted size.
    #[inline]
    pub fn with_size(&self, size: Size) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w: size.w,
            h: size.h,
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

    /// Return a new Rect with size clamped to fit within (max_w, max_h) and minimum 1x1.
    #[inline]
    pub fn clamped_to_monitor(&self, max_w: i32, max_h: i32) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: self.w.clamp(1, max_w),
            h: self.h.clamp(1, max_h),
        }
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
    pub fn contains_resize_border_point(&self, point: Point, border_zone: i32) -> bool {
        if point.x > self.x
            && point.x < self.x + self.w
            && point.y > self.y
            && point.y < self.y + self.h
        {
            return false;
        }
        if point.y < self.y - border_zone
            || point.x < self.x - border_zone
            || point.y > self.y + self.h + border_zone
            || point.x > self.x + self.w + border_zone
        {
            return false;
        }
        true
    }

    /// `true` when `point` lies on the top-middle segment of the resize border
    /// — used to treat a click as *move* rather than resize.
    #[inline]
    pub fn is_at_top_middle_edge(&self, point: Point, border_zone: i32) -> bool {
        let at_top = point.y >= self.y - border_zone && point.y < self.y + border_zone;
        let in_middle_third = point.x >= self.x + self.w / 3 && point.x <= self.x + 2 * self.w / 3;
        at_top && in_middle_third
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MonitorPosition, Point, Rect, RelativePosition, Size, constraints_prefer_floating,
    };

    #[test]
    fn rect_exposes_typed_position_and_size() {
        let rect = Rect::new(10, 20, 800, 600);

        assert_eq!(rect.position(), Point::new(10, 20));
        assert_eq!(rect.size(), Size::new(800, 600));
        assert_eq!(
            Rect::from_position_and_size(rect.position(), rect.size()),
            rect
        );
    }

    #[test]
    fn size_requires_both_dimensions_to_be_positive() {
        assert!(Size::new(1, 1).is_positive());
        assert!(!Size::new(0, 1).is_positive());
        assert!(!Size::new(1, -1).is_positive());
    }

    #[test]
    fn parses_absolute_monitor_position() {
        assert_eq!(
            MonitorPosition::parse("1920,0"),
            Some(MonitorPosition::Absolute(Point::new(1920, 0)))
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

        assert_eq!(
            pos.resolve(Size::new(1920, 1080), outputs),
            Some(Point::new(1920, 1440))
        );
    }

    #[test]
    fn rejects_unknown_monitor_position_syntax() {
        assert_eq!(MonitorPosition::parse("diagonal-of:DP-1"), None);
    }

    #[test]
    fn fixed_constraint_policy_matches_all_backends() {
        assert!(constraints_prefer_floating(640, 480, 640, 480));
        assert!(constraints_prefer_floating(640, 480, 640, 1080));
        assert!(constraints_prefer_floating(640, 480, 1920, 480));
        assert!(!constraints_prefer_floating(640, 480, 1920, 1080));
        assert!(!constraints_prefer_floating(0, 0, 0, 0));
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
    /// Returns the constrained size after applying:
    /// - Base size subtraction/addition
    /// - Aspect ratio constraints
    /// - Resize increments
    /// - Min/max bounds
    pub fn constrain_size(&self, size: Size, min_aspect: f32, max_aspect: f32) -> Size {
        let Size { mut w, mut h } = size;
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

        Size::new(w, h)
    }

    /// Check whether these constraints make tiling unsuitable.
    ///
    /// As in Sway, a window with a positive minimum in both dimensions and a
    /// fixed width or height is treated as fixed-size for placement policy.
    #[inline]
    pub fn is_fixed(&self) -> bool {
        constraints_prefer_floating(self.minw, self.minh, self.maxw, self.maxh)
    }
}

/// Return whether min/max constraints indicate that a window should float.
/// Shared by native X11, XWayland, and xdg-shell classification.
#[inline]
pub fn constraints_prefer_floating(minw: i32, minh: i32, maxw: i32, maxh: i32) -> bool {
    minw > 0 && minh > 0 && (minw == maxw || minh == maxh)
}

/// Compute the screen rectangle for a given snap position.
///
/// Returns the target `Rect` (border-aware) for a window snapped to one of the
/// nine screen regions (half/quarter/maximized), or `None` if `SnapPosition::None`.
pub fn snap_rect(snap_status: SnapPosition, border_width: i32, work_rect: &Rect) -> Option<Rect> {
    let half_w = work_rect.w / 2;
    let half_h = work_rect.h / 2;
    let horizontal_border = 2 * border_width;

    let rect = match snap_status {
        SnapPosition::Top => Rect::new(
            work_rect.x,
            work_rect.y,
            work_rect.w - horizontal_border,
            half_h - horizontal_border,
        ),
        SnapPosition::Bottom => Rect::new(
            work_rect.x,
            work_rect.y + half_h,
            work_rect.w - horizontal_border,
            half_h - horizontal_border,
        ),
        SnapPosition::Left => Rect::new(
            work_rect.x,
            work_rect.y,
            half_w - horizontal_border,
            work_rect.h - horizontal_border,
        ),
        SnapPosition::Right => Rect::new(
            work_rect.x + half_w,
            work_rect.y,
            half_w - horizontal_border,
            work_rect.h - horizontal_border,
        ),
        SnapPosition::TopLeft => Rect::new(
            work_rect.x,
            work_rect.y,
            half_w - horizontal_border,
            half_h - horizontal_border,
        ),
        SnapPosition::TopRight => Rect::new(
            work_rect.x + half_w,
            work_rect.y,
            half_w - horizontal_border,
            half_h - horizontal_border,
        ),
        SnapPosition::BottomLeft => Rect::new(
            work_rect.x,
            work_rect.y + half_h,
            half_w - horizontal_border,
            half_h - horizontal_border,
        ),
        SnapPosition::BottomRight => Rect::new(
            work_rect.x + half_w,
            work_rect.y + half_h,
            half_w - horizontal_border,
            half_h - horizontal_border,
        ),
        SnapPosition::Maximized => Rect::new(
            work_rect.x,
            work_rect.y,
            work_rect.w - horizontal_border,
            work_rect.h - horizontal_border,
        ),
        SnapPosition::None => return None,
    };

    Some(rect)
}
