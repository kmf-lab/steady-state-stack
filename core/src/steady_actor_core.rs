//! Core implementations for the SteadyActor trait that are identical between
//! `SteadyActorShadow` and `SteadyActorSpotlight`.
//!
//! This module defines `SteadyActorCore`, a stateless helper struct whose methods
//! implement the low-level channel operations (calling `TxCore` / `RxCore`) without
//! any telemetry instrumentation. Both the shadow and spotlight can delegate to
//! these methods, eliminating code duplication.

use std::time::Duration;
use futures::channel::oneshot;
use futures_util::future::{FusedFuture, Shared};
use futures_util::{select, FutureExt};
use futures_timer::Delay;
use parking_lot::RwLock;
use std::sync::Arc;
use crate::{
    ActorIdentity, GraphLiveliness, Rx, RxCore, RxDone, SendOutcome,
    SendSaturation, Tx, TxCore, TxDone,
};
use crate::yield_now;

/// Core methods that are common to both actor lifecycle implementations.
///
/// This struct holds no state; it is a collection of freestanding functions
/// that operate on `TxCore`/`RxCore` references and the lifecycle primitives
/// (`oneshot_shutdown`, `runtime_state`, etc.) passed explicitly.
pub struct SteadyActorCore;

impl SteadyActorCore {
    /// Returns `true` if the actor should keep running, `false` if it should stop.
    /// `accept_fn` is called only when `StopRequested` to determine the actor’s vote.
    #[inline]
    pub fn is_running<F: FnMut() -> bool>(
        runtime_state: &parking_lot::RwLock<GraphLiveliness>,
        ident: ActorIdentity,
        accept_fn: F,
    ) -> bool {
        let liveliness = runtime_state.read();
        liveliness.is_running(ident, accept_fn).unwrap_or(true)
    }

    /// Request shutdown via the graph liveliness state.
    /// Accepts a reference to the `Arc<RwLock<>>` and clones it.
    pub async fn request_shutdown(runtime_state: &Arc<RwLock<GraphLiveliness>>) {
        GraphLiveliness::internal_request_shutdown(runtime_state.clone()).await;
    }

    // ── RxCore delegation ─────────────────────────────────────────────────

