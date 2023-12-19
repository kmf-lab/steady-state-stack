use std::time::Duration;
use futures::select;
use futures_timer::Delay;
use crate::steady::{SteadyTx, SteadyMonitor};
use futures::FutureExt;

#[derive(Clone, Debug)]
pub struct WidgetInventory {
    pub(crate) count: u128
}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor
                     , mut tx: SteadyTx<WidgetInventory> ) -> Result<(),()> {
    loop {
        select! {
            _ = monitor.relay_stats().await => {},
            _ = process( &mut monitor
                       , &mut tx
                         ).fuse() => {}
        }
    }
    Ok(())
}

//  mut tx: SteadyTx<WidgetInventory>  ,   let args = (&mut tx,);
// can we pass T down? from main?

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, mut tx: SteadyTx<WidgetInventory>) -> Result<(),()> {
    loop {
        select! {
            _ = monitor.relay_stats().await => {},
            _ = process( &mut monitor
                       , &mut tx
                         ).fuse() => {}
        }
    }
    Ok(())
}



// Define a generic behavior function with flexible arguments


// (&mut SteadyTx<WidgetInventory>,)

async fn process(monitor: &mut SteadyMonitor
                          , tx_widget: &mut SteadyTx<WidgetInventory> )
{
    let mut counter:u128 = 0;
    loop {
        tx_widget.tx(monitor, WidgetInventory {count:counter }).await;
        counter += 1;
        Delay::new(Duration::from_secs(3)).await
    }
}



#[cfg(test)]
mod tests {


    #[async_std::test]
    async fn test_something() {


    }

}