use std::any::Any;
use std::error::Error;
use std::future::Future;
use std::pin::{pin, Pin};

use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};
use core::default::Default;
use std::task;

use futures::channel::oneshot;
use futures::channel::oneshot::{Receiver, Sender};
use futures_util::lock::Mutex;
use log::*;
use futures_util::future::{BoxFuture, FusedFuture, select_all};
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use futures_util::task::{LocalSpawn, Spawn};


use crate::{abstract_executor, actor_builder, AlertColor, Graph, GraphLivelinessState, Metric, StdDev, SteadyContext, Trigger};
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::{SideChannel, SideChannelHub};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::metrics_server;

use std::panic::{catch_unwind, AssertUnwindSafe};


#[derive(Clone)]
pub struct ActorBuilder {
    name: &'static str,
    suffix: Option<usize>,
    args: Arc<Box<dyn Any+Send+Sync>>,
    telemetry_tx: Arc<RwLock<Vec<CollectorDetail>>>,
    channel_count: Arc<AtomicUsize>,
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    monitor_count: Arc<AtomicUsize>,

    refresh_rate_in_bits: u8,
    window_bucket_in_bits: u8,
    usage_review: bool,

    percentiles_mcpu: Vec<Percentile>,
    percentiles_work: Vec<Percentile>,
    std_dev_mcpu: Vec<StdDev>,
    std_dev_work: Vec<StdDev>,
    trigger_mcpu: Vec<(Trigger<MCPU>,AlertColor)>,
    trigger_work: Vec<(Trigger<Work>,AlertColor)>,
    avg_mcpu: bool,
    avg_work: bool,
    io_driver: bool,
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    oneshot_startup_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    backplane: Arc<Mutex<Option<SideChannelHub>>>,
}

#[derive(Default)]
pub struct ActorGroup {
    future_builder: Vec<Box<dyn FnMut() -> (BoxFuture<'static, Result<(), Box<dyn Error>>>,bool) + Send >>,
}

impl ActorGroup {
    pub(crate) fn add_actor<F, I>(&mut self, mut actor_startup_receiver: Option<Receiver<()>>, context_archetype: SteadyContextArchetype<I>)
        where F: 'static + Future<Output=Result<(), Box<dyn Error>>> + Send
            , I: 'static + Fn(SteadyContext) -> F + Send + Sync {

        self.future_builder.push({
            let context_archetype = context_archetype.clone();
            Box::new(move || {
                let boxed_future: BoxFuture<'static, Result<(), Box<dyn Error>>>  = Box::pin(build_actor_future( &mut actor_startup_receiver
                                                                                                                , context_archetype.clone()));
                (boxed_future, context_archetype.drive_io)
            }) as Box<dyn FnMut() -> (BoxFuture<'static, Result<(), Box<dyn Error>>>,bool) + Send>
        });

    }


    pub fn spawn(mut self) {

        let super_task = {
            async move {
                let mut actor_future_vec = Vec::with_capacity(self.future_builder.len());
                let mut total_drive_io = false;

                for f in &mut self.future_builder {
                    let (future, drive_io) = f();
                    total_drive_io |= drive_io;
                    actor_future_vec.push(future);
                }

                loop {
                    //this result is for panics.
                    //panic will cause all the grouped actors under this thread to be restarted together.
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let (result, index, _remaining_futures) = if total_drive_io {
                            nuclei::drive(select_all(&mut actor_future_vec))
                        } else {
                            nuclei::block_on(select_all(&mut actor_future_vec))
                        };
                        if let Err(e) = result {
                            error!("Actor {:?} error {:?}", index, e); //TODO: need ident for index.
                            //rebuild the single actor which failed
                            actor_future_vec[index] = self.future_builder[index]().0;
                            true
                        } else {
                            //this actor was finished so remove it from the list
                            actor_future_vec.remove(index);
                            !actor_future_vec.is_empty()  // true we keep running
                        }
                    }));
                    match result {
                        Ok(true) => {
                            //keep running the loop
                        },
                        Ok(false) => {
                            //trace!("Actor {:?} finished ", name);
                            break; //exit the loop we are all done
                        },
                        Err(e) => {
                            //Actor Panic
                            error!("Actor panic {:?}",  e);
                            actor_future_vec.clear();
                            //we must rebuild all actors since we do not know what happened to the thread
                            //EVEN those finished will be restarted.
                            for f in &mut self.future_builder {
                                let (future, drive_io) = f();
                                total_drive_io |= drive_io;
                                actor_future_vec.push(future);
                            }
                            //continue running
                        }
                    }

                }
            }
        };
        abstract_executor::spawn_detached(super_task);

    }


}

