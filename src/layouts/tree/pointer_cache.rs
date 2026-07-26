use super::*;

impl PointerPlacementCache {
    pub(crate) fn new(
        tree: LayoutTree,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
        minimums: HashMap<WindowId, Size>,
    ) -> Self {
        let bounds = tree.bounds(layout_rect);
        Self {
            tree,
            source,
            layout_rect,
            edge_fraction: finite_clamp(edge_fraction, 0.05, 0.49, 0.34),
            minimums,
            bounds,
            center_slots: HashMap::new(),
            edge_slots: HashMap::new(),
        }
    }

    pub(crate) fn resolve_at_point(&mut self, point: Point) -> Option<PointerPlacementResolution> {
        let (&target, &target_rect) = self
            .bounds
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
            if let Some(slot) = self.center_slots.get(&target) {
                return *slot;
            }
            let mut candidate = self.tree.clone();
            let resolution = candidate
                .swap_windows(self.source, target)
                .then(|| {
                    let slot = candidate
                        .soft_constrained_bounds(self.layout_rect, &self.minimums)
                        .0
                        .get(&self.source)
                        .copied()?;
                    Some(PointerPlacementResolution {
                        target: PlacementTarget {
                            target,
                            side: None,
                            candidate_index: 0,
                            position: target_rect.center(),
                        },
                        slot,
                    })
                })
                .flatten();
            self.center_slots.insert(target, resolution);
            return resolution;
        };

        let key = (target, side);
        if !self.edge_slots.contains_key(&key) {
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
            let candidates = LayoutTree::normalized_soft_constrained_candidates(
                self.source,
                self.layout_rect,
                &self.minimums,
                candidates,
            );
            self.edge_slots.insert(key, candidates);
        }

        let candidates = &self.edge_slots[&key];
        if candidates.is_empty() {
            return None;
        }
        let index = ((distance.max(0.0) * candidates.len() as f64).floor() as usize)
            .min(candidates.len() - 1);
        candidates.get(index).copied()
    }

    pub(crate) fn preview_at_point(&mut self, point: Point) -> Option<Rect> {
        self.resolve_at_point(point)
            .map(|resolution| resolution.slot)
    }
}
