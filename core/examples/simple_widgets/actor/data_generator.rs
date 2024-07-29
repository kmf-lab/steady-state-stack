use std::error::Error;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use steady_state::SteadyRx;
use steady_state::{SteadyTx, Tx};
use crate::actor::data_feedback::ChangeRequest;

#[derive(Clone, Debug, Copy)]
pub struct WidgetInventory {
    pub(crate) count: u64,
    pub(crate) _payload: u64,
}



#[cfg(not(test))]
pub async fn run(context: SteadyContext
                               , feedback: SteadyRx<ChangeRequest>
                               , tx: SteadyTx<WidgetInventory> ) -> Result<(),Box<dyn Error>> {
    _internal_behavior(context, feedback, tx).await
}

struct InternalState {
    count: u64
}

pub async fn _internal_behavior(context: SteadyContext
                                , feedback: SteadyRx<ChangeRequest>
                                , tx: SteadyTx<WidgetInventory> ) -> Result<(),Box<dyn Error>> {

    let gen_rate_micros = if let Some(a) = context.args::<crate::Args>() {
        a.gen_rate_micros
    } else {
        10_000 //default
    };

    let mut monitor = into_monitor!(context, [feedback], [tx]);
    let mut feedback = feedback.lock().await;
    let mut tx = tx.lock().await;

    const MULTIPLIER:usize = 256;   //500_000 per second at 500 micros

    let mut state = InternalState {
        count: 0,
    };
    while monitor.is_running(&mut || tx.mark_closed() ) {

        let mut wids = Vec::with_capacity(MULTIPLIER);

        (0..=MULTIPLIER)
            .for_each(|num|
            wids.push(
                WidgetInventory {
                    count: state.count+num as u64,
                    _payload: 42,
                }));

        state.count+= MULTIPLIER as u64;


        let sent = monitor.send_slice_until_full(&mut tx, &wids);
        //iterator of sent until the end
        let consume = wids.into_iter().skip(sent);
        for send_me in consume {
            let _ = monitor.send_async(&mut tx, send_me, SendSaturation::Warn).await;
        }


        if let Some(feedback) = monitor.try_take(&mut feedback) {
              trace!("data_generator feedback: {:?}", feedback);
        }

        //this is an example of a telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        let _clean = monitor.relay_stats_periodic(std::time::Duration::from_micros(gen_rate_micros)).await;

    }
    Ok(())
}

#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ChangeRequest>
                 , tx: SteadyTx<WidgetInventory>) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([&rx], [&tx]);

    let mut _rx = rx.lock().await;
    let mut tx = tx.lock().await;

    loop {
         relay_test(& mut monitor, &mut tx).await;
         monitor.relay_stats_smartly();
   }
}


#[cfg(test)]
async fn relay_test<const R:usize, const T:usize>(
                     monitor: &mut LocalMonitor<R,T>
                    , tx: &mut Tx<WidgetInventory>) {

    if let Some(responder) = monitor.sidechannel_responder() {

        responder.respond_with(|message| {
            let msg: &WidgetInventory = message.downcast_ref::<WidgetInventory>().expect("error casting");
            match monitor.try_send(tx, msg.clone()) {
                Ok(()) => Box::new("ok".to_string()),
                Err(m) => Box::new(m),
            }
        }).await;
    }
}




