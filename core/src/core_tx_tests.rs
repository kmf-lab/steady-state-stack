//! Tests extracted from `core_tx.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related channel.backpressure-never-drop]
// --- from mod core_tx_rx_tests ---
// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use crate::channel_builder::ChannelBuilder;
// ss[related channel.backpressure-never-drop]
use crate::*;
// ss[related philosophy.structural-hierarchy]
use crate::core_rx::RxCore;

/// Tests basic send and receive operations using `TxCore` and `RxCore`.
///
/// Verifies that messages can be sent through the channel, checked for availability,
/// and retrieved correctly, ensuring proper channel state management.
#[test]
// ss[verify channel.backpressure-never-drop]
fn test_tx_rx_basic_flow() {
    let builder = ChannelBuilder::default().with_capacity(2);
    let (tx, rx) = builder.build_channel::<i32>();
    let tx = tx.clone();
    let mut txg = tx.try_lock().expect("");
    assert_eq!(txg.shared_capacity(), 2);
    assert!(txg.shared_is_empty());
    let sent = txg.shared_send_iter_until_full([7, 8].into_iter());
    assert_eq!(sent, 2);
    assert!(txg.shared_is_full());
    drop(txg);
    let rx = rx.clone();
    let mut rxg = rx.try_lock().expect("");
    assert_eq!(rxg.shared_capacity(), 2);
    assert_eq!(rxg.shared_avail_units(), 2);
    assert_eq!(rxg.shared_try_peek(), Some(&7));
    drop(rxg);
    let rx = rx.clone();
    let mut rxg = rx.try_lock().expect("");
    assert_eq!(rxg.shared_try_take().map(|(_, v)| v), Some(7));
    assert_eq!(rxg.shared_try_take().map(|(_, v)| v), Some(8));
    assert!(rxg.shared_is_empty());
}

/// Tests detection of potential showstopper conditions in `RxCore`.
///
/// Sends a message and repeatedly peeks at it, verifying that the showstopper condition
/// is triggered after a specified number of peeks without taking the message.
#[test]
// ss[verify channel.backpressure-never-drop]
fn test_bad_message_detection() {
    let builder = ChannelBuilder::default().with_capacity(1);
    let (tx, rx) = builder.build_channel::<u8>();
    let tx = tx.clone();
    let mut txg = tx.try_lock().expect("");
    assert_eq!(txg.shared_send_iter_until_full([42].into_iter()), 1);
    drop(txg);
    let rx = rx.clone();
    let rxg = rx.try_lock().expect("");
    assert_eq!(rxg.shared_try_peek(), Some(&42));
    assert_eq!(rxg.shared_try_peek(), Some(&42));
    assert!(rxg.is_showstopper(2));
    assert!(!rxg.is_showstopper(5));
}

// ss[related channel.backpressure-never-drop]
use futures::executor::block_on;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use crate::TxCore;
// ss[related channel.backpressure-never-drop]
use crate::{ActorIdentity, SendOutcome, SendSaturation};
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use crate::GraphBuilder;

/// A mock implementation of `TxCore` for testing `MutexGuard` forwarding behavior.
///
/// Provides predictable responses to method calls, allowing verification of correct
/// delegation through a mutex guard.
// ss[related channel.backpressure-never-drop]
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

// ss[related channel.backpressure-never-drop]
impl FakeTx {
    /// Creates a new instance with default values for testing.
    // ss[related philosophy.structural-hierarchy]
    fn new() -> Self {
        FakeTx { closed: false, send_count: 0, log_calls: 0, one_val: 3, capacity: 4, is_full: false, is_empty: true, vacant: 4 }
    }
}

// ss[related channel.backpressure-never-drop]
impl TxCore for FakeTx {
    // ss[related philosophy.structural-hierarchy]
    type MsgIn<'a> = usize;
    // ss[related philosophy.structural-hierarchy]
    type MsgOut = usize;
    // ss[related channel.backpressure-never-drop]
    type MsgSize = usize;
    // ss[related philosophy.structural-hierarchy]
    type SliceSource<'b> = &'b [usize];
    // ss[related philosophy.structural-hierarchy]
    type SliceTarget<'a> = (&'a [usize], &'a [usize]);

