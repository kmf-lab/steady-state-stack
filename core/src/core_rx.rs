use log::*;
use futures_util::{select, task};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use ringbuf::traits::Observer;
use futures_util::future::FusedFuture;
use async_ringbuf::consumer::AsyncConsumer;
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::{steady_config, Rx, MONITOR_NOT};
use crate::distributed::distributed_stream::{StreamControlItem, StreamRx};
use crate::steady_rx::{RxDone};
use futures_util::{FutureExt};
use crate::yield_now;


pub trait DoubleSlice<'a, T: 'a> {
    fn as_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a>;
    fn to_vec(&self) -> Vec<T> where T: Clone;
    fn total_len(&self) -> usize;

}

impl<'a, T: 'a> DoubleSlice<'a, T> for (&'a [T], &'a [T]) {
    fn as_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a> {
        Box::new(self.0.iter().chain(self.1.iter()))
    }
    fn to_vec(&self) -> Vec<T> where T: Clone {
        let mut v = Vec::with_capacity(self.0.len() + self.1.len());
        v.extend_from_slice(self.0);
        v.extend_from_slice(self.1);
        v
    }
    fn total_len(&self) -> usize {
        self.0.len() + self.1.len()
    }
}
pub trait DoubleSliceCopy<'a, T: 'a> {
    fn copy_into_slice(&self, target: &mut [T]) -> RxDone
    where
        T: Copy;
}

impl<'a, T: 'a> DoubleSliceCopy<'a, T> for (&'a [T], &'a [T]) {
    fn copy_into_slice(&self, target: &mut [T]) -> RxDone
    where
        T: Copy,
    {
        let (a, b) = *self;
        let mut copied = 0;

        let n = a.len().min(target.len());
        if n > 0 {
            target[..n].copy_from_slice(&a[..n]);
            copied += n;
        }

        if copied < target.len() {
            let m = b.len().min(target.len() - copied);
            if m > 0 {
                target[copied..copied + m].copy_from_slice(&b[..m]);
                copied += m;
            }
        }

        RxDone::Normal(copied)
    }
}

pub trait QuadSlice<'a, T: 'a, U: 'a> {
    fn items_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a>;
    fn payload_iter(&'a self) -> Box<dyn Iterator<Item = &'a U> + 'a>;
    fn items_vec(&self) -> Vec<T> where T: Clone;
    fn payload_vec(&self) -> Vec<U> where U: Clone;
    fn items_len(&self) -> usize;
    fn payload_len(&self) -> usize;
}

impl<'a, T: 'a, U: 'a> QuadSlice<'a, T, U> for (&'a [T], &'a [T], &'a [U], &'a [U]) {
    fn items_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a> {
        Box::new(self.0.iter().chain(self.1.iter()))
    }
    fn payload_iter(&'a self) -> Box<dyn Iterator<Item = &'a U> + 'a> {
        Box::new(self.2.iter().chain(self.3.iter()))
    }
    fn items_vec(&self) -> Vec<T> where T: Clone {
        let mut v = Vec::with_capacity(self.0.len() + self.1.len());
        v.extend_from_slice(self.0);
        v.extend_from_slice(self.1);
        v
    }
    fn payload_vec(&self) -> Vec<U> where U: Clone {
        let mut v = Vec::with_capacity(self.2.len() + self.3.len());
        v.extend_from_slice(self.2);
        v.extend_from_slice(self.3);
        v
    }
    fn items_len(&self) -> usize {
        self.0.len() + self.1.len()
    }
    fn payload_len(&self) -> usize {
        self.2.len() + self.3.len()
    }
}
pub trait StreamQuadSliceCopy<'a, T: StreamControlItem> {
    /// Copies as many items and their payloads as will fit into the targets.
    /// Returns (items_copied, bytes_copied).
    fn copy_items_and_payloads(
        &self,
        item_target: &mut [T],
        payload_target: &mut [u8],
    ) -> (usize, usize)
    where
        T: Copy;
}

