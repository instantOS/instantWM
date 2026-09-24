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

/// Collapse plans whose canonical tree structure is identical when split
/// weights and allocation-only split IDs are ignored. Candidate order is
/// deterministic, so retaining the first representative does not let current
/// geometry influence structural equivalence.
pub(super) fn topology_representatives(plans: Vec<PlacementPlan>) -> Vec<PlacementPlan> {
    let mut topologies = HashSet::new();
    plans
        .into_iter()
        .filter(|plan| {
            plan.candidate
                .root
                .as_ref()
                .is_some_and(|root| topologies.insert(placement_topology(root)))
        })
        .collect()
}

pub(super) fn sane_edge_fraction(edge_fraction: f64) -> f64 {
    finite_clamp(edge_fraction, 0.05, 0.49, 0.34)
}

/// Shared keyboard/pointer pipeline: drop candidates that leave the source
/// without a slot, then keep one representative per distinct topology.
pub(super) fn normalized_plans(
    source: WindowId,
    layout_rect: Rect,
    minimums: &HashMap<WindowId, Size>,
    candidates: impl IntoIterator<Item = (PlacementTarget, LayoutTree)>,
) -> Vec<PlacementPlan> {
    topology_representatives(
        candidates
            .into_iter()
            .filter_map(|(target, candidate)| {
                PlacementPlan::new(target, candidate, source, layout_rect, minimums)
            })
            .collect(),
    )
}

impl PlacementPlan {
    pub(super) fn new(
        target: PlacementTarget,
        candidate: LayoutTree,
        source: WindowId,
        layout_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Option<Self> {
        let source_slot = candidate
            .soft_constrained_bounds(layout_rect, minimums)
            .0
            .get(&source)
            .copied()?;
        Some(Self {
            target,
            candidate,
            source_slot,
        })
    }
}

impl LayoutTree {
    /// Distinct viable placement targets for `source`, as offered to keyboard
    /// navigation.
    pub(crate) fn placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
        minimums: &HashMap<WindowId, Size>,
    ) -> Vec<PlacementTarget> {
        normalized_plans(
            source,
            layout_rect,
            minimums,
            self.placement_candidates(source, layout_rect, sane_edge_fraction(edge_fraction)),
        )
        .into_iter()
        .map(|plan| plan.target)
        .collect()
    }

    /// Materialize one target previously returned by [`Self::placement_targets`].
    pub(crate) fn plan_placement(
        &self,
        source: WindowId,
        target: PlacementTarget,
        layout_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Option<PlacementPlan> {
        let candidate = match target.side {
            Some(side) => {
                self.edge_candidate(source, target.target, side, target.candidate_index)?
            }
            None => self.swapped(source, target.target)?,
        };
        PlacementPlan::new(target, candidate, source, layout_rect, minimums)
    }

    pub(super) fn swapped(&self, first: WindowId, second: WindowId) -> Option<LayoutTree> {
        let mut candidate = self.clone();
        candidate.swap_windows(first, second).then_some(candidate)
    }

    /// Every tree placing `source` on `side` of `target`, deepest scope first.
    /// A plain split beside the target is the fallback when no scope applies.
    pub(super) fn edge_candidates(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
    ) -> Vec<LayoutTree> {
        self.edge_candidates_with_geometry(source, target, side, &self.unit_bounds())
    }

    fn edge_candidates_with_geometry(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
        rects: &HashMap<NodeKey, FRect>,
    ) -> Vec<LayoutTree> {
        let candidates = self
            .edge_scopes(target, side, rects)
            .into_iter()
            .filter_map(|scope| self.moved_to_scope(source, target, side, scope))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            self.moved_beside(source, target, side)
                .into_iter()
                .collect()
        } else {
            candidates
        }
    }

    /// The `index`th entry of [`Self::edge_candidates`], materializing only
    /// the candidates up to it.
    fn edge_candidate(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
        index: usize,
    ) -> Option<LayoutTree> {
        self.edge_scopes(target, side, &self.unit_bounds())
            .into_iter()
            .filter_map(|scope| self.moved_to_scope(source, target, side, scope))
            .nth(index)
            .or_else(|| {
                (index == 0)
                    .then(|| self.moved_beside(source, target, side))
                    .flatten()
            })
    }