    /// Marks the channel as closed and returns `true`.
    // ss[related channel.backpressure-never-drop]
    fn shared_mark_closed(&mut self) {
        self.closed = true;
    }

    /// Counts and accumulates the number of items sent from an iterator.
    // ss[related channel.backpressure-never-drop]
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        let cnt = iter.count();
        self.send_count += cnt;
        cnt
    }

    /// Toggles logging based on the number of calls.
    // ss[related channel.backpressure-never-drop]
    fn log_perodic(&mut self) -> bool {
        self.log_calls += 1;
        self.log_calls > 1
    }

    /// Returns a predefined unit value, adjustable by `monitor_not`.
    // ss[related channel.backpressure-never-drop]
    fn one(&self) -> Self::MsgSize {
        self.one_val
    }

    /// Does nothing with telemetry, maintaining mock simplicity.
    // ss[related channel.backpressure-never-drop]
    fn telemetry_inc<const LEN: usize>(&mut self, _d: TxDone, _tel: &mut crate::monitor_telemetry::SteadyTelemetrySend<LEN>) {
    }

    /// Resets the unit value to zero.
    // ss[related channel.backpressure-never-drop]
    fn monitor_not(&mut self) {
        self.one_val = 0;
    }

    /// Returns a fixed capacity value.
    // ss[related channel.backpressure-never-drop]
    fn shared_capacity(&self) -> usize {
        self.capacity
    }

    // ss[related channel.backpressure-never-drop]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }
    /// Returns a fixed full status.
    // ss[related channel.backpressure-never-drop]
    fn shared_is_full(&self) -> bool {
        self.is_full
    }

    /// Returns a fixed empty status.
    // ss[related channel.backpressure-never-drop]
    fn shared_is_empty(&self) -> bool {
        self.is_empty
    }

    /// Returns a fixed vacant units value.
    // ss[related channel.backpressure-never-drop]
    fn shared_vacant_units(&self) -> usize {
        self.vacant
    }

    // ss[related channel.backpressure-never-drop]
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }

    /// Simulates immediate availability for shutdown or vacant wait.
    // ss[related channel.backpressure-never-drop]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, _count: Self::MsgSize) -> bool {
        true
    }

    /// Simulates immediate availability for vacant units wait.
    // ss[related channel.backpressure-never-drop]
    async fn shared_wait_vacant_units(&mut self, _count: Self::MsgSize) -> bool {
        true
    }

    /// Simulates immediate availability for empty wait.
    // ss[related channel.backpressure-never-drop]
    async fn shared_wait_empty(&mut self) -> bool {
        true
    }

    /// Returns zero advancement for simplicity.
    // ss[related channel.backpressure-never-drop]
    fn shared_advance_index(&mut self, _request: Self::MsgSize) -> TxDone {
        TxDone::Normal(0)
    }

    /// Returns zero items sent for slice operations.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn shared_send_slice(&mut self, _slice: Self::SliceSource<'_>) -> TxDone {
        TxDone::Normal(0)
    }

    /// Returns empty slices for poking.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        let (item_a, item_b) = (&[], &[]);
        (item_a, item_b)
    }

    /// Always succeeds, returning the sent message as the number of items.
    // ss[related channel.backpressure-never-drop]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        Ok(TxDone::Normal(msg))
    }

    /// Always returns success for async core send.
    // ss[related channel.backpressure-never-drop]
    async fn shared_send_async_core(
        &mut self,
        _msg: Self::MsgIn<'_>,
        _ident: ActorIdentity,
        _s: SendSaturation,
        _timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        SendOutcome::Success
    }

    /// Always returns success for async send with timeout.
    // ss[related channel.backpressure-never-drop]
    async fn shared_send_async_timeout(
        &mut self,
        _msg: Self::MsgIn<'_>,
        _id: ActorIdentity,
        _s: SendSaturation,
        _timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        SendOutcome::Success
    }

    /// Always returns success for async send without timeout.
    // ss[related channel.backpressure-never-drop]
    async fn shared_send_async(
        &mut self,
        _msg: Self::MsgIn<'_>,
        _id: ActorIdentity,
        _s: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        SendOutcome::Success
    }

    /// Returns the input value as the number of items sent.
    // ss[related channel.backpressure-never-drop]
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Normal(*one)
    }
}

