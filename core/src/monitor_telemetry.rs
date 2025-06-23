use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::Instant;
use std::sync::Arc;
use futures_util::lock::Mutex;
use log::error;
use std::ops::DerefMut;
use std::thread;
use num_traits::Zero;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel, ThreadInfo};
use crate::{steady_config, monitor, MONITOR_NOT, MONITOR_UNKNOWN, SteadyRx, SteadyTx};
use crate::steady_rx::{Rx};
use crate::steady_tx::{Tx};

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
    pub(crate) iteration_index_start: u64,
    pub(crate) instance_id: u32,
    pub(crate) bool_stop: bool,
    pub(crate) bool_blocking: bool,
    pub(crate) hot_profile_await_ns_unit: AtomicU64,
    pub(crate) hot_profile: AtomicU64,
    pub(crate) hot_profile_concurrent: AtomicU16,
    pub(crate) calls: [AtomicU16; 6],
}

impl SteadyTelemetryActorSend {
    /// Resets the status of the telemetry actor send.
    pub(crate) fn status_reset(&mut self, iteration_index: u64) {
        self.hot_profile_await_ns_unit = AtomicU64::new(0);
        self.instant_start = Instant::now();
        self.calls.iter().for_each(|f| f.store(0, Ordering::Relaxed));
        self.iteration_index_start = iteration_index;
    }

    /// Generates a status message for the actor.
    pub(crate) fn status_message(&self, iteration_index: u64) -> ActorStatus {

        //this is a little expensive, and we should consider doing this every N calls
        //the consumer node already holds the previous and uses it until we see a change.
        let thread_info = Some(ThreadInfo{
                            thread_id: thread::current().id(),
                            #[cfg(feature = "core_display")]
                            core: crate::telemetry::setup::get_current_cpu(),
                        });

        let total_ns = self.instant_start.elapsed().as_nanos() as u64;

        debug_assert!(
            total_ns >= self.hot_profile_await_ns_unit.load(Ordering::Relaxed),
            "should be: {} >= {}", total_ns, self.hot_profile_await_ns_unit.load(Ordering::Relaxed)
        );

        let calls: [u16; 6] = std::array::from_fn(|i| self.calls[i].load(Ordering::Relaxed));

        ActorStatus {
            total_count_restarts: self.instance_id,
            iteration_start: iteration_index,
            iteration_sum: (iteration_index-self.iteration_index_start),
            bool_stop: self.bool_stop,
            bool_blocking: self.bool_blocking,
            await_total_ns: self.hot_profile_await_ns_unit.load(Ordering::Relaxed),
            unit_total_ns: total_ns,
            thread_info,
            calls,
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
pub struct SteadyTelemetrySend<const LENGTH: usize> {
    /// The transmission channel for sending telemetry data.
    /// This is typically used for sending statistics or monitoring information.
    pub(crate) tx: SteadyTx<[usize; LENGTH]>,

    /// A fixed-size array tracking the count of specific telemetry events.
    /// Each index corresponds to a different event type or metric.
    pub(crate) count: [usize; LENGTH],

    /// The last recorded timestamp when a telemetry error occurred.
    /// Used for tracking and debugging issues in telemetry data collection.
    pub(crate) last_telemetry_error: Instant,

    /// A mapping of local indices to their inverse counterparts.
    /// This is used for quick lookups and efficient data processing.
    pub(crate) inverse_local_index: [usize; LENGTH],
}

impl<const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL, TXL> {
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

    fn consume_actor(&self) -> Option<ActorStatus> {
        if let Some(act) = &self.actor {
            let mut buffer = [ActorStatus::default(); steady_config::CONSUMED_MESSAGES_BY_COLLECTOR + 1];
            let count_of_actor_status_events = {
                if let Some(mut actor_status_rx) = act.try_lock() {
                    //TODO: if we have no messages then we also do not get any status on graph.dot.
                    actor_status_rx.deref_mut().deprecated_shared_take_slice(&mut buffer)
                } else {
                    error!("Internal error, unable to lock the actor!!!! {:?} ",&self.actor);
                    0
                }
            };

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
                unit_total_ns += status.unit_total_ns;
                if status.thread_info.is_some() {
                    thread_info = status.thread_info;
                }

                for (i, call) in status.calls.iter().enumerate() {
                    calls[i] = calls[i].saturating_add(*call);
                }
            }
//TODO: IF NEVER SENT WE SHOULD SEND??
            if count_of_actor_status_events > 0 {
                Some(ActorStatus {
                    iteration_start: buffer[0].iteration_start, //always the starting iterator count
                    iteration_sum,
                    //we just use the last event for these two, no need to check the others.
                    total_count_restarts: buffer[count_of_actor_status_events - 1].total_count_restarts,
                    bool_stop: buffer[count_of_actor_status_events - 1].bool_stop,
                    bool_blocking: buffer[count_of_actor_status_events - 1].bool_blocking,
                    await_total_ns,
                    unit_total_ns,
                    thread_info,
                    calls,
                })
            } else {
                Some(ActorStatus {
                    iteration_start: buffer[0].iteration_start, //always the starting iterator count
                    iteration_sum,
                    total_count_restarts: 0,
                    bool_stop: false,
                    bool_blocking: false,
                    await_total_ns,
                    unit_total_ns,
                    thread_info,
                    calls,
                })
              //HACK TESTFOR ZERO  None
            }
        } else {
            None
        }
    }

    #[inline]
    fn consume_take_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        future_take: &mut Vec<i64>,
        future_send: &mut Vec<i64>,
    ) -> bool {
        if let Some(take) = &self.take {
            let mut buffer = [[0usize; RXL]; steady_config::CONSUMED_MESSAGES_BY_COLLECTOR + 1];

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
                    if i64::is_zero(&future_take[meta.id]) && val + take_send_source[meta.id].0 <= limit {
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
    fn consume_send_into(
        &self,
        take_send_target: &mut Vec<(i64, i64)>,
        future_send: &mut Vec<i64>,
    ) -> bool {
        if let Some(send) = &self.send {
            let mut buffer = [[0usize; TXL]; steady_config::CONSUMED_MESSAGES_BY_COLLECTOR + 1];

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

impl<const LENGTH: usize> SteadyTelemetrySend<LENGTH> {
    /// Creates a new instance of SteadyTelemetrySend.
    pub fn new(
        tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
        count: [usize; LENGTH],
        inverse_local_index: [usize; LENGTH],
        last_telemetry_error: Instant
    ) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend {
            tx,
            count,
            last_telemetry_error,
            inverse_local_index
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

impl<const RX_LEN: usize, const TX_LEN: usize> SteadyTelemetry<RX_LEN, TX_LEN> {
    
    /// Returns true if non zero channel data is waiting to be sent
    pub(crate) fn is_dirty(&self) -> bool {        
        if let Some(send_tx) = &self.send_tx {
            send_tx.count.iter().any(|&x| x > 0)
        } else if let Some(send_rx) = &self.send_rx {
                send_rx.count.iter().any(|&x| x > 0)
            } else if let Some(state) = &self.state {
                    state.calls.iter().any(|x| x.load(Ordering::Relaxed) > 0)
                } else {
                    false
                } 
    }
}

//tests
#[cfg(test)]
mod monitor_telemetry_old_tests {
use std::sync::{Arc};
    use crate::monitor::{ActorMetaData, RxTel};
    use crate::monitor_telemetry::{SteadyTelemetryRx};


    #[test]
    fn test_steady_telemetry_rx_actor_metadata() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: actor_metadata.clone(),
        };

        let metadata = telemetry_rx.actor_metadata();
        assert_eq!(Arc::strong_count(&actor_metadata), 3);
        assert_eq!(Arc::strong_count(&metadata), 3);
    }

 

}

// tests for the monitor telemetry module
#[cfg(test)]
mod monitor_telemetry_tests {
    use std::sync::{Arc};
    use crate::monitor::{
        ActorMetaData, RxTel, 
    };
    use crate::monitor_telemetry::{
        SteadyTelemetry, SteadyTelemetryRx,
    };

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
    fn test_steady_telemetry_rx_is_empty_and_closed_all_none() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: actor_metadata.clone(),
        };

        assert!(telemetry_rx.is_empty_and_closed());
    }


    #[test]
    fn test_steady_telemetry_rx_actor_metadata_clone() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: actor_metadata.clone(),
        };

        let metadata = telemetry_rx.actor_metadata();
        assert_eq!(Arc::strong_count(&actor_metadata), 3);
        assert_eq!(Arc::strong_count(&metadata), 3);
    }

  

    #[test]
    fn test_steady_telemetry_rx_tx_channel_id_vec_empty() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
        };

        let tx_channels = telemetry_rx.tx_channel_id_vec();
        assert!(tx_channels.is_empty());
    }



    #[test]
    fn test_steady_telemetry_rx_rx_channel_id_vec_empty() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
        };

