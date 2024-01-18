pub(crate) mod telemetry {
    pub mod metrics_collector;
    pub mod metrics_server;
}

pub(crate) mod serialize {
    pub mod byte_buffer_packer;
    pub mod fast_protocol_packed;
}


use std::any::type_name;
use std::future::Future;
use std::ops::{Deref, DerefMut, Sub};
use std::sync::Arc;
use std::time::Duration;
use bastion::{Callbacks, run};
use bastion::children::Children;
use bastion::prelude::*;
use futures_timer::Delay;
use log::{error, info, warn};
use crate::steady::telemetry::metrics_collector::*;
use crate::steady::serialize::byte_buffer_packer::*;

use async_ringbuf::{AsyncRb, traits::*};
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use async_std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use ringbuf::storage::Heap;
use monitor::SteadyMonitor;
use telemetry::metrics_collector::CollectorDetail;

mod stats;
mod config;
pub mod util;

mod dot;
pub(crate) mod monitor;


const MAX_TELEMETRY_ERROR_RATE_SECONDS: usize = 60;

type ChannelBacking<T> = Heap<T>;

type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

    //TODO: we want to use Static but will use heap as first step
    //TODO: we must use static for all telemetry work.
    //let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
    //   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()



////


pub struct SteadyGraph {
    channel_count: Arc<AtomicUsize>,
    monitor_count: usize,

    //used by collector but could grow if we get new actors at runtime
    all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,

}
//NOTE: we could get a read access to the SteadyRx and SteadyTx
//      BUT this could cause stalls in the application so instead
//      this data is gathered on the developers schedule when batch
//      is called
//

pub struct SteadyRx<T> {
    pub(crate) id: usize,
    pub(crate) label: &'static [& 'static str],
    pub(crate) batch_limit: usize,
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) local_idx: usize,
}



pub struct SteadyTx<T> {
    id: usize,
    batch_limit: usize,
    tx: InternalSender<T>,
    label: &'static [& 'static str],
    local_idx: usize,
    actor_name: Option<&'static str>,
}

////////////////////////////////////////
//////////////////////////////////////////////
////////////////////////////////////////

macro_rules! guard {
    ($lock:expr) => {
        $lock.lock().await
    };
}

macro_rules! ref_mut {
    ($guard:expr) => {
        std::ops::DerefMut::deref_mut(&mut $guard)
    };
}


//TODO: lock and unwrap the actual SteadyTx and Rx values.. for use


//TODO: parallel children are a problem for the new single send recieve channels.

//TODO: refactor into struct later
pub(crate) fn callbacks() -> Callbacks {

    //is the call back recording events for the elemetry?
    Callbacks::new()
        .with_before_start( || {
            //TODO: record the name? of the telemetry on the graph needed?
            // info!("before start");
        })
        .with_after_start( || {
            //TODO: change telemetry to started if needed
            // info!("after start");
        })
        .with_after_stop( || {
            //TODO: record this telemetry has stopped on the graph
            //  info!("after stop");
        })
        .with_after_restart( || {
            //TODO: record restart count on the graph
//info!("after restart");
        })
}


fn build_new_monitor(name: & 'static str, ctx: Option<BastionContext>
                           , id: usize
                           , channel_count: Arc<AtomicUsize>
                           , all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>
) -> SteadyMonitor {

    SteadyMonitor {
        channel_count,
        name,
        ctx,
        id,
        all_telemetry_rx,
    }
}

//for testing only
#[cfg(test)]
impl SteadyGraph  {
    pub fn new_test_monitor(self: &mut Self, name: & 'static str ) -> SteadyMonitor
    {
        let id = self.monitor_count;
        self.monitor_count += 1;

        build_new_monitor(name, None, id, self.channel_count.clone()
                          , self.all_telemetry_rx.clone())

    }
}



fn build_channel<T>(
                     id: usize, cap: usize, labels: & 'static [& 'static str]
                    ) -> (Arc<Mutex<SteadyTx<T>>>, Arc<Mutex<SteadyRx<T>>>) {
    let rb = AsyncRb::<ChannelBacking<T>>::new(cap);
    let (tx, rx): (AsyncProd<Arc<AsyncRb<Heap<T>>>>, AsyncCons<Arc<AsyncRb<Heap<T>>>>) = rb.split();
    (  Arc::new(Mutex::new(SteadyTx::new(id, tx, cap, labels, None)))
     , Arc::new(Mutex::new(SteadyRx::new(id, rx, cap, labels))) )
}

