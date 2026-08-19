// ss[related channel.stream-dual-buffer]
use std::time::Duration;
// ss[related philosophy.structural-hierarchy]
use futures_timer::Delay;
// ss[related philosophy.structural-hierarchy]
use futures_util::{select, FutureExt};
// ss[related channel.stream-dual-buffer]
use ringbuf::traits::Observer;
// ss[related philosophy.structural-hierarchy]
use ringbuf::consumer::Consumer;
// ss[related philosophy.structural-hierarchy]
use futures_util::future::FusedFuture;
// ss[related channel.stream-dual-buffer]
use async_ringbuf::consumer::AsyncConsumer;
// ss[related philosophy.structural-hierarchy]
use crate::{yield_now, RxCore, RxDone, StreamControlItem, StreamRx, warn};
// ss[related philosophy.structural-hierarchy]
use crate::monitor_telemetry::SteadyTelemetrySend;

/// Implementation of `RxCore` for stream-based channels (`StreamRx<T>`).
///
/// This implementation manages a dual-channel system with a control channel for `T: StreamControlItem`
/// and a payload channel for byte data, ensuring synchronized reception of control messages and
/// their associated payloads.
// ss[related channel.stream-dual-buffer]
impl<T: StreamControlItem> RxCore for StreamRx<T> {
    /// The type of message item stored in the channel.
    // ss[related philosophy.structural-hierarchy]
    type MsgItem = T;

    /// The type of message that is taken out of the channel, a tuple of the control item and its payload.
    // ss[related channel.stream-dual-buffer]
    type MsgOut = (T, Box<[u8]>);

