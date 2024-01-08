
use std::any::{type_name};
use std::future::Future;

use std::sync::Arc;
use std::time::{Duration, Instant};
use async_std::sync::RwLock;

use bastion::{Callbacks, run};
use bastion::children::Children;

use bastion::prelude::*;
use bytes::{BufMut, BytesMut};
use flexi_logger::{Logger, LogSpecification};

use flume::{Receiver, Sender, bounded, RecvError};


use futures::AsyncReadExt;
use futures_timer::Delay;
use log::{debug, error, info, trace, warn};
use crate::steady::telemetry::metrics_collector::DiagramData;


use ringbuf::{CachingCons, CachingProd, SharedRb, StaticRb};
use ringbuf::storage::Static;
use ringbuf::traits::SplitRef;

mod telemetry {
    pub mod metrics_collector;
    pub mod telemetry_logging;
    pub mod telemetry_streaming;
    pub mod telemetry_polling;

}

// An Actor is a single threaded block of behavior that can send and receive messages
// A Troupe is a group of actors that are all working together to provide a specific service
// A Guild is a group of Troupes that are all working together on the same problem
// A Kingdom is a group of Guilds that are all working on the same problem
// A collection of kingdoms is a world





pub struct SteadyGraph {
    monitor_count: usize,
    //used by collector but could grow if we get new actors at runtime
    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    //shared all monitors and could change if new actors are added or monitoring is turned on
    shared_rx_array: Arc<RwLock<Vec<u8>>>, //common vec marks which are monitored locally
    shared_tx_array: Arc<RwLock<Vec<u8>>>, //common vec marks which are monitored locally

}
struct CollectorDetail {
    telemetry_take: Box<dyn RxTel>,
    name: &'static str,
    id: usize
}

pub struct SteadyMonitor
{
    pub name: & 'static str,

    last_instant: Instant,
    pub ctx: Option<BastionContext>,

    pub id: usize, //unique identifier for this child group

    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>, //these are Arc clones so they are the same for all monitors

    //these are Arc clones so they are the same for all monitors
    shared_rx_array: Arc<RwLock<Vec<u8>>>, //common vec marks which are monitored locally
    shared_tx_array: Arc<RwLock<Vec<u8>>>, //common vec marks which are monitored locally

}

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {

    telemetry_send_tx: SteadyTx<[usize; TX_LEN]>,
    telemetry_send_rx: SteadyTx<[usize; RX_LEN]>,

    monitor:                  SteadyMonitor,
    local_sent_count:        [usize; TX_LEN],
    local_sent_capacity:     [usize; TX_LEN],
    local_received_count:    [usize; RX_LEN],
    local_received_capacity: [usize; RX_LEN],

}

impl<const RX_LEN: usize, const TX_LEN: usize> LocalMonitor<RX_LEN, TX_LEN> {
    pub(crate) fn ctx(&self) -> &BastionContext {
       self.monitor.ctx.as_ref().unwrap()
    }
}

impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {
    pub async fn relay_stats_all(self: &Self) {
        // switch to a new vector and send the old one
        // we always clear the count so we can confirm this in testing
        // only relay if there is room otherwise we change nothing and roll up locally
        if self.telemetry_send_tx.has_room() {

            //we only send the result if we have a context, ie a graph we are monitoring
            if self.monitor.ctx.is_some() {
                /*
                                for elem in self.local_sent_count.iter_mut() {
                                    self.tx(&self.telemetry_tx, Telemetry::RelayTxStats(*elem, false)).await;
                                    *elem = 0;
                                }

                                for elem in self.local_received_count.iter_mut() {
                                    self.tx(&self.telemetry_tx, Telemetry::RelayRxStats(*elem, false)).await;
                                    *elem = 0;
                                }
                */
                if !self.telemetry_send_tx.has_room() {
                    error!("Telemetry channel is full for {}. Unable to send more telemetry, capacity is {}. It will be accumulated locally."
                        ,self.monitor.name
                        ,self.telemetry_send_tx.batch_limit);
                }
            }
        }
    }

    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {
        self.relay_stats_all().await;
        let run_duration: Duration = Instant::now().duration_since(self.monitor.last_instant);
        Delay::new(duration_rate.saturating_sub(run_duration)).await;
        self.monitor.last_instant = Instant::now();
    }

    pub async fn relay_stats_batch(self: &mut Self) {
        //only relay if one of the channels has reached or passed the capacity
        //walk both vecs to determine if any count has reached or passed the capacity
        let relay_now = self.local_sent_count.iter().zip(self.local_sent_capacity.iter()).any(|(count, capacity)| {
            count >= capacity
        });
        let relay_now = if !relay_now {
            self.local_received_count.iter().zip(self.local_received_capacity.iter()).any(|(count, capacity)| {
                count >= capacity
            })
        } else {
            relay_now
        };
        if relay_now {
            self.relay_stats_all().await;
        }
    }

    pub async fn relay_stats_tx_custom<T>(self: &mut Self, tx: &SteadyTx<T>, threshold: usize) {
        let idx: usize = {
            let local = self.monitor.shared_tx_array.read().await;
            if local.len() <= tx.id { local[tx.id] as usize } else { 0 }
        };
        if self.local_sent_count[idx] >= threshold {
            self.relay_stats_all().await;
        }
    }

    pub async fn relay_stats_rx_custom<T>(self: &mut Self, rx: &SteadyRx<T>, threshold: usize) {
        let idx: usize = {
            let local = self.monitor.shared_rx_array.read().await;
            if local.len() <= rx.id { local[rx.id] as usize } else { 0 }
        };
        if self.local_received_count[idx] >= threshold {
            self.relay_stats_all().await;
        }
    }
    pub async fn rx<T>(self: &mut Self, this: &SteadyRx<T>) -> Result<T, RecvError> {

        match self.monitor.rx(this).await {
            Ok(result) => {
                let guard = self.monitor.shared_rx_array.read().await;
                if guard.len() > this.id {
                    let idx = guard[this.id];
                    drop(guard);
                    if u8::MAX != idx { //only record those turned on (init)
                        self.local_received_count[idx as usize] += 1;
                    }
                }
                Ok(result)
            },
            Err(e) => {
                error!("Unexpected error recv_async: {} {}",e, self.monitor.name);
                Err(e)
            }
        }
    }
    pub async fn tx<T>(self: &mut Self, this: &SteadyTx<T>, a: T) -> Result<(), flume::SendError<T>> {
       match self.monitor.tx(this, a).await {
           Ok(_) => {
               let guard = self.monitor.shared_tx_array.read().await;
               if guard.len() > this.id {
                   let idx  = guard[this.id];
                   drop(guard);
                   if u8::MAX != idx { //only record those turned on (init)
                      self.local_sent_count[idx as usize] += 1;
                   }
               }
               return Ok(())
           },
           Err(e) => {
               error!("Unexpected error sending: {} {}",e, self.monitor.name);
               return Err(e);
           }
       }

    }

    //TODO: we need two of these later!!
    async fn telemetry_tx(self: &mut Self, a: [usize; TXL])  {
        //we try to send but if the channel is full we log that fact
        match self.telemetry_send_tx.try_send(a) {
            Ok(_) => {
                let guard = self.monitor.shared_tx_array.read().await;
                if guard.len() > self.telemetry_send_tx.id {
                    let idx  = guard[self.telemetry_send_tx.id];
                    drop(guard);
                    if u8::MAX != idx { //only record those turned on (init)
                       self.local_sent_count[idx as usize] += 1;
                    }
                }
            },
            Err(flume::TrySendError::Full(a)) => {
                error!("full telemetry channel detected from actor: {} capacity:{:?} "
                , self.monitor.name, self.telemetry_send_tx.batch_limit);
                //never block instead just return what we could not send
            },
            Err(e) => {
                error!("Unexpected error try sending lost telemetry: {} {}",e, self.monitor.name);
            }
        };
    }

}

impl SteadyMonitor {

