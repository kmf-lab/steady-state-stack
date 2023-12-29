
use std::any::{type_name};
use std::future::Future;
use std::mem::replace;

use std::sync::Arc;
use std::time::{Duration, Instant};
use async_std::sync::RwLock;

use bastion::{Callbacks, run};
use bastion::children::Children;

use bastion::prelude::*;
use flume::{Receiver, Sender, bounded, RecvError};
use futures_timer::Delay;
use log::{error, info, trace};
use crate::steady::telemetry::metrics_collector::DiagramData;

mod telemetry {
    pub mod metrics_collector;
    pub mod server;
}

// An Actor is a single threaded block of behavior that can send and receive messages
// A Troupe is a group of actors that are all working together to provide a specific service
// A Guild is a group of Troupes that are all working together on the same problem
// A Kingdom is a group of Guilds that are all working on the same problem
// A collection of kingdoms is a world


pub struct SteadyGraph {
    senders_count: usize,
    receivers_count: usize,
    rx_vec: Arc<RwLock<Vec<SteadyRx<Telemetry>>>>,
    shared_rx_array: Option<Arc<RwLock<Box<[u8]>>>>, //TODO: what if this is a vec instead??
    shared_tx_array: Option<Arc<RwLock<Box<[u8]>>>>,
}


//telemetry enum every actor uses to send information about actors name messages send and messages received
#[derive(Clone)]
pub enum Telemetry {
    ActorDef(& 'static str),
    MessagesSent(Vec<usize>),
    MessagesReceived(Vec<usize>),
    MessagesIndexSent(Vec<usize>),
    MessagesIndexReceived(Vec<usize>),
}


#[derive(Clone)]
pub struct SteadyControllerBuilder {
    telemetry_tx: SteadyTx<Telemetry>,
    shared_rx_array: Arc<RwLock<Box<[u8]>>>,
    shared_tx_array: Arc<RwLock<Box<[u8]>>>,
}



pub struct SteadyMonitor {
    pub name: & 'static str,
    last_instant: Instant,
    pub ctx: Option<BastionContext>,

    pub sent_count: Vec<usize>,
    pub sent_capacity: Vec<usize>,
    pub sent_index: Vec<usize>,

    pub received_count: Vec<usize>,
    pub received_capacity: Vec<usize>,
    pub received_index: Vec<usize>,

    pub base: SteadyControllerBuilder, //keep this instead of moving the 3 values

}

impl SteadyMonitor {

    pub async fn relay_stats_all(self: &mut Self) {
        // switch to a new vector and send the old one
        // we always clear the count so we can confirm this in testing

        let sc = self.sent_count.len();
        let msg_sent_count = Telemetry::MessagesSent(replace(&mut self.sent_count
                                                     , vec![0; sc]));

        let rc = self.received_count.len();
        let msg_received_count = Telemetry::MessagesReceived(replace(&mut self.received_count
                                                         , vec![0; rc]));

        //we only send the result if we have a context, ie a graph we are monitoring
        if self.ctx.is_some() {
            self.telemetry_tx(msg_sent_count).await;
            self.telemetry_tx(msg_received_count).await;
        }

    }

    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {
        self.relay_stats_all().await;
        let run_duration:Duration = Instant::now().duration_since(self.last_instant);
        Delay::new(duration_rate.saturating_sub(run_duration)).await;
        self.last_instant = Instant::now();
    }

    pub async fn relay_stats_batch(self: & mut Self) {
        //only relay if one of the channels has reached or passed the capacity
        //walk both vecs to determine if any count has reached or passed the capacity
        let relay_now = self.sent_count.iter().zip(self.sent_capacity.iter()).any(|(count, capacity)| {
             *count >= *capacity
        });
        let relay_now = if !relay_now {
            self.received_count.iter().zip(self.received_capacity.iter()).any(|(count, capacity)| {
                *count >= *capacity
            })
        } else {
            relay_now
        };
        if relay_now {
            self.relay_stats_all().await;
        }
    }

    pub async fn relay_stats_tx_custom<T>(self: & mut Self, tx: SteadyTx<T>, threshold: usize) {
        let idx = self.base.shared_tx_array.read().await[tx.id] as usize;
        if self.sent_count[idx] >= threshold {
            self.relay_stats_all().await;
        }
    }

    pub async fn relay_stats_rx_custom<T>(self: & mut Self, rx: SteadyRx<T>, threshold: usize) {
        let idx = self.base.shared_tx_array.read().await[rx.id] as usize;
        if self.received_count[idx] >= threshold {
            self.relay_stats_all().await;
        }
    }




