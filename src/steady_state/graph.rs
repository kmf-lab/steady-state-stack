use bastion::children::Children;
use std::future::Future;
use bastion::Callbacks;
use log::{error, info};
use bastion::distributor::Distributor;
use crate::steady_state::{SteadyContext, Graph};

pub(crate) fn configure_for_graph<F,I>(graph: & mut Graph, name: & 'static str, c: Children, init: I ) -> Children
    where I: Fn(SteadyContext) -> F + Send + 'static + Clone,
          F: Future<Output = Result<(),()>> + Send + 'static ,
{
    let result = {
        let init_clone = init.clone();

        let id = graph.monitor_count;
        graph.monitor_count += 1;
        let telemetry_tx = graph.all_telemetry_rx.clone();
        let channel_count = graph.channel_count.clone();
        let runtime_state = graph.runtime_state.clone();

        let callbacks = Callbacks::new().with_after_stop(|| {
            //TODO: new telemetry counting actor restarts
        });
        c.with_exec(move |ctx| {
            let init_fn_clone = init_clone.clone();
            let channel_count = channel_count.clone();
            let telemetry_tx = telemetry_tx.clone();
            let runtime_state = runtime_state.clone();

            async move {
                //this telemetry now owns this monitor

                let monitor = SteadyContext {
                    runtime_state:  runtime_state.clone(),
                    channel_count: channel_count.clone(),
                    name,
                    ctx: Some(ctx),
                    id,
                    all_telemetry_rx: telemetry_tx.clone(),
                };


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
            .with_callbacks(callbacks)
            .with_name(name)
    };

    #[cfg(test)] {
        result.with_distributor(Distributor::named(format!("testing-{name}")))
    }
    #[cfg(not(test))] {
        result
    }
}

/*   //TODO: future feature.

pub(crate) fn callbacks() -> Callbacks {

   //is the call back recording events for the elemetry?
   Callbacks::new()       .with_before_start( || {
           //TODO: record the name? of the telemetry on the graph needed?
           // info!("before start");
       })
       .with_after_start( || {
           //TODO: change telemetry to started if needed
           // info!("after start");
       })
       .with_after_stop( || {
           //TODO: record this telemetry has stopped on the graph
             info!("after stop");
       })
       .with_after_restart( || {
           //TODO: record restart count on the graph
//info!("after restart");
       })
}*/