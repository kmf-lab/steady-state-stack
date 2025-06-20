use std::future::pending;
use log::{error, trace, warn};
use futures_util::{select, FutureExt};
use std::time::{Duration, Instant};
use futures::pin_mut;
use futures_timer::Delay;
use futures_util::lock::{MutexGuard};
use ringbuf::traits::Observer;
use futures_util::future::{Either, FusedFuture};
use async_ringbuf::producer::AsyncProducer;
use ringbuf::producer::Producer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::steady_tx::TxDone;
use crate::{steady_config, ActorIdentity, SendOutcome, SendSaturation, StreamIngress, StreamEgress, Tx, MONITOR_NOT};
use crate::distributed::distributed_stream::{StreamControlItem, StreamTx};
use crate::core_exec;
use crate::yield_now;

pub trait TxCore {
    type MsgIn<'a>;
    type MsgOut;
    type MsgSize: Copy;
    type SliceSource<'b> where Self::MsgOut: 'b;
    type SliceTarget<'a> where Self: 'a;


    fn shared_mark_closed(&mut self) -> bool;

    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize;
    fn log_perodic(&mut self) -> bool;

    fn one(&self) -> Self::MsgSize;
    fn telemetry_inc<const LEN:usize>(&mut self, done_count:TxDone , tel:& mut SteadyTelemetrySend<LEN>);
    fn monitor_not(&mut self);
    fn shared_capacity(&self) -> usize;
    fn shared_is_full(&self) -> bool;
    fn shared_is_empty(&self) -> bool;
    fn shared_vacant_units(&self) -> usize;
    #[allow(async_fn_in_trait)]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool;
    #[allow(async_fn_in_trait)]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool;
    #[allow(async_fn_in_trait)]
    async fn shared_wait_empty(&mut self) -> bool;

    fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone;

    fn shared_send_slice(& mut self, source: Self::SliceSource<'_> ) -> TxDone  where Self::MsgOut: Copy;

    fn shared_poke_slice(& mut self) -> Self::SliceTarget<'_>;


    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut>;
    #[allow(async_fn_in_trait)]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut>;
    #[allow(async_fn_in_trait)]
    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>,) ->  SendOutcome<Self::MsgOut>;
    #[allow(async_fn_in_trait)]
    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) ->  SendOutcome<Self::MsgOut>;

    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone;

    }

impl<T> TxCore for Tx<T> {

    type MsgIn<'a> = T;
    type MsgOut = T;
    type MsgSize = usize;
    type SliceSource<'b> = &'b [T] where T: 'b;
    type SliceTarget<'a> = (&'a mut [std::mem::MaybeUninit<T>], &'a mut [std::mem::MaybeUninit<T>]) where T: 'a;


    fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone {
        let avail = self.tx.occupied_len();
        let idx = if request>avail {
            avail
        } else {
            request
        };
        unsafe { self.tx.advance_write_index(idx); }
        TxDone::Normal(idx)
    }