impl SteadyGraph {

    /// add new children actors to the graph
    pub fn add_to_graph<F,I>(self: &mut Self, name: & 'static str, c: Children, init: I ) -> Children
        where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,  {

        SteadyGraph::configure_for_graph(self, name, c, init)
    }

    /// create a new graph for the application typically done in main
    pub(crate) fn new() -> SteadyGraph {
        SteadyGraph {
            channel_count: Arc::new(AtomicUsize::new(0)),
            monitor_count: 0, //this is the count of all monitors
            all_telemetry_rx: Arc::new(Mutex::new(Vec::new())), //this is all telemetry receivers
        }
    }

    /// create new single channel before we build the graph actors
    pub fn new_channel<T>(&mut self
                          , cap: usize
                          , labels: &'static [& 'static str])
                                 -> (Arc<Mutex<SteadyTx<T>>>, Arc<Mutex<SteadyRx<T>>>) {
        build_channel( self.channel_count.fetch_add(1,Ordering::SeqCst)
                       , cap, labels)
    }



    fn configure_for_graph<F,I>(graph: & mut SteadyGraph, name: & 'static str, c: Children, init: I ) -> Children
            where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
                  F: Future<Output = Result<(),()>> + Send + 'static ,
        {
            let result = {
                let init_clone = init.clone();

                let id = graph.monitor_count;
                graph.monitor_count += 1;
                let (id, telemetry_tx) = (id, graph.all_telemetry_rx.clone());
                let channel_count = graph.channel_count.clone();

                c.with_callbacks(callbacks())
                    .with_exec(move |ctx| {
                        let init_fn_clone = init_clone.clone();
                        let channel_count = channel_count.clone();
                        let telemetry_tx = telemetry_tx.clone();
                        async move {
                            //this telemetry now owns this monitor
                            let monitor = build_new_monitor(name
                                         , Some(ctx)
                                         , id
                                         , channel_count.clone()
                                         , telemetry_tx.clone());

                            match init_fn_clone(monitor).await {
                                Ok(_) => {
                                    info!("Actor {:?} finished ", name);
                                },
                                Err(e) => {
                                    error!("{:?}", e);
                                    return Err(e);
                                }
                            }
                            Ok(())
                        }
                    })
                    .with_name(name)
            };

            #[cfg(test)] {
                result.with_distributor(Distributor::named(format!("testing-{name}")))
            }
            #[cfg(not(test))] {
                result
            }
        }


    pub(crate) fn init_telemetry(&mut self) {


        //The Troupe is restarted together if one telemetry fails
        let _ = Bastion::supervisor(|supervisor| {
            let supervisor = supervisor.with_strategy(SupervisionStrategy::OneForAll);

            let mut outgoing = Vec::new();

            let supervisor = if config::TELEMETRY_SERVER {
                let (tx, rx) = self.new_channel::<DiagramData>(config::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-polling"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                              telemetry::metrics_server::run(monitor
                                                                             , rx.clone()
                                              )
                    )
                })
            } else {
                supervisor
            };


            let senders_count:usize = {
                let guard = run!(self.all_telemetry_rx.lock());
                let v = guard.deref();
                v.len()
            };

            //only spin up the metrics collector if we have a consumer
            //OR if some actors are sending data we need to consume
            let supervisor = if config::TELEMETRY_SERVER || senders_count>0 {

                supervisor.children(|children| {
                    //we create this child last so we can clone the rx_vec
                    //and capture all the telemetry actors as well
                    let all_tel_rx = self.all_telemetry_rx.clone(); //using Arc here

                    SteadyGraph::configure_for_graph(self, "telemetry-collector"
                                                     , children.with_redundancy(0)
                                                     , move |monitor| {

                            let all_rx = all_tel_rx.clone();
                            telemetry::metrics_collector::run(monitor
                                                              , all_rx
                                                              , outgoing.clone()
                            )
                        }
                    )
                }
                )
            } else {
                supervisor
            };

            supervisor

        }).expect("Telemetry supervisor creation error.");
    }
}


