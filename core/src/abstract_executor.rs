use std::future::{Future, ready};
use std::{mem, thread};
use std::mem::ManuallyDrop;
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
            nuclei::spawn_blocking(|| { nuclei::drive(InfiniteSleep); }).detach();
        }
    });
}


/*

        //TODO: switch to iouring http
        //TODO: hide remote jsviz
////////////////////////////////////////refactor to use the new nuclei::drive
struct Helper<F>(F);


impl<F: Fn() + Send + Sync + 'static> Helper<F> {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        Self::clone_waker,
        Self::wake,
        Self::wake_by_ref,
        Self::drop_waker,
    );

    unsafe fn clone_waker(ptr: *const ()) -> RawWaker {
        let arc = ManuallyDrop::new(Arc::from_raw(ptr as *const F));
        mem::forget(arc.clone());
        RawWaker::new(ptr, &Self::VTABLE)
    }

    unsafe fn wake(ptr: *const ()) {
        let arc = Arc::from_raw(ptr as *const F);
        (arc)();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        let arc = ManuallyDrop::new(Arc::from_raw(ptr as *const F));
        (arc)();
    }

    unsafe fn drop_waker(ptr: *const ()) {
        drop(Arc::from_raw(ptr as *const F));
    }


}

pub(crate) fn waker_fn<F: Fn() + Send + Sync + 'static>(f: F) -> Waker {
    let raw = Arc::into_raw(Arc::new(f)) as *const ();
    let vtable = &Helper::<F>::VTABLE;
    unsafe { Waker::from_raw(RawWaker::new(raw, vtable)) }
}


pub fn my_drive<T>(future: impl Future<Output = T>) -> T {

    let p = Proactor::get();
    //  waker_fn  Creates a waker from a wake function.
     //           The function gets called every time the waker is woken.
    let waker = futures_util::task::waker_fn(move || {
        p.wake();
    });

    let cx = &mut Context::from_waker(&waker);
    futures::pin_mut!(future);

    let driver = spawn_blocking(move || loop {
        let _ = p.wait(1, None);
    });

    futures::pin_mut!(driver);

    loop {
        if let Poll::Ready(val) = future.as_mut().poll(cx) {
            return val;
        }

        cx.waker().wake_by_ref();

        // TODO: (vcq): we don't need this.
        // let _duration = Duration::from_millis(1);
        let _ = driver.as_mut().poll(cx);
    }
}
*/
////////////////////////////////////////////////



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


