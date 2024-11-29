//! The `actor_builder` module provides structures and functions to create, configure, and manage actors within a system.
//! This module includes the `ActorBuilder` for building actors, `ActorTeam` for managing groups of actors, and various utility
//! functions and types to support actor creation and telemetry monitoring.

use std::any::Any;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use core::default::Default;
use std::collections::VecDeque;

use futures::channel::oneshot;
use futures::channel::oneshot::{Receiver, Sender};
use futures_util::lock::Mutex;
use log::*;
use futures_util::future::{BoxFuture, select_all};

use crate::{abstract_executor, ActorName, AlertColor, Graph, Metric, StdDev, steady_config, SteadyContext, Trigger};
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::{SideChannel, SideChannelHub};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;

use std::panic::{catch_unwind, AssertUnwindSafe};

#[allow(unused_imports)]
#[cfg(feature = "core_affinity")]
use libc::pthread_setaffinity_np;


/// The `ActorBuilder` struct is responsible for building and configuring actors.
/// It contains various settings related to telemetry, triggers, and actor identification.
#[derive(Clone)]
pub struct ActorBuilder {
    actor_name: ActorName,
    args: Arc<Box<dyn Any + Send + Sync>>,
    telemetry_tx: Arc<RwLock<Vec<CollectorDetail>>>,
    channel_count: Arc<AtomicUsize>,
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    actor_count: Arc<AtomicUsize>,
    thread_lock: Arc<Mutex<()>>,

    refresh_rate_in_bits: u8,
    window_bucket_in_bits: u8,
    usage_review: bool,

    percentiles_mcpu: Vec<Percentile>,
    percentiles_load: Vec<Percentile>,
    std_dev_mcpu: Vec<StdDev>,
    std_dev_load: Vec<StdDev>,
    trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,
    trigger_load: Vec<(Trigger<Work>, AlertColor)>,
    show_thread_info: bool,
    avg_mcpu: bool,
    avg_load: bool,
    frame_rate_ms: u64,
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    backplane: Arc<Mutex<Option<SideChannelHub>>>,
    team_count: Arc<AtomicUsize>,
}

type FutureBuilderType = Box<dyn FnMut(usize) -> (BoxFuture<'static, Result<(), Box<dyn Error>>>, bool) + Send>;

#[cfg(feature = "core_affinity")]
fn get_num_cores() -> usize {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as usize }
}

#[cfg(feature = "core_affinity")]
fn pin_thread_to_core(core_id: usize) -> Result<(), String> {
    let num_cores = get_num_cores(); // Get the number of available cores
    //println!("Number of cores: {:?} {:?}", num_cores, core_id);
    let core_id = core_id % num_cores; // Adjust core_id to ensure it's within bounds

    let mut cpu_set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::CPU_ZERO(&mut cpu_set);
        libc::CPU_SET(core_id, &mut cpu_set);

        let thread_id = libc::pthread_self();
        //println!("Thread id: {:?}", thread_id);

        // Set the thread affinity
        let result = libc::pthread_setaffinity_np(
            thread_id,
            std::mem::size_of::<libc::cpu_set_t>(),
            &cpu_set,
        );

        if result != 0 {
            return Err(format!("Failed to set thread affinity: {}", result));
        }
    }
    Ok(())
}

/// The `ActorTeam` struct manages a collection of actors, facilitating their coordinated execution.
pub struct ActorTeam {
    future_builder: VecDeque<FutureBuilderType>,
    thread_lock: Arc<Mutex<()>>,
    team_id: usize,
}

impl ActorTeam {
    /// Creates a new instance of `ActorTeam`.
    pub fn new(graph: &Graph) -> Self { //TODO: add this method to graph.
        ActorTeam {
            future_builder: VecDeque::new(),
            thread_lock: graph.thread_lock.clone(),
            team_id: graph.team_count.fetch_add(1, Ordering::SeqCst),
        }
    }

