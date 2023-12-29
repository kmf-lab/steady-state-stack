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
    if let Err(e) = steady_logging_init(&opt.logging_level) {
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }

    Bastion::init(); //init bastion runtime
    build_graph(&opt); //graph is built here and tested below in the test section.
    Bastion::start(); //start the graph

    //remove this block to run forever.
    //run! is a macro provided by bastion that will block until the future is resolved.
    run!(async {
        // note we use (&opt) just to show we did NOT transfer ownership
        Delay::new(Duration::from_secs((&opt).run_duration)).await;
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
    let (generator_tx, generator_rx): (SteadyTx<WidgetInventory>, _) = graph.new_channel(8);
    let (consumer_tx, consumer_rx): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(8);
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
    use bastion::prelude::{BastionContext, Distributor, MessageHandler, RestartPolicy, RestartStrategy};
    use bastion::run;
    use super::*;

    #[async_std::test]
    async fn test_graph_one() {

        let test_ops = Args {
            run_duration: 21,
            logging_level: "debug".to_string(),
        };

        Bastion::init();

        build_graph(&test_ops);

        let r = RestartStrategy::default();
        let r = r.with_restart_policy(RestartPolicy::Never);

        /*
        let test_one = Bastion::supervisor(|supervisor|
            supervisor.with_strategy(SupervisionStrategy::OneForOne)
                .with_restart_strategy(r)
                .children(|children|
                     children
                        .with_redundancy(1)
                        .with_distributor(Distributor::named("say_hi"))
                        .with_name("test-one")
                        .with_exec(
                            move |ctx|
                                test_script_one(ctx)
                        )
                )
        ).expect("OneForOne supervisor creation error.");
*/

        // Launch Bastion
        Bastion::start();

        //the problem here is that we do not have the ref addr for the test actor.
        //let answer = children_ref.ask_anonymously(RequestMessage {
        //    content: "Hello, Bastion".into(),
        //}).expect("Failed to send message");

        let say_hi:Distributor = Distributor::named("testing-generator");
        //let say_hi:Distributor = Distributor::named("testing-consumer");


        run!(async {
            let answer: Result<&str, SendError> =
            say_hi.request("hi!").await.expect("Couldn't send request");

            assert_eq!(1,1);

            println!("{}", answer.expect("Couldn't receive answer"))

        });


        // Bastion::stop();
        Bastion::block_until_stopped();
    }

    pub async fn _test_script_one(ctx: BastionContext) -> Result<(),()> {

        MessageHandler::new(ctx.recv().await?)
            .on_tell(|message: &ApprovedWidgets, _sender_addr| {
                // Handle the message...
                println!("Received ApprovedWidgets: {:?}", message);
                // TODO: assert that the message is what we expected from the logic.

            })
          /*  .on_broadcast(|message: &SteadyBeacon, _sender_addr| {


                if let SteadyBeacon::TestCase(addr, case) = message {
                    if "One" == case {
                        test_one = Some(addr.clone());
                    }
                    // Handle the message...
                    println!("Received TestCase with case: {}", case);
                    // Potentially send a message back using addr
                }

            })*/
            /* .on_broadcast(|message: &SteadyBeacon, _sender_addr| {
                if let SteadyBeacon::Telemetry(addr) = message {
                    tel = Some(addr.clone());
                    //  init_actor(tel); //todo rebuild is a problem becuse broadcast wil be gone.
                    // Handle the message...
                    println!("Received target: {:?}", addr);
                    // Potentially send a message back using addr
                }

            })   */
        ;

        if true {
         //   Bastion::stop();
        }
        //get all the required messages for test one.
        //TODO: how do I make this test fail if the wront message is received?
        //     especially since we are in bastion and it eats panics.

        Ok(())
    }
}