////////////////////////////////////////////////////////////////


impl<T> SteadyRx<T> {
    fn new(id: usize, receiver: InternalReceiver<T>,cap: usize, label: &'static [& 'static str]) -> Self {
         SteadyRx { id, batch_limit: cap, rx: receiver, label, local_idx: usize::MAX  }
    }

    #[inline]
    pub async fn take_async(& mut self) -> Result<T,String> {
         match self.rx.pop().await {
            Some(a) => {Ok(a)}
            None => {Err("Producer is dropped".to_string())}
        }
    }

    #[inline]
    pub fn try_take(& mut self) -> Option<T> {
        self.rx.try_pop()
    }

    #[inline]
    pub fn try_peek(&self) -> Option<&T> {
        self.rx.first()
    }

    #[inline]
    pub async fn peek_async(& mut self) -> Option<&T> {
        self.rx.wait_occupied(1).await;
        self.rx.first()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
    #[inline]
    pub fn avail_units(& mut self) -> usize {
        self.rx.occupied_len()
    }

    #[inline]
    pub async fn wait_avail_units(& mut self, count: usize) {
        self.rx.wait_occupied(count).await
    }

}
impl<T> SteadyTx<T> {
    fn new(id: usize, sender: InternalSender<T>,cap: usize, label: &'static [& 'static str], actor_name: Option<&'static str>) -> Self {
        SteadyTx {actor_name, id, batch_limit: cap, tx: sender, label, local_idx: usize::MAX }
    }

    #[inline]
    pub fn try_send(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m)}
        }
    }

    #[inline]
    pub async fn send_async(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())},
            Err(msg) => {
                error!("full channel detected data_approval tx telemetry: {:?} labels: {:?} capacity:{:?} type:{} "
                , self.actor_name, self.label, self.batch_limit, type_name::<T>());
                //here we will await until there is room in the channel
                self.tx.push(msg).await
            }
        }
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.tx.is_full()
    }

    #[inline]
    pub fn vacant_units(&self) -> usize {
        self.tx.vacant_len()
    }

    #[inline]
    pub async fn wait_vacant_units(&self, count: usize) {
        self.tx.wait_vacant(count).await
    }


}
////////////////////////////////////////////
////////////////////////////////////////////
#[cfg(test)]
pub(crate) mod tests {
    use crate::steady::*;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use crate::steady;
    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    pub(crate) fn initialize_logger() {
        INIT.call_once(|| {
            if let Err(e) = steady::util::steady_logging_init("info") {
                print!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
            }
        });
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string) = graph.new_channel(8,&[]);

        let monitor = graph.new_test_monitor("test");
        let mut rx_string_guard = guard!(rx_string);
        let mut tx_string_guard = guard!(tx_string);

        let rxd: &mut SteadyRx<String> = ref_mut!(rx_string_guard);
        let txd: &mut SteadyTx<String> = ref_mut!(tx_string_guard);

        let mut monitor = monitor.init_stats(&mut[rxd], &mut[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], threshold);
        }
        monitor.relay_stats_tx_set_custom_batch_limit(txd, threshold);
        monitor.relay_stats_batch().await;

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], threshold);
        }
        monitor.relay_stats_rx_set_custom_batch_limit(rxd, threshold);
        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], 0);
        }
    }


    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let monitor = graph.new_test_monitor("test");

        let (tx_string,rx_string) = graph.new_channel(5,&[]);

        let mut rx_string_guard = guard!(rx_string);
        let mut tx_string_guard = guard!(tx_string);

        let rxd: &mut SteadyRx<String> = ref_mut!(rx_string_guard);
        let txd: &mut SteadyTx<String> = ref_mut!(tx_string_guard);

        let mut monitor = monitor.init_stats(&mut[rxd], &mut[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry_send_tx {
                assert_eq!(tx.count[txd.local_idx], count);
            }
            monitor.relay_stats_batch().await;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], threshold);
        }

        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], 0);
        }

    }
}