    /// Adds an actor to the team with the specified context and frame rate.
    fn add_actor<F, I>(&mut self, context_archetype: SteadyContextArchetype<I>, frame_rate_ms: u64)
        where
            F: 'static + Future<Output = Result<(), Box<dyn Error>>> + Send,
            I: 'static + Fn(SteadyContext) -> F + Send + Sync,
    {
        // trace!("add new actor to team {:?}", context_archetype.ident);


        self.future_builder.push_back({
            let context_archetype = context_archetype.clone();
            Box::new(move |team_display_id| {
                let boxed_future: BoxFuture<'static, Result<(), Box<dyn Error>>> =
                    Box::pin(build_actor_future(context_archetype.clone(), frame_rate_ms, team_display_id));
                (boxed_future, false)
            }) as Box<dyn FnMut(usize) -> (BoxFuture<'static, Result<(), Box<dyn Error>>>, bool) + Send>
        });
    }

    /// Transfers the front actor to another `ActorTeam`.
    pub fn transfer_front_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_front() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Transfers the back actor to another `ActorTeam`.
    pub fn transfer_back_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_back() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// take actor team and start the thread and consume the struct
    pub fn spawn(mut self) -> usize {
        let count = Arc::new(AtomicUsize::new(0));
        if self.future_builder.is_empty() {
            return 0; // Nothing to spawn, so return
        }

        let (local_send, local_take) = oneshot::channel();

        let super_task = {
            let count = count.clone();
            async move {
                //println!("spawn ActorTeam {:?}", self.team_id); //shoudl not all be zero.
                #[cfg(feature = "core_affinity")]
                let _= pin_thread_to_core(self.team_id);

                count.store(self.future_builder.len(),Ordering::SeqCst);
                let _ = local_send.send(()); //may now return we have count and started

                let mut actor_future_vec = Vec::with_capacity(self.future_builder.len());

                for f in &mut self.future_builder {
                    let (future, _drive_io) = f(self.team_id);
                    actor_future_vec.push(future);
                }

                loop {
                    // trace!("hello team loop");
                    // This result is for panics.
                    // Panic will cause all the grouped actors under this thread to be restarted together.
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let (result, index, _remaining_futures) = launch_actor(select_all(&mut actor_future_vec));

                        if let Err(e) = result {
                            error!("Actor {:?} error {:?}", index, e); // TODO: need ident for index.
                            // Rebuild the single actor which failed
                            actor_future_vec[index] = self.future_builder[index](self.team_id).0;
                            true
                        } else {
                            trace!("working on group stop");
                            // This actor was finished, so remove it from the list
                            drop(actor_future_vec.remove(index));
                            let result = !actor_future_vec.is_empty(); // true we keep running
                            trace!("Actor {:?} finished, result {:?} count of remaining {:?} ", index, result, actor_future_vec.len());
                            result
                        }
                    }));
                    match result {
                        Ok(true) => {
                            // Keep running the loop
                        }
                        Ok(false) => {
                            // info!("finished exit ");
                            break; // Exit the loop, we are all done
                        }
                        Err(e) => {
                            // Actor Panic
                            error!("Actor panic {:?}", e);
                            actor_future_vec.clear();
                            // We must rebuild all actors since we do not know what happened to the thread
                            // EVEN those finished will be restarted.
                            for f in &mut self.future_builder {
                                let (future, _drive_io) = f(self.team_id);
                                actor_future_vec.push(future);
                            }
                            // Continue running
                        }
                    }
                }
            }
        };
        abstract_executor::spawn_detached(self.thread_lock, super_task);
        //only continue after startup has finished
        let _ = nuclei::block_on(local_take);
        count.load(Ordering::SeqCst)
    }


}

/// WARNING: do not rename this function without change of backtrace printing since we use this as a "stop" to shorten traces.
pub fn launch_actor<F: Future<Output = T>, T>(future: F) -> T {
    nuclei::block_on(future)
}