impl<'a, T: StreamControlItem> StreamQuadSliceCopy<'a, T> for (&'a [T], &'a [T], &'a [u8], &'a [u8]) {
    fn copy_items_and_payloads(
        &self,
        item_target: &mut [T],
        payload_target: &mut [u8],
    ) -> (usize, usize)
    where
        T: Copy,
    {
        let (a, b, c, d) = self;

        // Flatten the items into a single iterator
        let items_iter = a.iter().chain(b.iter());

        let mut items_copied = 0;
        let mut bytes_needed = 0;

        // Figure out how many items and bytes we can fit
        for item in items_iter.clone() {
            let item_len = item.length() as usize;
            if items_copied < item_target.len() && bytes_needed + item_len <= payload_target.len() {
                items_copied += 1;
                bytes_needed += item_len;
            } else {
                break;
            }
        }

        // Copy the items
        let mut copied = 0;
        for item in items_iter.take(items_copied) {
            item_target[copied] = *item;
            copied += 1;
        }

        // Copy the payload bytes
        let mut payload_copied = 0;
        let n = c.len().min(bytes_needed);
        if n > 0 {
            payload_target[..n].copy_from_slice(&c[..n]);
            payload_copied += n;
        }
        if payload_copied < bytes_needed {
            let m = d.len().min(bytes_needed - payload_copied);
            if m > 0 {
                payload_target[payload_copied..payload_copied + m].copy_from_slice(&d[..m]);
                payload_copied += m;
            }
        }

        (items_copied, payload_copied)
    }
}



pub trait RxCore {
    type MsgItem;
    type MsgOut;
    type MsgPeek<'a> where Self: 'a;
    type MsgSize: Copy;
    type SliceSource<'a> where Self: 'a;  // source is for zero copy (inverse of Tx)
    type SliceTarget<'b> where Self::MsgOut: 'b; // target is where to copy data into on take


    fn shared_take_slice(& mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy; // with target   arg_in     (&[T], &[u8])
    fn shared_peek_slice(& mut self) -> Self::SliceSource<'_>; //  for zero copy result_out (&[T], &[T], &[u8], &[u8])


    #[allow(async_fn_in_trait)]
    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>>;

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>);

    fn monitor_not(&mut self);

    fn shared_capacity(&self) -> usize;

    fn log_periodic(&mut self) -> bool;

    fn shared_is_empty(&self) -> bool;

    fn shared_avail_units(&mut self) -> usize;

    #[allow(async_fn_in_trait)]
    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool;

    #[allow(async_fn_in_trait)]
    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool;

    #[allow(async_fn_in_trait)]
    async fn shared_wait_avail_units(&mut self, count: usize) -> bool;

    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)>;

    /// returns how many we advanced which may be smaller than requested
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> RxDone;

    fn is_closed_and_empty(&mut self) -> bool;
}

impl <T>RxCore for Rx<T> {
    type MsgItem = T;
    type MsgOut = T;
    type MsgPeek<'a> = &'a T where T: 'a;
    type MsgSize = usize;

    type SliceSource<'a> = (&'a [T],&'a [T]) where Self: 'a;  // source is for zero copy (inverse of Tx)
    type SliceTarget<'b> = & 'b mut [T] where T: 'b ;  // target is where to copy data into on take


    fn is_closed_and_empty(&mut self) -> bool {
        if self.is_closed.is_terminated() {
            self.shared_is_empty()
        } else if self.shared_is_empty() {
            let waker = task::noop_waker();
            let mut context = task::Context::from_waker(&waker);
            self.is_closed.poll_unpin(&mut context).is_ready()
        } else {
            false
        }
    }

    /// returns count of how many positions we advanced
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> RxDone {
        let avail = self.rx.occupied_len();
        let idx = if request>avail {
            avail
        } else {
            request
        };
        unsafe { self.rx.advance_read_index(idx); }
        RxDone::Normal(idx)
    }

