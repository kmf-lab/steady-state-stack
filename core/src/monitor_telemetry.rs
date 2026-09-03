// ss[impl telemetry.prometheus-metrics]
use crate::monitor::{
    ActorIdentity, ActorMetaData, ActorStatus, ChannelMetaData, RxTel, ThreadInfo,
};
// ss[impl telemetry.prometheus-metrics]
use crate::steady_rx::Rx;
// ss[related philosophy.structural-hierarchy]
use crate::steady_tx::Tx;
// ss[related philosophy.structural-hierarchy]
use crate::{MONITOR_NOT, MONITOR_UNKNOWN, SteadyRx, SteadyTx, monitor, steady_config};
// ss[impl telemetry.prometheus-metrics]
use futures_util::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use log::error;
// ss[related philosophy.structural-hierarchy]
use num_traits::Zero;
// ss[impl telemetry.prometheus-metrics]
use std::ops::DerefMut;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
// ss[impl telemetry.prometheus-metrics]
use std::thread;
// ss[related philosophy.structural-hierarchy]
use std::time::Instant;

/// Maximum characters stored for a DOT node subtitle (after newline collapse).
// ss[impl telemetry.prometheus-metrics]
pub(crate) const DOT_SUBTITLE_MAX_CHARS: usize = 256;

/// Pending subtitle for Graphviz DOT labels; shared between [`SteadyTelemetryActorSend`] and
/// [`SteadyTelemetryRx`] for the same actor registration.
// ss[impl telemetry.prometheus-metrics]
pub(crate) struct DotSubtitleMailbox {
    pending: parking_lot::Mutex<Option<DotSubtitlePending>>,
}

#[derive(Debug)]
// ss[impl telemetry.prometheus-metrics]
enum DotSubtitlePending {
    Clear,
    Set(String),
}

// ss[impl telemetry.prometheus-metrics]
impl DotSubtitleMailbox {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn new() -> Self {
        Self {
            pending: parking_lot::Mutex::new(None),
        }
    }

    /// Queue a clear (`None`) or set (`Some`); coalesces to the latest value under one lock.
    // ss[impl telemetry.prometheus-metrics]
    pub(crate) fn record(&self, text: Option<&str>) {
        let mut lock = self.pending.lock();
        *lock = Some(match text {
            None => DotSubtitlePending::Clear,
            Some(s) => {
                let collapsed = s.replace(['\n', '\r'], " ");
                let truncated: String = collapsed.chars().take(DOT_SUBTITLE_MAX_CHARS).collect();
                DotSubtitlePending::Set(truncated)
            }
        });
    }

    /// `None` = nothing to relay. `Some(None)` = clear subtitle. `Some(Some(s))` = set text.
    // ss[impl telemetry.prometheus-metrics]
    pub(crate) fn take_pending(&self) -> Option<Option<String>> {
        let mut lock = self.pending.lock();
        let pending = lock.take()?;
        Some(match pending {
            DotSubtitlePending::Clear => None,
            DotSubtitlePending::Set(s) => Some(s),
        })
    }
}

/// Structure representing the receiver side of steady telemetry.
// ss[impl telemetry.prometheus-metrics]
pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) send: Option<SteadyTelemetryTake<TXL>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) take: Option<SteadyTelemetryTake<RXL>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor: Option<SteadyRx<ActorStatus>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_metadata: Arc<ActorMetaData>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) dot_subtitle_mailbox: Option<Arc<DotSubtitleMailbox>>,
}

/// Structure representing the telemetry take side with a fixed length.
// ss[impl telemetry.prometheus-metrics]
pub struct SteadyTelemetryTake<const LENGTH: usize> {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) rx: Arc<Mutex<Rx<[usize; LENGTH]>>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) details: Vec<Arc<ChannelMetaData>>,
}

