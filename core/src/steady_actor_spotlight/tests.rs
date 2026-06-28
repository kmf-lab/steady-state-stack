#![allow(deprecated)] // legacy bundle-wait tests still exercise deprecated SteadyActor API
// ss[related actor.shadow-spotlight]
use std::sync::OnceLock;
use crate::*;
use crate::monitor::ActorMetaData;
// ss[related philosophy.single-wake-up]
use std::ops::DerefMut;
// ss[related philosophy.single-wake-up]
use std::time::Duration;
use futures_timer::Delay;
use std::sync::Arc;
// ss[related philosophy.single-wake-up]
use parking_lot::RwLock;
use futures::channel::oneshot;
use futures::FutureExt;
use std::time::Instant;
// ss[related philosophy.single-wake-up]
use std::sync::atomic::AtomicUsize;
use crate::channel_builder::ChannelBuilder;
// ss[related philosophy.single-wake-up]
use crate::steady_actor::SendOutcome;
// ss[related philosophy.single-wake-up]
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::graph_liveliness::GraphLiveliness;

// ss[related philosophy.single-wake-up]
fn build_tx_rx() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
    oneshot::channel()
}

// ss[related philosophy.single-wake-up]
fn test_steady_context() -> SteadyActorShadow {
    let (tx, rx) = build_tx_rx();
    let oneshot_shutdown_vec = Arc::new(Mutex::new(vec![tx]));
    SteadyActorShadow {
        runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
            oneshot_shutdown_vec.clone(),
            Default::default(),
            Default::default(),
        ))),
        channel_count: Arc::new(AtomicUsize::new(0)),
        ident: ActorIdentity::new(0, "test_actor", None),
        args: Arc::new(Box::new(())),
        all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
        actor_metadata: Arc::new(ActorMetaData::default()),
        oneshot_shutdown_vec,
        oneshot_shutdown: rx.shared(),
        node_tx_rx: None,
        regeneration: 0,
        last_periodic_wait: Default::default(),
        is_in_graph: true,
        actor_start_time: Instant::now(),
        frame_rate_ms: 1000,
        team_id: 0,
        show_thread_info: false,
        aeron_meda_driver: OnceLock::new(),
        aeron_init_for_tests: true,
        use_internal_behavior: true,
        shutdown_barrier: None,
        index_wait_last_avail: AtomicUsize::new(usize::MAX),
        index_wait_last_vacant: AtomicUsize::new(usize::MAX),
        index_wait_last_avail_vacant: AtomicUsize::new(usize::MAX),
    }
}

// ss[related philosophy.single-wake-up]
fn create_rx<T: std::fmt::Debug>(data: Vec<T>) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>) {
    let (tx, rx) = create_test_channel(10);
    let send = tx.clone();
    if let Some(ref mut send_guard) = send.try_lock() {
        for item in data {
            let _ = send_guard.shared_try_send(item);
        }
    }
    (tx.clone(), rx.clone())
}

// ss[related philosophy.single-wake-up]
fn create_test_channel<T: Debug>(capacity: usize) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
    let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
    let builder = ChannelBuilder::new(
        Arc::new(Default::default()),
        oneshot_shutdown_vec.clone(),
        40,
    )
    .with_capacity(capacity);
    let result = builder.build_channel::<T>();
    Box::leak(Box::new(oneshot_shutdown_vec));
    result
}

#[test]
// ss[verify philosophy.single-wake-up]
fn test_simple_monitor_build() {
    let context = test_steady_context();
    let monitor = context.into_spotlight([], []);
    assert_eq!("test_actor", monitor.ident.label.name);
}

/// Integration smoke: relay stats flush through a real testing graph channel.
#[async_std::test]
// ss[verify philosophy.single-wake-up]
async fn test_relay_stats_tx_rx_custom() {
    let _ = logging_util::steady_logger::initialize();

    let mut graph = GraphBuilder::for_testing().build("");
    let (tx_string, rx_string) = graph.channel_builder().with_capacity(8).build_channel();
    let tx_string = tx_string.clone();
    let rx_string = rx_string.clone();

    let context = graph.new_testing_test_monitor("test");
    let mut monitor = context.into_spotlight([&rx_string], [&tx_string]);

    let mut rxd = rx_string.lock().await;
    let mut txd = tx_string.lock().await;

    let threshold = 5;
    let mut count = 0;
    while count < threshold {
        let _ = monitor
            .send_async(&mut txd, "test".to_string(), SendSaturation::WarnThenAwait)
            .await;
        count += 1;
    }

    if let Some(ref mut tx) = monitor.telemetry.send_tx {
        assert_eq!(tx.count[txd.local_monitor_index], threshold);
    }

    Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
    monitor.relay_stats_smartly();

    if let Some(ref mut tx) = monitor.telemetry.send_tx {
        assert_eq!(tx.count[txd.local_monitor_index], 0);
    }

    while count > 0 {
        let x = monitor.take_async(&mut rxd).await;
        assert_eq!(x, Some("test".to_string()));
        count -= 1;
    }

    if let Some(ref mut rx) = monitor.telemetry.send_rx {
        assert_eq!(rx.count[rxd.local_monitor_index], threshold);
    }

    Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
    monitor.relay_stats_smartly();

    if let Some(ref mut rx) = monitor.telemetry.send_rx {
        assert_eq!(rx.count[rxd.local_monitor_index], 0);
    }
}

/// Integration smoke: batch relay stats through testing graph wiring.
#[async_std::test]
// ss[verify philosophy.single-wake-up]
async fn test_relay_stats_tx_rx_batch() {
    let _ = logging_util::steady_logger::initialize();

    let mut graph = GraphBuilder::for_testing().build("");
    let monitor = graph.new_testing_test_monitor("test");

    let (tx_string, rx_string) = graph.channel_builder().with_capacity(5).build_channel();
    let tx_string = tx_string.clone();
    let rx_string = rx_string.clone();

    let mut monitor = monitor.into_spotlight([&rx_string], [&tx_string]);

    let mut rx_string_guard = rx_string.lock().await;
    let mut tx_string_guard = tx_string.lock().await;

    let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
    let txd: &mut Tx<String> = tx_string_guard.deref_mut();

    let threshold = 5;
    let mut count = 0;
    while count < threshold {
        let _ = monitor
            .send_async(txd, "test".to_string(), SendSaturation::WarnThenAwait)
            .await;
        count += 1;
        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_monitor_index], count);
        }
    }
    Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
    monitor.relay_stats_smartly();

    if let Some(ref mut_tx) = monitor.telemetry.send_tx {
        assert_eq!(mut_tx.count[txd.local_monitor_index], 0);
    }

    while count > 0 {
        let x = monitor.take_async(rxd).await;
        assert_eq!(x, Some("test".to_string()));
        count -= 1;
    }
    if let Some(ref mut rx) = monitor.telemetry.send_rx {
        assert_eq!(rx.count[rxd.local_monitor_index], threshold);
    }
    Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
    monitor.relay_stats_smartly();

    if let Some(ref mut rx) = monitor.telemetry.send_rx {
        assert_eq!(rx.count[rxd.local_monitor_index], 0);
    }
}