struct SteadyContextArchetype<I> {

    build_actor_exec: Arc<I>,
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    channel_count: Arc<AtomicUsize>,
    ident: ActorIdentity,
    args: Arc<Box<dyn Any + Send + Sync>>,
    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    actor_metadata: Arc<ActorMetaData>,
    oneshot_shutdown_vec: Arc<Mutex<Vec<Sender<()>>>>,
    oneshot_shutdown: Arc<Mutex<Receiver<()>>>,
    node_tx_rx: Option<Arc<Mutex<SideChannel>>>,
    instance_id: Arc<AtomicU32>,
    drive_io: bool,
}

impl<I> Clone for SteadyContextArchetype<I>
{
    fn clone(&self) -> Self {
        SteadyContextArchetype {
            build_actor_exec: self.build_actor_exec.clone(),
            runtime_state: self.runtime_state.clone(),
            channel_count: self.channel_count.clone(),
            ident: self.ident.clone(),
            args: self.args.clone(),
            all_telemetry_rx: self.all_telemetry_rx.clone(),
            actor_metadata: self.actor_metadata.clone(),
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: self.oneshot_shutdown.clone(),
            node_tx_rx: self.node_tx_rx.clone(),
            instance_id: self.instance_id.clone(),
            drive_io: self.drive_io
        }
    }
}

impl ActorBuilder {

