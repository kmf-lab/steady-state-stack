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
use async_io::Async;
use bytes::BytesMut;
use lazy_static::lazy_static;
use log::{error, trace, warn};
use nuclei::config::{IoUringConfiguration, NucleiConfig};
use nuclei::Task;
use parking_lot::Once;
use crate::ProactorConfig;
use futures::{AsyncRead, AsyncWrite};
use futures_util::AsyncWriteExt;
use std::panic::{AssertUnwindSafe, catch_unwind};

// ## Traits and Implementations

/// A trait combining `AsyncRead` and `AsyncWrite` for types that support both.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// A trait for asynchronous listeners that can accept connections and provide their local address.
pub trait AsyncListener {
    /// Accepts a new connection asynchronously.
    ///
    /// Returns a future resolving to a stream implementing `AsyncReadWrite` and an optional socket address.
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>)>> + Send + 'a>>;

    /// Returns the local socket address of the listener.
    fn local_addr(&self) -> Result<SocketAddr>;
}

/// Implements `AsyncListener` for `Async<TcpListener>` to ensure true asynchronous acceptance.
impl AsyncListener for Async<TcpListener> {
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>)>> + Send + 'a>> {
        Box::pin(async move {
            let (stream, addr) = self.accept().await?;
            Ok((Box::new(stream) as Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Some(addr)))
        })
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        self.get_ref().local_addr()
    }
}

// ## Utility Functions

/// Binds a TCP listener to the specified address using `Async<TcpListener>`.
///
/// Returns an `Arc` containing the listener if successful, or `None` if binding fails.
pub fn bind_to_port(addr: &str) -> Arc<Option<Box<dyn AsyncListener + Send + Sync>>> {
    match TcpListener::bind(addr) {
        Ok(listener) => match Async::new(listener) {
            Ok(async_listener) => Arc::new(Some(Box::new(async_listener) as Box<dyn AsyncListener + Send + Sync>)),
            Err(e) => {
                error!("Unable to create async listener: {}", e);
                Arc::new(None)
            }
        },
        Err(e) => {
            error!("Unable to bind to http://{}: {}", addr, e);
            Arc::new(None)
        }
    }
}

/// Checks if an address can be bound to and returns the local address if successful.
pub(crate) fn check_addr(addr: &str) -> Option<String> {
    if let Ok(h) = TcpListener::bind(addr) {
        let local_addr = h.local_addr().expect("Unable to get local address");
        Some(format!("{}", local_addr))
    } else {
        None
    }
}
use std::io::Write;
// ## File I/O Function

/// Asynchronously writes data to a file, optionally flushing it.
///
/// Uses `spawn_blocking` to perform blocking file I/O in a separate thread.
pub(crate) async fn async_write_all(data: BytesMut, flush: bool, mut file: std::fs::File) -> Result<()> {
    spawn_blocking(move || {
        file.write_all(&data)?;
        if flush {
            file.flush()?;
        }
        Ok(())
    }).await
}

// ## Spawning Functions

/// Spawns a local task on the current thread for lightweight operations.
pub fn spawn_local<F: Future<Output = T> + 'static, T: 'static>(f: F) -> Task<T> {
    nuclei::spawn_local(f)
}

/// Spawns a blocking task on a separate thread for CPU-bound or blocking operations.
pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> Task<T> {
    nuclei::spawn_blocking(f)
}

/// Spawns a task that can run on any thread in the pool for parallel execution.
pub fn spawn<F: Future<Output = T> + Send + 'static, T: Send + 'static>(future: F) -> Task<T> {
    nuclei::spawn(future)
}

/// Asynchronously spawns additional threads in the executor.
pub async fn spawn_more_threads(count: usize) -> io::Result<usize> {
    nuclei::spawn_more_threads(count).await
}

// ## Initialization

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
    let nuclei_config = match proactor_config {
        ProactorConfig::InterruptDriven => IoUringConfiguration::interrupt_driven(queue_length),
        ProactorConfig::KernelPollDriven => IoUringConfiguration::kernel_poll_only(queue_length),
        ProactorConfig::LowLatencyDriven => IoUringConfiguration::low_latency_driven(queue_length),
        ProactorConfig::IoPoll => IoUringConfiguration::io_poll(queue_length),
    };

    INIT.call_once(|| {
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

// ## Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_init_without_driver() {
        let config = ProactorConfig::InterruptDriven;
        init(false, config, 256);
    }

    #[test]
    fn test_init_with_driver() {
        let config = ProactorConfig::InterruptDriven;
        init(true, config, 256);
        thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn test_block_on() {
        let future = async { 42 };
        let result = block_on(future);
        assert_eq!(result, 42);
    }
}