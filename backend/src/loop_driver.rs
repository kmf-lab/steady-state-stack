use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use futures::select;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use futures::FutureExt;
use crate::{LocalMonitor, Rx, Tx};
use crate::monitor::{CALL_OTHER, CALL_WAIT};

//TODO: Tasks today:
//  update overlaping monitor durations for async calls, secondary counter required
//  finish macros for drivers
//  update examples and their cargo file.



#[macro_export]
macro_rules! wait_for_all {
    // This pattern matches against any number of arguments
    ($($t:tt)*) => {
        // Delegate to the `futures::join!` macro
        futures::join!($($t)*)
    };
}

macro_rules! wait_for_all_or_proceed_upon {
    ($first_future:expr, $($rest_futures:expr),* $(,)?) => {{
        // Use `futures::future::select` to wait on the first future
        // and a `futures::join!` for the rest.
        // `select` returns a `Select` future which completes when either of the futures completes.
        let rest = futures::future::join!($($rest_futures),*);
        futures::select! {
            first_res = $first_future => {
                // If the first future completes first, we can take action based on `first_res`
                // This block allows early return or other actions based on the first future's completion.
                first_res
            },
            rest_res = rest => {
                // If one of the rest of the futures completes first,
                // this block will execute. `rest_res` will be the result of `futures::join!`,
                // which is a tuple of all the results of the futures.
                rest_res
            },
        }
    }};
}


/*
  SinglePassDriver::new().
      with(monitor.avail_units(&mut rx, 1).
      with(monitor.with_vacant_units(&mut tx, 1).
      with(monitor.with_vacant_units(&mut feedback, 1).
      wait_or_proceed_upon(monitor.periodic(Duration::from_secs(1))).await;
      // or we call wait_after(monitor.periodic(Duration::from_secs(1))).await;
      // or we call wait_for_all().await;
 */

/*
pub struct SinglePassDriver<'a> {
    futures: Option<FuturesUnordered<Pin<Box<dyn Future<Output = bool> + Send + 'a>>>>,
}

impl <'a>SinglePassDriver<'a> {
    pub fn new() -> Self {
        //pass monitor OR context in?
        Self { futures: Some(FuturesUnordered::new()) }
    }

    pub fn with(&mut self, future: impl Future<Output = bool> + Send + 'a ) -> &mut Self {
        if let Some(f) = self.futures.as_mut() {

            //TODO: we need to stop early here on shutdown..
            //one for context and one for the local monitor

            f.push(Box::pin(future));
        }
        self
    }

    pub fn wait_avail_units<T>(&mut self, rx: &'a mut Rx<T>, i: usize) -> &mut Self
     where T: Send + Sync + 'a {
        if let Some(f) = self.futures.as_mut() {
            f.push(Box::pin(rx.shared_wait_avail_units(i)));
        }
        self
    }

    pub fn wait_vacant_units<T>(&mut self, tx: &'a mut Tx<T>, i: usize) -> &mut Self
        where T: Send + Sync + 'a {
        if let Some(f) = self.futures.as_mut() {
            f.push(Box::pin(tx.shared_wait_vacant_units(i)));
        }
        self
    }



    async fn evaluate_conditions(&mut self) -> bool {
        let result = Arc::new(AtomicBool::new(true)); // Initial value is true for AND operation
        if let Some(f) = self.futures.take() {
            f.for_each_concurrent(None, |r| {
                let result_clone = result.clone();
                async move {
                    if !r {result_clone.store(false, Ordering::SeqCst);}
                }
            }).await;
        }
        result.load(Ordering::SeqCst)
    }


    ////////////////////////////////////////////////

    //TODO: need the context versions

    pub async fn wait_or_proceed_upon<const RX_LEN:usize, const TX_LEN:usize >(&mut self, monitor: &mut LocalMonitor<RX_LEN, TX_LEN>, duration: Duration) -> bool {
        monitor.start_hot_profile(CALL_WAIT);
        let result = select! {
            //if periodic gets done first we return
            //with a join!( we can do this as a macro here
            a = monitor.relay_stats_periodic(duration).fuse() => a,

            //this requies the above select.
            b = self.evaluate_conditions().fuse() => b,
        };
        monitor.rollup_hot_profile();
        result
    }


    pub async fn wait_after<const RX_LEN:usize, const TX_LEN:usize >(&'a mut self, monitor: &'a mut LocalMonitor<RX_LEN, TX_LEN>, duration: Duration) -> bool {
        monitor.start_hot_profile(CALL_WAIT);
        let result = {
            //with a join!( we can do this as a macro here
            let a:bool = self.evaluate_conditions().await;
            //relay stats has last call time so we only wait the remaining
            let b:bool = monitor.relay_stats_periodic(duration).await;
            a&b
        };
        monitor.rollup_hot_profile();
        result
    }


    pub async fn wait_for_conditions(&mut self) -> bool {
        //with a comma list we can do this with join!( macro
        self.evaluate_conditions().await
    }



}
//   */


