//! This module provides a simple future that yields once before completing.
//!
//! The `YieldNow` struct and the `yield_now` function are used to create a future
//! that will yield (i.e., return `Poll::Pending`) once before it completes. This is
//! useful for cooperative multitasking scenarios where you want to allow other tasks to run.

// ss[impl philosophy.mechanical-sympathy]
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A simple future that yields once before completing, using a tuple struct for simplicity.
///
/// This struct is used internally to create a future that will yield (i.e., return `Poll::Pending`)
/// once before it completes. It is useful for cooperative multitasking scenarios where you want
/// to allow other tasks to run.
// ss[impl philosophy.mechanical-sympathy]
pub struct YieldNow(bool);

impl YieldNow {
    /// Creates a new `YieldNow` instance.
    ///
    /// This function initializes a new `YieldNow` future that will yield once before completing.
    ///
    // ss[impl philosophy.mechanical-sympathy]
    fn new() -> Self {
        YieldNow(false)  // false indicates it has not yielded yet
    }
}

// ss[impl philosophy.mechanical-sympathy]
impl Future for YieldNow {
    type Output = ();

    /// Polls the future to determine if it is ready to complete.
    ///
    /// This method will yield (return `Poll::Pending`) once before completing.
    /// When the future is first polled, it sets its internal state to indicate that it has yielded,
    /// and then it wakes up the task context. On subsequent polls, it returns `Poll::Ready(())`.
    ///
    /// # Arguments
    ///
    /// * `self` - A mutable pinned reference to the future.
    /// * `cx` - The task context used for waking up the task.
    ///
    /// # Returns
    ///
    /// * `Poll::Pending` - If the future is yielding.
    /// * `Poll::Ready(())` - If the future is ready to complete.
    ///
    // ss[impl philosophy.mechanical-sympathy]
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
///
/// This function returns a future that will yield (i.e., return `Poll::Pending`) once before it
/// completes. It is useful for cooperative multitasking scenarios where you want to allow other
/// tasks to run.
///
// ss[impl philosophy.mechanical-sympathy]
// ss[impl graph.troupes]
pub fn yield_now() -> impl Future<Output = ()> {
    YieldNow::new()
}

#[cfg(test)]
// ss[impl philosophy.mechanical-sympathy]
mod tests {
    use super::*;
    use std::task::{Context, Poll, Waker};
    // ss[impl philosophy.mechanical-sympathy]
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc};

    // A simple waker that does nothing, used for testing.
    // ss[impl philosophy.mechanical-sympathy]
    fn noop_waker() -> Waker {
        struct NoopWaker;
        impl std::task::Wake for NoopWaker {
            // ss[impl philosophy.mechanical-sympathy]
            fn wake(self: Arc<Self>) {}
        }

        Arc::new(NoopWaker).into()
    }

    #[test]
    // ss[verify philosophy.mechanical-sympathy]
    fn test_yield_now_initially_pending() {
        let mut yield_now = yield_now();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // The first poll should return Poll::Pending
        assert_eq!(Pin::new(&mut yield_now).poll(&mut cx), Poll::Pending);
    }

    #[test]
    // ss[verify philosophy.mechanical-sympathy]
    fn test_yield_now_ready_after_yielding() {
        let mut yield_now = yield_now();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // The first poll should return Poll::Pending
        assert_eq!(Pin::new(&mut yield_now).poll(&mut cx), Poll::Pending);

        // The second poll should return Poll::Ready(())
        assert_eq!(Pin::new(&mut yield_now).poll(&mut cx), Poll::Ready(()));
    }

    // #[test]
    // fn test_yield_now_wakes_correctly() {
    //     let mut yield_now = yield_now();
    //     let (sender, receiver): (SyncSender<()>, Receiver<()>) = sync_channel(1);
    //     let waker = waker_fn::waker_fn(move || {
    //         let _ = sender.send(());
    //     });
    //     let mut cx = Context::from_waker(&waker);
    //
    //     // The first poll should return Poll::Pending and trigger the waker
    //     assert_eq!(Pin::new(&mut yield_now).poll(&mut cx), Poll::Pending);
    //
    //     // Ensure the waker was called
    //     assert!(receiver.try_recv().is_ok());
    //
    //     // The second poll should return Poll::Ready(())
    //     assert_eq!(Pin::new(&mut yield_now).poll(&mut cx), Poll::Ready(()));
    // }
}


