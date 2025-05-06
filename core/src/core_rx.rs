use log::warn;
use futures_util::select;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use ringbuf::traits::Observer;
use futures_util::future::FusedFuture;
use async_ringbuf::consumer::AsyncConsumer;
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::{steady_config, Rx, MONITOR_NOT};
use crate::distributed::distributed_stream::{StreamItem, StreamRx};
use crate::steady_rx::{RxDone};
use futures_util::{FutureExt};
use crate::yield_now;

pub trait RxCore {
    type MsgOut;
    type MsgPeek<'a> where Self: 'a;
    type MsgSize: Copy;

    #[allow(async_fn_in_trait)]
    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>>;

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>);

    fn monitor_not(&mut self);

    fn shared_capacity(&self) -> usize;

    fn log_perodic(&mut self) -> bool;

    fn shared_is_empty(&self) -> bool;

    fn shared_avail_units(&mut self) -> usize;

    #[allow(async_fn_in_trait)]
    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool;

    #[allow(async_fn_in_trait)]
    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool;

    #[allow(async_fn_in_trait)]
    async fn shared_wait_avail_units(&mut self, count: usize) -> bool;

    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)>;

    fn shared_advance_index(&mut self, count: Self::MsgSize) -> Self::MsgSize;

}

impl <T>RxCore for Rx<T> {

    type MsgOut = T;
    type MsgPeek<'a> = &'a T where T: 'a;
    type MsgSize = usize;

    fn shared_advance_index(&mut self, count: Self::MsgSize) -> Self::MsgSize {
        let avail = self.rx.occupied_len();
        let idx = if count>avail {
            avail
        } else {
            count
        };
        unsafe { self.rx.advance_read_index(idx); }
        idx
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

    fn log_perodic(&mut self) -> bool {
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
        self.rx.occupied_len()
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
        if self.rx.occupied_len() >= count {
            true
        } else {
            let operation = &mut self.rx.wait_occupied(count);
            operation.await;
            true
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


}

impl <T: StreamItem> RxCore for StreamRx<T> {

    //TODO: we have no shared_try_peek ???

    type MsgOut = (T, Box<[u8]>);
    type MsgPeek<'a> = (&'a T, &'a[u8],&'a[u8]) where T: 'a;
    type MsgSize = (usize, usize);
 
   fn shared_advance_index(&mut self, count: Self::MsgSize) -> Self::MsgSize {

       let avail = self.payload_channel.rx.occupied_len();
       let payload_step = if count.1>avail {
           avail
       } else {
           count.1
       };
       unsafe { self.payload_channel.rx.advance_read_index(payload_step); }

       let avail = self.item_channel.rx.occupied_len();
        let index_step = if count.0>avail {
            avail
        } else {
            count.0
        };
        unsafe { self.item_channel.rx.advance_read_index(index_step); }

       (index_step, payload_step)
    }

    async fn shared_peek_async_timeout<'a>(&'a mut self, timeout: Option<Duration>)
                        -> Option<Self::MsgPeek<'a>> {
        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.wait_occupied(1);
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
        let result = self.item_channel.rx.first();
        if let Some(item) = result {
            let take_count = self.item_channel.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.item_channel.cached_take_count.load(Ordering::Relaxed);
            if !cached_take_count == take_count {
                self.item_channel.peek_repeats.store(0, Ordering::Relaxed);
                self.item_channel.cached_take_count.store(take_count, Ordering::Relaxed);
            } else {
                self.item_channel.peek_repeats.fetch_add(1, Ordering::Relaxed);
            }
            let (a,b) = self.payload_channel.rx.as_slices();
            let count_a = a.len().min(item.length() as usize);
            let count_b  = item.length() as usize - count_a;
            Some((item, &a[0..count_a], &b[0..count_b]))

        } else {
            self.item_channel.peek_repeats.store(0, Ordering::Relaxed);
            None
        }
    }

    fn log_perodic(&mut self) -> bool {
        if self.item_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.item_channel.last_error_send = Instant::now();
            true
        }
    }

    #[inline]
    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            RxDone::Normal(i) => {
                self.item_channel.local_monitor_index = tel.process_event(self.item_channel.local_monitor_index, self.item_channel.channel_meta_data.meta_data.id, i as isize);
                warn!("internal error should have gotten Stream")},
            RxDone::Stream(i,p) => {
                self.item_channel.local_monitor_index = tel.process_event(self.item_channel.local_monitor_index, self.item_channel.channel_meta_data.meta_data.id, i as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.channel_meta_data.meta_data.id, p as isize);
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.item_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.item_channel.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool  {
        self.item_channel.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> usize {
        self.item_channel.rx.occupied_len()
    }


    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {

        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.item_channel.rx.occupied_len() >= count
        }

    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        if self.item_channel.rx.occupied_len() >= count {
            true
        } else {//we always return true if we have count regardless of the shutdown status
            let mut one_closed = &mut self.item_channel.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.item_channel.rx.wait_occupied(count);
                select! { _ = one_closed => self.item_channel.rx.occupied_len() >= count, _ = operation => true }
            } else {
                yield_now::yield_now().await; //this is a big help in closed shutdown tight loop
                self.item_channel.rx.occupied_len() >= count // if closed, we can still take
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        if self.item_channel.rx.occupied_len() >= count {
            true
        } else {
            let operation = &mut self.item_channel.rx.wait_occupied(count);
            operation.await;
            true
        }
    }


    #[inline]
    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)> {
        if let Some(item) = self.item_channel.rx.try_peek() {
            if item.length() <= self.payload_channel.rx.occupied_len() as i32 {

                let mut payload = vec![0u8; item.length() as usize];
                self.payload_channel.rx.peek_slice(&mut payload);
                let payload = payload.into_boxed_slice();

                if let Some(item) = self.item_channel.rx.try_pop() {
                    unsafe { self.payload_channel.rx.advance_read_index(payload.len()); }
                    self.item_channel.take_count.fetch_add(1,Ordering::Relaxed); //for DLQ!
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

}

impl<T: RxCore> RxCore for futures_util::lock::MutexGuard<'_, T> {
    type MsgOut = <T as RxCore>::MsgOut;
    type MsgPeek<'a> =  <T as RxCore>::MsgPeek<'a> where Self: 'a;
    type MsgSize = <T as RxCore>::MsgSize;
 
    async fn shared_peek_async_timeout<'a>(&'a mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'a>> {
        <T as RxCore>::shared_peek_async_timeout(& mut **self
                                                 ,timeout).await
    }

    fn log_perodic(&mut self) -> bool {
        <T as RxCore>::log_perodic(&mut **self)
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

    fn shared_advance_index(&mut self, count: Self::MsgSize) -> Self::MsgSize {
        <T as RxCore>::shared_advance_index(&mut **self, count)
    }
}