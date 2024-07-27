use std::error::Error;
use std::mem;
use std::time::Duration;
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

    //info!("running {:?} {:?}",context.id(),context.name());

    let mut monitor =into_monitor!(context,[],tx);

    const ARRAY_REPEAT_VALUE: Vec<Packet> = Vec::new();

    let mut buffers:[Vec<Packet>; GIRTH] = [ARRAY_REPEAT_VALUE; GIRTH];
    let mut tx:TxBundle<Packet> = tx.lock().await;

    let capacity = tx[0].capacity();
    let limit:usize = capacity/2;

    while monitor.is_running(&mut || tx.mark_closed()) {

        let _clean = wait_for_all!(
           // monitor.wait_periodic(Duration::from_secs(10)),
            monitor.wait_vacant_units_bundle(&mut tx, limit, GIRTH)

        ).await;


        loop {
            let route = thread_rng().gen::<u16>();
            let packet = Packet {
                route,
                data: Bytes::from_static(&[0u8; 62]),
            };
            let index = (packet.route as usize) % tx.len();
            &mut buffers[index].push(packet);
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


#[cfg(test)]
pub async fn run<const GIRTH:usize>(context: SteadyContext
                 , tx: SteadyTxBundle<Packet,GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context,[], tx);

    let mut tx:TxBundle<Packet> = tx.lock().await;

    loop {
         tx[0].wait_vacant_units(1).await;


  //       relay_test(& mut monitor, &mut tx).await;
         monitor.relay_stats_smartly();
   }
}


