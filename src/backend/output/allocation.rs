//! Deterministic maximum matching for scarce display resources.

/// Assign as many heads as possible, preserving valid existing assignments
/// unless moving one is necessary to accommodate another head. Input order
/// breaks ties; callers should supply resources in stable backend order.
pub fn assign_output_resources<T: Copy + Eq>(
    candidates: &[Vec<T>],
    preferred: &[Option<T>],
) -> Vec<Option<T>> {
    assert_eq!(candidates.len(), preferred.len());
    let mut assigned = vec![None; candidates.len()];
    for (index, preference) in preferred.iter().copied().enumerate() {
        if let Some(resource) = preference
            && candidates[index].contains(&resource)
            && !assigned.contains(&Some(resource))
        {
            assigned[index] = Some(resource);
        }
    }

    fn augment<T: Copy + Eq>(
        head: usize,
        candidates: &[Vec<T>],
        assigned: &mut [Option<T>],
        visited: &mut Vec<T>,
    ) -> bool {
        // Prefer a free resource before disturbing another output.
        for &resource in &candidates[head] {
            if !visited.contains(&resource) && !assigned.contains(&Some(resource)) {
                assigned[head] = Some(resource);
                return true;
            }
        }
        for &resource in &candidates[head] {
            if visited.contains(&resource) {
                continue;
            }
            visited.push(resource);
            if let Some(owner) = assigned.iter().position(|slot| *slot == Some(resource))
                && augment(owner, candidates, assigned, visited)
            {
                assigned[head] = Some(resource);
                return true;
            }
        }
        false
    }

    for head in 0..candidates.len() {
        if assigned[head].is_none() {
            augment(head, candidates, &mut assigned, &mut Vec::new());
        }
    }
    assigned
}

#[cfg(test)]
mod tests {
    use super::assign_output_resources as assign;

    #[test]
    fn late_constrained_head_can_move_an_existing_head() {
        assert_eq!(
            assign(&[vec![1, 2], vec![1]], &[Some(1), None]),
            vec![Some(2), Some(1)]
        );
    }

    #[test]
    fn working_assignments_survive_when_a_free_resource_exists() {
        assert_eq!(
            assign(&[vec![1, 2], vec![1, 2, 3]], &[Some(2), None]),
            vec![Some(2), Some(1)]
        );
    }

    #[test]
    fn shortage_keeps_working_heads_and_admits_other_independent_heads() {
        assert_eq!(
            assign(&[vec![1], vec![1], vec![2]], &[Some(1), None, None]),
            vec![Some(1), None, Some(2)]
        );
    }

    #[test]
    fn augmenting_path_can_move_multiple_heads() {
        assert_eq!(
            assign(
                &[vec![1, 2], vec![2, 3], vec![1]],
                &[Some(1), Some(2), None]
            ),
            vec![Some(2), Some(3), Some(1)]
        );
    }
    #[test]
    fn exhaustive_small_topologies_match_the_optimum() {
        // Independent brute-force oracle: all four-head/three-resource graphs,
        // both from scratch and with an existing assignment on the first head.
        fn optimum(candidates: &[Vec<u8>], used: u8) -> usize {
            let Some((head, rest)) = candidates.split_first() else {
                return 0;
            };
            let mut best = optimum(rest, used);
            for &resource in head {
                if used & (1 << resource) == 0 {
                    best = best.max(1 + optimum(rest, used | (1 << resource)));
                }
            }
            best
        }
        for graph in 0u16..4096 {
            let candidates: Vec<Vec<u8>> = (0..4)
                .map(|head| {
                    (0..3)
                        .filter(|resource| graph & (1 << (head * 3 + resource)) != 0)
                        .collect()
                })
                .collect();
            for preferred in [vec![None; 4], vec![Some(0), None, None, None]] {
                let result = assign(&candidates, &preferred);
                let mut used = Vec::new();
                for (head, resource) in result.iter().enumerate() {
                    if let Some(resource) = resource {
                        assert!(candidates[head].contains(resource));
                        assert!(!used.contains(resource));
                        used.push(*resource);
                    }
                }
                assert_eq!(
                    used.len(),
                    optimum(&candidates, 0),
                    "graph {graph}, {preferred:?}"
                );
            }
        }
    }
}
