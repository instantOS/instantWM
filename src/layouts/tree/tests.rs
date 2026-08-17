use super::*;

impl LayoutTree {
    fn raw_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<PlacementTarget> {
        self.raw_resolved_placement_targets(source, layout_rect, edge_fraction)
            .into_iter()
            .map(|resolved| resolved.target)
            .collect()
    }

    fn topology_after_placement(
        &self,
        source: WindowId,
        target: PlacementTarget,
        layout_rect: Rect,
    ) -> Option<placement::PlacementTopology> {
        self.raw_resolved_placement_targets(source, layout_rect, 0.34)
            .into_iter()
            .find(|resolved| resolved.target == target)?
            .candidate
            .root
            .as_ref()
            .map(placement::placement_topology)
    }
}

fn windows(count: u32) -> Vec<WindowId> {
    (1..=count).map(WindowId).collect()
}

fn placement_session(
    tree: &LayoutTree,
    source: WindowId,
    layout_rect: Rect,
    edge_fraction: f64,
) -> TreePlacementSession {
    TreePlacementSession::new(
        tree.clone(),
        source,
        layout_rect,
        edge_fraction,
        HashMap::new(),
    )
}

fn session_targets(
    tree: &LayoutTree,
    source: WindowId,
    layout_rect: Rect,
    edge_fraction: f64,
) -> Vec<PlacementTarget> {
    placement_session(tree, source, layout_rect, edge_fraction).targets()
}

fn apply_point_plan(
    tree: &mut LayoutTree,
    source: WindowId,
    point: Point,
    layout_rect: Rect,
    edge_fraction: f64,
) -> bool {
    let Some(plan) = placement_session(tree, source, layout_rect, edge_fraction).plan_point(point)
    else {
        return false;
    };
    *tree = plan.into_tree();
    true
}

fn reconcile(tree: &mut LayoutTree, visible: &[WindowId]) {
    tree.reconcile_for_layout(
        visible,
        NewWindowPlacement::default(),
        Rect::new(0, 0, 1600, 900),
        &HashMap::new(),
    );
}

fn assert_canonical(tree: &LayoutTree) {
    fn visit(node: &Node, leaves: &mut HashSet<WindowId>, splits: &mut HashSet<SplitId>) {
        match node {
            Node::Window(window) => assert!(leaves.insert(*window), "duplicate leaf"),
            Node::Split(split) => {
                assert!(splits.insert(split.id), "duplicate split id");
                assert!(split.children.len() >= 2);
                assert!(split.children.iter().all(|child| {
                    child.weight.is_finite()
                        && child.weight > 0.0
                        && !matches!(&child.node, Node::Split(nested) if nested.axis == split.axis)
                }));
                assert!(
                    (split.children.iter().map(|child| child.weight).sum::<f64>() - 1.0).abs()
                        < EPSILON
                );
                for child in &split.children {
                    visit(&child.node, leaves, splits);
                }
            }
        }
    }

    if let Some(root) = &tree.root {
        visit(root, &mut HashSet::new(), &mut HashSet::new());
    }
}

#[test]
fn automatic_second_window_is_the_left_half_even_after_tree_collapse() {
    let area = Rect::new(0, 0, 1600, 900);
    let mut tree = LayoutTree::default();
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2)],
        NewWindowPlacement::Auto,
        area,
        &HashMap::new(),
    );
    let first = tree.bounds(area);
    assert_eq!(first[&WindowId(2)], Rect::new(0, 0, 800, 900));
    assert_eq!(first[&WindowId(1)], Rect::new(800, 0, 800, 900));

    tree.reconcile_for_layout(
        &[WindowId(1)],
        NewWindowPlacement::Auto,
        area,
        &HashMap::new(),
    );
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(3)],
        NewWindowPlacement::Auto,
        area,
        &HashMap::new(),
    );
    let second = tree.bounds(area);
    assert_eq!(second[&WindowId(3)], Rect::new(0, 0, 800, 900));
    assert_eq!(second[&WindowId(1)], Rect::new(800, 0, 800, 900));
}

#[test]
fn automatic_axis_follows_the_work_area_shape() {
    let area = Rect::new(0, 0, 900, 1600);
    let mut tree = LayoutTree::default();
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2)],
        NewWindowPlacement::Auto,
        area,
        &HashMap::new(),
    );

    let rects = tree.bounds(area);
    assert_eq!(rects[&WindowId(2)], Rect::new(0, 0, 900, 800));
    assert_eq!(rects[&WindowId(1)], Rect::new(0, 800, 900, 800));
}

#[test]
fn automatic_placement_splits_one_leaf_without_rebalancing_unrelated_space() {
    let area = Rect::new(0, 0, 1600, 900);
    let mut tree = LayoutTree::default();
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2)],
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );
    let before = tree.bounds(area)[&WindowId(1)];

    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2), WindowId(3)],
        NewWindowPlacement::Auto,
        area,
        &HashMap::new(),
    );

    assert_eq!(tree.bounds(area)[&WindowId(1)], before);
}

#[test]
fn auto_resize_uses_a_root_region_when_local_insertion_is_cramped() {
    let area = Rect::new(0, 0, 1600, 900);
    let existing = windows(8);
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&existing, Axis::Vertical, &mut || tree.allocate());
    let mut visible = existing;
    visible.push(WindowId(9));

    tree.reconcile_for_layout(
        &visible,
        NewWindowPlacement::AutoResize,
        area,
        &HashMap::new(),
    );

    let rects = tree.bounds(area);
    assert_eq!(rects[&WindowId(9)], Rect::new(0, 0, 640, 900));
    assert!(visible[..8].iter().all(|window| rects[window].x >= 640));
    assert_canonical(&tree);
}

#[test]
fn auto_resize_considers_minimum_sizes_before_accepting_a_local_split() {
    let area = Rect::new(0, 0, 1200, 770);
    let mut tree = LayoutTree::default();
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2)],
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );
    let minimums = HashMap::from([
        (WindowId(1), Size::new(200, 500)),
        (WindowId(2), Size::new(200, 500)),
        (WindowId(3), Size::new(300, 500)),
    ]);

    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2), WindowId(3)],
        NewWindowPlacement::AutoResize,
        area,
        &minimums,
    );

    let rects = tree.constrained_bounds(area, &minimums).unwrap();
    assert_eq!(rects[&WindowId(3)].x, 0);
    assert_eq!(rects[&WindowId(3)].h, area.h);
}

#[test]
fn force_always_gives_the_new_window_the_left_half() {
    let area = Rect::new(0, 0, 1000, 700);
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(4), 1);
    tree.reconcile_for_layout(
        &windows(5),
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );

    let rects = tree.bounds(area);
    assert_eq!(rects[&WindowId(5)], Rect::new(0, 0, 500, 700));
    assert!(windows(4).iter().all(|window| rects[window].x >= 500));
    assert_canonical(&tree);
}

