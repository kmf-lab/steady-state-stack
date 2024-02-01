mod args;

use std::panic;
use std::process::exit;
use structopt::*;
use log::*;

use futures_timer::Delay;
use args::Args;
use std::time::Duration;

// here are the actors that will be used in the graph.
// note that the actors are in a separate module and we must use the structs/enums and
// bring in the behavior functions
mod actor {
    pub mod example_empty_actor;
    pub mod data_generator;
    #[cfg(test)]
    pub use data_generator::WidgetInventory;
    pub mod data_approval;
    #[cfg(test)]
    pub use data_approval::ApprovedWidgets;
    pub mod data_consumer;
}
#[cfg(test)]
use crate::actor::*;

use bastion::{Bastion, run};
use bastion::prelude::SupervisionStrategy;
use steady_state::*;


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

    if let Err(e) = steady_state::init_logging(&opt.loglevel) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }



    let mut graph = build_graph(&opt); //graph is built here and tested below in the test section.
    graph.start();



    //remove this block to run forever.
    //run! is a macro provided by bastion that will block until the future is resolved.
    run!(async {

        /*
        use futures::prelude::*;
        use nuclei::*;
        use std::os::unix::net::UnixStream;
        use signal_hook::low_level::pipe;

         nuclei::spawn(async move {
                let (a, mut b) = Handle::<UnixStream>::pair()?;
                signal_hook::pipe::register(signal_hook::SIGINT, a)?;
                println!("Waiting for Ctrl-C...");
                b.read_exact(&mut [0]).await?
         });
         */



        // note we use (&opt) just to show we did NOT transfer ownership
        Delay::new(Duration::from_secs(opt.duration)).await;
        graph.request_shutdown();
        //TODO:: need some way to wait for shutdown to complete
        Bastion::stop();
    });

    //wait for bastion to cleanly stop all actors
    Bastion::block_until_stopped();
}


fn build_graph(cli_arg: &Args) -> steady_state::Graph {
    debug!("args: {:?}",&cli_arg);

    //TODO: move for special debug flag.
    #[cfg(debug_assertions)]
    panic::set_hook(Box::new(|panic_info| {
        // You can log the panic information here if needed
        eprintln!("Application panicked: {}", panic_info);
        // Exit with status code -1
        exit(-1);
    }));

    //create the mutable graph object
    let mut graph = steady_state::Graph::new();

    Bastion::init(); //init bastion runtime

    //create all the needed channels between actors
    let example_capacity = 4000;

    //here are the parts of the channel they both have in common, this could be done
    // in place for each but we are showing here how you can do this for more complex projects.
    let base_builder = graph.channel_builder()
                            .with_compute_window_floor(Duration::from_secs(12))
                            .with_labels(&["widgets"],true)
                            .with_type()
                            .with_line_expansion();

    //upon construction these are set up to be monitored by the telemetry telemetry
    let (generator_tx, generator_rx) = base_builder
                     .with_rate_percentile(Percentile::p80())
                     .with_red(Trigger::AvgFilledAbove(Filled::p70()))
                     .with_yellow(Trigger::StdDevsFilledAbove(StdDev::one(),Filled::p70()))
                     .with_capacity(example_capacity)
                     .build();

    let (consumer_tx, consumer_rx) = base_builder
                     .with_filled_standard_deviation(StdDev::two_and_a_half())
                     .with_capacity(example_capacity)
                     .build();

    //the above tx rx objects will be owned by the children closures below then cloned
    //each time we need to startup a new child telemetry instance. This way when an telemetry fails
    //we still have the original to clone from.
    //
    //given your supervision strategy create the children to be added to the graph.
    let _ = Bastion::supervisor(|supervisor|
        supervisor.with_strategy(SupervisionStrategy::OneForOne)
            .children(|children| {
                let cli_arg = cli_arg.clone(); //example passing args to child telemetry
                graph.add_to_graph("generator"
                                   , children.with_redundancy(0)
                                   , move |monitor| actor::data_generator::run(monitor
                                                                      , cli_arg.clone()
                                                                      , generator_tx.clone()
                                                                     )
                )
            })
            .children(|children| {
                    graph.add_to_graph("approval"
                                      , children.with_redundancy(0)
                                   , move |monitor| actor::data_approval::run(monitor
                                                   , generator_rx.clone()
                                                   , consumer_tx.clone()
                                                                     )

                    )

            })
            .children(|children| {
                    graph.add_to_graph("consumer"
                                      , children.with_redundancy(0)
                                     ,move |monitor| actor::data_consumer::run(monitor
                                                           , consumer_rx.clone()
                                                                        )


                            )
            })
    ).expect("OneForOne supervisor creation error.");

    graph.init_telemetry();
    graph
}



#[cfg(test)]
mod tests {
    use bastion::prelude::{Distributor, SendError};
    use bastion::run;
    use super::*;

    #[async_std::test]
    async fn test_graph_one() {

        let test_ops = Args {
            duration: 21,
            loglevel: "debug".to_string(),
            gen_rate_micros: 0,
        };

        let mut graph = build_graph(&test_ops);
        graph.start();

        let distribute_generator:Distributor = Distributor::named("testing-generator");
        let distribute_consumer:Distributor = Distributor::named("testing-consumer");

        run!(async {

            let to_send = WidgetInventory {
                count: 42
                , _payload: 0
            };


            let answer_generator: Result<&str, SendError> = distribute_generator.request(to_send).await.unwrap();
            assert_eq!("ok",answer_generator.unwrap());

            let expected_message = ApprovedWidgets {
                original_count: 42,
                approved_count: 21,
            };
            let answer_consumer: Result<&str, SendError> = distribute_consumer.request(expected_message).await.unwrap();
            assert_eq!("ok",answer_consumer.unwrap());


            Bastion::stop();
        });
        Bastion::block_until_stopped();
    }

}