    /// Creates a new `ActorBuilder` instance, initializing it with defaults and configurations derived from the given `Graph`.
    ///
    /// # Arguments
    ///
    /// * `graph` - A mutable reference to the `Graph` from which to inherit settings.
    ///
    /// # Returns
    ///
    /// A new instance of `ActorBuilder`.
    pub fn new( graph: &mut Graph) -> ActorBuilder {


        ActorBuilder {
            backplane: graph.backplane.clone(),
            name: "",
            suffix: None,
            monitor_count: graph.monitor_count.clone(),
            args : graph.args.clone(),
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),
            refresh_rate_in_bits: 6, // 1<<6 == 64
            window_bucket_in_bits: 5, // 1<<5 == 32
            oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
            oneshot_startup_vec: graph.oneshot_startup_vec.clone(),
            percentiles_mcpu: Vec::with_capacity(0),
            percentiles_work: Vec::with_capacity(0),
            std_dev_mcpu: Vec::with_capacity(0),
            std_dev_work: Vec::with_capacity(0),
            trigger_mcpu: Vec::with_capacity(0),
            trigger_work: Vec::with_capacity(0),
            avg_mcpu: false,
            avg_work: false,
            io_driver: false,
            usage_review: false,

        }
    }

    /// Sets the compute refresh window floor and bucket size for telemetry, adjusting the resolution of performance metrics.
    ///
    /// # Arguments
    ///
    /// * `refresh` - The minimum refresh rate as a `Duration`.
    /// * `window` - The size of the window as a `Duration`.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified compute refresh window configuration.
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let result = self.clone();
        //we must compute the refresh rate first before we do the window
        let frames_per_refresh = refresh.as_micros() / (1000u128 * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128);
        let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
        let refresh_in_micros = (1000u128<<refresh_in_bits) * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        //now compute the window based on our new bucket size
        let buckets_per_window:f32 = window.as_micros() as f32 / refresh_in_micros as f32;
        //find the next largest power of 2
        let window_in_bits = buckets_per_window.log2().ceil() as u8;
        result.with_compute_refresh_window_bucket_bits(refresh_in_bits, window_in_bits)
    }

    /// Directly sets the compute refresh window bucket sizes in bits, providing fine-grained control over telemetry resolution.
    ///
    /// # Arguments
    ///
    /// * `refresh_bucket_in_bits` - The refresh rate in bits.
    /// * `window_bucket_in_bits` - The window size in bits.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified bucket sizes.
    pub fn with_compute_refresh_window_bucket_bits(&self
                                                   , refresh_bucket_in_bits:u8
                                                   , window_bucket_in_bits: u8) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = refresh_bucket_in_bits;
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    /// Assigns a name to the actor, used for identification and possibly logging.
    ///
    /// # Arguments
    ///
    /// * `name` - A static string slice representing the actor's name.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified name.
    pub fn with_name(&self, name: &'static str) -> Self {
        let mut result = self.clone();
        result.name = name;
        result
    }

    /// Appends a suffix to the actor's name, aiding in the differentiation of actor instances.
    ///
    /// # Arguments
    ///
    /// * `suffix` - A unique identifier to be appended to the actor's name.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the name suffix applied.
    pub fn with_name_suffix(&self, suffix: usize) -> Self {
        let mut result = self.clone();
        result.suffix = Some(suffix);
        result
    }

    pub fn with_name_and_suffix(&self, name: &'static str, suffix: usize) -> Self {
        let mut result = self.clone();
        result.name = name;
        result.suffix = Some(suffix);
        result
    }

    /// Configures the actor to monitor a specific CPU usage percentile for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `config` - The `Percentile` to monitor for CPU usage.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified CPU usage percentile configuration.
    pub fn with_mcpu_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_mcpu.push(config);
        result
    }

    /// Configures the actor to monitor a specific workload percentile, aiding in workload analysis and optimization.
    ///
    /// # Arguments
    ///
    /// * `config` - The `Percentile` to monitor for workload performance.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified workload percentile configuration.
    pub fn with_work_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_work.push(config);
        result
    }

    /// Enables average CPU usage monitoring for the actor, smoothing out short-term fluctuations in usage metrics.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average CPU monitoring enabled.
    pub fn with_avg_mcpu(&self) -> Self {
        let mut result = self.clone();
        result.avg_mcpu=true;
        result
    }

    /// Enables average workload monitoring, providing a more consistent view of the actor's workload over time.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average workload monitoring enabled.
    pub fn with_avg_work(&self) -> Self {
        let mut result = self.clone();
        result.avg_work = true;
        result
    }

    pub fn with_io_driver(&self) -> Self {
        let mut result = self.clone();
        result.io_driver = true;
        result
    }

    /// Sets a CPU usage threshold that, when exceeded, triggers an alert, helping maintain system performance and stability.
    ///
    /// # Arguments
    ///
    /// * `bound` - The trigger condition based on CPU usage.
    /// * `color` - The `AlertColor` to be used when the condition is met.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified CPU trigger condition.
    pub fn with_mcpu_trigger(&self, bound: Trigger<MCPU>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_mcpu.push((bound,color));
        result
    }

    /// Sets a workload threshold that, when exceeded, triggers an alert, assisting in proactive system monitoring.
    ///
    /// # Arguments
    ///
    /// * `bound` - The trigger condition based on workload.
    /// * `color` - The `AlertColor` to be used when the condition is met.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified workload trigger condition.
    pub fn with_work_trigger(&self, bound: Trigger<Work>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_work.push((bound,color));
        result
    }


    /// Sets the floor for compute window size, ensuring performance metrics are based on a sufficiently large sample.
    ///
    /// # Arguments
    ///
    /// * `duration` - The minimum duration for the compute window.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified compute window floor.
    pub fn with_compute_window_floor(&self, duration: Duration) -> Self {
        let result = self.clone();
        let millis: u128 = duration.as_micros();
        let frame_ms: u128 = 1000u128 * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        let est_buckets = millis / frame_ms;
        //find the next largest power of 2
        let window_bucket_in_bits = (est_buckets as f32).log2().ceil() as u8;
        result.with_compute_window_bucket_bits(window_bucket_in_bits)
    }

    /// Directly sets the compute window size in bits, offering precise control over the performance metrics' temporal resolution.
    ///
    /// # Arguments
    ///
    /// * `window_bucket_in_bits` - The size of the window in bits.
    ///
    /// # Note
    ///
    /// The default window size is 1 second.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified window size.
    pub fn with_compute_window_bucket_bits(&self, window_bucket_in_bits: u8) -> Self {
        assert!(window_bucket_in_bits <= 30);
        let mut result = self.clone();
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    /// Completes the actor configuration and initiates its execution with the provided logic.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `exec` - The execution logic for the actor.
    pub fn build_spawn<F,I>(self, build_actor_exec: I)
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send ,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {
        let mut actor_startup_receiver  = Some(Self::build_startup_oneshot(self.oneshot_startup_vec.clone()));
        let context_archetype = self.single_actor_exec_archetype(build_actor_exec);

        abstract_executor::spawn_detached(
            Box::pin( async move {
                loop {
                    let future = build_actor_future(  &mut actor_startup_receiver
                                                      , context_archetype.clone());

                    let result = catch_unwind(AssertUnwindSafe(|| {
                        if context_archetype.drive_io {
                            nuclei::drive(future)
                        } else {
                            nuclei::block_on(future)
                        }
                    }));

                    match result {
                        Ok(_) => {
                            //trace!("Actor {:?} finished ", name);
                            break; //exit the loop we are all done
                        },
                        Err(e) => {
                            //Panic or an actor Error, log and continue in the loop
                            error!("{:?} {:?}", context_archetype.ident, e);
                        }
                    }
                }
            })
        );
    }


    pub fn build_join<F,I>(self, build_actor_exec: I, target: &mut ActorGroup)
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send ,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {
        let actor_startup_receiver  = Some(Self::build_startup_oneshot(self.oneshot_startup_vec.clone()));
        target.add_actor(actor_startup_receiver, self.single_actor_exec_archetype(build_actor_exec));
    }



    fn single_actor_exec_archetype<F,I>(self, build_actor_exec: I) -> SteadyContextArchetype<I>
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send ,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {

        let name                     = self.name;
        let telemetry_tx             = self.telemetry_tx.clone();
        let channel_count            = self.channel_count.clone();
        let runtime_state            = self.runtime_state.clone();
        let args                     = self.args.clone();
        let oneshot_shutdown_vec     = self.oneshot_shutdown_vec.clone();
        let backplane                = self.backplane.clone();
        let drive_io                 = self.io_driver;

        let id                       = self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let immutable_identity       = ActorIdentity { id, name };
        //info!(" Actor {:?} defined ", immutable_identity);
        let immutable_actor_metadata = self.build_actor_metadata(id, name).clone();


        /////////////////////////////////////////////
        // this is used only when run under testing
        ////////////////////////////////////////////
        let immutable_node_tx_rx = abstract_executor::block_on( async move {
            let mut backplane = backplane.lock().await;
            //If the backplane is enabled then register every node name for use
            if let Some(pb) = &mut *backplane {
                let max_channel_capacity = 8;
                //TODO: name may not be good enough here and may need prefix as well.
                pb.register_node(name, max_channel_capacity);
                pb.node_tx_rx(name)
            } else {
                None
            }
        });
        /////////////////////////////////////////////

        ////////////////////////////////////////////
        // before starting the actor setup our shutdown oneshot
        ////////////////////////////////////////////
        //this single one shot is kept no matter how many times actor is restarted
        let immutable_oneshot_shutdown = {
            let (send_shutdown_notice_to_periodic_wait, oneshot_shutdown) = oneshot::channel();
            let oneshot_shutdown_vec = oneshot_shutdown_vec.clone();
            abstract_executor::block_on( async move {
                let mut v = oneshot_shutdown_vec.lock().await;
                v.push(send_shutdown_notice_to_periodic_wait);
            });
            Arc::new(Mutex::new(oneshot_shutdown))
        };
        ////////////////////////////////////////////
        let restart_counter = Arc::new(AtomicU32::new(0));

        SteadyContextArchetype {
            runtime_state: runtime_state.clone(),
            channel_count: channel_count.clone(),
            ident: immutable_identity.clone(),
            args: args.clone(),
            all_telemetry_rx: telemetry_tx.clone(),
            actor_metadata: immutable_actor_metadata.clone(),
            oneshot_shutdown_vec: oneshot_shutdown_vec.clone(),
            oneshot_shutdown: immutable_oneshot_shutdown.clone(),
            node_tx_rx: immutable_node_tx_rx.clone(),
            build_actor_exec: Arc::new(build_actor_exec),
            instance_id: restart_counter,
            drive_io: drive_io
        }

    }




    fn build_startup_oneshot(oneshot_startup_vec: Arc<Mutex<Vec<Sender<()>>>>) -> Receiver<()> {
        let (actor_startup_sender, actor_startup_receiver) = oneshot::channel();
        //we do not normally expect this to not get the lock
        if let Some(mut v) = oneshot_startup_vec.clone().try_lock() { //fast path
            v.push(actor_startup_sender);
        } else {
            abstract_executor::block_on(async move {
                oneshot_startup_vec.clone().lock().await.push(actor_startup_sender);
            });
        }
        actor_startup_receiver
    }


    fn build_actor_metadata(&self, id: usize, name: &'static str) -> Arc<ActorMetaData> {
        Arc::new(
            ActorMetaData {
                id,
                name,
                avg_mcpu: self.avg_mcpu.clone(),
                avg_work: self.avg_work.clone(),
                percentiles_mcpu: self.percentiles_mcpu.clone(),
                percentiles_work: self.percentiles_work.clone(),
                std_dev_mcpu: self.std_dev_mcpu.clone(),
                std_dev_work: self.std_dev_work.clone(),
                trigger_mcpu: self.trigger_mcpu.clone(),
                trigger_work: self.trigger_work.clone(),
                usage_review: self.usage_review.clone(),
                refresh_rate_in_bits: self.refresh_rate_in_bits.clone(),
                window_bucket_in_bits: self.window_bucket_in_bits.clone(),
            }
        )
    }
}


