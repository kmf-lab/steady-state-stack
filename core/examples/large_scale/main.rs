mod args;

use std::thread::sleep;
#[allow(unused_imports)]
use log::*;
use args::Args;
use std::time::Duration;
use steady_state::*;
use steady_state::actor_builder::{ScheduleAs};


// here are the actors that will be used in the graph.
// note that the actors are in a separate module and we must use the structs/enums and
// bring in the behavior functions
mod actor {
    pub mod data_generator;
    pub mod data_router;
    pub mod data_user;
    pub mod data_process;
}


//use steady_state::*;
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // a typical begging by fetching the command line args and starting logging
    let opt = Args::parse();

    if let Err(e) = init_logging(opt.loglevel) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }
    let large_scale = 45; //47-> 1056  40 ->888 // 13->400  // (24* (N+3)) TODO: run profiler and confirm threads needed !!
    let mut graph = build_graph::<3,2,2,2>(GraphBuilder::for_production()
        .with_telemtry_production_rate_ms(4000)
        .build(opt.clone()),false,false,large_scale); // (24* (N+3)) -> 400
    
    graph.start();

    {   //remove this block to run forever.
        sleep(Duration::from_secs(opt.duration));
        graph.request_shutdown();
    }

    graph.block_until_stopped(Duration::from_secs(80))

}