/// Structure representing the actor send side of steady telemetry.
// ss[impl telemetry.prometheus-metrics]
pub struct SteadyTelemetryActorSend {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) tx: SteadyTx<ActorStatus>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) ident: ActorIdentity,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) last_telemetry_error: Instant,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) instant_start: Instant,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) iteration_index_start: u64,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) regeneration: u32,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) bool_stop: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) bool_blocking: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) show_thread_info: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) hot_profile_await_ns_unit: AtomicU64,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) hot_profile: AtomicU64,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) hot_profile_concurrent: AtomicU16,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) calls: [AtomicU16; 6],
    // ss[related philosophy.structural-hierarchy]
    pub(crate) dot_subtitle_mailbox: Option<Arc<DotSubtitleMailbox>>,
}

// ss[impl telemetry.prometheus-metrics]
impl SteadyTelemetryActorSend {
    /// Resets the status of the telemetry actor send.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn status_reset(&mut self, iteration_index: u64) {
        self.hot_profile_await_ns_unit = AtomicU64::new(0);
        self.instant_start = Instant::now();
        self.calls
            .iter()
            .for_each(|f| f.store(0, Ordering::Relaxed));
        self.iteration_index_start = iteration_index;
    }

    //TODO: check the  calls for all zero?

    /// Generates a status message for the actor.
    // ss[impl telemetry.prometheus-metrics]
    pub(crate) fn status_message(&self, iteration_index: u64) -> ActorStatus {
        //this is a little expensive, and we should consider doing this every N calls
        //the consumer node already holds the previous and uses it until we see a change.
        let thread_info = if self.show_thread_info {
            Some(ThreadInfo {
                thread_id: thread::current().id(),
                #[cfg(feature = "core_display")]
                core: crate::telemetry::setup::get_current_cpu(),
            })
        } else {
            None
        };

        let total_ns = self.instant_start.elapsed().as_nanos() as u64;

        let hot = self.hot_profile_await_ns_unit.load(Ordering::Relaxed);
        debug_assert!(total_ns >= hot, "should be: {} >= {}", total_ns, hot);
        // trace!("status ratio {} over {} mCPU {}", hot, total_ns, (1024*hot)/total_ns );

        let calls: [u16; 6] = std::array::from_fn(|i| self.calls[i].load(Ordering::Relaxed));
        assert!(total_ns > 0);

        ActorStatus {
            ident: self.ident,
            total_count_restarts: self.regeneration,
            iteration_start: iteration_index,
            iteration_sum: (iteration_index - self.iteration_index_start),
            bool_stop: self.bool_stop,
            is_quiet: false,
            bool_blocking: self.bool_blocking,
            await_total_ns: hot,
            unit_total_ns: total_ns,
            thread_info,
            calls,
        }
    }

    // ss[impl telemetry.prometheus-metrics]
    pub(crate) fn set_dot_display_text(&self, text: Option<&str>) {
        if let Some(m) = &self.dot_subtitle_mailbox {
            m.record(text);
        }
    }
}

/// Represents the sender side of steady telemetry with a fixed-length buffer.
///
/// This structure is optimized for performance by keeping all its fields on the stack,
/// but as the size increases, heap allocation might be necessary.
///
/// # Type Parameters
/// - `LENGTH`: The fixed size of the internal arrays used for tracking telemetry data.
// ss[impl telemetry.prometheus-metrics]
pub struct SteadyTelemetrySend<const LENGTH: usize> {
    /// The transmission channel for sending telemetry data.
    /// This is typically used for sending statistics or monitoring information.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) tx: SteadyTx<[usize; LENGTH]>,

    /// A fixed-size array tracking the count of specific telemetry events.
    /// Each index corresponds to a different event type or metric.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) count: [usize; LENGTH],

    /// The last recorded timestamp when a telemetry error occurred.
    /// Used for tracking and debugging issues in telemetry data collection.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) last_telemetry_error: Instant,

