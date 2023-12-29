use std::sync::Arc;
use std::time::Duration;
use crate::steady::*;


#[derive(Clone, Debug)]
pub enum DiagramData {
   // Structure(),
    //Content(),
}



pub(crate) async fn telemetry(_monitor: SteadyMonitor
                              , all_rx: Arc<RwLock<Vec<SteadyRx<Telemetry>>>>
                              , _tx: SteadyTx<DiagramData>
) -> Result<(),()> {
    loop {
        let all = all_rx.read().await;

        let _:Vec<_> = all.iter().collect();
        //TODO: read from all these pipes and create our telemetry
        Delay::new(Duration::from_millis(20)).await;
        if false {
            break Ok(());
        }
    }
}