    pub fn init_stats<const RX_LEN: usize, const TX_LEN: usize>(self
                                                                , rx_def: &[&dyn RxDef; RX_LEN]
                                                                , tx_def: &[&dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {


        let mut rx_bash_limit = [0; RX_LEN];
        let mut rx_index = [0; RX_LEN];
        let mut rx_array = run!(async {self.shared_rx_array.write().await});
        rx_def.iter()
            .enumerate()
            .for_each(|(c,rx)| {
                    rx_array[rx.id()]=c as u8;
                    rx_bash_limit[c] = rx.bash_limit();
                    rx_index[c] = rx.id();
            });
        drop(rx_array);


        let mut tx_bash_limit = [0; TX_LEN];
        let mut tx_sent_index = [0; TX_LEN];
        let mut tx_array = run!(async {self.shared_tx_array.write().await});
        tx_def.iter()
            .enumerate()
            .for_each(|(c,tx)| {
                tx_array[tx.id()]=c as u8;
                tx_bash_limit[c] = tx.bash_limit();
                tx_sent_index[c] = tx.id();
            });
        drop(tx_array);


        //TODO: build my special telemetry channel and pass map in..
        let (telemetry_send_tx, telemetry_send_rx): (SteadyTx<[usize; TX_LEN]>, SteadyRx<[usize; TX_LEN]>)
                                                       =  build_channel(TELEMETRY_FEATURES.channel_length
                                                          , &["steady-telemetry"]
                                                          , self.shared_tx_array.clone()
                                                          , self.shared_rx_array.clone());

        let (telemetry_receive_tx, telemetry_receive_rx): (SteadyTx<[usize; RX_LEN]>, SteadyRx<[usize; RX_LEN]>)
                                                       =  build_channel(TELEMETRY_FEATURES.channel_length
                                                         , &["steady-telemetry"]
                                                         , self.shared_tx_array.clone()
                                                         , self.shared_rx_array.clone());

        let str = SteadyTelemetryRx {
            read_tx: telemetry_send_rx,
            map_tx: tx_sent_index,
            read_rx: telemetry_receive_rx,
            map_rx: rx_index,
        };

        run!(async{
                self.all_telemetry_rx.write().await.push(
                    CollectorDetail{
                          name: self.name
                        , id: self.id
                        , telemetry_take: Box::new(str)        }
                );
            });


        let mut result = LocalMonitor::<RX_LEN,TX_LEN> {
            telemetry_send_rx: telemetry_receive_tx,
            telemetry_send_tx: telemetry_send_tx,


            monitor: self,  //TODO: should be some kind of clone??

            local_sent_count: [0; TX_LEN],
            local_sent_capacity: tx_bash_limit,

            local_received_count: [0; RX_LEN],
            local_received_capacity: rx_bash_limit
        };

//TODO: we need 2 pipes since we have two lengths !!!

    //    run!( async {result.telemetry_tx(&rx_index).await} );
      //  run!( async {result.telemetry_tx(&tx_sent_index).await} );

        result
    }


    //TODO: add methods later to instrument non await blocks for cpu usage chart


    //TODO: wrap these methos to add telemetry after trasnsmit..

    pub async fn rx<T>(self: &mut Self, this: &SteadyRx<T>) -> Result<T, flume::RecvError> {
        this.recv_async().await
    }


    pub async fn tx<T>(self: &mut Self, this: &SteadyTx<T>, a: T) -> Result<(), flume::SendError<T>> {
        //we try to send but if the channel is full we log that fact
        match this.try_send(a) {
            Ok(_) => {return Ok(());},
            Err(flume::TrySendError::Full(a)) => {
                error!("full channel detected data_approval tx  actor: {} capacity:{:?} type:{} "
                , self.name, this.batch_limit, type_name::<T>());
                //here we will await until there is room in the channel
                this.send_async(a).await
            },
            Err(flume::TrySendError::Disconnected(d)) => {
                error!("Unexpected disconnect error try sending: {}", self.name);
                this.send_async(d).await
            }
        }
    }



}

//TODO: refactor into struct later
pub(crate) fn callbacks() -> Callbacks {

    //is the call back recording events for the elemetry?
    Callbacks::new()
        .with_before_start( || {
            //TODO: record the name? of the actor on the graph needed?
            // info!("before start");
        })
        .with_after_start( || {
            //TODO: change actor to started if needed
            // info!("after start");
        })
        .with_after_stop( || {
            //TODO: record this actor has stopped on the graph
            //  info!("after stop");
        })
        .with_after_restart( || {
            //TODO: record restart count on the graph
//info!("after restart");
        })
}


async fn new_monitor_internal (graph: &mut SteadyGraph, name: & 'static str, ctx: Option<BastionContext>) -> SteadyMonitor //<'graph>
{
    let (id, shared_rx_array, shared_tx_array, all_telemetry_channels) = prep_to_monitor(graph,name).await;
    build_new_monitor(name, ctx, id, shared_rx_array, shared_tx_array, all_telemetry_channels).await
}




async fn prep_to_monitor(graph: &mut SteadyGraph, name: & 'static str) -> (usize, Arc<RwLock<Vec<u8>>>, Arc<RwLock<Vec<u8>>>, Arc<RwLock<Vec<CollectorDetail>>>) {

    let id = graph.monitor_count;
    graph.monitor_count += 1;

    (id, graph.shared_rx_array.clone(), graph.shared_tx_array.clone(), graph.all_telemetry_rx.clone())
}

async fn build_new_monitor(name: & 'static str, ctx: Option<BastionContext>
                           , id: usize
                           , shared_rx_array: Arc<RwLock<Vec<u8>>>
                           , shared_tx_array: Arc<RwLock<Vec<u8>>>
                           , all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>
) -> SteadyMonitor {


    SteadyMonitor {
        last_instant: Instant::now(),
        name,
        ctx,
        id,
        all_telemetry_rx,
        shared_rx_array,
        shared_tx_array,
    }
}

//for testing only
#[cfg(test)]
impl SteadyGraph  {
    pub async fn new_test_monitor(self: &mut Self, name: & 'static str ) -> SteadyMonitor
    {
        new_monitor_internal(self, name, None).await
    }
}


fn build_channel<T>(cap: usize, labels: & 'static [& 'static str]
                    , sta: Arc<RwLock<Vec<u8>>>
                    , sra: Arc<RwLock<Vec<u8>>>) -> (SteadyTx<T>, SteadyRx<T>) {

    let (tx, rx): (Sender<T>, Receiver<T>) = bounded(cap);

    run!(async{
            let (stx,srx) = (SteadyTx::new(tx, sta.read().await.len(), labels).await
                           , SteadyRx::new(rx, sra.read().await.len(), labels).await);

            //NOTE: this is the ONLY place where we grow the length of the lookup vecs for channels
            let _ = sta.write().await.push(u8::MAX);
            let _ = sra.write().await.push(u8::MAX);

            (stx,srx)
           }
        )
}

impl SteadyGraph {
    //wrap the context with a new context that has a telemetry channel

    pub fn add_to_graph<F,I>(self: &mut Self, name: & 'static str, c: Children, init: I ) -> Children
        where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,
    {
       SteadyGraph::configure_for_graph(self, name, c, init)
    }

    pub(crate) fn new() -> SteadyGraph {
        SteadyGraph {
            monitor_count: 0, //this is the count of all monitors
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())), //this is all telemetry receivers
            //never pre allocate these since we use the len to count the id for the next element
            shared_rx_array: Arc::new(RwLock::new(Vec::new())), //this is the shared array of indexes for all receivers
            shared_tx_array: Arc::new(RwLock::new(Vec::new())), //this is the shared array of indexes for all senders
        }
    }

    pub fn new_channel<T>(&mut self
                                , cap: usize, labels: &'static [& 'static str]) -> (SteadyTx<T>, SteadyRx<T>)
    {
        let sta = self.shared_tx_array.clone();
        let sra = self.shared_rx_array.clone();
        build_channel(cap, labels, sta, sra)
    }


    fn create<T, const N: usize>() {
        let mut rb = StaticRb::<T, N>::default();
        //let (mut prod: StaticRbProducer<i32, RB_SIZE>, mut cons: StaticRbConsumer<i32, RB_SIZE>) = rb.split();
        let (mut prod, mut cons): (CachingProd<&SharedRb<Static<T, N>>>, CachingCons<&SharedRb<Static<T, N>>>) = rb.split_ref();

    }



    fn configure_for_graph<F,I>(graph: & mut SteadyGraph, name: & 'static str, c: Children, init: I ) -> Children
            where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
                  F: Future<Output = Result<(),()>> + Send + 'static ,
        {
            let result = {
                let init_clone = init.clone();

                let (id, telemetry_tx, shared_rx_array, shared_tx_array) =
                    run!(async  {prep_to_monitor(graph,name).await});

                c.with_callbacks(callbacks())
                    .with_exec(move |ctx| {
                        let init_clone = init_clone.clone();
                        let monitor = run!(
                            async {
                               build_new_monitor(name, Some(ctx), id
                                    , telemetry_tx.clone()
                                    , shared_rx_array.clone()
                                    , shared_tx_array.clone()).await

                                }
                        );

                        async move {
                            match init_clone(monitor).await {
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

        //The Troupe is restarted together if one actor fails
        let _ = Bastion::supervisor(|supervisor| {
            let supervisor = supervisor.with_strategy(SupervisionStrategy::OneForAll);

            let mut outgoing:Vec<SteadyTx<DiagramData>> = Vec::new();

            let supervisor = if TELEMETRY_FEATURES.logging {
                //TODO: we have two ideas for length here and we need to check volume.
                let (tx, rx) = self.new_channel::<DiagramData>(TELEMETRY_FEATURES.channel_length,&["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-logging"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                          telemetry::telemetry_logging::run(monitor
                                                                              , rx.clone()
                                          )
                    )
                })
            } else {
                supervisor
            };

            let supervisor = if TELEMETRY_FEATURES.streaming {
                let (tx, rx) = self.new_channel::<DiagramData>(TELEMETRY_FEATURES.channel_length,&["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-streaming"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                          telemetry::telemetry_streaming::run(monitor
                                                                              , rx.clone()
                                          )
                    )
                })
            } else {
                supervisor
            };

            let supervisor = if TELEMETRY_FEATURES.polling {
                let (tx, rx) = self.new_channel::<DiagramData>(TELEMETRY_FEATURES.channel_length,&["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-polling"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                          telemetry::telemetry_polling::run(monitor
                                                                              , rx.clone()
                                          )
                    )
                })
            } else {
                supervisor
            };


            supervisor.children(|children| {
                //we create this child last so we can clone the rx_vec
                //and capture all the telemetry actors as well
                let all_tel_rx = self.all_telemetry_rx.clone(); //using Arc here
                run!(
                         async {
                                             SteadyGraph::configure_for_graph(self, "telemetry-collector"
                                                                 , children.with_redundancy(0)
                                                                 , move |monitor| {
                                                        //TODO: test...this is AFTER the new_monitor so we can clone the rx_vec
                                                        let all_rx = all_tel_rx.clone();
                                                         telemetry::metrics_collector::run(monitor
                                                                                      , all_rx
                                                                                      , outgoing.clone()

                                                        )
                                                        }
                                                )

                        }
                    )
            })

             }
        ).expect("Telemetry supervisor creation error.");
    }
}

#[derive(Debug, Clone, Copy)]
struct TelemetryFeatures {
    channel_length: usize,
    logging: bool,
    streaming: bool,
    polling: bool,
}
impl TelemetryFeatures {
    const fn new(channel_length: usize, logging: bool, streaming: bool, polling: bool) -> Self {
        TelemetryFeatures { channel_length, logging, streaming, polling }
    }
}

const TELEMETRY_FEATURES: TelemetryFeatures = TelemetryFeatures::new(64, true, false, true);


////////////////////////////////////////////////////////////////

pub trait TxDef {
    fn id(&self) -> usize;
    fn bash_limit(&self) -> usize;


}

impl <T> TxDef for SteadyTx<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn bash_limit(&self) -> usize {
        self.batch_limit
    }

}

#[derive(Clone, Debug)]
pub struct SteadyTx<T> {
    pub id: usize,
    batch_limit: usize,
    tx: Arc<Sender<T>>,
    pub label: &'static [& 'static str],

}


impl<T> SteadyTx<T> {
    async fn new(sender: Sender<T>, id: usize, label: &'static [& 'static str]) -> Self {
        SteadyTx {id, batch_limit: sender.capacity().unwrap_or(usize::MAX)
                  , tx: Arc::new(sender), label }
    }

    #[inline]
    pub fn try_send(&self, msg: T) -> Result<(), flume::TrySendError<T>> {
        self.tx.try_send(msg)
    }

    #[inline]
    pub async fn send_async(&self, msg: T) -> Result<(), flume::SendError<T>> {
        self.tx.send_async(msg).await
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.tx.is_full()
    }

    #[allow(dead_code)]
    pub fn has_room(&self) -> bool {
        !self.is_full()
    }

}


pub trait RxDef {
    fn id(&self) -> usize;
    fn bash_limit(&self) -> usize;

}

impl <T> RxDef for SteadyRx<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn bash_limit(&self) -> usize {
        self.batch_limit
    }

