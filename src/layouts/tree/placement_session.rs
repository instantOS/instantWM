use super::*;

impl TreePlacementSession {
    pub(crate) fn new(
        tree: LayoutTree,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
        minimums: HashMap<WindowId, Size>,
    ) -> Self {
        Self {
            tree,
            source,
            layout_rect,
            edge_fraction: placement::sane_edge_fraction(edge_fraction),
            minimums,
            bounds: None,
            plans: HashMap::new(),
        }
    }

    pub(crate) fn source(&self) -> WindowId {
        self.source
    }

    /// Hit-test `point` against the snapshot. Hovering a target's centre swaps
    /// with it; its edge bands select among the distinct viable edge plans.
    fn resolve(&mut self, point: Point) -> Option<((WindowId, Option<Side>), usize)> {
        let bounds = self
            .bounds
            .get_or_insert_with(|| self.tree.bounds(self.layout_rect));
        let (&target, &target_rect) = bounds
            .iter()
            .find(|(window, rect)| **window != self.source && rect.contains_point(point))?;
        let inset_x = (f64::from(target_rect.w) * self.edge_fraction).max(1.0);
        let inset_y = (f64::from(target_rect.h) * self.edge_fraction).max(1.0);
        let nearest = [
            (Side::Left, f64::from(point.x - target_rect.x) / inset_x),
            (
                Side::Right,
                f64::from(target_rect.right() - point.x) / inset_x,
            ),
            (Side::Top, f64::from(point.y - target_rect.y) / inset_y),
            (
                Side::Bottom,
                f64::from(target_rect.bottom() - point.y) / inset_y,
            ),
        ]
        .into_iter()
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= 1.0);
        let (side, distance) = match nearest {
            Some((side, distance)) => (Some(side), distance),
            None => (None, 0.0),
        };

        // Incoming windows insert at edges; centre drops default to the right.
        // Only a source already in this tree can exchange slots with a target.
        let side = if self.tree.leaves().contains(&self.source) {
            side
        } else {
            Some(side.unwrap_or(Side::Right))
        };
        let key = (target, side);
        let plans = self.plans.entry(key).or_insert_with(|| {
            let placement_target = |candidate_index| PlacementTarget {
                target,
                side,
                candidate_index,
                position: target_rect.center(),
            };
            let candidates = match side {
                None => self
                    .tree
                    .swapped(self.source, target)
                    .map(|candidate| (placement_target(0), candidate))
                    .into_iter()
                    .collect::<Vec<_>>(),
                Some(side) => self
                    .tree
                    .edge_candidates(self.source, target, side)
                    .into_iter()
                    .enumerate()
                    .map(|(index, candidate)| (placement_target(index), candidate))
                    .collect(),
            };
            placement::normalized_plans(self.source, self.layout_rect, &self.minimums, candidates)
        });
        let last = plans.len().checked_sub(1)?;
        let index = ((distance.max(0.0) * plans.len() as f64).floor() as usize).min(last);
        Some((key, index))
    }

    fn empty_plan(&self, point: Point) -> Option<PlacementPlan> {
        if !self.tree.leaves().is_empty() || !self.layout_rect.contains_point(point) {
            return None;
        }
        let mut tree = self.tree.clone();
        tree.apply_preset(Preset::MasterStack, &[self.source], 1);
        PlacementPlan::new(
            PlacementTarget {
                target: self.source,
                side: None,
                candidate_index: 0,
                position: point,
            },
            tree,
            self.source,
            self.layout_rect,
            &self.minimums,
        )
    }

    fn layout_point(&self, point: Point) -> Point {
        Point::new(
            point
                .x
                .clamp(self.layout_rect.x, self.layout_rect.right() - 1),
            point
                .y
                .clamp(self.layout_rect.y, self.layout_rect.bottom() - 1),
        )
    }

    pub(crate) fn preview_point(&mut self, point: Point) -> Option<Rect> {
        let point = self.layout_point(point);
        if let Some(plan) = self.empty_plan(point) {
            return Some(plan.source_slot());
        }
        let (key, index) = self.resolve(point)?;
        Some(self.plans[&key][index].source_slot)
    }

    pub(crate) fn into_plan(mut self, point: Point) -> Option<PlacementPlan> {
        let point = self.layout_point(point);
        if let Some(plan) = self.empty_plan(point) {
            return Some(plan);
        }
        let (key, index) = self.resolve(point)?;
        Some(self.plans.remove(&key)?.swap_remove(index))
    }
}