        let rx_channels = telemetry_rx.rx_channel_id_vec();
        assert!(rx_channels.is_empty());
    }

    

    #[test]
    fn test_steady_telemetry_is_dirty_all_none() {
        let telemetry = SteadyTelemetry::<4, 4> {
            send_tx: None,
            send_rx: None,
            state: None,
        };

        assert!(!telemetry.is_dirty());
    }


    #[test]
    fn test_steady_telemetry_consume_actor_without_actor() {
        let actor_metadata = Arc::new(ActorMetaData::default());
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata,
        };

        let status = telemetry_rx.consume_actor();
        assert!(status.is_none());
    }

 

    #[test]
    fn test_steady_telemetry_consume_take_into_without_take() {
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
        };

        let mut take_send_source = vec![(0, 100)];
        let mut future_take = vec![50];
        let mut future_send = vec![0];

        let result = telemetry_rx.consume_take_into(&mut take_send_source, &mut future_take, &mut future_send);
        assert!(!result);
    }


    #[test]
    fn test_steady_telemetry_consume_send_into_without_send() {
        let telemetry_rx = SteadyTelemetryRx::<4, 4> {
            send: None,
            take: None,
            actor: None,
            actor_metadata: Arc::new(ActorMetaData::default()),
        };

        let mut take_send_target = vec![(0, 100)];
        let mut future_send = vec![10];

        let result = telemetry_rx.consume_send_into(&mut take_send_target, &mut future_send);
        assert!(!result);
    }


}

