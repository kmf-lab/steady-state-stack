use std::sync::Arc;
use std::time::Duration;
use crate::steady::*;

    //TODO: the u128 seq and count types should be moved out as type aliases based on use case need.

#[derive(Clone, Debug)]
pub enum DiagramData {
    //only allocates new space when new actor children are added.
    Structure(u128, & 'static str, usize, Arc<Vec<usize>>, Arc<Vec<usize>>),
    //all consumers will share the same seq vec and it is dropped when the last one consumed it
    //this copy was required so we can gather the next seq while the last gets rendered.
    Content(u128, Arc<Vec<u128>>, Arc<Vec<u128>>),
}

struct InternalState {
    total_sent: Vec<u128>,
    total_take: Vec<u128>,
    actor_count: usize,
    sequence: u128,
}

pub(crate) async fn run<const CONSUMERS: usize>(mut monitor: SteadyMonitor
                                                , all_rx: Arc<RwLock<Vec< CollectorDetail >>>
                                                , to_telemetry_consumers: [SteadyTx<DiagramData>; CONSUMERS]
) -> Result<(),()> {

    //TODO: may need custom method for this
    //      OUR RULES SAY THAT WE CAN ADD NEW ACTORS BUT WHAT IS MONITORED IS FIXED.
    //      THIS IS THE ACTOR DOING THE MONITORING SO THIS CAUSING A PROBLEM (BUT ONLY HERE)
    //let rx_def = all_rx.read().await.as_slice();
    //monitor.init_stats(&[], &to_telemetry_consumers); //ALSO NEED TO CAST DOWN CONSUMERS
    //NOTE: child actors builders do not have the tx end of this same channel so its not in stats either.

    let mut state = InternalState {
                total_take: Vec::new(),
                total_sent: Vec::new(),
                actor_count: 0,
                sequence: 0,
    };

    let outgoing = to_telemetry_consumers.clone();
    loop {
        //
        //the monitored channels per actor of set once on startup but we can add more actors if needed dynamically
        //
        let seq = state.sequence.clone(); //every frame has a unique sequence number
        state.sequence += 1;
        let locked_collector = all_rx.read().await; //we want to drop this as soon as we can

        if locked_collector.len() > state.actor_count {
            //get the max channel ids for the new actors
            let (max_rx, max_tx):(usize,usize) = locked_collector.iter().skip(state.actor_count).map(|x| {
                  ( x.telemetry_take.biggest_rx_id(), x.telemetry_take.biggest_tx_id() )
            }).fold((0,0), |mut acc, x| {
                if x.0 > acc.0 {acc.0 = x.0;}
                if x.1 > acc.1 {acc.1 = x.1;}
                acc
            });
            //grow our vecs as needed for the max ids found
            state.total_take.resize(max_rx, 0);
            state.total_sent.resize(max_tx, 0);

            //NOTE: sending data to consumers of the telemetry only happens once every 32ms or so
            //      it should be a s light weight as possible but not as critical as the collector
            //

            //send new actor definitions to our listeners
            let skip_amount = state.actor_count;
            let total_length = locked_collector.len();

            for i in skip_amount..total_length {
               let details = &locked_collector[i];
               let tt = &details.telemetry_take;
               let rxids:Arc<Vec<usize>> = Arc::new(tt.rx_ids_iter().collect());
               let txids:Arc<Vec<usize>> = Arc::new(tt.tx_ids_iter().collect());
               for c in to_telemetry_consumers.iter() {
                    let _ = monitor.tx(&c, DiagramData::Structure(seq
                                                  , details.name, details.id
                                                  , rxids.clone()
                                                  , txids.clone())).await;
               }
            };
        }
        for x in locked_collector.iter() {
                x.telemetry_take.consume_into(&mut state.total_take, &mut state.total_sent);
        }
        drop(locked_collector);

        let tr = Arc::new(state.total_take.clone()); //every one can see this but no one can change it
        let ts = Arc::new(state.total_sent.clone()); //every one can see this but no one can change it

        if outgoing.iter().all(|x| x.has_room()) {
            for tx in outgoing.clone() {
                let _ = monitor.tx(&tx, DiagramData::Content(seq, tr.clone(), ts.clone())).await;
            }
        }

        //NOTE: target 32ms updates for 30FPS, with a queue of 8 so writes must be no faster than 4ms
        //      we could double this speed if we have real animation needs but that is unlikely

        Delay::new(Duration::from_millis(features::TELEMETRY_PRODUCTION_RATE_MS as u64)).await;
        if false {
            break Ok(());
        }
    }
}