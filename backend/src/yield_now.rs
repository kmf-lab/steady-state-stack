use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A simple future that yields once before completing, using a tuple struct for simplicity.
pub(crate) struct YieldNow(bool);

impl YieldNow {
    fn new() -> Self {
        YieldNow(false)  // false indicates it has not yielded yet
    }
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Creates a future that yields once before completing.
/// We use the more verbose impl Future here just to be sure
/// nothing is jumping int to make use of the heap.
pub fn yield_now() -> impl Future<Output = ()> {
    YieldNow::new()
}


