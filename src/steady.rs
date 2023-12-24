
use std::any::{type_name};
use std::future::Future;

use std::sync::Arc;
use std::time::{Duration, Instant};
use async_std::sync::RwLock;

use bastion::{Callbacks, run};
use bastion::children::Children;

use bastion::prelude::*;
use flume::{Receiver, Sender, bounded, RecvError};
use futures_timer::Delay;
use log::{error, info, trace};


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
    ActorDef(String),
    MessagesSent(Vec<u64>),
    MessagesReceived(Vec<u64>),
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
    last_instant: Instant,
    pub ctx: BastionContext,

    pub sent_count: Vec<u64>,
    pub sent_capacity: Vec<u64>,
    pub sent_index: Vec<usize>,

    pub received_count: Vec<u64>,
    pub received_capacity: Vec<u64>,
    pub received_index: Vec<usize>,

    pub base: SteadyControllerBuilder, //keep this instead of moving the 3 values

}

impl SteadyMonitor {

    pub async fn relay_stats_all(self: &mut Self) {
        let tx = self.base.telemetry_tx.clone();
        let vec = self.sent_count.clone();
        tx.tx(self, Telemetry::MessagesSent(vec)).await;
        self.sent_count.fill(0);

        let tx = self.base.telemetry_tx.clone();
        let vec = self.received_count.clone();
        tx.tx(self, Telemetry::MessagesReceived(vec)).await;
        self.received_count.fill(0);

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

    pub async fn relay_stats_tx_custom(self: & mut Self, tx: SteadyTx<Telemetry>, threshold: u64) {
        if self.sent_count[tx.id] >= threshold {
            self.relay_stats_all().await;
        }
    }

    pub async fn relay_stats_rx_custom(self: & mut Self, rx: SteadyRx<Telemetry>, threshold: u64) {
        if self.received_count[rx.id] >= threshold {
            self.relay_stats_all().await;
        }
    }



}

impl SteadyControllerBuilder {

    pub(crate) fn callbacks(&self) -> Callbacks {
        //  Callbacks::new().
        todo!()
    }

    pub fn configure_for_graph<F,I>(self: &mut Self, root_name: &str, c: Children, init: I ) -> Children
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
                            let monitor = s.wrap(ctx);
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
                        let monitor = s.wrap(ctx);
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

    pub fn wrap(self, ctx: BastionContext) -> SteadyMonitor {
        let tx = self.telemetry_tx.clone();
        let mut monitor = SteadyMonitor {
            last_instant: Instant::now(),
            ctx,
            sent_count:    Vec::new(),
            sent_capacity: Vec::new(),
            sent_index:    Vec::new(),
            received_count:    Vec::new(),
            received_capacity: Vec::new(),
            received_index:    Vec::new(),
            base: self, //onwership transfered
        };

        let name = monitor.ctx.current().name().to_string();
        run!(
                async {
                    tx.tx(&mut monitor, Telemetry::ActorDef(name)).await;
                }
            );
        monitor
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


    pub fn add_to_graph<F,I>(self: &mut Self, root_name: &str, c: Children, init: I ) -> Children
        where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,
    {
        run!( async {
              self.new_monitor().await.configure_for_graph(root_name, c, init)
            }
        )
    }

    pub(crate) fn init_telemetry(&mut self) {

        //TODO: we need to turn on and off the monitoring of the the telemetry channels

      //  TODO: if we wrap rx_vec in an arc with read write lock we can keep dynamic graph changes.
      //  let all_rx = &self.rx_vec;


        let _ = Bastion::supervisor(|supervisor|
                                        supervisor.with_strategy(SupervisionStrategy::OneForOne)
                                            .children(|children| {
                                                self.add_to_graph("generator"
                                                                   , children.with_redundancy(0)
                                                                   , move |monitor| telemetry()
                                                        )

                                            })).expect("OneForOne supervisor creation error.");

    }



}

async fn telemetry() -> Result<(),()> {
    info!("hello");
    Ok(())
}

////////////////////////////////////////////////////////////////
#[derive(Clone)]
pub struct SteadyTx<T> {
    pub id: usize,
    pub sender: Sender<T>
}

impl<T: Send> SteadyTx<T> {
    fn new(sender: Sender<T>, model: &SteadyGraph) -> Self {
        SteadyTx {id:model.senders_count
                        , sender:sender }
    }

