//! Pure packing algorithms for dynamic troupes (no OS threads).
//!
//! Phase 1: even-split round-robin into `slots` clusters.
//! Phase 3: Kruskal on smallest-capacity incidence edges (+ bundle-sibling rule).

// ss[impl troupe.dynamic-no-incidence-fallback]
// ss[impl troupe.dynamic-kruskal-capacity]
// ss[impl troupe.dynamic-bundle-siblings]
// ss[impl philosophy.startup-schedule]

/// One directed packing edge between packable actors (build-time incidence).
#[derive(Clone, Debug, PartialEq, Eq)]
// ss[impl graph.pack.incidence-before-start]
pub struct PackEdge {
    /// Actor id that holds the TX end.
    pub from: usize,
    /// Actor id that holds the RX end.
    pub to: usize,
    /// Channel capacity (items); smaller merges first.
    pub capacity: usize,
    /// `capacity × ring_slot_byte_count` (tie-break after capacity).
    pub byte_footprint: usize,
    /// When set, consumers sharing the same producer + family are bundle siblings.
    pub bundle_family: Option<BundleFamily>,
}

/// Identifies a bundle fan-out so parallel consumers are not glued onto one core early.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// ss[impl troupe.dynamic-bundle-siblings]
pub struct BundleFamily {
    /// Producer actor id.
    pub producer: usize,
    /// Channel / partner identity for the bundle (e.g. channel id of lane 0, or partner key).
    pub family_key: usize,
}

/// Round-robin partition of actor indices into `slots` non-empty clusters when possible.
///
/// Used when incidence is missing (`troupe.dynamic-no-incidence-fallback`).
// ss[impl troupe.dynamic-no-incidence-fallback]
pub fn even_split(actor_ids: &[usize], slots: usize) -> Vec<Vec<usize>> {
    let slots = slots.max(1);
    if actor_ids.is_empty() {
        return Vec::new();
    }
    let n_clusters = slots.min(actor_ids.len());
    let mut clusters: Vec<Vec<usize>> = (0..n_clusters).map(|_| Vec::new()).collect();
    for (i, &id) in actor_ids.iter().enumerate() {
        clusters[i % n_clusters].push(id);
    }
    clusters.retain(|c| !c.is_empty());
    clusters
}

/// Kruskal-style clustering: merge along smallest capacity edges until `slots` clusters remain.
///
/// Bundle siblings (same `BundleFamily`) are not merged while `cluster_count > slots`.
/// Actors with conflicting `explicit_cores` are not merged (`troupe.dynamic-explicit-core`).
/// Each returned cluster is ordered producer→consumer along incidence (`troupe.dynamic-linedance-order`).
// ss[impl troupe.dynamic-kruskal-capacity]
// ss[impl troupe.dynamic-bundle-siblings]
// ss[impl troupe.dynamic-linedance-order]
// ss[impl troupe.dynamic-explicit-core]
pub fn pack_kruskal(
    actor_ids: &[usize],
    edges: &[PackEdge],
    slots: usize,
) -> Vec<Vec<usize>> {
    pack_kruskal_with_cores(actor_ids, edges, slots, &std::collections::HashMap::new())
}

/// Kruskal packing with optional per-actor explicit core ids (zero-based).
// ss[impl troupe.dynamic-explicit-core]
pub fn pack_kruskal_with_cores(
    actor_ids: &[usize],
    edges: &[PackEdge],
    slots: usize,
    explicit_cores: &std::collections::HashMap<usize, usize>,
) -> Vec<Vec<usize>> {
    let slots = slots.max(1);
    if actor_ids.is_empty() {
        return Vec::new();
    }
    if edges.is_empty() {
        return even_split(actor_ids, slots);
    }

    let mut parent: std::collections::HashMap<usize, usize> =
        actor_ids.iter().map(|&id| (id, id)).collect();
    let mut rank: std::collections::HashMap<usize, u8> =
        actor_ids.iter().map(|&id| (id, 0u8)).collect();

    fn find(parent: &mut std::collections::HashMap<usize, usize>, x: usize) -> usize {
        let p = parent[&x];
        if p != x {
            let root = find(parent, p);
            parent.insert(x, root);
            root
        } else {
            x
        }
    }
    fn union(
        parent: &mut std::collections::HashMap<usize, usize>,
        rank: &mut std::collections::HashMap<usize, u8>,
        a: usize,
        b: usize,
    ) -> bool {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra == rb {
            return false;
        }
        let ra_rank = rank[&ra];
        let rb_rank = rank[&rb];
        if ra_rank < rb_rank {
            parent.insert(ra, rb);
        } else if ra_rank > rb_rank {
            parent.insert(rb, ra);
        } else {
            parent.insert(rb, ra);
            rank.insert(ra, ra_rank + 1);
        }
        true
    }

    let mut cluster_count = actor_ids.len();
    let mut sorted = edges.to_vec();
    sorted.sort_by(|a, b| {
        a.capacity
            .cmp(&b.capacity)
            .then(a.byte_footprint.cmp(&b.byte_footprint))
            .then(a.from.cmp(&b.from))
            .then(a.to.cmp(&b.to))
    });

    for e in &sorted {
        if cluster_count <= slots {
            break;
        }
        if !parent.contains_key(&e.from) || !parent.contains_key(&e.to) {
            continue;
        }
        // Explicit-core conflict: do not merge onto one thread.
        // ss[impl troupe.dynamic-explicit-core]
        if let (Some(&ca), Some(&cb)) = (
            explicit_cores.get(&e.from),
            explicit_cores.get(&e.to),
        ) {
            if ca != cb {
                continue;
            }
        }
        // Bundle-sibling rule
        if let Some(fam) = e.bundle_family {
            if e.from != fam.producer && e.to != fam.producer {
                if cluster_count > slots {
                    continue;
                }
            }
            if would_merge_bundle_siblings(&parent, &find, actor_ids, edges, e.from, e.to, fam)
                && cluster_count > slots
            {
                continue;
            }
        }
        if union(&mut parent, &mut rank, e.from, e.to) {
            cluster_count -= 1;
        }
    }

    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for &id in actor_ids {
        let root = find(&mut parent, id);
        groups.entry(root).or_default().push(id);
    }
    let mut clusters: Vec<Vec<usize>> = groups.into_values().collect();
    if clusters.len() > slots {
        let flat: Vec<usize> = clusters.into_iter().flatten().collect();
        return even_split(&flat, slots);
    }
    for c in &mut clusters {
        linedance_order(c, edges);
    }
    clusters
}

