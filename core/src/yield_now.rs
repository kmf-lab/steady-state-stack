//! This module provides a simple future that yields once before completing.
//!
//! The `YieldNow` struct and the `yield_now` function are used to create a future
//! that will yield (i.e., return `Poll::Pending`) once before it completes. This is
//! useful for cooperative multitasking scenarios where you want to allow other tasks to run.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A simple future that yields once before completing, using a tuple struct for simplicity.
///
/// This struct is used internally to create a future that will yield (i.e., return `Poll::Pending`)
/// once before it completes. It is useful for cooperative multitasking scenarios where you want
/// to allow other tasks to run.
pub(crate) struct YieldNow(bool);

impl YieldNow {
    /// Creates a new `YieldNow` instance.
    ///
    /// This function initializes a new `YieldNow` future that will yield once before completing.
    ///
    fn new() -> Self {
        YieldNow(false)  // false indicates it has not yielded yet
    }
}

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
pub fn yield_now() -> impl Future<Output = ()> {
    YieldNow::new()
}

