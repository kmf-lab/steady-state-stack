mod args;

use std::thread::sleep;
use structopt::*;
use log::*;
use args::Args;
use std::time::Duration;

mod actor {
    pub mod data_generator;
    #[cfg(test)]
    pub use data_generator::WidgetInventory;
    pub mod data_approval;
    #[cfg(test)]
    pub use data_approval::ApprovedWidgets;
    pub mod data_consumer;
    pub mod data_feedback;
}
#[cfg(test)]
use crate::actor::*;
use steady_state::*;
use steady_state::actor_builder::{ActorTeam, MCPU, Percentile};
use steady_state::channel_builder::Filled;


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

    if let Err(e) = init_logging(&opt.loglevel) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }

    //TODO: 2025 add this as a feature to the code generator
    let service_executable_name = "simple_widgets";
    let service_user = "simple_widgets_user";
    let systemd_command = process_systemd_commands(  opt.systemd_action()
                                                   , opt.to_cli_string(service_executable_name)
                                                   , service_executable_name
                                                   , service_user);

    if !systemd_command {
        let mut graph = build_simple_widgets_graph(Graph::new(opt.clone()));

        graph.start();
        {  //remove this block to run forever.
            if opt.duration > 0 {
                sleep(Duration::from_secs(opt.duration));
                graph.request_stop();
            }
        }
        graph.block_until_stopped(Duration::from_secs(2));
    }
}

fn build_simple_widgets_graph(mut graph: Graph) -> Graph {
    let mut team = ActorTeam::new();

    //here are the parts of the channel they both have in common, this could be done
    // in place for each but we are showing here how you can do this for more complex projects.
    let base_channel_builder = graph.channel_builder()
        .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
        .with_labels(&["widgets"], true)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50())
                             , AlertColor::Red)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p20())
                             , AlertColor::Yellow)
        .with_type()
        .with_line_expansion(1.0f32);

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
        .connects_sidecar() //hint for display
        .build();

    let (change_tx, change_rx) = base_channel_builder
        .with_filled_max()
        .with_filled_min()
        .with_filled_percentile(Percentile::p25())
        .with_capacity(200)
        .build();

    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor....
        .with_mcpu_percentile(Percentile::p80())
        .with_work_percentile(Percentile::p80())
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m64()), AlertColor::Red)
        .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10));

    base_actor_builder.with_name("generator")
        .build_join(move |context| actor::data_generator::run(context
                                                              , change_rx.clone()
                                                              , generator_tx.clone())
                      , &mut team
        );

    base_actor_builder.with_name("approval")
        .build_join(move |context| actor::data_approval::run(context
                                                             , generator_rx.clone()
                                                             , consumer_tx.clone()
                                                             , failure_tx.clone())
                    , &mut team
        );

    base_actor_builder.with_name("feedback")
        .build_join(move |context| actor::data_feedback::run(context
                                                             , failure_rx.clone()
                                                             , change_tx.clone())
                    , &mut team
        );

    base_actor_builder.with_name("consumer")
        .build_join(move |context| actor::data_consumer::run(context
                                                             , consumer_rx.clone())
                    , &mut team
        );
    team.spawn();

    graph
}

#[cfg(test)]
mod simple_widget_tests {
    use std::ops::DerefMut;
    use super::*;

    #[cfg(test)]
    #[async_std::test]
    async fn test_simple_widget_graph() {

        let test_ops = Args {
            duration: 21,
            loglevel: "debug".to_string(),
            gen_rate_micros: 100,
            systemd_install: false,
            systemd_uninstall: false,
        };

        let graph = Graph::new_test(test_ops);

        let mut graph = build_simple_widgets_graph(graph);
        graph.start();

        let mut guard = graph.sidechannel_director().await;
        let g = guard.deref_mut();
        assert!(g.is_some(), "Internal error, this is a test so this back channel should have been created already");
        if let Some(plane) = g {
            let to_send = WidgetInventory {
                count: 42,
                _payload: 0
            };
            let response = plane.node_call(Box::new(to_send), "generator").await;
            if let Some(_) = response {
                let expected_message = ApprovedWidgets {
                    original_count: 42,
                    approved_count: 21,
                };
                let response = plane.node_call(Box::new(expected_message), "consumer").await;
                assert_eq!("ok", response.expect("no response")
                                        .downcast_ref::<String>().expect("bad type"));

            } else {
                panic!("bad response from generator: {:?}", response);
            }
        }
        drop(guard);

        graph.request_stop();
        //if you make this timeout very large you will have plenty of time to debug steam through
        //this test method if you like.
        graph.block_until_stopped(Duration::from_secs(3));

    }
}


