use super::*;

impl LayoutTree {
    /// Reconcile visible tiled windows using an explicit new-window policy.
    ///
    /// Existing leaves retain their topology and weights. The policy is
    /// consulted only for genuinely absent leaves, so changing configuration
    /// never rewrites an established manual layout.
    pub fn reconcile_for_layout(
        &mut self,
        visible: &[WindowId],
        policy: NewWindowPlacement,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) {
        let wanted: HashSet<_> = visible.iter().copied().collect();
        for stale in self
            .leaves()
            .into_iter()
            .filter(|window| !wanted.contains(window))
            .collect::<Vec<_>>()
        {
            self.remove(stale);
        }

        for &window in visible {
            if !self.root.as_ref().is_some_and(|root| root.contains(window)) {
                self.insert_new(window, policy, work_rect, minimums);
            }
        }
    }

    pub fn remove(&mut self, window: WindowId) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(window) {
            self.root = Some(root);
            return false;
        }
        self.root = root.remove(window);
        self.untouched_force_windows
            .retain(|candidate| *candidate != window);
        true
    }

    fn split_leaf(root: Node, target: WindowId, window: WindowId, axis: Axis, id: SplitId) -> Node {
        let split = make_split(
            id,
            axis,
            vec![
                WeightedNode {
                    node: Node::Window(window),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(target),
                    weight: 1.0,
                },
            ],
        )
        .expect("two leaves create a split");
        root.replace_window(target, split)
    }

    fn root_split(
        root: Node,
        window: WindowId,
        axis: Axis,
        newcomer_weight: f64,
        id: SplitId,
    ) -> Node {
        make_split(
            id,
            axis,
            vec![
                WeightedNode {
                    node: Node::Window(window),
                    weight: newcomer_weight,
                },
                WeightedNode {
                    node: root,
                    weight: 1.0 - newcomer_weight,
                },
            ],
        )
        .expect("a newcomer and an existing root create a split")
    }

    fn preferred_axis(rect: Rect, leading_fraction: f64) -> Axis {
        let quality = |axis| {
            let (width, height) = match axis {
                Axis::Vertical => (f64::from(rect.w) * leading_fraction, f64::from(rect.h)),
                Axis::Horizontal => (f64::from(rect.w), f64::from(rect.h) * leading_fraction),
            };
            ((width.max(1.0) / height.max(1.0)) / IDEAL_TILED_ASPECT_RATIO)
                .ln()
                .abs()
        };
        if quality(Axis::Vertical) <= quality(Axis::Horizontal) {
            Axis::Vertical
        } else {
            Axis::Horizontal
        }
    }

    /// Score every `(leaf, axis)` insertion by materializing a candidate split
    /// and solving it.
    ///
    /// This is quadratic in the leaf count — each candidate clones and re-solves
    /// the whole tree — but the leaf count per tag stays small in practice, and
    /// `make_split`'s same-axis canonicalization means a candidate's real slots
    /// can only be obtained by rebuilding the tree, not by solving the existing
    /// layout with a virtual split.
    fn automatic_insertion(
        root: &Node,
        window: WindowId,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Option<AutomaticInsertion> {
        let work_area = f64::from(work_rect.w.max(1)) * f64::from(work_rect.h.max(1));
        let mut best: Option<AutomaticInsertion> = None;

        let mut leaves = Vec::new();
        root.leaves(&mut leaves);
        for target in leaves {
            for axis in [Axis::Vertical, Axis::Horizontal] {
                let candidate = Self::split_leaf(root.clone(), target, window, axis, SplitId(0));
                let tree = Self {
                    root: Some(candidate),
                    next_split_id: 1,
                    untouched_force_windows: Vec::new(),
                };
                let constrained = tree.constrained_bounds(work_rect, minimums);
                let fits_constraints = constrained.is_some();
                let slots = constrained.unwrap_or_else(|| tree.bounds(work_rect));
                let Some(slot) = slots.get(&window).copied() else {
                    continue;
                };
                let Some(target_slot) = slots.get(&target).copied() else {
                    continue;
                };
                let aspect = f64::from(slot.w.max(1)) / f64::from(slot.h.max(1));
                let target_aspect =
                    f64::from(target_slot.w.max(1)) / f64::from(target_slot.h.max(1));
                let aspect_penalty = (aspect / IDEAL_TILED_ASPECT_RATIO).ln().abs();
                let target_aspect_penalty = (target_aspect / IDEAL_TILED_ASPECT_RATIO).ln().abs();
                let area_fraction = (f64::from(slot.w.max(1)) * f64::from(slot.h.max(1))
                    / work_area)
                    .clamp(EPSILON, 1.0);
                let area_penalty = -area_fraction.ln() * 0.25;
                let constraint_penalty = if fits_constraints { 0.0 } else { 1000.0 };
                let score =
                    constraint_penalty + aspect_penalty + target_aspect_penalty + area_penalty;

                let candidate = AutomaticInsertion {
                    score,
                    target,
                    axis,
                    newcomer_slot: slot,
                    target_slot,
                    fits_constraints,
                };
                if best.is_none_or(|best| candidate.score < best.score) {
                    best = Some(candidate);
                }
            }
        }

        best
    }

    fn insert_new(
        &mut self,
        window: WindowId,
        policy: NewWindowPlacement,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) {
        let Some(root) = self.root.take() else {
            self.root = Some(Node::Window(window));
            self.invalidate_force_provenance();
            return;
        };
        if root.contains(window) {
            self.root = Some(root);
            return;
        }

        if policy == NewWindowPlacement::Force {
            self.insert_force(root, window, work_rect, minimums);
            return;
        }

        self.root = Some(match policy {
            NewWindowPlacement::Force => unreachable!("handled above"),
            NewWindowPlacement::Auto | NewWindowPlacement::AutoResize => {
                self.invalidate_force_provenance();
                let Some(candidate) = Self::automatic_insertion(&root, window, work_rect, minimums)
                else {
                    self.root = Some(root);
                    return;
                };
                let id = self.allocate();
                if policy == NewWindowPlacement::AutoResize && !candidate.is_healthy(work_rect) {
                    let assisted_axis =
                        Self::preferred_axis(work_rect, AUTO_RESIZE_NEW_ROOT_WEIGHT);
                    let assisted = Self::root_split(
                        root.clone(),
                        window,
                        assisted_axis,
                        AUTO_RESIZE_NEW_ROOT_WEIGHT,
                        id,
                    );
                    let assisted_fits_constraints = Self {
                        root: Some(assisted.clone()),
                        next_split_id: self.next_split_id,
                        untouched_force_windows: Vec::new(),
                    }
                    .constrained_bounds(work_rect, minimums)
                    .is_some();
                    if assisted_fits_constraints || !candidate.fits_constraints {
                        assisted
                    } else {
                        Self::split_leaf(root, candidate.target, window, candidate.axis, id)
                    }
                } else {
                    Self::split_leaf(root, candidate.target, window, candidate.axis, id)
                }
            }
        });
    }

    fn insert_force(
        &mut self,
        root: Node,
        window: WindowId,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) {
        if self.untouched_force_windows.is_empty() {
            let id = self.allocate();
            self.root = Some(Self::root_split(root, window, Axis::Vertical, 0.5, id));
            self.untouched_force_windows.push(window);
            return;
        }

        let mut base = Some(root);
        let mut generated = Vec::new();
        for &candidate in &self.untouched_force_windows {
            let Some(current) = base.take() else {
                break;
            };
            if current.contains(candidate) {
                generated.push(Node::Window(candidate));
                base = current.remove(candidate);
            } else {
                base = Some(current);
            }
        }

        // A force cohort always has an older base when it is created, but that
        // base may since have closed. Keep every surviving generated leaf and
        // treat the surviving non-generated tree as one opaque layout item.
        let mut items = Vec::with_capacity(generated.len() + 2);
        items.push(Node::Window(window));
        items.extend(generated);
        if let Some(base) = base {
            items.push(base);
        }

        let (root, next_split_id) =
            Self::best_force_packing(items, work_rect, minimums, self.next_split_id);
        self.root = Some(root);
        self.next_split_id = next_split_id;
        self.untouched_force_windows.insert(0, window);
    }

    fn best_force_packing(
        items: Vec<Node>,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
        first_split_id: u64,
    ) -> (Node, u64) {
        debug_assert!(items.len() >= 2);
        let work_aspect = f64::from(work_rect.w.max(1)) / f64::from(work_rect.h.max(1));
        let mut best: Option<(f64, Node, u64)> = None;

        for outer_axis in [Axis::Vertical, Axis::Horizontal] {
            for group_count in 1..=items.len() {
                let group_sizes = balanced_group_sizes(items.len(), group_count);
                let mut penalties = Vec::with_capacity(items.len());
                for &group_size in &group_sizes {
                    let aspect = match outer_axis {
                        Axis::Vertical => work_aspect * group_size as f64 / group_count as f64,
                        Axis::Horizontal => work_aspect * group_count as f64 / group_size as f64,
                    };
                    let penalty = (aspect / IDEAL_TILED_ASPECT_RATIO).ln().abs();
                    penalties.extend(std::iter::repeat_n(penalty, group_size));
                }
                let worst = penalties.iter().copied().fold(0.0, f64::max);
                let average = penalties.iter().sum::<f64>() / penalties.len() as f64;

                let (candidate, next_split_id) =
                    build_grouped_nodes(&items, outer_axis, &group_sizes, first_split_id);
                let fits_constraints = Self {
                    root: Some(candidate.clone()),
                    next_split_id,
                    untouched_force_windows: Vec::new(),
                }
                .constrained_bounds(work_rect, minimums)
                .is_some();
                let score = worst * 2.0 + average + if fits_constraints { 0.0 } else { 1000.0 };
                if best
                    .as_ref()
                    .is_none_or(|(best_score, _, _)| score + EPSILON < *best_score)
                {
                    best = Some((score, candidate, next_split_id));
                }
            }
        }

        let (_, root, next_split_id) = best.expect("at least one force packing candidate");
        (root, next_split_id)
    }

    /// Promote a window to the primary (master) slot using force insertion,
    /// or cycle the primary slot if the window is already primary.
    ///
    /// If `window` is not currently the first visual leaf (master), it is
    /// removed from its position and re-inserted using force placement,
    /// placing it into the primary slot.
    ///
    /// If `window` is already the first visual leaf, it is swapped with the
    /// next tiled window in `candidate_order`. This changes which window
    /// occupies the primary slot without changing the tree topology, split
    /// weights, or slot geometry.
    ///
    /// Returns `Some(promoted_window_id)` if the layout was updated, or `None`
    /// if no promotion was possible (e.g. single window or empty tree).
    pub fn promote(
        &mut self,
        window: WindowId,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
        candidate_order: &[WindowId],
    ) -> Option<WindowId> {
        let leaves = self.leaves();
        if leaves.len() <= 1 {
            return None;
        }

        let is_primary = leaves.first() == Some(&window);
        if !is_primary {
            if !self.remove(window) {
                return None;
            }
            self.invalidate_force_provenance();
            self.insert_new(window, NewWindowPlacement::Force, work_rect, minimums);
            Some(window)
        } else {
            let next_primary = candidate_order
                .iter()
                .position(|candidate| *candidate == window)
                .and_then(|current| {
                    candidate_order
                        .iter()
                        .cycle()
                        .skip(current + 1)
                        .take(candidate_order.len().saturating_sub(1))
                        .find(|candidate| leaves.contains(candidate))
                        .copied()
                })
                .or_else(|| {
                    leaves
                        .iter()
                        .copied()
                        .find(|candidate| *candidate != window)
                })?;
            self.swap_windows(window, next_primary)
                .then_some(next_primary)
        }
    }
}
