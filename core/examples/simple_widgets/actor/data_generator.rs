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



#[derive(Clone, Debug, Copy)]
struct InternalState {
    pub(crate) count: u64,

}

#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , feedback: SteadyRx<ChangeRequest>
                 , tx: SteadyTx<WidgetInventory> ) -> Result<(),Box<dyn Error>> {
    //trace!("running {:?} {:?}",context.id(),context.name());

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

        iterate_once(&mut monitor, &mut state, &mut tx, MULTIPLIER).await;

        if let Some(feedback) = monitor.try_take(&mut feedback) {
              trace!("data_generator feedback: {:?}", feedback);
        }

        //Using this block to test the panic and stop support
        //if (65536<<2)==state.count {
        //    return monitor.stop().await; //stop the actor
        //    panic!("This is a panic");
        //}

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
            info!("relay_test: {:?}", message);

            let msg: &WidgetInventory = message.downcast_ref::<WidgetInventory>().expect("error casting");
            match monitor.try_send(tx, msg.clone()) {
                Ok(()) => Box::new("ok".to_string()),
                Err(m) => Box::new(m),
            }
        }).await;

    }

}



async fn iterate_once<const R: usize,const T: usize>(monitor: & mut LocalMonitor<R, T>
                      , state: &mut InternalState
                      , tx_widget: &mut Tx<WidgetInventory>
    , multiplier: usize
                    ) -> bool
{
    let mut wids = Vec::with_capacity(multiplier);

    (0..=multiplier)
        .for_each(|num|
            wids.push(
                WidgetInventory {
                count: state.count+num as u64,
                _payload: 42,
        }));

    state.count+= multiplier as u64;




    let sent = monitor.send_slice_until_full(tx_widget, &wids);
    //iterator of sent until the end
    let consume = wids.into_iter().skip(sent);
    for send_me in consume {
        let _ = monitor.send_async(tx_widget, send_me, SendSaturation::Warn).await;
    }

    false
}


#[cfg(test)]
mod tests {
    use std::ops::DerefMut;
    use crate::actor::data_generator::{InternalState, iterate_once};
    use steady_state::{Graph, util};

    #[async_std::test]
    async fn test_iterate_once() {
        util::logger::initialize();

        let block_fail_fast = false;
        let mut graph = Graph::internal_new("", block_fail_fast, false);
        let (tx, rx) = graph.channel_builder().with_capacity(5).build();

        let mock_monitor = graph.new_test_monitor("generator_monitor");
        let mut mock_monitor = mock_monitor.into_monitor([&rx], [&tx]);

        let mut steady_tx_guard = tx.lock().await;
        let mut steady_rx_guard = rx.lock().await;
        let steady_tx = steady_tx_guard.deref_mut();
        let steady_rx = steady_rx_guard.deref_mut();
        let mut state = InternalState {
            count: 10,
        };
        let exit = iterate_once(&mut mock_monitor, &mut state, steady_tx,1).await;
        assert_eq!(exit, false);

        let record = mock_monitor.take_async(steady_rx).await.unwrap();
        assert_eq!(record.count, 10);
    }

}