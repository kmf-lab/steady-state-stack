//! Tests extracted from `aqueduct_stream.rs` so the production file stays under the 1,200-line budget.
//! Nested test mods keep their own imports; top-level `use super::*` is unused here but documents the pattern.


#[cfg(test)]
// ss[related distributed.aqueduct-stream]
mod extra_stream_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::super::*;
    // ss[related philosophy.structural-hierarchy]
    
    // ss[related distributed.aqueduct-stream]
    use std::sync::Arc;
    // ss[related distributed.aqueduct-stream]
    use std::time::{Duration, Instant};
    // ss[related philosophy.structural-hierarchy]
    use crate::GraphBuilder;

    /// Tests the behavior of extracting payload slices from receiver buffers, covering both first and second slice cases.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_extract_stream_payload_slices_behavior() {
        let mut p1 = [1u8, 2, 3];
        let mut p2 = [4u8, 5, 6, 7];
        let mut on_first = true;
        let mut idx = 0;
        // First slice: take 2 bytes from p1
        let (a1, b1) = StreamRx::<StreamEgress>::extract_stream_payload_slices(&mut p1, &mut p2, &mut on_first, &mut idx, 2);
        assert_eq!(a1, &mut [1u8, 2][..]);
        assert_eq!(b1.len(), 0);
        assert!(on_first);
        assert_eq!(idx, 2);
        // Second slice: p1 has 1 left, so a from p1[2..3], b from p2[0..1]
        let (a2, b2) = StreamRx::<StreamEgress>::extract_stream_payload_slices(&mut p1, &mut p2, &mut on_first, &mut idx, 2);
        assert_eq!(a2, &mut [3u8][..]);
        assert_eq!(b2, &mut [4u8][..]);
        assert!(!on_first);
        assert_eq!(idx, 1);
        // Third slice: on_first=false, consume from p2
        let (a3, b3) = StreamRx::<StreamEgress>::extract_stream_payload_slices(&mut p1, &mut p2, &mut on_first, &mut idx, 2);
        assert_eq!(a3, &mut [5u8, 6][..]);
        assert_eq!(b3.len(), 0);
    }

    /// Tests the initialization of defragmentation entries, verifying field values and buffer capacities.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_defrag_new_properties() {
        let items = 3;
        let bytes = 5;
        let session_id = 42;
        let def: Defrag<StreamEgress> = Defrag::new(session_id, items, bytes);
        assert_eq!(def.session_id, session_id);
        assert_eq!(def.running_length, 0);
        assert!(def.arrival.is_none());
        assert!(def.finish.is_none());
        // Writer side vacant_len equals capacity
        assert_eq!(def.ringbuffer_items.0.vacant_len(), items);
        assert_eq!(def.ringbuffer_bytes.0.vacant_len(), bytes);
    }

    /// Tests the creation of incoming stream fragments from defragmentation entries.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_stream_session_message_new_and_from_defrag() {
        let arrival = Instant::now();
        let finish = arrival + Duration::from_secs(1);
        let mut def = Defrag::<StreamIngress>::new(7, 4, 4);
        def.arrival = Some(arrival);
        def.finish = Some(finish);
        def.running_length = 8;
        let msg = StreamIngress::from_defrag(&def);
        assert_eq!(msg.length(), 8);
        assert_eq!(msg.session_id, 7);
        assert_eq!(msg.arrival, arrival);
        assert_eq!(msg.finished, finish);
    }

    /// Tests the creation of incoming stream fragments for testing purposes.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_testing_new_methods() {
        let tn = StreamIngress::testing_new(9);
        assert_eq!(tn.length(), 9);
    }

    /// Tests the relationship between rate collector constants.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_rate_collector_constants() {
        assert_eq!(RATE_COLLECTOR_LEN, 32);
        assert_eq!(RATE_COLLECTOR_MASK, 31);
        // Masking LEN yields zero
        assert_eq!(RATE_COLLECTOR_LEN & RATE_COLLECTOR_MASK, 0);
    }

    /// Tests the behavior of marking an empty transmitter bundle as closed.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_stream_tx_bundle_trait_empty() {
        // ss[related philosophy.structural-hierarchy]
        type Bundle = StreamTxBundle<'static, StreamEgress>;
        let mut bundle: Bundle = Vec::new();
        assert!(bundle.mark_closed(), "even Empty bundle should return true");
    }

    /// Tests the state inspection methods for an empty receiver bundle.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_stream_rx_bundle_trait_empty() {
        // ss[related philosophy.structural-hierarchy]
        type Bundle = StreamRxBundle<'static, StreamEgress>;
        let mut bundle: Bundle = Vec::new();
        assert!(bundle.is_closed_and_empty());
        assert!(bundle.is_closed());
        assert!(bundle.is_empty());
    }

    /// Tests cloning an empty transmitter bundle.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_lazy_tx_bundle_clone_empty() {
        let empty: [LazyStreamTx<StreamEgress>; 0] = [];
        let cloned: SteadyStreamTxBundle<StreamEgress, 0> = empty.clone();
        // An Arc<[..;0]> has length 0
        assert_eq!(Arc::as_ref(&cloned).len(), 0);
    }

    /// Tests cloning an empty receiver bundle.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_lazy_rx_bundle_clone_empty() {
        let empty: [LazyStreamRx<StreamEgress>; 0] = [];
        let cloned: SteadyStreamRxBundle<StreamEgress, 0> = empty.clone();
        assert_eq!(Arc::as_ref(&cloned).len(), 0);
    }

    /// Tests the behavior of receiver bundle operations for an empty bundle.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_steady_rx_bundle_trait_empty() {
        let bundle: SteadyStreamRxBundle<StreamEgress, 0> = Arc::new([]);
        // lock() is a JoinAll over zero futures: completes immediately
        let guards: Vec<_> = core_exec::block_on(bundle.lock());
        assert!(guards.is_empty());

        let ctrl = bundle.control_meta_data();
        assert_eq!(ctrl.len(), 0, "control_meta_data for GIRTH=0 must be length 0");
        let payload = bundle.payload_meta_data();
        assert_eq!(payload.len(), 0, "payload_meta_data for GIRTH=0 must be length 0");
    }

    /// Tests the behavior of transmitter bundle operations for an empty bundle.
    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_steady_tx_bundle_trait_empty() {
        let bundle: SteadyStreamTxBundle<StreamEgress, 0> = Arc::new([]);
        let guards: Vec<_> = core_exec::block_on(bundle.lock());
        assert!(guards.is_empty());

        let ctrl = bundle.control_meta_data();
        assert_eq!(ctrl.len(), 0, "control_meta_data for GIRTH=0 must be length 0");
        let payload = bundle.payload_meta_data();
        assert_eq!(payload.len(), 0, "payload_meta_data for GIRTH=0 must be length 0");
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_defrag_ensure_additional_capacity() {
        let mut defrag = Defrag::<StreamEgress>::new(1, 2, 2);
        defrag.ensure_additional_capacity(5, 10);
        assert!(defrag.ringbuffer_items.0.vacant_len() >= 5);
        assert!(defrag.ringbuffer_bytes.0.vacant_len() >= 10);
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_stream_ingress_egress_build_helpers() {
        let now = Instant::now();
        let (ingress, payload) = StreamIngress::by_box(7, now, now + Duration::from_millis(1), b"abc");
        assert_eq!(StreamControlItem::length(&ingress), 3);
        assert_eq!(payload.as_ref(), b"abc");
        let (ingress2, _) = StreamIngress::build(8, now, now, b"xy");
        assert_eq!(ingress2.session_id, 8);

        let (egress, box_payload) = StreamEgress::build(b"payload");
        assert_eq!(egress.length(), 7);
        assert_eq!(box_payload.as_ref(), b"payload");
        let (egress2, slice) = StreamEgress::by_ref(b"z");
        assert_eq!(egress2.length(), 1);
        assert_eq!(slice, b"z");
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_stream_tx_rate_and_poll_helpers() {
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(32);
        let (lazy_tx, _lazy_rx) = cb.build_stream::<StreamIngress>(16);
        let steady = lazy_tx.clone();
        let mut tx = core_exec::block_on(steady.lock());

        tx.store_input_data_rate(Duration::from_millis(10), 2, 16);
        tx.store_output_data_rate(Duration::from_millis(20), 1, 8);
        tx.set_stored_vacant_values(4, 32);
        let (msgs, bytes) = tx.get_stored_vacant_values();
        assert_eq!(msgs, 4);
        assert_eq!(bytes, 32);

        let fastest = tx.fastest_byte_processing_duration();
        assert!(fastest.is_some());
        let (min, max) = tx.next_poll_bounds();
        assert!(max >= min);

        let (avg, std) = tx.guess_duration_between_arrivals();
        assert!(avg <= Duration::from_secs(1));
        assert!(std <= Duration::from_secs(1));
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_stream_tx_fragment_consume_and_defrag_room() {
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(32);
        let (lazy_tx, _lazy_rx) = cb.build_stream::<StreamIngress>(32);
        let steady_tx = lazy_tx.clone();
        let room = core_exec::block_on(async {
            let mut tx = steady_tx.lock().await;
            let now = Instant::now();
            tx.fragment_consume(42, b"frag", true, true, now);
            assert!(!tx.ready_msg_session.is_empty());
            tx.defrag_has_room_for()
        });
        assert!(room > 0);
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    fn test_lazy_stream_testing_send_and_take_roundtrip() {
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(64);
        let (lazy_tx, lazy_rx) = cb.build_stream::<StreamEgress>(32);
        lazy_tx.testing_send_frame(b"frame-bytes");
        lazy_tx.testing_close();
        assert!(lazy_rx.testing_avail_wait(1, Duration::from_secs(1)));
        let taken = lazy_rx.testing_take_all();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].1.as_ref(), b"frame-bytes");
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify bundle.girth-const-generic]
    fn test_lazy_bundle_clone_girth_2() {
        // ss[related philosophy.structural-hierarchy]
        const GIRTH: usize = 2;
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(32);
        let (lazy_tx, lazy_rx) = cb.build_stream_bundle::<StreamEgress, GIRTH>(16);
        let tx_bundle = LazySteadyStreamTxBundleClone::clone(&lazy_tx);
        let rx_bundle = LazySteadyStreamRxBundleClone::clone(&lazy_rx);
        assert_eq!(tx_bundle.len(), GIRTH);
        assert_eq!(rx_bundle.len(), GIRTH);
        assert_eq!(tx_bundle.control_meta_data().len(), GIRTH);
        assert_eq!(tx_bundle.payload_meta_data().len(), GIRTH);
        assert_eq!(rx_bundle.control_meta_data().len(), GIRTH);
        assert_eq!(rx_bundle.payload_meta_data().len(), GIRTH);
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify bundle.girth-const-generic]
    fn test_lazy_bundle_clone_girth_3() {
        // ss[related philosophy.structural-hierarchy]
        const GIRTH: usize = 3;
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(32);
        let (lazy_tx, lazy_rx) = cb.build_stream_bundle::<StreamEgress, GIRTH>(16);
        let tx_bundle = LazySteadyStreamTxBundleClone::clone(&lazy_tx);
        let rx_bundle = LazySteadyStreamRxBundleClone::clone(&lazy_rx);
        assert_eq!(tx_bundle.len(), GIRTH);
        assert_eq!(rx_bundle.len(), GIRTH);
        assert_eq!(tx_bundle.control_meta_data().len(), GIRTH);
        assert_eq!(rx_bundle.payload_meta_data().len(), GIRTH);
    }

    #[test]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify bundle.girth-const-generic]
    fn test_lazy_bundle_clone_girth_4() {
        // ss[related philosophy.structural-hierarchy]
        const GIRTH: usize = 4;
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(32);
        let (lazy_tx, lazy_rx) = cb.build_stream_bundle::<StreamEgress, GIRTH>(16);
        let tx_bundle = LazySteadyStreamTxBundleClone::clone(&lazy_tx);
        let rx_bundle = LazySteadyStreamRxBundleClone::clone(&lazy_rx);
        assert_eq!(tx_bundle.len(), GIRTH);
        assert_eq!(rx_bundle.len(), GIRTH);
        assert_eq!(tx_bundle.control_meta_data().len(), GIRTH);
        assert_eq!(rx_bundle.payload_meta_data().len(), GIRTH);
    }

    /// Tests combined `memory_bytes()` on an established stream Tx.
    // ss[verify channel.memory-usage-telemetry]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_stream_tx_memory_bytes() {
        // ss[related philosophy.structural-hierarchy]
        use std::mem::size_of;
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(10);
        let (lazy_tx, _lazy_rx) = cb.build_stream::<StreamEgress>(4);
        let stream_tx = lazy_tx.clone();
        let guard = crate::core_exec::block_on(stream_tx.lock());
        let (ctrl_cap, payload_cap) = guard.capacity();
        assert_eq!(ctrl_cap, 10);
        assert_eq!(payload_cap, 40); // 10 × 4 bytes_per_item
        let expected = ctrl_cap * size_of::<StreamEgress>() + payload_cap;
        assert_eq!(guard.memory_bytes(), expected);
    }

}