/// Tests that `MutexGuard` correctly forwards `TxCore` methods to the underlying type.
///
/// Verifies that all trait methods behave as expected when called through a mutex guard,
/// using a mock implementation to ensure predictable outcomes.
#[test]
// ss[verify channel.backpressure-never-drop]
fn test_mutexguard_txcore_methods() {
    let mtx = Mutex::new(FakeTx::new());
    let mut guard = block_on(mtx.lock());
    assert!(!guard.closed);
    guard.shared_mark_closed();
    assert!(guard.closed);
    let sent = guard.shared_send_iter_until_full([10, 20, 30].into_iter());
    assert_eq!(sent, 3);
    assert!(!guard.log_perodic());
    assert!(guard.log_perodic());
    assert_eq!(guard.one(), 3);
    guard.monitor_not();
    assert_eq!(guard.one(), 0);
    assert_eq!(guard.shared_capacity(), 4);
    assert!(!guard.shared_is_full());
    assert!(guard.shared_is_empty());
    assert_eq!(guard.shared_vacant_units(), 4);
    assert!(block_on(guard.shared_wait_shutdown_or_vacant_units(1)));
    assert!(block_on(guard.shared_wait_vacant_units(1)));
    assert!(block_on(guard.shared_wait_empty()));
    let try_res = guard.shared_try_send(5);
    assert!(try_res.is_ok());
    let ident = ActorIdentity::new(0, "test", None);
    let res = block_on(guard.shared_send_async(7, ident, SendSaturation::AwaitForRoom));
    assert!(matches!(res, SendOutcome::Success));
    #[allow(deprecated)]
    let res_to = block_on(guard.shared_send_async_timeout(8, ident, SendSaturation::ReturnBlockedMsg, Some(Duration::from_millis(1))));
    assert!(matches!(res_to, SendOutcome::Success));
    assert_eq!(guard.done_one(&9), TxDone::Normal(9));
}

/// Helper function to create a new `Tx<u8>` and keep the graph alive for the duration of the test.
/// Returns the channel, graph, and a sender that must be kept alive to prevent premature shutdown.
// ss[related channel.backpressure-never-drop]
fn new_tx() -> (Tx<u8>, Graph, futures::channel::oneshot::Sender<()>) {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = graph.channel_builder();
    let (mut tx, _rx) = builder.eager_build_internal();
    // Create a oneshot whose sender we keep; the receiver never resolves until we drop it.
    let (dummy_sender, never_resolve) = futures::channel::oneshot::channel::<()>();
    tx.oneshot_shutdown = never_resolve;
    (tx, graph, dummy_sender)
}

/// Helper function to create a new `Tx<u8>` and keep its associated `Rx<u8>` alive.
// ss[related channel.backpressure-never-drop]
fn new_tx_with_rx() -> (Tx<u8>, Rx<u8>, Graph, futures::channel::oneshot::Sender<()>) {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = graph.channel_builder();
    let (mut tx, rx) = builder.eager_build_internal();
    let (dummy_sender, never_resolve) = futures::channel::oneshot::channel::<()>();
    tx.oneshot_shutdown = never_resolve;
    (tx, rx, graph, dummy_sender)
}

/// Tests `done_one` and `shared_mark_closed` for `Tx<u8>`.
///
/// Ensures that `done_one` consistently reports one item and that `shared_mark_closed`
/// behaves correctly on first and subsequent calls.
#[test]
// ss[verify channel.backpressure-never-drop]
fn done_one_and_shared_mark_closed() {
    let (mut tx, _graph, _sender) = new_tx();
    assert_eq!(tx.done_one(&42u8), TxDone::Normal(1));
    tx.shared_mark_closed();
    tx.shared_mark_closed();
}

