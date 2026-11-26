use super::*;

fn new_session(
    tree: &LayoutTree,
    source: WindowId,
    area: Rect,
    fraction: f64,
    minimums: &HashMap<WindowId, Size>,
) -> TreePlacementSession {
    TreePlacementSession::new(tree.clone(), source, area, fraction, minimums.clone())
}

#[test]
fn hit_testing_uses_the_slots_that_size_hints_actually_display() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let area = Rect::new(-100, 50, 300, 180);
    let minimums = HashMap::from([(windows[0], Size::new(180, 1))]);
    let mut tree = LayoutTree::default();
    tree.root = presets::equal_run(&windows, Axis::Vertical, &mut || tree.allocate());
    let target_slot = tree.soft_constrained_bounds(area, &minimums).0[&windows[1]];
    let point = target_slot.center();
    assert!(!tree.bounds(area)[&windows[1]].contains_point(point));

    let mut session = new_session(&tree, windows[0], area, 0.34, &minimums);
    assert!(session.preview_point(Point::new(70, point.y)).is_none());
    assert_eq!(
        session
            .hitboxes(area, None)
            .spans
            .iter()
            .find(|span| span.rect.contains_point(point))
            .unwrap()
            .target,
        windows[1]
    );
    let pointer = session.into_plan(point).unwrap();
    let keyboard = tree
        .placement_targets(windows[0], area, 0.34, &minimums)
        .into_iter()
        .find(|target| target.target == windows[1] && target.side.is_none())
        .unwrap();
    assert_eq!(keyboard.position, point);
    assert_eq!(pointer.target, keyboard);
}

#[test]
fn dense_stacks_offer_usable_scope_bands_in_order_of_extent() {
    let windows = (1..=17).map(WindowId).collect::<Vec<_>>();
    let area = Rect::new(0, 0, 420, 800);
    let source = windows[0];
    let target = windows[8];
    let mut tree = LayoutTree::default();
    tree.apply_preset(Preset::MasterStack, &windows, 1);
    let row = tree.bounds(area)[&target];
    let mut session = new_session(&tree, source, area, 0.34, &HashMap::new());
    let hitboxes = session.hitboxes(area, None);
    let spans = hitboxes
        .spans
        .iter()
        .filter(|span| {
            span.target == target && span.side == Some(Side::Left) && span.rect.y == row.center().y
        })
        .collect::<Vec<_>>();
    assert!(spans.len() >= 2);
    assert!(
        session.plans[&(target, Some(Side::Left))].len() > spans.len(),
        "the stack must exercise downsampling of broader scopes"
    );
    let mut previous_height = area.h;
    for span in &spans {
        assert!(span.rect.w >= MIN_SCOPE_BAND as i32);
        let height = session.preview_point(span.rect.center()).unwrap().h;
        assert!(height <= previous_height);
        previous_height = height;
    }
    let local = spans.last().unwrap().rect;
    assert!(f64::from(local.w) >= (edge_depth(row.w, 0.34) / 2.0).floor());
    assert_eq!(session.preview_point(local.center()).unwrap().h, row.h);
}

#[test]
fn narrow_targets_keep_a_center_and_mirror_opposing_edge_bands() {
    for w in 1..=80 {
        for h in [1, 2, 3, 4, 17, 45] {
            let rect = Rect::new(-400, 25, w, h);
            let edges = HitboxEdges::new(rect, 0.49);
            assert_eq!(edges.side_at(rect.center()), None);
            for x in 0..w {
                let side_at = |x| {
                    edges
                        .side_at(Point::new(rect.x + x, rect.center().y))
                        .map(|(index, _)| SIDES[index])
                };
                assert_eq!(
                    side_at(x),
                    side_at(w - x - 1).map(|side| match side {
                        Side::Left => Side::Right,
                        Side::Right => Side::Left,
                        _ => unreachable!("middle of the target cannot be in a vertical edge band"),
                    })
                );
            }
        }
    }
}

fn next(random: &mut u32, below: u32) -> u32 {
    *random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*random >> 8) % below
}

fn random_node(tree: &mut LayoutTree, windows: &[WindowId], random: &mut u32) -> Node {
    if windows.len() == 1 {
        return Node::Window(windows[0]);
    }
    let axis = if next(random, 2) == 0 {
        Axis::Vertical
    } else {
        Axis::Horizontal
    };
    let mut windows = windows;
    let mut children = Vec::new();
    while !windows.is_empty() {
        let count = 1 + next(random, (windows.len() / 2).max(1) as u32) as usize;
        let (chunk, rest) = windows.split_at(count);
        children.push(WeightedNode {
            node: random_node(tree, chunk, random),
            weight: f64::from(1 + next(random, 70)),
        });
        windows = rest;
    }
    make_split(tree.allocate(), axis, children).unwrap()
}