    /// A mapping of local indices to their inverse counterparts.
    /// This is used for quick lookups and efficient data processing.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) inverse_local_index: [usize; LENGTH],
}

// ss[impl telemetry.prometheus-metrics]
impl<const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL, TXL> {
    // ss[related philosophy.structural-hierarchy]
    fn is_empty_and_closed(&self) -> bool {
        let s = if let Some(send) = &self.send {
            if let Some(mut rx) = send.rx.try_lock() {
                rx.is_empty() && rx.is_closed()
            } else {
                false
            }
        } else {
            true
        };

        let a = if let Some(actor) = &self.actor {
            if let Some(mut rx) = actor.try_lock() {
                rx.is_empty() && rx.is_closed()
            } else {
                false
            }
        } else {
            true
        };

        let t = if let Some(take) = &self.take {
            if let Some(mut rx) = take.rx.try_lock() {
                rx.is_empty() && rx.is_closed()
            } else {
                false
            }
        } else {
            true
        };

        s & a & t
    }

    // ss[impl telemetry.prometheus-metrics]
    fn is_empty(&self) -> bool {
        let s = if let Some(send) = &self.send {
            if let Some(rx) = send.rx.try_lock() {
                rx.is_empty()
            } else {
                false
            }
        } else {
            true
        };

        let a = if let Some(actor) = &self.actor {
            if let Some(rx) = actor.try_lock() {
                rx.is_empty()
            } else {
                false
            }
        } else {
            true
        };

        let t = if let Some(take) = &self.take {
            if let Some(rx) = take.rx.try_lock() {
                rx.is_empty()
            } else {
                false
            }
        } else {
            true
        };

        s & a & t
    }

    // ss[impl telemetry.prometheus-metrics]
    fn actor_metadata(&self) -> Arc<ActorMetaData> {
        self.actor_metadata.clone()
    }