    async fn update_tx_count_internal(self: &mut Self, that_id: usize, that_capacity: usize) {
//get the index but grow if we must
        let index = self.base.shared_tx_array.read().await[that_id];
        let index = if u8::MAX != index {
            index as usize
        } else {
            let idx = self.sent_count.len() as u8;
            self.base.shared_tx_array.write().await[that_id] = idx;

            self.sent_count.push(0);
            self.sent_capacity.push(that_capacity);
            self.sent_index.push(that_id);

            if self.ctx.is_some() {
                self.base.telemetry_tx.sender.send_async(Telemetry::MessagesIndexSent(self.sent_index.clone())).await
                    .expect("Unable to send telemetry"); //TODO: fix error handling
            }
            idx as usize
        };
        self.sent_count[index] += 1;
    }

    async fn update_rx_count_internal(self: &mut Self, that_id: usize, that_capacity: usize) {
//get the index but grow if we must
        let index = self.base.shared_rx_array.read().await[that_id];
        let index = if u8::MAX != index {
            index as usize
        } else {
            let idx = self.received_count.len() as u8;
            self.base.shared_rx_array.write().await[that_id] = idx;
            self.received_count.push(0);
            self.received_capacity.push(that_capacity);
            self.received_index.push(that_id);
            if self.ctx.is_some() {
                self.base.telemetry_tx.sender.send_async(Telemetry::MessagesIndexReceived(self.received_index.clone())).await
                    .expect("Unable to send telemetry"); //TODO: fix error handling
            }
            idx as usize
        };
        self.received_count[index] += 1;
    }


    pub async fn rx<T>(self: &mut Self, this: &SteadyRx<T>) -> Result<T, RecvError> {

        //if self ident not set then set it.

        let result = this.receiver.recv_async().await;
        if result.is_ok() {
            self.update_rx_count_internal(this.id, this.capacity).await;
        }
        result
    }


    pub async fn tx<T>(self: &mut Self, this: &SteadyTx<T>, a: T) {
        //we try to send but if the channel is full we log that fact
        match this.sender.try_send(a) {
            Ok(_) => {
                self.update_tx_count_internal(this.id, this.capacity).await;
            },
            Err(flume::TrySendError::Full(a)) => {
                error!("full channel detected data_approval tx  actor: {} capacity:{:?} type:{} "
                , self.name, this.capacity, type_name::<T>());
                //here we will await until there is room in the channel
                match this.sender.send_async(a).await {
                    Ok(_) => {
                        self.update_tx_count_internal(this.id, this.capacity).await;
                    },
                    Err(e) => {
                        error!("Unexpected error sending: {} {}",e, self.name);
                    }
                };
            },
            Err(e) => {
                error!("Unexpected error try sending: {} {}",e, self.name);
            }
        };
    }

    async fn telemetry_tx(self: &mut Self, a: Telemetry) {
        //we try to send but if the channel is full we log that fact
        match self.base.telemetry_tx.sender.try_send(a) {
            Ok(_) => {
                self.update_tx_count_internal(self.base.telemetry_tx.id, self.base.telemetry_tx.capacity).await;
            },
            Err(flume::TrySendError::Full(a)) => {
                error!("full channel detected data_approval tx  actor: {} capacity:{:?} "
                , self.name, self.base.telemetry_tx.capacity);
                //here we will await until there is room in the channel
                match self.base.telemetry_tx.sender.send_async(a).await {
                    Ok(_) => {
                        self.update_tx_count_internal(self.base.telemetry_tx.id, self.base.telemetry_tx.capacity).await;
                    },
                    Err(e) => {
                        error!("Unexpected error sending: {} {}",e, self.name);
                    }
                };
            },
            Err(e) => {
                error!("Unexpected error try sending: {} {}",e, self.name);
            }
        };
    }



}

impl SteadyControllerBuilder {

    pub(crate) fn callbacks(&self) -> Callbacks {
        //  Callbacks::new().
        todo!()
    }

    pub fn configure_for_graph<F,I>(self: &mut Self, root_name: & 'static str, c: Children, init: I ) -> Children
        where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,
    {
        #[cfg(test)] {
            let s = self.clone();
            let init_clone = init.clone();
            return c.with_callbacks(self.callbacks())
                .with_exec(move |ctx| {
                    let init_clone = init_clone.clone();
                    let s = s.clone();
                    async move {
                            let monitor = s.wrap(root_name,Some(ctx));
                            match init_clone(monitor).await {
                                Ok(r) => {
                                    info!("Actor {:?} ", r); //TODO: we may want to exit here
                                },
                                Err(e) => {
                                    error!("{:?}", e);
                                }
                            }
                            Ok(())
                    }
                })
                .with_name(root_name)
                .with_distributor(Distributor::named(format!("testing-{root_name}")));
        }
        #[cfg(not(test))] {
            let s = self.clone();
            let init_clone = init.clone();
            return c.with_callbacks(self.callbacks())
                .with_exec(move |ctx| {
                    let init_clone = init_clone.clone();
                    let s = s.clone();
                    async move {
                        let monitor = s.wrap(root_name, ctx);
                        match init_clone(monitor).await {
                            Ok(r) => {
                                info!("Actor {:?} ", r); //TODO: we may want to exit here
                            },
                            Err(e) => {
                                error!("{:?}", e);
                            }
                        }
                        Ok(())
                    }
                })
                .with_name(root_name);
        }
    }

