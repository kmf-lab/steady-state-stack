use std::error::Error;
use std::mem;
use std::time::{Duration, Instant};
use bytes::Bytes;

#[allow(unused_imports)]
use log::*;
use rand::{Rng, thread_rng};
use steady_state::*;


#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Packet {
    pub(crate) route: u16,
    pub(crate) data: Bytes,
}


#[cfg(not(test))]
pub async fn run<const GIRTH:usize>(context: SteadyContext
                                                  , tx: SteadyTxBundle<Packet, GIRTH>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, tx).await
}

async fn internal_behavior<const GIRTH:usize>(context: SteadyContext
                                    , tx: SteadyTxBundle<Packet, GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context,[],tx);

    const ARRAY_REPEAT_VALUE: Vec<Packet> = Vec::new();

    let mut buffers:[Vec<Packet>; GIRTH] = [ARRAY_REPEAT_VALUE; GIRTH];
    let mut tx:TxBundle<Packet> = tx.lock().await;

    let capacity = tx[0].capacity();
    let limit:usize = capacity/2;

    while monitor.is_running(&mut || tx.mark_closed()) {

        let _clean = wait_for_all!(
            monitor.wait_periodic(Duration::from_millis(500)),
            monitor.wait_shutdown_or_vacant_units_bundle(&mut tx, limit, GIRTH)
        );


        loop {
            let route = thread_rng().random::<u16>();
            let packet = Packet {
                route,
                data: Bytes::from_static(&[0u8; 62]),
            };
            let index = compute_index(&mut tx, &packet);
            buffers[index].push(packet);
            if &mut buffers[index].len() >= &mut (limit * 2) {
                //first one we fill to limit, the rest will not be as full
                break;
            }
        }
        //repeat
        for i in 0..GIRTH {
            let replace = mem::replace(&mut buffers[i], Vec::with_capacity(limit * 2));
            let iter = replace.into_iter();
            monitor.send_iter_until_full(&mut tx[i], iter);
        }
        monitor.relay_stats_smartly();

    }
    Ok(())
}

pub(crate) fn compute_index(tx: &mut TxBundle<Packet>, packet: &Packet) -> usize {
    (packet.route as usize) % tx.len()
}

#[cfg(test)]
pub async fn run<const GIRTH:usize>(context: SteadyContext
                 , tx: SteadyTxBundle<Packet,GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context,[], tx);
    let mut tx:TxBundle<Packet> = tx.lock().await;

    if let Some(mut responder) = monitor.sidechannel_responder() { //outside
        //info!("test generator running");
        while monitor.is_running(&mut || tx.mark_closed()) {

            let now = Instant::now();
            //info!("waiting for responder units");
            let clean = wait_for_all!(
               // monitor.wait_periodic(Duration::from_millis(500))
                responder.wait_available_units(1)
                ,
                monitor.wait_shutdown_or_vacant_units_bundle(&mut tx, 1, GIRTH)
            );
            let _duration = now.elapsed();
            //info!("got a responder and we have room to write, clean: {} duration:{:?}",clean,duration);

           if clean {
               let _ok = responder.respond_with(|message| {
                   let msg: &Packet = message.downcast_ref::<Packet>().expect("error casting");
                   let index = compute_index(&mut tx, msg);
                   match monitor.try_send(&mut tx[index], msg.clone()) {
                       Ok(()) => Box::new("ok".to_string()),
                       Err(m) => Box::new(format!("{:?}", m)),
                   }
               }).await;
           }


          monitor.relay_stats_smartly();
        }
      //  info!("shutdown generator");
    }
    Ok(())
}


#[cfg(test)]
mod generator_tests {
    use async_std::test;


    // #[test]
    // pub(crate) async fn test_generator() {
    //     //1. build test graph, the input and output channels and our actor
    //     let mut graph = Graph::new_test(());
    //
    //     // let (approved_widget_tx_out, approved_widget_rx_out) = graph.channel_builder()
    //     //     .with_capacity(BATCH_SIZE).build();
    //     //
    //     // let state = InternalState {
    //     //     last_approval: None,
    //     //     buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    //     // };
    //
    //     // graph.actor_builder()
    //     //     .with_name("UnitTest")
    //     //     .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state));
    //     //
    //     // // //2. add test data to the input channels
    //     // let test_data: Vec<Packet> = (0..BATCH_SIZE).map(|i| Packet { original_count: 0, approved_count: i as u64 }).collect();
    //     // approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;
    //
    //     // //3. run graph until the actor detects the input is closed
    //     graph.start_as_data_driven(Duration::from_secs(240));
    //
    //     //4. assert expected results
    //     // TODO: not sure how to make this work.
    //     //  println!("last approval: {:?}", &state.last_approval);
    //     //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);
    // }


}


