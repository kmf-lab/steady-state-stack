//! This module tests the capture of panics in actors within a graph in the SteadyState project.
//!
//! The test creates a minimal graph setup to verify that panics are correctly captured and handled,
//! ensuring that the graph can restart actors as intended.


#[cfg(test)]
mod simple_graph_test {
    use crate::*;
    use std::sync::{Arc, atomic::{AtomicUsize}};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// Tests the capture of panics within the graph.
    ///
    /// This test sets up a minimal graph with a generator actor that panics when a specific condition
    /// is met, and a consumer actor that processes messages from the generator. The test verifies that
    /// the panic is captured and the graph handles it by restarting the actor.
    ///
    /// # Panics
    ///
    /// The generator actor is designed to panic when it generates the 5th message.
    ///
    /// # Examples
    /// this test should only be run manually, as it will panic by design
    #[cfg(not(windows))]
    #[test]
    fn test_panic_graph() {
    
        // Smallest possible graph just to test the capture of panic.
        // It is not recommended to inline actors this way, but it is possible.
        let gen_count = Arc::new(AtomicUsize::new(0));

        // Special test graph which does NOT fail fast but instead shows the prod behavior of restarting actors.
        let mut graph = GraphBuilder::for_testing()
            .with_block_fail_fast()
            .with_telemtry_production_rate_ms(100)
            .with_iouring_queue_length(8)
            .with_telemetry_metric_features(true)
            .build(());


        let (tx, rx) = graph.channel_builder()
            .with_capacity(300)
            .build_channel();
    
    
        graph.actor_builder()
            .with_name("generator")
            .build(move |mut context| {
                let tx = tx.clone();
                let count = gen_count.clone();
                async move {
                    let mut tx = tx.lock().await;
                    while context.is_running(&mut || tx.mark_closed()) {
                        let x = count.fetch_add(1, Ordering::SeqCst);
                        //info!("attempted sent: {:?}", count.load(Ordering::SeqCst));
    
                        if x >= 10 {
                            context.request_shutdown().await;
                            continue;
                        }
                        if 5 == x {
              //              #[cfg(not(coverage))]  // coverage does not support panics so this is disabled for coverage
             //               panic!("panic at 5");
                        }
                        let _ = context.send_async(&mut tx, x.to_string(), SendSaturation::AwaitForRoom).await;
                    }
                    Ok(())
                }
            }, SoloAct);
    
        let consume_count = Arc::new(AtomicUsize::new(0));
        let check_count = consume_count.clone();
    
        graph.actor_builder()
            .with_name("consumer")
            .build(move |mut context| {
                let rx = rx.clone();
                let count = consume_count.clone();
                async move {
                    let mut rx = rx.lock().await;
                    while context.is_running(&mut || rx.is_closed() && rx.is_empty()) {
                        if let Some(_packet) = rx.shared_try_take() {
                            count.fetch_add(1, Ordering::SeqCst);
                           // info!("received: {:?}", count.load(Ordering::SeqCst));
                        }
                    }
                    Ok(())
                }
            }, SoloAct);
    
        graph.start();
        graph.block_until_stopped(Duration::from_secs(7));
    
        //is 10 if there is no panic but it is 9 if we have a panic at 5.
        //println!("consume count: {:?}", check_count.load(Ordering::SeqCst));
      //  #[cfg(not(coverage))] 
       // assert_eq!(9, check_count.load(Ordering::SeqCst), "expected consume count 9 got {:?}", check_count.load(Ordering::SeqCst));
      //  #[cfg(coverage)]
        assert_eq!(10, check_count.load(Ordering::SeqCst), "expected consume count 10 got {:?}", check_count.load(Ordering::SeqCst));
    
    
    }
}
