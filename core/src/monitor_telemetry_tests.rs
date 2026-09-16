//! Tests extracted from `monitor_telemetry.rs` so the production file stays under the 1,200-line budget.
//! Nested test mods keep their own imports; `use super::*` at the top re-exports the parent.


#[cfg(test)]
// ss[impl telemetry.prometheus-metrics]
mod monitor_telemetry_old_tests {
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor::{ActorMetaData, RxTel};
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor_telemetry::SteadyTelemetryRx;
    // ss[impl telemetry.prometheus-metrics]
    use std::sync::Arc;

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_actor_metadata() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: actor_metadata.clone(),
            dot_subtitle_mailbox: None,
        };

        let metadata = telemetry_rx.actor_metadata();
        assert_eq!(Arc::strong_count(&actor_metadata), 3);
        assert_eq!(Arc::strong_count(&metadata), 3);
        assert!(telemetry_rx.is_empty());
        assert!(telemetry_rx.is_empty_and_closed());
        assert!(telemetry_rx.tx_channel_id_vec().is_empty());
        assert!(telemetry_rx.rx_channel_id_vec().is_empty());
        assert!(telemetry_rx.consume_actor().is_none());
        assert!(telemetry_rx.actor_rx(0).is_none());
        assert_eq!(telemetry_rx.consume_dot_subtitle(), None);
    }
}

// tests for the monitor telemetry module
#[cfg(test)]
// ss[impl telemetry.prometheus-metrics]
mod monitor_telemetry_tests {
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor::{ActorMetaData, ChannelMetaData, RxTel};
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor_telemetry::{
        DotSubtitleMailbox, SteadyTelemetry, SteadyTelemetryActorSend, SteadyTelemetryRx,
        SteadyTelemetrySend, SteadyTelemetryTake, DOT_SUBTITLE_MAX_CHARS,
    };
    // ss[impl telemetry.prometheus-metrics]
    use std::sync::Arc;
    // ss[related philosophy.structural-hierarchy]
    use std::sync::atomic::Ordering;
    // ss[related philosophy.structural-hierarchy]
    use std::time::Instant;
    // ss[impl telemetry.prometheus-metrics]
    use crate::channel_builder::ChannelBuilder;
    // ss[impl telemetry.prometheus-metrics]
    use crate::*;

