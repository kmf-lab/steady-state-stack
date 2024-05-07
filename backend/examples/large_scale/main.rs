mod args;

use std::thread::sleep;
use structopt::*;
use log::*;
use args::Args;
use std::time::{Duration, Instant};
use steady_state::*;
use steady_state::actor_builder::ActorGroup;


// here are the actors that will be used in the graph.
// note that the actors are in a separate module and we must use the structs/enums and
// bring in the behavior functions
mod actor {
    pub mod data_generator;
    pub mod data_router;
    pub mod data_user;
    pub mod data_process;



}
#[cfg(test)]


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
   // let start = Instant::now();
    let mut graph = build_graph(&opt); //graph is built here and tested below in the test section.
    //println!("graph built in {:?}", start.elapsed());

    graph.start();
   // println!("graph started in {:?}", start.elapsed());


    {   //remove this block to run forever.
        sleep(Duration::from_secs(opt.duration));
        graph.stop();
    }


    graph.block_until_stopped(Duration::from_secs(80));

}

pub(crate) const LEVEL_1: usize = 2; //3
const LEVEL_2: usize = 2; //3
const LEVEL_3: usize = 2; //2
const LEVEL_4: usize = 2; //One will remove all the user filters and loggers

fn build_graph(cli_arg: &Args) -> steady_state::Graph {
    debug!("args: {:?}",&cli_arg);

    //create the mutable graph object
    let mut graph = steady_state::Graph::new(cli_arg.clone());
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
                            .with_capacity(2400);

    let base_actor_builder = graph
                            .actor_builder()
                            .with_avg_mcpu()
                            .with_avg_work()
                           // .with_work_percentile(Percentile::p80())
                           // .with_mcpu_percentile(Percentile::p80())
                            .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10));

        let (btx,brx) = base_channel_builder.build_as_bundle::<_,LEVEL_1>();

            base_actor_builder
                .with_name("generator")
                .build_spawn(
                       move |context| actor::data_generator::run(context
                                                  , btx.clone()
                       )
                );


        for x in 0..LEVEL_1 {

            let local_rx = brx[x].clone();
            let (btx,brx) = base_channel_builder.build_as_bundle::<_, LEVEL_2>();
                base_actor_builder
                    .with_name_and_suffix("routerA",x)
                    .build_spawn(
                           move |context| actor::data_router::run(context
                                                  , LEVEL_1
                                                  , local_rx.clone()
                                                  , btx.clone()
                           )
                    );


            for y in 0..LEVEL_2 {

                let local_rx = brx[y].clone();
                let (btx,brx) = base_channel_builder.build_as_bundle::<_, LEVEL_3>();
                base_actor_builder
                    .with_name_and_suffix("routerB",y)
                    .build_spawn(move |context| actor::data_router::run(context
                                                                        , LEVEL_1*LEVEL_2
                                                                        , local_rx.clone()
                                                                        , btx.clone()
                                     )
                    );

                for z in 0..LEVEL_3 {

                    let local_rx = brx[z].clone();
                    let (btx,brx) = base_channel_builder.build_as_bundle::<_, LEVEL_4>();
                        base_actor_builder
                            .with_name_and_suffix("routerC",z)
                            .build_spawn(move |context| actor::data_router::run(context
                                                                                , LEVEL_1*LEVEL_2*LEVEL_3
                                                                                , local_rx.clone()
                                                                                , btx.clone()
                                          )
                            );


                    if 1 == LEVEL_4 {
                        let local_rx = brx[0].clone();
                            base_actor_builder
                                .with_name_and_suffix("user",z)
                                .build_spawn(move |context| actor::data_user::run(context
                                                                                  , local_rx.clone()
                                             )
                                );


                    } else {
                        let mut group = ActorGroup::default();
                        for f in 0..LEVEL_4 {

                            let local_rx = brx[f].clone();

                            let (filter_tx, filter_rx) = base_channel_builder.build();
                                base_actor_builder
                                    .with_name_and_suffix("filter",z)
                                    .build_join(move |context| actor::data_process::run(context
                                                                                        , local_rx.clone()
                                                                                        , filter_tx.clone()
                                                  ), & mut group
                                    );

                            let (logging_tx, logging_rx) = base_channel_builder.build();

                                base_actor_builder
                                    .with_name_and_suffix("logger",z)
                                    .build_join(move |context| actor::data_process::run(context
                                                                                        , filter_rx.clone()
                                                                                        , logging_tx.clone()
                                                  ), & mut group
                                    );


                            let (decrypt_tx, decrypt_rx) = base_channel_builder.build();



                            base_actor_builder
                                    .with_name_and_suffix("XXdecrypt",z)
                                    .build_join(move |context| actor::data_process::run(context
                                                                                        , logging_rx.clone()
                                                                                        , decrypt_tx.clone()
                                                    ), & mut group
                                    );

                                base_actor_builder
                                    .with_name_and_suffix("XXuser",z)
                                    .build_join(move |context| actor::data_user::run(context
                                                                                     , decrypt_rx.clone()
                                                    ), & mut group
                                    );



                        }
                              group.spawn();
                    }
                }
            }
        }

    graph
}



#[cfg(test)]
mod tests {
    #[async_std::test]
    async fn test_graph_one() {



    }


}


