//! Property tests for zero-copy slice helpers on `RxCore` peek views.

// ss[related philosophy.zero-copy-discipline]
use super::{DoubleSlice, DoubleSliceCopy, QuadSlice, StreamQuadSliceCopy};
// ss[related philosophy.zero-copy-discipline]
use crate::distributed::aqueduct_stream::{StreamControlItem, StreamIngress};
// ss[related philosophy.zero-copy-discipline]
// ss[related philosophy.structural-hierarchy]
use crate::RxDone;
// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
use std::time::Instant;

ss_proptest! {
    /// Property: `DoubleSlice` iteration, clone, and length match concatenation.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_double_slice_concat_matches_parts(
        a in prop::collection::vec(0u8..16, 0..12),
        b in prop::collection::vec(0u8..16, 0..12),
    ) {
        let view = (a.as_slice(), b.as_slice());
        let expected: Vec<u8> = a.iter().copied().chain(b.iter().copied()).collect();
        prop_assert_eq!(view.total_len(), expected.len());
        prop_assert_eq!(view.to_vec(), expected);
        let via_iter: Vec<u8> = view.as_iter().copied().collect();
        prop_assert_eq!(via_iter, a.iter().copied().chain(b.iter().copied()).collect::<Vec<_>>());
    }

    /// Property: `DoubleSliceCopy` fills the target from first then second slice.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_double_slice_copy_respects_target(
        a in prop::collection::vec(0u8..32, 0..16),
        b in prop::collection::vec(0u8..32, 0..16),
        target_len in 0usize..24,
    ) {
        let view = (a.as_slice(), b.as_slice());
        let mut target = vec![0xFFu8; target_len];
        let done = view.copy_into_slice(&mut target);
        let want = a.len() + b.len();
        let copied = want.min(target_len);
        prop_assert_eq!(done, RxDone::Normal(copied));
        let mut expected = a.clone();
        expected.extend_from_slice(&b);
        expected.truncate(copied);
        prop_assert_eq!(&target[..copied], expected.as_slice());
        if copied < target_len {
            prop_assert!(target[copied..].iter().all(|b| *b == 0xFF));
        }
    }

    /// Property: `QuadSlice` item/payload views stay independent and ordered.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_quad_slice_items_and_payloads(
        ia in prop::collection::vec(0u16..32, 0..8),
        ib in prop::collection::vec(0u16..32, 0..8),
        pa in prop::collection::vec(0u8..32, 0..12),
        pb in prop::collection::vec(0u8..32, 0..12),
    ) {
        let view = (ia.as_slice(), ib.as_slice(), pa.as_slice(), pb.as_slice());
        let items: Vec<u16> = ia.iter().copied().chain(ib.iter().copied()).collect();
        let payload: Vec<u8> = pa.iter().copied().chain(pb.iter().copied()).collect();
        prop_assert_eq!(view.items_len(), items.len());
        prop_assert_eq!(view.payload_len(), payload.len());
        prop_assert_eq!(view.items_vec(), items);
        prop_assert_eq!(view.payload_vec(), payload);
        let items_iter: Vec<u16> = view.items_iter().copied().collect();
        let payload_iter: Vec<u8> = view.payload_iter().copied().collect();
        prop_assert_eq!(items_iter, ia.iter().copied().chain(ib.iter().copied()).collect::<Vec<_>>());
        prop_assert_eq!(payload_iter, pa.iter().copied().chain(pb.iter().copied()).collect::<Vec<_>>());
    }

    /// Property: stream quad copy stops when items or payload bytes no longer fit.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify stream.control-payload]
    // ss[verify verify.process.proptest]
    fn proptest_stream_quad_copy_stops_when_full(
        lens_a in prop::collection::vec(0u8..8, 0..6),
        lens_b in prop::collection::vec(0u8..8, 0..6),
        item_room in 0usize..10,
        payload_room in 0usize..24,
        split in 0usize..16,
    ) {
        let now = Instant::now();
        let items_a: Vec<StreamIngress> = lens_a
            .iter()
            .map(|len| StreamIngress::new(*len as i32, 0, now, now))
            .collect();
        let items_b: Vec<StreamIngress> = lens_b
            .iter()
            .map(|len| StreamIngress::new(*len as i32, 1, now, now))
            .collect();
        let mut payload: Vec<u8> = Vec::new();
        for len in lens_a.iter().chain(lens_b.iter()) {
            payload.extend(std::iter::repeat(7u8).take(*len as usize));
        }
        let split = split.min(payload.len());
        let (c, d) = payload.split_at(split);
        let view = (items_a.as_slice(), items_b.as_slice(), c, d);

        let mut item_target = vec![StreamIngress::default(); item_room];
        let mut payload_target = vec![0u8; payload_room];
        let (copied_items, copied_bytes) = view.copy_items_and_payloads(&mut item_target, &mut payload_target);

        let mut expect_items = 0usize;
        let mut expect_bytes = 0usize;
        for item in items_a.iter().chain(items_b.iter()) {
            let need = item.length() as usize;
            if expect_items < item_room && expect_bytes + need <= payload_room {
                expect_items += 1;
                expect_bytes += need;
            } else {
                break;
            }
        }
        prop_assert_eq!(copied_items, expect_items);
        prop_assert_eq!(copied_bytes, expect_bytes);
        prop_assert_eq!(&payload_target[..copied_bytes], &payload[..copied_bytes]);
        for (got, want) in item_target.iter().take(copied_items).zip(items_a.iter().chain(items_b.iter())) {
            prop_assert_eq!(got.length(), want.length());
        }
    }
}