    fn done_one(&self, _one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Normal(1)
    }
    fn shared_mark_closed(&mut self) -> bool {
        if let Some(c) = self.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                error!("close called but the receiver already dropped");
            }
        } else {
            //NOTE: this may happen more than once, expecially for simulations
             trace!("{:?}\n already marked closed, check for redundant calls, ensure mark_closed is called last after all other conditions!"
                ,  self.channel_meta_data.meta_data);
        }
        true // always returns true, close request is never rejected by this method.
    }

    fn one(&self) -> Self::MsgSize {
        1
    }

    fn log_perodic(&mut self) -> bool {
        if self.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.last_error_send = Instant::now();
            true
        }
    }

    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        self.tx.push_iter(iter)
    }

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:TxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(d) =>
              self.local_monitor_index = tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, d as isize),
            TxDone::Stream(i,_p) => {
                warn!("internal error should have gotten Normal");
                self.local_monitor_index = tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, i as isize)
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.local_monitor_index = MONITOR_NOT;
    }

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.tx.capacity().get()
    }

    #[inline]
    fn shared_is_full(&self) -> bool {
        self.tx.is_full()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.tx.is_empty()
    }

    #[inline]
    fn shared_vacant_units(&self) -> usize {

       // self.tx.vacant_len()
        let capacity = self.tx.capacity().get();
        let modulus = 2 * capacity;

        // Read read_index FIRST, then write_index for producer floor guarantee
        let read_idx = self.tx.read_index();
        let write_idx = self.tx.write_index();

        let result = (capacity + read_idx - write_idx) % modulus;
        assert!(result<=capacity);
        result

    }


    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool {
        if self.tx.is_empty() || self.tx.vacant_len() >= count {
            true
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let safe_count = count.min(self.tx.capacity().into());
                let mut operation = &mut self.tx.wait_vacant(safe_count);
                select! { _ = one_down => false, _ = operation => true, }
            } else {
                yield_now().await; //this is a big help in shutdown tight loop
                false
            }
        }
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let safe_count = count.min(self.tx.capacity().into());
            let operation = &mut self.tx.wait_vacant(safe_count);
            operation.await;
            true
        }
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.tx.wait_vacant(usize::from(self.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.tx.capacity().get() == self.tx.vacant_len()
        }
    }

    #[inline]
    fn shared_send_slice(& mut self, slice: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy {

        if !slice.is_empty() {
            // Try to push as many items as possible from the slice.
            // Returns the number of items actually pushed.
            let pushed = self.tx.push_slice(slice);
            TxDone::Normal(pushed)
        } else {
            TxDone::Normal(0)
        }
    }

    #[inline]
    fn shared_poke_slice(& mut self) -> Self::SliceTarget<'_> {
        self.tx.vacant_slices_mut()
    }


    #[inline]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        debug_assert!(self.make_closed.is_some(),"Send called after channel marked closed");

        match self.tx.try_push(msg) {
            Ok(_) => Ok(TxDone::Normal(1)),
            Err(m) => Err(m),
        }
    }

    #[inline]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }

        match self.tx.try_push(msg) {
            Ok(_) => SendOutcome::Success,
            Err(msg) => {
                match saturation {
                    SendSaturation::AwaitForRoom => {}
                    SendSaturation::ReturnBlockedMsg => return SendOutcome::Blocked(msg),
                    SendSaturation::WarnThenAwait => self.report_tx_full_warning(ident),
                    SendSaturation::DebugWarnThenAwait => {
                                                #[cfg(debug_assertions)]
                                                self.report_tx_full_warning(ident);
                    }
                }

                let has_room = self.tx.wait_vacant(1).fuse();
                pin_mut!(has_room);
                let mut one_down = &mut self.oneshot_shutdown;
                let timeout_future = match timeout {
                    Some(duration) => Delay::new(duration).fuse(),
                    None => Delay::new(Duration::from_secs(i32::MAX as u64)).fuse(), //do not use duration max as we need to add
                };
                pin_mut!(timeout_future);

                if !one_down.is_terminated() {
                    select! {
                        _ = one_down => SendOutcome::Blocked(msg),
                        _ = has_room => {
                            match self.tx.push(msg).await {
                                Ok(_) => SendOutcome::Success,
                                Err(t) => {
                                    error!("channel is closed");
                                    SendOutcome::Blocked(t)
                                }
                            }
                        }
                        _ = timeout_future => SendOutcome::Blocked(msg),
                    }
                } else {
                    SendOutcome::Blocked(msg)
                }
            }
        }
    }

    #[inline]
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, None).await
    }

    #[inline]
    async fn shared_send_async_timeout(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, timeout).await
    }

}