pub(crate) type NodeTxRx = Mutex<(SideChannel,Receiver<()>)>;
/// The `SteadyContextArchetype` struct serves as a template for building actor contexts,
/// encapsulating all the necessary parameters and state.
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
    node_tx_rx: Option<Arc<NodeTxRx>>,
    instance_id: Arc<AtomicU32>,
    show_thread_info: bool
}

impl<I> Clone for SteadyContextArchetype<I> {
    fn clone(&self) -> Self {
        SteadyContextArchetype {
            build_actor_exec: self.build_actor_exec.clone(),
            runtime_state: self.runtime_state.clone(),
            channel_count: self.channel_count.clone(),
            ident: self.ident,
            args: self.args.clone(),
            all_telemetry_rx: self.all_telemetry_rx.clone(),
            actor_metadata: self.actor_metadata.clone(),
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: self.oneshot_shutdown.clone(),
            node_tx_rx: self.node_tx_rx.clone(),
            instance_id: self.instance_id.clone(),
            show_thread_info: self.show_thread_info,
        }
    }
}

/// Enum to select the treading approach for building an Actor. Each actor can spawn a new thread
/// or it can take a Join wrapping an ActorTeam to run the Actors together.
///
pub enum Threading<'a> {
    /// Spawn a new thread for the actor now
    Spawn,
    /// Join the actor to the team where a common spawn will be called later
    Join(&'a mut ActorTeam),
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
    pub fn new(graph: &mut Graph) -> ActorBuilder {

        //build default window
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(graph.telemetry_production_rate_ms as u128
                                                                                              , Duration::from_secs(1)
                                                                                              , Duration::from_secs(10));
        ActorBuilder {
            actor_name: ActorName::new("",None),
            backplane: graph.backplane.clone(),
            thread_lock: graph.thread_lock.clone(),
            actor_count: graph.actor_count.clone(),
            args: graph.args.clone(),
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),
            refresh_rate_in_bits: refresh_in_bits, 
            window_bucket_in_bits: window_in_bits, 
            oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
            percentiles_mcpu: Vec::with_capacity(0),
            percentiles_load: Vec::with_capacity(0),
            std_dev_mcpu: Vec::with_capacity(0),
            std_dev_load: Vec::with_capacity(0),
            trigger_mcpu: Vec::with_capacity(0),
            trigger_load: Vec::with_capacity(0),
            team_count: graph.team_count.clone(),
            show_thread_info: false,
            avg_mcpu: false,
            avg_load: false,
            frame_rate_ms: graph.telemetry_production_rate_ms,
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
        let mut result = self.clone();
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(self.frame_rate_ms as u128, refresh, window);
        result.refresh_rate_in_bits = refresh_in_bits;
        result.window_bucket_in_bits = window_in_bits;
        result
    }

    /// Disables any metric collection
    /// 
    pub fn with_no_refresh_window(&self) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = 0;
        result.window_bucket_in_bits = 0;
        result
    }

    pub(crate) fn internal_compute_refresh_window(frame_rate_ms: u128, refresh: Duration, window: Duration) -> (u8, u8) {
        if frame_rate_ms>0 {
            // We must compute the refresh rate first before we do the window
            let frames_per_refresh = refresh.as_micros() / (1000u128 * frame_rate_ms);
            let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
            let refresh_in_micros = (1000u128 << refresh_in_bits) * frame_rate_ms;
            // Now compute the window based on our new bucket size
            let buckets_per_window: f32 = window.as_micros() as f32 / refresh_in_micros as f32;
            // Find the next largest power of 2
            let window_in_bits = buckets_per_window.log2().ceil() as u8;
            (refresh_in_bits, window_in_bits) 
        } else {
            (0,0)
        }
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

    /// Name the actor for telemetry with an instance suffex for more clarity.
    /// 
    pub fn with_name_and_suffix(&self, name: &'static str, suffex: usize) -> Self {
        let mut result = self.clone();
        result.actor_name = ActorName::new(name,Some(suffex));
        result
    }

    /// Name the actor for use in telemetry
    /// 
    pub fn with_name(&self, name: &'static str) -> Self {
        let mut result = self.clone();
        result.actor_name = ActorName::new(name,None);
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
    pub fn with_load_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_load.push(config);
        result
    }

    /// Enables average CPU usage monitoring for the actor, smoothing out short-term fluctuations in usage metrics.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average CPU monitoring enabled.
    pub fn with_mcpu_avg(&self) -> Self {
        let mut result = self.clone();
        result.avg_mcpu = true;
        result
    }

    /// Enables average workload monitoring, providing a more consistent view of the actor's workload over time.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average workload monitoring enabled.
    pub fn with_load_avg(&self) -> Self {
        let mut result = self.clone();
        result.avg_load = true;
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
        result.trigger_mcpu.push((bound, color));
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
    pub fn with_load_trigger(&self, bound: Trigger<Work>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_load.push((bound, color));
        result
    }
    
    /// Show the thread on the telemetry
    pub fn with_thread_info(&self) -> Self {
        let mut result = self.clone();
        result.show_thread_info = true;
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
    pub fn build_spawn<F, I>(self, build_actor_exec: I)
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {
        let team_id = self.team_count.clone().fetch_add(1, Ordering::SeqCst);
        let thread_lock = self.thread_lock.clone();
        let rate_ms = self.frame_rate_ms;
        let context_archetype = self.single_actor_exec_archetype(build_actor_exec);
               
        abstract_executor::spawn_detached(thread_lock, Box::pin(async move {
            #[cfg(feature = "core_affinity")]
            let _= pin_thread_to_core(team_id);

            loop {
                let future = build_actor_future(context_archetype.clone(), rate_ms, team_id);
                let result = catch_unwind(AssertUnwindSafe(|| launch_actor(future)));
                match result {
                    Ok(_) => {
                        // trace!("Actor {:?} finished ", name);
                        break; // Exit the loop we are all done
                    }
                    Err(e) => {
                        if let Some(specific_error) = e.downcast_ref::<std::io::Error>() {
                            warn!("IO Error encountered: {}", specific_error);
                        } else if let Some(specific_error) = e.downcast_ref::<String>() {
                            warn!("String Error encountered: {}", specific_error);
                        }
                        // Panic or an actor Error, log and continue in the loop
                        warn!("Restarting: {:?} ", context_archetype.ident);
                    }
                }
            }
        }));
    }

    /// Adds an actor to the specified `ActorTeam`, enabling group execution.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    /// * `target` - The `ActorTeam` to which the actor will be added.
    pub fn build_join<F, I>(self, build_actor_exec: I, target: &mut ActorTeam)
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send,
            F: Future<Output = Result<(), Box<dyn Error>>> + 'static + Send,
    {
        let rate = self.frame_rate_ms;
        target.add_actor(self.single_actor_exec_archetype(build_actor_exec), rate);
    }

    /// Builds actor but can either spawn or team the threading based on enum
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    /// * `threading` - The `Threading` to use for the actor.
    pub fn build<F, I>(self, build_actor_exec: I, threading: &mut Threading)
    where
        I: Fn(SteadyContext) -> F + 'static + Sync + Send,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static + Send,
    {
        match threading {
            Threading::Spawn =>      {self.build_spawn(build_actor_exec);}
            Threading::Join(team) => {self.build_join(build_actor_exec, team);}
        }
    }

    /// Creates a `SteadyContextArchetype` for actor execution, encapsulating the necessary parameters and state.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    ///
    /// # Returns
    ///
    /// A `SteadyContextArchetype` instance configured with the actor's execution logic.
    fn single_actor_exec_archetype<F, I>(self, build_actor_exec: I) -> SteadyContextArchetype<I>
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {

        let telemetry_tx = self.telemetry_tx.clone();
        let channel_count = self.channel_count.clone();
        let runtime_state = self.runtime_state.clone();
        let args = self.args.clone();
        let oneshot_shutdown_vec = self.oneshot_shutdown_vec.clone();
        let backplane = self.backplane.clone();

        let id = self.actor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let immutable_identity = ActorIdentity::new(id, self.actor_name.name, self.actor_name.suffix);
        if steady_config::SHOW_ACTORS {
            info!(" Actor {:?} defined ", immutable_identity);
        }
        
        let immutable_actor_metadata = self.build_actor_metadata(immutable_identity).clone();

        /////////////////////////////////////////////
        // This is used only when run under testing
        ////////////////////////////////////////////
        let oneshot_shutdown_vec_for_node = oneshot_shutdown_vec.clone();
        let immutable_node_tx_rx = abstract_executor::block_on(async move {
            let mut backplane = backplane.lock().await;
            // If the backplane is enabled then register every node name for use
            if let Some(pb) = &mut *backplane {
                let (shutdown_tx,shutdown_rx) = oneshot::channel();
                abstract_executor::block_on(async move {
                    let mut v = oneshot_shutdown_vec_for_node.lock().await;
                    v.push(shutdown_tx);
                });

                pb.register_node(immutable_identity.label, steady_config::BACKPLANE_CAPACITY, shutdown_rx);
                pb.node_tx_rx(immutable_identity.label) //returns side channel

            } else {
                None
            }
        });




        ////////////////////////////////////////////
        // Before starting the actor setup our shutdown oneshot
        ////////////////////////////////////////////
        // This single one shot is kept no matter how many times actor is restarted
        let immutable_oneshot_shutdown = {
            let (send_shutdown_notice_to_periodic_wait, oneshot_shutdown) = oneshot::channel();
            let oneshot_shutdown_vec = oneshot_shutdown_vec.clone();
            abstract_executor::block_on(async move {
                let mut v = oneshot_shutdown_vec.lock().await;
                v.push(send_shutdown_notice_to_periodic_wait);
            });
            Arc::new(Mutex::new(oneshot_shutdown))
        };

        let restart_counter = Arc::new(AtomicU32::new(0));

        SteadyContextArchetype {
            runtime_state: runtime_state.clone(),
            channel_count: channel_count.clone(),
            ident: immutable_identity,
            args: args.clone(),
            all_telemetry_rx: telemetry_tx.clone(),
            actor_metadata: immutable_actor_metadata.clone(),
            oneshot_shutdown_vec: oneshot_shutdown_vec.clone(),
            oneshot_shutdown: immutable_oneshot_shutdown.clone(),
            node_tx_rx: immutable_node_tx_rx.clone(),
            build_actor_exec: Arc::new(build_actor_exec),
            instance_id: restart_counter,
            show_thread_info: self.show_thread_info,
        }
    }

    

    /// Constructs actor metadata for the given actor ID and name, encapsulating telemetry and trigger configurations.
    ///
    /// # Arguments
    ///
    /// * `ident` - The unique identifier for the actor.
    ///
    /// # Returns
    ///
    /// An `Arc` containing the actor metadata.
    fn build_actor_metadata(&self, ident: ActorIdentity) -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            ident,
            avg_mcpu: self.avg_mcpu,
            avg_work: self.avg_load,
            percentiles_mcpu: self.percentiles_mcpu.clone(),
            percentiles_work: self.percentiles_load.clone(),
            std_dev_mcpu: self.std_dev_mcpu.clone(),
            std_dev_work: self.std_dev_load.clone(),
            trigger_mcpu: self.trigger_mcpu.clone(),
            trigger_work: self.trigger_load.clone(),
            usage_review: self.usage_review,
            refresh_rate_in_bits: self.refresh_rate_in_bits,
            window_bucket_in_bits: self.window_bucket_in_bits
        })
    }
}

