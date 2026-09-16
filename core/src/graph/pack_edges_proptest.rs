//! Property tests for `Graph::rebuild_pack_edges` incidence reconstruction.

use super::{Graph, PackPorts};
use crate::actor_builder::pack::{BundleFamily, PackEdge};
use crate::ss_proptest;
use crate::*;
use proptest::prelude::*;

fn empty_graph() -> Graph {
    GraphBuilder::for_testing().build(())
}

ss_proptest! {
    /// Property: empty pack_ports falls back to recorded pack_incidence.
    #[test]
    // ss[verify graph.pack.incidence-before-start]
    // ss[verify verify.process.proptest]
    fn proptest_rebuild_empty_ports_uses_incidence(
        cap in 1usize..64,
    ) {
        let graph = empty_graph();
        let fallback = PackEdge {
            from: 1,
            to: 2,
            capacity: cap,
            byte_footprint: cap * 8,
            bundle_family: None,
        };
        graph.pack_incidence.lock().push(fallback.clone());
        let edges = graph.rebuild_pack_edges();
        prop_assert_eq!(edges, vec![fallback]);
    }

    /// Property: RX without TX, self-loop, girth/bundle, and missing meta defaults.
    #[test]
    // ss[verify graph.pack.incidence-before-start]
    // ss[verify verify.process.proptest]
    fn proptest_rebuild_pack_edges_incidence_rules(
        cap in 1usize..128,
        bytes in 8usize..4096,
        girth in 1usize..6,
        family_key in 0usize..16,
        use_meta in any::<bool>(),
        include_orphan_rx in any::<bool>(),
        include_self_loop in any::<bool>(),
    ) {
        let graph = empty_graph();
        {
            let mut ports = graph.pack_ports.lock();
            ports.insert(
                10,
                PackPorts {
                    rx_channel_ids: vec![],
                    tx_channel_ids: vec![1],
                },
            );
            ports.insert(
                11,
                PackPorts {
                    rx_channel_ids: vec![1],
                    tx_channel_ids: vec![],
                },
            );
            if include_self_loop {
                ports.insert(
                    12,
                    PackPorts {
                        rx_channel_ids: vec![2],
                        tx_channel_ids: vec![2],
                    },
                );
            }
            if include_orphan_rx {
                ports.insert(
                    13,
                    PackPorts {
                        rx_channel_ids: vec![99],
                        tx_channel_ids: vec![],
                    },
                );
            }
        }
        if use_meta {
            graph.channel_pack_meta.lock().insert(
                1,
                (cap, bytes, girth, Some(family_key)),
            );
            if include_self_loop {
                graph
                    .channel_pack_meta
                    .lock()
                    .insert(2, (cap, bytes, 1, None));
            }
        }
        let edges = graph.rebuild_pack_edges();
        let pair: Vec<_> = edges.iter().filter(|e| e.from == 10 && e.to == 11).collect();
        prop_assert_eq!(pair.len(), 1);
        let e = pair[0];
        if use_meta {
            prop_assert_eq!(e.capacity, cap);
            prop_assert_eq!(e.byte_footprint, bytes);
            if girth > 1 {
                prop_assert_eq!(
                    e.bundle_family,
                    Some(BundleFamily {
                        producer: 10,
                        family_key,
                    })
                );
            } else {
                prop_assert!(e.bundle_family.is_none());
            }
        } else {
            prop_assert_eq!(e.capacity, 64);
            prop_assert_eq!(e.byte_footprint, 512);
            prop_assert!(e.bundle_family.is_none());
        }
        prop_assert!(!edges.iter().any(|e| e.from == e.to));
        prop_assert!(!edges.iter().any(|e| e.from == 13 || e.to == 13));
    }
}

#[test]
// ss[verify troupe.dynamic-finalize-at-start]
// ss[verify troupe.dynamic-is-a-bag]
fn finalize_dynamic_troupes_empty_pending_is_noop() {
    let mut graph = GraphBuilder::for_testing().build(());
    graph.finalize_dynamic_troupes();
    graph.finalize_dynamic_troupes();
}

#[test]
// ss[verify troupe.dynamic-finalize-at-start]
fn finalize_dynamic_troupes_empty_builder_bag_is_noop() {
    let mut graph = GraphBuilder::for_testing().build(());
    {
        let bag = graph.dynamic_troupe();
        drop(bag);
    }
    graph.finalize_dynamic_troupes();
}