impl TxCore for StreamTx<StreamIngress> {
    type MsgIn<'a> = (StreamIngress, &'a[u8]);
    type MsgOut = StreamIngress;
    type MsgSize = (usize, usize);
    type SliceSource<'b> = (&'b [StreamIngress], &'b [u8]);
    type SliceTarget<'a> = (
        &'a mut [std::mem::MaybeUninit<StreamIngress>],
        &'a mut [std::mem::MaybeUninit<StreamIngress>],
        &'a mut [std::mem::MaybeUninit<u8>],
        &'a mut [std::mem::MaybeUninit<u8>],
    );

    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
       TxDone::Stream(1, one.1.len())
    }


    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {

        let control_avail = self.control_channel.tx.occupied_len();
        let payload_avail = self.payload_channel.tx.occupied_len();
        //NOTE: we only have two counts so either we do all or none because we can
        //      not compute a safe cut point. so it is all or nothing

        if count.0 <= control_avail && count.1 <= payload_avail {
            unsafe {
                self.payload_channel.tx.advance_write_index(count.1);
                self.control_channel.tx.advance_write_index(count.0);
            }
            TxDone::Stream(count.0,count.1)
        } else {
            TxDone::Stream(0,0)
        }
    }

    fn shared_mark_closed(&mut self) -> bool {
        if let Some(c) = self.control_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                trace!("close called but the receiver already dropped");
            }
        }
        if let Some(c) = self.payload_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                trace!("close called but the receiver already dropped");
            }
        }
        true // always returns true, close request is never rejected by this method.
    }

    fn one(&self) -> Self::MsgSize {
        (1,self.payload_channel.capacity()/self.control_channel.capacity())
    }
    fn log_perodic(&mut self) -> bool {
        if self.control_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.control_channel.last_error_send = Instant::now();
            true
        }
    }

    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(),"Send called after channel marked closed");

        let mut count = 0;

        //while we have room keep going.
        let item_limit = self.control_channel.tx.vacant_len();
        let limited_iter = iter
            .take(item_limit);

        for (item,payload) in limited_iter {
            assert_eq!(item.length(),payload.len() as i32);
            if payload.len() > self.payload_channel.tx.vacant_len() {
                warn!("the payload of the stream should be larger we need {} but found {}",payload.len(),self.payload_channel.tx.vacant_len());
                core_exec::block_on(self.payload_channel.tx.wait_vacant(payload.len()));
            }
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(item);
            count += 1;
        }
        count

    }

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:TxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(i) => {
                warn!("internal error should have gotten Stream");
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
            },
            TxDone::Stream(i,p) => {
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

     #[inline]
    fn shared_capacity(&self) -> usize {
        self.control_channel.tx.capacity().get()
    }

    #[inline]
    fn shared_is_full(&self) -> bool {
        self.control_channel.tx.is_full()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.tx.is_empty()
    }

    #[inline]
    fn shared_vacant_units(&self) -> usize {
        self.control_channel.tx.vacant_len()
    }

    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool {
        if (self.control_channel.tx.is_empty() ||  self.control_channel.tx.vacant_len() >= count.0) &&
            (self.payload_channel.tx.is_empty() ||  self.payload_channel.tx.vacant_len() >= count.1) {
            true
        } else {
            let icap = self.control_channel.capacity();
            let pcap = self.payload_channel.capacity();

            let mut one_down = &mut self.control_channel.oneshot_shutdown;
            if !one_down.is_terminated() {
                let operation = async {
                                       self.control_channel.tx.wait_vacant(count.0.min(icap)).await;
                                       self.payload_channel.tx.wait_vacant(count.1.min(pcap)).await;
                                   };
                select! { _ = one_down => false, _ = operation.fuse() => true, }
            } else {
                yield_now().await; //this is a big help in shutdown tight loop
                false
            }
        }
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.control_channel.tx.vacant_len() >= count.0 &&
            self.payload_channel.tx.vacant_len() >= count.1 {
            true
        } else {
            self.control_channel.tx.wait_vacant(count.0.min(self.control_channel.capacity())).await;
            self.payload_channel.tx.wait_vacant(count.1.min(self.payload_channel.capacity())).await;
            true
        }
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.tx.wait_vacant(usize::from(self.control_channel.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.control_channel.tx.capacity().get() == self.control_channel.tx.vacant_len()
        }
    }

    #[inline]
    fn shared_send_slice(& mut self, slice: Self::SliceSource<'_>) -> TxDone
    where
        Self::MsgOut: Copy,
    {
        let mut items_sent = 0;
        let mut bytes_sent = 0;
        let (ingress_items, payload_bytes) = slice;

        if !ingress_items.is_empty() {
            let item_vacant = self.control_channel.tx.vacant_len();
            let payload_vacant = self.payload_channel.tx.vacant_len();


            for &ingress in ingress_items.iter().take(item_vacant) {
                let len = ingress.length() as usize;
                if bytes_sent + len > payload_bytes.len() || bytes_sent + len > payload_vacant {
                    break;
                }
                let payload_chunk = &payload_bytes[bytes_sent..bytes_sent + len];
                let pushed = self.payload_channel.tx.push_slice(payload_chunk);
                if pushed != len {
                    break;
                }
                if self.control_channel.tx.try_push(ingress).is_err() {
                    break;
                }
                items_sent += 1;
                bytes_sent += len;
            }
        }
        TxDone::Stream(items_sent, bytes_sent)
    }

    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        let (item_a, item_b) = self.control_channel.tx.vacant_slices_mut();
        let (payload_a, payload_b) = self.payload_channel.tx.vacant_slices_mut();
        (item_a, item_b, payload_a, payload_b)
    }

    #[inline]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        let (item,payload) = msg;
        assert_eq!(item.length(),payload.len() as i32);

        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(),"Send called after channel marked closed");

        if self.payload_channel.tx.vacant_len() >= item.length() as usize &&
           self.control_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(item);
            Ok(TxDone::Stream(1,payload.len()))
        } else {
            Err(item)
        }
    }

    #[inline]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        let (item, payload) = msg;
        assert_eq!(item.length(), payload.len() as i32);

        debug_assert!(
            self.control_channel.make_closed.is_some(),
            "Send called after channel marked closed"
        );
        debug_assert!(
            self.payload_channel.make_closed.is_some(),
            "Send called after channel marked closed"
        );

        // Try to push immediately
        let push_result = {
            let payload_tx = &mut self.payload_channel.tx;
            let item_tx = &mut self.control_channel.tx;
            if payload_tx.vacant_len() >= item.length() as usize && item_tx.vacant_len() >= 1 {
                let payload_size = payload_tx.push_slice(payload);
                debug_assert_eq!(payload_size, item.length() as usize);
                let _ = item_tx.try_push(item);
                Ok(())
            } else {
                Err(msg)
            }
        };

        match push_result {
            Ok(_) => SendOutcome::Success,
            Err((item, payload)) => {
                // Handle saturation
                match saturation {
                    SendSaturation::AwaitForRoom => {}
                    SendSaturation::ReturnBlockedMsg => return SendOutcome::Blocked(item),
                    SendSaturation::WarnThenAwait => {
                        self.control_channel.report_tx_full_warning(ident);
                        self.payload_channel.report_tx_full_warning(ident);
                    }
                    SendSaturation::DebugWarnThenAwait => {
                        #[cfg(debug_assertions)]
                        {
                            self.control_channel.report_tx_full_warning(ident);
                            self.payload_channel.report_tx_full_warning(ident);
                        }
                    }
                }

                // Wait for vacant space, shutdown, or timeout
                let has_room = async {
                    self.payload_channel.tx.wait_vacant(payload.len()).await;
                    self.control_channel.tx.wait_vacant(1).await;
                }
                    .fuse();
                pin_mut!(has_room);
                let mut one_down = &mut self.control_channel.oneshot_shutdown;
                let timeout_future = match timeout {
                    Some(duration) => Either::Left(Delay::new(duration).fuse()),
                    None => Either::Right(pending::<()>().fuse()),
                };
                pin_mut!(timeout_future);

                if !one_down.is_terminated() {
                    select! {
                    _ = one_down => SendOutcome::Blocked(item), // Shutdown detected
                    _ = has_room => {
                        // Push payload
                        let pushed = self.payload_channel.tx.push_slice(payload) == payload.len();
                        if !pushed {
                            error!("channel is closed");
                            return SendOutcome::Blocked(item);
                        }
                        // Push item
                        match self.control_channel.tx.push(item).await {
                            Ok(_) => SendOutcome::Success,
                            Err(t) => {
                                error!("channel is closed");
                                SendOutcome::Blocked(t)
                            }
                        }
                    }
                    _ = timeout_future => SendOutcome::Blocked(item), // Timeout
                }
                } else {
                    // Already shut down
                    SendOutcome::Blocked(item)
                }
            }
        }
    }

    #[inline]
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, None).await
    }

    #[inline]
    async fn shared_send_async_timeout(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, timeout).await
    }

}