#[test]
fn consecutive_force_spawns_adapt_before_columns_become_unhealthy() {
    let area = Rect::new(0, 0, 1600, 900);
    let mut tree = LayoutTree::default();

    tree.reconcile_for_layout(
        &windows(3),
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );

    let three = tree.bounds(area);
    assert_eq!(three[&WindowId(3)], Rect::new(0, 0, 800, 900));
    assert_eq!(three[&WindowId(2)], Rect::new(800, 0, 800, 450));
    assert_eq!(three[&WindowId(1)], Rect::new(800, 450, 800, 450));

    tree.reconcile_for_layout(
        &windows(6),
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );

    for rect in tree.bounds(area).values() {
        let aspect = f64::from(rect.w) / f64::from(rect.h);
        assert!(
            (MIN_HEALTHY_ASPECT_RATIO..=MAX_HEALTHY_ASPECT_RATIO).contains(&aspect),
            "force packing produced an unhealthy {rect:?}"
        );
    }
    assert_canonical(&tree);
}

#[test]
fn manual_edit_ends_adaptive_force_sequence() {
    let area = Rect::new(0, 0, 1600, 900);
    let mut tree = LayoutTree::default();
    tree.reconcile_for_layout(
        &windows(3),
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );
    assert!(!tree.untouched_force_windows.is_empty());

    assert!(tree.resize(WindowId(2), Side::Bottom));
    assert!(tree.untouched_force_windows.is_empty());
    tree.reconcile_for_layout(
        &windows(4),
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );

    let rects = tree.bounds(area);
    assert_eq!(rects[&WindowId(4)], Rect::new(0, 0, 800, 900));
    assert!(
        windows(3)
            .iter()
            .all(|window| rects[window].x >= area.w / 2)
    );
    assert_eq!(tree.untouched_force_windows, vec![WindowId(4)]);
}

#[test]
fn closing_a_force_spawned_window_preserves_adaptive_sequence() {
    let area = Rect::new(0, 0, 1600, 900);
    let mut tree = LayoutTree::default();
    tree.reconcile_for_layout(
        &windows(4),
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2), WindowId(4)],
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );
    tree.reconcile_for_layout(
        &[WindowId(1), WindowId(2), WindowId(4), WindowId(5)],
        NewWindowPlacement::Force,
        area,
        &HashMap::new(),
    );

    let newcomer = tree.bounds(area)[&WindowId(5)];
    assert_eq!(newcomer, Rect::new(0, 0, 800, 450));
    assert_eq!(
        tree.untouched_force_windows,
        vec![WindowId(5), WindowId(4), WindowId(2)]
    );
    assert_canonical(&tree);
}

#[test]
fn reconciliation_retains_surviving_topology_and_collapses_parents() {
    let mut tree = LayoutTree::default();
    reconcile(&mut tree, &windows(4));
    let before = tree.bounds(Rect::new(0, 0, 100, 100));
    assert_eq!(tree.len(), 4);
    reconcile(&mut tree, &[WindowId(1), WindowId(3), WindowId(4)]);
    assert_eq!(tree.len(), 3);
    assert!(!tree.leaves().contains(&WindowId(2)));
    assert!(before.contains_key(&WindowId(1)));
}

#[test]
fn same_axis_insertions_form_one_n_ary_run() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
    let Node::Split(root) = tree.root.as_ref().unwrap() else {
        panic!("expected split");
    };
    assert_eq!(root.children.len(), 3);
    assert!(
        root.children
            .iter()
            .all(|child| (child.weight - 1.0 / 3.0).abs() < EPSILON)
    );
}

#[test]
fn adjacent_same_axis_splits_are_flattened_without_losing_weights() {
    let mut next_id = 1;
    let mut allocate = || {
        let id = SplitId(next_id);
        next_id += 1;
        id
    };
    let left = equal_run(&[WindowId(1), WindowId(2)], Axis::Vertical, &mut allocate).unwrap();
    let right = equal_run(&[WindowId(3), WindowId(4)], Axis::Vertical, &mut allocate).unwrap();
    let root = make_split(
        allocate(),
        Axis::Vertical,
        vec![
            WeightedNode {
                node: left,
                weight: 0.4,
            },
            WeightedNode {
                node: right,
                weight: 0.6,
            },
        ],
    )
    .unwrap();

    let Node::Split(root) = root else {
        panic!("four leaves must remain a split");
    };
    assert_eq!(root.children.len(), 4);
    assert!(
        root.children
            .iter()
            .all(|child| matches!(child.node, Node::Window(_)))
    );
    let weights = root
        .children
        .iter()
        .map(|child| child.weight)
        .collect::<Vec<_>>();
    for (actual, expected) in weights.into_iter().zip([0.2, 0.2, 0.3, 0.3]) {
        assert!((actual - expected).abs() < EPSILON);
    }
}

#[test]
fn grid_is_a_persistent_tree_transformation() {
    let mut tree = LayoutTree::default();
    let wins = windows(4);
    tree.apply_preset(Preset::Grid, &wins, 1);
    let rects = tree.bounds(Rect::new(0, 0, 100, 100));
    assert_eq!(rects[&WindowId(1)], Rect::new(0, 0, 50, 50));
    assert_eq!(rects[&WindowId(4)], Rect::new(50, 50, 50, 50));
    assert!(tree.resize(WindowId(1), Side::Right));
    assert_ne!(tree.bounds(Rect::new(0, 0, 100, 100)), rects);
}

#[test]
fn traversal_uses_first_structural_seam() {
    let mut tree = LayoutTree::default();
    let wins = windows(4);
    tree.apply_preset(Preset::Grid, &wins, 1);
    assert_eq!(
        tree.visual_neighbor(WindowId(4), Side::Left),
        Some(WindowId(2))
    );
    assert_eq!(
        tree.visual_neighbor(WindowId(4), Side::Top),
        Some(WindowId(3))
    );
}

#[test]
fn resize_preserves_peer_ratios() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
    assert!(tree.resize(WindowId(2), Side::Right));
    let rects = tree.bounds(Rect::new(0, 0, 300, 100));
    assert!(rects[&WindowId(2)].w > 100);
    assert_eq!(rects[&WindowId(1)].w, rects[&WindowId(3)].w);
}

