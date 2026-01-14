mod args;
#[allow(unused_imports)]
use log::*;
use crate::args::Args;
use std::time::Duration;
use steady_state::*;

mod actor {
        pub mod final_consumer;
        pub mod tick_consumer;
        pub mod tick_generator;
        pub mod tick_relay;
}

fn main() -> Result<(), Box<dyn Error>> {
    let opt = Args::parse();

    if let Err(e) = steady_state::init_logging(opt.loglevel, None) {
        //do not use logger to report logger could not start
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }
    info!("Starting up");


    let mut graph = build_graph(GraphBuilder::for_production().build(opt.clone()));

    graph.start();

    {  //remove this block to run forever.
       std::thread::sleep(Duration::from_secs(600));
       graph.request_shutdown(); //actors can also call stop as desired on the context or monitor
    }

    graph.block_until_stopped(Duration::from_secs(2))
}

fn build_graph(mut graph: Graph) -> steady_state::Graph {


    //this common root of the channel builder allows for common config of all channels
    let base_channel_builder = graph.channel_builder()
        .with_type()
        .with_line_expansion(0.002f32);//expecting a lot of records
    //this common root of the actor builder allows for common config of all actors
    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor        
        .with_mcpu_percentile(Percentile::p80())
        .with_load_percentile(Percentile::p80());
    //build channels

    const PARALLEL:usize = 5;
    const CHANNEL_SIZE:usize = 2000;

    let (tick_consumern_to_finalconsumer_tick_counts_tx, finalconsumer_tick_counts_rx) = base_channel_builder
        .with_capacity(CHANNEL_SIZE)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()),AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p80()),AlertColor::Red)
        .with_avg_rate()
        .build_channel_bundle::<_,PARALLEL>();
    
    let (tickgenerator_ticks_tx, tickgenerator_to_tick_consumer_ticks_rx) = base_channel_builder
        .with_capacity(CHANNEL_SIZE)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()),AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p80()),AlertColor::Red)
        .with_avg_rate()
        .build_channel_bundle::<_,PARALLEL>();



    {
       base_actor_builder.with_name("FinalConsumer")
                 .build( move |context| actor::final_consumer::run(context
                                            , finalconsumer_tick_counts_rx.clone())
                               , ScheduleAs::SoloAct
                 );
    }

    tickgenerator_to_tick_consumer_ticks_rx.iter()
         .zip(tick_consumern_to_finalconsumer_tick_counts_tx.iter())
        .for_each(|(tick_consumer_ticks_rx, tick_consumer_tick_counts_tx)| {
            {
                let tick_consumer_ticks_rx = tick_consumer_ticks_rx.clone();
                let tick_consumer_tick_counts_tx = tick_consumer_tick_counts_tx.clone();

                let (tickrelay_ticks_tx, tickrelay_ticks_rx) = base_channel_builder
                    .with_capacity(CHANNEL_SIZE)
                    .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Yellow)
                    .with_filled_trigger(Trigger::AvgAbove(Filled::p50()),AlertColor::Orange)
                    .with_filled_trigger(Trigger::AvgAbove(Filled::p80()),AlertColor::Red)
                    .with_avg_rate()
                    .build_channel();


                base_actor_builder.with_name("TickRelay")
                    .build( move |context| actor::tick_relay::run(context
                                            , tick_consumer_ticks_rx.clone()
                                            , tickrelay_ticks_tx.clone()
                    ), ScheduleAs::SoloAct
                    );

                base_actor_builder.with_name("TickConsumer")
                    .build( move |context| actor::tick_consumer::run(context
                                           , tickrelay_ticks_rx.clone()
                                           , tick_consumer_tick_counts_tx.clone()
                    ), ScheduleAs::SoloAct
                   );

            }
        });

    {
       base_actor_builder.with_name("TickGenerator")
                 .build( move |context| actor::tick_generator::run(context
                                            , tickgenerator_ticks_tx.clone())
                               , ScheduleAs::SoloAct
                 );
    }


    graph
}


#[cfg(test)]
mod graph_tests {
    use steady_state::GraphBuilder;

    #[test]
    fn test_graph_one() {
    
             let _graph = GraphBuilder::for_testing().build(());
    //         graph.start();
    //
    //         let mut director = graph.sidechannel_director();
    //
    //           //  write your test here, send messages to edge nodes and get responses
    //           let response = plane.node_call(Box::new(Tick { value: 42 }), ActorName::new("TickGenerator",None)).await;
    //           if let Some(msg) = response {
    //
    //              //TODO: confirm
    //
    //           }
    //           Delay::new(Duration::from_millis(100)).await;  //wait for message to propagate
    //           let response = plane.node_call(Box::new(()), ActorName::new("FinalConsumer",None)).await;
    //
    //              //TODO: confirm
    //
    //         graph.request_shutdown();
    //         graph.block_until_stopped(Duration::from_secs(3));
    
    }
}