pub(crate) fn build_actor_future<'a, F, I>(
    actor_startup_receiver: &mut Option<Receiver<()>>
    , builder_source: SteadyContextArchetype<I>) -> F
    where I: Fn(SteadyContext) -> F + 'static + Sync + Send,
          F: Future<Output=Result<(), Box<dyn Error>>> + Send + 'static {

    if let Some(actor_startup_receiver_internal) = actor_startup_receiver.take() {
        //no start signal yet so we block until we get it
        let _ = abstract_executor::block_on(actor_startup_receiver_internal);
    }
    (builder_source.build_actor_exec)(SteadyContext {
        runtime_state: builder_source.runtime_state,
        channel_count: builder_source.channel_count,
        ident: builder_source.ident,
        args: builder_source.args,
        all_telemetry_rx: builder_source.all_telemetry_rx,
        actor_metadata: builder_source.actor_metadata,
        oneshot_shutdown_vec: builder_source.oneshot_shutdown_vec,
        oneshot_shutdown: builder_source.oneshot_shutdown,
        node_tx_rx: builder_source.node_tx_rx,
        instance_id: builder_source.instance_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        last_periodic_wait: Default::default(),
        is_in_graph: true,
        actor_start_time: Instant::now()
    })

}

impl Metric for Work {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    work: u16, // out of 10000 where 10000 is 100%
}