/// Order actors in a cluster along incidence (producers before consumers).
// ss[impl troupe.dynamic-linedance-order]
pub fn linedance_order(cluster: &mut Vec<usize>, edges: &[PackEdge]) {
    if cluster.len() <= 1 {
        return;
    }
    let set: std::collections::HashSet<usize> = cluster.iter().copied().collect();
    let mut indeg: std::collections::HashMap<usize, usize> =
        cluster.iter().map(|&id| (id, 0usize)).collect();
    let mut outs: std::collections::HashMap<usize, Vec<usize>> =
        cluster.iter().map(|&id| (id, Vec::new())).collect();
    for e in edges {
        if set.contains(&e.from) && set.contains(&e.to) && e.from != e.to {
            outs.get_mut(&e.from).unwrap().push(e.to);
            *indeg.get_mut(&e.to).unwrap() += 1;
        }
    }
    let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<usize>> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(&id, _)| std::cmp::Reverse(id))
        .collect();
    let mut ordered = Vec::with_capacity(cluster.len());
    while let Some(std::cmp::Reverse(id)) = ready.pop() {
        ordered.push(id);
        for &to in outs.get(&id).into_iter().flatten() {
            let d = indeg.get_mut(&to).unwrap();
            *d -= 1;
            if *d == 0 {
                ready.push(std::cmp::Reverse(to));
            }
        }
    }
    if ordered.len() == cluster.len() {
        *cluster = ordered;
    }
}

fn would_merge_bundle_siblings(
    parent: &std::collections::HashMap<usize, usize>,
    find: &dyn Fn(&mut std::collections::HashMap<usize, usize>, usize) -> usize,
    _actor_ids: &[usize],
    edges: &[PackEdge],
    a: usize,
    b: usize,
    fam: BundleFamily,
) -> bool {
    // Collect consumers of this family currently in each side's cluster.
    let mut parent_mut = parent.clone();
    let ra = find(&mut parent_mut, a);
    let rb = find(&mut parent_mut, b);
    if ra == rb {
        return false;
    }
    let consumers: Vec<usize> = edges
        .iter()
        .filter(|e| e.bundle_family == Some(fam) && e.from == fam.producer)
        .map(|e| e.to)
        .collect();
    let a_has = consumers.iter().any(|&c| {
        let mut p = parent.clone();
        find(&mut p, c) == ra
    });
    let b_has = consumers.iter().any(|&c| {
        let mut p = parent.clone();
        find(&mut p, c) == rb
    });
    a_has && b_has
}

/// Choose packing strategy: Kruskal when edges exist, else even-split.
// ss[impl troupe.dynamic-no-incidence-fallback]
pub fn pack_actors(
    actor_ids: &[usize],
    edges: &[PackEdge],
    slots: usize,
) -> Vec<Vec<usize>> {
    if edges.is_empty() {
        even_split(actor_ids, slots)
    } else {
        pack_kruskal(actor_ids, edges, slots)
    }
}

#[cfg(test)]
#[path = "pack_proptest.rs"]
// ss[related troupe.dynamic-kruskal-capacity]
mod pack_proptest;

#[cfg(test)]
#[path = "pack_integration.rs"]
// ss[related troupe.dynamic-finalize-at-start]
mod pack_integration;

