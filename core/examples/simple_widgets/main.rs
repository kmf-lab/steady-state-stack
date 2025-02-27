mod args;
use steady_state::{Percentile, MCPU};
use steady_state::StdDev;
use std::sync::Arc;
use std::thread::sleep;
#[allow(unused_imports)]
use log::*;
use args::Args;
use std::time::Duration;
use futures_util::lock::Mutex;

mod actor {
    pub mod data_generator;
    #[cfg(test)]
    pub use data_generator::WidgetInventory;
    pub mod data_approval;
    pub mod data_consumer;
    pub mod data_feedback;
}

use steady_state::channel_builder::Filled;
use crate::actor::data_consumer::InternalState;
use clap::*;

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
    let opt = Args::parse();

    if let Err(e) = steady_state::init_logging(&opt.loglevel) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }


    //TODO: 2025 add this as a feature to the code generator
    let service_executable_name = "simple_widgets";
    let service_user = "simple_widgets_user";
    let systemd_command = steady_state::SystemdBuilder::process_systemd_commands(  opt.systemd_action()
                                                   , service_executable_name
                                                   , service_user);

    if !systemd_command {
        let (mut graph, _state) = build_simple_widgets_graph(steady_state::GraphBuilder::for_production().build(opt.clone()));

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

fn build_simple_widgets_graph(mut graph: steady_state::Graph) -> (steady_state::Graph, Arc<Mutex<InternalState>>) {

    //here are the parts of the channel they both have in common, this could be done
    // in place for each but we are showing here how you can do this for more complex projects.
    let base_channel_builder = graph.channel_builder()
        .with_labels(&["widgets"], true)
        .with_filled_trigger(steady_state::Trigger::AvgAbove(Filled::p50())
                             , steady_state::AlertColor::Red)
        .with_filled_trigger(steady_state::Trigger::AvgAbove(Filled::p20())
                             , steady_state::AlertColor::Yellow)
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
        .with_type()
        .with_line_expansion(0.1f32)
        .with_avg_rate()
        .with_avg_latency()
        .with_filled_standard_deviation(StdDev::two())
        .with_filled_percentile(Percentile::p25())
        .with_capacity(200)
        .build();

    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor....
        .with_mcpu_percentile(Percentile::p80())
        .with_load_percentile(Percentile::p80())
        .with_mcpu_avg()
        .with_load_avg()
        .with_mcpu_trigger(steady_state::Trigger::AvgAbove(MCPU::m64()), steady_state::AlertColor::Red)
        .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10));

    base_actor_builder.with_name("generator")
        .build_spawn(move |context| actor::data_generator::run(context
                                                              , change_rx.clone()
                                                              , generator_tx.clone())
                     
        );

    base_actor_builder.with_name("approval")
        .build_spawn(move |context| actor::data_approval::run(context
                                                             , generator_rx.clone()
                                                             , consumer_tx.clone()
                                                             , failure_tx.clone())
                  
        );

    base_actor_builder.with_name("feedback")
        .build_spawn(move |context| actor::data_feedback::run(context
                                                             , failure_rx.clone()
                                                             , change_tx.clone())
                 
        );

    let state = Arc::new(Mutex::new(InternalState::new()));

    let actor_state = state.clone();
    base_actor_builder.with_name("consumer")
        .build_spawn(move |context| actor::data_consumer::run(context
                                                             , consumer_rx.clone(),
                                                             actor_state.clone())
                 
        );

    (graph,state)
}

#[cfg(test)]
mod simple_widget_tests {
    // use std::ops::DerefMut;
    // use futures_timer::Delay;
    // use isahc::{Body, Error, ReadResponseExt, Response};
    // use super::*;

