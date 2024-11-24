mod args;

use std::thread::sleep;
use structopt::*;
#[allow(unused_imports)]
use log::*;
use args::Args;
use std::time::Duration;
use steady_state::*;
use steady_state::actor_builder::{ActorTeam, Threading};


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
fn main() {
    // a typical begging by fetching the command line args and starting logging
    let opt = Args::from_args();

    if let Err(e) = init_logging(&opt.loglevel) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }
    let large_scale = 45; //47-> 1056  40 ->888 // 13->400  // (24* (N+3)) TODO: run profiler and confirm threads needed !!
    let mut graph = build_graph::<3,2,2,2>(GraphBuilder::for_production()
        .with_telemtry_production_rate_ms(4000)
        .build(opt.clone()),false,false,large_scale); // (24* (N+3)) -> 400
    
    graph.start();

    {   //remove this block to run forever.
        sleep(Duration::from_secs(opt.duration));
        graph.request_stop();
    }

    graph.block_until_stopped(Duration::from_secs(80));

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

    let mut actor_team = ActorTeam::new(&graph);
    let mut thread_top = if spawn_a {
        Threading::Spawn
    } else {
        Threading::Join(&mut actor_team)
    };


    let (btx,brx) = base_channel_builder.build_as_bundle::<_, { LEVEL_1 }>();

            base_actor_builder
                .with_name("Generator")
                .build(
                       move |context| actor::data_generator::run(context
                                                  , btx.clone()
                       )
                       , &mut thread_top
                );


        for x in 0..LEVEL_1 {

            let local_rx = brx[x].clone();
            let (btx,brx) = base_channel_builder.build_as_bundle::<_, { LEVEL_2 }>();
                base_actor_builder
                    .with_name_and_suffix("RouterA",x)
                    .build(
                           move |context| actor::data_router::run(context
                                                                  , LEVEL_1
                                                                  , local_rx.clone()
                                                                  , btx.clone()
                           )
                          , &mut thread_top
                    );


            for y in 0..LEVEL_2 {

                let local_rx = brx[y].clone();
                let (btx,brx) = base_channel_builder.build_as_bundle::<_, { LEVEL_3 }>();
                base_actor_builder
                    .with_name_and_suffix("RouterB", route_b_count)
                    .build(move |context| actor::data_router::run(context
                                                                  , LEVEL_1 * LEVEL_2
                                                                  , local_rx.clone()
                                                                  , btx.clone()
                                     )
                                 , &mut thread_top
                    );

                for z in 0..LEVEL_3 {

                    let local_rx = brx[z].clone();
                    let (btx,brx) = base_channel_builder.build_as_bundle::<_, { LEVEL_4 }>();
                        base_actor_builder
                            .with_name_and_suffix("RouterC", route_c_count)
                            .build(move |context| actor::data_router::run(context
                                                                          , LEVEL_1 * LEVEL_2 * LEVEL_3
                                                                          , local_rx.clone()
                                                                          , btx.clone()
                                          )
                                         , &mut thread_top
                            );

                    if 1 == LEVEL_4 {
                        let mut actor_linedance_tream = ActorTeam::new(&graph);
                        if let Threading::Join(ref mut team) = thread_top {
                            team.transfer_back_to(&mut actor_linedance_tream);
                        }
                        let mut thread = if spawn_b {
                            Threading::Spawn
                        } else {
                            Threading::Join(&mut actor_linedance_tream)
                        };

                        let local_rx = brx[0].clone();
                            base_actor_builder
                                .with_name_and_suffix("User",user_count)
                                .build(move |context| actor::data_user::run(context
                                                                                  , local_rx.clone()
                                             )
                                            , &mut thread
                                );
                        actor_linedance_tream.spawn();
                        user_count += 1;

                    } else {

                        for f in 0..LEVEL_4 {
                            let mut group_line = ActorTeam::new(&graph  );

                            if let Threading::Join(ref mut team) = thread_top {
                                team.transfer_back_to(&mut group_line);
                            }

                            let mut thread = if spawn_b {
                                Threading::Spawn
                            } else {
                                Threading::Join(&mut group_line)
                            };

                            let local_rx = brx[f].clone();

                            let (filter_tx, filter_rx) = base_channel_builder.build();
                                base_actor_builder
                                    .with_name_and_suffix("Logger",user_count)
                                    .build(move |context| actor::data_process::run(context
                                                                                        , local_rx.clone()
                                                                                        , filter_tx.clone()
                                                  )
                                                 , &mut thread
                                    );

                            let mut input_rx = filter_rx;
                            let output_rx = {
                                //TODO: we need to profile this, also slow the frame rate
                                let mut count = large_scale_test;
                                loop {
                                    let (logging_tx, logging_rx) = base_channel_builder.build();

                                    base_actor_builder
                                        .with_name_and_suffix("Filter", (100*count)+user_count)
                                        .build(move |context| actor::data_process::run(context
                                                                                            , input_rx.clone()
                                                                                            , logging_tx.clone()
                                        )
                                                    , &mut thread
                                        );

                                    count = count - 1;
                                    if 0==count {
                                        break logging_rx;
                                    }
                                    input_rx = logging_rx;
                                }
                            };


                            let (decrypt_tx, decrypt_rx) = base_channel_builder.build();
                            base_actor_builder
                                    .with_name_and_suffix("Decrypt",user_count)
                                    .build(move |context| actor::data_process::run(context
                                                                                        , output_rx.clone()
                                                                                        , decrypt_tx.clone()
                                                    )
                                                 , &mut thread
                                    );

                                base_actor_builder
                                    .with_name_and_suffix("User",user_count)
                                    .build(move |context| actor::data_user::run(context
                                                                                     , decrypt_rx.clone()
                                                    )
                                                 , &mut thread
                                    );

                            group_line.spawn();
                            user_count += 1;

                        }
                    }
                    route_c_count +=1;
                }
                route_b_count += 1;
            }
        }
    let _x = actor_team.spawn();
    //trace!("remaining actors in global group: {:?}",_x);

    graph
}

