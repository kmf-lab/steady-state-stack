//! This module provides an abstraction layer for threading solutions, allowing for easier
//! swapping of threading implementations in the future.
//!
//! The module leverages the `nuclei` library for asynchronous execution and provides
//! utilities for initialization, spawning detached tasks, and blocking on futures.

// ===================================================================
// Imports
// ===================================================================

use core::net::SocketAddr;
use std::future::Future;
use std::{io, thread};
use std::fs::File;
use std::io::Error;
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread::sleep;
use std::time::Duration;
use bytes::BytesMut;
use lazy_static::lazy_static;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
#[allow(unused_imports)]
use log::{error, trace, warn};
use parking_lot::Once;
use crate::ProactorConfig;
use futures::{AsyncRead, AsyncWrite};
use futures_util::AsyncWriteExt;
use nuclei::Task;

// Removed unused imports: TcpStream, AsyncWriteExt, abstract_executor

// ===================================================================
// Traits and Implementations
// ===================================================================

/// A trait combining `AsyncRead` and `AsyncWrite`, used for types that can perform
/// both asynchronous reading and writing.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// A trait for asynchronous listeners that can accept connections and provide their local address.
pub trait AsyncListener {
    /// Accepts a new connection asynchronously.
    ///
    /// Returns a future that resolves to a stream implementing `AsyncReadWrite` and an optional socket address.
    ///
    /// # Returns
    /// A pinned boxed future yielding a `Result` containing:
    /// - A boxed `AsyncReadWrite` trait object that is `Send`, `Unpin`, and `'static`.
    /// - An optional `SocketAddr` representing the remote address.
    /// - An `io::Error` on failure.
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>), io::Error>> + Send + 'a>>;

    /// Returns the local socket address of the listener.
    ///
    /// # Returns
    /// A `Result` containing the `SocketAddr` or an `io::Error` if the address cannot be retrieved.
    fn local_addr(&self) -> Result<SocketAddr, io::Error>;
}

impl AsyncListener for nuclei::Handle<TcpListener> {
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>), io::Error>> + Send + 'a>> {
        let fut = self.accept(); // Provided by nuclei::Handle<TcpListener>
        Box::pin(async move {
            let (stream, addr) = fut.await?;
            Ok((Box::new(stream) as Box<dyn AsyncReadWrite + Send + Unpin + 'static>, addr))
        })
    }

    fn local_addr(&self) -> Result<SocketAddr, io::Error> {
        self.local_addr() // Provided by nuclei::Handle<TcpListener>
    }
}

// ===================================================================
// Utility Functions
// ===================================================================

/// Binds a TCP listener to the specified address using `nuclei::Handle<TcpListener>`.
///
/// If binding is successful, returns an `Arc` containing `Some(Box<dyn AsyncListener + Send + Sync>)`.
/// If binding fails, logs an error and returns `Arc::new(None)`.
///
/// # Arguments
/// * `addr` - The address to bind to, e.g., "127.0.0.1:8080".
///
/// # Returns
/// An `Arc` wrapping an `Option` containing the boxed listener or `None` on failure.
pub fn bind_to_port(addr: &str) -> Arc<Option<Box<dyn AsyncListener + Send + Sync>>> {
    match nuclei::Handle::<TcpListener>::bind(addr) {
        Ok(listener) => Arc::new(Some(Box::new(listener) as Box<dyn AsyncListener + Send + Sync>)),
        Err(e) => {
            error!("Unable to bind to http://{}: {}", addr, e);
            Arc::new(None)
        }
    }
}

/// Checks if binding to an address is possible and returns the local address if successful.
///
/// # Arguments
/// * `addr` - The address to check, e.g., "127.0.0.1:8080".
///
/// # Returns
/// An `Option<String>` containing the local address as a string if binding succeeds, or `None` if it fails.
pub(crate) fn check_addr(addr: &str) -> Option<String> {
    if let Ok(h) = nuclei::Handle::<TcpListener>::bind(addr) {
        let local_addr = h.local_addr().expect("Unable to get local address");
        Some(format!("{}", local_addr))
    } else {
        None
    }
}

