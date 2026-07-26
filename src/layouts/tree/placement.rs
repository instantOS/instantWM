use super::*;

/// Canonical tree structure with split identities and weights erased.
///
/// Candidates with the same topology differ only in how much space their
/// equivalent splits inherited. They should be one placement choice, not
/// several adjacent pointer bands.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum PlacementTopology {
    Window(WindowId),
    Split(Axis, Vec<PlacementTopology>),
}

pub(super) fn placement_topology(node: &Node) -> PlacementTopology {
    match node {
        Node::Window(window) => PlacementTopology::Window(*window),
        Node::Split(split) => PlacementTopology::Split(
            split.axis,
            split
                .children
                .iter()
                .map(|child| placement_topology(&child.node))
                .collect(),
        ),
    }
}

/// Collapse candidates whose canonical tree structure is identical when split
/// weights and allocation-only split IDs are ignored. Candidate order is
/// deterministic, so retaining the first representative does not let current
/// geometry influence structural equivalence.
pub(super) fn topology_representatives<T>(
    candidates: Vec<(ResolvedPlacementTarget, T)>,
) -> Vec<(ResolvedPlacementTarget, T)> {
    let mut topologies = HashSet::new();
    let mut distinct = Vec::new();
    for candidate in candidates {
        let Some(root) = candidate.0.candidate.root.as_ref() else {
            continue;
        };
        if topologies.insert(placement_topology(root)) {
            distinct.push(candidate);
        }
    }
    distinct
}

impl LayoutTree {
    /// Move `source` beside `target`. The requested side selects the split axis;
    /// canonicalisation automatically inserts into an existing matching run.
    pub fn move_beside(&mut self, source: WindowId, target: WindowId, side: Side) -> bool {
        if source == target {
            return false;
        }
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(source) || !root.contains(target) {
            self.root = Some(root);
            return false;
        }
        let id = self.allocate();
        let without_source = root
            .remove(source)
            .expect("moving one of at least two leaves leaves a root");
        let (first, second) = if side.is_leading() {
            (source, target)
        } else {
            (target, source)
        };
        let replacement = make_split(
            id,
            side.axis(),
            vec![
                WeightedNode {
                    node: Node::Window(first),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(second),
                    weight: 1.0,
                },
            ],
        )
        .expect("two leaves create a split");
        self.root = Some(without_source.replace_window(target, replacement));
        self.invalidate_force_provenance();
        true
    }

    fn edge_candidates(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
    ) -> Vec<EdgeCandidate> {
        self.resolved_edge_candidates(source, target, side)
            .into_iter()
            .map(|(candidate, _)| candidate)
            .collect()
    }

    pub(super) fn resolved_edge_candidates(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
    ) -> Vec<(EdgeCandidate, LayoutTree)> {
        let rects = self.all_float_bounds();
        let leaf_rects = self.float_bounds();
        self.resolved_edge_candidates_with_geometry(source, target, side, &rects, &leaf_rects)
    }

    fn resolved_edge_candidates_with_geometry(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
        rects: &HashMap<NodeKey, FRect>,
        leaf_rects: &HashMap<WindowId, FRect>,
    ) -> Vec<(EdgeCandidate, LayoutTree)> {
        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };
        let Some(target_rect) = leaf_rects.get(&target).copied() else {
            return Vec::new();
        };
        let mut path = Vec::new();
        if !path_to(root, target, &mut path) {
            return Vec::new();
        }

        let axis = side.axis();
        let mut exposed = vec![NodeKey::Window(target)];
        for (split, branch_index) in path.iter().rev() {
            if split.axis == axis {
                let edge_index = if side.is_leading() {
                    0
                } else {
                    split.children.len() - 1
                };
                if *branch_index != edge_index {
                    break;
                }
            }
            exposed.push(NodeKey::Split(split.id));
        }

        let seam = target_rect.axis_start(axis)
            + if side.is_leading() {
                0.0
            } else {
                target_rect.axis_size(axis)
            };
        let mut candidates = exposed
            .into_iter()
            .filter_map(|key| {
                rects.get(&key)?;
                let scope_depth = match key {
                    NodeKey::Window(_) => 0,
                    NodeKey::Split(id) => path
                        .iter()
                        .position(|(split, _)| split.id == id)
                        .map_or(0, |index| path.len() - index),
                };
                Some(EdgeCandidate {
                    scope: PlacementScope::Node(key),
                    scope_depth,
                })
            })
            .collect::<Vec<_>>();