    /// The type used to peek at a message, a tuple of references to the control item and its payload slices.
    // ss[related channel.stream-dual-buffer]
    type MsgPeek<'a> = (&'a T, &'a [u8], &'a [u8]) where T: 'a;

    /// The type used to count messages, a tuple of control items and payload bytes.
    // ss[related channel.stream-dual-buffer]
    type MsgSize = (usize, usize);

    /// The type for a slice of messages to be peeked at, a quadruple of slices for control and payload.
    // ss[related channel.stream-dual-buffer]
    type SliceSource<'a> = (&'a [T], &'a [T], &'a [u8], &'a [u8]) where T: 'a;

    /// The type for the target slices where messages are copied, a pair of mutable slices for control and payload.
    // ss[related channel.stream-dual-buffer]
    type SliceTarget<'b> = (&'b mut [T], &'b mut [u8]) where T: 'b;

    // ss[related philosophy.structural-hierarchy]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        match done_count {
            RxDone::Normal(i) => {
                warn!("internal error should have gotten Stream");
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.id(), i as isize);
            }
            RxDone::Stream(c, p) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.id(), c as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.id(), p as isize);
            }
        }
    }

    // ss[related channel.stream-dual-buffer]
    fn monitor_not(&mut self) {
        self.control_channel.monitor_not();
        self.payload_channel.monitor_not();
    }

    // ss[related channel.stream-dual-buffer]
    fn log_periodic(&mut self) -> bool {
        self.control_channel.log_periodic()
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_validate_capacity_items(&self, items_count: usize) -> usize {
        self.shared_capacity().0.min(items_count)
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_avail_items_count(&mut self) -> usize {
        self.shared_avail_units().0
    }

    // ss[related channel.stream-dual-buffer]
    fn is_closed_and_empty(&mut self) -> bool {
        self.control_channel.is_closed_and_empty() && self.payload_channel.is_closed_and_empty()
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_advance_index(&mut self, count: Self::MsgSize) -> RxDone {
        let control_avail = self.control_channel.rx.occupied_len();
        let payload_avail = self.payload_channel.rx.occupied_len();
        if count.0 <= control_avail && count.1 <= payload_avail {
            unsafe {
                self.payload_channel.rx.advance_read_index(count.1);
                self.control_channel.rx.advance_read_index(count.0);
            }

            self.payload_channel.take_count.fetch_add(count.1 as u32, std::sync::atomic::Ordering::Relaxed);
            self.control_channel.take_count.fetch_add(count.0 as u32, std::sync::atomic::Ordering::Relaxed);

            RxDone::Stream(count.0, count.1)
        } else {
            RxDone::Stream(0, 0)
        }
    }

    // ss[related channel.stream-dual-buffer]
    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>> {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.rx.wait_occupied(1);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down => {}, _ = operation => {}, _ = timeout => {} };
            } else {
                select! { _ = one_down => {}, _ = operation => {} };
            }
        }
        let result = self.control_channel.rx.first();
        if let Some(item) = result {
            let take_count = self.control_channel.take_count.load(std::sync::atomic::Ordering::Relaxed);
            let cached_take_count = self.control_channel.cached_take_count.load(std::sync::atomic::Ordering::Relaxed);
            if cached_take_count != take_count {
                self.control_channel.peek_repeats.store(0, std::sync::atomic::Ordering::Relaxed);
                self.control_channel.cached_take_count.store(take_count, std::sync::atomic::Ordering::Relaxed);
            } else {
                self.control_channel.peek_repeats.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let (a, b) = self.payload_channel.rx.as_slices();
            let count_a = a.len().min(item.length() as usize);
            let count_b = item.length() as usize - count_a;
            Some((item, &a[0..count_a], &b[0..count_b]))
        } else {
            self.control_channel.peek_repeats.store(0, std::sync::atomic::Ordering::Relaxed);
            None
        }
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_capacity(&self) -> Self::MsgSize {
        (self.control_channel.rx.capacity().get(), self.payload_channel.rx.capacity().get())
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size<=cap
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.rx.is_empty()
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_avail_units(&mut self) -> Self::MsgSize {
        (self.control_channel.rx.occupied_len(), self.payload_channel.rx.occupied_len())
    }
    // ss[related channel.stream-dual-buffer]
    fn shared_avail_units_for(&mut self, size: Self::MsgSize) -> bool {
        let avail = self.shared_avail_units();
         avail >= size
    }

    // ss[related channel.stream-dual-buffer]
    async fn shared_wait_shutdown_or_avail_units(&mut self, count: Self::MsgSize) -> bool {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.rx.wait_occupied(count.0);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.shared_avail_units() >= count
        }
    }

    // ss[related channel.stream-dual-buffer]
    async fn shared_wait_closed_or_avail_units(&mut self, count:usize) -> bool {
        if self.shared_avail_units_for((count,1)) {
            true
        } else {
            let mut i_closed = &mut self.control_channel.is_closed;
            if !i_closed.is_terminated() {
                let mut operation = &mut self.control_channel.rx.wait_occupied(count);
                select! { _ = i_closed => self.control_channel.rx.occupied_len() >= count, _ = operation => true }
            } else {
                yield_now::yield_now().await;
                self.shared_avail_units_for((count,1))
            }
        }
    }

    // ss[related channel.stream-dual-buffer]
    async fn shared_wait_avail_units(&mut self, size: Self::MsgSize) -> bool {
        if self.shared_avail_units_for(size) {
            true
        } else {
            let operation = &mut self.control_channel.rx.wait_occupied(size.0);
            operation.await;
            true
        }
    }

    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        if let Some(item) = self.control_channel.rx.try_peek() {
            if item.length() <= self.payload_channel.rx.occupied_len() as i32 {
                let mut payload = vec![0u8; item.length() as usize];
                self.payload_channel.rx.peek_slice(&mut payload);
                let payload = payload.into_boxed_slice();
                if let Some(item) = self.control_channel.rx.try_pop() {
                    unsafe { self.payload_channel.rx.advance_read_index(payload.len()); }
                    self.control_channel.take_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Some((RxDone::Stream(1, payload.len()), (item, payload)))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_take_slice(&mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy {
        let (item_target, payload_target) = target;
        let (item_a, item_b) = self.control_channel.rx.as_slices();
        let mut items_copied = 0;
        let mut payload_bytes_needed = 0;
        let max_items = item_target.len();
        let max_payload = payload_target.len();

        for item in item_a {
            let item_len = item.length() as usize;
            if items_copied < max_items && payload_bytes_needed + item_len <= max_payload {
                item_target[items_copied] = *item;
                items_copied += 1;
                payload_bytes_needed += item_len;
            } else {
                break;
            }
        }
        for item in item_b {
            let item_len = item.length() as usize;
            if items_copied < max_items && payload_bytes_needed + item_len <= max_payload {
                item_target[items_copied] = *item;
                items_copied += 1;
                payload_bytes_needed += item_len;
            } else {
                break;
            }
        }

        let (payload_a, payload_b) = self.payload_channel.rx.as_slices();
        let mut payload_copied = 0;
        let n = payload_a.len().min(payload_bytes_needed);
        if n > 0 {
            payload_target[..n].copy_from_slice(&payload_a[..n]);
            payload_copied += n;
        }
        if payload_copied < payload_bytes_needed {
            let m = payload_b.len().min(payload_bytes_needed - payload_copied);
            if m > 0 {
                payload_target[payload_copied..payload_copied + m].copy_from_slice(&payload_b[..m]);
                payload_copied += m;
            }
        }

        unsafe {
            self.payload_channel.rx.advance_read_index(payload_copied);
            self.control_channel.rx.advance_read_index(items_copied);
        }

        self.control_channel.take_count.fetch_add(payload_copied as u32, std::sync::atomic::Ordering::Relaxed);
        self.payload_channel.take_count.fetch_add(items_copied as u32, std::sync::atomic::Ordering::Relaxed);

        RxDone::Stream(items_copied, payload_copied)
    }

    // ss[related channel.stream-dual-buffer]
    fn shared_peek_slice(&mut self) -> Self::SliceSource<'_> {
        let (item_a, item_b) = self.control_channel.rx.as_slices();
        let (payload_a, payload_b) = self.payload_channel.rx.as_slices();
        (item_a, item_b, payload_a, payload_b)
    }

    // ss[related channel.stream-dual-buffer]
    fn one(&self) -> Self::MsgSize {
       (1,1)
    }
}

#[cfg(test)]
// ss[related channel.stream-dual-buffer]
mod core_rx_stream_tests {
    // ss[related philosophy.structural-hierarchy]
    use std::time::Duration;
    // ss[related philosophy.structural-hierarchy]
    use async_ringbuf::traits::Producer;
    // ss[related channel.stream-dual-buffer]
    use crate::{GraphBuilder, ScheduleAs, SteadyActor, StreamEgress, StreamIngress, RxCore, core_exec, RxDone, steady_rx::RxMetaDataProvider};
    // ss[related philosophy.structural-hierarchy]
    use crate::core_tx::TxCore;

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_general() -> Result<(),Box<dyn std::error::Error>> {
        let mut graph = GraphBuilder::for_testing().build(());

        let bytes_per_item = 128;
        let mut channel_builder = graph.channel_builder();
        channel_builder = channel_builder.with_capacity(100);
        channel_builder = channel_builder.with_type();
        let (_tx, _rx) = channel_builder.build_stream::<StreamEgress>(bytes_per_item);

        graph.actor_builder().with_name("unit_test").build(
            move |mut actor| {
                Box::pin(async move {
                    while actor.is_running(|| true) {
                        actor.wait_periodic(Duration::from_millis(1)).await;
                    }
                    Ok::<(), Box<dyn std::error::Error>>(())
                })
            },
            ScheduleAs::SoloAct,
        );

        graph.start();
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(5))?;
        Ok(())
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_core_basics() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder()
                .with_capacity(10)
                .build_stream::<StreamIngress>(100);
            
            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            
            // Test shared_capacity
            let cap = rx_guard.shared_capacity();
            assert!(cap.0 >= 10);
            assert!(cap.1 >= 1000);

            // Test shared_is_empty
            assert!(rx_guard.shared_is_empty());

            // Test shared_avail_units
            assert_eq!(rx_guard.shared_avail_units(), (0, 0));

            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_shared_capacity_for_bounds() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph
                .channel_builder()
                .with_capacity(10)
                .build_stream::<StreamIngress>(100);

            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            let cap = rx_guard.shared_capacity();
            assert!(rx_guard.shared_capacity_for(cap));
            assert!(!rx_guard.shared_capacity_for((cap.0.saturating_add(1), cap.1)));
            assert!(!rx_guard.shared_avail_units_for((1, 1)));

            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_peek_async_timeout() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder()
                .with_capacity(10)
                .build_stream::<StreamIngress>(100);
            
            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            
            let start = std::time::Instant::now();
            let peeked = rx_guard.shared_peek_async_timeout(Some(Duration::from_millis(50))).await;
            assert!(peeked.is_none());
            assert!(start.elapsed() >= Duration::from_millis(50));
            
            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_take_slice_logic() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder()
                .with_capacity(10)
                .build_stream::<StreamEgress>(100);
            
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            tx_guard.shared_try_send(&[1, 2, 3][..]).unwrap();
            tx_guard.shared_try_send(&[4, 5][..]).unwrap();
            drop(tx_guard);

            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            
            let mut item_target = [StreamEgress::default(); 2];
            let mut payload_target = [0u8; 10];
            let done = rx_guard.shared_take_slice((&mut item_target, &mut payload_target));
            
            assert!(matches!(done, RxDone::Stream(2, 5)));
            assert_eq!(&payload_target[0..5], &[1, 2, 3, 4, 5]);
            
            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_telemetry_normal() {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder()
                .with_capacity(5)
                .build_stream::<StreamIngress>(10);
            
            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            
            let meta = rx_guard.control_channel.channel_meta_data.meta_data.clone();
            let mut actor = graph.new_testing_test_monitor("test")
                .into_spotlight([&meta as &dyn RxMetaDataProvider], []);

            if let Some(ref mut tel) = actor.telemetry.send_rx {
                rx_guard.telemetry_inc(RxDone::Normal(1), tel);
            }
        });
    }

    // #[test]
    // fn test_stream_rx_peek_repeats_logic() -> Result<(), Box<dyn std::error::Error>> {
    //     core_exec::block_on(async {
    //         let mut graph = GraphBuilder::for_testing().build(());
    //         let (tx, rx) = graph.channel_builder()
    //             .with_capacity(10)
    //             .build_stream::<StreamIngress>(10);
    //         
    //         let tx_clone = tx.clone();
    //         let mut tx_guard = tx_clone.lock().await;
    //         let now = std::time::Instant::now();
    //         tx_guard.shared_try_send((StreamIngress::new(5, 0, now, now), &[0u8; 5][..])).unwrap();
    //         drop(tx_guard);
    // 
    //         let rx_clone = rx.clone();
    //         let mut rx_guard = rx_clone.lock().await;
    //         
    //         // First peek
    //         rx_guard.shared_peek_async_timeout(None).await;
    //         assert_eq!(rx_guard.control_channel.peek_repeats.load(std::sync::atomic::Ordering::Relaxed), 0);
    //         
    //         // Second peek (same take_count)
    //         rx_guard.shared_peek_async_timeout(None).await;
    //         assert_eq!(rx_guard.control_channel.peek_repeats.load(std::sync::atomic::Ordering::Relaxed), 1);
    //         
    //         // Take it
    //         rx_guard.shared_try_take().unwrap();
    //         
    //         // Peek again (empty)
    //         rx_guard.shared_peek_async_timeout(None).await;
    //         assert_eq!(rx_guard.control_channel.peek_repeats.load(std::sync::atomic::Ordering::Relaxed), 0);
    //         
    //         Ok::<(), Box<dyn std::error::Error>>(())
    //     })
    // }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_take_slice_wrap_around() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder()
                .with_capacity(4) // Small capacity
                .build_stream::<StreamEgress>(1); // 4 bytes total
            
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            
            // Fill and empty to move indices
            tx_guard.shared_try_send(&[1, 2][..]).unwrap();
            tx_guard.shared_try_send(&[3, 4][..]).unwrap();
            drop(tx_guard);
            
            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            rx_guard.shared_try_take().unwrap();
            rx_guard.shared_try_take().unwrap();
            drop(rx_guard);
            
            // Now indices are at the end. Push more to wrap.
            let mut tx_guard = tx_clone.lock().await;
            tx_guard.shared_try_send(&[5, 6][..]).unwrap();
            tx_guard.shared_try_send(&[7, 8][..]).unwrap();
            drop(tx_guard);
            
            let mut rx_guard = rx_clone.lock().await;
            let mut item_target = [StreamEgress::default(); 2];
            let mut payload_target = [0u8; 4];
            let done = rx_guard.shared_take_slice((&mut item_target, &mut payload_target));
            
            assert_eq!(done, RxDone::Stream(2, 4));
            assert_eq!(&payload_target, &[5, 6, 7, 8]);
            
            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_advance_fail() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder()
                .with_capacity(10)
                .build_stream::<StreamIngress>(100);
            
            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            
            let done = rx_guard.shared_advance_index((20, 2000));
            assert_eq!(done, RxDone::Stream(0, 0));
            
            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    fn test_stream_rx_try_take_partial_payload() -> Result<(), Box<dyn std::error::Error>> {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder()
                .with_capacity(10)
                .build_stream::<StreamIngress>(10);
            
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            
            // Manually push an item to control but NOT enough to payload
            let now = std::time::Instant::now();
            tx_guard.control_channel.tx.try_push(StreamIngress::new(10, 0, now, now)).unwrap();
            // Payload needs 10, but we push 5
            tx_guard.payload_channel.tx.push_slice(&[0u8; 5]);
            drop(tx_guard);

            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            
            let result = rx_guard.shared_try_take();
            assert!(result.is_none()); // Should fail because payload is incomplete
            
            Ok::<(), Box<dyn std::error::Error>>(())
        })
    }

    // ss[related channel.stream-dual-buffer]
    use proptest::prelude::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::proptest_support::capacity;

    ss_proptest! {

        /// Property: stream rx avail control and payload units never exceed capacity.
        #[test]
        // ss[verify channel.stream-dual-buffer]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_stream_rx_avail_le_capacity(
            cap in 2usize..32,
            payload_len in 1usize..8,
            send_count in 1usize..6,
        ) {
            core_exec::block_on(async {
                let mut graph = GraphBuilder::for_testing().build(());
                let (tx, rx) = graph.channel_builder()
                    .with_capacity(cap)
                    .build_stream::<StreamEgress>(cap * 16);
                let tx_clone = tx.clone();
                let mut tx_guard = tx_clone.lock().await;
                for _ in 0..send_count.min(cap) {
                    let payload = vec![0u8; payload_len];
                    if tx_guard.shared_try_send(payload.as_slice()).is_err() {
                        break;
                    }
                }
                drop(tx_guard);
                let rx_clone = rx.clone();
                let mut rx_guard = rx_clone.lock().await;
                let (ctrl_avail, payload_avail) = rx_guard.shared_avail_units();
                let (ctrl_cap, payload_cap) = rx_guard.shared_capacity();
                prop_assert!(ctrl_avail <= ctrl_cap);
                prop_assert!(payload_avail <= payload_cap);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: stream rx peek leaves avail unchanged.
        #[test]
        // ss[verify philosophy.zero-copy-discipline]
        // ss[verify channel.stream-dual-buffer]
        // ss[verify verify.process.proptest]
        fn proptest_stream_rx_peek_preserves_avail(
            cap in 2usize..16,
            payload_len in 1usize..8,
        ) {
            core_exec::block_on(async {
                let mut graph = GraphBuilder::for_testing().build(());
                let (tx, rx) = graph.channel_builder()
                    .with_capacity(cap)
                    .build_stream::<StreamEgress>(cap * 16);
                let tx_clone = tx.clone();
                let mut tx_guard = tx_clone.lock().await;
                tx_guard.shared_try_send(&vec![0u8; payload_len]).unwrap();
                drop(tx_guard);
                let rx_clone = rx.clone();
                let mut rx_guard = rx_clone.lock().await;
                let avail_before = rx_guard.shared_avail_units();
                let _ = rx_guard
                    .shared_peek_async_timeout(Some(Duration::from_millis(1)))
                    .await;
                prop_assert_eq!(rx_guard.shared_avail_units(), avail_before);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: stream rx advance_index on empty channel returns zero.
        #[test]
        // ss[verify channel.stream-dual-buffer]
        // ss[verify verify.process.proptest]
        fn proptest_stream_rx_advance_empty_zero(
            cap in capacity(),
            overshoot in 1usize..16,
        ) {
            core_exec::block_on(async {
                let mut graph = GraphBuilder::for_testing().build(());
                let (_tx, rx) = graph.channel_builder()
                    .with_capacity(cap)
                    .build_stream::<StreamIngress>(cap * 8);
                let rx_clone = rx.clone();
                let mut rx_guard = rx_clone.lock().await;
                let done = rx_guard.shared_advance_index((overshoot, overshoot * 8));
                prop_assert_eq!(done, RxDone::Stream(0, 0));
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: stream egress sent payload bytes equal received bytes.
        #[test]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify channel.stream-dual-buffer]
        // ss[verify verify.process.proptest]
        fn proptest_stream_egress_no_silent_drop(
            cap in 2usize..16,
            payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..8), 1..4),
        ) {
            core_exec::block_on(async {
                let mut graph = GraphBuilder::for_testing().build(());
                let (tx, rx) = graph.channel_builder()
                    .with_capacity(cap)
                    .build_stream::<StreamEgress>(cap * 32);
                let tx_clone = tx.clone();
                let mut tx_guard = tx_clone.lock().await;
                let mut sent_bytes = 0usize;
                for p in &payloads {
                    if tx_guard.shared_try_send(p.as_slice()).is_ok() {
                        sent_bytes += p.len();
                    } else {
                        break;
                    }
                }
                drop(tx_guard);
                let rx_clone = rx.clone();
                let mut rx_guard = rx_clone.lock().await;
                let mut taken_bytes = 0usize;
                while let Some((done, (_item, payload))) = rx_guard.shared_try_take() {
                    let payload: &Box<[u8]> = &payload;
                    if let RxDone::Stream(_, b) = done {
                        prop_assert_eq!(b, payload.len());
                        taken_bytes += b;
                    }
                }
                prop_assert_eq!(taken_bytes, sent_bytes);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: stream rx advance_index never exceeds available control/payload units.
        #[test]
        // ss[verify channel.stream-dual-buffer]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_stream_rx_advance_index_bounded(
            cap in 2usize..16,
            payload_len in 1usize..8,
            overshoot in 1usize..16,
        ) {
            core_exec::block_on(async {
                let mut graph = GraphBuilder::for_testing().build(());
                let (tx, rx) = graph.channel_builder()
                    .with_capacity(cap)
                    .build_stream::<StreamEgress>(cap * 16);
                let tx_clone = tx.clone();
                let mut tx_guard = tx_clone.lock().await;
                tx_guard.shared_try_send(&vec![0u8; payload_len]).unwrap();
                drop(tx_guard);
                let rx_clone = rx.clone();
                let mut rx_guard = rx_clone.lock().await;
                let (ctrl_avail, payload_avail) = rx_guard.shared_avail_units();
                let done = rx_guard.shared_advance_index((
                    ctrl_avail + overshoot,
                    payload_avail + overshoot * payload_len,
                ));
                if let RxDone::Stream(ctrl, payload) = done {
                    prop_assert!(ctrl <= ctrl_avail);
                    prop_assert!(payload <= payload_avail);
                }
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }
    }
}