/// Constructs a future for the actor, incorporating startup synchronization and context initialization.
///
/// # Type Parameters
///
/// * `F` - The future returned by the execution logic.
/// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
///
/// # Arguments
///
/// * `actor_startup_receiver` - An optional receiver for the startup synchronization channel.
/// * `builder_source` - The archetype for building the actor context.
/// * `frame_rate_ms` - The frame rate in milliseconds.
///
/// # Returns
///
/// A future representing the actor's execution.
fn build_actor_future<F, I>(
    builder_source: SteadyContextArchetype<I>,
    frame_rate_ms: u64,
    team_id: usize,
) -> F
    where
        I: Fn(SteadyContext) -> F + 'static + Sync + Send,
        F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
{

    {
        let mut liveliness = builder_source.runtime_state.write();
        liveliness.register_voter(builder_source.ident);       
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
        instance_id: builder_source.instance_id.fetch_add(1, Ordering::SeqCst),
        last_periodic_wait: Default::default(),
        is_in_graph: true,
        actor_start_time: Instant::now(),
        team_id,
        frame_rate_ms,
        show_thread_info: builder_source.show_thread_info,
    })
}

/// Implements the `Metric` trait for the `Work` struct, enabling it to be used as a telemetry metric.
impl Metric for Work {}

/// The `Work` struct represents a unit of work, used for workload analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    work: u16, // out of 10000 where 10000 is 100%
}

