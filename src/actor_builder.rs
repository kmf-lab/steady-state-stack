use std::any::Any;
use bastion::children::Children;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use bastion::Callbacks;
use futures::lock::Mutex;
use log::*;
use crate::{Graph, GraphLivelinessState, SteadyContext};
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
    args: Arc<Box<dyn Any+Send+Sync>>,
    telemetry_tx: Arc<Mutex<Vec<CollectorDetail>>>,
    channel_count: Arc<AtomicUsize>,
    runtime_state: Arc<Mutex<GraphLivelinessState>>,
    redundancy: usize,
    id: usize,
}


impl ActorBuilder {
    pub fn new( graph: &mut Graph, name: &'static str) -> ActorBuilder {
        let id = graph.monitor_count;
        graph.monitor_count += 1;

        ActorBuilder {
            name,
            id,
            args : graph.args.clone(),
            redundancy: 1,
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),
        }
    }

    pub fn with_redundancy(&self, count:usize) -> ActorBuilder {
        let mut result = self.clone();
        result.redundancy = count.max(1);
        result
    }


    pub fn build<F,I>(self, children: Children, init: I) -> Children
        where I: Fn(SteadyContext) -> F + Send + 'static + Clone,
             F: Future<Output = Result<(),()>> + Send + 'static ,
    {
        let name          = self.name;
        let id            = self.id;
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
                let init_fn_clone = init.clone();
                let channel_count = channel_count.clone();
                let telemetry_tx  = telemetry_tx.clone();
                let runtime_state = runtime_state.clone();
                let args          = args.clone();

                let count_restarts = count_restarts.clone();

                async move {

                    let monitor = SteadyContext {
                        runtime_state:  runtime_state.clone(),
                        channel_count: channel_count.clone(),
                        name,
                        ctx: Some(ctx),
                        id,
                        redundancy,
                        args: args.clone(),
                        all_telemetry_rx: telemetry_tx.clone(),
                        count_restarts: count_restarts.clone(),
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