/// Asynchronously writes data to a file using `nuclei::Handle<File>`.
///
/// # Arguments
/// * `data` - The data to write as a `BytesMut`.
/// * `flush` - Whether to flush the file after writing.
/// * `file` - The file to write to.
///
/// # Returns
/// A `Result<(), Error>` indicating success or failure.
pub(crate) async fn async_write_all(data: BytesMut, flush: bool, file: File) -> Result<(), Error> {
    let mut h = nuclei::Handle::<File>::new(file)?;
    h.write_all(data.as_ref()).await?;
    if flush {
        nuclei::Handle::<File>::flush(&mut h).await?;
    }
    Ok(())
}

/// Synchronously writes data to a file by driving the `async_write_all` function.
///
/// # Arguments
/// * `data` - The data to write as a `BytesMut`.
/// * `file` - The file to write to.
///
/// # Note
/// This function ignores the result of the write operation for simplicity.
pub(crate) fn test_write_all(data: BytesMut, file: File) {
    let _ = nuclei::drive(async_write_all(data, true, file));
}

// ===================================================================
// Spawning Functions
// ===================================================================

/// Spawns a local task that runs on the current thread.
///
/// Use this for lightweight tasks that do not need to be sent to other threads.
///
/// # Arguments
/// * `f` - The future to spawn.
///
/// # Returns
/// A `Task<T>` representing the spawned task.
pub fn spawn_local<F: Future<Output = T> + 'static, T: 'static>(f: F) -> Task<T> {
    nuclei::spawn_local(f)
}

/// Spawns a blocking task that runs on a separate thread.
///
/// Use this for CPU-bound or blocking operations.
///
/// # Arguments
/// * `f` - The closure to execute.
///
/// # Returns
/// A `Task<T>` representing the spawned task.
pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> Task<T> {
    nuclei::spawn_blocking(f)
}

/// Spawns a task that can be sent to other threads in the thread pool.
///
/// Use this for tasks that benefit from parallel execution.
///
/// # Arguments
/// * `future` - The future to spawn.
///
/// # Returns
/// A `Task<T>` representing the spawned task.
pub fn spawn<F: Future<Output = T> + Send + 'static, T: Send + 'static>(future: F) -> Task<T> {
    nuclei::spawn(future)
}

/// Asynchronously spawns additional threads in the executor.
///
/// # Arguments
/// * `count` - The number of additional threads to spawn.
///
/// # Returns
/// An `io::Result<usize>` indicating the number of threads spawned or an error.
pub async fn spawn_more_threads(count: usize) -> io::Result<usize> {
    nuclei::spawn_more_threads(count).await
}

// ===================================================================
// Initialization
// ===================================================================

lazy_static! {
    /// Ensures that initialization code runs only once across all threads.
    static ref INIT: Once = Once::new();
}

/// A future that parks the thread indefinitely to keep the executor running without consuming CPU.
///
/// This is used to maintain an interrupt-based executor without saturating a core.
///
/// # Note
/// This is a temporary solution and may need revisiting in future `nuclei` releases.
struct InfiniteSleep;

impl Future for InfiniteSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        cx.waker().wake_by_ref(); // Defensive, ensures the waker is registered
        trace!("InfiniteSleep started");
        thread::park();
        Poll::Pending
    }
}

/// Initializes the `nuclei` executor with the specified configuration.
///
/// If `enable_driver` is true, starts the IOUring driver in a separate thread to keep the executor active.
///
/// # Arguments
/// * `enable_driver` - Whether to start the IOUring driver.
/// * `proactor_config` - The configuration variant for the executor.
/// * `queue_length` - The length of the IOUring queue.
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
                        sleep(Duration::from_secs(1));
                        warn!("Restarting IOUring driver");
                    }
                }
            }).detach();
        }
    });
}

// ===================================================================
// Blocking Function
// ===================================================================

/// Blocks the current thread until the given future completes.
///
/// Useful for running asynchronous code in a synchronous context.
///
/// # Arguments
/// * `future` - The future to block on.
///
/// # Returns
/// The output of the completed future.
pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
    nuclei::block_on(future)
}

// ===================================================================
// Tests
// ===================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_init_without_driver() {
        // Test initialization without starting the IOUring driver
        let config = ProactorConfig::InterruptDriven;
        init(false, config, 256);
    }

    #[test]
    fn test_init_with_driver() {
        // Test initialization with the IOUring driver
        let config = ProactorConfig::InterruptDriven;
        init(true, config, 256);
        thread::sleep(Duration::from_millis(100)); // Allow driver to start
    }

    #[test]
    fn test_block_on() {
        // Test blocking on a simple async future
        let future = async { 42 };
        let result = block_on(future);
        assert_eq!(result, 42);
    }
}