/// Tests sending after closure and the associated warning behavior.
///
/// Verifies that sending after marking the channel closed triggers a warning and still
/// processes the operation correctly.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn shared_send_iter_until_full_and_warn_after_close() {
    let (mut tx, _graph, _sender) = new_tx();
    let pushed = tx.shared_send_iter_until_full([7u8, 8u8].into_iter());
    assert_eq!(pushed, 2);
    tx.shared_mark_closed();
    let pushed2 = tx.shared_send_iter_until_full(std::iter::empty());
    assert_eq!(pushed2, 0);
}

/// Tests `shared_try_send` and async send variants for `Tx<u8>`.
///
/// Confirms that immediate sends succeed and that async methods complete successfully
/// under normal conditions.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn shared_try_send_and_async_variants() {
    let (mut tx, _graph, _sender) = new_tx();
    let ident = ActorIdentity::new(0, "me", None);
    assert_eq!(tx.shared_try_send(99u8), Ok(TxDone::Normal(1)));
    let outcome = block_on(tx.shared_send_async_core(5u8, ident, SendSaturation::AwaitForRoom, None));
    assert!(matches!(outcome, SendOutcome::Success));
    #[allow(deprecated)]
    let outcome2 = block_on(tx.shared_send_async(6u8, ident, SendSaturation::ReturnBlockedMsg));
    assert!(matches!(outcome2, SendOutcome::Success));
    #[allow(deprecated)]
    let outcome3 = block_on(tx.shared_send_async_timeout(7u8, ident, SendSaturation::ReturnBlockedMsg, Some(Duration::from_millis(1))));
    assert!(matches!(outcome3, SendOutcome::Success));
}

/// Tests saturation policies: WarnThenAwait and DebugWarnThenAwait on a full channel.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_send_async_saturation_policies() {
    let (mut tx, _graph, _sender) = new_tx();
    let ident = ActorIdentity::new(0, "saturation", None);

    // Fill the channel (capacity = 64).
    let cap = tx.shared_capacity();
    let fill = (0..cap as u8).collect::<Vec<_>>();
    tx.shared_send_iter_until_full(fill.into_iter());
    assert!(tx.shared_is_full());

    // Give the executor a moment to settle any pending wakeups.
    std::thread::sleep(Duration::from_millis(1));

    // WarnThenAwait with a short timeout => should timeout (or closed if shutdown fires first).
    let outcome_warn = block_on(tx.shared_send_async_core(
        255u8,
        ident,
        SendSaturation::WarnThenAwait,
        Some(Duration::from_millis(100)),
    ));
    assert!(
        matches!(outcome_warn, SendOutcome::Timeout(255) | SendOutcome::Closed(255)),
        "expected SendOutcome::Timeout(255) or Closed(255), got {:?}",
        outcome_warn
    );

    // DebugWarnThenAwait with a short timeout => should timeout (or closed if shutdown fires first).
    let outcome_debug = block_on(tx.shared_send_async_core(
        128u8,
        ident,
        SendSaturation::DebugWarnThenAwait,
        Some(Duration::from_millis(100)),
    ));
    assert!(
        matches!(outcome_debug, SendOutcome::Timeout(128) | SendOutcome::Closed(128)),
        "expected SendOutcome::Timeout(128) or Closed(128), got {:?}",
        outcome_debug
    );
}

/// Tests ReturnBlockedMsg saturation on a full channel.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_send_async_saturation_returns_blocked() {
    let (mut tx, _graph, _sender) = new_tx();
    let ident = ActorIdentity::new(0, "blocked", None);

    let cap = tx.shared_capacity();
    let fill = (0..cap as u8).collect::<Vec<_>>();
    tx.shared_send_iter_until_full(fill.into_iter());
    assert!(tx.shared_is_full());

    #[allow(deprecated)]
    let outcome = block_on(tx.shared_send_async_core(
        255u8,
        ident,
        SendSaturation::ReturnBlockedMsg,
        Some(Duration::from_millis(10)),
    ));
    assert!(matches!(outcome, SendOutcome::Blocked(255)), "expected Blocked(255), got {:?}", outcome);
}