    // // Helper function to create default ChannelMetaData
    // fn create_channel_meta(id: usize, capacity: usize) -> Arc<ChannelMetaData> {
    //     Arc::new(ChannelMetaData {
    //         id,
    //         labels: vec![],
    //         capacity,
    //         display_labels: false,
    //         line_expansion: 0.0,
    //         show_type: None,
    //         refresh_rate_in_bits: 0,
    //         window_bucket_in_bits: 0,
    //         percentiles_filled: vec![],
    //         percentiles_rate: vec![],
    //         percentiles_latency: vec![],
    //         std_dev_inflight: vec![],
    //         std_dev_consumed: vec![],
    //         std_dev_latency: vec![],
    //         trigger_rate: vec![],
    //         trigger_filled: vec![],
    //         trigger_latency: vec![],
    //         avg_filled: false,
    //         avg_rate: false,
    //         avg_latency: false,
    //         min_filled: false,
    //         max_filled: false,
    //         connects_sidecar: false,
    //         type_byte_count: 0,
    //         show_total: true,
    //     })
    // }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_dot_subtitle_mailbox_take_and_clear() {
        let m = Arc::new(DotSubtitleMailbox::new());
        m.record(Some("x"));
        assert_eq!(m.take_pending(), Some(Some("x".into())));
        assert_eq!(m.take_pending(), None);
        m.record(None);
        assert_eq!(m.take_pending(), Some(None));
        assert_eq!(m.take_pending(), None);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_consume_dot_subtitle_with_mailbox() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let mailbox = Arc::new(DotSubtitleMailbox::new());
        mailbox.record(Some("live".into()));
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
            dot_subtitle_mailbox: Some(mailbox),
        };

        assert_eq!(
            telemetry_rx.consume_dot_subtitle(),
            Some(Some("live".into()))
        );
        assert_eq!(telemetry_rx.consume_dot_subtitle(), None);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_is_empty_and_closed_all_none() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: actor_metadata.clone(),
            dot_subtitle_mailbox: None,
        };

        assert!(telemetry_rx.is_empty_and_closed());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_actor_metadata_clone() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: actor_metadata.clone(),
            dot_subtitle_mailbox: None,
        };

        let metadata = telemetry_rx.actor_metadata();
        assert_eq!(Arc::strong_count(&actor_metadata), 3);
        assert_eq!(Arc::strong_count(&metadata), 3);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_tx_channel_id_vec_empty() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
            dot_subtitle_mailbox: None,
        };

        let tx_channels = telemetry_rx.tx_channel_id_vec();
        assert!(tx_channels.is_empty());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_rx_channel_id_vec_empty() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
            dot_subtitle_mailbox: None,
        };

        let rx_channels = telemetry_rx.rx_channel_id_vec();
        assert!(rx_channels.is_empty());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_is_dirty_all_none() {
        let telemetry = SteadyTelemetry::<4, 4> {
            send_tx: None,
            send_rx: None,
            state: None,
            dirty: std::sync::atomic::AtomicBool::new(false),
        };

        assert!(!telemetry.is_dirty());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_is_dirty_when_flag_set() {
        let telemetry = SteadyTelemetry::<0, 0> {
            send_tx: None,
            send_rx: None,
            state: None,
            dirty: std::sync::atomic::AtomicBool::new(true),
        };
        assert!(telemetry.is_dirty());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_channel_id_vecs_populated() {
        let meta_take = Arc::new(ChannelMetaData {
            id: 2,
            capacity: 8,
            ..Default::default()
        });
        let meta_send = Arc::new(ChannelMetaData {
            id: 5,
            capacity: 8,
            ..Default::default()
        });
        let builder = ChannelBuilder::test_channel_builder().with_capacity(4);
        let (_take_tx, take_rx) = builder.eager_build::<[usize; 1]>();
        let (_send_tx, send_rx) = builder.eager_build::<[usize; 1]>();
        let telemetry_rx = SteadyTelemetryRx::<1, 1> {
            send: Some(SteadyTelemetryTake {
                rx: send_rx,
                details: vec![meta_send.clone()],
            }),
            take: Some(SteadyTelemetryTake {
                rx: take_rx,
                details: vec![meta_take.clone()],
            }),
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };
        assert_eq!(telemetry_rx.tx_channel_id_vec()[0].id, 5);
        assert_eq!(telemetry_rx.rx_channel_id_vec()[0].id, 2);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_consume_actor_without_actor() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
            dot_subtitle_mailbox: None,
        };

        let status = telemetry_rx.consume_actor();
        assert!(status.is_none());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_consume_take_into_without_take() {
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };

        let mut take_send_source = vec![(0, 100)];
        let mut future_take = vec![50];
        let mut future_send = vec![0];

        let result = telemetry_rx.consume_take_into(
            &mut take_send_source,
            &mut future_take,
            &mut future_send,
        );
        assert!(!result);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_consume_send_into_without_send() {
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };

        let mut take_send_target = vec![(0, 100)];
        let mut future_send = vec![10];

        let result = telemetry_rx.consume_send_into(&mut take_send_target, &mut future_send);
        assert!(!result);
    }

    // --- New tests for SteadyTelemetrySend::process_event ---

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_send_process_event_normal() {
        let builder = ChannelBuilder::test_channel_builder().with_capacity(10);
        let (tx, _rx) = builder.eager_build::<[usize; 4]>();

        let mut send = SteadyTelemetrySend::new(tx, [0; 4], [0; 4], Instant::now());

        let new_index = send.process_event(0, 0, 5);
        assert_eq!(new_index, 0);
        assert_eq!(send.count[0], 5);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_send_process_event_unknown() {
        let builder = ChannelBuilder::test_channel_builder().with_capacity(10);
        let (tx, _rx) = builder.eager_build::<[usize; 4]>();

        let mut send = SteadyTelemetrySend::new(tx, [0; 4], [0; 4], Instant::now());

        // When index == MONITOR_UNKNOWN, the code will try to resolve via find_my_index.
        // The telemetry channel is not registered in the global registry so it will return
        // MONITOR_NOT or some value >= MONITOR_NOT. The test ensures no panic.
        let new_index = send.process_event(MONITOR_UNKNOWN, 42, 10);
        assert!(new_index >= MONITOR_NOT);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_send_process_event_not_monitored() {
        let builder = ChannelBuilder::test_channel_builder().with_capacity(10);
        let (tx, _rx) = builder.eager_build::<[usize; 4]>();

        let mut send = SteadyTelemetrySend::new(tx, [0; 4], [0; 4], Instant::now());

        let new_index = send.process_event(MONITOR_NOT, 1, 7);
        assert_eq!(new_index, MONITOR_NOT);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_send_process_event_normal_inverse() {
        // Test that process_event with an index (not zero) works
        let builder = ChannelBuilder::test_channel_builder().with_capacity(10);
        let (tx, _rx) = builder.eager_build::<[usize; 3]>();

        let mut send = SteadyTelemetrySend::new(tx, [10, 20, 30], [2, 1, 0], Instant::now());

        // index=1, id=5, done=7
        let new_index = send.process_event(1, 5, 7);
        assert_eq!(new_index, 1);
        assert_eq!(send.count[1], 27);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_dot_subtitle_mailbox_collapses_and_truncates() {
        let m = DotSubtitleMailbox::new();
        let noisy = format!("{}\n{}", "x".repeat(DOT_SUBTITLE_MAX_CHARS), "tail");
        m.record(Some(&noisy));
        let taken = m.take_pending().expect("pending").expect("text");
        assert!(!taken.contains('\n'));
        assert!(!taken.contains('\r'));
        assert!(taken.len() <= DOT_SUBTITLE_MAX_CHARS);
    }

    #[test]
    #[ignore] // hangs under load..
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_actor_send_status_reset_and_message() {
        // ss[related philosophy.structural-hierarchy]
        use crate::graph_liveliness::ActorIdentity;
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::ActorStatus;
        // ss[impl telemetry.prometheus-metrics]
        use std::sync::atomic::AtomicU64;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(4);
        let (act_tx, _act_rx) = builder.eager_build::<ActorStatus>();
        let mailbox = Arc::new(DotSubtitleMailbox::new());
        let ident = ActorIdentity::new(9, "status_actor", None);
        let mut actor_send = SteadyTelemetryActorSend {
            tx: act_tx,
            ident,
            last_telemetry_error: Instant::now(),
            instant_start: Instant::now(),
            iteration_index_start: 0,
            regeneration: 1,
            bool_stop: false,
            bool_blocking: false,
            show_thread_info: true,
            hot_profile_await_ns_unit: AtomicU64::new(50),
            hot_profile: AtomicU64::new(0),
            hot_profile_concurrent: Default::default(),
            calls: Default::default(),
            dot_subtitle_mailbox: Some(mailbox.clone()),
        };
        actor_send.status_reset(4);
        let status = actor_send.status_message(9);
        assert_eq!(status.iteration_sum, 5);
        assert!(status.unit_total_ns >= status.await_total_ns);
        actor_send.set_dot_display_text(Some("live subtitle"));
        assert_eq!(
            mailbox.take_pending(),
            Some(Some("live subtitle".into()))
        );
        actor_send.set_dot_display_text(None);
        assert_eq!(mailbox.take_pending(), Some(None));
    }

    #[test]
    #[ignore] //far too slow
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_actor_send_status_without_thread_info() {
        // ss[related philosophy.structural-hierarchy]
        use crate::graph_liveliness::ActorIdentity;
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::ActorStatus;
        // ss[impl telemetry.prometheus-metrics]
        use std::sync::atomic::AtomicU64;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(4);
        let (act_tx, _act_rx) = builder.eager_build::<ActorStatus>();
        let ident = ActorIdentity::new(11, "no_thread_info", None);
        let actor_send = SteadyTelemetryActorSend {
            tx: act_tx,
            ident,
            last_telemetry_error: Instant::now(),
            instant_start: Instant::now(),
            iteration_index_start: 2,
            regeneration: 0,
            bool_stop: false,
            bool_blocking: false,
            show_thread_info: false,
            hot_profile_await_ns_unit: AtomicU64::new(0),
            hot_profile: AtomicU64::new(0),
            hot_profile_concurrent: Default::default(),
            calls: Default::default(),
            dot_subtitle_mailbox: None,
        };
        let status = actor_send.status_message(5);
        assert!(status.thread_info.is_none());
        assert_eq!(status.iteration_sum, 3);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_consume_actor_aggregates_status() {
        // ss[related philosophy.structural-hierarchy]
        use crate::core_exec;
        // ss[related philosophy.structural-hierarchy]
        use crate::graph_liveliness::ActorIdentity;
        // ss[impl telemetry.prometheus-metrics]
        use crate::monitor::ActorStatus;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(8);
        let (act_tx, act_rx) = builder.eager_build::<ActorStatus>();
        let ident = ActorIdentity::new(3, "agg", None);
        let status = ActorStatus {
            ident,
            await_total_ns: 40,
            unit_total_ns: 80,
            iteration_start: 1,
            iteration_sum: 2,
            calls: [1, 0, 2, 0, 0, 3],
            ..Default::default()
        };
        core_exec::block_on(async {
            let mut tx = act_tx.lock().await;
            tx.shared_try_send(status).expect("send actor status");
        });

        let telemetry_rx = SteadyTelemetryRx::<0, 0> {
            send: None,
            take: None,
            actor: Some(act_rx),
            actor_metadata: Arc::new(ActorMetaData {
                ident,
                ..Default::default()
            }),
            dot_subtitle_mailbox: None,
        };
        let consumed = telemetry_rx.consume_actor().expect("aggregated status");
        assert_eq!(consumed.iteration_sum, 2);
        assert_eq!(consumed.await_total_ns, 40);
        assert_eq!(consumed.calls[0], 1);
        assert_eq!(consumed.calls[2], 2);
        assert_eq!(consumed.calls[5], 3);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_consume_take_and_send_into_with_data() {
        // ss[related philosophy.structural-hierarchy]
        use crate::core_exec;
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::ChannelMetaData;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(8);
        let (take_tx, take_rx) = builder.eager_build::<[usize; 1]>();
        let (send_tx, send_rx) = builder.eager_build::<[usize; 1]>();
        let meta_take = Arc::new(ChannelMetaData {
            id: 0,
            capacity: 8,
            ..Default::default()
        });
        let meta_send = Arc::new(ChannelMetaData {
            id: 0,
            capacity: 8,
            ..Default::default()
        });

        core_exec::block_on(async {
            let mut tx = take_tx.lock().await;
            tx.shared_try_send([4usize]).expect("send take telemetry");
        });
        core_exec::block_on(async {
            let mut tx = send_tx.lock().await;
            tx.shared_try_send([9usize]).expect("send send telemetry");
        });

        let telemetry_rx = SteadyTelemetryRx::<1, 1> {
            send: Some(SteadyTelemetryTake {
                rx: send_rx,
                details: vec![meta_send],
            }),
            take: Some(SteadyTelemetryTake {
                rx: take_rx,
                details: vec![meta_take],
            }),
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };

        let mut take_send_source = vec![(0i64, 64i64)];
        let mut future_take = vec![0i64];
        let mut future_send = vec![0i64];
        assert!(telemetry_rx.consume_take_into(
            &mut take_send_source,
            &mut future_take,
            &mut future_send,
        ));
        assert!(take_send_source[0].0 >= 4);

        let mut take_send_target = vec![(0i64, 0i64)];
        let mut future_send_out = vec![0i64];
        assert!(telemetry_rx.consume_send_into(
            &mut take_send_target,
            &mut future_send_out,
        ));
        assert!(take_send_target[0].1 >= 9);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_consume_take_into_caps_future_send() {
        // ss[related philosophy.structural-hierarchy]
        use crate::core_exec;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(8);
        let (take_tx, take_rx) = builder.eager_build::<[usize; 1]>();
        let meta = Arc::new(ChannelMetaData {
            id: 0,
            capacity: 4,
            ..Default::default()
        });
        core_exec::block_on(async {
            let mut tx = take_tx.lock().await;
            tx.shared_try_send([2usize]).expect("send take telemetry");
        });

        let telemetry_rx = SteadyTelemetryRx::<1, 0> {
            send: None,
            take: Some(SteadyTelemetryTake {
                rx: take_rx,
                details: vec![meta],
            }),
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };

        let mut take_send_source = vec![(0i64, 100i64)];
        let mut future_take = vec![0i64];
        let mut future_send = vec![0i64];
        assert!(telemetry_rx.consume_take_into(
            &mut take_send_source,
            &mut future_take,
            &mut future_send,
        ));
        assert!(future_send[0] > 0 || take_send_source[0].0 > 0);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_is_empty_false_when_data_present() {
        // ss[related philosophy.structural-hierarchy]
        use crate::core_exec;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(4);
        let (take_tx, take_rx) = builder.eager_build::<[usize; 1]>();
        let meta = Arc::new(ChannelMetaData {
            id: 0,
            capacity: 4,
            ..Default::default()
        });
        core_exec::block_on(async {
            let mut tx = take_tx.lock().await;
            tx.shared_try_send([1usize]).expect("send");
        });

        let telemetry_rx = SteadyTelemetryRx::<1, 0> {
            send: None,
            take: Some(SteadyTelemetryTake {
                rx: take_rx,
                details: vec![meta],
            }),
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };
        assert!(!telemetry_rx.is_empty());
        assert!(!telemetry_rx.is_empty_and_closed());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_actor_rx_sets_version() {
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::ActorStatus;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(4);
        let (_act_tx, act_rx) = builder.eager_build::<ActorStatus>();
        let telemetry_rx = SteadyTelemetryRx::<0, 0> {
            send: None,
            take: None,
            actor: Some(act_rx.clone()),
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };
        let cloned = telemetry_rx.actor_rx(42).expect("actor rx clone");
        let guard = act_rx.try_lock().expect("lock actor rx");
        assert_eq!(guard.rx_version.load(Ordering::SeqCst), 42);
        drop(guard);
        drop(cloned);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_actor_rx_none_without_actor() {
        let telemetry_rx = SteadyTelemetryRx::<0, 0> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };
        assert!(telemetry_rx.actor_rx(1).is_none());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_consume_dot_subtitle_without_mailbox() {
        let telemetry_rx = SteadyTelemetryRx::<0, 0> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };
        assert!(telemetry_rx.consume_dot_subtitle().is_none());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_steady_telemetry_rx_is_empty_and_closed_when_channels_closed() {
        // ss[related philosophy.structural-hierarchy]
        use crate::core_exec;

        let builder = ChannelBuilder::test_channel_builder().with_capacity(4);
        let (take_tx, take_rx) = builder.eager_build::<[usize; 1]>();
        let meta = Arc::new(ChannelMetaData {
            id: 0,
            capacity: 4,
            ..Default::default()
        });
        core_exec::block_on(async {
            let mut tx = take_tx.lock().await;
            tx.mark_closed();
        });

        let telemetry_rx = SteadyTelemetryRx::<1, 0> {
            send: None,
            take: Some(SteadyTelemetryTake {
                rx: take_rx,
                details: vec![meta],
            }),
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
            dot_subtitle_mailbox: None,
        };
        assert!(telemetry_rx.is_empty_and_closed());
    }
}

#[cfg(test)]
#[path = "monitor_telemetry_proptest.rs"]
// ss[impl telemetry.prometheus-metrics]
mod monitor_telemetry_proptest;
