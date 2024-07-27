mod args;
use structopt::StructOpt;
#[allow(unused_imports)]
use log::*;
use crate::args::Args;
use std::time::Duration;
use steady_state::*;
use steady_state::actor_builder::ActorTeam;

mod actor {
        pub mod final_consumer;
        pub mod tick_consumer;
        pub mod tick_generator;
        pub mod tick_relay;
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
       std::thread::sleep(Duration::from_secs(600));
       graph.request_stop(); //actors can also call stop as desired on the context or monitor
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
        .with_line_expansion(0.002f32);//expecting a lot of records
    //this common root of the actor builder allows for common config of all actors
    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor
        .with_mcpu_percentile(Percentile::p80())
        .with_work_percentile(Percentile::p80())
        .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10));
    //build channels

    const PARALLEL:usize = 5;
    const CHANNEL_SIZE:usize = 2000;

    let (tick_consumern_to_finalconsumer_tick_counts_tx, finalconsumer_tick_counts_rx) = base_channel_builder
        .with_capacity(CHANNEL_SIZE)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()),AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p80()),AlertColor::Red)
        .with_avg_rate()
        .build_as_bundle::<_,PARALLEL>();
    
    let (tickgenerator_ticks_tx, tickgenerator_to_tick_consumer_ticks_rx) = base_channel_builder
        .with_capacity(CHANNEL_SIZE)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()),AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p80()),AlertColor::Red)
        .with_avg_rate()
        .build_as_bundle::<_,PARALLEL>();

    {
        base_actor_builder.with_name("TickGenerator")
            .build_spawn( move |context| actor::tick_generator::run(context
                                                                    , tickgenerator_ticks_tx.clone())
            );
    }
    {
       base_actor_builder.with_name("FinalConsumer")
                 .build_spawn( move |context| actor::final_consumer::run(context
                                            , finalconsumer_tick_counts_rx.clone())
                 );
    }

    tickgenerator_to_tick_consumer_ticks_rx.iter()
         .zip(tick_consumern_to_finalconsumer_tick_counts_tx.iter()).enumerate()
        .for_each(|(_i, (tick_consumer_ticks_rx, tick_consumer_tick_counts_tx))| {
            {
                let mut team =  ActorTeam::default();

                let tick_consumer_ticks_rx = tick_consumer_ticks_rx.clone();
                let tick_consumer_tick_counts_tx = tick_consumer_tick_counts_tx.clone();

                let (tickrelay_ticks_tx, tickrelay_ticks_rx) = base_channel_builder
                    .with_capacity(CHANNEL_SIZE)
                    .with_filled_trigger(Trigger::AvgAbove(Filled::p30()), AlertColor::Yellow)
                    .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Orange)
                    .with_filled_trigger(Trigger::AvgAbove(Filled::p80()), AlertColor::Red)
                    .with_avg_rate()
                    .build();

                base_actor_builder.with_name("TickRelay")
                    .build_join(move |context| actor::tick_relay::run(context
                                                                      , tick_consumer_ticks_rx.clone()
                                                                      , tickrelay_ticks_tx.clone()
                    )
                                , &mut team
                    );
                base_actor_builder.with_name("TickConsumer")
                    .build_join(move |context| actor::tick_consumer::run(context
                                                                         , tickrelay_ticks_rx.clone()
                                                                         , tick_consumer_tick_counts_tx.clone()
                    )
                                , &mut team
                    );


                team.spawn();
            }
        });




    graph
}


#[cfg(test)]
mod graph_tests {
    use std::ops::DerefMut;
    use std::time::Duration;
    use async_std::test;
    use futures_timer::Delay;
    use crate::actor::tick_generator::Tick;
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
            let response = plane.node_call(Box::new(Tick { value: 42 }), "TickGenerator").await;
            if let Some(msg) = response {
                //TODO: confirm
            }
            Delay::new(Duration::from_millis(100)).await;  //wait for message to propagate
            let response = plane.node_call(Box::new(()), "FinalConsumer").await;

            //TODO: confirm

        }
        drop(guard);
        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(3));

    }
}

