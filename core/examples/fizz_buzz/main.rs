mod args;

#[allow(unused_imports)]
use log::*;
use crate::args::Args;
use std::time::Duration;
use steady_state::*;
use steady_state::actor_builder::{Troupe, ScheduleAs};

mod actor {
        pub mod console_printer;
        pub mod div_by_3_producer;
        pub mod div_by_5_producer;
        pub mod error_logger;
        pub mod fizz_buzz_processor;
        pub mod timer_actor;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {

    let opt = Args::parse();
    if let Err(e) = init_logging(opt.loglevel) {
        //do not use logger to report logger could not start
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }



    let service_executable_name = "fizz_buzz";
    let service_user = "fizz_buzz_user";
    let systemd_command = SystemdBuilder::process_systemd_commands(  opt.systemd_action()
                                                   , service_executable_name
                                                   , service_user);

    if !systemd_command {

        let mut graph = build_graph(GraphBuilder::for_production()
                        .with_telemtry_production_rate_ms(200)
                        .with_shutdown_barrier(2)
                        .build(opt.clone()) );
        graph.start();

        graph.block_until_stopped(Duration::from_millis(500))
    } else {
        Ok(())
    }
}


fn build_graph(mut graph: Graph) -> Graph {

   // graph.telemetry_production_rate_ms()
    //this common root of the channel builder allows for common config of all channels
    let base_channel_builder = graph.channel_builder()
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        .with_filled_trigger(Trigger::AvgAbove(Filled::percentage(75.00f32).expect("internal range error")), AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
        .with_line_expansion(0.0001f32)
        .with_type();
    //this common root of the actor builder allows for common config of all actors
    let base_actor_builder = graph.actor_builder() //with default OneForOne supervisor
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange)
        .with_mcpu_trigger(Trigger::AvgAbove( MCPU::m768()), AlertColor::Red)
        .with_thread_info()
        .with_mcpu_avg()
        .with_load_avg();
    //build channels

    let (n_to_fizzbuzzprocessor_numbers_tx, fizzbuzzprocessor_numbers_rx) = base_channel_builder
        .with_capacity(32000)
        .with_avg_rate()
        .with_avg_filled()
        .build_channel_bundle::<_,2>();
    
    let (fizzbuzzprocessor_fizzbuzz_messages_tx, consoleprinter_fizzbuzz_messages_rx) = base_channel_builder
        .with_capacity(64000)
        .with_avg_rate()
        .with_avg_filled()
      //  .with_avg_latency()
        .build_channel();
    
    let (fizzbuzzprocessor_errors_tx, errorlogger_errors_rx) = base_channel_builder
        .with_capacity(100)
        .build_channel();
    
    let (timeractor_print_signal_tx, consoleprinter_print_signal_rx) = base_channel_builder
        .with_capacity(10)
        .build_channel();


    {
      
       let divby3producer_numbers_tx = n_to_fizzbuzzprocessor_numbers_tx[0].clone();
      
    
       base_actor_builder.with_name("DivBy3Producer")
           .with_explicit_core(8)
           .build(move |context| actor::div_by_3_producer::run(context
                                            , divby3producer_numbers_tx.clone()),
                  ScheduleAs::SoloAct
                 );
    }
    {
      
       let divby5producer_numbers_tx = n_to_fizzbuzzprocessor_numbers_tx[1].clone();
      
    
       base_actor_builder.with_name("DivBy5Producer")
           .with_explicit_core(8)
           .build(move |context| actor::div_by_5_producer::run(context
                                            , divby5producer_numbers_tx.clone()),
                  ScheduleAs::SoloAct
                 );
    }

    {
       let state = new_state();
       base_actor_builder.with_name("FizzBuzzProcessor")
              .with_explicit_core(3)
                 .build(move |context| actor::fizz_buzz_processor::run(context
                                            , fizzbuzzprocessor_numbers_rx.clone()
                                            , fizzbuzzprocessor_fizzbuzz_messages_tx.clone()
                                            , fizzbuzzprocessor_errors_tx.clone(), state.clone()),
                        ScheduleAs::SoloAct
                 );
    }
    {


        base_actor_builder.with_name("TimerActor")
            .with_explicit_core(9)
            .build(move |context| actor::timer_actor::run(context
                                                           , timeractor_print_signal_tx.clone()),
                   ScheduleAs::SoloAct
            );
    }

    {
        base_actor_builder.with_name("ConsolePrinter")
            .with_explicit_core(3+4)
            .build(move |context| actor::console_printer::run(context
                                                               , consoleprinter_fizzbuzz_messages_rx.clone()
                                                               , consoleprinter_print_signal_rx.clone()),
                   ScheduleAs::SoloAct
            );
    }
    {
        base_actor_builder.with_name("ErrorLogger")
            .with_explicit_core(9)
            .build(move |context| actor::error_logger::run(context
                                                            , errorlogger_errors_rx.clone()),
                   ScheduleAs::SoloAct
            );
    }
    graph
}

// #[cfg(test)]
// mod graph_tests {
//     use steady_state::*;
//     use std::time::Duration;
//     use crate::args::Args;
//     use crate::build_graph;
//     use std::thread::sleep;
//
//     #[test]
//     fn test_graph() {  //TODO: copy this from the generated code!!
//
//         let test_ops = Args {
//             loglevel: LogLevel::Debug,
//             systemd_install: false,
//             systemd_uninstall: false,
//         };
//         let mut graph = build_graph( GraphBuilder::for_testing().build(test_ops.clone()) );
//         graph.start();
//         let messenger = graph.sidechannel_messenger();
// //TODO: should the simulator be the foucs can we push back to the actor on the other end?
//
//
//             //NOTE: to ensure the node_call is for the correct channel for a given actor unique types for each channel are required
//
//
//             //TODO:   Adjust as needed to inject test values into the graph
//             //  let response = plane.call_actor(Box::new(FizzBuzzMessage::default()), "ConsolePrinter");
//             //  if let Some(msg) = response { // ok indicates the message was echoed
//             //     //trace!("response: {:?} {:?}", msg.downcast_ref::<String>(),i);
//             //     assert_eq!("ok", msg.downcast_ref::<String>().expect("bad type"));
//             //  } else {
//             //     error!("bad response from generator: {:?}", response);
//             //    // panic!("bad response from generator: {:?}", response);
//             //  }
//             //TODO:   Adjust as needed to inject test values into the graph
//             //  let response = plane.call_actor(Box::new(PrintSignal::default()), "ConsolePrinter");
//             //  if let Some(msg) = response { // ok indicates the message was echoed
//             //     //trace!("response: {:?} {:?}", msg.downcast_ref::<String>(),i);
//             //     assert_eq!("ok", msg.downcast_ref::<String>().expect("bad type"));
//             //  } else {
//             //     error!("bad response from generator: {:?}", response);
//             //    // panic!("bad response from generator: {:?}", response);
//             //  }
//
//
//
//             //TODO:   Adjust as needed to inject test values into the graph
//             //  let response = plane.call_actor(Box::new(ErrorMessage::default()), "ErrorLogger");
//             //  if let Some(msg) = response { // ok indicates the message was echoed
//             //     //trace!("response: {:?} {:?}", msg.downcast_ref::<String>(),i);
//             //     assert_eq!("ok", msg.downcast_ref::<String>().expect("bad type"));
//             //  } else {
//             //     error!("bad response from generator: {:?}", response);
//             //    // panic!("bad response from generator: {:?}", response);
//             //  }
//
//
//
//             //TODO:   Adjust as needed to test the values produced by the graph
//             //  let response = plane.call_actor(Box::new(NumberMessage::default()), "DivBy3Producer");
//             //  if let Some(msg) = response { // ok indicates the expected structure instance matched
//             //     //trace!("response: {:?} {:?}", msg.downcast_ref::<String>(),i);
//             //     assert_eq!("ok", msg.downcast_ref::<String>().expect("bad type"));
//             //  } else {
//             //     error!("bad response from generator: {:?}", response);
//             //    // panic!("bad response from generator: {:?}", response);
//             //  }
//
//             //TODO:   Adjust as needed to test the values produced by the graph
//             //  let response = plane.call_actor(Box::new(NumberMessage::default()), "DivBy5Producer");
//             //  if let Some(msg) = response { // ok indicates the expected structure instance matched
//             //     //trace!("response: {:?} {:?}", msg.downcast_ref::<String>(),i);
//             //     assert_eq!("ok", msg.downcast_ref::<String>().expect("bad type"));
//             //  } else {
//             //     error!("bad response from generator: {:?}", response);
//             //    // panic!("bad response from generator: {:?}", response);
//             //  }
//
//
//             //TODO:   Adjust as needed to test the values produced by the graph
//             //  let response = plane.call_actor(Box::new(PrintSignal::default()), "TimerActor");
//             //  if let Some(msg) = response { // ok indicates the expected structure instance matched
//             //     //trace!("response: {:?} {:?}", msg.downcast_ref::<String>(),i);
//             //     assert_eq!("ok", msg.downcast_ref::<String>().expect("bad type"));
//             //  } else {
//             //     error!("bad response from generator: {:?}", response);
//             //    // panic!("bad response from generator: {:?}", response);
//             //  }
//         drop(messenger);
//
//         graph.request_shutdown();
//         graph.block_until_stopped(Duration::from_secs(3));
//
//     }
// }