#[test]
fn seeded_layouts_match_every_drawn_pixel_to_the_drop_resolver() {
    for seed in 0..48 {
        let mut random = seed;
        let windows = (1..=2 + next(&mut random, 12))
            .map(WindowId)
            .collect::<Vec<_>>();
        let mut tree = LayoutTree::default();
        tree.root = Some(random_node(&mut tree, &windows, &mut random));
        let source = if seed % 3 == 0 {
            WindowId(50)
        } else {
            windows[next(&mut random, windows.len() as u32) as usize]
        };
        let area = Rect::new(
            -120,
            350,
            2 + next(&mut random, 120) as i32,
            2 + next(&mut random, 100) as i32,
        );
        let minimums = windows
            .iter()
            .map(|&window| {
                (
                    window,
                    Size::new(
                        1 + next(&mut random, 50) as i32,
                        1 + next(&mut random, 35) as i32,
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let fraction = [0.05, 0.34, 0.49][seed as usize % 3];
        let mut session = new_session(&tree, source, area, fraction, &minimums);
        let output = Rect::new(area.x - 3, area.y - 5, area.w + 9, area.h + 11);
        let bar = Rect::new(output.x, output.y + 1, output.w, 3);
        let hitboxes = session.hitboxes(output, Some(bar));
        assert!(Arc::ptr_eq(&hitboxes, &session.hitboxes(output, Some(bar))));
        let offset = |x, y| ((y - output.y) * output.w + x - output.x) as usize;
        let mut drawn = vec![None; output.area() as usize];
        for span in &hitboxes.spans {
            assert!(output.contains_rect(&span.rect));
            for y in span.rect.y..span.rect.bottom() {
                for x in span.rect.x..span.rect.right() {
                    assert!(
                        drawn[offset(x, y)].replace(span).is_none(),
                        "seed {seed} contains overlapping regions"
                    );
                }
            }
        }

        let bounds = session.bounds().iter().copied().collect::<HashMap<_, _>>();
        let mut sampled = HashSet::new();
        for y in output.y..output.bottom() {
            for x in output.x..output.right() {
                let point = Point::new(x, y);
                let draw = drawn[offset(x, y)];
                let resolved = if bar.contains_point(point) {
                    None
                } else {
                    session.resolve(session.layout_point(point))
                };
                assert_eq!(draw.is_some(), resolved.is_some(), "seed {seed}, {point:?}");
                let (Some(span), Some((key, index))) = (draw, resolved) else {
                    continue;
                };
                let side = span.side.or_else(|| session.center_side());
                assert_eq!(key, (span.target, side), "seed {seed}, {point:?}");
                let plans = &session.plans[&key];
                let displayed_index = span.side.map_or(plans.len() - 1, |side| {
                    let edges = HitboxEdges::new(bounds[&span.target], fraction);
                    EdgeBands::new(edges.depth(side), plans.len()).plan_index(span.band)
                });
                assert_eq!(displayed_index, index, "seed {seed}, {point:?}");

                if sampled.insert((key, index)) {
                    let preview = session.preview_point(point).unwrap();
                    let plan = new_session(&tree, source, area, fraction, &minimums)
                        .into_plan(point)
                        .expect("a fresh mouse-up must resolve the same region");
                    assert_eq!(plan.source_slot(), preview);
                    let applied = plan.into_tree();
                    let applied_bounds = applied.soft_constrained_bounds(area, &minimums).0;
                    assert!(area.contains_rect(&preview));
                    assert_eq!(applied_bounds[&source], preview);
                    assert_eq!(
                        applied.len(),
                        tree.len() + usize::from(!tree.leaves().contains(&source))
                    );
                }
            }
        }

        if tree.leaves().contains(&source) {
            let targets = tree.placement_targets(source, area, fraction, &minimums);
            for target in targets {
                assert!(
                    bounds[&target.target].contains_point(target.position),
                    "seed {seed}"
                );
                let plan = tree
                    .plan_placement(source, target, area, &minimums)
                    .unwrap();
                assert!(area.contains_rect(&plan.source_slot()));
            }
        }
    }
}