#[test]
fn resize_stops_at_the_local_run_across_an_orthogonal_parent() {
    let mut tree = LayoutTree::default();
    let local_id = tree.allocate();
    let row_id = tree.allocate();
    let root_id = tree.allocate();
    let local = make_split(
        local_id,
        Axis::Vertical,
        windows(2)
            .into_iter()
            .map(|window| WeightedNode {
                node: Node::Window(window),
                weight: 1.0,
            })
            .collect(),
    )
    .unwrap();
    let row = make_split(
        row_id,
        Axis::Horizontal,
        vec![
            WeightedNode {
                node: local,
                weight: 1.0,
            },
            WeightedNode {
                node: Node::Window(WindowId(3)),
                weight: 1.0,
            },
        ],
    )
    .unwrap();
    tree.root = make_split(
        root_id,
        Axis::Vertical,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(4)),
                weight: 1.0,
            },
            WeightedNode {
                node: row,
                weight: 1.0,
            },
        ],
    );
    let before = tree.bounds(Rect::new(0, 0, 100, 100));
    assert!(tree.resize(WindowId(1), Side::Right));
    let after = tree.bounds(Rect::new(0, 0, 100, 100));
    assert_eq!(after[&WindowId(4)].w, before[&WindowId(4)].w);
    assert!(after[&WindowId(1)].w > before[&WindowId(1)].w);
    assert!(after[&WindowId(2)].w < before[&WindowId(2)].w);
}

#[test]
fn repeated_swaps_walk_visual_neighbors_without_changing_topology() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(4), Axis::Horizontal, &mut || tree.allocate());
    for neighbor in [WindowId(3), WindowId(2), WindowId(1)] {
        assert_eq!(
            tree.swap_with_neighbor(WindowId(4), Side::Top),
            Some(neighbor)
        );
        assert_canonical(&tree);
    }
    assert_eq!(
        tree.leaves(),
        vec![WindowId(4), WindowId(1), WindowId(2), WindowId(3)]
    );
    assert_eq!(tree.swap_with_neighbor(WindowId(4), Side::Top), None);
}

#[test]
fn t_junction_swap_exchanges_complete_visual_slots() {
    let mut tree = LayoutTree::default();
    let left_id = tree.allocate();
    let root_id = tree.allocate();
    let left = make_split(
        left_id,
        Axis::Horizontal,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(1)),
                weight: 1.0,
            },
            WeightedNode {
                node: Node::Window(WindowId(2)),
                weight: 1.0,
            },
        ],
    )
    .unwrap();
    tree.root = make_split(
        root_id,
        Axis::Vertical,
        vec![
            WeightedNode {
                node: left,
                weight: 1.0,
            },
            WeightedNode {
                node: Node::Window(WindowId(3)),
                weight: 1.0,
            },
        ],
    );
    assert_eq!(
        tree.swap_with_neighbor(WindowId(1), Side::Right),
        Some(WindowId(3))
    );
    let rects = tree.bounds(Rect::new(0, 0, 100, 100));
    assert_eq!(rects[&WindowId(1)], Rect::new(50, 0, 50, 100));
    assert_eq!(rects[&WindowId(3)], Rect::new(0, 0, 50, 50));
    assert_canonical(&tree);
}

#[test]
fn pointer_center_swaps_and_edge_reparents() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
    assert!(apply_point_plan(
        &mut tree,
        WindowId(1),
        Point::new(150, 50),
        Rect::new(0, 0, 300, 100),
        0.3
    ));
    assert_eq!(tree.leaves(), vec![WindowId(2), WindowId(1), WindowId(3)]);
    assert!(apply_point_plan(
        &mut tree,
        WindowId(1),
        Point::new(201, 50),
        Rect::new(0, 0, 300, 100),
        0.3
    ));
    assert_eq!(tree.len(), 3);
}

#[test]
fn aligned_seam_can_target_a_contiguous_virtual_scope() {
    let mut tree = LayoutTree::default();
    let column_one_id = tree.allocate();
    let column_two_id = tree.allocate();
    let root_id = tree.allocate();
    let column_one = make_split(
        column_one_id,
        Axis::Horizontal,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(2)),
                weight: 1.0,
            },
            WeightedNode {
                node: Node::Window(WindowId(3)),
                weight: 1.0,
            },
        ],
    )
    .unwrap();
    let column_two = make_split(
        column_two_id,
        Axis::Horizontal,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(4)),
                weight: 1.0,
            },
            WeightedNode {
                node: Node::Window(WindowId(5)),
                weight: 1.0,
            },
        ],
    )
    .unwrap();
    tree.root = make_split(
        root_id,
        Axis::Vertical,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(1)),
                weight: 0.25,
            },
            WeightedNode {
                node: column_one,
                weight: 0.25,
            },
            WeightedNode {
                node: column_two,
                weight: 0.25,
            },
            WeightedNode {
                node: Node::Window(WindowId(6)),
                weight: 0.25,
            },
        ],
    );

    // Window 6 moves to the seam above window 3. Window 1 crosses that
    // seam, so only the contiguous two-column grid is a valid scope.
    assert!(apply_point_plan(
        &mut tree,
        WindowId(6),
        Point::new(26, 51),
        Rect::new(0, 0, 100, 100),
        0.34,
    ));
    let rects = tree.bounds(Rect::new(0, 0, 100, 100));
    // Removing the former right-hand peer normalises the three surviving
    // outer items, so the full-height leaf keeps its ratio to both grid
    // columns (1:1:1) rather than its old absolute percentage.
    assert_eq!(rects[&WindowId(1)], Rect::new(0, 0, 33, 100));
    assert_eq!(rects[&WindowId(6)].x, 33);
    assert_eq!(rects[&WindowId(6)].w, 67);
    // The source occupies the seam itself; the two original rows remain
    // on either side instead of being compressed together below it.
    assert_eq!(rects[&WindowId(6)].y, 33);
    assert_eq!(rects[&WindowId(6)].h, 34);
}

#[test]
fn outer_edge_can_target_a_contiguous_child_range() {
    let source = WindowId(1);
    let mut tree = LayoutTree::default();
    let stack = equal_run(
        &[WindowId(2), WindowId(3), WindowId(4), WindowId(5)],
        Axis::Horizontal,
        &mut || tree.allocate(),
    )
    .unwrap();
    tree.root = make_split(
        tree.allocate(),
        Axis::Vertical,
        vec![
            WeightedNode {
                node: Node::Window(source),
                weight: 1.0,
            },
            WeightedNode {
                node: stack,
                weight: 1.0,
            },
        ],
    );
    let rect = Rect::new(0, 0, 1000, 800);
    let desired_slot = Rect::new(500, 0, 500, 400);
    let raw = tree.resolved_edge_candidates(source, WindowId(2), Side::Right);
    assert!(
        raw.iter()
            .any(|(_, candidate)| candidate.bounds(rect)[&source] == desired_slot),
        "the outer edge of the top window must expose the top-two child range"
    );

    let normalized = LayoutTree::normalized_soft_constrained_candidates(
        source,
        rect,
        &HashMap::new(),
        raw.into_iter()
            .enumerate()
            .map(
                |(candidate_index, (_, candidate))| ResolvedPlacementTarget {
                    target: PlacementTarget {
                        target: WindowId(2),
                        side: Some(Side::Right),
                        candidate_index,
                        position: Point::new(0, 0),
                    },
                    candidate,
                },
            ),
    );
    assert!(
        normalized
            .iter()
            .any(|plan| plan.source_slot() == desired_slot),
        "normalization must preserve the structurally distinct top-two range"
    );
}

