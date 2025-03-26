
#[cfg(any(feature = "proactor_nuclei", feature = "proactor_tokio"))]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, allowing for easier
    //! swapping of threading implementations in the future.
    //!
    //! The module leverages the `nuclei` library for asynchronous execution and provides
    //! utilities for initialization, spawning detached tasks, and blocking on futures.

    // ## Imports
    use std::net::{SocketAddr, TcpListener};
    use std::future::Future;
    use std::io::{self, Result};
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::thread;
    use std::time::Duration;
    use bytes::BytesMut;
    use lazy_static::lazy_static;
    use log::{error, trace, warn};
    use nuclei::config::{IoUringConfiguration, NucleiConfig};
    use parking_lot::Once;
    use crate::ProactorConfig;
    use futures::{AsyncRead, AsyncWrite};
    use futures_util::AsyncWriteExt;
    use std::panic::{catch_unwind, AssertUnwindSafe};


    /// Spawns a local task on the current thread for lightweight operations.
    pub fn spawn_local<F: Future<Output=T> + 'static, T: 'static>(f: F) -> nuclei::Task<T> {
        nuclei::spawn_local(f)
    }

    /// Spawns a blocking task on a separate thread for CPU-bound or blocking operations.
    pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> nuclei::Task<T> {
        nuclei::spawn_blocking(f)
    }

    /// Spawns a task that can run on any thread in the pool for parallel execution.
    pub fn spawn<F: Future<Output=T> + Send + 'static, T: Send + 'static>(future: F) -> nuclei::Task<T> {
        nuclei::spawn(future)
    }

    /// Asynchronously spawns additional threads in the executor.
    pub async fn spawn_more_threads(count: usize) -> io::Result<usize> {
        nuclei::spawn_more_threads(count).await
    }

    lazy_static! {
        /// Ensures initialization runs only once across all threads.
        static ref INIT: Once = Once::new();
    }

    /// A future that parks the thread indefinitely to keep the executor running without CPU usage.
    struct InfiniteSleep;

    impl Future for InfiniteSleep {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            cx.waker().wake_by_ref();
            trace!("InfiniteSleep started");
            thread::park();
            Poll::Pending
        }
    }

    /// Initializes the `nuclei` executor with the specified configuration.
    pub(crate) fn init(enable_driver: bool, proactor_config: ProactorConfig, queue_length: u32) {
        INIT.call_once(|| {
            let nuclei_config = match proactor_config {
                ProactorConfig::InterruptDriven => IoUringConfiguration::interrupt_driven(queue_length),
                ProactorConfig::KernelPollDriven => IoUringConfiguration::kernel_poll_only(queue_length),
                ProactorConfig::LowLatencyDriven => IoUringConfiguration::low_latency_driven(queue_length),
                ProactorConfig::IoPoll => IoUringConfiguration::io_poll(queue_length),
            };

            let _ = nuclei::Proactor::with_config(NucleiConfig { iouring: nuclei_config });
            if enable_driver {
                trace!("Starting IOUring driver");
                nuclei::spawn_blocking(|| {
                    loop {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            nuclei::drive(InfiniteSleep);
                        }));
                        if let Err(e) = result {
                            error!("IOUring Driver panicked: {:?}", e);
                            thread::sleep(Duration::from_secs(1));
                            warn!("Restarting IOUring driver");
                        }
                    }
                }).detach();
            }
        });
    }

    // ## Blocking Function

    /// Blocks the current thread until the given future completes.
    pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
        nuclei::block_on(future)
    }

}
