mod args;
use structopt::StructOpt;
#[allow(unused_imports)]
use log::*;
use crate::args::Args;
use std::time::Duration;
use steady_state::*;





mod actor {
    
        pub mod final_consumer;
    
        pub mod tick_consumer;
    
        pub mod tick_generator;
    
}

fn main() {
    let opt = Args::from_args();
    if let Err(e) = steady_state::init_logging(&opt.loglevel) {
        //do not use logger to report logger could not start
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }
    info!("Starting up");

    let mut graph = build_graph(&opt);

    graph.start();

    {  //remove this block to run forever.
       std::thread::sleep(Duration::from_secs(60));
       graph.stop(); //actors can also call stop as desired on the context or monitor
    }

    graph.block_until_stopped(Duration::from_secs(2));
}

fn build_graph(cli_arg: &Args) -> steady_state::Graph {
    debug!("args: {:?}",&cli_arg);

    let mut graph = steady_state::Graph::new(cli_arg.clone());

    //this common root of the channel builder allows for common config of all channels
    let base_channel_builder = graph.channel_builder()
        .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10))
        .with_type()
        .with_line_expansion();
    //this common root of the actor builder allows for common config of all actors
    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor
        .with_mcpu_percentile(Percentile::p80())
        .with_work_percentile(Percentile::p80())
        .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10));
    //build channels
    
    let (tick_consumern_to_finalconsumer_tick_counts_tx, finalconsumer_tick_counts_rx) = base_channel_builder
        .with_capacity(100)
        .build_as_bundle::<_,3>();
    
    let (tickgenerator_ticks_tx, tickgenerator_to_tick_consumer_ticks_rx) = base_channel_builder
        .with_capacity(1000)
        .build_as_bundle::<_,3>();
    
    //build actors
    
    {
       base_actor_builder.with_name("FinalConsumer")
                 .build_spawn( move |context| actor::final_consumer::run(context
                                            , finalconsumer_tick_counts_rx.clone())
                 );
    }
    {
       let tickconsumer1_ticks_rx = tickgenerator_to_tick_consumer_ticks_rx[0].clone();
      
      
       let tickconsumer1_tick_counts_tx = tick_consumern_to_finalconsumer_tick_counts_tx[0].clone();
      
    
       base_actor_builder.with_name("TickConsumer1")
                 .build_spawn( move |context| actor::tick_consumer::run(context
                                            , tickconsumer1_ticks_rx.clone()
                                            , tickconsumer1_tick_counts_tx.clone())
                 );
    }
    {
       let tickconsumer2_ticks_rx = tickgenerator_to_tick_consumer_ticks_rx[1].clone();
      
      
       let tickconsumer2_tick_counts_tx = tick_consumern_to_finalconsumer_tick_counts_tx[1].clone();
      
    
       base_actor_builder.with_name("TickConsumer2")
                 .build_spawn( move |context| actor::tick_consumer::run(context
                                            , tickconsumer2_ticks_rx.clone()
                                            , tickconsumer2_tick_counts_tx.clone())
                 );
    }
    {
       let tickconsumer3_ticks_rx = tickgenerator_to_tick_consumer_ticks_rx[2].clone();
      
      
       let tickconsumer3_tick_counts_tx = tick_consumern_to_finalconsumer_tick_counts_tx[1].clone();
      
    
       base_actor_builder.with_name("TickConsumer3")
                 .build_spawn( move |context| actor::tick_consumer::run(context
                                            , tickconsumer3_ticks_rx.clone()
                                            , tickconsumer3_tick_counts_tx.clone())
                 );
    }
    {
       base_actor_builder.with_name("TickGenerator")
                 .build_spawn( move |context| actor::tick_generator::run(context
                                            , tickgenerator_ticks_tx.clone())
                 );
    }
    graph
}

/*
#[cfg(test)]
mod graph_tests {
    use std::ops::DerefMut;
    use std::time::Duration;
    use async_std::test;
    use steady_state::*;
    use crate::args::Args;
    use crate::build_graph;

    #[test]
    async fn test_graph_one() {

            let test_ops = Args {
                loglevel: "debug".to_string(),
                systemd_install: false,
                systemd_uninstall: false,
            };
            let mut graph = build_graph(&test_ops);
            graph.start();
            let mut guard = graph.sidechannel_director().await;
            if let Some(plane) = guard.deref_mut() {

              //  write your test here, send messages to edge nodes and get responses
              //  let response = plane.node_call(Box::new(SOME_STRUCT), "SOME_NODE_NAME").await;
              //  if let Some(msg) = response {
              //  }

            }
            drop(guard);
            graph.stop();
            graph.block_until_stopped(Duration::from_secs(3));

    }
}

 */