mod args;
#[macro_use]
mod steady;

use structopt::*;
use log;
use log::{debug};
use bastion::Bastion;
use bastion::prelude::*;
use futures_timer::Delay;
use crate::args::Args;
use std::time::Duration;
use crate::steady::{SteadyGraph, SteadyTx, steady_logging_init};

// here are the actors that will be used in the graph.
// note that the actors are in a separate module and we must use the structs/enums and
// bring in the behavior functions
mod actor {
    pub mod example_empty_actor;
    pub mod data_generator;
    pub use data_generator::WidgetInventory;
    pub mod data_approval;
    pub use data_approval::ApprovedWidgets;
    pub mod data_consumer;
}
use crate::actor::*;

// This is a good template for your future main function. It should me minimal and just
// get the command line args and start the graph. The graph is built in a separate function.
// This is important so that the graph can be tested.
// Try to keep the entire method small enough to see on one screen. Note that sometimes you
// will need to validate the command line args before building the graph. This is fine.
// review the Opt struct in args.rs for how to validate command line args.
// The running graph will never fail or exit with a panic but you can panic in the main
// if you discover a problem with the command line args. Fail fast is a good thing.
// Note that the main function is not async. This keeps it simple.
// Further note the main function is not a test. It is not run by any test. Keep it small.
fn main() {
    // a typical begging by fetching the command line args and starting logging
    let opt = Args::from_args();
    if let Err(e) = steady_logging_init(&opt.loglevel) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }

    Bastion::init(); //init bastion runtime
    build_graph(&opt); //graph is built here and tested below in the test section.
    Bastion::start(); //start the graph

    //remove this block to run forever.
    //run! is a macro provided by bastion that will block until the future is resolved.
    run!(async {
        // note we use (&opt) just to show we did NOT transfer ownership
        Delay::new(Duration::from_secs((&opt).duration)).await;
        Bastion::stop();
    });

    //wait for bastion to cleanly stop all actors
    Bastion::block_until_stopped();
}

fn build_graph(cli_arg: &Args) {
    debug!("args: {:?}",&cli_arg);

    //create the mutable graph object
    let mut graph = SteadyGraph::new();

    //create all the needed channels between actors

    //upon construction these are set up to be monitored by the telemetry actor
    let (generator_tx, generator_rx): (SteadyTx<WidgetInventory>, _) = graph.new_channel(38);
    let (consumer_tx, consumer_rx): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(38);
    //the above tx rx objects will be owned by the children closures below then cloned
    //each time we need to startup a new child actor instance. This way when an actor fails
    //we still have the original to clone from.
    //
    //given your supervision strategy create the children to be added to the graph.
    let _ = Bastion::supervisor(|supervisor|
        supervisor.with_strategy(SupervisionStrategy::OneForOne)
            .children(|children| {
                let cli_arg = cli_arg.clone(); //example passing args to child actor
                graph.add_to_graph("generator"
                                  , children.with_redundancy(0)
                        , move |monitor| actor::data_generator::run(monitor
                                                                    , cli_arg.clone()
                                                                    , generator_tx.clone())
                    )
            })
            .children(|children| {
                    graph.add_to_graph("approval"
                                      , children.with_redundancy(0)
                        , move |monitor| actor::data_approval::run(monitor
                                                   , generator_rx.clone()
                                                   , consumer_tx.clone())
                                                    )
            })
            .children(|children| {
                    graph.add_to_graph("consumer"
                                      , children.with_redundancy(0)
                            ,move |monitor| actor::data_consumer::run(monitor
                                                     , consumer_rx.clone())
                            )
            })
    ).expect("OneForOne supervisor creation error.");

    graph.init_telemetry();

}

#[cfg(test)]
mod tests {
    use bastion::prelude::{Distributor};
    use bastion::run;
    use super::*;

    #[async_std::test]
    async fn test_graph_one() {

        let test_ops = Args {
            duration: 21,
            loglevel: "debug".to_string(),
            gen_rate_ms: 0,
        };

        Bastion::init();
        build_graph(&test_ops);
        Bastion::start();

        let distribute_generator:Distributor = Distributor::named("testing-generator");
        let distribute_consumer:Distributor = Distributor::named("testing-consumer");

        run!(async {

            let to_send = WidgetInventory { count: 42 };

            let answer_generator: Result<&str, SendError> = distribute_generator.request(to_send).await.unwrap();
            assert_eq!("ok",answer_generator.unwrap());

            let expected_message = ApprovedWidgets {
                original_count: 42,
                approved_count: 21
            };
            let answer_consumer: Result<&str, SendError> = distribute_consumer.request(expected_message).await.unwrap();
            assert_eq!("ok",answer_consumer.unwrap());


            Bastion::stop();
        });
        Bastion::block_until_stopped();
    }

}


