use super::*;

impl LayoutTree {
    /// Swap the focused leaf with the topology-first visual neighbour.
    pub fn swap_with_neighbor(&mut self, source: WindowId, side: Side) -> Option<WindowId> {
        let neighbor = self.visual_neighbor(source, side)?;
        self.swap_windows(source, neighbor).then_some(neighbor)
    }

    pub fn visual_neighbor(&self, source: WindowId, side: Side) -> Option<WindowId> {
        let root = self.root.as_ref()?;
        let rects = self.float_bounds();
        let source_rect = *rects.get(&source)?;
        visual_neighbor_in(root, source, source_rect, side, &rects).0
    }

    /// Resize the nearest applicable axis run. Peer ratios are preserved.
    pub fn resize(&mut self, source: WindowId, side: Side) -> bool {
        self.resize_with_config(source, side, CommandConfig::default())
    }

    pub fn resize_with_config(
        &mut self,
        source: WindowId,
        side: Side,
        config: CommandConfig,
    ) -> bool {
        let axis = side.axis();
        let grow = matches!(side, Side::Top | Side::Right);
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, changed) = resize_deepest_run(root, source, axis, grow, config);
        self.root = Some(root);
        if changed {
            self.invalidate_force_provenance();
        }
        changed
    }

    /// Resize the deepest applicable run by a pointer displacement.
    ///
    /// `pixels` is the movement of the grabbed physical edge: positive means
    /// right/down and negative means left/up. It is normalized against the
    /// actual containing run rather than the whole monitor, so nested splits
    /// track the pointer one-for-one.
    pub fn resize_by_pixels(
        &mut self,
        source: WindowId,
        side: Side,
        pixels: i32,
        layout_rect: Rect,
        minimum_weight: f64,
    ) -> bool {
        if pixels == 0 {
            return false;
        }
        let axis = side.axis();
        let Some(normalized_span) = self.resize_span(source, axis) else {
            return false;
        };
        let layout_span = match axis {
            Axis::Vertical => layout_rect.w,
            Axis::Horizontal => layout_rect.h,
        };
        let physical_span = normalized_span * f64::from(layout_span.max(1));
        if physical_span <= EPSILON {
            return false;
        }
        let edge_delta = f64::from(pixels) / physical_span;
        let weight_delta = if side.is_leading() {
            -edge_delta
        } else {
            edge_delta
        };
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, changed) =
            resize_deepest_run_by(root, source, axis, weight_delta, minimum_weight);
        self.root = Some(root);
        if changed {
            self.invalidate_force_provenance();
        }
        changed
    }

    /// Move only the grabbed seam, keeping the source's opposite edge fixed.
    ///
    /// Unlike keyboard growth, pointer resizing transfers space solely from
    /// the source branch's peers on `side`. Their existing ratios are
    /// preserved, while peers beyond the source's opposite edge stay fixed.
    pub fn resize_edge_by_pixels(
        &mut self,
        source: WindowId,
        side: Side,
        pixels: i32,
        layout_rect: Rect,
        minimum_weight: f64,
    ) -> bool {
        if pixels == 0 {
            return false;
        }
        let axis = side.axis();
        let Some(normalized_span) = self.resize_edge_span(source, side) else {
            return false;
        };
        let layout_span = match axis {
            Axis::Vertical => layout_rect.w,
            Axis::Horizontal => layout_rect.h,
        };
        let physical_span = normalized_span * f64::from(layout_span.max(1));
        if physical_span <= EPSILON {
            return false;
        }
        let edge_delta = f64::from(pixels) / physical_span;
        let source_delta = if side.is_leading() {
            -edge_delta
        } else {
            edge_delta
        };
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, changed) =
            resize_deepest_edge_by(root, source, side, source_delta, minimum_weight);
        self.root = Some(root);
        if changed {
            self.invalidate_force_provenance();
        }
        changed
    }

    /// Whether `source` belongs to a split that can be resized on `axis`.
    pub fn can_resize_axis(&self, source: WindowId, axis: Axis) -> bool {
        self.resize_span(source, axis).is_some()
    }

    /// Whether the physical edge is backed by an adjustable tree seam.
    pub fn can_resize_side(&self, source: WindowId, side: Side) -> bool {
        self.resize_edge_span(source, side).is_some()
    }

    fn resize_span(&self, source: WindowId, axis: Axis) -> Option<f64> {
        let root = self.root.as_ref()?;
        let split = deepest_resize_split(root, source, axis)?;
        let bounds = self.all_float_bounds();
        let rect = bounds.get(&NodeKey::Split(split))?;
        Some(rect.axis_size(axis))
    }

    fn resize_edge_span(&self, source: WindowId, side: Side) -> Option<f64> {
        let root = self.root.as_ref()?;
        let split = deepest_resize_edge_split(root, source, side)?;
        let bounds = self.all_float_bounds();
        let rect = bounds.get(&NodeKey::Split(split))?;
        Some(rect.axis_size(side.axis()))
    }

    pub fn resize_smart(&mut self, source: WindowId, grow: bool) -> bool {
        self.resize_smart_with_config(source, grow, CommandConfig::default())
    }

    pub fn resize_smart_with_config(
        &mut self,
        source: WindowId,
        grow: bool,
        config: CommandConfig,
    ) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        let Some(axis) = immediate_parent_axis(root, source) else {
            return false;
        };
        let side = match (axis, grow) {
            (Axis::Vertical, true) => Side::Right,
            (Axis::Vertical, false) => Side::Left,
            (Axis::Horizontal, true) => Side::Top,
            (Axis::Horizontal, false) => Side::Bottom,
        };
        self.resize_with_config(source, side, config)
    }
}