impl TxCore for StreamTx<StreamEgress> {
    type MsgIn<'a> = &'a[u8]; 
    type MsgOut = StreamEgress;
    type MsgSize = (usize, usize);
    type SliceSource<'b> = (&'b [StreamEgress], &'b [u8]);
    type SliceTarget<'a> = (
        &'a mut [std::mem::MaybeUninit<StreamEgress>],
        &'a mut [std::mem::MaybeUninit<StreamEgress>],
        &'a mut [std::mem::MaybeUninit<u8>],
        &'a mut [std::mem::MaybeUninit<u8>]
    );


    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {

        let control_avail = self.control_channel.tx.occupied_len();
        let payload_avail = self.payload_channel.tx.occupied_len();
        //NOTE: we only have two counts so either we do all or none because we can
        //      not compute a safe cut point. so it is all or nothing

        if count.0 <= control_avail && count.1 <= payload_avail {
            unsafe {
                self.payload_channel.tx.advance_write_index(count.1);
                self.control_channel.tx.advance_write_index(count.0);
            }
            TxDone::Stream(count.0,count.1)
        } else {
            TxDone::Stream(0,0)
        }
    }

    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Stream(1, one.len())
    }
    fn shared_mark_closed(&mut self) -> bool {
        if let Some(c) = self.control_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                trace!("close called but the receiver already dropped");
            }
        }
        if let Some(c) = self.payload_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                trace!("close called but the receiver already dropped");
            }
        }
        true // always returns true, close request is never rejected by this method.
    }

    fn one(&self) -> Self::MsgSize {
        (1,self.payload_channel.capacity()/self.control_channel.capacity())
    }

    fn log_perodic(&mut self) -> bool {
        if self.control_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.control_channel.last_error_send = Instant::now();
            true
        }
    }

    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(),"Send called after channel marked closed");

        let mut count = 0;

        //while we have room keep going.
        let item_limit = self.control_channel.tx.vacant_len();
        let limited_iter = iter
            .take(item_limit);

        for payload in limited_iter {
            if payload.len() > self.payload_channel.tx.vacant_len() {
                warn!("the payload of the stream should be larger we need {} but found {}",payload.len(),self.payload_channel.tx.vacant_len());
                core_exec::block_on(self.payload_channel.tx.wait_vacant(payload.len()));
            }
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(StreamEgress { length: payload.len() as i32 });
            count += 1;
        }
        count

    }

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:TxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(i) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
            },
            TxDone::Stream(i,p) => {
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

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.control_channel.tx.capacity().get()
    }

    #[inline]
    fn shared_is_full(&self) -> bool {
        self.control_channel.tx.is_full()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.tx.is_empty()
    }

    #[inline]
    fn shared_vacant_units(&self) -> usize {
        self.control_channel.tx.vacant_len()
    }

    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool {
        if  (self.control_channel.tx.is_empty() || self.control_channel.tx.vacant_len() >= count.0) &&
            (self.payload_channel.tx.is_empty() || self.payload_channel.tx.vacant_len() >= count.1) {
            true
        } else {
            let icap = self.control_channel.capacity();
            let pcap = self.payload_channel.capacity();

            let mut one_down = &mut self.control_channel.oneshot_shutdown;
            if !one_down.is_terminated() {
                let operation = async {
                    self.control_channel.tx.wait_vacant(count.0.min(icap)).await;
                    self.payload_channel.tx.wait_vacant(count.1.min(pcap)).await;
                };
                select! { _ = one_down => false, _ = operation.fuse() => true, }
            } else {
                yield_now().await; //this is a big help in shutdown tight loop
                false
            }
        }
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.control_channel.tx.vacant_len() >= count.0 &&
            self.payload_channel.tx.vacant_len() >= count.1 {
            true
        } else {
            self.control_channel.tx.wait_vacant(count.0.min(self.control_channel.capacity())).await;
            self.payload_channel.tx.wait_vacant(count.1.min(self.payload_channel.capacity())).await;
            true
        }
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.tx.wait_vacant(usize::from(self.control_channel.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.control_channel.tx.capacity().get() == self.control_channel.tx.vacant_len()
        }
    }

    #[inline]
    fn shared_send_slice(& mut self, slice: Self::SliceSource<'_>) -> TxDone
    where
        Self::MsgOut: Copy,
    {

        let mut items_sent = 0;
        let mut bytes_sent = 0;
        let (egress_items, payload_bytes) = slice;

        if !egress_items.is_empty() {
            // We'll send as many as we can, limited by both item and payload channel space
            let item_vacant = self.control_channel.tx.vacant_len();
            let payload_vacant = self.payload_channel.tx.vacant_len();

            // Each egress item corresponds to a payload chunk of length egress_item.length()
            // But for StreamEgress, the convention is that each item is just a length marker for a payload chunk.
            // In this case, we expect egress_items.len() == 1 and payload_bytes.len() == egress_items[0].length() as usize
            // But for a batch, we can support multiple.


            for &egress in egress_items.iter().take(item_vacant) {
                let len = egress.length as usize;
                if bytes_sent + len > payload_bytes.len() || bytes_sent + len > payload_vacant {
                    break;
                }
                // Push the payload chunk
                let payload_chunk = &payload_bytes[bytes_sent..bytes_sent + len];
                let pushed = self.payload_channel.tx.push_slice(payload_chunk);
                if pushed != len {
                    break;
                }
                // Push the egress item
                if self.control_channel.tx.try_push(egress).is_err() {
                    break;
                }
                items_sent += 1;
                bytes_sent += len;
            }
        }
        TxDone::Stream(items_sent, bytes_sent)
    }

    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
         let (item_a, item_b) = self.control_channel.tx.vacant_slices_mut();
         let (payload_a, payload_b) = self.payload_channel.tx.vacant_slices_mut();
         (item_a, item_b, payload_a, payload_b)
    }


    #[inline]
    fn shared_try_send(&mut self, payload: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {

        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(),"Send called after channel marked closed");

        if self.payload_channel.tx.vacant_len() >= payload.len() as usize &&
            self.control_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(StreamEgress { length: payload.len() as i32 });
            Ok(TxDone::Stream(1,payload.len()))
        } else {
            Err(StreamEgress { length: payload.len() as i32 })
        }
    }

    #[inline]
    async fn shared_send_async_core(
        &mut self,
        payload: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        debug_assert!(
            self.control_channel.make_closed.is_some(),
            "Send called after channel marked closed"
        );
        debug_assert!(
            self.payload_channel.make_closed.is_some(),
            "Send called after channel marked closed"
        );

        // Try to push immediately
        let push_result = {
            let payload_tx = &mut self.payload_channel.tx;
            let item_tx = &mut self.control_channel.tx;
            if payload_tx.vacant_len() >= payload.len() as usize && item_tx.vacant_len() >= 1 {
                let payload_size = payload_tx.push_slice(payload);
                debug_assert_eq!(payload_size, payload.len() as usize);
                let _ = item_tx.try_push(StreamEgress { length: payload.len() as i32 });
                Ok(())
            } else {
                Err(StreamEgress { length: payload.len() as i32 })
            }
        };

        match push_result {
            Ok(_) => SendOutcome::Success,
            Err(item) => {
                // Handle saturation
                match saturation {
                    SendSaturation::AwaitForRoom => {}
                    SendSaturation::ReturnBlockedMsg => return SendOutcome::Blocked(item),
                    SendSaturation::WarnThenAwait => {
                        self.control_channel.report_tx_full_warning(ident);
                        self.payload_channel.report_tx_full_warning(ident);
                    }
                    SendSaturation::DebugWarnThenAwait => {
                        #[cfg(debug_assertions)]
                        {
                            self.control_channel.report_tx_full_warning(ident);
                            self.payload_channel.report_tx_full_warning(ident);
                        }
                    }
                }

                // Wait for vacant space, shutdown, or timeout
                let has_room = async {
                    self.payload_channel.tx.wait_vacant(payload.len()).await;
                    self.control_channel.tx.wait_vacant(1).await;
                }
                    .fuse();
                pin_mut!(has_room);
                let mut one_down = &mut self.control_channel.oneshot_shutdown;
                let timeout_future = match timeout {
                    Some(duration) => Either::Left(Delay::new(duration).fuse()),
                    None => Either::Right(pending::<()>().fuse()),
                };
                pin_mut!(timeout_future);

                if !one_down.is_terminated() {
                    select! {
                    _ = one_down => SendOutcome::Blocked(item), // Shutdown detected
                    _ = has_room => {
                        // Push payload
                        let pushed = self.payload_channel.tx.push_slice(payload) == payload.len();
                        if !pushed {
                            error!("channel is closed");
                            return SendOutcome::Blocked(item);
                        }
                        // Push item
                        match self.control_channel.tx.push(item).await {
                            Ok(_) => SendOutcome::Success,
                            Err(t) => {
                                error!("channel is closed");
                                SendOutcome::Blocked(t)
                            }
                        }
                    }
                    _ = timeout_future => SendOutcome::Blocked(item), // Timeout
                }
                } else {
                    // Already shut down
                    SendOutcome::Blocked(item)
                }
            }
        }
    }

    #[inline]
    async fn shared_send_async(
        &mut self,
        payload: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(payload, ident, saturation, None).await
    }

    #[inline]
    async fn shared_send_async_timeout(
        &mut self,
        payload: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(payload, ident, saturation, timeout).await
    }

}



