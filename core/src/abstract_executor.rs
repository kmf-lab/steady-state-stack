//! This module provides an abstraction layer for threading solutions, allowing for easier
//! swapping of threading implementations in the future.
//!
//! The module leverages the `nuclei` library for asynchronous execution and provides
//! utilities for initialization, spawning detached tasks, and blocking on futures.

use std::future::Future;
use std::thread;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::sleep;
use std::time::Duration;
use lazy_static::lazy_static;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
#[allow(unused_imports)]
use log::*;
use parking_lot::Once;

lazy_static! {
    static ref INIT: Once = Once::new();
}

/// A future that sleeps indefinitely.
///
/// This future is used to keep a thread parked until it is explicitly woken up.
/// It is typically used with the `nuclei::drive` function to keep the executor running.
///
/// # Note
///
/// This is a creative hack to remain interrupt-based. If we did not park this thread,
/// it would saturate one core. This approach may need to be revisited in future releases of `nuclei`.
struct InfiniteSleep;

impl Future for InfiniteSleep {
    type Output = ();

    /// Polls the future to determine if it is ready to complete.
    ///
    /// This method will park the current thread indefinitely, effectively blocking it.
    ///
    /// # Note
    ///
    /// This is a creative hack to remain interrupt-based. If we did not park this thread,
    /// it would saturate one core. This approach may need to be revisited in future releases of `nuclei`.
    ///
    /// # Arguments
    ///
    /// * `self` - A pinned reference to the future.
    /// * `cx` - The task context used for waking up the task.
    ///
    /// # Returns
    ///
    /// * `Poll::Pending` - Always returns `Poll::Pending` as this future never completes.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        cx.waker().wake_by_ref(); // Not strictly needed, defensive
        trace!("InfiniteSleep started");
        thread::park();
        Poll::Pending
    }
}

/// Initializes the executor and, optionally, the IOUring driver.
///
/// This function initializes the `nuclei` executor with the given configuration and,
/// if `enable_driver` is set to `true`, starts the IOUring driver in a separate thread.
///
/// # Arguments
///
/// * `enable_driver` - A boolean indicating whether to start the IOUring driver.
/// * `nuclei_config` - The configuration for IOUring.
///
pub(crate) fn init(enable_driver: bool, nuclei_config: IoUringConfiguration) {
    INIT.call_once(|| {
        let _ = nuclei::Proactor::with_config(NucleiConfig { iouring: nuclei_config });
        nuclei::init_with_config(
            nuclei::GlobalExecutorConfig::default()
                .with_min_threads(3)
                .with_max_threads(usize::MAX),
        );

        if enable_driver {
            nuclei::spawn_blocking(|| {
                loop {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        nuclei::drive(InfiniteSleep);
                    }));

                    if let Err(e) = result {
                        error!("IOUring Driver panicked: {:?}", e);
                        sleep(Duration::from_secs(1));
                        warn!("Restarting IOUring driver");
                    }
                }
            })
                .detach();
        }
    });
}

/// Spawns a future as a detached task.
///
/// This function spawns an asynchronous task that runs the given future. The task is detached,
/// meaning it runs independently and its result is not awaited.
///
/// # Arguments
///
/// * `future` - The future to run as a detached task.
///
pub(crate) fn spawn_detached<F, T>(future: F)
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
{
    nuclei::spawn(async move {
        if let Err(e) = nuclei::spawn_more_threads(1).await {
            error!("Failed to spawn one more thread: {:?}", e);
        };
        future.await;
    })
        .detach(); // Spawn an async task with nuclei
}

/// Blocks the current thread until the given future resolves.
///
/// This function blocks the current thread, running the given future to completion.
/// It is useful for running asynchronous code from a synchronous context.
///
/// # Arguments
///
/// * `future` - The future to block on.
///
/// # Returns
///
/// The result of the future.
///
pub(crate) fn block_on<F, T>(future: F) -> T
    where
        F: Future<Output = T> + Send + 'static,
        T: Send,
{
    nuclei::block_on(future) // Block until the future is resolved
}


