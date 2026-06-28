//! Property tests for aqueduct stream ingress/egress control items.

use std::time::{Duration, Instant};

use proptest::prelude::*;

use crate::distributed::aqueduct_stream::{
    Defrag, StreamControlItem, StreamEgress, StreamIngress,
};
use crate::ss_proptest;

ss_proptest! {
    /// Property: stream control item lengths match payload sizes from builders.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify verify.process.proptest]
    fn proptest_stream_egress_length_matches_payload(
        len in 0usize..64usize,
    ) {
        let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
        let (egress, _) = StreamEgress::by_box(&payload);
        prop_assert_eq!(StreamControlItem::length(&egress) as usize, len);
    }

    /// Property: `StreamIngress::testing_new` length matches the requested size.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify verify.process.proptest]
    fn proptest_stream_ingress_testing_new_length(
        len in 0usize..128usize,
    ) {
        let ingress = StreamIngress::testing_new(len as i32);
        prop_assert_eq!(StreamControlItem::length(&ingress) as usize, len);
    }

    /// Property: `StreamIngress::from_defrag` preserves session id and running length.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify verify.process.proptest]
    fn proptest_stream_ingress_from_defrag_roundtrip(
        len in 0usize..512usize,
        session_id in -512i32..512,
    ) {
        let arrival = Instant::now();
        let finish = arrival + Duration::from_millis(1);
        let mut def = Defrag::<StreamIngress>::new(session_id, 4, len.max(1));
        def.arrival = Some(arrival);
        def.finish = Some(finish);
        def.running_length = len;
        let msg = StreamIngress::from_defrag(&def);
        prop_assert_eq!(StreamControlItem::length(&msg) as usize, len);
        prop_assert_eq!(msg.session_id, session_id);
        prop_assert_eq!(msg.arrival, arrival);
        prop_assert_eq!(msg.finished, finish);
    }

    /// Property: `StreamEgress::from_defrag` preserves running byte length.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify verify.process.proptest]
    fn proptest_stream_egress_from_defrag_roundtrip(len in 0usize..1024usize) {
        let mut def = Defrag::<StreamEgress>::new(1, 4, len.max(1));
        def.running_length = len;
        let msg = StreamEgress::from_defrag(&def);
        prop_assert_eq!(StreamControlItem::length(&msg) as usize, len);
    }

    /// Property: ingress/egress builder helpers agree on payload length.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify verify.process.proptest]
    fn proptest_stream_ingress_egress_build_length(
        len in 0usize..64usize,
    ) {
        let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
        let now = Instant::now();
        let (ingress, _) = StreamIngress::by_box(9, now, now, &payload);
        let (egress, _) = StreamEgress::by_box(&payload);
        prop_assert_eq!(StreamControlItem::length(&ingress) as usize, len);
        prop_assert_eq!(StreamControlItem::length(&egress) as usize, len);
    }

    /// Property: `StreamIngress::by_ref` and `by_box` agree on length and session id.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify verify.process.proptest]
    fn proptest_stream_ingress_by_ref_by_box_agree(
        len in 1usize..48usize,
        session_id in -256i32..256,
    ) {
        let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
        let now = Instant::now();
        let (by_ref, _) = StreamIngress::by_ref(session_id, now, now, &payload);
        let (by_box, box_payload) = StreamIngress::by_box(session_id, now, now, &payload);
        prop_assert_eq!(StreamControlItem::length(&by_ref), StreamControlItem::length(&by_box));
        prop_assert_eq!(by_ref.session_id, by_box.session_id);
        prop_assert_eq!(box_payload.as_ref(), payload.as_slice());
    }
}
