use log::{error, warn};
use futures_util::{select, FutureExt};
use std::time::{Duration, Instant};
use futures::pin_mut;
use futures_timer::Delay;
use futures_util::lock::MutexGuard;
use ringbuf::traits::Observer;
use futures_util::future::FusedFuture;
use async_ringbuf::producer::AsyncProducer;
use nuclei::block_on;
use ringbuf::producer::Producer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::steady_tx::TxDone;
use crate::{steady_config, ActorIdentity, SendSaturation, Tx, MONITOR_NOT};
use crate::distributed::distributed_stream::{StreamItem, StreamTx};


pub trait TxCore {
    type MsgIn<'a>;
    type MsgOut;
    type MsgSize;


    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize;
    fn log_perodic(&mut self) -> bool;

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:TxDone , tel:& mut SteadyTelemetrySend<LEN>);
    fn monitor_not(&mut self);
    fn shared_capacity(&self) -> usize;
    fn shared_is_full(&self) -> bool;
    fn shared_is_empty(&self) -> bool;
    fn shared_vacant_units(&self) -> usize;
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool;
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool;
    async fn shared_wait_empty(&mut self) -> bool;
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut>;
    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>,) -> Result<(), Self::MsgOut>;
    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), Self::MsgOut>;


    }

impl<T> TxCore for Tx<T> {

    type MsgIn<'a> = T;
    type MsgOut = T;
    type MsgSize = usize;

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
              self.local_index = tel.process_event(self.local_index, self.channel_meta_data.id, d as isize),
            TxDone::Stream(i,_p) => {
                warn!("internal error should have gotten Normal");
                self.local_index = tel.process_event(self.local_index, self.channel_meta_data.id, i as isize)
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.local_index = MONITOR_NOT;
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
        self.tx.vacant_len()
    }

    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let safe_count = count.min(self.tx.capacity().into());
                let mut operation = &mut self.tx.wait_vacant(safe_count);
                select! { _ = one_down => false, _ = operation => true, }
            } else {
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
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        debug_assert!(!self.make_closed.is_none(),"Send called after channel marked closed");

        match self.tx.try_push(msg) {
            Ok(_) => Ok(TxDone::Normal(1)),
            Err(m) => Err(m),
        }
    }

    #[inline]
    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>,) -> Result<(), Self::MsgOut> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }

        match self.tx.try_push(msg) {
            Ok(_) => Ok(()),
            Err(msg) => {
                match saturation {
                    SendSaturation::IgnoreAndWait => {
                    }
                    SendSaturation::IgnoreAndErr => {
                        return Err(msg);
                    }
                    SendSaturation::Warn => {
                        self.report_tx_full_warning(ident);
                    }
                    SendSaturation::IgnoreInRelease => {
                        #[cfg(debug_assertions)]
                        {
                            self.report_tx_full_warning(ident);
                        }
                    }
                }
                if let Some(timeout) = timeout {
                    let has_room = self.tx.wait_vacant(1).fuse();
                    pin_mut!(has_room);
                    let mut timeout_future = Delay::new(timeout).fuse();
                    select! {
                        _ = has_room => {
                            let result = self.tx.push(msg).await;
                            match result {
                                Ok(_) => Ok(()),
                                Err(t) => {
                                    error!("channel is closed");
                                    Err(t)
                                }
                            }
                        },
                        _ = timeout_future => Err(msg),
                    }
                } else {
                    //NOTE: may block here on shutdown if graph is built poorly
                    match self.tx.push(msg).await {
                        Ok(_) => Ok(()),
                        Err(t) => {
                            error!("channel is closed");
                            Err(t)
                        }
                    }
                }
            }
        }
    }


    #[inline]
    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), Self::MsgOut> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }

        match self.tx.try_push(msg) {
            Ok(_) => Ok(()),
            Err(msg) => {
                match saturation {
                    SendSaturation::IgnoreAndWait => {
                    }
                    SendSaturation::IgnoreAndErr => {
                        return Err(msg);
                    }
                    SendSaturation::Warn => {
                        self.report_tx_full_warning(ident);
                    }
                    SendSaturation::IgnoreInRelease => {
                        #[cfg(debug_assertions)]
                        {
                            self.report_tx_full_warning(ident);
                        }
                    }
                }
                //NOTE: may block here on shutdown if graph is built poorly
                match self.tx.push(msg).await {
                    Ok(_) => Ok(()),
                    Err(t) => {
                        error!("channel is closed");
                        Err(t)
                    }
                }
            }
        }
    }

}