    #[allow(dead_code)]
    pub fn has_room(&self) -> bool {
        !self.sender.is_full()
    }

    pub async fn tx(&self, monitor: &mut SteadyMonitor, a: T) {

        info!("full channel from actor: {:?} path:{:?} capacity:{:?} type:{} Endpoints: S{}->R{} "
                , monitor.ctx.signature().to_owned()
                , monitor.ctx.current().path()
                , self.sender.capacity()
                , type_name::<T>()
                , self.sender.sender_count(), self.sender.receiver_count()
        );

        //we try to send but if the channel is full we log that fact
        match self.sender.try_send(a) {
            Ok(_) => {
                self.update_count(monitor).await;
            },
            Err(flume::TrySendError::Full(a)) => {

                error!("full channel detected data_approval tx  actor: {} capacity:{:?} type:{} "
                , monitor.ctx.current().path()
                , self.sender.capacity()
                , type_name::<T>());
                //here we will await until there is room in the channel
                match self.sender.send_async(a).await {
                    Ok(_) => {
                        self.update_count(monitor).await;
                    },
                    Err(e) => {
                        error!("Unexpected error sending: {} {}",e, monitor.ctx.current().path());
                    }
                };
            },
            Err(e) => {
                error!("Unexpected error try sending: {} {}",e, monitor.ctx.current().path());
            }
        };



    }

    async fn update_count(&self, monitor: &mut SteadyMonitor) {
//get the index but grow if we must
        let index = monitor.base.shared_tx_array.read().await[self.id];
        let index = if u8::MAX != index {
            index as usize
        } else {
            let idx = monitor.sent_count.len() as u8;
            monitor.base.shared_tx_array.write().await[self.id] = idx;
            monitor.sent_count.push(0);
            monitor.sent_capacity.push(self.sender.capacity().unwrap() as u64);
            monitor.sent_index.push(self.id);
            monitor.base.telemetry_tx.sender.send_async(Telemetry::MessagesIndexSent(monitor.sent_index.clone())).await
                .expect("Unable to send telemetry"); //TODO: fix error handling
            idx as usize
        };

        //////

        monitor.sent_count[index] += 1;
    }
}


////////////////////////////////////////////////////////////////
#[derive(Clone)]
pub struct SteadyRx<T> {
    pub id: usize,
    pub receiver: Receiver<T>,

}
impl<T: Send> SteadyRx<T> {
    pub fn new(receiver: Receiver<T>, model: &SteadyGraph) -> Self {
        SteadyRx { id: model.receivers_count
                           , receiver
        }
    }
    #[allow(dead_code)]
    pub fn has_message(&self) -> bool {
        !self.receiver.is_empty()
    }

   pub async fn rx(&self, monitor: &mut SteadyMonitor) -> Result<T, RecvError> {

        info!("full channel from actor: {:?} path:{:?} capacity:{:?} type:{} Endpoints: S{}->R{} "
                , monitor.ctx.signature().to_owned()
                , monitor.ctx.current().path()
                , self.receiver.capacity()
                , type_name::<T>()
                , self.receiver.sender_count(), self.receiver.receiver_count()
        );

        //if self ident not set then set it.

       let result = self.receiver.recv_async().await;
       if result.is_ok() {
            self.update_count(monitor).await;
       }

       result
   }

   async fn update_count(&self, monitor: &mut SteadyMonitor) {
//get the index but grow if we must
       let index = monitor.base.shared_rx_array.read().await[self.id];
       let index = if u8::MAX != index {
           index as usize
       } else {
           let idx = monitor.received_count.len() as u8;
           monitor.base.shared_rx_array.write().await[self.id] = idx;
           monitor.received_count.push(0);
           monitor.received_capacity.push(self.receiver.capacity().unwrap() as u64);
           monitor.received_index.push(self.id);
           monitor.base.telemetry_tx.sender.send_async(Telemetry::MessagesIndexReceived(monitor.received_index.clone())).await
               .expect("Unable to send telemetry"); //TODO: fix error handling
           idx as usize
       };

       //////

       monitor.received_count[index] += 1;
   }
}



