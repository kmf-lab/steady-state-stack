
use std::time::Duration;
use crate::steady::{SteadyTx, SteadyMonitor};

#[derive(Clone, Debug)]
pub struct WidgetInventory {
    pub(crate) count: u128
}

struct InternalState {
    pub(crate) count: u128
}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor
                     , mut tx: SteadyTx<WidgetInventory> ) -> Result<(),()> {
    let mut state = InternalState { count: 0 };
    loop {
        //single pass of work, do not loop in here
        if iterate_once(&mut monitor, &mut state, &mut tx).await {
            break Ok(());
        }
        monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
    }

}

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor
                      , mut tx: SteadyTx<WidgetInventory>) -> Result<(),()> {
    let mut state = InternalState { count: 0 };
    loop {
        //single pass of work, do not loop in here
        if iterate_once(&mut monitor, &mut state, &mut tx).await {
            break Ok(());
        }
        monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
    }
}

async fn iterate_once(monitor: &mut SteadyMonitor
                      , state: &mut InternalState
                      , tx_widget: &SteadyTx<WidgetInventory> ) -> bool
{
    monitor.tx(tx_widget, WidgetInventory {count: state.count.clone() }).await;
    state.count += 1;
    false
}


#[cfg(test)]
mod tests {
    use crate::actor::{WidgetInventory};
    use crate::actor::data_generator::{InternalState, iterate_once};
    use crate::steady::{SteadyGraph, SteadyTx};

    #[async_std::test]
    async fn test_something() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx, rx): (SteadyTx<WidgetInventory>, _) = graph.new_channel(8);

        let mut monitor = graph.new_monitor().await.wrap("test", None); //TODO: smelly

        let mut state = InternalState { count: 0 };

        let exit = iterate_once(&mut monitor, &mut state, &tx).await;   //TODO: smelly
        assert_eq!(exit, false);


// TODO: need more testing

        /*
    #[async_std::test]
    async fn test_iterate_once() {
        let (sender, receiver): (Sender<WidgetInventory>, Receiver<WidgetInventory>) = channel();

        let mut state = InternalState { count: 0 };
        let monitor = MockSteadyMonitor;
        let tx_widget = MockSteadyTx { sender };

        // Call iterate_once and test its effects
        iterate_once(&mut monitor, &mut state, &mut tx_widget).await;

        // Check that state.count has been incremented
        assert_eq!(state.count, 1);

        // Verify that a WidgetInventory was sent
        match receiver.try_recv() {
            Ok(widget_inventory) => assert_eq!(widget_inventory.count, 0),
            Err(e) => panic!("Expected a WidgetInventory, but got an error: {:?}", e),
        }
    }
    //  */
    }

}