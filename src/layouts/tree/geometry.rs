use super::*;

impl LayoutTree {
    pub fn bounds(&self, rect: Rect) -> HashMap<WindowId, Rect> {
        let mut float_bounds = HashMap::new();
        if let Some(root) = &self.root {
            root.bounds(FRect::from_rect(rect), &mut float_bounds);
        }
        float_bounds
            .into_iter()
            .map(|(window, rect)| (window, rect.to_rect()))
            .collect()
    }

    /// Resolve tree slots while enforcing each leaf's minimum outer size.
    ///
    /// Returns `None` when the tree cannot fit all requirements in `rect`.
    /// Callers can therefore reject a placement or resize atomically instead
    /// of asking a backend to realize overlapping geometry.
    pub fn constrained_bounds(
        &self,
        rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Option<HashMap<WindowId, Rect>> {
        let root = self.root.as_ref()?;
        let required = required_size(root, minimums);
        if required.w > rect.w || required.h > rect.h {
            return None;
        }
        let mut output = HashMap::new();
        constrained_node_bounds(root, FRect::from_rect(rect), minimums, &mut output)?;
        Some(
            output
                .into_iter()
                .map(|(window, bounds)| (window, bounds.to_rect()))
                .collect(),
        )
    }

    /// Resolve tiled slots, treating client minimum sizes as soft constraints.
    ///
    /// Minimums are honored whenever the complete tree can satisfy them. If
    /// its topology and work area make that impossible, preserve the same
    /// non-overlapping tree partition without client minimums instead.
    pub fn soft_constrained_bounds(
        &self,
        rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> (HashMap<WindowId, Rect>, bool) {
        match self.constrained_bounds(rect, minimums) {
            Some(bounds) => (bounds, true),
            None => (self.bounds(rect), false),
        }
    }

    pub(super) fn float_bounds(&self) -> HashMap<WindowId, FRect> {
        let mut output = HashMap::new();
        if let Some(root) = &self.root {
            root.bounds(
                FRect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                &mut output,
            );
        }
        output
    }

    pub(super) fn all_float_bounds(&self) -> HashMap<NodeKey, FRect> {
        let mut output = HashMap::new();
        if let Some(root) = &self.root {
            root.all_bounds(
                FRect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                &mut output,
            );
        }
        output
    }

    pub fn apply_preset(
        &mut self,
        preset: Preset,
        ordered_windows: &[WindowId],
        master_count: usize,
    ) {
        self.invalidate_force_provenance();
        let master_ratio = match preset {
            Preset::MasterStack => self.root_leading_ratio(Axis::Vertical),
            Preset::BottomStack | Preset::BottomStackHorizontal => {
                self.root_leading_ratio(Axis::Horizontal)
            }
            Preset::Grid | Preset::HorizontalGrid => 0.5,
        };
        let wanted = ordered_windows.iter().copied().collect::<HashSet<_>>();
        for stale in self
            .leaves()
            .into_iter()
            .filter(|window| !wanted.contains(window))
            .collect::<Vec<_>>()
        {
            self.remove(stale);
        }
        let mut windows = self.leaves();
        for &window in ordered_windows {
            if !windows.contains(&window) {
                windows.push(window);
            }
        }
        if windows.is_empty() {
            return;
        }
        let next = &mut self.next_split_id;
        let mut allocate = || {
            let id = SplitId(*next);
            *next = next
                .checked_add(1)
                .expect("manual-layout split id space exhausted");
            id
        };
        self.root = match preset {
            Preset::MasterStack => build_master_stack(
                &windows,
                master_count,
                master_ratio,
                Axis::Vertical,
                &mut allocate,
            ),
            Preset::BottomStack => build_master_stack(
                &windows,
                master_count,
                master_ratio,
                Axis::Horizontal,
                &mut allocate,
            ),
            Preset::Grid => build_grid(&windows, false, &mut allocate),
            Preset::HorizontalGrid => build_grid(&windows, true, &mut allocate),
            Preset::BottomStackHorizontal => {
                build_master_stack(&windows, 1, master_ratio, Axis::Horizontal, &mut allocate)
            }
        };
    }

    /// Preserve the leading share of an existing root split when a preset is
    /// reapplied. The tree is the source of truth for proportions; presets use
    /// an even split when the current root has no corresponding axis.
    fn root_leading_ratio(&self, axis: Axis) -> f64 {
        let Some(Node::Split(split)) = &self.root else {
            return 0.5;
        };
        if split.axis != axis {
            return 0.5;
        }
        split.children.first().map_or(0.5, |child| child.weight)
    }
}