#[test]
fn keyboard_and_pointer_targets_apply_the_same_semantic_candidate() {
    let mut original = LayoutTree::default();
    original.root = equal_run(&windows(3), Axis::Vertical, &mut || original.allocate());
    let rect = Rect::new(0, 0, 300, 100);
    let target = session_targets(&original, WindowId(1), rect, 0.34)
        .into_iter()
        .find(|target| target.side.is_some())
        .unwrap();

    let keyboard = placement_session(&original, WindowId(1), rect, 0.34)
        .plan_target(target)
        .unwrap()
        .into_tree();
    let mut pointer = original;
    assert!(apply_point_plan(
        &mut pointer,
        WindowId(1),
        target.position,
        rect,
        0.34
    ));
    assert_eq!(keyboard.bounds(rect), pointer.bounds(rect));
}

#[test]
fn adjacent_descriptions_of_the_same_seam_share_one_candidate() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(4), Axis::Vertical, &mut || tree.allocate());
    let source = WindowId(4);
    let rect = Rect::new(0, 0, 400, 100);
    let raw = tree.raw_placement_targets(source, rect, 0.34);
    let right_of_a = raw
        .iter()
        .copied()
        .find(|target| target.target == WindowId(1) && target.side == Some(Side::Right))
        .expect("right edge of A is an advertised raw target");
    let left_of_b = raw
        .iter()
        .copied()
        .find(|target| target.target == WindowId(2) && target.side == Some(Side::Left))
        .expect("left edge of B is an advertised raw target");
    let expected = tree
        .topology_after_placement(source, right_of_a, rect)
        .unwrap();
    assert_eq!(
        expected,
        tree.topology_after_placement(source, left_of_b, rect)
            .unwrap(),
        "both descriptions address the seam between A and B"
    );

    let equivalent_targets = session_targets(&tree, source, rect, 0.34)
        .into_iter()
        .filter(|target| {
            tree.topology_after_placement(source, *target, rect)
                .as_ref()
                == Some(&expected)
        })
        .count();
    assert_eq!(equivalent_targets, 1);

    let mut from_a = tree.clone();
    let mut from_b = tree;
    assert!(apply_point_plan(
        &mut from_a,
        source,
        right_of_a.position,
        rect,
        0.34
    ));
    assert!(apply_point_plan(
        &mut from_b,
        source,
        left_of_b.position,
        rect,
        0.34
    ));
    assert_eq!(from_a.bounds(rect), from_b.bounds(rect));
}

#[test]
fn placement_topology_ignores_weights_but_preserves_structure() {
    let first = make_split(
        SplitId(1),
        Axis::Vertical,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(1)),
                weight: 0.2,
            },
            WeightedNode {
                node: Node::Window(WindowId(2)),
                weight: 0.8,
            },
        ],
    )
    .unwrap();
    let resized = make_split(
        SplitId(99),
        Axis::Vertical,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(1)),
                weight: 0.7,
            },
            WeightedNode {
                node: Node::Window(WindowId(2)),
                weight: 0.3,
            },
        ],
    )
    .unwrap();
    let different_axis = make_split(
        SplitId(1),
        Axis::Horizontal,
        vec![
            WeightedNode {
                node: Node::Window(WindowId(1)),
                weight: 0.2,
            },
            WeightedNode {
                node: Node::Window(WindowId(2)),
                weight: 0.8,
            },
        ],
    )
    .unwrap();

    assert_eq!(
        placement::placement_topology(&first),
        placement::placement_topology(&resized)
    );
    assert_ne!(
        placement::placement_topology(&first),
        placement::placement_topology(&different_axis)
    );
}

#[test]
fn normalization_preserves_different_topologies_with_identical_geometry() {
    fn split(axis: Axis, children: Vec<Node>, next_id: &mut u64) -> Node {
        let id = SplitId(*next_id);
        *next_id += 1;
        make_split(
            id,
            axis,
            children
                .into_iter()
                .map(|node| WeightedNode { node, weight: 1.0 })
                .collect(),
        )
        .unwrap()
    }

    let mut next_id = 1;
    let left_column = split(
        Axis::Horizontal,
        vec![Node::Window(WindowId(1)), Node::Window(WindowId(2))],
        &mut next_id,
    );
    let right_column = split(
        Axis::Horizontal,
        vec![Node::Window(WindowId(3)), Node::Window(WindowId(4))],
        &mut next_id,
    );
    let columns = LayoutTree {
        root: Some(split(
            Axis::Vertical,
            vec![left_column, right_column],
            &mut next_id,
        )),
        ..LayoutTree::default()
    };

    let top_row = split(
        Axis::Vertical,
        vec![Node::Window(WindowId(1)), Node::Window(WindowId(3))],
        &mut next_id,
    );
    let bottom_row = split(
        Axis::Vertical,
        vec![Node::Window(WindowId(2)), Node::Window(WindowId(4))],
        &mut next_id,
    );
    let rows = LayoutTree {
        root: Some(split(
            Axis::Horizontal,
            vec![top_row, bottom_row],
            &mut next_id,
        )),
        ..LayoutTree::default()
    };

    let rect = Rect::new(0, 0, 100, 100);
    assert_eq!(columns.bounds(rect), rows.bounds(rect));
    assert_ne!(
        placement::placement_topology(columns.root.as_ref().unwrap()),
        placement::placement_topology(rows.root.as_ref().unwrap())
    );
    let candidates = vec![
        (
            ResolvedPlacementTarget {
                target: PlacementTarget {
                    target: WindowId(1),
                    side: Some(Side::Right),
                    candidate_index: 0,
                    position: Point::new(0, 0),
                },
                candidate: columns,
            },
            (),
        ),
        (
            ResolvedPlacementTarget {
                target: PlacementTarget {
                    target: WindowId(1),
                    side: Some(Side::Right),
                    candidate_index: 1,
                    position: Point::new(0, 0),
                },
                candidate: rows,
            },
            (),
        ),
    ];
    assert_eq!(placement::topology_representatives(candidates).len(), 2);
}