    // specialized telemetry load method
    // the caller can pass in the target and it will pull all the data
}


pub trait RxTel : Send + Sync {
    //NOTE: we will do one dyn call per node every 20 milliseconds or so to build the image
    //      we only have 1 impl assuming the compiler will inline this if possible
    // This closure, when called, returns a Pin<Box<dyn Future>>.
    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);

    //fn consume_into(&self) -> Box<dyn Fn(&mut Vec<u128>, &mut Vec<u128>) + Send >;
   // fn consume_into(&self) -> Box<dyn Fn(&mut Vec<u128>, &mut Vec<u128>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

}
impl <const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL,TXL> {
    #[inline]
    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>) {
        //-> Box<dyn Fn(&mut Vec<u128>, &mut Vec<u128>) + Send > {
        //Box::new( |take_target: &mut Vec<u128>, send_target: &mut Vec<u128>| {

            //this method can not be async since we ned vtable and dyn
            //so we will return a future to be used later which will also remove any dyn call.

            //read all the messages available and put the total into the right vec index
            //do for both reads and sends.
            run!(async { //TODO: this is a hack until I can write the future pined boxed function.
            loop {
                let mut count = 0;

                if self.read_rx.has_message() {
                    match self.read_rx.recv_async().await {
                        Ok(a) => {
                            self.map_rx.iter().zip(a.iter())
                                .for_each(|(idx, val)| take_target[*idx] += *val as u128);
                        },
                        Err(e) => {
                            error!("Unexpected error recv_async: {}",e);
                        }
                    }
                } else {
                    count += 1;
                }
                if self.read_tx.has_message() {
                    match self.read_tx.recv_async().await {
                        Ok(a) => {
                            self.map_tx.iter().zip(a.iter())
                                .for_each(|(idx, val)| send_target[*idx] += *val as u128);
                        },
                        Err(e) => {
                            error!("Unexpected error recv_async: {}",e);
                        }
                    }
                } else {
                    count += 1;
                }
                //leave when both channels are empty
                if 2 == count {
                    break;
                }
            }
            })
        //  })
    }

}

