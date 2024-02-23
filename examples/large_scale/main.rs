mod args;

use structopt::*;
use log::*;
use futures_timer::Delay;
use args::Args;
use std::time::Duration;
use crate::actor::data_generator::Packet;



// here are the actors that will be used in the graph.
// note that the actors are in a separate module and we must use the structs/enums and
// bring in the behavior functions
mod actor {
    pub mod data_generator;
    pub mod data_router;
    pub mod data_user;
    pub mod data_process;
    #[cfg(test)]
    pub use data_generator::Packet;


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
        info!("waiting for duration: {:?}", Duration::from_secs(opt.duration));
        // note we use (&opt) just to show we did NOT transfer ownership
        Delay::new(Duration::from_secs(opt.duration)).await;
        info!("exit now");
        graph.request_shutdown();
        //TODO:: need some way to wait for clean shutdown to complete
        graph.stop();
    });

    //wait for bastion to cleanly stop all actors
    info!("waiting for all actors to stop");
    Bastion::block_until_stopped();
}

const LEVEL_1: usize = 2; //3
const LEVEL_2: usize = 1; //3
const LEVEL_3: usize = 1; //2
const LEVEL_4: usize = 2; //One will remove all the user filters and loggers

fn build_graph(cli_arg: &Args) -> steady_state::Graph {
    debug!("args: {:?}",&cli_arg);

    //create the mutable graph object
    let mut graph = steady_state::Graph::new(cli_arg.clone());
    graph.start();
    //here are the parts of the channel they both have in common, this could be done
    // in place for each but we are showing here how you can do this for more complex projects.
    let base_channel_builder = graph.channel_builder()
                            .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(5))
                            .with_line_expansion()
                            .with_avg_filled()
                            .with_avg_rate()
                            .with_avg_latency()
        .with_filled_trigger(Trigger::AvgAbove(Filled::p20()),AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p30()),AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p40()),AlertColor::Red)

                            .with_capacity(800);

    let base_actor_builder = graph
                            .actor_builder()
                            .with_avg_mcpu()
                            .with_avg_work()
                           // .with_work_percentile(Percentile::p80())
                           // .with_mcpu_percentile(Percentile::p80())
                            .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10));


    //the above tx rx objects will be owned by the children closures below then cloned
    //each time we need to startup a new child telemetry instance. This way when an telemetry fails
    //we still have the original to clone from.
    //
    //given your supervision strategy create the children to be added to the graph.
    let _ = Bastion::supervisor(|supervisor| {
        let mut supervisor = supervisor.with_strategy(SupervisionStrategy::OneForOne);

        let (btx,brx) = SteadyBundle::new_bundles::<Packet, LEVEL_1>(&base_channel_builder);
        supervisor = supervisor.children(|children| {
            let (c,i) = base_actor_builder
                .with_name("generator")
                .build_with_exec(
                       move |context| actor::data_generator::run(context
                                                  , btx.clone()
                       ));
             c.finish(children,i)

        });

        for x in 0..LEVEL_1 {
            let local_rx = brx[x].clone();
            let (btx,brx) = SteadyBundle::new_bundles::<Packet, LEVEL_2>(&base_channel_builder);
            supervisor = supervisor.children(|children| {
                    let (c,i) = base_actor_builder
                        .with_name("router")
                        .with_name_suffix(x)
                        .build_with_exec(
                               move |context| actor::data_router::run(context
                                                      , LEVEL_1
                                                      , local_rx.clone()
                                                      , btx.clone()
                               )
                        );
                    c.finish(children,i)

                });

            for y in 0..LEVEL_2 {
                let local_rx = brx[y].clone();
                let (btx,brx) = SteadyBundle::new_bundles::<Packet, LEVEL_3>(&base_channel_builder);
                supervisor = supervisor.children(|children| {
                    let (c,i) = base_actor_builder
                        .with_name("router")
                        .with_name_suffix(y)
                        .build_with_exec(move |context| actor::data_router::run(context
                                                                                , LEVEL_1*LEVEL_2
                                                                                , local_rx.clone()
                                                                                , btx.clone()
                        )
                        );
                    c.finish(children,i)
                });

                for z in 0..LEVEL_3 {
                    let local_rx = brx[z].clone();
                    let (btx,brx) = SteadyBundle::new_bundles::<Packet, LEVEL_4>(&base_channel_builder);

                    supervisor = supervisor.children(|children| {
                        let (c,i) = base_actor_builder
                            .with_name("router")
                            .with_name_suffix(z)
                            .build_with_exec(move |context| actor::data_router::run(context
                                                                                    , LEVEL_1*LEVEL_2*LEVEL_3
                                                                                    , local_rx.clone()
                                                                                    , btx.clone()
                            )
                            );
                        c.finish(children,i)


                    });

                    if 1 == LEVEL_4 {
                        let local_rx = brx[0].clone();
                        supervisor = supervisor.children(|children| {
                            let (c,i) = base_actor_builder
                                .with_name("user")
                                .with_name_suffix(z)
                                .build_with_exec(move |context| actor::data_user::run(context
                                                                                      , local_rx.clone()
                                )
                                );
                            c.finish(children,i)
                        });


                    } else {
                        for f in 0..LEVEL_4 {
                            let local_rx = brx[f].clone();

                            let (filter_tx, filter_rx) = base_channel_builder.build();

                            supervisor = supervisor.children(|children| {
                                let (c,i) = base_actor_builder
                                    .with_name("filter")
                                    .with_name_suffix(z)
                                    .build_with_exec(move |context| actor::data_process::run(context
                                                                                             , local_rx.clone()
                                                                                             , filter_tx.clone()
                                    )
                                    );
                                c.finish(children,i)
                            });

                            let (logging_tx, logging_rx) = base_channel_builder.build();

                            supervisor = supervisor.children(|children| {
                                let (c,i) = base_actor_builder
                                    .with_name("logger")
                                    .with_name_suffix(z)
                                    .build_with_exec(move |context| actor::data_process::run(context
                                                                                             , filter_rx.clone()
                                                                                             , logging_tx.clone()
                                    )
                                    );
                                c.finish(children,i)

                            });

                            let (decrypt_tx, decrypt_rx) = base_channel_builder.build();

                            supervisor = supervisor.children(|children| {
                                let (c,i) = base_actor_builder
                                    .with_name("decrypt")
                                    .with_name_suffix(z)
                                    .build_with_exec(move |context| actor::data_process::run(context
                                                                                             , logging_rx.clone()
                                                                                             , decrypt_tx.clone()
                                    )
                                    );
                                c.finish(children,i)
                            });

                            supervisor = supervisor.children(|children| {
                                let (c,i) = base_actor_builder
                                    .with_name("user")
                                    .with_name_suffix(z)
                                    .build_with_exec(move |context| actor::data_user::run(context
                                                                                          , decrypt_rx.clone()
                                         )
                                    );

                                c.finish(children,i)

                            });
                        }
                    }

                }
            }

        }

        supervisor
    }).expect("OneForOne supervisor creation error.");

    graph.init_telemetry();
    graph
}



#[cfg(test)]
mod tests {
    use bastion::prelude::{Distributor, SendError};
    use bastion::run;
    use super::*;

    /*
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



            let answer_generator: Result<&str, SendError> = distribute_generator.request(to_send).await.unwrap();
            assert_eq!("ok",answer_generator.unwrap());

            let answer_consumer: Result<&str, SendError> = distribute_consumer.request(expected_message).await.unwrap();
            assert_eq!("ok",answer_consumer.unwrap());

            graph.stop();
        });
        Bastion::block_until_stopped();
    }
    //  */

}