#[test]
fn keyboard_navigation_steps_through_distinct_topologies() {
    use crate::core_state::KeyboardTreePlacement;
    use crate::types::{MonitorId, TagMask};

    let mut tree = LayoutTree::default();
    let top_id = tree.allocate();
    let bottom_id = tree.allocate();
    let root_id = tree.allocate();
    let top = make_split(
        top_id,
        Axis::Vertical,
        vec![WindowId(1), WindowId(2)]
            .into_iter()
            .map(|window| WeightedNode {
                node: Node::Window(window),
                weight: 1.0,
            })
            .collect(),
    )
    .unwrap();
    let bottom = make_split(
        bottom_id,
        Axis::Vertical,
        vec![WindowId(4), WindowId(5)]
            .into_iter()
            .map(|window| WeightedNode {
                node: Node::Window(window),
                weight: 1.0,
            })
            .collect(),
    )
    .unwrap();
    tree.root = make_split(
        root_id,
        Axis::Horizontal,
        vec![
            WeightedNode {
                node: top,
                weight: 0.35,
            },
            WeightedNode {
                node: Node::Window(WindowId(3)),
                weight: 0.35,
            },
            WeightedNode {
                node: bottom,
                weight: 0.30,
            },
        ],
    );

    let source = WindowId(1);
    let layout_rect = Rect::new(0, 0, 1234, 657);
    let targets = session_targets(&tree, source, layout_rect, 0.34);
    let mut placement = KeyboardTreePlacement::new_nearest(
        source,
        MonitorId::default(),
        TagMask::EMPTY,
        targets,
        tree.bounds(layout_rect)[&source].center(),
    )
    .unwrap();
    assert!(placement.select_direction(Side::Top));
    assert!(placement.select_direction(Side::Right));
    assert!(placement.select_direction(Side::Bottom));
    let first_down = tree
        .topology_after_placement(source, placement.selected_target(), layout_rect)
        .unwrap();

    assert!(placement.select_direction(Side::Bottom));
    let second_down = tree
        .topology_after_placement(source, placement.selected_target(), layout_rect)
        .unwrap();

    assert_ne!(
        first_down, second_down,
        "successive navigation steps must not expose equivalent topologies"
    );
}

#[test]
fn session_normalization_matches_topology_reference() {
    let rect = Rect::new(0, 0, 1600, 900);
    for preset in [Preset::Grid, Preset::MasterStack, Preset::BottomStack] {
        let mut tree = LayoutTree::default();
        tree.apply_preset(preset, &windows(8), 2);

        let candidates = tree
            .raw_resolved_placement_targets(WindowId(1), rect, 0.34)
            .into_iter()
            .map(|resolved| (resolved, ()))
            .collect();
        let reference = placement::topology_representatives(candidates)
            .into_iter()
            .map(|(resolved, ())| resolved.target)
            .collect::<Vec<_>>();

        assert_eq!(session_targets(&tree, WindowId(1), rect, 0.34), reference);
    }
}

#[test]
fn soft_constrained_normalization_preserves_every_distinct_viable_outcome() {
    let rect = Rect::new(0, 0, 2000, 1200);
    let minimums = windows(20)
        .into_iter()
        .map(|window| (window, Size::new(120, 90)))
        .collect::<HashMap<_, _>>();
    for preset in [Preset::Grid, Preset::MasterStack, Preset::BottomStack] {
        let mut tree = LayoutTree::default();
        tree.apply_preset(preset, &windows(20), 2);

        let candidates = tree
            .raw_resolved_placement_targets(WindowId(1), rect, 0.34)
            .into_iter()
            .filter_map(|resolved| {
                let slot = resolved
                    .candidate
                    .soft_constrained_bounds(rect, &minimums)
                    .0
                    .get(&WindowId(1))
                    .copied()?;
                Some((resolved, slot))
            })
            .collect();
        let reference = placement::topology_representatives(candidates)
            .into_iter()
            .map(|(resolved, _)| resolved.target)
            .collect::<Vec<_>>();

        assert_eq!(
            TreePlacementSession::new(tree.clone(), WindowId(1), rect, 0.34, minimums.clone(),)
                .targets(),
            reference,
            "constraint filtering must happen before distinct viable outcomes are normalized"
        );
    }
}

#[test]
fn placement_preview_is_exact_and_does_not_mutate_the_tree() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(4), 1);
    let rect = Rect::new(0, 0, 400, 300);
    let before = tree.bounds(rect);
    let session = placement_session(&tree, WindowId(1), rect, 0.34);
    let target = session
        .targets()
        .into_iter()
        .find(|target| target.target == WindowId(4) && target.side == Some(Side::Left))
        .unwrap();

    let plan = session.plan_target(target).unwrap();
    let preview = plan.source_slot();
    assert_eq!(tree.bounds(rect), before);

    let applied = plan.into_tree();
    assert_eq!(applied.bounds(rect)[&WindowId(1)], preview);
}

#[test]
fn keyboard_session_work_does_not_compute_pointer_hit_bounds() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(6), 1);
    let rect = Rect::new(20, 30, 600, 400);
    let session = placement_session(&tree, WindowId(1), rect, 0.34);
    assert!(session.bounds.is_none());

    let target = session.targets().into_iter().next().unwrap();
    assert!(session.bounds.is_none());
    assert!(session.plan_target(target).is_some());
    assert!(session.bounds.is_none());
}

#[test]
fn pointer_placement_preview_matches_release_and_does_not_mutate() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(6), 1);
    let rect = Rect::new(20, 30, 600, 400);
    let before = tree.bounds(rect);
    let points = session_targets(&tree, WindowId(1), rect, 0.34)
        .into_iter()
        .filter(|target| target.target == WindowId(5))
        .map(|target| target.position)
        .collect::<Vec<_>>();
    assert!(points.len() > 1, "exercise both centre and edge targets");

    for point in points {
        let plan = placement_session(&tree, WindowId(1), rect, 0.34)
            .plan_point(point)
            .expect("advertised placement point must be valid");
        let preview = plan.source_slot();
        assert_eq!(tree.bounds(rect), before);

        let applied = plan.into_tree();
        assert_eq!(applied.bounds(rect)[&WindowId(1)], preview);
    }
}

#[test]
fn placement_session_reuses_a_target_edge_without_changing_its_preview() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(20), 1);
    let rect = Rect::new(0, 0, 2000, 1000);
    let point = Point::new(801, 625);
    let expected = placement_session(&tree, WindowId(1), rect, 0.34)
        .preview_point(point)
        .unwrap();
    let mut session = placement_session(&tree, WindowId(1), rect, 0.34);

    assert_eq!(session.preview_point(point), Some(expected));
    assert_eq!(session.edge_resolutions.len(), 1);
    assert_eq!(
        session.preview_point(Point::new(point.x + 1, point.y)),
        Some(expected)
    );
    assert_eq!(
        session.edge_resolutions.len(),
        1,
        "motion within one target edge must reuse its solved candidates"
    );
}