    async fn shared_peek_async_timeout<'a>(&'a mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'a>> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(1);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down => {}
                        , _ = operation => {}
                        , _ = timeout => {}
                };
            } else {
                select! { _ = one_down => {}
                        , _ = operation => {}
                };
            }
        }
        let result = self.rx.first();
        if result.is_some() {
            let take_count = self.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.cached_take_count.load(Ordering::Relaxed);
            if !cached_take_count == take_count {
                self.peek_repeats.store(0, Ordering::Relaxed);
                self.cached_take_count.store(take_count, Ordering::Relaxed);
            } else {
                self.peek_repeats.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            self.peek_repeats.store(0, Ordering::Relaxed);
        }
        result
    }

    fn log_periodic(&mut self) -> bool {
        if self.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.last_error_send = Instant::now();
            true
        }
    }

    #[inline]
    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        self.local_monitor_index = match done_count {
            RxDone::Normal(d) =>
                tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, d as isize),
            RxDone::Stream(i,_p) => {
                warn!("internal error should have gotten Normal");
                tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, i as isize)
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.local_monitor_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool  {
        self.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> usize {
        //self.rx.occupied_len()
        let capacity = self.rx.capacity().get();
        let modulus = 2 * capacity;

        // Read write_index FIRST, then read_index for consumer floor guarantee
        let write_idx = self.rx.write_index();
        let read_idx = self.rx.read_index();

        let result =  (modulus + write_idx - read_idx) % modulus;
        assert!(result<=capacity);

        result
    }





    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {

        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.rx.occupied_len() >= count
        }

    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        if self.rx.occupied_len() >= count {
            true
        } else {//we always return true if we have count regardless of the shutdown status
            let mut one_closed = &mut self.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.rx.wait_occupied(count);
                select! { _ = one_closed => self.rx.occupied_len() >= count, _ = operation => true }
            } else {
                yield_now::yield_now().await; //this is a big help in closed shutdown tight loop
                self.rx.occupied_len() >= count // if closed, we can still take
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;

        if self.rx.occupied_len() >= count {
            true
        } else if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            false
        }
    }

    #[inline]
    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        match self.rx.try_pop() {
            Some(r) => {
                self.take_count.fetch_add(1,Ordering::Relaxed); //for DLQ
                Some((RxDone::Normal(1), r))
            },
            None => None
        }
    }

    fn shared_take_slice(& mut self, target: Self::SliceTarget<'_>) -> RxDone
      where Self::MsgItem : Copy
    {
        // Get the available slices from the ring buffer
        let (a, b) = self.rx.as_slices();
        let mut copied = 0;

        // Copy from the first slice
        let n = a.len().min(target.len());
        if n > 0 {
            // SAFETY: target is &[T], so we can't mutate it, but the trait
            // probably should be &mut [T] for this to make sense.
            // If you meant to copy into a mutable slice, change the trait to &mut [T].
            // For now, let's assume it's a typo and use &mut [T].
            let target = unsafe { &mut *(target as *const [T] as *mut [T]) };
            target[..n].copy_from_slice(&a[..n]);
            copied += n;
        }

        // Copy from the second slice if there's still space
        if copied < target.len() {
            let m = b.len().min(target.len() - copied);
            if m > 0 {
                let target = unsafe { &mut *(target as *const [T] as *mut [T]) };
                target[copied..copied + m].copy_from_slice(&b[..m]);
                copied += m;
            }
        }

        // Advance the read index by the number of items copied
        unsafe { self.rx.advance_read_index(copied); }

        if copied>0 { //mut be done on every possible take method
            self.take_count.fetch_add(1,Ordering::Relaxed); //for DLQ
        }

        RxDone::Normal(copied)
    }

    fn shared_peek_slice(& mut self ) -> Self::SliceSource<'_> {
        // Get the available slices from the ring buffer
        self.rx.as_slices()
    }
}

impl <T: StreamControlItem> RxCore for StreamRx<T> {

