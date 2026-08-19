//! Property tests for monitor helper utilities (`find_my_index`, drift tracking, profile guard).

use super::*;
use crate::channel_builder::ChannelBuilder;
use crate::graph_liveliness::ActorIdentity;
use crate::monitor::{ActorStatus};
use crate::monitor_telemetry::{SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::ss_proptest;
use crate::MONITOR_NOT;
use proptest::prelude::*;
use std::sync::atomic::{AtomicIsize, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn minimal_actor_send() -> SteadyTelemetryActorSend {
    let builder = ChannelBuilder::default().with_capacity(4);
    let (tx, _rx) = builder.eager_build::<ActorStatus>();
    SteadyTelemetryActorSend {
        tx,
        ident: ActorIdentity::new(0, "helpers_test", None),
        last_telemetry_error: Instant::now(),
        instant_start: Instant::now(),
        iteration_index_start: 0,
        regeneration: 0,
        bool_stop: false,
        bool_blocking: false,
        show_thread_info: false,
        hot_profile_await_ns_unit: AtomicU64::new(0),
        hot_profile: AtomicU64::new(50),
        hot_profile_concurrent: AtomicU16::new(1),
        calls: Default::default(),
        dot_subtitle_mailbox: None,
    }
}

ss_proptest! {
    /// Property: `find_my_index` returns the local slot for a mapped global id.
    #[test]
    // ss[verify verify.process.proptest]
    fn proptest_find_my_index_maps_global_to_local(
        len in 1usize..6,
        goal_slot_offset in 0usize..6,
    ) {
        let goal_slot = goal_slot_offset % len;
        let builder = ChannelBuilder::default().with_capacity(4);
        let (tx, _rx) = builder.eager_build::<[usize; 6]>();
        let mut inverse = [MONITOR_NOT; 6];
        let mut globals = Vec::new();
        for i in 0..len {
            let global = 100 + i * 17;
            inverse[i] = global;
            globals.push(global);
        }
        let goal = globals[goal_slot];
        let send = SteadyTelemetrySend::new(
            tx,
            [0; 6],
            inverse,
            Instant::now(),
        );
        prop_assert_eq!(find_my_index(&send, goal), goal_slot);
        prop_assert_eq!(find_my_index(&send, 9_999), MONITOR_NOT);
    }

    /// Property: `DriftCountIterator` records actual-vs-expected drift on drop.
    #[test]
    // ss[verify verify.process.proptest]
    fn proptest_drift_count_iterator_records_delta(
        expected in 0usize..24,
        produced in 0usize..24,
    ) {
        let drift = Arc::new(AtomicIsize::new(0));
        {
            let items: Vec<u32> = (0..produced as u32).collect();
            let mut iter = DriftCountIterator::new(expected, items.into_iter(), drift.clone());
            while iter.next().is_some() {}
        }
        prop_assert_eq!(
            drift.load(Ordering::Relaxed),
            produced as isize - expected as isize
        );
    }

    /// Property: zero drift when iterator yields exactly the expected count.
    #[test]
    // ss[verify verify.process.proptest]
    fn proptest_drift_count_iterator_zero_when_counts_match(n in 0usize..32) {
        let drift = Arc::new(AtomicIsize::new(0));
        {
            let mut iter = DriftCountIterator::new(
                n,
                (0..n as u32).collect::<Vec<_>>().into_iter(),
                drift.clone(),
            );
            prop_assert_eq!(iter.count(), n);
        }
        prop_assert_eq!(drift.load(Ordering::Relaxed), 0);
    }

    /// Property: profile guard rolls await time when the last concurrent holder drops.
    #[test]
    // ss[verify verify.process.proptest]
    fn proptest_profile_guard_rollup_on_last_drop(
        profile_ns in 1u64..10_000,
    ) {
        let st = minimal_actor_send();
        st.hot_profile.store(profile_ns, Ordering::Relaxed);
        st.hot_profile_concurrent.store(1, Ordering::SeqCst);
        {
            let _guard = FinallyRollupProfileGuard {
                st: &st,
                start: Instant::now() - Duration::from_nanos(profile_ns),
            };
        }
        prop_assert!(st.hot_profile_await_ns_unit.load(Ordering::Relaxed) > 0);
    }
}
