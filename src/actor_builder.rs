use std::any::Any;
use bastion::children::Children;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use std::time::Duration;
use bastion::Callbacks;
use futures::lock::Mutex;
use log::*;
use crate::{AlertColor, Graph, GraphLivelinessState, MCPU, Percentile, StdDev, SteadyContext, Trigger, Work};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;

fn build_node_monitor_callback( count_restarts: &Arc<AtomicU32>

) -> Callbacks {
    let callback_count_restarts = count_restarts.clone();

    Callbacks::new()
        .with_after_restart(move || {
            if callback_count_restarts.load(std::sync::atomic::Ordering::Relaxed).lt(&u32::MAX) {
                callback_count_restarts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })

}

#[derive(Clone)]
pub struct ActorBuilder {
    name: &'static str,
    suffix: Option<usize>,
    args: Arc<Box<dyn Any+Send+Sync>>,
    telemetry_tx: Arc<Mutex<Vec<CollectorDetail>>>,
    channel_count: Arc<AtomicUsize>,
    runtime_state: Arc<Mutex<GraphLivelinessState>>,
    redundancy: usize,
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

}


impl ActorBuilder {
    pub fn new( graph: &mut Graph) -> ActorBuilder {

        ActorBuilder {
            name: "",
            suffix: None,
            monitor_count: graph.monitor_count.clone(),
            args : graph.args.clone(),
            redundancy: 1,
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),

            refresh_rate_in_bits: 6, // 1<<6 == 64
            window_bucket_in_bits: 5, // 1<<5 == 32

            percentiles_mcpu: Vec::new(),
            percentiles_work: Vec::new(),
            std_dev_mcpu: Vec::new(),
            std_dev_work: Vec::new(),
            trigger_mcpu: Vec::new(),
            trigger_work: Vec::new(),
            avg_mcpu: false,
            avg_work: false,
            usage_review: false,

        }
    }

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

    pub fn with_compute_refresh_window_bucket_bits(&self
                                                   , refresh_bucket_in_bits:u8
                                                   , window_bucket_in_bits: u8) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = refresh_bucket_in_bits;
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    pub fn with_name(&self, name: &'static str) -> Self {
        let mut result = self.clone();
        result.name = name;
        result
    }

    pub fn with_name_suffix(&self, suffix: usize) -> Self {
        let mut result = self.clone();
        result.suffix = Some(suffix);
        result
    }
    pub fn with_mcpu_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_mcpu.push(config);
        result
    }
    pub fn with_work_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_work.push(config);
        result
    }

    pub fn with_avg_mcpu(&self) -> Self {
        let mut result = self.clone();
        result.avg_mcpu=true;
        result
    }
    pub fn with_avg_work(&self) -> Self {
        let mut result = self.clone();
        result.avg_work = true;
        result
    }

    pub fn with_mcpu_trigger(&self, bound: Trigger<MCPU>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_mcpu.push((bound,color));
        result
    }

    pub fn with_work_trigger(&self, bound: Trigger<Work>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_work.push((bound,color));
        result
    }

    pub fn with_redundancy(&self, count:usize) -> ActorBuilder {
        let mut result = self.clone();
        result.redundancy = count.max(1);
        result
    }


    /// moving average and percentile window will be at least this size but may be larger
    pub fn with_compute_window_floor(&self, duration: Duration) -> Self {
        let result = self.clone();
        let millis: u128 = duration.as_micros();
        let frame_ms: u128 = 1000u128 * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        let est_buckets = millis / frame_ms;
        //find the next largest power of 2
        let window_bucket_in_bits = (est_buckets as f32).log2().ceil() as u8;
        result.with_compute_window_bucket_bits(window_bucket_in_bits)
    }

    /// NOTE the default is 1 second
    pub fn with_compute_window_bucket_bits(&self, window_bucket_in_bits: u8) -> Self {
        assert!(window_bucket_in_bits <= 30);
        let mut result = self.clone();
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    pub fn build<F,I>(self, children: Children, init: I) -> Children
        where I: Fn(SteadyContext) -> F + Send + 'static + Clone,
             F: Future<Output = Result<(),()>> + Send + 'static ,
    {
        let name          = self.name;
        let id            = self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let telemetry_tx  = self.telemetry_tx;
        let channel_count = self.channel_count;
        let runtime_state = self.runtime_state;
        let args          = self.args;
        let redundancy    = self.redundancy;

        let result = {
            let count_restarts = Arc::new(AtomicU32::new(0));
            let watch_node_callback = build_node_monitor_callback(&count_restarts);
            let count_restarts = count_restarts.clone();

            children.with_redundancy(self.redundancy)
                    .with_exec(move |ctx| {

                let init_fn_clone  = init.clone();
                let channel_count  = channel_count.clone();
                let telemetry_tx   = telemetry_tx.clone();
                let runtime_state  = runtime_state.clone();
                let args           = args.clone();
                let count_restarts = count_restarts.clone();

                let actor_metadata = Arc::new(
                    ActorMetaData {
                        id, name,
                        avg_mcpu: self.avg_mcpu,
                        avg_work: self.avg_work,
                        percentiles_mcpu: self.percentiles_mcpu.clone(),
                        percentiles_work: self.percentiles_work.clone(),
                        std_dev_mcpu: self.std_dev_mcpu.clone(),
                        std_dev_work: self.std_dev_work.clone(),
                        trigger_mcpu: self.trigger_mcpu.clone(),
                        trigger_work: self.trigger_work.clone(),
                        usage_review: self.usage_review,
                        refresh_rate_in_bits: self.refresh_rate_in_bits,
                        window_bucket_in_bits: self.window_bucket_in_bits,
                    }
                );

                async move {
                    let monitor = SteadyContext {
                        runtime_state,
                        channel_count: channel_count.clone(),
                        name,
                        ctx: Some(ctx),
                        id,
                        redundancy,
                        args: args.clone(),
                        all_telemetry_rx: telemetry_tx.clone(),
                        count_restarts: count_restarts.clone(),
                        actor_metadata
                    };

                    match init_fn_clone(monitor).await {
                        Ok(_) => {
                            trace!("Actor {:?} finished ", name);
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
                .with_callbacks(watch_node_callback)
                //.with_resizer()  TODO: add later.
        };

        #[cfg(debug_assertions)]
        {
            use bastion::distributor::Distributor;
            result.with_distributor(Distributor::named(format!("testing-{name}")))
        }
        #[cfg(not(debug_assertions))]
        {
            result
        }
    }
}