    #[inline]
    pub fn peek_slice<'b, T: RxCore>(
        this: &'b mut T,
    ) -> T::SliceSource<'b> {
        this.shared_peek_slice()
    }

    #[inline]
    pub fn take_slice<T: RxCore>(
        this: &mut T,
        target: T::SliceTarget<'_>,
    ) -> RxDone
    where
        T::MsgItem: Copy,
    {
        this.shared_take_slice(target)
    }

    #[inline]
    pub fn advance_take_index<T: RxCore>(
        this: &mut T,
        count: T::MsgSize,
    ) -> RxDone {
        this.shared_advance_index(count)
    }

    #[inline]
    pub fn try_peek<'a, T>(this: &'a mut Rx<T>) -> Option<&'a T> {
        this.shared_try_peek()
    }

    #[inline]
    pub fn try_peek_iter<'a, T>(this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        this.shared_try_peek_iter()
    }

    #[inline]
    pub fn is_empty<T: RxCore>(this: &mut T) -> bool {
        this.shared_is_empty()
    }

    #[inline]
    pub fn avail_units<T: RxCore>(this: &mut T) -> T::MsgSize {
        this.shared_avail_units()
    }

    #[inline]
    pub fn try_take<T: RxCore>(this: &mut T) -> Option<T::MsgOut> {
        match this.shared_try_take() {
            Some((_done, msg)) => Some(msg),
            None => None,
        }
    }

    #[inline]
    pub fn take_into_iter<'a, T: Sync + Send>(
        this: &'a mut Rx<T>,
    ) -> impl Iterator<Item = T> + 'a {
        this.shared_take_into_iter()
    }

    // ── TxCore delegation ─────────────────────────────────────────────────

    #[inline]
    pub fn send_slice<T: TxCore>(
        this: &mut T,
        slice: T::SliceSource<'_>,
    ) -> TxDone
    where
        T::MsgOut: Copy,
    {
        this.shared_send_slice(slice)
    }

    #[inline]
    pub fn poke_slice<'b, T: TxCore>(
        this: &'b mut T,
    ) -> T::SliceTarget<'b> {
        this.shared_poke_slice()
    }

    #[inline]
    pub fn advance_send_index<T: TxCore>(
        this: &mut T,
        count: T::MsgSize,
    ) -> TxDone {
        this.shared_advance_index(count)
    }

    #[inline]
    pub fn send_iter_until_full<T, I: Iterator<Item = T>>(
        this: &mut Tx<T>,
        iter: I,
    ) -> usize {
        this.shared_send_iter_until_full(iter)
    }

    #[inline]
    pub fn try_send<T: TxCore>(
        this: &mut T,
        msg: T::MsgIn<'_>,
    ) -> SendOutcome<T::MsgOut> {
        match this.shared_try_send(msg) {
            Ok(_) => SendOutcome::Success,
            Err(msg) => SendOutcome::Blocked(msg),
        }
    }

    #[inline]
    pub fn is_full<T: TxCore>(this: &mut T) -> bool {
        this.shared_is_full()
    }

    #[inline]
    pub fn vacant_units<T: TxCore>(this: &mut T) -> T::MsgSize {
        this.shared_vacant_units()
    }

    /// Low-level async send (no telemetry wrappers).
    pub async fn send_async<T: TxCore>(
        this: &mut T,
        msg: T::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<T::MsgOut> {
        this.shared_send_async(msg, ident, saturation).await
    }

    // ── Asynchronous wait helpers (shared by both spotlight and shadow) ───

    /// Wait for the shutdown signal to be received.
    pub async fn wait_shutdown(oneshot_shutdown: &Shared<oneshot::Receiver<()>>) -> bool {
        let _ = oneshot_shutdown.clone().await;
        true
    }

    /// Wait for a specified duration, aborting early if shutdown fires.
    pub async fn wait(
        oneshot_shutdown: &Shared<oneshot::Receiver<()>>,
        duration: Duration,
    ) {
        let mut shutdown_fused = oneshot_shutdown.clone().fuse();
        futures::pin_mut!(shutdown_fused);
        let delay = Delay::new(duration).fuse();
        futures::pin_mut!(delay);
        select! {
            _ = shutdown_fused => {},
            _ = delay => {},
        }
    }

    /// Yield execution to the runtime.
    pub async fn yield_now() {
        yield_now().await;
    }

    /// Wait for a future to complete, aborting if shutdown fires.
    pub async fn wait_future_void<F>(
        oneshot_shutdown: &Shared<oneshot::Receiver<()>>,
        fut: F,
    ) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync,
    {
        let mut shutdown_fused = oneshot_shutdown.clone().fuse();
        futures::pin_mut!(shutdown_fused);
        let mut pinned_fut = Box::pin(fut);
        select! {
            _ = shutdown_fused => false,
            _ = pinned_fut => true,
        }
    }

    /// Call an async function, with a timeout based on shutdown.
    pub async fn call_async<F, T>(
        oneshot_shutdown: &Shared<oneshot::Receiver<()>>,
        is_liveliness_shutdown_timeout: Option<Duration>,
        operation: F,
    ) -> Option<T>
    where
        F: Future<Output = T>,
    {
        let operation = operation.fuse();
        futures::pin_mut!(operation);

        let mut shutdown_fused = oneshot_shutdown.clone().fuse();
        futures::pin_mut!(shutdown_fused);

        // Poll shutdown once to see if it's already ready.
        if futures::poll!(&mut shutdown_fused).is_ready() {
            // Shutdown has already been requested.
            if let Some(duration) = is_liveliness_shutdown_timeout {
                // Allow a grace period for the operation to complete.
                let timeout = Delay::new(duration.div_f32(4f32)).fuse();
                futures::pin_mut!(timeout);
                select! {
                    _ = timeout => None,
                    r = operation => Some(r),
                }
            } else {
                None
            }
        } else {
            // Shutdown not yet fired; wait for either.
            select! {
                _ = shutdown_fused => None,
                r = operation => Some(r),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod steady_actor_core_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::core_exec;
    use crate::core_rx::{DoubleSlice, RxCore};
    use crate::core_tx::TxCore;
    use crate::steady_rx::Rx;
    use crate::steady_tx::Tx;
    use crate::{ActorIdentity, GraphBuilder, SendSaturation, SendOutcome, RxDone, TxDone};
    use std::time::Duration;
    use futures::channel::oneshot;
    use futures_util::future::Shared;
    use parking_lot::RwLock;

    /// Helper to create a simple channel with capacity 10.
    fn make_channel() -> (Tx<i32>, Rx<i32>) {
        let builder = ChannelBuilder::default().with_capacity(10);
        builder.eager_build_internal()
    }

    /// Helper to create a terminated oneshot (shutdown already fired).
    fn terminated_shutdown() -> Shared<oneshot::Receiver<()>> {
        let (tx, rx) = oneshot::channel();
        drop(tx);
        rx.shared()
    }

    /// Helper to create a pending oneshot (shutdown not fired).
    /// Returns the sender so the caller can keep it alive.
    fn pending_shutdown() -> (oneshot::Sender<()>, Shared<oneshot::Receiver<()>>) {
        let (tx, rx) = oneshot::channel::<()>();
        let shared = rx.shared();
        (tx, shared)
    }

    // ── RxCore delegation tests ───────────────────────────────────────────

    #[test]
    fn test_peek_slice() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![1, 2, 3].into_iter());
        let slice = SteadyActorCore::peek_slice(&mut rx);
        assert_eq!(slice.total_len(), 3);
        assert_eq!(slice.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn test_take_slice() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![10, 20, 30].into_iter());
        let mut buf = [0i32; 2];
        let done = SteadyActorCore::take_slice(&mut rx, &mut buf);
        assert_eq!(done, RxDone::Normal(2));
        assert_eq!(buf, [10, 20]);
    }

    #[test]
    fn test_advance_take_index() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![1, 2, 3].into_iter());
        let done = SteadyActorCore::advance_take_index(&mut rx, 2);
        assert_eq!(done, RxDone::Normal(2));
        assert_eq!(rx.shared_avail_units(), 1);
    }

    #[test]
    fn test_try_peek() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![42].into_iter());
        let peeked = SteadyActorCore::try_peek(&mut rx);
        assert_eq!(peeked, Some(&42));
    }

    #[test]
    fn test_try_peek_iter() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![1, 2, 3].into_iter());
        let collected: Vec<&i32> = SteadyActorCore::try_peek_iter(&mut rx).collect();
        assert_eq!(collected, vec![&1, &2, &3]);
    }

    #[test]
    fn test_is_empty() {
        let (_, mut rx) = make_channel();
        assert!(SteadyActorCore::is_empty(&mut rx));
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![1].into_iter());
        assert!(!SteadyActorCore::is_empty(&mut rx));
    }

    #[test]
    fn test_avail_units() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![1, 2].into_iter());
        assert_eq!(SteadyActorCore::avail_units(&mut rx), 2);
    }

    #[test]
    fn test_try_take() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![99].into_iter());
        let taken = SteadyActorCore::try_take(&mut rx);
        assert_eq!(taken, Some(99));
        assert!(SteadyActorCore::try_take(&mut rx).is_none());
    }

    #[test]
    fn test_take_into_iter() {
        let (mut tx, mut rx) = make_channel();
        tx.shared_send_iter_until_full(vec![1, 2, 3].into_iter());
        let collected: Vec<i32> = SteadyActorCore::take_into_iter(&mut rx).collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    // ── TxCore delegation tests ───────────────────────────────────────────

    #[test]
    fn test_send_slice() {
        let (mut tx, _rx) = make_channel();
        let done = SteadyActorCore::send_slice(&mut tx, &[5, 6, 7]);
        assert_eq!(done, TxDone::Normal(3));
    }

    #[test]
    fn test_poke_slice() {
        let (mut tx, _rx) = make_channel();
        let (a, b) = SteadyActorCore::poke_slice(&mut tx);
        // Just ensure it returns slices; we can't easily write to MaybeUninit in a test.
        assert!(a.len() + b.len() > 0);
    }

    #[test]
    fn test_advance_send_index() {
        let (mut tx, _rx) = make_channel();
        let done = SteadyActorCore::advance_send_index(&mut tx, 3);
        assert_eq!(done, TxDone::Normal(3));
    }

    #[test]
    fn test_send_iter_until_full() {
        let (mut tx, _rx) = make_channel();
        let sent = SteadyActorCore::send_iter_until_full(&mut tx, vec![1, 2, 3].into_iter());
        assert_eq!(sent, 3);
    }

    #[test]
    fn test_try_send() {
        let (mut tx, _rx) = make_channel();
        let outcome = SteadyActorCore::try_send(&mut tx, 42);
        assert!(matches!(outcome, SendOutcome::Success));
    }

    #[test]
    fn test_is_full() {
        let (mut tx, _rx) = make_channel();
        assert!(!SteadyActorCore::is_full(&mut tx));
        // Fill the channel
        for i in 0..10 {
            let _ = tx.shared_try_send(i);
        }
        assert!(SteadyActorCore::is_full(&mut tx));
    }

    #[test]
    fn test_vacant_units() {
        let (mut tx, _rx) = make_channel();
        assert_eq!(SteadyActorCore::vacant_units(&mut tx), 10);
        let _ = tx.shared_try_send(1);
        assert_eq!(SteadyActorCore::vacant_units(&mut tx), 9);
    }

    #[test]
    fn test_send_async() {
        let (mut tx, _rx) = make_channel();
        let ident = ActorIdentity::new(0, "test", None);
        let outcome = core_exec::block_on(SteadyActorCore::send_async(
            &mut tx,
            42,
            ident,
            SendSaturation::AwaitForRoom,
        ));
        assert!(matches!(outcome, SendOutcome::Success));
    }

    // ── Async wait helpers tests ──────────────────────────────────────────

    #[test]
    fn test_wait_shutdown() {
        let shutdown = terminated_shutdown();
        let result = core_exec::block_on(SteadyActorCore::wait_shutdown(&shutdown));
        assert!(result);
    }

    #[test]
    fn test_wait() {
        let (_sender, shutdown) = pending_shutdown();
        let start = std::time::Instant::now();
        core_exec::block_on(SteadyActorCore::wait(&shutdown, Duration::from_millis(50)));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn test_wait_shutdown_aborts() {
        let shutdown = terminated_shutdown();
        let start = std::time::Instant::now();
        core_exec::block_on(SteadyActorCore::wait(&shutdown, Duration::from_secs(10)));
        // Should return immediately because shutdown is already terminated.
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn test_yield_now() {
        core_exec::block_on(SteadyActorCore::yield_now());
    }
}