    // #[cfg(test)]
    // #[async_std::test]
    // async fn test_simple_widget_graph() {
    //
    //     let test_ops = Args {
    //         duration: 21,
    //         loglevel: "debug".to_string(),
    //         gen_rate_micros: 100,
    //         systemd_install: false,
    //         systemd_uninstall: false,
    //     };
    //
    //     let graph = Graph::new_test_with_telemetry(test_ops);
    //
    //     let (mut graph, state) = build_simple_widgets_graph(graph);
    //     graph.start();
    //
    //     {
    //         let mut guard = graph.sidechannel_director().await;
    //         let g = guard.deref_mut();
    //         assert!(g.is_some(), "Internal error, this is a test so this back channel should have been created already");
    //         if let Some(plane) = g {
    //             let to_send = WidgetInventory {
    //                 count: 42,
    //                 _payload: 0
    //             };
    //             let response = plane.node_call(Box::new(to_send), ActorName::new("generator",None)).await;
    //             if let Some(_) = response {
    //                 let expected_message = ApprovedWidgets {
    //                     original_count: 42,
    //                     approved_count: 21,
    //                 };
    //                 let response = plane.node_call(Box::new(expected_message), ActorName::new("consumer",None)).await;
    //                 assert_eq!("ok", response.expect("no response")
    //                     .downcast_ref::<String>().expect("bad type"));
    //             } else {
    //                 panic!("bad response from generator: {:?}", response);
    //             }
    //         }
    //     }
    //     //wait for one page of telemetry
    //     Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms()*10)).await;
    //
    //
    //     //hit the telemetry site and validate if it returns
    //     // this test will only work if the feature is on
    //     // hit 127.0.0.1:9100/metrics using isahc
    //     match isahc::get("http://127.0.0.1:9100/metrics") {
    //         Ok(mut response) => {
    //             assert_eq!(200, response.status().as_u16());
    //
    //             let body:String = response.text().expect("body text");
    //             let _ =response.consume();
    //
    //             trace!("body: {}", body);
    //             let _ = body.lines().find(|line| line.contains("inflight{widgets=\"T\", type=\"ChangeRequest\", from=\"feedback\", to=\"generator\"} 0")).expect("expected line not found");
    //         }
    //         Err(e) => {
    //             info!("failed to get metrics: {:?}", e);
    //             //this is only an error if the feature is not on
    //             #[cfg(feature = "prometheus_metrics")]
    //             {
    //                 warn!("failed to get metrics: {:?}", e);
    //                 panic!("failed to get metrics: {:?}", e);
    //             }
    //         }
    //     };
    //     match isahc::get("http://127.0.0.1:9100/graph.dot") {
    //         Ok(mut response) => {
    //             assert_eq!(200, response.status().as_u16());
    //
    //             let body = response.text().expect("body text");
    //             let _ =response.consume();
    //
    //             trace!("body: {}", body);
    //
    //             let _ = body.lines().find(|line| line.contains("\"feedback\" -> \"generator\" [label=")).expect("expected line not found");
    //
    //         }
    //         Err(e) => {
    //             info!("failed to get metrics: {:?}", e);
    //             // //this is only an error if the feature is not on
    //             #[cfg(any(feature = "telemetry_server_builtin",feature = "telemetry_server_cdn"))]
    //             {
    //                 warn!("failed to get metrics: {:?}", e);
    //                 panic!("failed to get metrics: {:?}", e);
    //             }
    //         }
    //     };
    //
    //     //trace!("state {:?}",state);
    //     let other_files = ["images/preview-icon.svg"
    //                       ,"images/refresh-time-icon.svg"
    //                       ,"images/spinner.gif"
    //                       ,"images/user-icon.svg"
    //                       ,"images/zoom-in-icon.svg"
    //                       ,"images/zoom-in-icon-disabled.svg"
    //                       ,"images/zoom-out-icon.svg"
    //                       ,"images/zoom-out-icon-disabled.svg"
    //                       ,"dot-viewer.css"
    //                       ,"dot-viewer.js"
    //                       ,"webworker.js"
    //     ];
    //     for file in other_files.iter() {
    //         match isahc::get(format!("http://127.0.0.1:9100/{}",file )) {
    //             Ok(mut response) => {
    //                 assert_eq!(200, response.status().as_u16());
    //                 let _ =response.consume();
    //
    //             }
    //             Err(e) => {
    //                 #[cfg(any(feature = "telemetry_server_builtin",feature = "telemetry_server_cdn"))]
    //                 {
    //                     warn!("failed to fetch {}: {:?}", file, e);
    //                     error!("failed to fetch {}: {:?}", file, e);
    //                 }
    //             }
    //         }
    //     }
    //
    //
    //     graph.request_stop();
    //     //if you make this timeout very large you will have plenty of time to debug steam through
    //     //this test method if you like.
    //     assert!(graph.block_until_stopped(Duration::from_secs(11)));
    //
    //     //trace!("state {:?}",state);
    //
    // }
}