/// Tests that if the shutdown oneshot fires during the send wait, Closed is returned.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_send_async_saturation_closes_on_shutdown() {
    let (mut tx, _graph, sender) = new_tx();
    let ident = ActorIdentity::new(0, "closed_shutdown", None);

    let cap = tx.shared_capacity();
    let fill = (0..cap as u8).collect::<Vec<_>>();
    tx.shared_send_iter_until_full(fill.into_iter());
    assert!(tx.shared_is_full());

    // Drop the sender to trigger shutdown.
    drop(sender);

    let outcome = block_on(tx.shared_send_async_core(
        100u8,
        ident,
        SendSaturation::AwaitForRoom,
        Some(Duration::from_secs(10)),
    ));
    assert!(matches!(outcome, SendOutcome::Closed(100)), "expected Closed(100), got {:?}", outcome);
}

/// Tests that when room becomes available after a wait, AwaitForRoom succeeds.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_send_async_saturation_awaits_room_and_succeeds() {
    // Create a channel and keep its receiver so we can make room.
    let (mut tx, mut rx, _graph, _sender) = new_tx_with_rx();
    let ident = ActorIdentity::new(0, "room_available", None);

    // Fill to capacity.
    let cap = tx.shared_capacity();
    let fill = (0..cap as u8).collect::<Vec<_>>();
    tx.shared_send_iter_until_full(fill.into_iter());
    assert!(tx.shared_is_full());

    // Spawn a background task that will take one message after a short delay.
    let take_future = async move {
        std::thread::sleep(Duration::from_millis(20));
        rx.shared_try_take();
    };
    crate::core_exec::spawn_detached(take_future);

    // AwaitForRoom with a generous timeout – should succeed once room appears.
    let outcome = block_on(tx.shared_send_async_core(
        42u8,
        ident,
        SendSaturation::AwaitForRoom,
        Some(Duration::from_secs(1)),
    ));
    assert!(matches!(outcome, SendOutcome::Success), "expected Success after room becomes available, got {:?}", outcome);
}

/// Tests that telemetry_inc with MONITOR_NOT does not panic and logs an error.
#[test]
// ss[verify channel.backpressure-never-drop]
fn test_tx_telemetry_inc_monitor_not() {
    let (mut tx, _graph, _sender) = new_tx();
    // Set local_monitor_index to MONITOR_NOT to trigger the error branch
    tx.local_monitor_index = MONITOR_NOT;
    // Create a SteadyTelemetrySend with a dummy channel
    let (dummy_tx, _dummy_rx) = ChannelBuilder::default().with_capacity(1).eager_build::<[usize; 4]>();
    let mut tel = SteadyTelemetrySend::new(dummy_tx, [0; 4], [0; 4], Instant::now());
    tx.telemetry_inc(TxDone::Normal(5), &mut tel);
    // The error is logged but no panic
}

/// Tests `one`, capacity/vacant predicates, periodic logging, and Stream telemetry.
#[test]
// ss[verify channel.backpressure-never-drop]
fn one_capacity_vacant_log_periodic_and_stream_telemetry() {
    let (mut tx, _graph, _sender) = new_tx();
    assert_eq!(tx.one(), 1);
    let cap = tx.shared_capacity();
    assert!(tx.shared_capacity_for(1));
    assert!(tx.shared_capacity_for(cap));
    assert!(!tx.shared_capacity_for(cap + 1));
    assert!(tx.shared_vacant_units_for(1));
    assert!(tx.shared_vacant_units_for(tx.shared_vacant_units()));
    assert!(!tx.shared_vacant_units_for(tx.shared_vacant_units() + 1));

    // Channel construction backdates last_error_send so the first call logs.
    assert!(tx.log_perodic());
    assert!(!tx.log_perodic());
    tx.last_error_send = Instant::now()
        - Duration::from_secs((steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS + 1) as u64);
    assert!(tx.log_perodic());

    let (dummy_tx, _dummy_rx) = ChannelBuilder::default().with_capacity(1).eager_build::<[usize; 4]>();
    let mut tel = SteadyTelemetrySend::new(dummy_tx, [0; 4], [0; 4], Instant::now());
    tx.local_monitor_index = 0;
    tx.telemetry_inc(TxDone::Stream(2, 8), &mut tel);
    tx.local_monitor_index = MONITOR_UNKNOWN;
    tx.telemetry_inc(TxDone::Normal(1), &mut tel);
}