#[test]
fn twenty_window_pointer_edges_divide_hitboxes_by_distinct_viable_outcomes() {
    let source = WindowId(1);
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(20), 1);
    let rect = Rect::new(0, 0, 20_000, 10_000);
    let bounds = tree.bounds(rect);
    let minimums = HashMap::new();

    let duplicated_edge = tree
        .leaves()
        .into_iter()
        .filter(|target| *target != source)
        .flat_map(|target| {
            [Side::Left, Side::Right, Side::Top, Side::Bottom]
                .into_iter()
                .map(move |side| (target, side))
        })
        .find_map(|(target, side)| {
            let raw = tree.resolved_edge_candidates(source, target, side);
            let raw_count = raw.len();
            let resolved = raw
                .into_iter()
                .enumerate()
                .map(
                    |(candidate_index, (_, candidate))| ResolvedPlacementTarget {
                        target: PlacementTarget {
                            target,
                            side: Some(side),
                            candidate_index,
                            position: bounds[&target].center(),
                        },
                        candidate,
                    },
                )
                .collect::<Vec<_>>();
            let normalized = LayoutTree::normalized_soft_constrained_candidates(
                source, rect, &minimums, resolved,
            );
            (normalized.len() < raw_count).then_some((target, side, raw_count, normalized))
        })
        .expect("the 20-window grid should exercise weight-only topology duplicates");

    let (target, side, raw_count, normalized) = duplicated_edge;
    assert!(
        normalized.len() < raw_count,
        "normalization must remove duplicate hitbox bands"
    );

    let target_rect = bounds[&target];
    let inset_x = (f64::from(target_rect.w) * 0.34).max(1.0);
    let inset_y = (f64::from(target_rect.h) * 0.34).max(1.0);
    let mut session = TreePlacementSession::new(tree.clone(), source, rect, 0.34, minimums);
    for (index, expected) in normalized.iter().enumerate() {
        let distance = (index as f64 + 0.5) / normalized.len() as f64;
        let point = match side {
            Side::Left => Point::new(
                target_rect.x + (inset_x * distance).round() as i32,
                target_rect.center().y,
            ),
            Side::Right => Point::new(
                target_rect.right() - (inset_x * distance).round() as i32,
                target_rect.center().y,
            ),
            Side::Top => Point::new(
                target_rect.center().x,
                target_rect.y + (inset_y * distance).round() as i32,
            ),
            Side::Bottom => Point::new(
                target_rect.center().x,
                target_rect.bottom() - (inset_y * distance).round() as i32,
            ),
        };
        let actual = session
            .plan_point(point)
            .expect("the midpoint of every normalized band must resolve");
        assert_eq!(actual.target(), expected.target());
        assert_eq!(actual.source_slot(), expected.source_slot());

        let applied = actual.into_tree();
        assert_eq!(
            applied.soft_constrained_bounds(rect, &HashMap::new()).0[&source],
            expected.source_slot(),
            "preview and release must use the same retained semantic candidate"
        );
    }

    assert_eq!(
        session.edge_resolutions[&(target, side)].len(),
        normalized.len(),
        "only normalized outcomes may consume pointer hitbox width"
    );
}

#[test]
fn twenty_window_pointer_candidates_collapse_weight_only_variants() {
    let source = WindowId(1);
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(20), 1);
    let rect = Rect::new(0, 0, 2000, 1000);
    let mut found_weight_only_duplicates = false;
    for target in tree.leaves().into_iter().filter(|window| *window != source) {
        for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
            let raw = tree.resolved_edge_candidates(source, target, side);
            let viable = raw
                .into_iter()
                .enumerate()
                .filter_map(|(candidate_index, (_, candidate))| {
                    let slot = candidate
                        .constrained_bounds(rect, &HashMap::new())?
                        .get(&source)
                        .copied()?;
                    Some((
                        ResolvedPlacementTarget {
                            target: PlacementTarget {
                                target,
                                side: Some(side),
                                candidate_index,
                                position: Point::new(0, 0),
                            },
                            candidate,
                        },
                        slot,
                    ))
                })
                .collect::<Vec<_>>();
            let topology_count = viable
                .iter()
                .filter_map(|(resolved, _)| {
                    resolved
                        .candidate
                        .root
                        .as_ref()
                        .map(placement::placement_topology)
                })
                .collect::<HashSet<_>>()
                .len();
            found_weight_only_duplicates |= viable.len() > topology_count;

            let representatives = placement::topology_representatives(viable.clone());
            assert_eq!(
                representatives.len(),
                topology_count,
                "each unweighted canonical topology must have exactly one representative"
            );
            let normalized = LayoutTree::normalized_soft_constrained_candidates(
                source,
                rect,
                &HashMap::new(),
                viable.into_iter().map(|(resolved, _)| resolved),
            );
            assert_eq!(
                normalized.len(),
                topology_count,
                "normalization must remove only weight variants of the same topology"
            );
        }
    }
    assert!(
        found_weight_only_duplicates,
        "the 20-window grid must reproduce the weight-only candidate duplication"
    );
}

#[test]
fn pointer_resize_tracks_the_grabbed_edge_in_pixels() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(2), 1);
    let layout = Rect::new(0, 0, 1000, 600);
    let before = tree.bounds(layout)[&WindowId(1)];

    assert!(tree.resize_by_pixels(WindowId(1), Side::Right, 120, layout, 0.15));
    let after = tree.bounds(layout)[&WindowId(1)];

    assert_eq!(after.w - before.w, 120);
    assert_canonical(&tree);
}

#[test]
fn pointer_resize_moves_only_the_grabbed_seam() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
    let layout = Rect::new(0, 0, 900, 600);
    let before = tree.bounds(layout);

    assert!(tree.resize_edge_by_pixels(WindowId(2), Side::Right, 90, layout, 0.15));
    let after = tree.bounds(layout);

    assert_eq!(after[&WindowId(1)], before[&WindowId(1)]);
    assert_eq!(after[&WindowId(2)].x, before[&WindowId(2)].x);
    assert_eq!(after[&WindowId(2)].w - before[&WindowId(2)].w, 90);
    assert_eq!(after[&WindowId(3)].x - before[&WindowId(3)].x, 90);
    assert_eq!(
        after[&WindowId(3)].x + after[&WindowId(3)].w,
        before[&WindowId(3)].x + before[&WindowId(3)].w
    );
}

#[test]
fn dense_equal_pointer_runs_have_a_valid_resize_interval() {
    let layout = Rect::new(0, 0, 1300, 700);
    for count in [7, 13] {
        let mut trailing = LayoutTree::default();
        trailing.root = equal_run(&windows(count), Axis::Vertical, &mut || trailing.allocate());
        let mut leading = trailing.clone();

        assert!(trailing.resize_edge_by_pixels(WindowId(1), Side::Right, 20, layout, 0.15,));
        assert!(leading.resize_edge_by_pixels(WindowId(count), Side::Left, -20, layout, 0.15,));
        assert_canonical(&trailing);
        assert_canonical(&leading);
    }
}

#[test]
fn pointer_outer_edge_resize_matches_keyboard_peer_redistribution() {
    let mut keyboard = LayoutTree::default();
    keyboard.root = equal_run(&windows(4), Axis::Vertical, &mut || keyboard.allocate());
    let mut pointer = keyboard.clone();
    let layout = Rect::new(0, 0, 1000, 600);

    assert!(keyboard.resize_by_pixels(WindowId(1), Side::Right, 100, layout, 0.15));
    assert!(pointer.resize_edge_by_pixels(WindowId(1), Side::Right, 100, layout, 0.15));

    assert_eq!(pointer.bounds(layout), keyboard.bounds(layout));
    assert_canonical(&pointer);
}

