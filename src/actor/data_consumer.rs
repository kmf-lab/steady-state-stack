
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use crate::steady::*;

struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>
}

async fn iterate_once(monitor: &mut SteadyMonitor
                 , state: &mut InternalState
                 , rx_approved_widgets: &SteadyRx<ApprovedWidgets>) -> bool  {

    //by design we wait here for new work
    match monitor.rx(rx_approved_widgets).await {
        Ok(m) => {
            state.last_approval = Some(m.to_owned());
            info!("recieved: {:?}", m.to_owned());
        },
        Err(e) => {
            state.last_approval = None;
            error!("Unexpected error recv_async: {}",e);
        }
    }
    false

}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx_approved_widgets: SteadyRx<ApprovedWidgets>) -> Result<(),()> {
    let mut state = InternalState { last_approval: None };
    loop {
        //single pass of work, do not loop in here
        if iterate_once(&mut monitor, &mut state, &mut rx_approved_widgets).await {
            break Ok(());
        }
        monitor.relay_stats_all().await;
    }
}

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, rx_approved_widgets: SteadyRx<ApprovedWidgets>) -> Result<(),()> {

    //let mut test_one:Option<RefAddr> = None;
    //let mut tel:Option<RefAddr> = None; //store in rx core..

    // waiting for the test framework to send a message

 //   loop {
        //single pass of work, do not loop in here
  //      process( &mut monitor, &mut rx_approved_widgets).await;
   //     monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
   // }

    match monitor.rx(&rx_approved_widgets).await {
        Ok(m) => {
            //  send to the unit test
            //  sc.tell(&test_one.unwrap(), m).expect("Unable to send test message");
        },
        Err(e) => {
            error!("Unable to read: {}",e);
        }
    }

    // test init  vs prod init
    // both have telmetry if feature on
/*
    MessageHandler::new(ctx.recv().await?)
        .on_broadcast(|message: &SteadyBeacon, _sender_addr| {
            if let SteadyBeacon::TestCase(addr, case) = message {
                if "One" == case {
                    test_one = Some(addr.clone());
                }
                // Handle the message...
                println!("Received TestCase with case: {}", case);
                // Potentially send a message back using addr
            }
        })
        .on_broadcast(|message: &SteadyBeacon, _sender_addr| {
            if let SteadyBeacon::Telemetry(addr) = message {
                tel = Some(addr.clone());
              //  init_actor(tel); //todo rebuild is a problem becuse broadcast wil be gone.
                // Handle the message...
                println!("Received target: {:?}", addr);
                // Potentially send a message back using addr
            }

        });
//  */
    // now we can run the tests


    Ok(())
}




#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;
    use flexi_logger::{Logger, LogSpecification};

    #[test]
    async fn test_something() {

        let _ = Logger::with(LogSpecification::env_or_parse("info").unwrap())
            .format(flexi_logger::colored_with_thread)
            .start();

        let mut graph = SteadyGraph::new();
        let (tx, rx): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(8);
        let mut monitor = graph.new_monitor().await.wrap("test",None);

        monitor.tx(&tx, ApprovedWidgets {
            original_count: 1,
            approved_count: 2
        }).await;

        let mut state = InternalState { last_approval: None };

        let exit= iterate_once(&mut monitor, &mut state, &rx).await;

        assert_eq!(exit, false);
        let widgets = state.last_approval.unwrap();
        assert_eq!(widgets.original_count, 1);
        assert_eq!(widgets.approved_count, 2);

    }

}