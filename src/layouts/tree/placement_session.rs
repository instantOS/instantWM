use std::sync::Arc;

use super::*;

/// Minimum depth of an optional scope at the middle of the target edge.
/// Corners taper because they are shared by two perpendicular edges.
const MIN_SCOPE_BAND: f64 = 12.0;
const SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

#[derive(Debug)]
pub(crate) struct PlacementHitboxes {
    pub area: Rect,
    excluded: Option<Rect>,
    pub spans: Vec<PlacementHitboxSpan>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacementHitboxSpan {
    pub rect: Rect,
    pub side: Option<Side>,
    pub band: usize,
    #[cfg(test)]
    pub target: WindowId,
}

#[derive(Clone, Copy)]
struct EdgeBands {
    broader: usize,
    last_plan: usize,
}

impl EdgeBands {
    fn new(depth: f64, plan_count: usize) -> Self {
        let last_plan = plan_count.saturating_sub(1);
        Self {
            broader: last_plan.min((depth / (2.0 * MIN_SCOPE_BAND)) as usize),
            last_plan,
        }
    }

    fn band(self, distance: f64) -> usize {
        ((distance * 2.0 * self.broader as f64) as usize).min(self.broader)
    }

    fn plan_index(self, band: usize) -> usize {
        if band == self.broader {
            self.last_plan
        } else if self.broader == 1 {
            0
        } else {
            // Sample the existing scope order, including both ends of the
            // broader range, when there is no room for all intermediate scopes.
            band * (self.last_plan - 1) / (self.broader - 1)
        }
    }
}

fn edge_depth(length: i32, fraction: f64) -> f64 {
    // Leave at least the middle pixel (both middle pixels for an even length)
    // available for a centre drop, even on tiny slots at a large fraction.
    (f64::from(length) * fraction).min(f64::from((length - 1) / 2))
}

#[derive(Clone, Copy)]
struct HitboxEdges {
    rect: Rect,
    inset_x: f64,
    inset_y: f64,
}

impl HitboxEdges {
    fn new(rect: Rect, fraction: f64) -> Self {
        Self {
            rect,
            inset_x: edge_depth(rect.w, fraction),
            inset_y: edge_depth(rect.h, fraction),
        }
    }

    fn depth(self, side: Side) -> f64 {
        match side.axis() {
            Axis::Vertical => self.inset_x,
            Axis::Horizontal => self.inset_y,
        }
    }

    fn side_at(self, point: Point) -> Option<(usize, f64)> {
        let x = f64::from(point.x - self.rect.x) + 0.5;
        let y = f64::from(point.y - self.rect.y) + 0.5;
        // All distances are measured to the pixel centre. A zero inset
        // yields infinity and therefore cannot claim a pixel.
        [
            x / self.inset_x,
            (f64::from(self.rect.w) - x) / self.inset_x,
            y / self.inset_y,
            (f64::from(self.rect.h) - y) / self.inset_y,
        ]
        .into_iter()
        .enumerate()
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= 1.0)
    }
}

type PlanKey = (WindowId, Option<Side>);

impl TreePlacementSession {
    pub(crate) fn new(
        tree: LayoutTree,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
        minimums: HashMap<WindowId, Size>,
    ) -> Self {
        let source_in_tree = tree.root.as_ref().is_some_and(|root| root.contains(source));
        Self {
            tree,
            source,
            source_in_tree,
            layout_rect,
            hover_rect: layout_rect,
            edge_fraction: placement::sane_edge_fraction(edge_fraction),
            minimums,
            bounds: None,
            plans: HashMap::new(),
            hitboxes: None,
        }
    }

    pub(crate) fn source(&self) -> WindowId {
        self.source
    }

