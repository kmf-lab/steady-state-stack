use log::warn;
use futures_util::select;
use std::sync::atomic::Ordering;
use std::time::Instant;
use ringbuf::traits::Observer;
use futures_util::future::FusedFuture;
use async_ringbuf::consumer::AsyncConsumer;
use ringbuf::consumer::Consumer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::{steady_config, Rx, MONITOR_NOT};
use crate::distributed::distributed_stream::{StreamItem, StreamRx};
use crate::steady_rx::RxDone;

pub trait RxCore {
    type MsgOut;


    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>);

    fn monitor_not(&mut self);

    fn shared_capacity(&self) -> usize;

    fn log_perodic(&mut self) -> bool;

    fn shared_is_empty(&self) -> bool;

    fn shared_avail_units(&mut self) -> usize;

    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool;

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool;

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool;

    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)>;

}

impl <T>RxCore for Rx<T> {

    type MsgOut = T;


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
                tel.process_event(self.local_monitor_index, self.channel_meta_data.id, d as isize),
            RxDone::Stream(i,p) => {
                warn!("internal error should have gotten Normal");
                tel.process_event(self.local_monitor_index, self.channel_meta_data.id, i as isize)
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

    type MsgOut = (T, Box<[u8]>);


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
            RxDone::Normal(d) =>
                warn!("internal error should have gotten Stream"),
            RxDone::Stream(i,p) => {
                self.item_channel.local_monitor_index = tel.process_event(self.item_channel.local_monitor_index, self.item_channel.channel_meta_data.id, i as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.channel_meta_data.id, p as isize);
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

                drop(item);
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
}