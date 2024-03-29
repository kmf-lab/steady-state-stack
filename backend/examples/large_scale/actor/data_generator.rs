use std::error::Error;
use std::mem;
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
#[allow(unreachable_code)]
pub async fn run<const GIRTH:usize>(context: SteadyContext
                                    , tx: SteadyTxBundle<Packet, GIRTH>) -> Result<(),Box<dyn Error>> {


    let mut monitor = context.into_monitor([], tx.def_slice());

    const ARRAY_REPEAT_VALUE: Vec<Packet> = Vec::new();

    let mut buffers:[Vec<Packet>; GIRTH] = [ARRAY_REPEAT_VALUE; GIRTH];
    let mut tx:TxBundle<Packet> = tx.lock().await;

    let capacity = tx[0].capacity();
    let limit:usize = capacity/4;

    while monitor.is_running(&mut || tx.mark_closed()) {

        monitor.wait_vacant_units_bundle(&mut tx, limit, GIRTH).await;
        single_iteration(&mut monitor, &mut buffers, &mut tx, limit).await;
        monitor.relay_stats_smartly().await;

    }
    Ok(())
}

async fn single_iteration<const GIRTH: usize>(monitor: &mut LocalMonitor<0, GIRTH>, buffers: &mut [Vec<Packet>; GIRTH]
                                              , tx: &mut TxBundle<'_, Packet>, limit: usize) {
    loop {
        let route = thread_rng().gen::<u16>();
        let packet = Packet {
            route,
            data: Bytes::from_static(&[0u8; 128]),
        };
        let index = (packet.route as usize) % tx.len();
        buffers[index].push(packet);
        if buffers[index].len() >= limit {
            //first one we fill to limit, the rest will not be as full
            break;
        }
    }
    //repeat
    for i in 0..GIRTH {
        let iter = mem::replace(&mut buffers[i], Vec::new()).into_iter();
        monitor.send_iter_until_full(&mut tx[i], iter);
    }

}


#[cfg(test)]
pub async fn run<const GIRTH:usize>(context: SteadyContext
                 , tx: SteadyTxBundle<Packet,GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([], tx.def_slice());

    let mut tx:TxBundle<Packet> = tx.lock().await;

    loop {
         tx[0].wait_vacant_units(1).await;


  //       relay_test(& mut monitor, &mut tx).await;
         monitor.relay_stats_smartly().await;
   }
}
#[cfg(test)]
async fn relay_test<const R:usize, const T:usize>(monitor: &mut LocalMonitor<R,T>
                    , tx: &Tx<Packet>) {

    /*
    if let Some(ctx) = monitor.ctx() {
        MessageHandler::new(ctx.recv().await.unwrap())
            .on_question(|message: WidgetInventory, answer_sender| {
                info!("relay_test: {:?}", message);
                run!(async {
                    let _ = monitor.send_async(tx, message).await;
                    answer_sender.reply("ok").unwrap();
                   });
            });
    }
    //      */


}



/*
#[cfg(test)]
mod tests {
    use std::ops::DerefMut;
    use crate::actor::data_generator::{InternalState, iterate_once};
    use steady_state::{Graph, util};

    #[async_std::test]
    async fn test_iterate_once() {
        util::logger::initialize();

        let mut graph = Graph::new();

    }

}
//             */