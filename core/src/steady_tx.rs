use std::sync::Arc;
use log::{error, trace, warn};
use futures_util::{FutureExt, select};
use std::any::type_name;
use std::backtrace::Backtrace;
use std::time::{Duration, Instant};
use futures::channel::oneshot;
use futures_util::lock::{MutexGuard, MutexLockFuture};
use ringbuf::traits::Observer;
use ringbuf::producer::Producer;
use futures_util::future::{FusedFuture, select_all};
use async_ringbuf::producer::AsyncProducer;
use std::fmt::Debug;
use std::ops::Deref;
use std::thread;
use futures::pin_mut;
use futures_timer::Delay;
use crate::{steady_config, ActorIdentity, SendSaturation, SteadyTx, SteadyTxBundle, TxBundle};
use crate::channel_builder::InternalSender;
use crate::distributed::steady_stream::{StreamItem, StreamTx};
use crate::monitor::{ChannelMetaData, TxMetaData};

/// The `Tx` struct represents a transmission channel for messages of type `T`.
/// It provides methods to send messages to the channel, check the channel's state, and handle the transmission lifecycle.
pub struct Tx<T> {
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize, //set on first usage
    pub(crate) last_error_send: Instant,
    pub(crate) make_closed: Option<oneshot::Sender<()>>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
}



impl<T> Tx<T> {

    /// Returns the unique identifier of the transmission channel.
    ///
    /// # Returns
    /// A `usize` representing the channel's unique ID.
    pub fn id(&self) -> usize {
        self.channel_meta_data.id
    }

    /// Marks the channel as closed, indicating no more messages are expected.
    ///
    /// # Returns
    /// `true` if the channel was successfully marked as closed, otherwise `false`.
    pub fn mark_closed(&mut self) -> bool {
        if let Some(c) = self.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                trace!("close called but the receiver already dropped");
            }
        } 
        true // always returns true, close request is never rejected by this method.        
    }

    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// This method is useful for understanding the size constraints of the channel
    /// and for configuring buffer sizes or for performance tuning.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }
 


    /// Checks if the channel is currently full.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    ///
    /// # Example Usage
    /// Can be used to avoid calling `try_send` on a full channel, or to implement backpressure mechanisms.
    pub fn is_full(&self) -> bool {
        self.shared_is_full()
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Returns
    /// `true` if the channel is empty
    ///    
    pub fn is_empty(&self) -> bool {
        self.shared_is_empty()
    }

    /// Returns the number of vacant units in the channel.
    /// Indicates how many more messages the channel can accept before becoming full.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    ///
    /// # Example Usage
    /// Useful for dynamically adjusting the rate of message sends or for implementing custom backpressure strategies.
    pub fn vacant_units(&self) -> usize {
        self.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Example Usage
    /// Use this method to delay message sending until there's sufficient space, suitable for scenarios where message delivery must be paced or regulated.
    pub async fn wait_vacant_units(&mut self, count: usize) -> bool {
        self.shared_wait_shutdown_or_vacant_units(count).await
    }

    /// Asynchronously waits until the channel is empty.
    /// This method can be used to ensure that all messages have been processed before performing further actions.
    ///
    /// # Example Usage
    /// Ideal for scenarios where a clean state is required before proceeding, such as before shutting down a system or transitioning to a new state.
    pub async fn wait_empty(&mut self) -> bool {
        self.shared_wait_empty().await
    }

    #[inline]
    pub(crate) fn shared_send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        self.tx.push_iter(iter)
    }

    #[inline]
    pub(crate) fn shared_send_slice_until_full(&mut self, slice: &[T]) -> usize
    where T: Copy {
        debug_assert!(!self.make_closed.is_none(),"Send called after channel marked closed");
        self.tx.push_slice(slice)
    }


    

    fn report_tx_full_warning(&mut self, ident: ActorIdentity) {
        if self.last_error_send.elapsed().as_secs() > steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            let type_name = type_name::<T>().split("::").last();
            warn!("{:?} tx full channel #{} {:?} cap:{:?} type:{:?} ",
                  ident, self.channel_meta_data.id, self.channel_meta_data.labels,
                  self.tx.capacity(), type_name);
            self.last_error_send = Instant::now();
        }
    }
}

/// A trait representing a definition for a transmission channel.
pub trait TxDef: Debug {
    /// Retrieves the metadata for the transmission channel.
    fn meta_data(&self) -> TxMetaData;
}

impl<T> TxDef for SteadyTx<T> {
    fn meta_data(&self) -> TxMetaData {
        let mut count = 0;
        loop {
            if let Some(guard) = self.try_lock() {
                return TxMetaData(guard.deref().channel_meta_data.clone());
            }
            thread::yield_now();
            count += 1;

            //only print once we have tried for a while
            if 10000 == count {
                let backtrace = Backtrace::capture();
                eprintln!("{:?}", backtrace);
                error!("got stuck on meta_data, unable to get lock on ChannelMetaData");
            }
        }
    }
}

/// A trait for handling bundles of transmission channels.
pub trait SteadyTxBundleTrait<T, const GIRTH: usize> {
    /// Locks all channels in the bundle, returning a future that resolves when all locks are acquired.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>>;

