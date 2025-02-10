use log::warn;
use futures_util::select;
use std::sync::atomic::Ordering;
use std::time::Duration;
use ringbuf::traits::Observer;
use futures_util::future::FusedFuture;
use async_ringbuf::consumer::AsyncConsumer;
use bytes::BufMut;
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use crate::distributed::steady_stream::{StreamItem, StreamRx};
use crate::{Rx, MONITOR_NOT};
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::steady_rx::CountingIterator;
use futures_util::{FutureExt};


pub trait RxCore {

    type MsgOut;
    type MsgSize;



    async fn shared_take_async_timeout(&mut self, timeout: Option<Duration> ) -> Option<(RxDone, Self::MsgOut)>;

    async fn shared_take_async(&mut self) -> Option<(RxDone, Self::MsgOut)>;

    async fn shared_peek_async_iter_timeout(&mut self, wait_for_count: usize, timeout: Option<Duration>) -> impl Iterator<Item = &Self::MsgOut>;

    fn shared_take_into_iter(&mut self) -> impl Iterator<Item = Self::MsgOut>;

    fn shared_try_peek_iter(&self) -> impl Iterator<Item = &Self::MsgOut>;

    async fn shared_peek_async_iter(&mut self, wait_for_count: usize) -> impl Iterator<Item = &Self::MsgOut>;

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>);

    fn monitor_not(&mut self);

    fn shared_capacity(&self) -> usize;

    fn shared_is_empty(&self) -> bool;

    fn shared_avail_units(&mut self) -> Self::MsgSize;

    async fn shared_wait_shutdown_or_avail_units(&mut self, items: usize) -> bool;

    async fn shared_wait_closed_or_avail_units(&mut self, items: usize) -> bool;

    async fn shared_wait_avail_units(&mut self, items: usize) -> bool;

    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)>;

    fn shared_take_slice(&mut self, elems: &mut [Self::MsgOut]) -> usize;

    fn shared_advance_index(&mut self, step: Self::MsgSize) -> (RxDone, Self::MsgSize);

}

impl <T: std::marker::Copy>RxCore for Rx<T> {

    // type MsgRef<'a> = &'a T where T: 'a;
    type MsgOut = T;
    type MsgSize = usize;