    pub fn wrap(self, root_name: & 'static str, ctx: Option<BastionContext>) -> SteadyMonitor {
        let tx = self.telemetry_tx.clone();
        let is_in_graph = ctx.is_some();
        let mut monitor = self.new_monitor(root_name, ctx);
        if is_in_graph {
            run!(
                async {
                    monitor.tx(&tx, Telemetry::ActorDef(root_name)).await;
                }
            );
        }
        monitor
    }

    fn new_monitor(self, name: & 'static str, ctx: Option<BastionContext>) -> SteadyMonitor {
        SteadyMonitor {
            last_instant: Instant::now(),
            name,
            ctx,
            sent_count: Vec::new(),
            sent_capacity: Vec::new(),
            sent_index: Vec::new(),
            received_count: Vec::new(),
            received_capacity: Vec::new(),
            received_index: Vec::new(),
            base: self, //onwership transfered
        }
    }
}


impl SteadyGraph {
    //wrap the context with a new context that has a telemetry channel


    pub(crate) fn new() -> SteadyGraph {
        SteadyGraph {
            senders_count: 0,
            receivers_count: 0,
            rx_vec: Arc::new(RwLock::new(Vec::new())),
            shared_rx_array: None,
            shared_tx_array: None,
        }
    }

    pub fn new_channel<T: Send>(&mut self, cap: usize) -> (SteadyTx<T>, SteadyRx<T>) {
        let (tx, rx): (Sender<T>, Receiver<T>) = bounded(cap);
        let result = (SteadyTx::new(tx, self), SteadyRx::new(rx, self));
        self.senders_count += 1;
        self.receivers_count += 1;
        result
    }


    pub async fn new_monitor(&mut self) -> SteadyControllerBuilder {
        let (tx, rx) = self.new_channel::<Telemetry>(8);

        self.rx_vec.write().await.push(rx);

        // Here we are creating a shared array of bytes that will be used to store the
        // indexes of the channels that are used by each actor.

        //NOTE: each single actor child is limited to 254 channels

        let expected_count = (&self).receivers_count;
        if (&self).shared_rx_array.is_none() {
            self.shared_rx_array = Some(Arc::new(RwLock::new(vec![u8::MAX; expected_count].into_boxed_slice())));
        } else {
            //if someone is creating new channels after children this code will compensate
            let rw_lock = &(&self).shared_rx_array.as_ref().unwrap();
            let len = rw_lock.read().await.len();

            if len != expected_count {
                //this is a feature not a bug but it does require a little more memory to be dynamic
                trace!("Using more memory than needed. Consider building the channels before the actors.");
                self.shared_rx_array = Some(Arc::new(RwLock::new(vec![u8::MAX; expected_count].into_boxed_slice())));
            }
        }

        let expected_count = (&self).senders_count;
        if (&self).shared_tx_array.is_none() {
            self.shared_tx_array = Some(Arc::new(RwLock::new(vec![u8::MAX; expected_count].into_boxed_slice())));
        } else {
            //if someone is creating new channels after children this code will compensate
            let rw_lock = &(&self).shared_tx_array.as_ref().unwrap();
            let len = rw_lock.read().await.len();

            if len != expected_count {
                //this is a feature not a bug but it does require a little more memory to be dynamic
                trace!("Using more memory than needed. Consider building the channels before the actors.");
                self.shared_tx_array = Some(Arc::new(RwLock::new(vec![u8::MAX; expected_count].into_boxed_slice())));
            }
        }

        //these are not real clones, they are just references to the same data
        let tx_a = self.shared_tx_array.clone().expect("Shared tx array is None");
        let rx_a = self.shared_rx_array.clone().expect("Shared rx array is None");

        SteadyControllerBuilder {
            telemetry_tx: tx,
            shared_tx_array: tx_a,
            shared_rx_array: rx_a,
        }
    }


