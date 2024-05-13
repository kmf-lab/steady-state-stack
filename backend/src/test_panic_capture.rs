#[cfg(test)]
mod simple_graph_test {

    use crate::*;


    #[async_std::test]
    async fn test_graph() {
        if let Err(e) = init_logging("info") {
            eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
        }
        //smallest possible graph just to test the capture of throw.
        //it is not recommended to inline actors this way, but it is possible.


        //special test graph which does NOT fail fast but instead shows the prod behavior of restarting actors.
        let mut graph = Graph::internal_new((),true);

        let (tx,rx) = graph.channel_builder()
            .with_capacity(300)
            .build();

        let gen_count = Arc::new(AtomicUsize::new(0));

        graph.actor_builder()
            .with_name("generator")
            .build_spawn( move |mut context| {
            let tx = tx.clone();
            let count = gen_count.clone();
            async move {
                let mut tx = tx.lock().await;
                while context.is_running( &mut || tx.mark_closed()) {
                    let x = count.fetch_add(1, Ordering::Relaxed);
                    if x>=10 {
                        context.request_graph_stop();
                        continue;
                    }
                    if 5==x {
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
            .build_spawn( move |context| {
                let rx = rx.clone();
                let count = consume_count.clone();
                async move {
                    let mut rx = rx.lock().await;
                    while context.is_running( &mut || rx.is_closed() && rx.is_empty()) {
                        if let Some(packet) = rx.try_take() {
                            count.fetch_add(1, Ordering::Relaxed);
                            //info!("received packet: {:?}",packet);
                        }
                    }
                    Ok(())
                }
        });

      graph.start();

      graph.block_until_stopped(Duration::from_secs(5));

      assert_eq!(9, check_count.load(Ordering::Relaxed));

    }


}