    type MsgItem =T;
    type MsgOut = (T, Box<[u8]>);
    type MsgPeek<'a> = (&'a T, &'a[u8],&'a[u8]) where T: 'a;
    type MsgSize = (usize, usize);
    type SliceSource<'a> = (&'a [T], &'a [T], &'a[u8], &'a[u8] ) where T: 'a;  // source is for zero copy (inverse of Tx)
    type SliceTarget<'b> = (&'b mut [T],&'b mut [u8]) where T: 'b;  // target is where to copy data into on take

    // shared_peek_slice for zero copy result_out (&[T], &[T], &[u8], &[u8])
    // shared_take_slice with target   arg_in     (&[T], &[u8])


    fn is_closed_and_empty(&mut self) -> bool {
        //debug!("closed_empty {} {}", self.item_channel.is_closed_and_empty(), self.payload_channel.is_closed_and_empty());
        self.control_channel.is_closed_and_empty()
            && self.payload_channel.is_closed_and_empty()
    }

   fn shared_advance_index(&mut self, count: Self::MsgSize) -> RxDone {

       let control_avail = self.control_channel.rx.occupied_len();
       let payload_avail = self.payload_channel.rx.occupied_len();
       //NOTE: we only have two counts so either we do all or none because we can
       //      not compute a safe cut point. so it is all or nothing

       if count.0 <= control_avail && count.1 <= payload_avail {
           unsafe {
               self.payload_channel.rx.advance_read_index(count.1);
               self.control_channel.rx.advance_read_index(count.0);
           }
           RxDone::Stream(count.0,count.1)
       } else {
           RxDone::Stream(0,0)
       }
    }

    async fn shared_peek_async_timeout<'a>(&'a mut self, timeout: Option<Duration>)
                        -> Option<Self::MsgPeek<'a>> {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.rx.wait_occupied(1);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down => {}
                        , _ = operation => {}
                        , _ = timeout => {}
                };
            } else {
                select! { _ = one_down => {}
                        , _ = operation => {}
                };
            }
        }
        let result = self.control_channel.rx.first();
        if let Some(item) = result {
            let take_count = self.control_channel.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.control_channel.cached_take_count.load(Ordering::Relaxed);
            if !cached_take_count == take_count {
                self.control_channel.peek_repeats.store(0, Ordering::Relaxed);
                self.control_channel.cached_take_count.store(take_count, Ordering::Relaxed);
            } else {
                self.control_channel.peek_repeats.fetch_add(1, Ordering::Relaxed);
            }
            let (a,b) = self.payload_channel.rx.as_slices();
            let count_a = a.len().min(item.length() as usize);
            let count_b  = item.length() as usize - count_a;
            Some((item, &a[0..count_a], &b[0..count_b]))

        } else {
            self.control_channel.peek_repeats.store(0, Ordering::Relaxed);
            None
        }
    }

    fn log_periodic(&mut self) -> bool {
        if self.control_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.control_channel.last_error_send = Instant::now();
            true
        }
    }

    #[inline]
    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            RxDone::Normal(i) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
            }
            RxDone::Stream(i,p) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.channel_meta_data.meta_data.id, p as isize);
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.control_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.control_channel.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool  {
        self.control_channel.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> usize {
        self.control_channel.rx.occupied_len()
    }


    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {

        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.control_channel.rx.occupied_len() >= count
        }

    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        if self.control_channel.rx.occupied_len() >= count {
            true
        } else {//we always return true if we have count regardless of the shutdown status
            let mut one_closed = &mut self.control_channel.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.control_channel.rx.wait_occupied(count);
                select! { _ = one_closed => self.control_channel.rx.occupied_len() >= count, _ = operation => true }
            } else {
                yield_now::yield_now().await; //this is a big help in closed shutdown tight loop
                self.control_channel.rx.occupied_len() >= count // if closed, we can still take
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        if self.control_channel.rx.occupied_len() >= count {
            true
        } else {
            let operation = &mut self.control_channel.rx.wait_occupied(count);
            operation.await;
            true
        }
    }


    #[inline]
    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)> {
        if let Some(item) = self.control_channel.rx.try_peek() {
            if item.length() <= self.payload_channel.rx.occupied_len() as i32 {

                let mut payload = vec![0u8; item.length() as usize];
                self.payload_channel.rx.peek_slice(&mut payload);
                let payload = payload.into_boxed_slice();

                if let Some(item) = self.control_channel.rx.try_pop() {
                    unsafe { self.payload_channel.rx.advance_read_index(payload.len()); }
                    self.control_channel.take_count.fetch_add(1, Ordering::Relaxed); //for DLQ!
                    Some( (RxDone::Stream(1,payload.len()),(item,payload)) )
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

    fn shared_take_slice<'a>(
        &'a mut self,
        target: Self::SliceTarget<'a>,
    ) -> RxDone where Self::MsgItem: Copy
    {
        let (item_target, payload_target) = target;

        // Get available item slices
        let (item_a, item_b) = self.control_channel.rx.as_slices();


        let mut items_copied = 0;
        let mut payload_bytes_needed = 0;

        // We need to keep track of how many items and bytes we can fit
        let max_items = item_target.len();
        let max_payload = payload_target.len();

        // We'll copy items into item_target, and sum up the payload bytes needed
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

        // Now copy the payload bytes
        let (payload_a, payload_b) = self.payload_channel.rx.as_slices();
        let mut payload_copied = 0;

        // Copy from payload_a
        let n = payload_a.len().min(payload_bytes_needed);
        if n > 0 {
            payload_target[..n].copy_from_slice(&payload_a[..n]);
            payload_copied += n;
        }
        // Copy from payload_b if needed
        if payload_copied < payload_bytes_needed {
            let m = payload_b.len().min(payload_bytes_needed - payload_copied);
            if m > 0 {
                payload_target[payload_copied..payload_copied + m]
                    .copy_from_slice(&payload_b[..m]);
                payload_copied += m;
            }
        }

        // Advance both read indices
        unsafe { //payload first
            self.payload_channel.rx.advance_read_index(payload_copied);
            self.control_channel.rx.advance_read_index(items_copied);
        }


        // if items_copied>0 { //TODO: mut be done on every possible take method confirm we have done it here.
        //     self.control_channel.fetch_add(1,Ordering::Relaxed); //for DLQ
        //     self.payload_channel.fetch_add(1,Ordering::Relaxed); //for DLQ
        //
        // }


        RxDone::Stream(items_copied, payload_copied)
    }

    fn shared_peek_slice<'a>(&'a mut self) -> Self::SliceSource<'a> {
        let (item_a, item_b) = self.control_channel.rx.as_slices();
        let (payload_a, payload_b) = self.payload_channel.rx.as_slices();
        (item_a, item_b, payload_a, payload_b)
    }
}