    pub fn add_to_graph<F,I>(self: &mut Self, root_name: & 'static str, c: Children, init: I ) -> Children
        where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,
    {
        run!( async {
              self.new_monitor().await.configure_for_graph(root_name, c, init)
            }
        )
    }

    pub(crate) fn init_telemetry(&mut self) {

        let (tx, rx): (SteadyTx<DiagramData>, _) = self.new_channel(8);

        let _ = Bastion::supervisor(|supervisor|
                                supervisor.with_strategy(SupervisionStrategy::OneForOne)
                                    .children(|children| {
                                        let all_rx = self.rx_vec.clone();
                                        self.add_to_graph("generator"
                                                           , children.with_redundancy(0)
                                                           , move |monitor|
                                                    telemetry::metrics_collector::telemetry(monitor
                                                                                                           ,all_rx.clone()
                                                                                                           ,tx.clone()
                                                    )
                                                )

                                    })
                                    .children(|children| {
                                        let all_rx = self.rx_vec.clone();
                                        self.add_to_graph("generator"
                                                          , children.with_redundancy(0)
                                                          , move |monitor|
                                                              telemetry::server::telemetry(monitor
                                                                                                          ,rx.clone()
                                                              )
                                        )

                                    })
        ).expect("OneForOne supervisor creation error.");
    }
}


////////////////////////////////////////////////////////////////
#[derive(Clone)]
pub struct SteadyTx<T> {
    pub id: usize,
    pub capacity: usize,
    pub sender: Sender<T>
}

impl<T: Send> SteadyTx<T> {
    fn new(sender: Sender<T>, model: &SteadyGraph) -> Self {
        SteadyTx {id:model.senders_count
                  , capacity: sender.capacity().unwrap_or(usize::MAX)
                  , sender:sender }
    }

    #[allow(dead_code)]
    pub fn has_room(&self) -> bool {
        !self.sender.is_full()
    }



}


////////////////////////////////////////////////////////////////
#[derive(Clone)]
pub struct SteadyRx<T> {
    pub id: usize,
    pub capacity: usize,
    pub receiver: Receiver<T>,

}
impl<T: Send> SteadyRx<T> {
    pub fn new(receiver: Receiver<T>, model: &SteadyGraph) -> Self {
        SteadyRx { id: model.receivers_count
                     , capacity: receiver.capacity().unwrap_or(usize::MAX)
                           , receiver
        }
    }
    #[allow(dead_code)]
    pub fn has_message(&self) -> bool {
        !self.receiver.is_empty()
    }



}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_std::test;
    use flexi_logger::{colored_with_thread, Logger, LogSpecification};
    use lazy_static::lazy_static;
    use std::sync::Once;

    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    pub(crate) fn initialize_logger() {
        INIT.call_once(|| {
            let _ = Logger::with(LogSpecification::env_or_parse("info").unwrap())
                .format(colored_with_thread)
                .start()
                .expect("Logger initialization failed");
        });
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string): (SteadyTx<String>, _) = graph.new_channel(8);

        let mut monitor = graph.new_monitor().await.wrap("test", None);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            monitor.tx(&tx_string, "test".to_string()).await;
            count += 1;
        }
        let idx = monitor.base.shared_tx_array.read().await[tx_string.id] as usize;

        assert_eq!(monitor.sent_count[idx], threshold);
        monitor.relay_stats_tx_custom(tx_string.clone(), threshold).await;
        assert_eq!(monitor.sent_count[idx], 0);

        while count > 0 {
            let x = monitor.rx(&rx_string).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        let idx = monitor.base.shared_rx_array.read().await[rx_string.id] as usize;

        assert_eq!(monitor.received_count[idx], threshold);
        monitor.relay_stats_rx_custom(rx_string.clone(), threshold).await;
        assert_eq!(monitor.received_count[idx], 0);


    }

    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string): (SteadyTx<String>, _) = graph.new_channel(5);

        let mut monitor = graph.new_monitor().await.wrap("test", None);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {

            monitor.tx(&tx_string, "test".to_string()).await;
            count += 1;
            let idx = monitor.base.shared_tx_array.read().await[tx_string.id] as usize;
            assert_eq!(monitor.sent_count[idx], count);
            monitor.relay_stats_batch().await;
        }
        let idx = monitor.base.shared_tx_array.read().await[tx_string.id] as usize;
        assert_eq!(monitor.sent_count[idx], 0);

        while count > 0 {
            let x = monitor.rx(&rx_string).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        let idx = monitor.base.shared_rx_array.read().await[rx_string.id] as usize;

        assert_eq!(monitor.received_count[idx], threshold);
        monitor.relay_stats_batch().await;
        assert_eq!(monitor.received_count[idx], 0);


    }
}