impl<T: TxCore> TxCore for MutexGuard<'_, T> {
    type MsgIn<'a> = <T as TxCore>::MsgIn<'a>;
    type MsgOut = <T as TxCore>::MsgOut;
    type MsgSize =  <T as TxCore>::MsgSize;
    type SliceSource<'b> = <T as TxCore>::SliceSource<'b> where Self::MsgOut: 'b;
    type SliceTarget<'a> = <T as TxCore>::SliceTarget<'a> where Self: 'a;



    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {
        <T as TxCore>::shared_advance_index(&mut **self, count)
    }

    fn shared_mark_closed(&mut self) -> bool {
        <T as TxCore>::shared_mark_closed(&mut **self)
    }

    fn one(&self) -> Self::MsgSize {
        <T as TxCore>::one(& **self)
    }

    fn log_perodic(&mut self) -> bool {
        <T as TxCore>::log_perodic(&mut **self)
    }

    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        <T as TxCore>::telemetry_inc(&mut **self, done_count, tel)
    }

    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        <T as TxCore>::shared_send_iter_until_full(&mut **self,iter)
     }

    fn monitor_not(&mut self) {
        <T as TxCore>::monitor_not(&mut **self)
    }

    #[inline]
    fn shared_capacity(&self) -> usize {
        <T as TxCore>::shared_capacity(&**self)
    }

    #[inline]
    fn shared_is_full(&self) -> bool {
        <T as TxCore>::shared_is_full(&**self)
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        <T as TxCore>::shared_is_empty(&**self)
    }

    #[inline]
    fn shared_vacant_units(&self) -> usize {

        <T as TxCore>::shared_vacant_units(&**self)
    }



    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        <T as TxCore>::shared_wait_shutdown_or_vacant_units(&mut **self, count).await
    }

    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        <T as TxCore>::shared_wait_vacant_units(&mut **self,count).await
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        <T as TxCore>::shared_wait_empty(&mut **self).await
    }

    #[inline]
    fn shared_send_slice(& mut self, slice: Self::SliceSource<'_> ) -> TxDone  where Self::MsgOut: Copy {
        <T as TxCore>::shared_send_slice(self, slice)
    }
    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        <T as TxCore>::shared_poke_slice(&mut **self)
    }

    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        <T as TxCore>::shared_try_send(&mut **self, msg)
    }
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        <T as TxCore>::shared_send_async_core(&mut **self,msg,ident,saturation,timeout).await
    }

    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>) ->SendOutcome<Self::MsgOut> {
        <T as TxCore>::shared_send_async_timeout(&mut **self,msg,ident,saturation,timeout).await
    }

    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> SendOutcome<Self::MsgOut> {
         <T as TxCore>::shared_send_async(&mut **self,msg,ident,saturation).await
    }

    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        <T as TxCore>::done_one(&self,one)
    }


}

