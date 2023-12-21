use futures::{FutureExt, select};
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use crate::steady::*;

async fn process(mut monitor: &mut SteadyMonitor
                 , rx_approved_widgets: &SteadyRx<ApprovedWidgets>)  {

    match rx_approved_widgets.rx(&mut monitor ).await {
        Ok(m) => {
            info!("recieved: {:?}", m);
        },
        Err(e) => {}
    }

}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx_approved_widgets: SteadyRx<ApprovedWidgets>) -> Result<(),()> {
    process( &mut monitor
              , &mut rx_approved_widgets).await;
    Ok(())
}



#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx_approved_widgets: SteadyRx<ApprovedWidgets>) -> Result<(),()> {

    //let mut test_one:Option<RefAddr> = None;
    //let mut tel:Option<RefAddr> = None; //store in rx core..

    // waiting for the test framework to send a message



    match rx_approved_widgets.rx(&mut monitor).await {
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
    use async_std::task;

    #[async_std::test]
    async fn test_something() {

    }

}