#[test]
fn pointer_resize_preserves_ratios_only_on_the_grabbed_side() {
    let mut tree = LayoutTree::default();
    let split_id = tree.allocate();
    tree.root = make_split(
        split_id,
        Axis::Vertical,
        windows(4)
            .into_iter()
            .zip([0.2, 0.2, 0.4, 0.2])
            .map(|(window, weight)| WeightedNode {
                node: Node::Window(window),
                weight,
            })
            .collect(),
    );
    let layout = Rect::new(0, 0, 1000, 600);
    let before = tree.bounds(layout);

    assert!(tree.resize_edge_by_pixels(WindowId(2), Side::Right, 100, layout, 0.05));
    let after = tree.bounds(layout);

    assert_eq!(after[&WindowId(1)], before[&WindowId(1)]);
    assert_eq!(after[&WindowId(2)].x, before[&WindowId(2)].x);
    assert_eq!(after[&WindowId(2)].w - before[&WindowId(2)].w, 100);
    assert!(
        (after[&WindowId(3)].w - 2 * after[&WindowId(4)].w).abs() <= 1,
        "integer rounding may differ by one pixel, but the 2:1 ratio must remain"
    );
    assert_eq!(
        after[&WindowId(4)].x + after[&WindowId(4)].w,
        before[&WindowId(4)].x + before[&WindowId(4)].w
    );
    assert_canonical(&tree);
}

#[test]
fn pointer_leading_edge_preserves_ratios_and_the_opposite_edge() {
    let mut tree = LayoutTree::default();
    let split_id = tree.allocate();
    tree.root = make_split(
        split_id,
        Axis::Vertical,
        windows(4)
            .into_iter()
            .zip([0.2, 0.4, 0.2, 0.2])
            .map(|(window, weight)| WeightedNode {
                node: Node::Window(window),
                weight,
            })
            .collect(),
    );
    let layout = Rect::new(0, 0, 1000, 600);
    let before = tree.bounds(layout);

    assert!(tree.resize_edge_by_pixels(WindowId(3), Side::Left, -100, layout, 0.05));
    let after = tree.bounds(layout);

    assert_eq!(after[&WindowId(4)], before[&WindowId(4)]);
    assert_eq!(
        after[&WindowId(3)].x + after[&WindowId(3)].w,
        before[&WindowId(3)].x + before[&WindowId(3)].w
    );
    assert_eq!(after[&WindowId(3)].w - before[&WindowId(3)].w, 100);
    assert!(
        (after[&WindowId(2)].w - 2 * after[&WindowId(1)].w).abs() <= 1,
        "leading peers must retain their 2:1 ratio"
    );
    assert_canonical(&tree);
}

#[test]
fn constrained_bounds_reserve_minimums_without_overlap() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
    let layout = Rect::new(10, 20, 300, 100);
    let minimums = HashMap::from([
        (WindowId(1), Size::new(40, 50)),
        (WindowId(2), Size::new(160, 50)),
        (WindowId(3), Size::new(40, 50)),
    ]);

    let bounds = tree.constrained_bounds(layout, &minimums).unwrap();

    assert!(bounds[&WindowId(2)].w >= 160);
    assert_eq!(
        bounds[&WindowId(1)].x + bounds[&WindowId(1)].w,
        bounds[&WindowId(2)].x
    );
    assert_eq!(
        bounds[&WindowId(2)].x + bounds[&WindowId(2)].w,
        bounds[&WindowId(3)].x
    );
    assert_eq!(
        bounds[&WindowId(3)].x + bounds[&WindowId(3)].w,
        layout.x + layout.w
    );
    assert!(bounds.values().all(|rect| {
        rect.x >= layout.x
            && rect.y >= layout.y
            && rect.x + rect.w <= layout.x + layout.w
            && rect.y + rect.h <= layout.y + layout.h
    }));
}

#[test]
fn constrained_bounds_reject_impossible_minimums() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(2), Axis::Vertical, &mut || tree.allocate());
    let minimums = HashMap::from([
        (WindowId(1), Size::new(151, 50)),
        (WindowId(2), Size::new(150, 50)),
    ]);

    assert!(
        tree.constrained_bounds(Rect::new(0, 0, 300, 100), &minimums)
            .is_none()
    );
}

#[test]
fn soft_constraints_preserve_tree_geometry_when_minimums_are_impossible() {
    let mut tree = LayoutTree::default();
    tree.root = equal_run(&windows(2), Axis::Vertical, &mut || tree.allocate());
    let layout = Rect::new(0, 0, 300, 100);
    let minimums = HashMap::from([
        (WindowId(1), Size::new(200, 50)),
        (WindowId(2), Size::new(200, 50)),
    ]);

    let (bounds, honored) = tree.soft_constrained_bounds(layout, &minimums);

    assert!(!honored);
    assert_eq!(bounds, tree.bounds(layout));
    assert_eq!(bounds[&WindowId(1)].right(), bounds[&WindowId(2)].x);
    assert_eq!(bounds[&WindowId(2)].right(), layout.right());
}

#[test]
fn leading_edge_motion_has_the_opposite_weight_sign() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(2), 1);
    let layout = Rect::new(0, 0, 1000, 600);
    let before = tree.bounds(layout)[&WindowId(1)];

    assert!(tree.resize_by_pixels(WindowId(1), Side::Left, 100, layout, 0.15));
    let after = tree.bounds(layout)[&WindowId(1)];

    assert_eq!(before.w - after.w, 100);
}

#[test]
fn resize_axis_reports_only_structural_runs() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(2), 1);

    assert!(tree.can_resize_axis(WindowId(1), Axis::Vertical));
    assert!(!tree.can_resize_axis(WindowId(1), Axis::Horizontal));
}

#[test]
fn stock_presets_build_their_signature_geometries() {
    // 1200x800 divides evenly for every group count below, so each preset's
    // stock bounds are exactly predictable.
    let layout = Rect::new(0, 0, 1200, 800);
    let wins = windows(5);

    // Grid: columns-first, two full columns before the remainder column.
    let mut grid = LayoutTree::default();
    grid.apply_preset(Preset::Grid, &wins, 1);
    let rects = grid.bounds(layout);
    assert_eq!(rects[&WindowId(1)], Rect::new(0, 0, 400, 400));
    assert_eq!(rects[&WindowId(2)], Rect::new(0, 400, 400, 400));
    assert_eq!(rects[&WindowId(3)], Rect::new(400, 0, 400, 400));
    assert_eq!(rects[&WindowId(4)], Rect::new(400, 400, 400, 400));
    assert_eq!(rects[&WindowId(5)], Rect::new(800, 0, 400, 800));

    // BottomStack: two masters side by side on the top row, stack below.
    let mut bottom_stack = LayoutTree::default();
    bottom_stack.apply_preset(Preset::BottomStack, &wins, 2);
    let rects = bottom_stack.bounds(layout);
    assert_eq!(rects[&WindowId(1)], Rect::new(0, 0, 600, 400));
    assert_eq!(rects[&WindowId(2)], Rect::new(600, 0, 600, 400));
    assert_eq!(rects[&WindowId(3)], Rect::new(0, 400, 400, 400));
    assert_eq!(rects[&WindowId(4)], Rect::new(400, 400, 400, 400));
    assert_eq!(rects[&WindowId(5)], Rect::new(800, 400, 400, 400));
}