    #[inline]
    // ss[impl telemetry.prometheus-metrics]
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        if let Some(send) = &self.send {
            send.details.to_vec()
        } else {
            vec![]
        }
    }

    #[inline]
    // ss[impl telemetry.prometheus-metrics]
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        if let Some(take) = &self.take {
            take.details.to_vec()
        } else {
            vec![]
        }
    }

    // ss[impl telemetry.prometheus-metrics]
    fn actor_rx(&self, version: u32) -> Option<Box<SteadyRx<ActorStatus>>> {
        if let Some(act) = &self.actor {
            if let Some(mut act) = act.try_lock() {
                act.deref_mut().rx_version.store(version, Ordering::SeqCst);
            } else {
                error!("Internal error, unable to store rx version");
            }
            Some(Box::new(act.clone()))
        } else {
            None
        }
    }

    // ss[impl telemetry.prometheus-metrics]
    fn consume_actor(&self) -> Option<ActorStatus> {
        if let Some(act) = &self.actor {
            let mut buffer =
                vec![ActorStatus::default(); steady_config::TELEMETRY_COLLECTOR_SLICE_MAX + 1];
            let count_of_actor_status_events = {
                if let Some(mut actor_status_rx) = act.try_lock() {
                    //TODO: if we have no messages then we also do not get any status on graph.dot.
                    actor_status_rx
                        .deref_mut()
                        .deprecated_shared_take_slice(&mut buffer)
                } else {
                    error!(
                        "Internal error, unable to lock the actor!!!! {:?} ",
                        &self.actor
                    );
                    0
                }
            };

            // Each `ActorStatus` is a disjoint slice since the last `status_reset` after a successful send.
            // Summing `await_total_ns` / `unit_total_ns` yields the combined busy ratio across backlog drained this wake.
            let mut await_total_ns: u64 = 0;
            let mut unit_total_ns: u64 = 0;

            let mut calls = [0u16; 6];
            let mut iteration_sum = 0;
            let mut thread_info: Option<ThreadInfo> = None;
            for status in buffer.iter().take(count_of_actor_status_events) {
                assert!(
                    status.unit_total_ns >= status.await_total_ns,
                    "{} {}",
                    status.unit_total_ns,
                    status.await_total_ns
                );

                iteration_sum += status.iteration_sum;
                await_total_ns += status.await_total_ns;
                assert!(status.unit_total_ns > 0);

                unit_total_ns += status.unit_total_ns;
                if status.thread_info.is_some() {
                    thread_info = status.thread_info;
                }

                for (i, call) in status.calls.iter().enumerate() {
                    calls[i] = calls[i].saturating_add(*call);
                }
            }

            if unit_total_ns == 0 {
                None
            } else {
                assert!(unit_total_ns > 0);
                if count_of_actor_status_events > 0 {
                    Some(ActorStatus {
                        ident: buffer[0].ident,
                        iteration_start: buffer[0].iteration_start, //always the starting iterator count
                        iteration_sum,
                        //we just use the last event for these two, no need to check the others.
                        total_count_restarts: buffer[count_of_actor_status_events - 1]
                            .total_count_restarts,
                        bool_stop: buffer[count_of_actor_status_events - 1].bool_stop,
                        is_quiet: buffer[count_of_actor_status_events - 1].is_quiet,
                        bool_blocking: buffer[count_of_actor_status_events - 1].bool_blocking,
                        await_total_ns,
                        unit_total_ns,
                        thread_info,
                        calls,
                    })
                } else {
                    Some(ActorStatus {
                        ident: buffer[0].ident,
                        iteration_start: buffer[0].iteration_start, //always the starting iterator count
                        iteration_sum,
                        total_count_restarts: 0,
                        bool_stop: false,
                        is_quiet: false,
                        bool_blocking: false,
                        await_total_ns,
                        unit_total_ns,
                        thread_info,
                        calls,
                    })
                    //HACK TESTFOR ZERO  None
                }
            }
        } else {
            None
        }
    }

    // ss[impl telemetry.prometheus-metrics]
    fn consume_dot_subtitle(&self) -> Option<Option<String>> {
        self.dot_subtitle_mailbox
            .as_ref()
            .and_then(|m| m.take_pending())
    }

    #[inline]
    /// Consumes messages from a channel and updates telemetry state, ensuring a "sane" state by
    /// postponing takes when necessary. Returns true if messages were consumed, false otherwise.
    ///
    /// # Arguments
    /// * `channel_state` - Vector of (current_take, current_limit) pairs for each channel.
    /// * `pending_takes` - Vector of postponed take values for each channel.
    /// * `pending_sends` - Vector of postponed send values for each channel.
    // ss[impl telemetry.prometheus-metrics]
    fn consume_take_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        future_take: &mut Vec<i64>,
        future_send: &mut Vec<i64>,
    ) -> bool {
        if let Some(take) = &self.take {
            let mut buffer = vec![[0usize; RXL]; steady_config::TELEMETRY_COLLECTOR_SLICE_MAX + 1];

            let count = {
                if let Some(mut rx_guard) = take.rx.try_lock() {
                    let rx = rx_guard.deref_mut();
                    rx.deprecated_shared_take_slice(&mut buffer)
                } else {
                    0
                }
            };
            let populated_slice = &buffer[0..count];

            take.details.iter().for_each(|meta| {
                let max_takeable = take_send_source[meta.id].1 - take_send_source[meta.id].0;
                assert!(max_takeable.ge(&0), "internal error");
                let value_taken = max_takeable.min(future_take[meta.id]);
                take_send_source[meta.id].0 += value_taken;
                future_take[meta.id] -= value_taken;
            });

            populated_slice.iter().for_each(|msg| {
                take.details.iter().zip(msg.iter()).for_each(|(meta, val)| {
                    let limit = take_send_source[meta.id].1;
                    let val = *val as i64;
                    if i64::is_zero(&future_take[meta.id])
                        && val + take_send_source[meta.id].0 <= limit
                    {
                        take_send_source[meta.id].0 += val;
                    } else {
                        future_take[meta.id] += val;
                    }
                });
            });

            take.details.iter().for_each(|meta| {
                let dif = take_send_source[meta.id].1 - take_send_source[meta.id].0;
                if dif > (meta.capacity as i64) {
                    let extra = dif - (meta.capacity as i64);
                    future_send[meta.id] += extra;
                    take_send_source[meta.id].1 -= extra;
                }
            });

            count > 0
        } else {
            false
        }
    }

    #[inline]
    // ss[impl telemetry.prometheus-metrics]
    fn consume_send_into(
        &self,
        take_send_target: &mut Vec<(i64, i64)>,
        future_send: &mut Vec<i64>,
    ) -> bool {
        if let Some(send) = &self.send {
            let mut buffer = vec![[0usize; TXL]; steady_config::TELEMETRY_COLLECTOR_SLICE_MAX + 1];

            let count = {
                if let Some(mut tx_guard) = send.rx.try_lock() {
                    let tx = tx_guard.deref_mut();
                    tx.deprecated_shared_take_slice(&mut buffer)
                } else {
                    0
                }
            };
            let populated_slice = &buffer[0..count];

            assert_eq!(future_send.len(), take_send_target.len());

            populated_slice.iter().for_each(|msg| {
                send.details.iter().zip(msg.iter()).for_each(|(meta, val)| {
                    take_send_target[meta.id].1 += future_send[meta.id];
                    future_send[meta.id] = 0;
                    take_send_target[meta.id].1 += *val as i64;
                });
            });
            count > 0
        } else {
            false
        }
    }
}