fn build_graph<const LEVEL_1: usize,
               const LEVEL_2: usize,
               const LEVEL_3: usize,
               const LEVEL_4: usize
             >(mut graph: Graph, spawn_a: bool, spawn_b: bool, large_scale_test: usize) -> Graph {

    //here are the parts of the channel they both have in common, this could be done
    // in place for each but we are showing here how you can do this for more complex projects.
    let base_channel_builder = graph.channel_builder()
                            .with_line_expansion(1.0f32)
                            .with_avg_filled()
                            .with_avg_rate()
                            //.with_filled_max()  //TODO: not yet implemented
                            //.with_filled_min()  //TODO: not yet implemented
                            .with_filled_standard_deviation(StdDev::one())
                            .with_avg_latency()        
        .with_filled_trigger(Trigger::AvgAbove(Filled::p20()),AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p40()),AlertColor::Red)
                            .with_capacity(4000);

    let base_actor_builder = graph
                            .actor_builder()
                            .with_mcpu_avg()
                            .with_thread_info()
                           // .with_work_percentile(Percentile::p80())
                           // .with_mcpu_percentile(Percentile::p80())
                            .with_load_avg();


    let mut user_count:usize = 0;
    let mut route_b_count:usize = 0;
    let mut route_c_count:usize = 0;

    let mut a_troupe = if spawn_a {None} else {Some(graph.actor_troupe())};

    let (btx,brx) = base_channel_builder.build_channel_bundle::<_, { LEVEL_1 }>();

            base_actor_builder
                .with_name("Generator")
                .build(
                       move |context| actor::data_generator::run(context
                                                  , btx.clone()
                       )
                       , ScheduleAs::dynamic_schedule(&mut a_troupe)
                );


        for x in 0..LEVEL_1 {

            let local_rx = brx[x].clone();
            let (btx,brx) = base_channel_builder.build_channel_bundle::<_, { LEVEL_2 }>();
                base_actor_builder
                    .with_name_and_suffix("RouterA",x)
                    .build(
                           move |context| actor::data_router::run(context
                                                                  , LEVEL_1
                                                                  , local_rx.clone()
                                                                  , btx.clone()
                           )
                          , ScheduleAs::dynamic_schedule(&mut a_troupe)
                    );


            for y in 0..LEVEL_2 {

                let local_rx = brx[y].clone();
                let (btx,brx) = base_channel_builder.build_channel_bundle::<_, { LEVEL_3 }>();
                base_actor_builder
                    .with_name_and_suffix("RouterB", route_b_count)
                    .build(move |context| actor::data_router::run(context
                                                                  , LEVEL_1 * LEVEL_2
                                                                  , local_rx.clone()
                                                                  , btx.clone()
                                     )
                                 , ScheduleAs::dynamic_schedule(&mut a_troupe)
                    );

                for z in 0..LEVEL_3 {

                    let local_rx = brx[z].clone();
                    let (btx,brx) = base_channel_builder.build_channel_bundle::<_, { LEVEL_4 }>();
                        base_actor_builder
                            .with_name_and_suffix("RouterC", route_c_count)
                            .build(move |context| actor::data_router::run(context
                                                                          , LEVEL_1 * LEVEL_2 * LEVEL_3
                                                                          , local_rx.clone()
                                                                          , btx.clone()
                                          )
                                         , ScheduleAs::dynamic_schedule(&mut a_troupe)
                            );

                    if 1 == LEVEL_4 {
                        let mut actor_linedance_tream = graph.actor_troupe();
                        if let Some(ref mut troupe) = a_troupe {
                            troupe.transfer_back_to(&mut actor_linedance_tream);
                        }

                        let mut b_troupe = if spawn_b {None} else {Some(actor_linedance_tream)};
                        let state = new_state();
                        let local_rx = brx[0].clone();
                            base_actor_builder
                                .with_name_and_suffix("User",user_count)
                                .build(move |context| actor::data_user::run(context
                                                                                 , local_rx.clone()
                                                                                 , state.clone()
                                             )
                                            , ScheduleAs::dynamic_schedule(&mut b_troupe)
                                );
                        user_count += 1;

                    } else {

                        for f in 0..LEVEL_4 {
                            let mut group_line = graph.actor_troupe();

                            if let Some(ref mut troupe) = a_troupe {
                                troupe.transfer_back_to(&mut group_line);
                            }
                            let mut b_troupe = if spawn_b {None} else {Some(group_line)};

                            let local_rx = brx[f].clone();

                            let (filter_tx, filter_rx) = base_channel_builder.build_channel();
                                base_actor_builder
                                    .with_name_and_suffix("Logger",user_count)
                                    .build(move |context| actor::data_process::run(context
                                                                                        , local_rx.clone()
                                                                                        , filter_tx.clone()
                                                  )
                                                 , ScheduleAs::dynamic_schedule(&mut b_troupe)
                                    );

                            let mut input_rx = filter_rx;
                            let output_rx = {
                                //TODO: we need to profile this, also slow the frame rate
                                let mut count = large_scale_test;
                                loop {
                                    let (logging_tx, logging_rx) = base_channel_builder.build_channel();

                                    base_actor_builder
                                        .with_name_and_suffix("Filter", (100*count)+user_count)
                                        .build(move |context| actor::data_process::run(context
                                                                                            , input_rx.clone()
                                                                                            , logging_tx.clone()
                                        )
                                                    , ScheduleAs::dynamic_schedule(&mut b_troupe)
                                        );

                                    count -= 1;
                                    if 0==count {
                                        break logging_rx;
                                    }
                                    input_rx = logging_rx;
                                }
                            };


                            let (decrypt_tx, decrypt_rx) = base_channel_builder.build_channel();
                            base_actor_builder
                                    .with_name_and_suffix("Decrypt",user_count)
                                    .build(move |context| actor::data_process::run(context
                                                                                        , output_rx.clone()
                                                                                        , decrypt_tx.clone()
                                                    )
                                                 , ScheduleAs::dynamic_schedule(&mut b_troupe)
                                    );
                            let state = new_state();
                            base_actor_builder
                                    .with_name_and_suffix("User",user_count)
                                    .build(move |context| actor::data_user::run(context
                                                                                , decrypt_rx.clone()
                                                                                , state.clone()
                                                    )
                                                 , ScheduleAs::dynamic_schedule(&mut b_troupe)
                                    );
                            user_count += 1;

                        }
                    }
                    route_c_count +=1;
                }
                route_b_count += 1;
            }
        }
    graph
}