// Unit-tests for the combined TxCore / RxCore behavior
#[cfg(test)]
mod core_tx_rx_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::*;
    use crate::core_rx::RxCore;

    #[test]
    fn test_tx_rx_basic_flow() {
        // build a channel of capacity 2
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx, rx) = builder.build_channel::<i32>();

        // --- test TxCore methods ---
        let tx = tx.clone();
        let mut txg = core_exec::block_on(tx.lock());
        assert_eq!(txg.shared_capacity(), 2);
        assert!(txg.shared_is_empty());
        let sent = txg.shared_send_slice_until_full(&[7, 8]);
        assert_eq!(sent, 2);
        assert!(txg.shared_is_full());
        drop(txg);

        // --- test RxCore methods ---
        let rx = rx.clone();
        let mut rxg = core_exec::block_on(rx.lock());
        assert_eq!(rxg.shared_capacity(), 2);
        assert_eq!(rxg.shared_avail_units(), 2);
        assert_eq!(rxg.shared_try_peek(), Some(&7));
        drop(rxg);

        let rx = rx.clone();
        let mut rxg = core_exec::block_on(rx.lock());
        assert_eq!(rxg.shared_try_take().map(|(_,v)| v), Some(7));
        assert_eq!(rxg.shared_try_take().map(|(_,v)| v), Some(8));
        assert!(rxg.shared_is_empty());
    }

    #[test]
    fn test_bad_message_detection() {
        let builder = ChannelBuilder::default().with_capacity(1);
        let (tx, rx) = builder.build_channel::<u8>();

        // send one byte
        let tx = tx.clone();
        let mut txg = core_exec::block_on(tx.lock());
        assert_eq!(txg.shared_send_slice_until_full(&[42]), 1);
        drop(txg);

        // repeated peeks should bump `peek_repeats`
        let rx = rx.clone();
        let mut rxg = core_exec::block_on(rx.lock());
        assert_eq!(rxg.shared_try_peek(), Some(&42));
        assert_eq!(rxg.shared_try_peek(), Some(&42));
        assert!(rxg.is_showstopper(2));
        assert!(!rxg.is_showstopper(5));
    }
}

