use std::future::{Future, ready};
use std::{mem, thread};
use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread::sleep;
use std::time::Duration;
use futures_timer::Delay;
use lazy_static::lazy_static;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
#[allow(unused_imports)]
use log::*;
use nuclei::{Proactor, spawn_blocking};
use parking_lot::{Mutex, Once};
use crate::yield_now::yield_now;


lazy_static! {
    static ref INIT: Once = Once::new();
}

struct InfiniteSleep;
impl Future for InfiniteSleep {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        cx.waker().wake_by_ref(); //not strictly needed, defencive
        trace!("InfiniteSleep Started");
        thread::park();
        Poll::Pending
    }
}
//call this on startup:  nuclei::spawn_blocking(|| { nuclei::drive(InfiniteSleep); }).detach();

pub(crate) fn init(enable_driver: bool, nuclei_config: IoUringConfiguration) {

    INIT.call_once(|| {
        let _ = nuclei::Proactor::with_config(NucleiConfig { iouring: nuclei_config});
        nuclei::init_with_config(nuclei::GlobalExecutorConfig::default()
                                .with_min_threads(3)
                                .with_max_threads(usize::MAX));
        if enable_driver {
            nuclei::spawn_blocking(
                || {
                    loop {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            nuclei::drive(InfiniteSleep);
                        }));
                        if let Err(e) = result {
                            error!("IOuring Driver panicked: {:?}", e);
                            sleep(Duration::from_secs(1));
                            warn!("Restarting IOuring driver");
                        }
                    }
                }
            ).detach();
        }
    });
}




pub(crate) fn spawn_detached<F, T>(future: F)
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
{ //only used for supervisors who will be blocked running the actor most of the time.
    nuclei::spawn(async move {
        match nuclei::spawn_more_threads(1).await {
            Ok(_) => {}
            Err(_) => {} //log max thread issues?
        };
        future.await;
    }).detach();  // Spawn an async task with nuclei
}

pub(crate) fn block_on<F, T>(future: F) -> T
    where
        F: Future<Output = T> + Send + 'static,
        T: Send,
{
    nuclei::block_on(future)  // Block until the future is resolved
}