impl<T: RxCore> RxCore for futures_util::lock::MutexGuard<'_, T> {
    type MsgItem = <T as RxCore>::MsgItem;
    type MsgOut = <T as RxCore>::MsgOut;
    type MsgPeek<'a> =  <T as RxCore>::MsgPeek<'a> where Self: 'a;
    type MsgSize = <T as RxCore>::MsgSize;
    type SliceSource<'a> = <T as RxCore>::SliceSource<'a> where Self: 'a;
    type SliceTarget<'b> = <T as RxCore>::SliceTarget<'b> where Self::MsgOut: 'b;

    fn is_closed_and_empty(&mut self) -> bool {
        <T as RxCore>::is_closed_and_empty(&mut **self)
    }

    async fn shared_peek_async_timeout<'a>(&'a mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'a>> {
        <T as RxCore>::shared_peek_async_timeout(& mut **self
                                                 ,timeout).await
    }

    fn log_periodic(&mut self) -> bool {
        <T as RxCore>::log_periodic(&mut **self)
    }

    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        <T as RxCore>::telemetry_inc(&mut **self,done_count, tel)
    }

    fn monitor_not(&mut self) {
        <T as RxCore>::monitor_not(&mut **self)
    }

    fn shared_capacity(&self) -> usize {
        <T as RxCore>::shared_capacity(&**self)
    }

    fn shared_is_empty(&self) -> bool {
        <T as RxCore>::shared_is_empty(&**self)
    }

    fn shared_avail_units(&mut self) -> usize {
        <T as RxCore>::shared_avail_units(&mut **self)
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {
        <T as RxCore>::shared_wait_shutdown_or_avail_units(&mut **self, count).await
    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        <T as RxCore>::shared_wait_closed_or_avail_units(&mut **self, count).await
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        <T as RxCore>::shared_wait_avail_units(&mut **self, count).await
    }

    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)> {
        <T as RxCore>::shared_try_take(&mut **self)
    }

    fn shared_advance_index(&mut self, count: Self::MsgSize) -> RxDone {
        <T as RxCore>::shared_advance_index(&mut **self, count)
    }

    fn shared_take_slice(& mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy {
        <T as RxCore>::shared_take_slice(&mut **self, target)
    }

    fn shared_peek_slice<'a>(&'a mut self) -> Self::SliceSource<'a> {
        <T as RxCore>::shared_peek_slice(&mut **self)
    }
}