    fn edge_scopes(
        &self,
        target: WindowId,
        side: Side,
        rects: &HashMap<NodeKey, FRect>,
    ) -> Vec<PlacementScope> {
        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };
        let Some(target_rect) = rects.get(&NodeKey::Window(target)).copied() else {
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
        let split_depth = |id: SplitId| {
            path.iter()
                .position(|(split, _)| split.id == id)
                .map_or(0, |index| path.len() - index)
        };
        let mut candidates = exposed
            .into_iter()
            .filter(|key| rects.contains_key(key))
            .map(|key| EdgeCandidate {
                scope: PlacementScope::Node(key),
                scope_depth: match key {
                    NodeKey::Window(_) => 0,
                    NodeKey::Split(id) => split_depth(id),
                },
            })
            .collect::<Vec<_>>();

        // Recover aligned pseudo-seams, including rectangular contiguous child
        // ranges hidden by canonical same-axis flattening.
        for (index, (split, branch_index)) in path.iter().enumerate() {
            let scope_depth = path.len() - index;
            let scope_key = NodeKey::Split(split.id);
            let rect = rects[&scope_key];
            let tolerance = rect.axis_size(axis) * 0.04;
            let target_cross_size = cross_size(target_rect, axis);
            let parent_cross_size = cross_size(rect, axis);
            if parent_cross_size > target_cross_size + tolerance
                && seam > rect.axis_start(axis) + tolerance
                && seam < rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                && let Some(before) = seam_partition(&split.children, seam, axis, rects, tolerance)
            {
                candidates.push(EdgeCandidate {
                    scope: PlacementScope::AlignedNode {
                        key: scope_key,
                        seam,
                        before,
                    },
                    scope_depth,
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
                                scope_depth,
                            });
                        }
                    }
                    if cross_size(rect, axis) <= target_cross_size + tolerance
                        || seam <= rect.axis_start(axis) + tolerance
                        || seam >= rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                    {
                        continue;
                    }
                    let Some(before) = seam_partition(children, seam, axis, rects, tolerance)
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
                        scope_depth,
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
            .map(|candidate| candidate.scope)
            .collect()
    }

    pub(super) fn placement_candidates(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<(PlacementTarget, LayoutTree)> {
        let bounds = self.bounds(layout_rect);
        let unit_bounds = self.unit_bounds();
        let mut output = Vec::new();
        for target in self.leaves().into_iter().filter(|window| *window != source) {
            let Some(rect) = bounds.get(&target).copied() else {
                continue;
            };
            if let Some(candidate) = self.swapped(source, target) {
                output.push((
                    PlacementTarget {
                        target,
                        side: None,
                        candidate_index: 0,
                        position: rect.center(),
                    },
                    candidate,
                ));
            }
            for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
                let candidates =
                    self.edge_candidates_with_geometry(source, target, side, &unit_bounds);
                let candidate_count = candidates.len();
                for (index, candidate) in candidates.into_iter().enumerate() {
                    let band_fraction =
                        edge_fraction * (index as f64 + 0.5) / candidate_count as f64;
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
                    output.push((
                        PlacementTarget {
                            target,
                            side: Some(side),
                            candidate_index: index,
                            position,
                        },
                        candidate,
                    ));
                }
            }
        }
        output
    }

    /// `root` without `source`, provided both windows are distinct leaves.
    fn without_source(&self, source: WindowId, target: WindowId) -> Option<Node> {
        let root = self.root.as_ref()?;
        (source != target && root.contains(source) && root.contains(target)).then(|| {
            root.clone()
                .remove(source)
                .expect("removing one of at least two leaves leaves a root")
        })
    }

    fn from_root(root: Node, next_split_id: u64) -> LayoutTree {
        LayoutTree {
            root: Some(root),
            next_split_id,
            untouched_force_windows: Vec::new(),
        }
    }

    /// Move `source` beside `target`. The requested side selects the split axis;
    /// canonicalisation automatically inserts into an existing matching run.
    fn moved_beside(&self, source: WindowId, target: WindowId, side: Side) -> Option<LayoutTree> {
        let without_source = self.without_source(source, target)?;
        let mut next_split_id = self.next_split_id;
        let (first, second) = if side.is_leading() {
            (source, target)
        } else {
            (target, source)
        };
        let replacement = make_split(
            take_split_id(&mut next_split_id),
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
        Some(Self::from_root(
            without_source.replace_key(NodeKey::Window(target), replacement),
            next_split_id,
        ))
    }

    fn moved_to_scope(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
        scope: PlacementScope,
    ) -> Option<LayoutTree> {
        let without_source = self.without_source(source, target)?;
        let mut next_split_id = self.next_split_id;
        let mut allocate = || take_split_id(&mut next_split_id);
        let rebuilt = match scope {
            PlacementScope::Node(mut key) => {
                if !without_source.contains_key(key) {
                    key = NodeKey::Window(target);
                }
                insert_at_scope_edge(without_source, key, target, source, side, allocate())
            }
            PlacementScope::ChildRange { parent, children } => insert_at_child_range_edge(
                without_source,
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
                insert_across_aligned_node(without_source, key, &insertion, &mut allocate)
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
                    without_source,
                    parent,
                    &children,
                    &insertion,
                    &mut allocate,
                )
            }
        }?;
        Some(Self::from_root(rebuilt, next_split_id))
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
        self.clear_insertion_provenance();
        true
    }
}
