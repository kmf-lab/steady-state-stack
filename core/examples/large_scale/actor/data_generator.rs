use std::time::Instant;
use std::error::Error;
use std::mem;
use std::sync::Arc;
use std::time::{Duration};
use bytes::Bytes;
use futures_util::lock::Mutex;
#[allow(unused_imports)]
use log::*;
use rand::{random};
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

#[cfg(test)]
pub async fn run<const GIRTH:usize>(context: SteadyContext
                                    , tx: SteadyTxBundle<Packet,GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut control = context.into_monitor([], tx.meta_data());
    let test_echos:Vec<_> = tx.iter().map(|f| TestEcho((*f).clone())).collect();
    let sims: Vec<&dyn IntoSimRunner<_>> = test_echos.iter().map(|te| te as &dyn IntoSimRunner<_>).collect();
    control.simulated_behavior(sims).await

}

async fn internal_behavior<const GIRTH:usize>(context: SteadyContext
                                    , tx: SteadyTxBundle<Packet, GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([],tx.meta_data());

    const ARRAY_REPEAT_VALUE: Vec<Packet> = Vec::new();

    let mut buffers:[Vec<Packet>; GIRTH] = [ARRAY_REPEAT_VALUE; GIRTH];
    let mut tx:TxBundle<Packet> = tx.lock().await;

    let capacity = tx[0].capacity();
    let limit:usize = capacity/2;

    while monitor.is_running(&mut || tx.mark_closed()) {

        let _clean = await_for_all!(
            monitor.wait_periodic(Duration::from_millis(500)),
            monitor.wait_vacant_bundle(&mut tx, limit, GIRTH)
        );


        loop {
            let route = random::<u16>();
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
mod generator_tests {
    use std::time::Duration;
    use futures_timer::Delay;
    use steady_state::*;
    use crate::actor::data_generator::{internal_behavior, Packet};

    #[async_std::test]
    async fn test_generator() {

        let mut graph = GraphBuilder::for_testing()
                          .build(());
        let expected_count = 100;
        let (approved_widget_tx_out, approved_widget_rx_out) = graph.channel_builder()
            .with_capacity(expected_count)
            .build_as_bundle::<Packet, 4>();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context| internal_behavior(context,approved_widget_tx_out.clone()));

        graph.start();

        Delay::new(Duration::from_secs(1)).await;

        graph.request_stop();
        graph.block_until_stopped(Duration::from_millis(3000));

        
        let count0 = approved_widget_rx_out[0].testing_avail_units().await;
        let count1 = approved_widget_rx_out[1].testing_avail_units().await;
        let count2 = approved_widget_rx_out[2].testing_avail_units().await;
        let count3 = approved_widget_rx_out[3].testing_avail_units().await;
        
        // println!("count0: {:?} count1: {:?} count2: {:?} count3: {:?}", count0, count1, count2, count3);

        assert_eq!(expected_count, count0);
        assert_eq!(expected_count, count1);
        assert_eq!(expected_count, count2);
        assert_eq!(expected_count, count3);


    }


}