impl Work {
    /// Creates a new `Work` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value of the work, as a percentage.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Work` instance if the value is valid.
    pub fn new(value: f32) -> Option<Self> {
        if (0.0..=100.00).contains(&value) {
            Some(Work { work: (value * 100.0) as u16 }) // 10_000 is 100%
        } else {
            None
        }
    }

    /// Returns the rational representation of the work.
    pub fn rational(&self) -> (u64, u64) {
        (self.work as u64, 10_000)
    }

    /// Returns a `Work` instance representing 10% work.
    pub fn p10() -> Self {
        Work { work: 1000 }
    }

    /// Returns a `Work` instance representing 20% work.
    pub fn p20() -> Self {
        Work { work: 2000 }
    }

    /// Returns a `Work` instance representing 30% work.
    pub fn p30() -> Self {
        Work { work: 3000 }
    }

    /// Returns a `Work` instance representing 40% work.
    pub fn p40() -> Self {
        Work { work: 4000 }
    }

    /// Returns a `Work` instance representing 50% work.
    pub fn p50() -> Self {
        Work { work: 5000 }
    }

    /// Returns a `Work` instance representing 60% work.
    pub fn p60() -> Self {
        Work { work: 6000 }
    }

    /// Returns a `Work` instance representing 70% work.
    pub fn p70() -> Self {
        Work { work: 7000 }
    }