#[cfg(test)]
mod extended_coverage {
    use super::*;
    use futures::executor::block_on;
    use futures_util::lock::Mutex;
    use crate::TxCore;
    use crate::{ActorIdentity, SendSaturation, SendOutcome};
    use std::time::Duration;
    use crate::GraphBuilder;

    /// FakeTx implements TxCore with predictable behavior for testing the MutexGuard<TxCore> forwarding.
    struct FakeTx {
        closed: bool,
        send_count: usize,
        log_calls: usize,
        one_val: usize,
        capacity: usize,
        is_full: bool,
        is_empty: bool,
        vacant: usize,
    }

    impl FakeTx {
        fn new() -> Self {
            FakeTx { closed: false, send_count: 0, log_calls: 0, one_val: 3, capacity: 4, is_full: false, is_empty: true, vacant: 4 }
        }
    }

    impl TxCore for FakeTx {
        type MsgIn<'a> = usize;
        type MsgOut = usize;
        type MsgSize = usize;
        type SliceSource<'b> = &'b [usize];
        type SliceTarget<'a> = (&'a [usize],&'a [usize]);


        fn shared_mark_closed(&mut self) -> bool {
            self.closed = true;
            true
        }

        fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
            let cnt = iter.count();
            self.send_count += cnt;
            cnt
        }

        fn log_perodic(&mut self) -> bool {
            self.log_calls += 1;
            self.log_calls > 1
        }

        fn one(&self) -> Self::MsgSize {
            self.one_val
        }

        fn telemetry_inc<const LEN: usize>(&mut self, _d: TxDone, _tel: &mut crate::monitor_telemetry::SteadyTelemetrySend<LEN>) {
            // no internal state change needed
        }

        fn monitor_not(&mut self) {
            self.one_val = 0;
        }

        fn shared_capacity(&self) -> usize {
            self.capacity
        }

        fn shared_is_full(&self) -> bool {
            self.is_full
        }

        fn shared_is_empty(&self) -> bool {
            self.is_empty
        }

        fn shared_vacant_units(&self) -> usize {
            self.vacant
        }

        async fn shared_wait_shutdown_or_vacant_units(&mut self, _count: Self::MsgSize) -> bool {
            // simulate immediate availability
            true
        }

        async fn shared_wait_vacant_units(&mut self, _count: Self::MsgSize) -> bool {
            true
        }

        async fn shared_wait_empty(&mut self) -> bool {
            true
        }

        fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone {
            TxDone::Normal(0)
        }

