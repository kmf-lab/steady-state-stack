mod args;
use structopt::StructOpt;
#[allow(unused_imports)]
use log::*;
use crate::args::Args;
use std::time::Duration;
use steady_state::*;


#[allow(unused)]


mod actor {
    
        pub mod console_printer;
    
        pub mod div_by_3_producer;
    
        pub mod div_by_5_producer;
    
        pub mod error_logger;
    
        pub mod fizz_buzz_processor;
    
        pub mod timer_actor;
    
}

fn main() {
    let opt = Args::from_args();
    if let Err(e) = steady_state::init_logging(&opt.loglevel) {
        //do not use logger to report logger could not start
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }

    let service_executable_name = "fizz_buzz";
    let service_user = "fizz_buzz_user";
    let systemd_command = SystemdBuilder::process_systemd_commands(  opt.systemd_action()
                                                   , opt.to_cli_string(service_executable_name)
                                                   , service_executable_name
                                                   , service_user);

    if !systemd_command {
        info!("Starting up");
        let mut graph = build_graph(steady_state::GraphBuilder::for_production().build(opt.clone()) );
        graph.start();

        {  //remove this block to run forever.
           std::thread::sleep(Duration::from_secs(60));
           graph.request_stop(); //actors can also call stop as desired on the context or monitor
        }

        graph.block_until_stopped(Duration::from_secs(2));
    }
}

fn build_graph(mut graph: Graph) -> steady_state::Graph {

    //this common root of the channel builder allows for common config of all channels
    let base_channel_builder = graph.channel_builder()
        .with_type()
        .with_line_expansion(1.0f32);
    //this common root of the actor builder allows for common config of all actors
    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor
        .with_mcpu_percentile(Percentile::p80())
        .with_load_percentile(Percentile::p80());
    //build channels
    
    let (n_to_fizzbuzzprocessor_numbers_tx, fizzbuzzprocessor_numbers_rx) = base_channel_builder
        .with_capacity(1000)
        .build_as_bundle::<_,2>();
    
    let (fizzbuzzprocessor_fizzbuzz_messages_tx, consoleprinter_fizzbuzz_messages_rx) = base_channel_builder
        .with_capacity(10000)
        .build();
    
    let (fizzbuzzprocessor_errors_tx, errorlogger_errors_rx) = base_channel_builder
        .with_capacity(100)
        .build();
    
    let (timeractor_print_signal_tx, consoleprinter_print_signal_rx) = base_channel_builder
        .with_capacity(1)
        .build();
    
    //build actors
    
    {
       base_actor_builder.with_name("ConsolePrinter")
                 .build_spawn( move |context| actor::console_printer::run(context
                                            , consoleprinter_fizzbuzz_messages_rx.clone()
                                            , consoleprinter_print_signal_rx.clone())
                 );
    }
    {
      
       let divby3producer_numbers_tx = n_to_fizzbuzzprocessor_numbers_tx[0].clone();
      
    
       base_actor_builder.with_name("DivBy3Producer")
                 .build_spawn( move |context| actor::div_by_3_producer::run(context
                                            , divby3producer_numbers_tx.clone())
                 );
    }
    {
      
       let divby5producer_numbers_tx = n_to_fizzbuzzprocessor_numbers_tx[1].clone();
      
    
       base_actor_builder.with_name("DivBy5Producer")
                 .build_spawn( move |context| actor::div_by_5_producer::run(context
                                            , divby5producer_numbers_tx.clone())
                 );
    }
    {
       base_actor_builder.with_name("ErrorLogger")
                 .build_spawn( move |context| actor::error_logger::run(context
                                            , errorlogger_errors_rx.clone())
                 );
    }
    {
      
    
      
    
       base_actor_builder.with_name("FizzBuzzProcessor")
                 .build_spawn( move |context| actor::fizz_buzz_processor::run(context
                                            , fizzbuzzprocessor_numbers_rx.clone()
                                            , fizzbuzzprocessor_fizzbuzz_messages_tx.clone()
                                            , fizzbuzzprocessor_errors_tx.clone())
                 );
    }
    {
      
    
       base_actor_builder.with_name("TimerActor")
                 .build_spawn( move |context| actor::timer_actor::run(context
                                            , timeractor_print_signal_tx.clone())
                 );
    }
    graph
}

#[cfg(test)]
mod graph_tests {
    use async_std::test;
    use steady_state::*;
    use std::time::Duration;
    use crate::args::Args;
    use crate::build_graph;
    use std::ops::DerefMut;


    #[test]
    async fn test_graph_one() {

            let test_ops = Args {
                loglevel: "debug".to_string(),
                systemd_install: false,
                systemd_uninstall: false,
            };
            let mut graph = build_graph( GraphBuilder::for_testing().build(test_ops.clone()) );
            graph.start();
            let mut guard = graph.sidechannel_director().await;
            let g = guard.deref_mut();
            assert!(g.is_some(), "Internal error, this is a test so this back channel should have been created already");
            if let Some(_plane) = g {

              //  write your test here, send messages to edge nodes and get responses
              //  let response = plane.node_call(Box::new(SOME_STRUCT), "SOME_NODE_NAME").await;
              //  if let Some(msg) = response {
              //  }

            }
            drop(guard);
            graph.request_stop();
            graph.block_until_stopped(Duration::from_secs(3));

    }
}