
use std::any::{type_name};
use std::future::Future;
use std::time::Duration;
use bastion::{Callbacks};
use bastion::children::Children;
use bastion::distributor::Distributor;
use bastion::prelude::{BastionContext};
use flume::{Receiver, Sender, bounded, RecvError};
use futures::future::Fuse;
use futures_timer::Delay;
use log::{error, info};
use futures::{FutureExt, select};
use crate::actor;

pub struct SteadyGraph {
    senders_count: usize,
    receivers_count: usize,
    rx_vec: Vec<SteadyRx<Telemetry>>,
}

//telemetry enum every actor uses to send information about actors name messages send and messages received
#[derive(Clone)]
pub enum Telemetry {
    ActorDef(String),
    MessagesSent(usize, usize, u64),
    MessagesReceived(usize, usize, u64),
}



macro_rules! process_select_loop {
    ($monitor:expr, $($args:expr),*) => {
        loop {
            select! {
                _ = $monitor.relay_stats().await => {},
                _ = process(&mut $monitor, $($args),*).fuse() => {},
            }
        }
    };
}


#[derive(Clone)]
pub struct SteadyControllerBuilder {
    telemetry_tx: SteadyTx<Telemetry>,
}


pub struct SteadyMonitor {
    telemetry_tx: SteadyTx<Telemetry>,
    pub(crate) ctx: BastionContext,
    pub sent_count: u64,  // TODO: need more complex structure to hold channel specifics etc
    pub recived_count: u64
}

impl SteadyMonitor {



    pub async fn relay_stats<'a>(mut self: & 'a mut Self,) -> Fuse<Delay> {
        info!("hello");
        let tx = self.telemetry_tx.clone();

        tx.tx(self, Telemetry::MessagesSent(0,0, 0)).await;

        Delay::new(Duration::from_secs(3)).fuse()
    }

    // + 'static,
    pub async fn generic_behavior<F, Fut, Args>(
        &mut self,
        process_fn: F,
        args: &Args,
    ) -> Result<(),()>
        where
            F: Fn(&mut SteadyMonitor, &Args) -> Fut,
            Fut: Future<Output = ()> + Send,
    {
        loop {
            select! {
            _ = self.relay_stats().await => {},
            _ = process_fn(self, args).fuse() => {}
          }
        }
        Ok(())
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
                            let init_clone = init_clone.clone();
                            let mut monitor = s.wrap(ctx);
                            init_clone(monitor).await;
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
                        let init_clone = init_clone.clone();
                        let mut monitor = s.wrap(ctx);
                        init_clone(monitor).await;
                        Ok(())
                    }
                })
                .with_name(root_name);
        }
    }

    pub fn wrap(self: &Self, ctx: BastionContext) -> SteadyMonitor {
       let monitor = SteadyMonitor {
           telemetry_tx: self.telemetry_tx.clone(),
           ctx,
           sent_count:0,
           recived_count:0
       };

        monitor
    }
}


impl SteadyGraph {
    //wrap the context with a new context that has a telemetry channel


    pub(crate) fn new() -> SteadyGraph {
        SteadyGraph {
            senders_count: 0,
            receivers_count: 0,
            rx_vec: Vec::new(),
        }
    }

    pub fn new_channel<T: Send>(&mut self, cap: usize) -> (SteadyTx<T>, SteadyRx<T>) {
        let (tx, rx): (Sender<T>, Receiver<T>) = bounded(cap);
        self.senders_count += 1;
        self.receivers_count += 1;
        (SteadyTx::new(tx, self), SteadyRx::new(rx, self))
    }

    pub(crate) fn new_monitor(&mut self) -> SteadyControllerBuilder {
        let (tx, rx) = self.new_channel::<Telemetry>(8);
        self.rx_vec.push(rx);
        SteadyControllerBuilder {
            telemetry_tx: tx
        }
    }

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
    pub fn id(&self) -> usize {
        self.id
    }

    pub async fn tx(&self, monitor: &mut SteadyMonitor, a: T) {

        monitor.sent_count += 1;

        info!("full channel from actor: {:?} path:{:?} capacity:{:?} type:{} Endpoints: S{}->R{} "
                , monitor.ctx.signature().to_owned()
                , monitor.ctx.current().path()
                , self.sender.capacity()
                , type_name::<T>()
                , self.sender.sender_count(), self.sender.receiver_count()
        );

        //tx.id  sent from ctx context, set once only


        //we try to send but if the channel is full we log that fact
        match self.sender.try_send(a) {
            Ok(a) => {
                //ok TODO: bastion notification or summery not on 20ms limit
            },
            Err(flume::TrySendError::Full(a)) => {

                error!("full channel detected data_approval tx  actor: {} capacity:{:?} type:{} "
                , monitor.ctx.current().path()
                , self.sender.capacity()
                , type_name::<T>());
                //here we will await until there is room in the channel
                match self.sender.send_async(a).await {
                    Ok(a) => {
                        //ok  TODO: bastion notification or summery not
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


   pub async fn rx(&self, monitor: &mut SteadyMonitor) -> Result<T, RecvError> {

         monitor.recived_count += 1;

        info!("full channel from actor: {:?} path:{:?} capacity:{:?} type:{} Endpoints: S{}->R{} "
                , monitor.ctx.signature().to_owned()
                , monitor.ctx.current().path()
                , self.receiver.capacity()
                , type_name::<T>()
                , self.receiver.sender_count(), self.receiver.receiver_count()
        );

        //if self ident not set then set it.

        self.receiver.recv_async().await

   }
//   */

   pub fn id(&self) -> usize {
           self.id
   }

}