    /// An incoming window can change the destination's gaps and borders.
    /// Hit-test the currently displayed layout and solve the drop separately.
    pub(crate) fn with_hover_layout(
        mut self,
        rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Self {
        self.hover_rect = rect;
        self.bounds = Some(Self::ordered_bounds(
            &self.tree,
            self.source,
            rect,
            minimums,
        ));
        self
    }

    fn bounds(&mut self) -> &[(WindowId, Rect)] {
        self.bounds.get_or_insert_with(|| {
            Self::ordered_bounds(&self.tree, self.source, self.hover_rect, &self.minimums)
        })
    }

    fn ordered_bounds(
        tree: &LayoutTree,
        source: WindowId,
        rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Vec<(WindowId, Rect)> {
        let (bounds, _) = tree.soft_constrained_bounds(rect, minimums);
        // Rounded slots can share a pixel in an extremely small area. Use
        // tree order so the winner is stable across rebuilds.
        tree.leaves()
            .into_iter()
            .filter(|window| *window != source)
            .filter_map(|window| Some((window, bounds.get(&window)?.intersection(&rect)?)))
            .collect()
    }

    fn plans(&mut self, key: PlanKey, target_rect: Rect) -> &[PlacementPlan] {
        let (target, side) = key;
        self.plans.entry(key).or_insert_with(|| {
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
            // An ancestor can collapse to the same topology as a local
            // scope after removing the source. Keep the local representative
            // so the innermost hitbox continues to use the local geometry.
            let mut plans = placement::normalized_plans(
                self.source,
                self.layout_rect,
                &self.minimums,
                candidates.into_iter().rev(),
            );
            plans.reverse();
            plans
        })
    }

    /// Incoming windows have no slot to exchange. Their centre target uses
    /// the most local right-edge plan.
    fn center_side(&self) -> Option<Side> {
        (!self.source_in_tree).then_some(Side::Right)
    }

    fn resolve(&mut self, point: Point) -> Option<(PlanKey, usize)> {
        let &(target, rect) = self
            .bounds()
            .iter()
            .find(|(_, rect)| rect.contains_point(point))?;
        let edges = HitboxEdges::new(rect, self.edge_fraction);
        let Some((side_index, distance)) = edges.side_at(point) else {
            let key = (target, self.center_side());
            return Some((key, self.plans(key, rect).len().checked_sub(1)?));
        };
        let side = SIDES[side_index];
        let key = (target, Some(side));
        let plan_count = self.plans(key, rect).len();
        if plan_count == 0 {
            return None;
        }
        let bands = EdgeBands::new(edges.depth(side), plan_count);
        Some((key, bands.plan_index(bands.band(distance))))
    }

    /// Rasterize the same hit test as compact horizontal spans in logical
    /// pixels. This walks regions per row, without testing each pixel or
    /// allocating a render-sized buffer in the layout engine.
    pub(crate) fn hitboxes(
        &mut self,
        area: Rect,
        excluded: Option<Rect>,
    ) -> Arc<PlacementHitboxes> {
        if let Some(hitboxes) = &self.hitboxes
            && hitboxes.area == area
            && hitboxes.excluded == excluded
        {
            return Arc::clone(hitboxes);
        }
        let mut spans = Vec::new();
        let layout = self.hover_rect;
        let mut push = |mut span: PlacementHitboxSpan| {
            // Motion outside the gapped work area is clamped to its nearest
            // pixel. Extend those edge pixels over the same outer margins.
            let right = if span.rect.right() == layout.right() {
                area.right()
            } else {
                span.rect.right()
            };
            let bottom = if span.rect.bottom() == layout.bottom() {
                area.bottom()
            } else {
                span.rect.bottom()
            };
            if span.rect.x == layout.x {
                span.rect.x = area.x;
            }
            if span.rect.y == layout.y {
                span.rect.y = area.y;
            }
            span.rect.w = right - span.rect.x;
            span.rect.h = bottom - span.rect.y;
            if let Some(rect) = span.rect.intersection(&area) {
                span.rect = rect;
                if let Some(excluded) = excluded.filter(|excluded| rect.intersects_other(excluded))
                {
                    spans.extend(
                        rect.subtract(&excluded)
                            .into_iter()
                            .map(|rect| PlacementHitboxSpan { rect, ..span }),
                    );
                } else {
                    spans.push(span);
                }
            }
        };
        if self.tree.is_empty() {
            push(PlacementHitboxSpan {
                rect: self.hover_rect,
                side: None,
                band: 0,
                #[cfg(test)]
                target: self.source,
            });
        }
        let bounds = self.bounds().to_vec();
        for (index, &(target, rect)) in bounds.iter().enumerate() {
            let edges = HitboxEdges::new(rect, self.edge_fraction);
            let bands = SIDES.map(|side| {
                let count = self.plans((target, Some(side)), rect).len();
                (count > 0).then(|| EdgeBands::new(edges.depth(side), count))
            });
            let center = self.center_side();
            let has_center = !self.plans((target, center), rect).is_empty();
            let zone_at = |point| match edges.side_at(point) {
                Some((index, distance)) => (
                    Some(SIDES[index]),
                    bands[index].map(|bands| bands.band(distance)),
                ),
                None => (None, has_center.then_some(0)),
            };
            let mut visible = vec![rect];
            for (_, previous) in &bounds[..index] {
                if rect.intersects_other(previous) {
                    visible = visible
                        .into_iter()
                        .flat_map(|rect| rect.subtract(previous))
                        .collect();
                }
            }
            for visible in visible {
                for y in visible.y..visible.bottom() {
                    let mut x = visible.x;
                    while x < visible.right() {
                        let zone = zone_at(Point::new(x, y));
                        // A zone is contiguous along a row: left bands, the
                        // centre or one horizontal band, then right bands.
                        let (mut lo, mut hi) = (x + 1, visible.right());
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if zone_at(Point::new(mid, y)) == zone {
                                lo = mid + 1;
                            } else {
                                hi = mid;
                            }
                        }
                        if let (side, Some(band)) = zone {
                            push(PlacementHitboxSpan {
                                rect: Rect::new(x, y, lo - x, 1),
                                side,
                                band,
                                #[cfg(test)]
                                target,
                            });
                        }
                        x = lo;
                    }
                }
            }
        }
        let hitboxes = Arc::new(PlacementHitboxes {
            area,
            excluded,
            spans,
        });
        self.hitboxes = Some(Arc::clone(&hitboxes));
        hitboxes
    }

    fn empty_plan(&self, point: Point) -> Option<PlacementPlan> {
        if !self.tree.is_empty() {
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
                .clamp(self.hover_rect.x, self.hover_rect.right() - 1),
            point
                .y
                .clamp(self.hover_rect.y, self.hover_rect.bottom() - 1),
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

#[cfg(test)]
#[path = "placement_hitbox_tests.rs"]
mod tests;