        #[inline]
        fn shared_send_slice<'a,'b>(&'a mut self, slice: Self::SliceSource<'b> ) -> TxDone {
            TxDone::Normal(0)
        }
        #[inline]
        fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
            let (item_a, item_b) = (&[],&[]);
            (item_a, item_b)
        }

        fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
            Ok(TxDone::Normal(msg))
        }

        async fn shared_send_async_core(
            &mut self,
            _msg: Self::MsgIn<'_>,
            _ident: ActorIdentity,
            _s: SendSaturation,
            _timeout: Option<Duration>,
        ) -> SendOutcome<Self::MsgOut> {
            SendOutcome::Success
        }

        async fn shared_send_async_timeout(
            &mut self,
            msg: Self::MsgIn<'_>,
            _id: ActorIdentity,
            _s: SendSaturation,
            _timeout: Option<Duration>,
        ) -> SendOutcome<Self::MsgOut> {
            SendOutcome::Success
        }

        async fn shared_send_async(
            &mut self,
            msg: Self::MsgIn<'_>,
            _id: ActorIdentity,
            _s: SendSaturation,
        ) -> SendOutcome<Self::MsgOut> {
            SendOutcome::Success
        }

        fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
            TxDone::Normal(*one)
        }
    }

    #[test]
    fn test_mutexguard_txcore_methods() {
        // Use the futures_util Mutex to get a MutexGuard<'_, FakeTx>
        let mtx = Mutex::new(FakeTx::new());
        let mut guard = block_on(mtx.lock());

        // Test shared_mark_closed
        assert!(!guard.closed);
        assert!(guard.shared_mark_closed());
        assert!(guard.closed);

        // Test shared_send_iter_until_full
        let sent = guard.shared_send_iter_until_full([10,20,30].into_iter());
        assert_eq!(sent, 3);

        // Test log_perodic toggles
        assert!(!guard.log_perodic());
        assert!(guard.log_perodic());

        // one() value
        assert_eq!(guard.one(), 3);
        // monitor_not resets one()
        guard.monitor_not();
        assert_eq!(guard.one(), 0);

        // capacity, full, empty, vacant
        assert_eq!(guard.shared_capacity(), 4);
        assert!(!guard.shared_is_full());
        assert!(guard.shared_is_empty());
        assert_eq!(guard.shared_vacant_units(), 4);

        // Async wait methods
        assert!(block_on(guard.shared_wait_shutdown_or_vacant_units(1)));
        assert!(block_on(guard.shared_wait_vacant_units(1)));
        assert!(block_on(guard.shared_wait_empty()));

        // Try send
        let try_res = guard.shared_try_send(5);
        assert!(try_res.is_ok());
        // Async send
        let ident = ActorIdentity::new(0, "test", None);
        let res = block_on(guard.shared_send_async(7, ident, SendSaturation::AwaitForRoom));
        assert!(matches!(res, SendOutcome::Success));

        let res_to = block_on(guard.shared_send_async_timeout(8, ident, SendSaturation::ReturnBlockedMsg, Some(Duration::from_millis(1))));
        assert!(matches!(res_to, SendOutcome::Success));

        // done_one
        assert_eq!(guard.done_one(&9), TxDone::Normal(9));
    }


    /// A tiny helper to build a fresh Tx<u8> and its matching Rx<u8>.
    fn new_tx() -> Tx<u8> {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = graph.channel_builder();
        let (tx, _rx) = builder.eager_build_internal();
        tx
    }

    #[test]
    fn done_one_and_shared_mark_closed() {
        let mut tx = new_tx();

        // done_one should always return Normal(1)
        assert_eq!(tx.done_one(&42u8), TxDone::Normal(1));

        // First close takes the oneshot sender and succeeds.
        assert!(tx.shared_mark_closed());
        // Second close finds `make_closed` == None → the `warn!` branch.
        assert!(tx.shared_mark_closed());
    }

    #[test]
    fn shared_send_iter_until_full_and_warn_after_close() {
        let mut tx = new_tx();
        // push two items
        let pushed = tx.shared_send_iter_until_full([7u8, 8u8].into_iter());
        assert_eq!(pushed, 2);

        // consume the close flag so that `make_closed` becomes None
        let _ = tx.shared_mark_closed();
        // now warn branch should run
        let pushed2 = tx.shared_send_iter_until_full(std::iter::empty());
        assert_eq!(pushed2, 0);
    }
    
    #[test]
    fn shared_try_send_and_async_variants() {
        let mut tx = new_tx();
        let ident = ActorIdentity::new(0, "me", None);

        // shared_try_send success
        assert_eq!(tx.shared_try_send(99u8), Ok(TxDone::Normal(1)));

        // shared_send_async_core success immediately
        let outcome = block_on(tx.shared_send_async_core(5u8, ident, SendSaturation::AwaitForRoom, None));
        assert!(matches!(outcome, SendOutcome::Success));

        // shared_send_async = core with None timeout
        let outcome2 = block_on(tx.shared_send_async(6u8, ident, SendSaturation::ReturnBlockedMsg));
        assert!(matches!(outcome2, SendOutcome::Success));

        // shared_send_async_timeout passes through
        let outcome3 = block_on(tx.shared_send_async_timeout(7u8, ident, SendSaturation::ReturnBlockedMsg, Some(Duration::from_millis(1))));
        assert!(matches!(outcome3, SendOutcome::Success));
    }

}