#[derive(Clone,Debug)]
pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    read_tx: SteadyRx<[usize; TXL]>,
    map_tx: [usize; TXL],

    read_rx: SteadyRx<[usize; RXL]>,
    map_rx: [usize; RXL],

}

#[derive(Clone,Debug)]
pub struct SteadyRx<T> {
    pub id: usize,
    batch_limit: usize,
    rx: Arc<Receiver<T>>,
    pub label: &'static [& 'static str],
}
impl<T> SteadyRx<T> {
    async fn new(receiver: Receiver<T>, id: usize, label: &'static [& 'static str]) -> Self {
        SteadyRx { id, batch_limit: receiver.capacity().unwrap_or(usize::MAX)
                           , rx: Arc::new(receiver), label
        }
    }

    #[inline]
    pub async fn recv_async(&self) -> Result<T, RecvError> {
        self.rx.recv_async().await
    }

    //this can and should be in-line when possible, hint to compiler
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    #[allow(dead_code)]
    pub fn has_message(&self) -> bool {
        !self.is_empty()
    }
}

/// Initializes logging for the application using the provided log level.
///
/// This function sets up the logger based on the specified log level, which can be adjusted through
/// command line arguments or environment variables. It's designed to be used both in the main
/// application and in test cases. The function demonstrates the use of traditional Rust error
/// propagation with the `?` operator. Note that actors do not initialize logging as it is done
/// for them in `main` before they are started.
///
/// # Parameters
/// * `level`: A string slice (`&str`) that specifies the desired log level. The log level can be
///            dynamically set via environment variables or directly passed as an argument.
///
/// # Returns
/// This function returns a `Result<(), Box<dyn std::error::Error>>`. On successful initialization
/// of the logger, it returns `Ok(())`. If an error occurs during initialization, it returns an
/// `Err` with the error wrapped in a `Box<dyn std::error::Error>`.
///
/// # Errors
/// This function will return an error if the logger initialization fails for any reason, such as
/// an invalid log level string or issues with logger setup.
///
/// # Security Considerations
/// Be cautious to never log any personally identifiable information (PII) or credentials. It is
/// the responsibility of the developer to ensure that sensitive data is not exposed in the logs.
///
/// # Examples
/// ```
/// let log_level = "info"; // Typically obtained from command line arguments or env vars
/// if let Err(e) = init_logging(log_level) {
///     eprintln!("Logger initialization failed: {:?}", e);
///     // Handle error appropriately (e.g., exit the program or fallback to a default configuration)
/// }
/// ```
pub(crate) fn steady_logging_init(level: &str) -> Result<(), Box<dyn std::error::Error>> {
    let log_spec = LogSpecification::env_or_parse(level)?;

    Logger::with(log_spec)
        .format(flexi_logger::colored_with_thread)
        .start()?;

    /////////////////////////////////////////////////////////////////////
    // for all log levels use caution and never write any personal identifiable data
    // to the logs. YOU are always responsible to ensure credentials are never logged.
    ////////////////////////////////////////////////////////////////////
    if log::log_enabled!(log::Level::Trace) || log::log_enabled!(log::Level::Debug)  {

        trace!("Trace: deep application tracing");
        //Rationale: "Trace" level is typically used for detailed debugging information, often in a context where the flow through the system is being traced.

        debug!("Debug: complex part analysis");
        //Rationale: "Debug" is used for information useful in a debugging context, less detailed than trace, but more so than higher levels.

        warn!("Warn: recoverable issue, needs attention");
        //Rationale: Warnings indicate something unexpected but not necessarily fatal; it's a signal that something should be looked at but isn't an immediate failure.

        error!("Error: unexpected issue encountered");
        //Rationale: Errors signify serious issues, typically ones that are unexpected and may disrupt normal operation but are not application-wide failures.

        info!("Info: key event occurred");
        //Rationale: Info messages are for general, important but not urgent information. They should convey key events or state changes in the application.
    }
    Ok(())
}
////////////////////////////////////////////
////////////////////////////////////////////


