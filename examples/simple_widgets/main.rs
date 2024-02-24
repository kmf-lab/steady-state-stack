mod args;

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

    pub mod data_feedback;
    #[cfg(test)]
    pub use data_feedback::FailureFeedback;
    #[cfg(test)]
    pub use data_feedback::ChangeRequest;

}
#[cfg(test)]
use crate::actor::*;

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
        Delay::new(Duration::from_secs(opt.duration)).await;
        graph.stop(Duration::from_secs(2));
    });

    graph.block_until_stopped();

}


fn build_graph(cli_arg: &Args) -> steady_state::Graph {
    debug!("args: {:?}",&cli_arg);

    //create the mutable graph object
    let mut graph = steady_state::Graph::new(cli_arg.clone());
    graph.start();

    //here are the parts of the channel they both have in common, this could be done
    // in place for each but we are showing here how you can do this for more complex projects.
    let base_channel_builder = graph.channel_builder()
                            .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10))
                            .with_labels(&["widgets"],true)
                            .with_filled_trigger(Trigger::AvgAbove(Filled::p50())
                                             ,AlertColor::Red)
                            .with_filled_trigger(Trigger::AvgAbove(Filled::p20())
                                             ,AlertColor::Yellow)
                            .with_type()
                            .with_line_expansion();

    //upon construction these are set up to be monitored by the telemetry telemetry
    let (generator_tx, generator_rx) = base_channel_builder
                         .with_rate_percentile(Percentile::p80())
                         .with_capacity(4000)
                         .build();

    let (consumer_tx, consumer_rx) = base_channel_builder
                         .with_avg_rate()
                         .with_capacity(4000)
                         .build();

    let (failure_tx, failure_rx) = base_channel_builder
                        .with_capacity(300)
                        .connects_sidecar()//hint for display
                        .build();

    let (change_tx, change_rx) = base_channel_builder
        // .with_fillled_max()  //TODO: new feature to add
        // .with_fillled_min() //TODO: new feature to add
                        .with_filled_percentile(Percentile::p25())
                        .with_capacity(200)
                        .build();

    let base_actor_builder = graph.actor_builder()//with default OneForOne supervisor....
        .with_mcpu_percentile(Percentile::p80())
        .with_work_percentile(Percentile::p80())
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m64()),AlertColor::Red)
        .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10));

        base_actor_builder.with_name("generator")
             .build_with_exec( move |context| actor::data_generator::run(context
                                                                     , change_rx.clone()
                                                                     , generator_tx.clone())
        );

        base_actor_builder.with_name("approval")
             .build_with_exec( move |context| actor::data_approval::run(context
                                                              , generator_rx.clone()
                                                              , consumer_tx.clone()
                                                              , failure_tx.clone()   )
        );

        base_actor_builder.with_name("feedback")
             .build_with_exec( move |context| actor::data_feedback::run(context
                                                              , failure_rx.clone()
                                                              , change_tx.clone() )
            );

       base_actor_builder.with_name("consumer")
            .build_with_exec( move |context| actor::data_consumer::run(context
                                                        , consumer_rx.clone() )
            );

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

            graph.stop(Duration::from_secs(3));
        });
        graph.block_until_stopped();
    }

}