    /// Retrieves the metadata for all transmission channels in the bundle.
    fn meta_data(&self) -> [TxMetaData; GIRTH];

    /// Waits until a specified number of units are vacant in the channels.
    ///
    /// # Parameters
    /// - `avail_count`: The number of vacant units to wait for.
    /// - `ready_channels`: The number of channels that should have the vacant units.
    fn wait_vacant_units(&self, avail_count: usize, ready_channels: usize) -> impl std::future::Future<Output = ()>;
}

impl<T: Sync + Send, const GIRTH: usize> SteadyTxBundleTrait<T, GIRTH> for SteadyTxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn meta_data(&self) -> [TxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data())
            .collect::<Vec<TxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

    async fn wait_vacant_units(&self, avail_count: usize, ready_channels: usize) {
        let futures = self.iter().map(|tx| {
            let tx = tx.clone();
            async move {
                let mut tx = tx.lock().await;
                tx.wait_vacant_units(avail_count).await;
            }
                .boxed()
        });

        let mut futures: Vec<_> = futures.collect();
        let mut count_down = ready_channels.min(GIRTH);

        while !futures.is_empty() {
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if count_down == 0 {
                break;
            }
        }
    }
}

/// A trait for handling transmission channel bundles.
pub trait TxBundleTrait {
    /// Marks all channels in the bundle as closed.
    fn mark_closed(&mut self) -> bool;


}

impl<T> TxBundleTrait for TxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        //NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {let _ = f.mark_closed();});
        true  // always returns true, close request is never rejected by this method.
    }

}

/////////////////////////////////////////////////////////////////

pub trait TxCore {
    type MsgIn<'a>;
    type MsgOut;

    fn shared_capacity(&self) -> usize;

    fn shared_is_full(&self) -> bool;

    fn shared_is_empty(&self) -> bool;

    fn shared_vacant_units(&self) -> usize;

    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: usize) -> bool;

    async fn shared_wait_vacant_units(&mut self, count: usize) -> bool;

    async fn shared_wait_empty(&mut self) -> bool;

    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<(), Self::MsgOut>;

    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>,) -> Result<(), Self::MsgOut>;

    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), Self::MsgOut>;


    }

impl<T> TxCore for Tx<T> {

    type MsgIn<'a> = T;
    type MsgOut = T;

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
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: usize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let mut operation = &mut self.tx.wait_vacant(count);
                select! { _ = one_down => false, _ = operation => true, }
            } else {
                false
            }
        }
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: usize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let operation = &mut self.tx.wait_vacant(count);
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
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<(), Self::MsgOut> {
        debug_assert!(!self.make_closed.is_none(),"Send called after channel marked closed");

        match self.tx.try_push(msg) {
            Ok(_) => Ok(()),
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
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: usize) -> bool {
        if self.item_channel.tx.vacant_len() >= count {
            true
        } else {
            let mut one_down = &mut self.item_channel.oneshot_shutdown;
            if !one_down.is_terminated() {
                let mut operation = &mut self.item_channel.tx.wait_vacant(count);
                select! { _ = one_down => false, _ = operation => true, }
            } else {
                false
            }
        }
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: usize) -> bool {
        if self.item_channel.tx.vacant_len() >= count {
            true
        } else {
            let operation = &mut self.item_channel.tx.wait_vacant(count);
            operation.await;
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
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<(), Self::MsgOut> {
        let (item,payload) = msg;
        assert_eq!(item.length(),payload.len() as i32);

        debug_assert!(!self.item_channel.make_closed.is_none(),"Send called after channel marked closed");
        debug_assert!(!self.payload_channel.make_closed.is_none(),"Send called after channel marked closed");

        if self.payload_channel.tx.vacant_len() >= item.length() as usize &&
           self.item_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.item_channel.tx.try_push(item);
            Ok(())
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
            Err(msg) => {
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
                    let has_room = self.shared_wait_vacant_units(1).fuse();
                    pin_mut!(has_room);
                    let mut timeout_future = Delay::new(timeout).fuse();

                    select! {
                    _ = has_room => {
                        {
                            // Borrow the payload channel in its own block:
                            let payload_tx = &mut self.payload_channel.tx;
                            payload_tx.wait_vacant(payload.len()).await;
                            let got_all = payload_tx.push_slice(&payload) == payload.len();
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
            Err(msg) => {
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
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: usize) -> bool {
        <T as TxCore>::shared_wait_shutdown_or_vacant_units(&mut **self, count).await
    }

    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: usize) -> bool {
        <T as TxCore>::shared_wait_vacant_units(&mut **self,count).await
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        <T as TxCore>::shared_wait_empty(&mut **self).await
    }

    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<(), Self::MsgOut> {
        <T as TxCore>::shared_try_send(&mut **self, msg)
    }

    async fn shared_send_async_timeout(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation, timeout: Option<Duration>) -> Result<(), Self::MsgOut> {
        <T as TxCore>::shared_send_async_timeout(&mut **self,msg,ident,saturation,timeout).await
    }

    async fn shared_send_async(&mut self, msg: Self::MsgIn<'_>, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), Self::MsgOut> {
         <T as TxCore>::shared_send_async(&mut **self,msg,ident,saturation).await
    }
}