#[test]
fn every_public_mutation_preserves_canonical_invariants() {
    let wins = windows(7);
    for preset in [Preset::MasterStack, Preset::Grid, Preset::BottomStack] {
        let mut tree = LayoutTree::default();
        tree.apply_preset(preset, &wins, 2);
        assert_canonical(&tree);
        tree.resize(WindowId(4), Side::Right);
        tree.resize_smart(WindowId(5), false);
        tree.resize_with_config(
            WindowId(3),
            Side::Left,
            CommandConfig {
                resize_step: f64::NAN,
                minimum_weight: f64::INFINITY,
            },
        );
        tree.swap_with_neighbor(WindowId(4), Side::Bottom);
        assert_canonical(&tree);
        let target = session_targets(&tree, WindowId(7), Rect::new(0, 0, 700, 500), 0.34)
            .into_iter()
            .find(|target| target.side.is_some());
        if let Some(target) = target {
            tree = placement_session(&tree, WindowId(7), Rect::new(0, 0, 700, 500), 0.34)
                .plan_target(target)
                .unwrap()
                .into_tree();
        }
        reconcile(
            &mut tree,
            &[WindowId(1), WindowId(3), WindowId(4), WindowId(8)],
        );
        assert_canonical(&tree);
        let leaves = tree.leaves().into_iter().collect::<HashSet<_>>();
        assert_eq!(
            leaves,
            [WindowId(1), WindowId(3), WindowId(4), WindowId(8)]
                .into_iter()
                .collect()
        );
    }
}

#[test]
fn verify_redistribute_bug_demo() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(3), 1);
    let before = tree.bounds(Rect::new(0, 0, 1000, 1000));
    let master_w_before = before[&WindowId(1)].w;
    eprintln!("master width before insert: {}", master_w_before);

    reconcile(
        &mut tree,
        &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
    );
    let after = tree.bounds(Rect::new(0, 0, 1000, 1000));
    let master_w_after = after[&WindowId(1)].w;
    eprintln!(
        "master width after insert: {} (bug: drops to ~250)",
        master_w_after
    );
    assert_canonical(&tree);
}

#[test]
fn reapplying_master_stack_preserves_ratio_from_the_tree() {
    let mut tree = LayoutTree::default();
    let area = Rect::new(0, 0, 1000, 600);
    tree.apply_preset(Preset::MasterStack, &windows(2), 1);
    assert!(tree.resize_by_pixels(WindowId(1), Side::Right, 120, area, 0.15));
    let before = tree.bounds(area)[&WindowId(1)].w;

    tree.apply_preset(Preset::MasterStack, &windows(3), 1);

    assert_eq!(tree.bounds(area)[&WindowId(1)].w, before);
}

#[test]
fn master_count_supports_every_value_through_the_window_count() {
    let area = Rect::new(0, 0, 900, 600);
    for count in 0..=3 {
        let mut tree = LayoutTree::default();
        tree.apply_preset(Preset::MasterStack, &windows(3), count);
        let bounds = tree.bounds(area);
        assert_eq!(bounds.len(), 3);
        if count == 0 || count == 3 {
            assert!(bounds.values().all(|rect| rect.w == area.w));
        }
        assert_canonical(&tree);
    }
}

#[test]
fn promote_force_inserts_then_swaps_without_changing_the_layout() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(3), 1);
    let work_rect = Rect::new(0, 0, 1000, 600);
    let minimums = HashMap::new();
    let candidate_order = windows(3);
    let slot_geometry = |bounds: &HashMap<WindowId, Rect>| {
        let mut slots = bounds
            .values()
            .map(|rect| (rect.x, rect.y, rect.w, rect.h))
            .collect::<Vec<_>>();
        slots.sort_unstable();
        slots
    };

    // Initial leaves: [WindowId(1), WindowId(2), WindowId(3)]
    assert_eq!(tree.leaves(), vec![WindowId(1), WindowId(2), WindowId(3)]);

    // A non-primary window is reinserted with force placement and gets the
    // left half of the work area.
    let promoted = tree.promote(WindowId(3), work_rect, &minimums, &candidate_order);
    assert_eq!(promoted, Some(WindowId(3)));
    assert_eq!(tree.leaves()[0], WindowId(3));
    let before_swap = tree.bounds(work_rect);
    assert_eq!(before_swap[&WindowId(3)], Rect::new(0, 0, 500, 600));

    // Once the desired force layout exists, promoting its primary only swaps
    // window identities. Every slot keeps exactly the same geometry.
    let candidate = tree.leaves()[1];
    let cycled_1 = tree.promote(WindowId(3), work_rect, &minimums, &candidate_order);
    assert_eq!(cycled_1, Some(candidate));
    assert_eq!(tree.leaves()[0], candidate);

    let after_swap = tree.bounds(work_rect);
    assert_eq!(after_swap[&candidate], before_swap[&WindowId(3)]);
    assert_eq!(after_swap[&WindowId(3)], before_swap[&candidate]);
    for window in tree
        .leaves()
        .into_iter()
        .filter(|window| *window != candidate && *window != WindowId(3))
    {
        assert_eq!(after_swap[&window], before_swap[&window]);
    }
    assert_canonical(&tree);

    // Candidate selection follows a stable order rather than the mutated leaf
    // order, so subsequent presses visit every window instead of toggling two.
    let cycled_2 = tree.promote(candidate, work_rect, &minimums, &candidate_order);
    assert_eq!(cycled_2, Some(WindowId(2)));
    assert_eq!(tree.leaves()[0], WindowId(2));
    let after_second_swap = tree.bounds(work_rect);
    assert_eq!(
        slot_geometry(&after_second_swap),
        slot_geometry(&before_swap)
    );

    let cycled_3 = tree.promote(WindowId(2), work_rect, &minimums, &candidate_order);
    assert_eq!(cycled_3, Some(WindowId(3)));
    assert_eq!(tree.leaves()[0], WindowId(3));
    let after_third_swap = tree.bounds(work_rect);
    assert_eq!(
        slot_geometry(&after_third_swap),
        slot_geometry(&before_swap)
    );
    assert_canonical(&tree);
}