// ss[impl telemetry.prometheus-metrics]
impl<const LENGTH: usize> SteadyTelemetrySend<LENGTH> {
    /// Creates a new instance of SteadyTelemetrySend.
    // ss[related philosophy.structural-hierarchy]
    pub fn new(
        tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
        count: [usize; LENGTH],
        inverse_local_index: [usize; LENGTH],
        last_telemetry_error: Instant,
    ) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend {
            tx,
            count,
            last_telemetry_error,
            inverse_local_index,
        }
    }

    /// Processes an event for telemetry.
    // ss[impl telemetry.prometheus-metrics]
    pub(crate) fn process_event(&mut self, index: usize, id: usize, done: isize) -> usize {
        let telemetry = self;
        if index < MONITOR_NOT {
            let result: isize = done.saturating_add(telemetry.count[index] as isize);
            assert!(
                result >= 0,
                "internal error, already added then subtracted so negative is not possible"
            );
            telemetry.count[index] = result as usize;
            index
        } else if index == MONITOR_UNKNOWN {
            let local_index = monitor::find_my_index(telemetry, id);
            if local_index < MONITOR_NOT {
                let result: isize = done.saturating_add(telemetry.count[local_index] as isize);
                assert!(
                    result >= 0,
                    "internal error, already added then subtracted so negative is not possible"
                );
                telemetry.count[local_index] = result as usize;
            }
            local_index
        } else {
            index
        }
    }
}

/// Main structure representing steady telemetry.
// ss[impl telemetry.prometheus-metrics]
pub(crate) struct SteadyTelemetry<const RX_LEN: usize, const TX_LEN: usize> {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) state: Option<SteadyTelemetryActorSend>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) dirty: AtomicBool,
}

// ss[impl telemetry.prometheus-metrics]
impl<const RX_LEN: usize, const TX_LEN: usize> SteadyTelemetry<RX_LEN, TX_LEN> {
    /// Returns true if non zero channel data is waiting to be sent
    #[inline]
    // ss[impl telemetry.prometheus-metrics]
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
}

//tests

#[cfg(test)]
// ss[impl telemetry.prometheus-metrics]
#[path = "monitor_telemetry_tests.rs"]
mod monitor_telemetry_tests;
