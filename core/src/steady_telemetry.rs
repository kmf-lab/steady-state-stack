use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::Instant;
use std::thread;
use std::sync::Arc;
use futures_util::lock::Mutex;
use log::error;
use std::process::exit;
use std::ops::DerefMut;
use num_traits::Zero;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
use crate::{steady_config, monitor, MONITOR_NOT, MONITOR_UNKNOWN, SteadyRx, SteadyTx};
use crate::steady_rx::{Rx, RxDef};
use crate::steady_tx::{ Tx};

/// Structure representing the receiver side of steady telemetry.
pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    pub(crate) send: Option<SteadyTelemetryTake<TXL>>,
    pub(crate) take: Option<SteadyTelemetryTake<RXL>>,
    pub(crate) actor: Option<SteadyRx<ActorStatus>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
}

/// Structure representing the telemetry take side with a fixed length.
pub struct SteadyTelemetryTake<const LENGTH: usize> {
    pub(crate) rx: Arc<Mutex<Rx<[usize; LENGTH]>>>,
    pub(crate) details: Vec<Arc<ChannelMetaData>>,
}

/// Structure representing the actor send side of steady telemetry.
pub struct SteadyTelemetryActorSend {
    pub(crate) tx: SteadyTx<ActorStatus>,
    pub(crate) last_telemetry_error: Instant,
    pub(crate) instant_start: Instant,
    pub(crate) instance_id: u32,
    pub(crate) bool_stop: bool,
    pub(crate) hot_profile_await_ns_unit: AtomicU64,
    pub(crate) hot_profile: AtomicU64,
    pub(crate) hot_profile_concurrent: AtomicU16,
    pub(crate) calls: [AtomicU16; 6],
}

impl SteadyTelemetryActorSend {
    /// Resets the status of the telemetry actor send.
    pub(crate) fn status_reset(&mut self) {
        self.hot_profile_await_ns_unit = AtomicU64::new(0);
        self.instant_start = Instant::now();
        self.calls.iter().for_each(|f| f.store(0, Ordering::Relaxed));
    }

    /// Generates a status message for the actor.
    pub(crate) fn status_message(&self) -> ActorStatus {
        let total_ns = self.instant_start.elapsed().as_nanos() as u64;

        assert!(
            total_ns >= self.hot_profile_await_ns_unit.load(Ordering::Relaxed),
            "should be: {} >= {}",
            total_ns,
            self.hot_profile_await_ns_unit.load(Ordering::Relaxed)
        );

        let calls: Vec<u16> = self.calls.iter().map(|f| f.load(Ordering::Relaxed)).collect();
        let calls: [u16; 6] = calls.try_into().unwrap_or([0u16; 6]);

        let current_thread = thread::current();

        ActorStatus {
            total_count_restarts: self.instance_id,
            bool_stop: self.bool_stop,
            await_total_ns: self.hot_profile_await_ns_unit.load(Ordering::Relaxed),
            unit_total_ns: total_ns,
            thread_id: Some(current_thread.id()),
            calls,
        }
    }
}

/// Structure representing the sender side of steady telemetry with a fixed length.
pub struct SteadyTelemetrySend<const LENGTH: usize> {
    pub(crate) tx: SteadyTx<[usize; LENGTH]>,
    pub(crate) count: [usize; LENGTH],
    pub(crate) last_telemetry_error: Instant,
    pub(crate) inverse_local_index: [usize; LENGTH],
}

impl<const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL, TXL> {
    fn is_empty_and_closed(&self) -> bool {
        let s = if let Some(ref send) = &self.send {
            if let Some(mut rx) = send.rx.try_lock() {
                rx.is_empty() && rx.is_closed()
            } else {
                false
            }
        } else {
            true
        };

        let a = if let Some(ref actor) = &self.actor {
            if let Some(mut rx) = actor.try_lock() {
                rx.is_empty() && rx.is_closed()
            } else {
                false
            }
        } else {
            true
        };

        let t = if let Some(ref take) = &self.take {
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

    fn actor_metadata(&self) -> Arc<ActorMetaData> {
        self.actor_metadata.clone()
    }

    #[inline]
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        if let Some(send) = &self.send {
            send.details.to_vec()
        } else {
            vec![]
        }
    }

    #[inline]
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        if let Some(take) = &self.take {
            take.details.to_vec()
        } else {
            vec![]
        }
    }

    fn actor_rx(&self, version: u32) -> Option<Box<dyn RxDef>> {
        if let Some(ref act) = &self.actor {
            if let Some(mut act) = act.try_lock() {
                act.deref_mut().rx_version.store(version, Ordering::SeqCst);
            } else {
                error!("warning I should have been able to get this lock.");
                exit(-1);
            }
            Some(Box::new(act.clone()))
        } else {
            None
        }
    }

