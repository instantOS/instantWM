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
            edge_fraction: finite_clamp(edge_fraction, 0.05, 0.49, 0.34),
            minimums,
            bounds: None,
            center_resolutions: HashMap::new(),
            edge_resolutions: HashMap::new(),
        }
    }

    pub(crate) fn targets(&self) -> Vec<PlacementTarget> {
        LayoutTree::normalized_soft_constrained_candidates(
            self.source,
            self.layout_rect,
            &self.minimums,
            self.tree.raw_resolved_placement_targets(
                self.source,
                self.layout_rect,
                self.edge_fraction,
            ),
        )
        .into_iter()
        .map(|plan| plan.target())
        .collect()
    }

    pub(crate) fn plan_target(&self, target: PlacementTarget) -> Option<PlacementPlan> {
        let candidate = if let Some(side) = target.side {
            let candidates = self
                .tree
                .resolved_edge_candidates(self.source, target.target, side);
            if candidates.is_empty() {
                let mut candidate = self.tree.clone();
                if !candidate.move_beside(self.source, target.target, side) {
                    return None;
                }
                candidate
            } else {
                candidates.into_iter().nth(target.candidate_index)?.1
            }
        } else {
            let mut candidate = self.tree.clone();
            if !candidate.swap_windows(self.source, target.target) {
                return None;
            }
            candidate
        };
        let source_slot = candidate
            .soft_constrained_bounds(self.layout_rect, &self.minimums)
            .0
            .get(&self.source)
            .copied()?;
        Some(PlacementPlan {
            target,
            candidate,
            source_slot,
        })
    }

    pub(crate) fn plan_point(&mut self, point: Point) -> Option<PlacementPlan> {
        let resolution = self.resolve_point(point)?;
        let plan = self.plan_target(resolution.target)?;
        if plan.source_slot() != resolution.source_slot {
            return None;
        }
        Some(plan)
    }

    fn resolve_point(&mut self, point: Point) -> Option<PlacementResolution> {
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
        .min_by(|left, right| left.1.total_cmp(&right.1));

        let Some((side, distance)) = nearest.filter(|(_, distance)| *distance <= 1.0) else {
            if let Some(resolution) = self.center_resolutions.get(&target) {
                return *resolution;
            }
            let mut candidate = self.tree.clone();
            let resolution = candidate.swap_windows(self.source, target).then(|| {
                let source_slot = candidate
                    .soft_constrained_bounds(self.layout_rect, &self.minimums)
                    .0
                    .get(&self.source)
                    .copied()?;
                Some(PlacementResolution {
                    target: PlacementTarget {
                        target,
                        side: None,
                        candidate_index: 0,
                        position: target_rect.center(),
                    },
                    source_slot,
                })
            })?;
            self.center_resolutions.insert(target, resolution);
            return resolution;
        };

        let key = (target, side);
        if !self.edge_resolutions.contains_key(&key) {
            let candidates = self
                .tree
                .resolved_edge_candidates(self.source, target, side)
                .into_iter()
                .enumerate()
                .map(
                    |(candidate_index, (_, candidate))| ResolvedPlacementTarget {
                        target: PlacementTarget {
                            target,
                            side: Some(side),
                            candidate_index,
                            position: target_rect.center(),
                        },
                        candidate,
                    },
                )
                .collect::<Vec<_>>();
            let candidates = if candidates.is_empty() {
                let mut candidate = self.tree.clone();
                if candidate.move_beside(self.source, target, side) {
                    vec![ResolvedPlacementTarget {
                        target: PlacementTarget {
                            target,
                            side: Some(side),
                            candidate_index: 0,
                            position: target_rect.center(),
                        },
                        candidate,
                    }]
                } else {
                    Vec::new()
                }
            } else {
                candidates
            };
            let resolutions = LayoutTree::normalized_soft_constrained_candidates(
                self.source,
                self.layout_rect,
                &self.minimums,
                candidates,
            )
            .into_iter()
            .map(|plan| PlacementResolution {
                target: plan.target(),
                source_slot: plan.source_slot(),
            })
            .collect();
            self.edge_resolutions.insert(key, resolutions);
        }

        let resolutions = &self.edge_resolutions[&key];
        if resolutions.is_empty() {
            return None;
        }
        let index = ((distance.max(0.0) * resolutions.len() as f64).floor() as usize)
            .min(resolutions.len() - 1);
        resolutions.get(index).copied()
    }

    pub(crate) fn preview_point(&mut self, point: Point) -> Option<Rect> {
        self.resolve_point(point)
            .map(|resolution| resolution.source_slot)
    }
}
