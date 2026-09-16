//! Property tests for pure packing (no OS threads).

// ss[related troupe.dynamic-kruskal-capacity]
use super::{
    even_split, pack_actors, pack_kruskal, pack_kruskal_with_cores, BundleFamily, PackEdge,
};
use std::collections::HashMap;
// ss[related philosophy.structural-hierarchy]
// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;

ss_proptest! {
    /// Property: even_split never exceeds slots and covers every actor exactly once.
    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    // ss[verify verify.process.proptest]
    fn proptest_even_split_partition(
        ids in prop::collection::vec(0usize..100, 0..40),
        slots in 1usize..16,
    ) {
        let mut unique = ids;
        unique.sort_unstable();
        unique.dedup();
        let clusters = even_split(&unique, slots);
        if unique.is_empty() {
            prop_assert!(clusters.is_empty());
            return Ok(());
        }
        prop_assert!(clusters.len() <= slots.min(unique.len()));
        let mut flat: Vec<usize> = clusters.into_iter().flatten().collect();
        flat.sort_unstable();
        prop_assert_eq!(flat, unique);
    }

    /// Property: pack_actors with no edges equals even_split; cluster count ≤ slots.
    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    // ss[verify verify.process.proptest]
    fn proptest_pack_empty_edges_is_even_split(
        n in 1usize..24,
        slots in 1usize..12,
    ) {
        let ids: Vec<usize> = (0..n).collect();
        let a = pack_actors(&ids, &[], slots);
        let b = even_split(&ids, slots);
        prop_assert_eq!(&a, &b);
        prop_assert!(a.len() <= slots.min(n));
    }

    /// Property: Kruskal cluster count ≤ slots and covers all actors.
    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_covers_and_bounds_slots(
        n in 2usize..16,
        slots in 1usize..8,
        seed_cap in 1usize..64,
    ) {
        let ids: Vec<usize> = (0..n).collect();
        // Chain 0→1→…→n-1 with increasing capacity
        let edges: Vec<PackEdge> = (0..n - 1)
            .map(|i| PackEdge {
                from: i,
                to: i + 1,
                capacity: seed_cap + i * 10,
                byte_footprint: (seed_cap + i * 10) * 8,
                bundle_family: None,
            })
            .collect();
        let clusters = pack_kruskal(&ids, &edges, slots);
        prop_assert!(clusters.len() <= slots.min(n));
        let mut flat: Vec<usize> = clusters.into_iter().flatten().collect();
        flat.sort_unstable();
        prop_assert_eq!(flat, ids);
    }

    /// Property: girth-G bundle with slots=G keeps consumers in distinct clusters.
    #[test]
    // ss[verify troupe.dynamic-bundle-siblings]
    // ss[verify verify.process.proptest]
    fn proptest_bundle_siblings_separated(
        girth in 2usize..8,
    ) {
        let fam = BundleFamily {
            producer: 0,
            family_key: 1,
        };
        let mut ids = vec![0usize];
        let mut edges = Vec::new();
        for i in 1..=girth {
            ids.push(i);
            edges.push(PackEdge {
                from: 0,
                to: i,
                capacity: 4,
                byte_footprint: 32,
                bundle_family: Some(fam),
            });
        }
        let clusters = pack_kruskal(&ids, &edges, girth);
        prop_assert!(clusters.len() <= girth);
        for c in &clusters {
            let consumers = c.iter().filter(|&&id| id != 0).count();
            prop_assert!(
                consumers <= 1,
                "consumers glued together: {:?}",
                clusters
            );
        }
    }

    /// Property: explicit core conflict keeps actors in separate clusters when slots allow.
    #[test]
    // ss[verify troupe.dynamic-explicit-core]
    // ss[verify verify.process.proptest]
    fn proptest_explicit_core_conflict_blocks_merge(
        cap in 1usize..32,
    ) {
        let edges = vec![PackEdge {
            from: 0,
            to: 1,
            capacity: cap,
            byte_footprint: cap * 8,
            bundle_family: None,
        }];
        let mut cores = HashMap::new();
        cores.insert(0, 0usize);
        cores.insert(1, 1usize);
        let clusters = pack_kruskal_with_cores(&[0, 1], &edges, 2, &cores);
        prop_assert_eq!(clusters.len(), 2);
        let merged = pack_kruskal_with_cores(
            &[0, 1],
            &edges,
            1,
            &HashMap::new(),
        );
        prop_assert_eq!(merged.len(), 1);
    }

    /// Property: when clusters exceed slots, result is re-partitioned to ≤ slots.
    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_overflow_even_splits(
        n in 4usize..12,
        slots in 1usize..3,
    ) {
        let ids: Vec<usize> = (0..n).collect();
        let edges: Vec<PackEdge> = ids
            .iter()
            .zip(ids.iter().skip(1))
            .map(|(&from, &to)| PackEdge {
                from,
                to,
                capacity: 1,
                byte_footprint: 8,
                bundle_family: None,
            })
            .collect();
        let clusters = pack_kruskal(&ids, &edges, slots);
        prop_assert!(clusters.len() <= slots.min(n));
        let mut flat: Vec<usize> = clusters.into_iter().flatten().collect();
        flat.sort_unstable();
        prop_assert_eq!(flat, ids);
    }

    /// Property: empty actor list yields empty clusters for with_cores path.
    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_with_cores_empty_ids(
        slots in 1usize..8,
    ) {
        let clusters = pack_kruskal_with_cores(&[], &[], slots, &HashMap::new());
        prop_assert!(clusters.is_empty());
    }

    /// Property: with_cores and no edges uses even_split fallback.
    #[test]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_with_cores_no_edges_even_split(
        n in 1usize..12,
        slots in 1usize..6,
    ) {
        let ids: Vec<usize> = (0..n).collect();
        let a = pack_kruskal_with_cores(&ids, &[], slots, &HashMap::new());
        let b = even_split(&ids, slots);
        prop_assert_eq!(&a, &b);
    }

    /// Property: edges whose endpoints are not in `actor_ids` are ignored.
    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_dangling_endpoints_ignored(
        n in 2usize..10,
        slots in 1usize..4,
        dangling_from in 100usize..200,
        dangling_to in 200usize..300,
    ) {
        let ids: Vec<usize> = (0..n).collect();
        let edges = vec![
            PackEdge {
                from: dangling_from,
                to: dangling_to,
                capacity: 1,
                byte_footprint: 8,
                bundle_family: None,
            },
            PackEdge {
                from: 0,
                to: 1,
                capacity: 2,
                byte_footprint: 16,
                bundle_family: None,
            },
        ];
        let clusters = pack_kruskal(&ids, &edges, slots);
        prop_assert!(clusters.len() <= slots.min(n));
        let mut flat: Vec<usize> = clusters.into_iter().flatten().collect();
        flat.sort_unstable();
        prop_assert_eq!(flat, ids);
    }

    /// Property: equal capacity prefers the smaller byte_footprint when merging first.
    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_footprint_tie_break(
        small_fp in 8usize..64,
        extra in 1usize..64,
    ) {
        let large_fp = small_fp + extra;
        let edges = vec![
            PackEdge {
                from: 1,
                to: 2,
                capacity: 4,
                byte_footprint: large_fp,
                bundle_family: None,
            },
            PackEdge {
                from: 0,
                to: 1,
                capacity: 4,
                byte_footprint: small_fp,
                bundle_family: None,
            },
        ];
        let clusters = pack_kruskal(&[0, 1, 2], &edges, 2);
        prop_assert_eq!(clusters.len(), 2);
        let together_01 = clusters.iter().any(|c| c.contains(&0) && c.contains(&1));
        prop_assert!(together_01, "smaller footprint must merge first: {clusters:?}");
    }

    /// Property: two disjoint first-merges then a third merge hits equal-rank union.
    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_equal_rank_union(cap in 1usize..16) {
        let edges = vec![
            PackEdge {
                from: 0,
                to: 1,
                capacity: cap,
                byte_footprint: 8,
                bundle_family: None,
            },
            PackEdge {
                from: 2,
                to: 3,
                capacity: cap,
                byte_footprint: 8,
                bundle_family: None,
            },
            PackEdge {
                from: 0,
                to: 2,
                capacity: cap + 1,
                byte_footprint: 16,
                bundle_family: None,
            },
        ];
        let clusters = pack_kruskal(&[0, 1, 2, 3], &edges, 1);
        prop_assert_eq!(clusters.len(), 1);
        let mut flat = clusters[0].clone();
        flat.sort_unstable();
        prop_assert_eq!(flat, vec![0, 1, 2, 3]);
    }

    /// Property: unmergeable edges leave more clusters than slots → flatten + even_split.
    #[test]
    // ss[verify troupe.dynamic-kruskal-capacity]
    // ss[verify troupe.dynamic-no-incidence-fallback]
    // ss[verify verify.process.proptest]
    fn proptest_kruskal_unmergeable_falls_back_even_split(
        n in 4usize..10,
        slots in 1usize..3,
    ) {
        let ids: Vec<usize> = (0..n).collect();
        let edges = vec![PackEdge {
            from: 100,
            to: 101,
            capacity: 1,
            byte_footprint: 8,
            bundle_family: None,
        }];
        let clusters = pack_kruskal(&ids, &edges, slots);
        let expected = even_split(&ids, slots);
        prop_assert_eq!(clusters, expected);
    }

    /// Property: tight slots force bundle consumers apart when girth equals slots.
    #[test]
    // ss[verify troupe.dynamic-bundle-siblings]
    // ss[verify verify.process.proptest]
    fn proptest_bundle_siblings_respected_under_tight_slots(
        girth in 3usize..6,
    ) {
        let fam = BundleFamily {
            producer: 0,
            family_key: 7,
        };
        let mut ids = vec![0usize];
        let mut edges = Vec::new();
        for i in 1..=girth {
            ids.push(i);
            edges.push(PackEdge {
                from: 0,
                to: i,
                capacity: 4,
                byte_footprint: 32,
                bundle_family: Some(fam),
            });
        }
        let clusters = pack_kruskal(&ids, &edges, girth);
        prop_assert!(clusters.len() <= girth);
        for c in &clusters {
            let consumers = c.iter().filter(|&&id| id != 0).count();
            prop_assert!(consumers <= 1, "consumers merged: {:?}", clusters);
        }
    }
}