impl<T: StreamItem> TxCore for StreamTx<T> {
    type MsgIn<'a> = (T, &'a[u8]);
    type MsgOut = T;
    type MsgSize = (usize, usize);


    fn log_perodic(&mut self) -> bool {
        if self.item_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.item_channel.last_error_send = Instant::now();
            true
        }
    }

    fn shared_send_iter_until_full<'a,I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        debug_assert!(!self.item_channel.make_closed.is_none(),"Send called after channel marked closed");
        debug_assert!(!self.payload_channel.make_closed.is_none(),"Send called after channel marked closed");

        let mut count = 0;

        //while we have room keep going.
        let item_limit = self.item_channel.tx.vacant_len();
        let limited_iter = iter
            .take(item_limit);

        for (item,payload) in limited_iter {
            assert_eq!(item.length(),payload.len() as i32);
            if payload.len() > self.payload_channel.tx.vacant_len() {
                warn!("the payload of the stream should be larger we need {} but found {}",payload.len(),self.payload_channel.tx.vacant_len());
                block_on(self.payload_channel.tx.wait_vacant(payload.len()));
            }
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.item_channel.tx.try_push(item);
            count += 1;
        }
        count

    }

    fn telemetry_inc<const LEN:usize>(&mut self, done_count:TxDone , tel:& mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(i) => {
                warn!("internal error should have gotten Stream");
                self.item_channel.local_index = tel.process_event(self.item_channel.local_index, self.item_channel.channel_meta_data.id, i as isize);
            },
            TxDone::Stream(i,p) => {
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

     #[inline]
    fn shared_capacity(&self) -> usize {
        self.item_channel.tx.capacity().get()
    }

    #[inline]
    fn shared_is_full(&self) -> bool {
        self.item_channel.tx.is_full()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.item_channel.tx.is_empty()
    }

    #[inline]
    fn shared_vacant_units(&self) -> usize {
        self.item_channel.tx.vacant_len()
    }

    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count:  Self::MsgSize) -> bool {
        if self.item_channel.tx.vacant_len() >= count.0 &&
            self.payload_channel.tx.vacant_len() >= count.1 {
            true
        } else {
            let icap = self.item_channel.capacity();
            let pcap = self.payload_channel.capacity();

            let mut one_down = &mut self.item_channel.oneshot_shutdown;
            if !one_down.is_terminated() {
                let operation = async {
                                       self.item_channel.tx.wait_vacant(count.0.min(icap)).await;
                                       self.payload_channel.tx.wait_vacant(count.1.min(pcap)).await;
                                   };
                select! { _ = one_down => false, _ = operation.fuse() => true, }
            } else {
                false
            }
        }
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.item_channel.tx.vacant_len() >= count.0 &&
            self.payload_channel.tx.vacant_len() >= count.1 {
            true
        } else {
            self.item_channel.tx.wait_vacant(count.0.min(self.item_channel.capacity())).await;
            self.payload_channel.tx.wait_vacant(count.1.min(self.payload_channel.capacity())).await;
            true
        }
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.item_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.item_channel.tx.wait_vacant(usize::from(self.item_channel.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.item_channel.tx.capacity().get() == self.item_channel.tx.vacant_len()
        }
    }

    #[inline]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        let (item,payload) = msg;
        assert_eq!(item.length(),payload.len() as i32);

        debug_assert!(!self.item_channel.make_closed.is_none(),"Send called after channel marked closed");
        debug_assert!(!self.payload_channel.make_closed.is_none(),"Send called after channel marked closed");

        if self.payload_channel.tx.vacant_len() >= item.length() as usize &&
           self.item_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.item_channel.tx.try_push(item);
            Ok(TxDone::Stream(1,payload.len()))
        } else {
            Err(item)
        }
    }

    #[inline]
    async fn shared_send_async_timeout(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> Result<(), Self::MsgOut> {
        let (item, payload) = msg;
        assert_eq!(item.length(), payload.len() as i32);

        debug_assert!(
            !self.item_channel.make_closed.is_none(),
            "Send called after channel marked closed"
        );
        debug_assert!(
            !self.payload_channel.make_closed.is_none(),
            "Send called after channel marked closed"
        );

        // Do the quick push attempt in a block so that the mutable borrows end before any await.
        let push_result = {
            let payload_tx = &mut self.payload_channel.tx;
            let item_tx = &mut self.item_channel.tx;
            if payload_tx.vacant_len() >= item.length() as usize && item_tx.vacant_len() >= 1 {
                let _ = payload_tx.push_slice(payload);
                let _ = item_tx.try_push(item);
                Ok(())
            } else {
                Err(msg)
            }
        };

        match push_result {
            Ok(_) => Ok(()),
            Err((item,payload)) => {
                match saturation {
                    SendSaturation::IgnoreAndWait => { /* fallthrough */ }
                    SendSaturation::IgnoreAndErr => {
                        return Err(item);
                    }
                    SendSaturation::Warn => {
                        self.item_channel.report_tx_full_warning(ident);
                        self.payload_channel.report_tx_full_warning(ident);
                    }
                    SendSaturation::IgnoreInRelease => {
                        #[cfg(debug_assertions)]
                        {
                            self.item_channel.report_tx_full_warning(ident);
                            self.payload_channel.report_tx_full_warning(ident);
                        }
                    }
                }
                if let Some(timeout) = timeout {
                    let has_room = async {
                        self.payload_channel.tx.wait_vacant(payload.len()).await;
                        self.item_channel.tx.wait_vacant(1).await;
                    }.fuse();
                    pin_mut!(has_room);
                    let mut timeout_future = Delay::new(timeout).fuse();

                    select! {
                    _ = has_room => {
                        {
                            // Borrow the payload channel in its own block:
                            let got_all = self.payload_channel.tx.push_slice(&payload) == payload.len();
                            if !got_all {
                                error!("channel is closed");
                                return Err(item);
                            }
                        } // payload_tx borrow dropped here

                        {
                            // Now borrow the item channel:
                            let item_tx = &mut self.item_channel.tx;
                            match item_tx.push(item).await {
                                Ok(_) => return Ok(()),
                                Err(t) => {
                                    error!("channel is closed");
                                    return Err(t);
                                }
                            }
                        }
                    },
                    _ = timeout_future => Err(item),
                }
                } else {
                    {
                        let payload_tx = &mut self.payload_channel.tx;
                        payload_tx.wait_vacant(payload.len()).await;
                        let pushed = payload_tx.push_slice(&payload) == payload.len();
                        if !pushed {
                            error!("channel is closed");
                            return Err(item);
                        }
                    }
                    {
                        let item_tx = &mut self.item_channel.tx;
                        match item_tx.push(item).await {
                            Ok(_) => Ok(()),
                            Err(t) => {
                                error!("channel is closed");
                                Err(t)
                            }
                        }
                    }
                }
            }
        }
    }


    #[inline]
    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), Self::MsgOut> {
        let (item,payload) = msg;
        assert_eq!(item.length(),payload.len() as i32);

        debug_assert!(!self.item_channel.make_closed.is_none(),"Send called after channel marked closed");
        debug_assert!(!self.payload_channel.make_closed.is_none(),"Send called after channel marked closed");

        let push_result = if self.payload_channel.tx.vacant_len() >= item.length() as usize &&
            self.item_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.item_channel.tx.try_push(item);
            Ok(())
        } else {
            Err(msg)
        };

        match push_result {
            Ok(_) => Ok(()),
            Err((item,payload)) => {
                match saturation {
                    SendSaturation::IgnoreAndWait => {
                    }
                    SendSaturation::IgnoreAndErr => {
                        return Err(item);
                    }
                    SendSaturation::Warn => {
                        self.item_channel.report_tx_full_warning(ident);
                        self.payload_channel.report_tx_full_warning(ident);
                    }
                    SendSaturation::IgnoreInRelease => {
                        #[cfg(debug_assertions)]
                        {
                            self.item_channel.report_tx_full_warning(ident);
                            self.payload_channel.report_tx_full_warning(ident);
                        }
                    }
                }
                //NOTE: may block here on shutdown if graph is built poorly
                self.payload_channel.tx.wait_vacant(payload.len()).await;
                match self.payload_channel.tx.push_slice(&payload)==payload.len() {
                    true => {
                        match self.item_channel.tx.push(item).await {
                            Ok(_) => Ok(()),
                            Err(t) => {
                                error!("channel is closed");
                                Err(t)
                            }
                        }
                    }
                    false => {
                        error!("channel is closed");
                        Err(item)
                    }
                }
            }
        }
    }

}

impl<T: TxCore> TxCore for MutexGuard<'_, T> {
    type MsgIn<'a> = <T as TxCore>::MsgIn<'a>;
    type MsgOut = <T as TxCore>::MsgOut;
    type MsgSize =  <T as TxCore>::MsgSize;

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

    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        <T as TxCore>::shared_try_send(&mut **self, msg)
    }

    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>) -> Result<(), Self::MsgOut> {
        <T as TxCore>::shared_send_async_timeout(&mut **self,msg,ident,saturation,timeout).await
    }

    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), Self::MsgOut> {
         <T as TxCore>::shared_send_async(&mut **self,msg,ident,saturation).await
    }
}