    /// Returns a `Work` instance representing 80% work.
    pub fn p80() -> Self {
        Work { work: 8000 }
    }

    /// Returns a `Work` instance representing 90% work.
    pub fn p90() -> Self {
        Work { work: 9000 }
    }

    /// Returns a `Work` instance representing 100% work.
    pub fn p100() -> Self {
        Work { work: 10_000 }
    }
}

/// Implements the `Metric` trait for the `MCPU` struct, enabling it to be used as a telemetry metric.
impl Metric for MCPU {}

/// The `MCPU` struct represents a unit of CPU usage, used for performance analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MCPU {
    mcpu: u16, // max 1024
}

impl MCPU {
    /// Creates a new `MCPU` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value of the MCPU, up to a maximum of 1024.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `MCPU` instance if the value is valid.
    pub fn new(value: u16) -> Option<Self> {
        if value <= 1024 && value > 0 {
            Some(Self { mcpu: value })
        } else {
            None
        }
    }

    /// Returns the mCPU value
    pub fn mcpu(&self) -> u16 {
        self.mcpu
    }

    /// Returns an `MCPU` instance representing 16 MCPU.
    pub fn m16() -> Self {
        MCPU { mcpu: 16 }
    }

    /// Returns an `MCPU` instance representing 64 MCPU.
    pub fn m64() -> Self {
        MCPU { mcpu: 64 }
    }

