use std::mem;
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};
use std::time::Duration;
use bytes::Bytes;
#[allow(unused_imports)]
use log::*;
use rand::{Rng, thread_rng};
use crate::args::Args;
use steady_state::*;


#[derive(Clone, Debug)]
pub struct Packet {
    pub(crate) route: u16,
    pub(crate) data: Bytes,
}



#[cfg(not(test))]
pub async fn run<const GURTH:usize>(context: SteadyContext
                                  , tx: SteadyTxBundle<Packet,GURTH>) -> Result<(),()> {

    let gen_rate_micros = if let Some(a) = context.args::<Args>() {
        a.gen_rate_micros
    } else {
        10_000 //default
    };
    let mut monitor = context.into_monitor([], SteadyBundle::tx_def_slice(&tx));

    const ARRAY_REPEAT_VALUE: Vec<Packet> = Vec::new();

    let mut buffers:[Vec<Packet>;GURTH] = [ARRAY_REPEAT_VALUE;GURTH];
    let capacity = tx[0].lock().await.capacity();
    let limit:usize = capacity.get()/4;

    loop {
        loop {
            let route = thread_rng().gen::<u16>();
            let packet = Packet {
                route,
                data: Bytes::from_static(&[0u8; 250]),
            };
            let index = (packet.route as usize) % tx.len();
            buffers[index].push(packet);
            if buffers[index].len() >= limit {
                break;
            }
        }

        //repeat
        for i in 0..GURTH {
            let iter = mem::replace(&mut buffers[i], Vec::new()).into_iter();

            let mut lock = tx[i].lock().await;
            let tx = lock.deref_mut();
            monitor.wait_vacant_units(tx, buffers[i].len()).await;
            monitor.send_iter_until_full(tx,iter);
        }
        monitor.relay_stats_all().await;
        //monitor.relay_stats_periodic(Duration::from_micros(gen_rate_micros)).await;
    }
    Ok(())
}






#[cfg(test)]
pub async fn run<const LEN:usize>(context: SteadyContext
                 , tx: [&SteadyTx<Packet>;LEN]) -> Result<(),()> {

    let mut monitor = context.into_monitor([], tx);

    let mut tx_guard = tx.lock().await;
    let tx = tx_guard.deref_mut();


    loop {
         relay_test(& mut monitor, &tx).await;
         monitor.relay_stats_all().await;
   }
}
#[cfg(test)]
async fn relay_test<const R:usize, const T:usize, const LEN:usize>(monitor: &mut LocalMonitor<R,T>
                    , tx: &[SteadyTx<Packet>;LEN]) {
    use bastion::run;
    use bastion::message::MessageHandler;

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