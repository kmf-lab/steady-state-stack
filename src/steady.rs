
use std::any::type_name;
use std::future::Future;

use std::sync::Arc;
use std::time::{Duration, Instant};
use async_std::sync::RwLock;

use bastion::{Callbacks, run};
use bastion::children::Children;

use bastion::prelude::*;




use futures_timer::Delay;
use log::{error, info};
use petgraph::matrix_graph::Zero;
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady_feature::steady_feature;

////
//define our internal channel implementation for use
////
use flume;

type InternalSender<T> = flume::Sender<T>;
type InternalReceiver<T> = flume::Receiver<T>;

#[inline]
fn build_internal_channel<T>(cap: usize) -> (InternalSender<T>,InternalReceiver<T>)  {
    flume::bounded(cap)
}

////

mod telemetry {
    pub mod metrics_collector;
    pub mod telemetry_logging;
    pub mod telemetry_streaming;
    pub mod telemetry_polling;

}

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

pub struct SteadyMonitor {
    name: & 'static str,
    last_instant: Instant,
    ctx: Option<BastionContext>,
    id: usize, //unique identifier for this child group
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
    #[allow(dead_code)]
    pub(crate) fn ctx(&self) -> Option<&BastionContext> {
       if let Some(ctx) = &self.monitor.ctx {
           Some(ctx)
       } else {
           None
       }
    }
}

impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {
    pub async fn relay_stats_all(&mut self) {

        // do not run any faster than the framerate of the telemetry can consume
        //TODO: this is not right needs alignment with the periodic counter and other calls
        //let run_duration: Duration = Instant::now().duration_since(self.monitor.last_instant);
        //if run_duration.as_millis() as usize >= steady_feature::MIN_TELEMETRY_CAPTURE_RATE_MS
        {

            // switch to a new vector and send the old one
            // we always clear the count so we can confirm this in testing
            if self.local_sent_count.iter().any(|x| !x.is_zero()) {
                // only relay if there is room otherwise we change nothing and roll up locally
                //we need something worth saying before we send
                if self.telemetry_send_tx.has_room() {
                    //we only send the result if we have a context, ie a graph we are monitoring
                    if self.monitor.ctx.is_some() {
                        self.telemetry_tx_count(self.local_sent_count).await;
                        if !self.telemetry_send_tx.has_room() {
                            error!("Telemetry channel is full for {}. Unable to send more telemetry, capacity is {}. It will be accumulated locally."
                            ,self.monitor.name
                            ,self.telemetry_send_tx.batch_limit);
                        }
                    }
                    self.local_sent_count.fill(0);
                }
            }
            if self.local_received_count.iter().any(|x| !x.is_zero()) {
                if self.telemetry_send_rx.has_room() {
                    //we only send the result if we have a context, ie a graph we are monitoring
                    if self.monitor.ctx.is_some() {
                        self.telemetry_rx_count(self.local_received_count).await;
                        if !self.telemetry_send_rx.has_room() {
                            error!("Telemetry channel is full for {}. Unable to send more telemetry, capacity is {}. It will be accumulated locally."
                        ,self.monitor.name
                        ,self.telemetry_send_rx.batch_limit);
                        }
                    }
                    self.local_received_count.fill(0);
                }
            }
        }
    }

    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {
        assert_eq!(true, duration_rate.ge(&Duration::from_millis(steady_feature::MIN_TELEMETRY_CAPTURE_RATE_MS as u64)));

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
    pub async fn rx<T>(self: &mut Self, this: &SteadyRx<T>) -> Result<T,String> {

        match self.monitor.rx(this).await {
            Ok(result) => {
                let guard = self.monitor.shared_rx_array.read().await;
                if guard.len() > this.id {
                    let idx = guard[this.id];
                    drop(guard);
                    if u8::MAX != idx { //only record those turned on (init)
                        self.local_received_count[idx as usize] =
                        self.local_received_count[idx as usize].saturating_add(1);
                    }
                }
                Ok(result)
            },
            Err(msg) => {
                error!("Unexpected error recv_async: {} {}", msg, self.monitor.name);
                Err(msg)
            }
        }
    }
    pub async fn tx<T>(self: &mut Self, this: &SteadyTx<T>, a: T) -> Result<(), T> {
       match self.monitor.tx(this, a).await {
           Ok(_) => {
               let guard = self.monitor.shared_tx_array.read().await;
               if guard.len() > this.id {
                   let idx  = guard[this.id];
                   drop(guard);
                   if u8::MAX != idx { //only record those turned on (init)
                      self.local_sent_count[idx as usize] =
                      self.local_sent_count[idx as usize].saturating_add(1);
                   }
               }
               return Ok(())
           },
           Err(d) => {
               return Err(d);
           }
       }

    }

    async fn telemetry_tx_count(self: &mut Self, a: [usize; TXL])  {
        //we try to send but if the channel is full we log that fact
        match self.telemetry_send_tx.try_send(a) {
            Ok(_) => {
                let guard = self.monitor.shared_tx_array.read().await;
                if guard.len() > self.telemetry_send_tx.id {
                    let idx  = guard[self.telemetry_send_tx.id];
                    drop(guard);
                    if u8::MAX != idx { //only record those turned on (init)
                        self.local_sent_count[idx as usize]
                        = self.local_sent_count[idx as usize].saturating_add(1);
                    }
                }
            },
            Err(a) => {
                error!("full telemetry channel detected from actor: {} capacity:{:?} value:{:?} "
                , self.monitor.name, self.telemetry_send_tx.batch_limit,a);
                //never block instead just return what we could not send
            }
        };
    }
    async fn telemetry_rx_count(self: &mut Self, a: [usize; RXL])  {
        //we try to send but if the channel is full we log that fact
        match self.telemetry_send_rx.try_send(a) {
            Ok(_) => {
                let guard = self.monitor.shared_rx_array.read().await;
                if guard.len() > self.telemetry_send_rx.id {
                    let idx  = guard[self.telemetry_send_rx.id];
                    drop(guard);
                    if u8::MAX != idx { //only record those turned on (init)
                        self.local_received_count[idx as usize] += 1;
                    }
                }
            },
            Err(a) => {
                error!("full telemetry channel detected from actor: {} capacity:{:?} value:{:?} "
                , self.monitor.name, self.telemetry_send_rx.batch_limit,a);
                //never block instead just return what we could not send
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
        let mut map_rx = [0; RX_LEN];
        let mut rx_array = run!(async {self.shared_rx_array.write().await});
        rx_def.iter()
            .enumerate()
            .for_each(|(c,rx)| {
                    rx_array[rx.id()]=c as u8;
                    rx_bash_limit[c] = rx.bash_limit();
                    map_rx[c] = rx.id();
            });
        drop(rx_array);


        let mut tx_bash_limit = [0; TX_LEN];
        let mut map_tx = [0; TX_LEN];
        let mut tx_array = run!(async {self.shared_tx_array.write().await});
        tx_def.iter()
            .enumerate()
            .for_each(|(c,tx)| {
                tx_array[tx.id()]=c as u8;
                tx_bash_limit[c] = tx.bash_limit();
                map_tx[c] = tx.id();
            });
        drop(tx_array);


        let (telemetry_send_tx, telemetry_take_tx): (SteadyTx<[usize; TX_LEN]>, SteadyRx<[usize; TX_LEN]>)
                                                       =  build_channel(steady_feature::CHANNEL_LENGTH_TO_COLLECTOR
                                                                        , &["steady-telemetry"]
                                                                        , self.shared_tx_array.clone()
                                                                        , self.shared_rx_array.clone());

        let (telemetry_send_rx, telemetry_take_rx): (SteadyTx<[usize; RX_LEN]>, SteadyRx<[usize; RX_LEN]>)
                                                       =  build_channel(steady_feature::CHANNEL_LENGTH_TO_COLLECTOR
                                                                        , &["steady-telemetry"]
                                                                        , self.shared_tx_array.clone()
                                                                        , self.shared_rx_array.clone());

        run!(async{
                self.all_telemetry_rx.write().await.push(
                    CollectorDetail{
                          name: self.name
                        , id: self.id
                        , telemetry_take: Box::new(SteadyTelemetryRx {
                                                    read_tx: telemetry_take_tx, map_tx,
                                                    read_rx: telemetry_take_rx, map_rx,
                                                })        }
                );
            });


        LocalMonitor::<RX_LEN,TX_LEN> {
            telemetry_send_rx,  telemetry_send_tx,

            monitor: self,  //TODO: should be some kind of clone??

            local_sent_count: [0; TX_LEN],
            local_sent_capacity: tx_bash_limit,

            local_received_count: [0; RX_LEN],
            local_received_capacity: rx_bash_limit
        }
    }


    //TODO: add methods later to instrument non await blocks for cpu usage chart


    //TODO: wrap these methos to add telemetry after trasnsmit..

    pub async fn rx<T>(self: &mut Self, this: &SteadyRx<T>) -> Result<T,String> {
        this.take_async().await
    }


    pub async fn tx<T>(self: &mut Self, this: &SteadyTx<T>, a: T) -> Result<(), T> {
        //we try to send but if the channel is full we log that fact
        match this.try_send(a) {
            Ok(_) => {Ok(())},
            Err(a) => {
                error!("full channel detected data_approval tx  actor: {} capacity:{:?} type:{} "
                , self.name, this.batch_limit, type_name::<T>());
                //here we will await until there is room in the channel
                this.send_async(a).await
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
        let id = self.monitor_count;
        self.monitor_count += 1;

        build_new_monitor(name, None, id
                          , self.shared_rx_array.clone()
                          , self.shared_tx_array.clone()
                          , self.all_telemetry_rx.clone()).await

    }
}



fn build_channel<T>(
                     cap: usize, labels: & 'static [& 'static str]
                    , sta: Arc<RwLock<Vec<u8>>>
                    , sra: Arc<RwLock<Vec<u8>>>) -> (SteadyTx<T>, SteadyRx<T>) {
    let (tx, rx): (InternalSender<T>,InternalReceiver<T>) = build_internal_channel(cap);
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

    //TODO: this is example code but not called
    /*
    fn create<T, const N: usize>() {
        //let mut rb = StaticRb::<usize, N>::default();
        //let (mut prod: StaticRbProducer<i32, RB_SIZE>, mut cons: StaticRbConsumer<i32, RB_SIZE>) = rb.split();
        // let (mut prod, mut cons) = rb.split_ref();
//: (CachingProd<&SharedRb<Static<T, N>>>, CachingCons<&SharedRb<Static<T, N>>>)

        const RB_SIZE: usize = 1;
        let mut rb = StaticRb::<i32, RB_SIZE>::default();
        let (mut prod, mut cons) = rb.split_ref();

        assert_eq!(prod.try_push(123), Ok(()));
        assert_eq!(prod.try_push(321), Err(321));

        assert_eq!(cons.read_index(), 123);

        assert_eq!(cons.try_pop(), Some(123));
        assert_eq!(cons.try_pop(), None);

        //TODO: new and beter design here so we can peek and never loose a read ! (pronghorn)
    }
//     */


    fn configure_for_graph<F,I>(graph: & mut SteadyGraph, name: & 'static str, c: Children, init: I ) -> Children
            where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
                  F: Future<Output = Result<(),()>> + Send + 'static ,
        {
            let result = {
                let init_clone = init.clone();

                let (id, telemetry_tx, shared_rx_array, shared_tx_array) =
                    run!(async  {

                        let id = graph.monitor_count;
                        graph.monitor_count += 1;
                        (id, graph.shared_rx_array.clone(), graph.shared_tx_array.clone(), graph.all_telemetry_rx.clone())

                    });

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

            let mut outgoing = Vec::new();

            let supervisor = if steady_feature::TELEMETRY_LOGGING {
                let (tx, rx) = self.new_channel::<DiagramData>(steady_feature::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
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

            let supervisor = if steady_feature::TELEMETRY_STREAMING {
                let (tx, rx) = self.new_channel::<DiagramData>(steady_feature::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
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

            let supervisor = if steady_feature::TELEMETRY_POLLING {
                let (tx, rx) = self.new_channel::<DiagramData>(steady_feature::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
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

                                                        let tel_tx_array: [SteadyTx<DiagramData>; steady_feature::FEATURE_LEN]
                                                                           = outgoing.clone().try_into()
                                                                             .unwrap_or_else(|_| panic!("Incorrect length"));

                                                        let all_rx = all_tel_rx.clone();
                                                         telemetry::metrics_collector::run(monitor
                                                                                      , all_rx
                                                                                      , tel_tx_array

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
    //NOTE: we will do one dyn call per node every 32ms or so to build the image
    //      we only have 1 impl assuming the compiler will inline this if possible
    // TODO: in the future we could rewrite this to return a future that can be pinned and boxed
    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);
    fn biggest_tx_id(&self) -> usize;
    fn biggest_rx_id(&self) -> usize;

    //returns an iterator of usize channel ids for tx channels
    fn tx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_>;
    fn rx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_>;
}
impl <const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL,TXL> {

    #[inline]
    fn tx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_> {
        Box::new((0..=self.biggest_tx_id()).into_iter())
    }
    #[inline]
    fn rx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_> {
        Box::new((0..=self.biggest_tx_id()).into_iter())
    }

    #[inline]
    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>) {

            //this method can not be async since we ned vtable and dyn
            //so we will return a future to be used later which will also remove any dyn call.

            //read all the messages available and put the total into the right vec index
            //do for both reads and sends.
            run!(async {
            loop {
                let mut count = 0;

                if self.read_rx.has_message() {
                    match self.read_rx.take_async().await {
                        Ok(a) => {
                            self.map_rx.iter().zip(a.iter())
                                .for_each(|(idx, val)| take_target[*idx] += *val as u128);
                        },
                        Err(msg) => {
                            error!("Unexpected error recv_async: {}",msg);
                        }
                    }
                } else {
                    count += 1;
                }
                if self.read_tx.has_message() {
                    match self.read_tx.take_async().await {
                        Ok(a) => {
                            self.map_tx.iter().zip(a.iter())
                                .for_each(|(idx, val)| send_target[*idx] += *val as u128);
                        },
                        Err(msg) => {
                            error!("Unexpected error recv_async: {}",msg);
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

    }

    fn biggest_tx_id(&self) -> usize {
        self.map_tx.iter().max().unwrap_or(&0).clone()
    }

    fn biggest_rx_id(&self) -> usize {
        self.map_rx.iter().max().unwrap_or(&0).clone()
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
    id: usize,
    batch_limit: usize,
    rx: Arc<InternalReceiver<T>>,
    label: &'static [& 'static str],
}

#[derive(Clone, Debug)]
pub struct SteadyTx<T> {
    id: usize,
    batch_limit: usize,
    tx: Arc<InternalSender<T>>,
    label: &'static [& 'static str],

}


impl<T> SteadyRx<T> {
    async fn new(receiver: InternalReceiver<T>, id: usize, label: &'static [& 'static str]) -> Self {
        SteadyRx { id, batch_limit: receiver.capacity().unwrap_or(usize::MAX)
                           , rx: Arc::new(receiver), label
        }
    }

    #[inline]
    pub async fn take_async(&self) -> Result<T,String> {
         match self.rx.recv_async().await {
            Ok(a) => {Ok(a)}
            Err(e) => {Err(e.to_string())}
        }
    }

    #[inline]
    pub async fn take_now(&self) -> Option<T> {
        match self.rx.recv() {
            Ok(a) => {Some(a)}
            Err(_) => {None}
        }
    }

    #[inline]
    pub fn try_peek(&self) -> Option<&T> {
        todo!();
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
impl<T> SteadyTx<T> {
    async fn new(sender: InternalSender<T>, id: usize, label: &'static [& 'static str]) -> Self {
        SteadyTx {id, batch_limit: sender.capacity().unwrap_or(usize::MAX)
            , tx: Arc::new(sender), label }
    }



    #[inline]
    pub fn try_send(&self, msg: T) -> Result<(), T> {
        match self.tx.try_send(msg) {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m.into_inner())}
        }
    }

    #[inline]
    pub async fn send_async(&self, msg: T) -> Result<(), T> {
        match self.tx.send_async(msg).await {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m.into_inner())}
        }
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
////////////////////////////////////////////
////////////////////////////////////////////

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use crate::steady_util::steady_util;


    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    pub(crate) fn initialize_logger() {
        INIT.call_once(|| {
            if let Err(e) = steady_util::steady_logging_init("info") {
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

        let monitor = graph.new_test_monitor("test").await;
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

        let monitor = graph.new_test_monitor("test").await;
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