    /// Returns an `MCPU` instance representing 256 MCPU.
    pub fn m256() -> Self {
        MCPU { mcpu: 256 }
    }

    /// Returns an `MCPU` instance representing 512 MCPU.
    pub fn m512() -> Self {
        MCPU { mcpu: 512 }
    }

    /// Returns an `MCPU` instance representing 768 MCPU.
    pub fn m768() -> Self {
        MCPU { mcpu: 768 }
    }

    /// Returns an `MCPU` instance representing 1024 MCPU.
    pub fn m1024() -> Self {
        MCPU { mcpu: 1024 }
    }
}

/// The `Percentile` struct represents a percentile value, used for performance and workload analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentile(pub f64);

impl Percentile {
    /// Creates a new `Percentile` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value of the percentile, between 0.0 and 100.0.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Percentile` instance if the value is valid.
    fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns a `Percentile` instance representing the 25th percentile.
    pub fn p25() -> Self {
        Self(25.0)
    }

    /// Returns a `Percentile` instance representing the 50th percentile.
    pub fn p50() -> Self {
        Self(50.0)
    }

    /// Returns a `Percentile` instance representing the 75th percentile.
    pub fn p75() -> Self {
        Self(75.0)
    }

    /// Returns a `Percentile` instance representing the 90th percentile.
    pub fn p90() -> Self {
        Self(90.0)
    }

    /// Returns a `Percentile` instance representing the 80th percentile.
    pub fn p80() -> Self {
        Self(80.0)
    }

    /// Returns a `Percentile` instance representing the 96th percentile.
    pub fn p96() -> Self {
        Self(96.0)
    }

    /// Returns a `Percentile` instance representing the 99th percentile.
    pub fn p99() -> Self {
        Self(99.0)
    }