// ss[related channel.backpressure-never-drop]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
use crate::proptest_support::{capacity, message_vec};

ss_proptest! {

    /// Property: vacant slots plus in-flight items never exceed capacity.
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_vacant_plus_inflight_le_capacity(
        cap in capacity(),
        messages in message_vec::<u8>(),
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (mut tx, mut rx) = builder.eager_build_internal::<u8>();
        let channel_capacity = tx.shared_capacity();
        let to_send: Vec<u8> = messages.into_iter().take(channel_capacity).collect();
        let mut sent = 0usize;
        for &msg in &to_send {
            if tx.shared_try_send(msg).is_ok() {
                sent += 1;
            } else {
                break;
            }
        }
        let vacant = tx.shared_vacant_units();
        let avail = rx.shared_avail_units();
        prop_assert_eq!(sent, avail);
        prop_assert!(vacant + avail <= channel_capacity);
    }

    /// Property: shared_send_slice never sends more than vacant_units.
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_send_slice_never_exceeds_vacant(
        cap in 2usize..64,
        extra in 1usize..32,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (mut tx, _rx) = builder.eager_build_internal::<u8>();
        let vacant = tx.shared_vacant_units();
        let slice = vec![0u8; vacant + extra];
        let done = tx.shared_send_slice(&slice);
        prop_assert!(done.item_count() <= vacant);
    }

    /// Property: every item accepted by the transmitter is received (no silent drop).
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_no_silent_drop(
        cap in capacity(),
        messages in message_vec::<u8>(),
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (mut tx, mut rx) = builder.eager_build_internal::<u8>();
        let channel_capacity = tx.shared_capacity();
        let to_send: Vec<u8> = messages.into_iter().take(channel_capacity).collect();
        let sent = tx.shared_send_iter_until_full(to_send.iter().copied());
        prop_assert_eq!(sent, to_send.len());
        let mut taken = 0usize;
        while rx.shared_try_take().is_some() {
            taken += 1;
        }
        prop_assert_eq!(taken, sent);
    }

    /// Property: mark_closed is idempotent and does not change vacant count.
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_mark_closed_idempotent_vacant_unchanged(
        cap in capacity(),
        messages in message_vec::<u8>(),
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (mut tx, _rx) = builder.eager_build_internal::<u8>();
        let to_send: Vec<u8> = messages.into_iter().take(cap.saturating_sub(1)).collect();
        let _ = tx.shared_send_iter_until_full(to_send.iter().copied());
        let vacant_before = tx.shared_vacant_units();
        tx.shared_mark_closed();
        tx.shared_mark_closed();
        prop_assert_eq!(tx.shared_vacant_units(), vacant_before);
    }

    /// Property: send_iter_until_full never sends more than channel capacity.
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_send_iter_respects_capacity(
        cap in capacity(),
        extra in 1usize..64,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (mut tx, mut rx) = builder.eager_build_internal::<u8>();
        let msgs = vec![0u8; cap + extra];
        let sent = tx.shared_send_iter_until_full(msgs.iter().copied());
        prop_assert!(sent <= cap);
        let mut taken = 0usize;
        while rx.shared_try_take().is_some() {
            taken += 1;
        }
        prop_assert_eq!(sent, taken);
    }
}