struct Node<'a> {
    id: & 'a String,
    color: & 'static str,
    pen_width: & 'static str,
    label: String, //label may also have (n) for replicas
}

struct Edge<'a> {
    from: & 'a String,
    to: & 'a String,
    color: & 'static str,
    pen_width: & 'static str,
    label: String,
}

fn build_dot(nodes: Vec<Node>, edges: Vec<Edge>, rankdir: &str, dot_graph: &mut BytesMut) {
    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(rankdir.as_bytes());
    dot_graph.put_slice(b";\n");

    for node in nodes {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(node.id.as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    for edge in edges {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(edge.from.as_bytes());
        dot_graph.put_slice(b"\" -> \"");
        dot_graph.put_slice(edge.to.as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(edge.label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(edge.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(edge.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    dot_graph.put_slice(b"}\n");
}


////////////////////////////////////////////
////////////////////////////////////////////

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;

    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    pub(crate) fn initialize_logger() {
        INIT.call_once(|| {
            if let Err(e) = steady_logging_init("info") {
                print!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
            }
        });
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string): (SteadyTx<String>, _) = graph.new_channel(8,&[]);

        let mut monitor = new_monitor_internal(&mut graph,"test", None).await;
        let mut monitor = monitor.init_stats(&[&rx_string], &[&tx_string]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.tx(&tx_string, "test".to_string()).await;
            count += 1;
        }
        let idx = monitor.monitor.shared_tx_array.read().await[tx_string.id] as usize;

        assert_eq!(monitor.local_sent_count[idx], threshold);
        monitor.relay_stats_tx_custom(&tx_string, threshold).await;
        assert_eq!(monitor.local_sent_count[idx], 0);

        while count > 0 {
            let x = monitor.rx(&rx_string).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        let idx = monitor.monitor.shared_rx_array.read().await[rx_string.id] as usize;

        assert_eq!(monitor.local_received_count[idx], threshold);
        monitor.relay_stats_rx_custom(&rx_string.clone(), threshold).await;
        assert_eq!(monitor.local_received_count[idx], 0);

    }

    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string): (SteadyTx<String>, _) = graph.new_channel(5,&[]);

        let mut monitor = new_monitor_internal(&mut graph, "test", None).await;
        let mut monitor = monitor.init_stats(&[&rx_string], &[&tx_string]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {

            let _ = monitor.tx(&tx_string, "test".to_string()).await;
            count += 1;
            let idx = monitor.monitor.shared_tx_array.read().await[tx_string.id] as usize;
            assert_eq!(monitor.local_sent_count[idx], count);
            monitor.relay_stats_batch().await;
        }
        let idx = monitor.monitor.shared_tx_array.read().await[tx_string.id] as usize;
        assert_eq!(monitor.local_sent_count[idx], 0);

        while count > 0 {
            let x = monitor.rx(&rx_string).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        let idx = monitor.monitor.shared_rx_array.read().await[rx_string.id] as usize;

        assert_eq!(monitor.local_received_count[idx], threshold);
        monitor.relay_stats_batch().await;
        assert_eq!(monitor.local_received_count[idx], 0);


    }
}

/////////////////////////////////