    /// Allows custom values within the valid range.
    ///
    /// # Arguments
    ///
    /// * `value` - The custom percentile value.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Percentile` instance if the value is valid.
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    /// Getter to access the inner percentile value.
    pub fn percentile(&self) -> f64 {
        self.0
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_work_new_valid() {
        assert_eq!(Work::new(50.0), Some(Work { work: 5000 }));
        assert_eq!(Work::new(0.0), Some(Work { work: 0 }));
        assert_eq!(Work::new(100.0), Some(Work { work: 10_000 }));
    }

    #[test]
    fn test_work_new_invalid() {
        assert_eq!(Work::new(-1.0), None);
        assert_eq!(Work::new(101.0), None);
    }

    #[test]
    fn test_work_rational() {
        let work = Work::new(25.0).unwrap();
        assert_eq!(work.rational(), (2500, 10_000));
    }

    #[test]
    fn test_work_percent_methods() {
        assert_eq!(Work::p10(), Work { work: 1000 });
        assert_eq!(Work::p20(), Work { work: 2000 });
        assert_eq!(Work::p30(), Work { work: 3000 });
        assert_eq!(Work::p40(), Work { work: 4000 });
        assert_eq!(Work::p50(), Work { work: 5000 });
        assert_eq!(Work::p60(), Work { work: 6000 });
        assert_eq!(Work::p70(), Work { work: 7000 });
        assert_eq!(Work::p80(), Work { work: 8000 });
        assert_eq!(Work::p90(), Work { work: 9000 });
        assert_eq!(Work::p100(), Work { work: 10_000 });
    }

    #[test]
    fn test_mcpu_new_valid() {
        assert_eq!(MCPU::new(512), Some(MCPU { mcpu: 512 }));
        assert_eq!(MCPU::new(0), None);
        assert_eq!(MCPU::new(1024), Some(MCPU { mcpu: 1024 }));
    }

    #[test]
    fn test_mcpu_new_invalid() {
        assert_eq!(MCPU::new(1025), None);
    }

    #[test]
    fn test_mcpu_rational() {
        let mcpu = MCPU::new(256).unwrap();
        assert_eq!(mcpu.mcpu(), 256);
    }

    #[test]
    fn test_mcpu_methods() {
        assert_eq!(MCPU::m16(), MCPU { mcpu: 16 });
        assert_eq!(MCPU::m64(), MCPU { mcpu: 64 });
        assert_eq!(MCPU::m256(), MCPU { mcpu: 256 });
        assert_eq!(MCPU::m512(), MCPU { mcpu: 512 });
        assert_eq!(MCPU::m768(), MCPU { mcpu: 768 });
        assert_eq!(MCPU::m1024(), MCPU { mcpu: 1024 });
    }

    #[test]
    fn test_percentile_new_valid() {
        assert_eq!(Percentile::new(25.0), Some(Percentile(25.0)));
        assert_eq!(Percentile::new(0.0), Some(Percentile(0.0)));
        assert_eq!(Percentile::new(100.0), Some(Percentile(100.0)));
    }

    #[test]
    fn test_percentile_new_invalid() {
        assert_eq!(Percentile::new(-1.0), None);
        assert_eq!(Percentile::new(101.0), None);
    }

    #[test]
    fn test_percentile_methods() {
        assert_eq!(Percentile::p25(), Percentile(25.0));
        assert_eq!(Percentile::p50(), Percentile(50.0));
        assert_eq!(Percentile::p75(), Percentile(75.0));
        assert_eq!(Percentile::p80(), Percentile(80.0));
        assert_eq!(Percentile::p90(), Percentile(90.0));
        assert_eq!(Percentile::p96(), Percentile(96.0));
        assert_eq!(Percentile::p99(), Percentile(99.0));
    }

    #[test]
    fn test_percentile_custom() {
        assert_eq!(Percentile::custom(42.0), Some(Percentile(42.0)));
        assert_eq!(Percentile::custom(-1.0), None);
        assert_eq!(Percentile::custom(101.0), None);
    }

    #[test]
    fn test_percentile_getter() {
        let percentile = Percentile::new(42.0).unwrap();
        assert_eq!(percentile.percentile(), 42.0);
    }
}


#[cfg(test)]
mod test_actor_builder {
    use crate::{GraphBuilder, GraphLivelinessState, SteadyCommander};
    use super::*;

    #[test]
    fn test_actor_builder_creation_spawn() {
        let mut graph =  GraphBuilder::for_testing().build(());
        let builder = ActorBuilder::new(&mut graph);
        assert_eq!(builder.actor_name.name, "");
        assert_eq!(builder.refresh_rate_in_bits, 0);
        assert_eq!(builder.window_bucket_in_bits, 0);
        builder.build(|c| async move {             
            assert_eq!(true, c.is_liveliness_in(&vec![ GraphLivelinessState::Building ]));            
            Ok(()) }, &mut Threading::Spawn);
    }

    #[test]
    fn test_actor_builder_creation_join() {
        let mut graph =  GraphBuilder::for_testing().build(());
        let builder = ActorBuilder::new(&mut graph);
        assert_eq!(builder.actor_name.name, "");
        assert_eq!(builder.refresh_rate_in_bits, 0);
        assert_eq!(builder.window_bucket_in_bits, 0);
        let mut t = ActorTeam::new(&graph);
        builder.build(|c| async move {
            assert_eq!(true, c.is_liveliness_in(&vec![ GraphLivelinessState::Building ]));
            Ok(()) }, &mut Threading::Join(&mut t));
    }

    #[test]
    fn test_work_new() {
        let work = Work::new(50.0).unwrap();
        assert_eq!(work.work, 5000);
    }

    #[test]
    fn test_mcpu_new() {
        let mcpu = MCPU::new(512).unwrap();
        assert_eq!(mcpu.mcpu, 512);
    }

    #[test]
    fn test_percentile_new() {
        let percentile = Percentile::new(99.0).unwrap();
        assert_eq!(percentile.percentile(), 99.0);
    }
}