// #[cfg(test)]
// mod large_tests {
//     use std::ops::DerefMut;
//     use crate::{Args, LogLevel, build_graph};
//     use crate::actor::data_generator::Packet;
//     use steady_state::{ActorName, GraphBuilder};
//     use bytes::Bytes;
//     use futures_timer::Delay;
//     use futures_util::future::join_all;
//     use isahc::ReadResponseExt;
//     use log::{error, info};
//     use std::time::Duration;
//
//     #[async_std::test]
//     async fn test_large_graph() {
//         // Define test options
//         let test_ops = Args {
//             duration: 21,
//             loglevel: LogLevel::Debug,
//             gen_rate_micros: 100,
//         };
//
//         // Build the graph with telemetry enabled
//         let graph = GraphBuilder::for_testing()
//             .with_telemtry_production_rate_ms(500)
//             .with_telemetry_metric_features(true)
//             .build(test_ops);
//
//         // Define graph levels for a small-scale test to avoid thread exhaustion
//         const LEVEL_1: usize = 2;
//         const LEVEL_2: usize = 1;
//         const LEVEL_3: usize = 1;
//         const LEVEL_4: usize = 2;
//         const KNOWN_USERS: usize = LEVEL_1 * LEVEL_2 * LEVEL_3 * LEVEL_4;
//
//         // Build and start the graph
//         let mut graph = build_graph::<LEVEL_1, LEVEL_2, LEVEL_3, LEVEL_4>(graph, true, false, 1);
//         graph.start_with_timeout(Duration::from_secs(4));
//
//         // Send packets to the Generator actor
//         {
//             let mut guard = graph.sidechannel_director().await;
//             if let Some(plane) = guard.deref_mut() {
//                 for i in 0..KNOWN_USERS {
//                     let packet = Packet {
//                         route: i as u16,
//                         data: Bytes::from_static(&[0u8; 62]),
//                     };
//                     let response = plane.call_actor(Box::new(packet), ActorName::new("Generator", None)).await;
//                     if let Some(r) = response {
//                         assert_eq!("ok", r.downcast_ref::<String>().expect("Response should be a String"));
//                     } else {
//                         error!("No response from Generator for user {}", i);
//                         panic!("Generator failed to respond for user {}", i);
//                     }
//                 }
//             } else {
//                 panic!("Sidechannel director not available");
//             }
//         }
//
//         // Wait for telemetry to be produced
//         Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms() * 10)).await;
//
//         // Verify telemetry endpoints
//         verify_telemetry_endpoints();
//
//         // Check responses from User actors
//         {
//             let mut guard = graph.sidechannel_director().await;
//             if let Some(plane) = guard.deref_mut() {
//                 let mut tasks = Vec::with_capacity(KNOWN_USERS);
//                 for i in 0..KNOWN_USERS {
//                     let expected_packet = Packet {
//                         route: i as u16,
//                         data: Bytes::from_static(&[0u8; 62]),
//                     };
//                     let user_name = ActorName::new("User", Some(i));
//                     let fut = plane.call_actor(Box::new(expected_packet), user_name);
//                     tasks.push(async move {
//                         match fut.await {
//                             Some(r) => {
//                                 let response_str = r.downcast_ref::<String>().expect("Response should be a String");
//                                 if response_str != "ok" {
//                                     error!("User {} returned '{}', expected 'ok'", i, response_str);
//                                     panic!("User {} returned '{}', expected 'ok'", i, response_str);
//                                 }
//                             }
//                             None => {
//                                 error!("No response from User {}", i);
//                                 panic!("No response from User {}", i);
//                             }
//                         }
//                     });
//                 }
//                 join_all(tasks).await;
//             } else {
//                 panic!("Sidechannel director not available");
//             }
//         }
//
//         // Stop the graph and wait for shutdown
//         graph.request_shutdown();
//         graph.block_until_stopped(Duration::from_secs(21));
//     }
//
//     /// Verifies the telemetry endpoints (/metrics and /graph.dot).
//     fn verify_telemetry_endpoints() {
//         // Check /metrics endpoint
//         match isahc::get("http://127.0.0.1:9100/metrics") {
//             Ok(mut response) => {
//                 assert_eq!(200, response.status().as_u16());
//                 let _body = response.text().expect("Failed to read metrics response");
//                 // Add specific checks for metrics content if needed
//             }
//             Err(e) => {
//                 info!("Metrics endpoint unavailable: {:?}", e);
//             }
//         }
//
//         // Check /graph.dot endpoint
//         match isahc::get("http://127.0.0.1:9100/graph.dot") {
//             Ok(mut response) => {
//                 assert_eq!(200, response.status().as_u16());
//                 let _body = response.text().expect("Failed to read graph.dot response");
//                 // Add specific checks for graph.dot content if needed
//             }
//             Err(e) => {
//                 info!("Graph.dot endpoint unavailable: {:?}", e);
//             }
//         }
//     }
// }