#[cfg(test)]
// ss[related troupe.dynamic-no-incidence-fallback]
mod pack_unit {
    // ss[related philosophy.structural-hierarchy]
    use super::*;

    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    fn even_split_four_into_two() {
        let c = even_split(&[10, 20, 30, 40], 2);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0], vec![10, 30]);
        assert_eq!(c[1], vec![20, 40]);
    }

    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    fn even_split_slots_gt_actors() {
        let c = even_split(&[1, 2], 8);
        assert_eq!(c.len(), 2);
    }

    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    fn kruskal_merges_small_edge_first() {
        // A --2--> B --4096--> C, slots=2 → {A,B} and {C}
        let edges = vec![
            PackEdge {
                from: 0,
                to: 1,
                capacity: 2,
                byte_footprint: 16,
                bundle_family: None,
            },
            PackEdge {
                from: 1,
                to: 2,
                capacity: 4096,
                byte_footprint: 32768,
                bundle_family: None,
            },
        ];
        let clusters = pack_kruskal(&[0, 1, 2], &edges, 2);
        assert_eq!(clusters.len(), 2);
        let mut ab_together = false;
        for c in &clusters {
            if c.contains(&0) && c.contains(&1) {
                ab_together = true;
                assert!(!c.contains(&2));
            }
        }
        assert!(ab_together, "A and B should share a cluster: {clusters:?}");
    }

    #[test]
    // ss[verify troupe.dynamic-bundle-siblings]
    fn bundle_siblings_not_merged_when_slots_equal_girth() {
        // Producer 0 → consumers 1,2,3 (girth 3), slots=3 → three clusters
        let fam = BundleFamily {
            producer: 0,
            family_key: 99,
        };
        let edges = vec![
            PackEdge {
                from: 0,
                to: 1,
                capacity: 8,
                byte_footprint: 64,
                bundle_family: Some(fam),
            },
            PackEdge {
                from: 0,
                to: 2,
                capacity: 8,
                byte_footprint: 64,
                bundle_family: Some(fam),
            },
            PackEdge {
                from: 0,
                to: 3,
                capacity: 8,
                byte_footprint: 64,
                bundle_family: Some(fam),
            },
        ];
        let clusters = pack_kruskal(&[0, 1, 2, 3], &edges, 3);
        // With 4 actors and 3 slots we must merge once; merging producer with one
        // consumer is OK; merging two consumers is not.
        assert!(clusters.len() <= 3);
        for c in &clusters {
            let consumers_in: Vec<_> = c.iter().filter(|&&id| id != 0).copied().collect();
            assert!(
                consumers_in.len() <= 1,
                "bundle consumers must not share a cluster while slots allow: {clusters:?}"
            );
        }
    }

    #[test]
    // ss[verify troupe.dynamic-linedance-order]
    fn linedance_orders_producer_before_consumer() {
        let edges = vec![PackEdge {
            from: 1,
            to: 2,
            capacity: 1,
            byte_footprint: 8,
            bundle_family: None,
        }];
        let mut cluster = vec![2, 1];
        linedance_order(&mut cluster, &edges);
        assert_eq!(cluster, vec![1, 2]);
    }

    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    fn even_split_empty_and_single() {
        assert!(even_split(&[], 4).is_empty());
        assert_eq!(even_split(&[7], 8), vec![vec![7]]);
    }

    #[test]
    // ss[verify troupe.dynamic-linedance-order]
    fn linedance_noop_on_singleton_or_cycle() {
        let mut one = vec![9usize];
        linedance_order(&mut one, &[]);
        assert_eq!(one, vec![9]);

        // Cycle: ordering may leave original order when topo sort fails.
        let edges = vec![
            PackEdge {
                from: 1,
                to: 2,
                capacity: 1,
                byte_footprint: 8,
                bundle_family: None,
            },
            PackEdge {
                from: 2,
                to: 1,
                capacity: 1,
                byte_footprint: 8,
                bundle_family: None,
            },
        ];
        let mut cluster = vec![1, 2];
        linedance_order(&mut cluster, &edges);
        assert_eq!(cluster.len(), 2);
    }

    #[test]
    // ss[verify troupe.dynamic-explicit-core]
    fn explicit_core_conflict_blocks_merge() {
        let edges = vec![PackEdge {
            from: 0,
            to: 1,
            capacity: 1,
            byte_footprint: 8,
            bundle_family: None,
        }];
        let mut cores = std::collections::HashMap::new();
        cores.insert(0, 0usize);
        cores.insert(1, 1usize);
        // With enough slots, conflict keeps actors on distinct clusters.
        let clusters2 = pack_kruskal_with_cores(&[0, 1], &edges, 2, &cores);
        assert_eq!(clusters2.len(), 2);
        // Without conflict, slots=1 merges them.
        let merged = pack_kruskal_with_cores(
            &[0, 1],
            &edges,
            1,
            &std::collections::HashMap::new(),
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].len(), 2);
    }

    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify troupe.dynamic-linedance-order]
    fn kruskal_cluster_is_linedance_ordered() {
        let edges = vec![
            PackEdge {
                from: 0,
                to: 1,
                capacity: 2,
                byte_footprint: 16,
                bundle_family: None,
            },
            PackEdge {
                from: 1,
                to: 2,
                capacity: 2,
                byte_footprint: 16,
                bundle_family: None,
            },
        ];
        let clusters = pack_kruskal(&[0, 1, 2], &edges, 1);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0], vec![0, 1, 2], "producer→consumer linedance order");
    }
}
