use super::*;

fn windows(count: u32) -> Vec<WindowId> {
    (1..=count).map(WindowId).collect()
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
fn reconciliation_retains_surviving_topology_and_collapses_parents() {
    let mut tree = LayoutTree::default();
    tree.reconcile(&windows(4));
    let before = tree.bounds(Rect::new(0, 0, 100, 100));
    assert_eq!(tree.len(), 4);
    tree.reconcile(&[WindowId(1), WindowId(3), WindowId(4)]);
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
fn grid_is_a_persistent_tree_transformation() {
    let mut tree = LayoutTree::default();
    let wins = windows(4);
    tree.apply_preset(Preset::Grid, &wins, None, 1, 0.55);
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
    tree.apply_preset(Preset::Grid, &wins, None, 1, 0.55);
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
    assert!(tree.place_at_point(
        WindowId(1),
        Point::new(150, 50),
        Rect::new(0, 0, 300, 100),
        0.3
    ));
    assert_eq!(tree.leaves(), vec![WindowId(2), WindowId(1), WindowId(3)]);
    assert!(tree.place_at_point(
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
    assert!(tree.place_at_point(
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
fn keyboard_and_pointer_targets_apply_the_same_semantic_candidate() {
    let mut original = LayoutTree::default();
    original.root = equal_run(&windows(3), Axis::Vertical, &mut || original.allocate());
    let rect = Rect::new(0, 0, 300, 100);
    let target = original
        .placement_targets(WindowId(1), rect, 0.34)
        .into_iter()
        .find(|target| target.target == WindowId(2) && target.side == Some(Side::Right))
        .unwrap();

    let mut keyboard = original.clone();
    let mut pointer = original;
    assert!(keyboard.apply_placement_target(WindowId(1), target));
    assert!(pointer.place_at_point(WindowId(1), target.position, rect, 0.34));
    assert_eq!(keyboard.bounds(rect), pointer.bounds(rect));
}

#[test]
fn placement_preview_is_exact_and_does_not_mutate_the_tree() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(4), None, 1, 0.55);
    let rect = Rect::new(0, 0, 400, 300);
    let before = tree.bounds(rect);
    let target = tree
        .placement_targets(WindowId(1), rect, 0.34)
        .into_iter()
        .find(|target| target.target == WindowId(4) && target.side == Some(Side::Left))
        .unwrap();

    let preview = tree
        .preview_placement_target(WindowId(1), target, rect)
        .unwrap();
    assert_eq!(tree.bounds(rect), before);

    let mut applied = tree.clone();
    assert!(applied.apply_placement_target(WindowId(1), target));
    assert_eq!(applied.bounds(rect)[&WindowId(1)], preview);
}

#[test]
fn pointer_placement_preview_matches_release_and_does_not_mutate() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::Grid, &windows(6), None, 1, 0.55);
    let rect = Rect::new(20, 30, 600, 400);
    let before = tree.bounds(rect);
    let points = tree
        .placement_targets(WindowId(1), rect, 0.34)
        .into_iter()
        .filter(|target| target.target == WindowId(5))
        .map(|target| target.position)
        .collect::<Vec<_>>();
    assert!(points.len() > 1, "exercise both centre and edge targets");

    for point in points {
        let preview = tree
            .preview_placement_at_point(WindowId(1), point, rect, 0.34)
            .expect("advertised placement point must be valid");
        assert_eq!(tree.bounds(rect), before);

        let mut applied = tree.clone();
        assert!(applied.place_at_point(WindowId(1), point, rect, 0.34));
        assert_eq!(applied.bounds(rect)[&WindowId(1)], preview);
    }
}

#[test]
fn pointer_resize_tracks_the_grabbed_edge_in_pixels() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(2), None, 1, 0.5);
    let layout = Rect::new(0, 0, 1000, 600);
    let before = tree.bounds(layout)[&WindowId(1)];

    assert!(tree.resize_by_pixels(WindowId(1), Side::Right, 120, layout, 0.15));
    let after = tree.bounds(layout)[&WindowId(1)];

    assert_eq!(after.w - before.w, 120);
    assert_canonical(&tree);
}

#[test]
fn leading_edge_motion_has_the_opposite_weight_sign() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(2), None, 1, 0.5);
    let layout = Rect::new(0, 0, 1000, 600);
    let before = tree.bounds(layout)[&WindowId(1)];

    assert!(tree.resize_by_pixels(WindowId(1), Side::Left, 100, layout, 0.15));
    let after = tree.bounds(layout)[&WindowId(1)];

    assert_eq!(before.w - after.w, 100);
}

#[test]
fn resize_axis_reports_only_structural_runs() {
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows(2), None, 1, 0.5);

    assert!(tree.can_resize_axis(WindowId(1), Axis::Vertical));
    assert!(!tree.can_resize_axis(WindowId(1), Axis::Horizontal));
}

#[test]
fn every_public_mutation_preserves_canonical_invariants() {
    let wins = windows(7);
    for preset in [
        Preset::MasterStack,
        Preset::Grid,
        Preset::HorizontalGrid,
        Preset::BottomStack,
        Preset::BottomStackHorizontal,
        Preset::Focus,
    ] {
        let mut tree = LayoutTree::default();
        tree.apply_preset(preset, &wins, Some(WindowId(4)), 2, 0.6);
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
        let target = tree
            .placement_targets(WindowId(7), Rect::new(0, 0, 700, 500), 0.34)
            .into_iter()
            .find(|target| target.side.is_some());
        if let Some(target) = target {
            assert!(tree.apply_placement_target(WindowId(7), target));
        }
        tree.reconcile(&[WindowId(1), WindowId(3), WindowId(4), WindowId(8)]);
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
