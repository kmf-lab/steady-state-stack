mod args;
#[macro_use]
mod steady;

use structopt::*;
use log;
use log::{debug, error, info, trace, warn};
use flexi_logger;
use bastion::Bastion;
use bastion::prelude::*;
use flexi_logger::{Logger, LogSpecification};
use crate::steady::*;
use crate::args::Opt;

mod actor {
    pub mod example_empty_actor;
    pub mod data_generator;
    pub use data_generator::WidgetInventory;
    pub mod data_approval;
    pub use data_approval::ApprovedWidgets;
    pub mod data_consumer;
}
use crate::actor::*; //message structs


fn build_graph(opt: Opt) {
    trace!("args: {:?}",&opt);

    let mut graph = SteadyGraph::new();
    let (generator_tx, generator_rx): (SteadyTx<WidgetInventory>, _) = graph.new_channel(8);
    let (consumer_tx, consumer_rx): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(8);

    let _ = Bastion::supervisor(|supervisor|
        supervisor.with_strategy(SupervisionStrategy::OneForOne)
            .children(|children| {
                graph.add_to_graph("generator"
                                  , children.with_redundancy(0)
                        , move |monitor|
                             actor::data_generator::behavior(monitor
                                                            , generator_tx.clone())
                    )
            })
            .children(|children| {
                    graph.add_to_graph("approval"
                                      , children.with_redundancy(0)
                        , move |monitor|
                                actor::data_approval::behavior(monitor
                                                               , generator_rx.clone()
                                                               , consumer_tx.clone())
                                                    )
            })
            .children(|children| {
                    graph.add_to_graph("consumer"
                                      , children.with_redundancy(0)
                            ,move |monitor|
                                actor::data_consumer::behavior(monitor
                                                               , consumer_rx.clone())
                            )
            })
    ).expect("OneForOne supervisor creation error.");

    graph.init_telemetry();


}

fn main() {

    let opt = Opt::from_args();

    match LogSpecification::env_or_parse(&opt.logging_level) {
        Ok(log_spec) => {
            match Logger::with(log_spec)
                .format(flexi_logger::colored_with_thread)
                .start() {
                Ok(_) => {
                    // for all log levels use caution and never write any personal data to the logs.
                    trace!("trace log message, use when deep tracing through the application");
                    debug!("debug log message, use when debugging complex parts of the application");
                    warn!("warn log message, when something recoverable happens, may be a flag that this area needs attention");
                    error!("error log message, use when the unexpected happens");
                    info!("info log message, use rarely when key events happen");
                },
                Err(e) => {
                    // Logger initialization failed
                    println!("Warning: Logger initialization failed with {}. There will be no logging.", e);
                },
            };
        },
        Err(e) => {
            // Logger initialization failed
            println!("Warning: Logger initialization failed with {}. There will be no logging.", e);
        }
    }

    Bastion::init();
    build_graph(opt);
    Bastion::start();
//    Bastion::stop();
    Bastion::block_until_stopped();

}




#[cfg(test)]
mod tests {
    use bastion::prelude::{BastionContext, Distributor, MessageHandler, RestartPolicy, RestartStrategy};
    use bastion::run;
    use super::*;

    #[async_std::test]
    async fn test_graph_one() {

        let test_ops = Opt {
            peers_string: "".to_string(),
            my_idx: 0,
            logging_level: "debug".to_string(),
        };

        Bastion::init();

        build_graph(test_ops);

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