// TODO: show it some love,  we need a test example much much larger. lets target 64K actors. 16K at a minimum

#[cfg(test)]
mod large_tests {
    use std::ops::DerefMut;
    use std::time::Duration;
    use bytes::Bytes;
    use futures_timer::Delay;
    use futures_util::future::join_all;
    use isahc::ReadResponseExt;
    use log::{error, info, trace};
    use steady_state::{ActorName, GraphBuilder};
    use crate::actor::data_generator::Packet;
    use crate::args::Args;
    use crate::build_graph;

    #[cfg(test)]
    #[async_std::test]
    async fn test_large_graph() {

        let test_ops = Args {
            duration: 21,
            loglevel: "debug".to_string(),
            gen_rate_micros: 100,
        };

        let graph = GraphBuilder::for_testing()
                       .with_telemtry_production_rate_ms(500)
                       .with_telemetry_metric_features(true)
                       .build(test_ops);

        //do not build super large unit test or we will not have enough threads on
        //the github workflow action builder.
        const LEVEL_1:usize = 2;
        const LEVEL_2:usize = 1;
        const LEVEL_3:usize = 1;
        const LEVEL_4:usize = 2;

        //TODO: when we set first to false the unit test fails due to unclean shutdown (needs investigation)
        let mut graph = build_graph::<LEVEL_1,LEVEL_2,LEVEL_3,LEVEL_4>(graph,true,false,1);
        const KNOWN_USERS:usize = LEVEL_1 * LEVEL_2 * LEVEL_3 * LEVEL_4;

        //info!("finished building graph");
        graph.start_with_timeout(Duration::from_secs(4));
        //info!("after graph start");
        {
            let mut guard = graph.sidechannel_director().await;
            let g = guard.deref_mut();
            assert!(g.is_some(), "Internal error, this is a test so this back channel should have been created already");
            if let Some(plane) = g {

                 for i in 0..KNOWN_USERS {
                     let to_send = Packet {
                         route: i as u16,
                         data: Bytes::from_static(&[0u8; 62]),
                     };
                     //trace!("send test packet {}",i);
                     let response = plane.call_actor(Box::new(to_send), ActorName::new("Generator", None)).await;
                     if let Some(r) = response {
                            //trace!("generator response: {:?}", r.downcast_ref::<String>());
                            assert_eq!("ok", r.downcast_ref::<String>().expect("bad type"));

                     } else {
                         error!("bad response from generator: {:?}", response);
                        // panic!("bad response from generator: {:?}", response);
                     }
                 }
            }
        }

        //wait for one page of telemetry
        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms()*10)).await;

       //hit the telemetry site and validate if it returns
        // this test will only work if the feature is on
       // hit 127.0.0.1:9100/metrics using isahc
        match isahc::get("http://127.0.0.1:9100/metrics") {
            Ok(mut response) => {
                assert_eq!(200, response.status().as_u16());
                let _body = response.text().expect("body text");
                //info!("metrics: {}", body); //TODO: add more checks
            }
            Err(e) => {
                info!("failed to get metrics: {:?}", e);
                // //this is only an error if the feature is not on
                // #[cfg(feature = "prometheus_metrics")]
                // {
                //     panic!("failed to get metrics: {:?}", e);
                // }
            }
        };
        match isahc::get("http://127.0.0.1:9100/graph.dot") {
            Ok(mut response) => {
                assert_eq!(200, response.status().as_u16());
                let _body = response.text().expect("body text");
               //  info!("metrics: {}", body); //TODO: add more checks
            }
            Err(e) => {
                info!("failed to get metrics: {:?}", e);
                // //this is only an error if the feature is not on
                 #[cfg(any(feature = "telemetry_server_builtin",feature = "telemetry_server_cdn"))]
                 {
                     //panic!("failed to get metrics: {:?}", e);
                 }
            }
        };


        {
            let mut guard = graph.sidechannel_director().await;// mut drop this guard before shutdown request
            let g = guard.deref_mut();
            assert!(g.is_some(), "Internal error, this is a test so this back channel should have been created already");
            let mut tasks = Vec::with_capacity(KNOWN_USERS);
            if let Some(plane) = g {
                for i in 0..KNOWN_USERS {
                    let expected_message = Packet {
                        route: i as u16,
                        data: Bytes::from_static(&[0u8; 62]),
                    };
                    let name = ActorName::new("User", Some(i));
                    // trace!("sent: {:?}",name);
                    let fut = plane.call_actor(Box::new(expected_message), name);
                    tasks.push(async move {
                        match fut.await {
                            Some(r) => {
                                trace!("user response: {:?} {:?}", r.downcast_ref::<String>(),i);
                                assert_eq!("ok", r.downcast_ref::<String>().expect("bad type"));
                            },
                            None => {
                                error!("bad response from generator");
                                panic!("bad response from generator");
                            }
                        }
                    });
                }
                // Wait for all tasks to complete
                //due to threading this is important to avoid deadlocks
                //info!(" join on tasks {:?} ",tasks.len());
                join_all(tasks).await;
            }
        }
        //we got all users responses here why should user hang after this?

        // //if you need to debug this test you can bump this 1 sec up to 300 which will give you time
        // //to open your browser to http://0.0.0.0:9100 and observe where the data is stuck.
        // Delay::new(Duration::from_secs(2)).await;
        graph.request_stop();
       // Delay::new(Duration::from_secs(5)).await; //this delay seems critical for testing but I am not sure why
        graph.block_until_stopped(Duration::from_secs(52));

    }
}