#[cfg(test)]
mod core_rx_async_tests {
    use super::*;
    use crate::steady_rx::{Rx};
    use crate::steady_tx::Tx;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use futures::lock::Mutex;
    use crate::core_tx::TxCore;
    use crate::{ActorIdentity, SendSaturation};
    use crate::*;

    /// Helper function to set up a channel for tests.
    ///
    /// - `capacity`: The capacity of the channel.
    /// - `data`: Optional initial data to populate the channel.
    /// - Returns: A tuple of `(Tx, Rx)` wrapped in `Arc<Mutex<...>>` for shared access.
    fn setup_channel<T: Clone + Send + 'static>(
        capacity: usize,
        data: Option<Vec<T>>,
    ) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>, Graph) {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = graph.channel_builder();
        let builder = builder.with_capacity(capacity);
        let (tx_lazy, rx_lazy) = builder.build_channel::<T>();
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        if let Some(values) = data {
            core_exec::block_on(async {
                let mut tx_guard = tx.lock().await;
                for value in values {
                    let _ = tx_guard.shared_send_async(value, ActorIdentity::default(), SendSaturation::default()).await;
                }
            });
        }
        graph.start();
        (tx, rx, graph)
    }

    #[test]
    fn test_peek_async_timeout_empty() {
        let (_tx, rx, graph) = setup_channel::<i32>(1, None);
        assert!(graph.runtime_state.read().is_in_state(&[GraphLivelinessState::Running]), "Graph should be Running");
        let mut rx_guard = core_exec::block_on(rx.lock());
        assert!(!rx_guard.oneshot_shutdown.is_terminated() );

        let start = Instant::now();
        let peeked = core_exec::block_on(rx_guard.shared_peek_async_timeout(Some(Duration::from_millis(120))));

        assert!(peeked.is_none(), "Peek should return None on an empty channel after timeout or shutdown");
        eprintln!("timeout {:?}",start.elapsed());
        assert!(start.elapsed() >= Duration::from_millis(100), "Timeout duration should be at least 100ms");
    }

    #[test]
    fn test_peek_async_timeout_with_data() {
        let (_tx, rx, _) = setup_channel(1, Some(vec![42]));
        let mut rx_guard = core_exec::block_on(rx.lock());
        let peeked = core_exec::block_on(rx_guard.shared_peek_async_timeout(None));
        assert_eq!(peeked, Some(&42), "Peek should return the available data");
    }

    #[test]
    fn test_peek_async_timeout_shutdown() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let peek_future = rx_guard.shared_peek_async_timeout(Some(Duration::from_secs(1)));
        drop(core_exec::block_on(tx.lock())); // Simulate shutdown by dropping Tx guard
        let peeked = core_exec::block_on(peek_future);
        assert!(peeked.is_none(), "Peek should return None after shutdown");
    }

    #[test]
    fn test_wait_avail_units() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let wait_future = rx_guard.shared_wait_avail_units(1);
        core_exec::block_on(async {
            let mut tx_guard = tx.lock().await;
            tx_guard.shared_send_async(42,ActorIdentity::default(), SendSaturation::default()).await;
        });
        assert!(core_exec::block_on(wait_future), "Wait should return true when units become available");
    }

    #[test]
    fn test_wait_shutdown_or_avail_units() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let wait_future = rx_guard.shared_wait_shutdown_or_avail_units(1);
        drop(core_exec::block_on(tx.lock())); // Simulate shutdown by dropping Tx guard
        assert!(!core_exec::block_on(wait_future), "Wait should return false on shutdown");
    }

    #[test]
    fn test_wait_closed_or_avail_units() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let wait_future = rx_guard.shared_wait_closed_or_avail_units(1);
        core_exec::block_on(async {
            let mut tx_guard = tx.lock().await;
            tx_guard.mark_closed();
        });
        let result = core_exec::block_on(wait_future);
        assert!(!result, "Wait should return false when channel is closed with no units available");
    }
}
