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
            crate::core_rx::note_peek_repeat(
                &self.control_channel.peek_repeats,
                &self.control_channel.take_count,
                &self.control_channel.cached_take_count,
                true,
            );
            let (a, b) = self.payload_channel.rx.as_slices();
            let count_a = a.len().min(item.length() as usize);
            let count_b = item.length() as usize - count_a;
            Some((item, &a[0..count_a], &b[0..count_b]))
        } else {
            crate::core_rx::note_peek_repeat(
                &self.control_channel.peek_repeats,
                &self.control_channel.take_count,
                &self.control_channel.cached_take_count,
                false,
            );
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
        if n != 0 {
            payload_target[..n].copy_from_slice(&payload_a[..n]);
            payload_copied += n;
        }
        if payload_copied != payload_bytes_needed {
            let m = payload_b.len().min(payload_bytes_needed - payload_copied);
            if m != 0 {
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
#[path = "core_rx_stream_tests.rs"]
mod core_rx_stream_tests;