    fn consume_actor(&self) -> Option<ActorStatus> {
        if let Some(ref act) = &self.actor {
            let mut buffer = [ActorStatus::default(); steady_config::CONSUMED_MESSAGES_BY_COLLECTOR + 1];
            let count = {
                if let Some(mut guard) = act.try_lock() {
                    let act = guard.deref_mut();
                    act.shared_take_slice(&mut buffer)
                } else {
                    0
                }
            };

            let mut await_total_ns: u64 = 0;
            let mut unit_total_ns: u64 = 0;

            let mut calls = [0u16; 6];
            for status in buffer.iter().take(count) {
                assert!(
                    status.unit_total_ns >= status.await_total_ns,
                    "{} {}",
                    status.unit_total_ns,
                    status.await_total_ns
                );

                await_total_ns += status.await_total_ns;
                unit_total_ns += status.unit_total_ns;

                for (i, call) in status.calls.iter().enumerate() {
                    calls[i] = calls[i].saturating_add(*call);
                }
            }

            if count > 0 {
                let current_thread = thread::current();

                Some(ActorStatus {
                    total_count_restarts: buffer[count - 1].total_count_restarts,
                    bool_stop: buffer[count - 1].bool_stop,
                    await_total_ns,
                    unit_total_ns,
                    thread_id: Some(current_thread.id()),
                    calls,
                })
            } else {
                None
            }
        } else {
            None
        }
    }

    #[inline]
    fn consume_take_into(
        &self,
        take_send_source: &mut Vec<(i128, i128)>,
        future_take: &mut Vec<i128>,
        future_send: &mut Vec<i128>,
    ) -> bool {
        if let Some(ref take) = &self.take {
            let mut buffer = [[0usize; RXL]; steady_config::CONSUMED_MESSAGES_BY_COLLECTOR + 1];

            let count = {
                if let Some(mut rx_guard) = take.rx.try_lock() {
                    let rx = rx_guard.deref_mut();
                    rx.shared_take_slice(&mut buffer)
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
                    let val = *val as i128;
                    if i128::is_zero(&future_take[meta.id]) && val + take_send_source[meta.id].0 <= limit {
                        take_send_source[meta.id].0 += val;
                    } else {
                        future_take[meta.id] += val;
                    }
                });
            });

            take.details.iter().for_each(|meta| {
                let dif = take_send_source[meta.id].1 - take_send_source[meta.id].0;
                if dif > (meta.capacity as i128) {
                    let extra = dif - (meta.capacity as i128);
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
    fn consume_send_into(
        &self,
        take_send_target: &mut Vec<(i128, i128)>,
        future_send: &mut Vec<i128>,
    ) -> bool {
        if let Some(ref send) = &self.send {
            let mut buffer = [[0usize; TXL]; steady_config::CONSUMED_MESSAGES_BY_COLLECTOR + 1];

            let count = {
                if let Some(mut tx_guard) = send.rx.try_lock() {
                    let tx = tx_guard.deref_mut();
                    tx.shared_take_slice(&mut buffer)
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
                    take_send_target[meta.id].1 += *val as i128;
                });
            });
            count > 0
        } else {
            false
        }
    }
}

impl<const LENGTH: usize> SteadyTelemetrySend<LENGTH> {
    /// Creates a new instance of SteadyTelemetrySend.
    pub fn new(
        tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
        count: [usize; LENGTH],
        inverse_local_index: [usize; LENGTH],
        start_now: Instant,
    ) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend {
            tx,
            count,
            last_telemetry_error: start_now,
            inverse_local_index,
        }
    }

    /// Processes an event for telemetry.
    pub(crate) fn process_event(&mut self, index: usize, id: usize, done: isize) -> usize {
        let telemetry = self;
        if index < MONITOR_NOT {
            let result: isize = done.saturating_add(telemetry.count[index] as isize);
            assert!(result >= 0, "internal error, already added then subtracted so negative is not possible");
            telemetry.count[index] = result as usize;
            index
        } else if index == MONITOR_UNKNOWN {
            let local_index = monitor::find_my_index(telemetry, id);
            if local_index < MONITOR_NOT {
                let result: isize = done.saturating_add(telemetry.count[local_index] as isize);
                assert!(result >= 0, "internal error, already added then subtracted so negative is not possible");
                telemetry.count[local_index] = result as usize;
            }
            local_index
        } else {
            index
        }
    }
}

/// Main structure representing steady telemetry.
pub(crate) struct SteadyTelemetry<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) state: Option<SteadyTelemetryActorSend>,
}