    async fn shared_take_async_timeout(&mut self, timeout: Option<Duration> ) -> Option<(RxDone, Self::MsgOut)> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down  => self.rx.try_pop()
                        , p = operation => p
                        , _ = timeout   => self.rx.try_pop()
                }
            } else {
                select! { _ = one_down  => self.rx.try_pop()
                        , p = operation => p
                }
            }
        } else {
            self.rx.try_pop()
        };
        if let Some(result) = result {
            self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
            Some((RxDone::Normal(1),result))
        } else {
            None
        }
    }



    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` when a message becomes available.
    /// None is ONLY returned if there is no data AND a shutdown was requested!
    ///
    /// # Asynchronous
    async fn shared_take_async(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            select! { _ = one_down => self.rx.try_pop(),
                     p = operation => p }
        } else {
            self.rx.try_pop()
        };
        if let Some(result) = result {
            self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
            Some((RxDone::Normal(1), result))
        } else {
            None

        }
    }

    fn shared_take_into_iter(&mut self) -> impl Iterator<Item = Self::MsgOut> {
        CountingIterator::new(self.rx.pop_iter(), &self.take_count)
        // self.rx.pop_iter()
    }

    fn shared_try_peek_iter(&self) -> impl Iterator<Item = &Self::MsgOut> {
        self.rx.iter()
    }


    async fn shared_peek_async_iter_timeout(&mut self, wait_for_count: usize, timeout: Option<Duration>) -> impl Iterator<Item = &Self::MsgOut> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
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
        self.rx.iter()
    }

    async fn shared_peek_async_iter(&mut self, wait_for_count: usize) -> impl Iterator<Item = &Self::MsgOut> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
            select! { _ = one_down => {}
                    , _ = operation => {}
                    , }
        }
        self.rx.iter()
    }
    fn shared_advance_index(&mut self, step: Self::MsgSize) -> (RxDone, Self::MsgSize) {
        let avail = self.rx.occupied_len();
        let idx = if step>avail {
            avail
        } else {
            step
        };
        unsafe { self.rx.advance_read_index(idx); }
        (RxDone::Normal(idx),idx)
    }

    fn shared_take_slice(&mut self, elems: &mut [T]) -> usize {
        let count = self.rx.pop_slice(elems);
        self.take_count.fetch_add(count as u32, Ordering::Relaxed); //wraps on overflow
        count
    }

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            RxDone::Normal(d) =>
                self.local_index = tel.process_event(self.local_index, self.channel_meta_data.id, d as isize),
            RxDone::Stream(i,p) => {
                warn!("internal error should have gotten Normal");
                self.local_index = tel.process_event(self.local_index, self.channel_meta_data.id, i as isize)
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.local_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool  {
        self.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> Self::MsgSize {
        self.rx.occupied_len()
    }


   async fn shared_wait_shutdown_or_avail_units(&mut self, items: usize) -> bool {

        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(items);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.rx.occupied_len() >= items
        }

    }

    async fn shared_wait_closed_or_avail_units(&mut self, items: usize) -> bool {
        if self.rx.occupied_len() >= items {
            true
        } else {//we always return true if we have count regardless of the shutdown status
            let mut one_closed = &mut self.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.rx.wait_occupied(items);
                select! { _ = one_closed => self.rx.occupied_len() >= items, _ = operation => true }
            } else {
                self.rx.occupied_len() >= items // if closed, we can still take
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, items: usize) -> bool {
        if self.rx.occupied_len() >= items {
            true
        } else {
            let operation = &mut self.rx.wait_occupied(items);
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
    // type MsgRef<'a> = (T, &'a[u8]);
    type MsgOut = (T, Box<[u8]>);
    type MsgSize = (usize,usize);



    async fn shared_take_async_timeout(&mut self, timeout: Option<Duration> ) -> Option<(RxDone, Self::MsgOut)> {
        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.pop();
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down  => self.shared_try_take()
                        , p = operation =>  if let Some(p) = p {
                                       let mut target = vec![0u8;p.length() as usize];
                                        self.payload_channel.rx.pop_slice(&mut target[..p.length() as usize]);
                                       Some((RxDone::Stream(1,target.len()),(p,target.into_boxed_slice()  )))
                                 } else {
                                       None
                                }
                        , _ = timeout   => self.shared_try_take()
                }
            } else {
                select! { _ = one_down  => self.shared_try_take()
                        , p = operation =>  if let Some(p) = p {
                                       let mut target = vec![0u8;p.length() as usize];
                                        self.payload_channel.rx.pop_slice(&mut target[..p.length() as usize]);
                                       Some((RxDone::Stream(1,target.len()),(p,target.into_boxed_slice()  )))
                                 } else {
                                       None
                                }
                }
            }
        } else {
            self.shared_try_take()
        };
        if let Some(result) = result {
            self.item_channel.take_count.fetch_add(1,Ordering::Relaxed);  //for DLQ detect
            Some(result)
        } else {
            None
        }
    }




    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` when a message becomes available.
    /// None is ONLY returned if there is no data AND a shutdown was requested!
    ///
    /// # Asynchronous
    async fn shared_take_async(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.pop();

            select! { _ = one_down => self.shared_try_take(),
                     p = operation => {
                                  if let Some(p) = p {
                                        let mut target = vec![0u8;p.length() as usize];
                                        self.payload_channel.rx.pop_slice(&mut target[..p.length() as usize]);
                                       Some((RxDone::Stream(1,target.len()),(p,target.into_boxed_slice()  )))
                                 } else {
                                       None
                                }
                     } }
        } else {
            self.shared_try_take()
        };
        if result.is_some() {
            self.item_channel.take_count.fetch_add(1,Ordering::Relaxed); //for DLQ detect
        }
        result
    }

    fn shared_take_into_iter(&mut self) -> impl Iterator<Item = Self::MsgOut> {

        let iter =
        self.item_channel
            .rx.pop_iter() //this iter DOES remove items
            .map(|i| {
                let mut target = vec![0u8; i.length() as usize];
                self.payload_channel.rx.pop_slice(&mut target[..i.length() as usize]);
                (i,target.into_boxed_slice())
            });

        CountingIterator::new(iter, &self.item_channel.take_count)
    }

    async fn shared_peek_async_iter_timeout(&mut self, wait_for_count: usize, timeout: Option<Duration>) -> impl Iterator<Item = &Self::MsgOut> {
        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.wait_occupied(wait_for_count);
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
        self.shared_try_peek_iter()
    }

    async fn shared_peek_async_iter(&mut self, wait_for_count: usize) -> impl Iterator<Item = &Self::MsgOut> {
        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.wait_occupied(wait_for_count);
            select! { _ = one_down => {}
                    , _ = operation => {}
                    , }
        }
        self.shared_try_peek_iter()
    }
    fn shared_try_peek_iter(&self) -> impl Iterator<Item = &Self::MsgOut> {

        let slices = self.payload_channel.rx.as_slices();
        let mut using_first = true;
        let mut payload_index = 0;

        self.item_channel
            .rx.iter() //this iter does NOT remove items
            .map(|i| {
                let mut target = vec![0u8; i.length() as usize];
                if using_first {
                    let a_len = i.length().min((slices.0.len() - payload_index) as i32);
                    let b_len = i.length()-a_len;
                    if 0==b_len {
                        let next_index = payload_index + a_len as usize;
                        target.put_slice(&slices.0[payload_index..next_index]);
                        payload_index = next_index;
                    } else {
                        let end_index = payload_index + a_len as usize;
                        target.put_slice(&slices.0[payload_index..end_index]);
                        target.put_slice(&slices.1[..b_len as usize]);
                        using_first = false;
                        payload_index = b_len as usize;
                    }
                } else {
                    let next_index = payload_index + i.length() as usize;
                    target.put_slice(&slices.1[payload_index..next_index]);
                    payload_index = next_index;
                }
                (i,target.into_boxed_slice())
            })
    }

    fn shared_advance_index(&mut self, step: Self::MsgSize) -> (RxDone, Self::MsgSize) {
        let item_avail = self.item_channel.rx.occupied_len();
        let items = if step.0>item_avail { item_avail  } else { step.0 };

        let payload_avail = self.payload_channel.rx.occupied_len();
        let payloads = if step.1>payload_avail { payload_avail  } else { step.1 };

        unsafe {
            self.payload_channel.rx.advance_read_index(payloads);
            self.item_channel.rx.advance_read_index(items);
        }
        (RxDone::Stream(items,payloads),(items,payloads))
    }

    fn shared_take_slice(&mut self, elems: &mut [Self::MsgOut]) -> usize where T: Copy {

        let target_count = elems.len().min(self.item_channel.avail_units());

        for i in 0..target_count {
          if let Some(item) = self.item_channel.rx.try_pop() {
              let mut payload = vec![0u8, item.length() as u8];
              self.payload_channel.rx.pop_slice(&mut payload);
             elems[i] = (item,payload.into_boxed_slice());
          }
        }

        self.item_channel.take_count.fetch_add(target_count as u32, Ordering::Relaxed);  //for DLQ!
        target_count
    }

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:RxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            RxDone::Normal(d) =>
                warn!("internal error should have gotten Stream"),
            RxDone::Stream(i,p) => {
                self.item_channel.local_index = tel.process_event(self.item_channel.local_index, self.item_channel.channel_meta_data.id, i as isize);
                self.payload_channel.local_index = tel.process_event(self.payload_channel.local_index, self.payload_channel.channel_meta_data.id, p as isize);
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.item_channel.local_index = MONITOR_NOT;
        self.payload_channel.local_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.item_channel.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool  {
        self.item_channel.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> Self::MsgSize {
        ( self.item_channel.rx.occupied_len(),
          self.payload_channel.rx.occupied_len())
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self,items: usize) -> bool {

        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.rx.wait_occupied(items);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.item_channel.rx.occupied_len() >= items
        }

    }

    async fn shared_wait_closed_or_avail_units(&mut self, items: usize) -> bool {
        if self.item_channel.rx.occupied_len() >= items {
            true
        } else {//we always return true if we have count regardless of the shutdown status
            let mut one_closed = &mut self.item_channel.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.item_channel.rx.wait_occupied(items);
                select! { _ = one_closed => self.item_channel.rx.occupied_len() >= items, _ = operation => true }
            } else {
                self.item_channel.rx.occupied_len() >= items // if closed, we can still take
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, items: usize) -> bool {
        if self.item_channel.rx.occupied_len() >= items{
            true
        } else {
            let operation = &mut self.item_channel.rx.wait_occupied(items);
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
    // type MsgRef<'a> = <T as RxCore>::MsgRef<'a>;
    type MsgOut = <T as RxCore>::MsgOut;
    type MsgSize = <T as RxCore>::MsgSize;



    async fn shared_take_async_timeout(&mut self, timeout: Option<Duration> ) -> Option<(RxDone, Self::MsgOut)> {
        <T as RxCore>::shared_take_async_timeout(&mut **self, timeout).await
    }


    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` when a message becomes available.
    /// None is ONLY returned if there is no data AND a shutdown was requested!
    ///
    /// # Asynchronous
    async fn shared_take_async(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        <T as RxCore>::shared_take_async(&mut **self).await
    }

    fn shared_take_into_iter(&mut self) -> impl Iterator<Item = Self::MsgOut> {
        <T as RxCore>::shared_take_into_iter(&mut **self)
    }

    async fn shared_peek_async_iter(&mut self, wait_for_count: usize) -> impl Iterator<Item = &Self::MsgOut> {
        <T as RxCore>::shared_peek_async_iter(&mut **self, wait_for_count).await
    }


    async fn shared_peek_async_iter_timeout(&mut self, wait_for_count: usize, timeout: Option<Duration>) -> impl Iterator<Item = &Self::MsgOut> {
        <T as RxCore>::shared_peek_async_iter_timeout(&mut **self, wait_for_count, timeout).await
    }

    fn shared_try_peek_iter(&self) -> impl Iterator<Item = &Self::MsgOut> {
        <T as RxCore>::shared_try_peek_iter(& **self)
    }

    fn shared_advance_index(&mut self, step: Self::MsgSize) -> (RxDone,Self::MsgSize) {
        <T as RxCore>::shared_advance_index(&mut **self, step)
    }

    fn shared_take_slice(&mut self, elems: &mut [Self::MsgOut]) -> usize {
        <T as RxCore>::shared_take_slice(&mut **self, elems)
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

    fn shared_avail_units(&mut self) -> Self::MsgSize {
        <T as RxCore>::shared_avail_units(&mut **self)
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self, items: usize) -> bool {
        <T as RxCore>::shared_wait_shutdown_or_avail_units(&mut **self, items).await
    }

    async fn shared_wait_closed_or_avail_units(&mut self, items: usize) -> bool {
        <T as RxCore>::shared_wait_closed_or_avail_units(&mut **self, items).await
    }

    async fn shared_wait_avail_units(&mut self, items: usize) -> bool {
        <T as RxCore>::shared_wait_avail_units(&mut **self, items).await
    }

    fn shared_try_take(&mut self) -> Option<(RxDone,Self::MsgOut)> {
        <T as RxCore>::shared_try_take(&mut **self)
    }
}

pub(crate) enum RxDone { //returns counts for telemetry, can be ignored, also this is internal only
    Normal(usize),
    Stream(usize,usize)
}