        // Recover aligned pseudo-seams, including rectangular contiguous child
        // ranges hidden by canonical same-axis flattening.
        for (split, branch_index) in &path {
            let scope_key = NodeKey::Split(split.id);
            let rect = rects[&scope_key];
            let tolerance = rect.axis_size(axis) * 0.04;
            let target_cross_size = cross_size(target_rect, axis);
            let parent_cross_size = cross_size(rect, axis);
            if parent_cross_size > target_cross_size + tolerance
                && seam > rect.axis_start(axis) + tolerance
                && seam < rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                && let Some(before) =
                    seam_partition(&split.children, seam, axis, leaf_rects, tolerance)
            {
                candidates.push(EdgeCandidate {
                    scope: PlacementScope::AlignedNode {
                        key: scope_key,
                        seam,
                        before,
                    },
                    scope_depth: path
                        .iter()
                        .position(|(candidate, _)| candidate.id == split.id)
                        .map_or(0, |index| path.len() - index),
                });
            }
            // Every contiguous child range is contained by its parent. If the
            // parent is no wider than the target on the cross axis, no range
            // can expose an aligned seam either. Avoid the O(k²) range scan
            // (and its repeated O(k) geometry collection) for common flat
            // k-window runs.
            if parent_cross_size <= target_cross_size {
                continue;
            }
            for first in 0..=*branch_index {
                for last in *branch_index..split.children.len() {
                    if first == 0 && last + 1 == split.children.len() {
                        continue;
                    }
                    let children = &split.children[first..=last];
                    let selected_rects = children
                        .iter()
                        .filter_map(|child| rects.get(&child.node.key()).copied())
                        .collect::<Vec<_>>();
                    let Some(rect) = bounding_rect(&selected_rects) else {
                        continue;
                    };
                    let tolerance = rect.axis_size(axis) * 0.04;
                    if split.axis != axis && children.len() > 1 {
                        let range_edge = rect.axis_start(axis)
                            + if side.is_leading() {
                                0.0
                            } else {
                                rect.axis_size(axis)
                            };
                        if (range_edge - seam).abs() <= tolerance {
                            candidates.push(EdgeCandidate {
                                scope: PlacementScope::ChildRange {
                                    parent: split.id,
                                    children: children
                                        .iter()
                                        .map(|child| child.node.key())
                                        .collect(),
                                },
                                scope_depth: path
                                    .iter()
                                    .position(|(candidate, _)| candidate.id == split.id)
                                    .map_or(0, |index| path.len() - index),
                            });
                        }
                    }
                    if cross_size(rect, axis) <= target_cross_size + tolerance
                        || seam <= rect.axis_start(axis) + tolerance
                        || seam >= rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                    {
                        continue;
                    }
                    let Some(before) = seam_partition(children, seam, axis, leaf_rects, tolerance)
                    else {
                        continue;
                    };
                    candidates.push(EdgeCandidate {
                        scope: PlacementScope::AlignedChildRange {
                            parent: split.id,
                            children: children.iter().map(|child| child.node.key()).collect(),
                            seam,
                            before,
                        },
                        scope_depth: path
                            .iter()
                            .position(|(candidate, _)| candidate.id == split.id)
                            .map_or(0, |index| path.len() - index),
                    });
                }
            }
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.scope_depth));
        // Do not deduplicate by geometry here: different structures can
        // currently produce identical rectangles. Constraint filtering and
        // weight-independent topology normalization happen in the shared
        // keyboard/pointer pipeline after all viable scopes are materialized.
        candidates
            .into_iter()
            .filter_map(|candidate| {
                let mut preview = self.clone();
                preview
                    .move_to_scope(source, target, side, candidate.scope.clone())
                    .then_some((candidate, preview))
            })
            .collect()
    }

    /// Enumerate structurally distinct placement results. Raw hit regions can
    /// describe one canonical topology in several ways (for example, either
    /// side of a shared seam); only the first deterministic representative is
    /// exposed.
    pub fn placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<PlacementTarget> {
        self.normalized_placement_candidates(source, layout_rect, edge_fraction)
            .into_iter()
            .map(|(target, _)| target)
            .collect()
    }

    pub(crate) fn constrained_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
        minimums: &HashMap<WindowId, Size>,
    ) -> Vec<PlacementTarget> {
        Self::normalized_constrained_candidates(
            source,
            layout_rect,
            minimums,
            self.raw_resolved_placement_targets(source, layout_rect, edge_fraction),
        )
        .into_iter()
        .map(|resolution| resolution.target)
        .collect()
    }

    pub(super) fn normalized_constrained_candidates(
        source: WindowId,
        layout_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
        candidates: impl IntoIterator<Item = ResolvedPlacementTarget>,
    ) -> Vec<PointerPlacementResolution> {
        // Viability must be established first. Otherwise an infeasible early
        // representative can suppress an equivalent candidate that actually
        // satisfies minimum sizes.
        let viable = candidates
            .into_iter()
            .filter_map(|resolved| {
                let Some(slot) = resolved
                    .candidate
                    .constrained_bounds(layout_rect, minimums)
                    .and_then(|bounds| bounds.get(&source).copied())
                else {
                    return None;
                };
                Some((resolved, slot))
            })
            .collect();
        topology_representatives(viable)
            .into_iter()
            .map(|(resolved, slot)| PointerPlacementResolution {
                target: resolved.target,
                slot,
            })
            .collect()
    }

    fn normalized_placement_candidates(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<(PlacementTarget, LayoutTree)> {
        topology_representatives(
            self.raw_resolved_placement_targets(source, layout_rect, edge_fraction)
                .into_iter()
                .map(|resolved| (resolved, ()))
                .collect(),
        )
        .into_iter()
        .map(|(resolved, ())| (resolved.target, resolved.candidate))
        .collect()
    }

    pub(super) fn raw_resolved_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<ResolvedPlacementTarget> {
        let bounds = self.bounds(layout_rect);
        let node_bounds = self.all_float_bounds();
        let leaf_bounds = self.float_bounds();
        let fraction = finite_clamp(edge_fraction, 0.05, 0.49, 0.34);
        let mut output = Vec::new();
        for target in self.leaves().into_iter().filter(|window| *window != source) {
            let Some(rect) = bounds.get(&target).copied() else {
                continue;
            };
            output.push(ResolvedPlacementTarget {
                target: PlacementTarget {
                    target,
                    side: None,
                    candidate_index: 0,
                    position: rect.center(),
                },
                candidate: {
                    let mut candidate = self.clone();
                    let _ = candidate.swap_windows(source, target);
                    candidate
                },
            });
            for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
                let candidates = self.resolved_edge_candidates_with_geometry(
                    source,
                    target,
                    side,
                    &node_bounds,
                    &leaf_bounds,
                );
                let candidate_count = candidates.len();
                for (index, (_edge, candidate)) in candidates.into_iter().enumerate() {
                    let band_fraction = fraction * (index as f64 + 0.5) / candidate_count as f64;
                    let position = match side {
                        Side::Left => Point::new(
                            rect.x + (f64::from(rect.w) * band_fraction).round() as i32,
                            rect.center().y,
                        ),
                        Side::Right => Point::new(
                            rect.right() - (f64::from(rect.w) * band_fraction).round() as i32,
                            rect.center().y,
                        ),
                        Side::Top => Point::new(
                            rect.center().x,
                            rect.y + (f64::from(rect.h) * band_fraction).round() as i32,
                        ),
                        Side::Bottom => Point::new(
                            rect.center().x,
                            rect.bottom() - (f64::from(rect.h) * band_fraction).round() as i32,
                        ),
                    };
                    output.push(ResolvedPlacementTarget {
                        target: PlacementTarget {
                            target,
                            side: Some(side),
                            candidate_index: index,
                            position,
                        },
                        candidate,
                    });
                }
            }
        }
        output
    }

    pub fn apply_placement_target(&mut self, source: WindowId, target: PlacementTarget) -> bool {
        let Some(side) = target.side else {
            return self.swap_windows(source, target.target);
        };
        let candidates = self.edge_candidates(source, target.target, side);
        if candidates.is_empty() {
            return self.move_beside(source, target.target, side);
        }
        let Some(candidate) = candidates.get(target.candidate_index) else {
            return false;
        };
        self.move_to_scope(source, target.target, side, candidate.scope.clone())
    }

    /// Return the source slot produced by a semantic target without mutating
    /// the authoritative tree.
    pub fn preview_placement_target(
        &self,
        source: WindowId,
        target: PlacementTarget,
        layout_rect: Rect,
    ) -> Option<Rect> {
        let mut preview = self.clone();
        preview
            .apply_placement_target(source, target)
            .then(|| preview.bounds(layout_rect).get(&source).copied())
            .flatten()
    }

    /// Return the source slot produced by dropping at `point`, without
    /// mutating the authoritative tree. This deliberately calls
    /// [`Self::place_at_point`] on a clone so pointer previews and releases
    /// cannot drift into subtly different target-resolution rules.
    pub fn preview_placement_at_point(
        &self,
        source: WindowId,
        point: Point,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Option<Rect> {
        let mut preview = self.clone();
        preview
            .place_at_point(source, point, layout_rect, edge_fraction)
            .then(|| preview.bounds(layout_rect).get(&source).copied())
            .flatten()
    }

    fn move_to_scope(
        &mut self,
        source: WindowId,
        target: WindowId,
        side: Side,
        scope: PlacementScope,
    ) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(source) || !root.contains(target) || root.leaf_count() < 2 {
            self.root = Some(root);
            return false;
        }
        let Some(without_source) = root.remove(source) else {
            self.root = Some(Node::Window(source));
            return false;
        };
        let next = &mut self.next_split_id;
        let mut allocate = || {
            let id = SplitId(*next);
            *next = next
                .checked_add(1)
                .expect("manual-layout split id space exhausted");
            id
        };
        let rebuilt = match scope {
            PlacementScope::Node(mut key) => {
                if !without_source.contains_key(key) {
                    key = NodeKey::Window(target);
                }
                insert_at_scope_edge(
                    without_source.clone(),
                    key,
                    target,
                    source,
                    side,
                    allocate(),
                )
            }
            PlacementScope::ChildRange { parent, children } => insert_at_child_range_edge(
                without_source.clone(),
                parent,
                &children,
                source,
                side,
                &mut allocate,
            ),
            PlacementScope::AlignedNode { key, seam, before } => {
                let insertion = AlignedInsertion {
                    seam,
                    before: &before,
                    source,
                    axis: side.axis(),
                };
                insert_across_aligned_node(without_source.clone(), key, &insertion, &mut allocate)
            }
            PlacementScope::AlignedChildRange {
                parent,
                children,
                seam,
                before,
            } => {
                let insertion = AlignedInsertion {
                    seam,
                    before: &before,
                    source,
                    axis: side.axis(),
                };
                insert_across_aligned_range(
                    without_source.clone(),
                    parent,
                    &children,
                    &insertion,
                    &mut allocate,
                )
            }
        };
        let Some(rebuilt) = rebuilt else {
            self.root = Some(without_source);
            return false;
        };
        self.root = Some(rebuilt);
        self.invalidate_force_provenance();
        true
    }

    /// Resolve a pointer drop into a semantic local edge placement. The centre
    /// swaps slots; edge bands choose the corresponding side. This backend-free
    /// operation is shared by the X11 and Wayland drag completion paths.
    pub fn place_at_point(
        &mut self,
        source: WindowId,
        point: Point,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> bool {
        let mut resolver = PointerPlacementCache::new(
            self.clone(),
            source,
            layout_rect,
            edge_fraction,
            HashMap::new(),
        );
        let Some(resolution) = resolver.resolve_at_point(point) else {
            return false;
        };
        self.apply_placement_target(source, resolution.target)
    }

    pub fn swap_windows(&mut self, first: WindowId, second: WindowId) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if first == second || !root.contains(first) || !root.contains(second) {
            self.root = Some(root);
            return false;
        }
        self.root = Some(swap_windows(root, first, second));
        self.invalidate_force_provenance();
        true
    }
}
