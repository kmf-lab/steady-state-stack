use std::error::Error;
use std::mem;
use std::time::{Duration};
use bytes::Bytes;
#[allow(unused_imports)]
use log::*;
use rand::{random};
use steady_state::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Packet {
    pub(crate) route: u16,
    pub(crate) data: Bytes,
}


pub async fn run<const GIRTH:usize>(context: SteadyContext
                                                  , tx: SteadyTxBundle<Packet, GIRTH>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor([], tx.meta_data());
    if cfg!(not(test)) {
        internal_behavior(cmd, tx).await
    } else {
        let test_echos:Vec<_> = tx.iter().map(|f| TestEcho((*f).clone())).collect();
        let sims: Vec<&dyn IntoSimRunner<_>> = test_echos.iter().map(|te| te as &dyn IntoSimRunner<_>).collect();
        cmd.simulated_behavior(sims).await
    }
}

async fn internal_behavior<const GIRTH:usize,C:SteadyCommander>(mut cmd: C
                                    , tx: SteadyTxBundle<Packet, GIRTH>) -> Result<(),Box<dyn Error>> {

    const ARRAY_REPEAT_VALUE: Vec<Packet> = Vec::new();

    let mut buffers:[Vec<Packet>; GIRTH] = [ARRAY_REPEAT_VALUE; GIRTH];
    let mut tx:TxBundle<Packet> = tx.lock().await;

    let capacity = tx[0].capacity();
    let limit:usize = capacity/2;

    while cmd.is_running(&mut || tx.mark_closed()) {

        let _clean = await_for_all!(
            cmd.wait_periodic(Duration::from_millis(500)),
            cmd.wait_vacant_bundle(&mut tx, limit, GIRTH)
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
            cmd.send_iter_until_full(&mut tx[i], iter);
        }
        cmd.relay_stats_smartly();

    }
    Ok(())
}

pub(crate) fn compute_index(tx: &mut TxBundle<Packet>, packet: &Packet) -> usize {
    (packet.route as usize) % tx.len()
}

#[cfg(test)]
mod generator_tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use crate::actor::data_generator::{internal_behavior, Packet};

    #[test]
    fn test_generator() {

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
        sleep(Duration::from_secs(1));
        graph.request_stop();
        graph.block_until_stopped(Duration::from_millis(3000));

        crate::assert_steady_rx_eq_count!(&approved_widget_rx_out[0],expected_count);
        crate::assert_steady_rx_eq_count!(&approved_widget_rx_out[1],expected_count);
        crate::assert_steady_rx_eq_count!(&approved_widget_rx_out[2],expected_count);
        crate::assert_steady_rx_eq_count!(&approved_widget_rx_out[3],expected_count);
    }
}


