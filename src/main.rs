mod args;
mod steady;
use std::future::Future;
use structopt::*;
use log;
use log::{debug, error, info, trace, warn};
use flexi_logger;
use bastion::Bastion;
use bastion::prelude::{Children, ChildrenRef, SupervisionStrategy};
use bastion::supervisor::SupervisorRef;
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
                let monitor = graph.new_monitor();
                children
                    .with_redundancy(1)
                    .with_name("generator")
                    .with_callbacks(monitor.callbacks())
                    .with_exec(
                        move |ctx| actor::data_generator::
                                    behavior(monitor.wrap(ctx)
                                            , generator_tx.clone()

                        )
                    )
            })
            .children(|children| {
                    let monitor = graph.new_monitor();
                    children
                        .with_redundancy(1)
                        .with_name("approval")
                        .with_callbacks(monitor.callbacks())
                        .with_exec(
                            move |ctx| actor::data_approval::behavior(monitor.wrap(ctx)
                                                                      , generator_rx.clone()
                                                                      , consumer_tx.clone())
                        )
            })
            .children(|children| {
                        let monitor = graph.new_monitor();
                        children
                            .with_redundancy(1)
                            .with_name("consumer")
                            .with_callbacks(monitor.callbacks())
                            .with_exec(move |ctx|
                                            actor::data_consumer::behavior(monitor.wrap(ctx)
                                                                              , consumer_rx.clone())
                            )
            })
    ).expect("OneForOne supervisor creation error.");

}



pub fn single_actor<F, Fut, G>(sup: &SupervisorRef, sc: SteadyControllerBuilder, f: F, g: G) -> ChildrenRef
    where
        F: Fn(SteadyMonitor) -> Fut,
        Fut: Future<Output = Result<(), ()>> + Send + 'static,
        F: Send + Sync + 'static,
        G: Fn(Children) -> Children,
{
    sup.children(|children| g(children)
                           .with_exec(
        move |ctx| Box::pin(f(sc.clone().wrap(ctx)))
    )).expect("Error creating approve")
}



fn main() {

    let opt = Opt::from_args();

    match LogSpecification::env_or_parse(&opt.logging_level) {
        Ok(log_spec) => {
            match Logger::with(log_spec)
                .format(flexi_logger::colored_with_thread)
                .start() {
                Ok(_) => {
                    trace!("This is an trace log message");
                    debug!("This is an debug log message");
                    info!("This is an info log message");
                    warn!("This is a warn log message");
                    error!("This is an error log message");
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


//TODO: here we need the test graph the same as teh main graph as test so the actors are different.
//      in this case ...

#[cfg(test)]
mod tests {
    use bastion::prelude::BastionContext;
    use super::*;

    #[async_std::test]
    async fn test_graph() {

        let test_ops = Opt {
            peers_string: "".to_string(),
            my_idx: 0,
            logging_level: "debug".to_string(),
        };

        Bastion::init();

        build_graph(test_ops);

        let one_for_one: SupervisorRef = Bastion::supervisor(|supervisor|
            supervisor.with_strategy(
                SupervisionStrategy::OneForOne
            )
        ).expect("OneForOne supervisor creation error.");

        let test_ref_one: ChildrenRef = one_for_one.children(|children| children.with_exec(move |ctx|
            test_script_one(ctx)
        )).expect("Error creating approve");

        //we send this so all test implementations know the test actor
        let report_actor_addr = test_ref_one.elems().get(0).unwrap().addr();

      //  Bastion::broadcast(SteadyBeacon::TestCase(report_actor_addr.clone(), "One".to_owned()));

        // Launch Bastion
        Bastion::start();

        // Bastion::stop();
        Bastion::block_until_stopped();
    }

    pub async fn test_script_one(ctx: BastionContext) -> Result<(),()> {

        let msg = ctx.recv();


        Ok(())
    }
}