impl Work {
    pub fn new(value: f32) -> Option<Self> {
        if (0.0..=100.00).contains(&value) {
            Some(Work{work: (value * 100.0) as u16}) //10_000 is 100%
        } else {
            None
        }
    }

    pub fn rational(&self) -> (u64,u64) {
        (self.work as u64, 10_000)
    }

    pub fn p10() -> Self { Work{ work: 1000 }}
    pub fn p20() -> Self { Work{ work: 2000 }}
    pub fn p30() -> Self { Work{ work: 3000 }}
    pub fn p40() -> Self { Work{ work: 4000 }}
    pub fn p50() -> Self { Work{ work: 5000 }}
    pub fn p60() -> Self { Work{ work: 6000 }}
    pub fn p70() -> Self { Work{ work: 7000 }}
    pub fn p80() -> Self { Work{ work: 8000 }}
    pub fn p90() -> Self { Work{ work: 9000 }}
    pub fn p100() -> Self { Work{ work: 10000 }}

}

impl Metric for MCPU {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MCPU {
    mcpu: u16, // max 1024
}

impl MCPU {
    pub fn new(value: u16) -> Option<Self> {
        if value<=1024 {
            Some(Self {
                mcpu: value,
            })
        } else {
            None
        }
    }

    pub fn rational(&self) -> (u64,u64) {
        (self.mcpu as u64,1024)
    }

    pub fn m16() -> Self { MCPU{mcpu:16}}
    pub fn m64() -> Self { MCPU{mcpu:64}}
    pub fn m256() -> Self { MCPU{mcpu:256}}
    pub fn m512() -> Self { MCPU{mcpu:512}}
    pub fn m768() -> Self { MCPU{mcpu:768}}
    pub fn m1024() -> Self { MCPU{mcpu:1024}}

}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentile(f64);

impl Percentile {
    // Private constructor to directly set the value inside the struct.
    // Ensures that all public constructors go through validation.
    fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    // Convenience methods for common percentiles
    pub fn p25() -> Self {
        Self(25.0)
    }

    pub fn p50() -> Self {Self(50.0) }

    pub fn p75() -> Self {
        Self(75.0)
    }

    pub fn p90() -> Self {
        Self(90.0)
    }

    pub fn p80() -> Self {
        Self(80.0)
    }

    pub fn p96() -> Self {
        Self(96.0)
    }

    pub fn p99() -> Self {
        Self(99.0)
    }

    // Allows custom values within the valid range.
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    // Getter to access the inner f32 value.
    pub fn percentile(&self) -> f64 {
        self.0
    }
}

//Note: redundancy and auto-scale features of bastion are not supported for the following reasons:
// 1. channel usage requires lock and parallel actors would not work well and cause deadlocks
// 2. we want clear telemetry on every actor instance, never shared
// 3. there was no way to support this and the code generator without marking the methods unsafe.
// 4. we might use this as an internal feature as it is difficult to get right.


