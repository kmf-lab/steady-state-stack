//! This module tests the capture of panics in actors within a graph in the SteadyState project.
//!
//! The test creates a minimal graph setup to verify that panics are correctly captured and handled,
//! ensuring that the graph can restart actors as intended.

#[cfg(test)]
mod simple_graph_test {
    use crate::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
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
    ///
    /// ```
    /// #[async_std::test]
    /// async fn test_panic_graph() {
    ///     // Test implementation...
    /// }
    /// ```
    #[async_std::test]
    async fn test_panic_graph() {
        if let Err(e) = init_logging("info") {
            eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
        }

        // Smallest possible graph just to test the capture of panic.
        // It is not recommended to inline actors this way, but it is possible.

        // Special test graph which does NOT fail fast but instead shows the prod behavior of restarting actors.
        let mut graph = Graph::internal_new((), true, false);

        let (tx, rx) = graph.channel_builder()
            .with_capacity(300)
            .build();

        let gen_count = Arc::new(AtomicUsize::new(0));

        graph.actor_builder()
            .with_name("generator")
            .build_spawn(move |mut context| {
                let tx = tx.clone();
                let count = gen_count.clone();
                async move {
                    let mut tx = tx.lock().await;
                    while context.is_running(&mut || tx.mark_closed()) {
                        let x = count.fetch_add(1, Ordering::Relaxed);
                        if x >= 10 {
                            context.request_graph_stop();
                            continue;
                        }
                        if 5 == x {
                            panic!("panic at 5");
                        }
                        let _ = context.send_async(&mut tx, x.to_string(), SendSaturation::IgnoreAndWait).await;
                    }
                    Ok(())
                }
            });

        let consume_count = Arc::new(AtomicUsize::new(0));
        let check_count = consume_count.clone();

        graph.actor_builder()
            .with_name("consumer")
            .build_spawn(move |context| {
                let rx = rx.clone();
                let count = consume_count.clone();
                async move {
                    let mut rx = rx.lock().await;
                    while context.is_running(&mut || rx.is_closed() && rx.is_empty()) {
                        if let Some(_packet) = rx.shared_try_take() {
                            count.fetch_add(1, Ordering::SeqCst);
                            // info!("received packet: {:?}", _packet);
                        }
                    }
                    Ok(())
                }
            });

        graph.start();
        graph.block_until_stopped(Duration::from_secs(7));

        assert_eq!(9, check_count.load(Ordering::SeqCst),"expected consume count");
    }
}
