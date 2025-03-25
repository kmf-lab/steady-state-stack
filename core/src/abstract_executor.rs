//! This module provides an abstraction layer for threading solutions, allowing for easier
//! swapping of threading implementations in the future.
//!
//! The module leverages the `nuclei` library for asynchronous execution and provides
//! utilities for initialization, spawning detached tasks, and blocking on futures.

use core::net::SocketAddr;
use std::future::Future;
use std::{io, thread};
use std::fs::File;
use std::io::Error;
use std::net::{TcpListener, TcpStream};
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
use log::*;
use parking_lot::Once;
use crate::{abstract_executor, ProactorConfig};
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};
use nuclei::Task;


pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}



pub trait AsyncListener {
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>), io::Error>> + Send + 'a>>;
    fn local_addr(&self) -> Result<SocketAddr, io::Error>;
}


impl AsyncListener for nuclei::Handle<TcpListener> {
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>), io::Error>> + Send + 'a>> {
        let fut = self.accept(); // Assuming this is an inherent method of Handle<TcpListener>
        Box::pin(async move {
            let (stream, addr) = fut.await?;
            Ok((Box::new(stream) as Box<dyn AsyncReadWrite + Send + Unpin + 'static>, addr))
        })
    }

    fn local_addr(&self) -> Result<SocketAddr, io::Error> {
        self.local_addr() // Assuming this is an inherent method of Handle<TcpListener>
    }
}


pub fn bind_to_port(addr: &String) -> Arc<Option<Box<dyn AsyncListener + Send + Sync>>> {
    match nuclei::Handle::<TcpListener>::bind(addr) {
        Ok(listener) => Arc::new(Some(Box::new(listener) as Box<dyn AsyncListener + Send + Sync>)),
        Err(e) => {
            eprintln!("Unable to bind to http://{}: {}", addr, e);
            Arc::new(None)
        }
    }
}

pub(crate) fn check_addr(addr: &str) -> Option<String> {
    if let Ok(h) = nuclei::Handle::<TcpListener>::bind(addr) {
        let local_addr = h.local_addr().expect("Unable to get local address");
        Some(format!("{}", local_addr).to_string())
    } else {
        None
    }
}

pub(crate) async fn async_write_all(data: BytesMut, flush: bool, file: File) -> Result<(), Error> {
    let mut h = nuclei::Handle::<File>::new(file)?;
    h.write_all(data.as_ref()).await?;
    if flush {
        nuclei::Handle::<File>::flush(&mut h).await?;
    }
    Ok(())
}
pub(crate) fn test_write_all(data: BytesMut, file: File) {
    let _ = nuclei::drive(abstract_executor::async_write_all(data, true, file));
}

pub fn spawn_local<F: Future<Output = T> + 'static, T: 'static>(f: F) -> Task<T> {
    nuclei::spawn_local(f)
}
pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> Task<T> {
    nuclei::spawn_blocking(f)
}
pub fn spawn<F: Future<Output = T> + Send + 'static, T: Send + 'static>(future: F) -> Task<T> {
    nuclei::spawn(future)
}
pub async fn spawn_more_threads(count: usize) -> io::Result<usize> {
    nuclei::spawn_more_threads(count).await
}


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
pub(crate) fn init(enable_driver: bool, proactor_config: ProactorConfig, queue_length: u32) {
    //setup our threading and IO driver
    let nuclei_config = match proactor_config {
        ProactorConfig::InterruptDriven => IoUringConfiguration::interrupt_driven(queue_length),
        ProactorConfig::KernelPollDriven => IoUringConfiguration::kernel_poll_only(queue_length),
        ProactorConfig::LowLatencyDriven => IoUringConfiguration::low_latency_driven(queue_length),
        ProactorConfig::IoPoll => IoUringConfiguration::io_poll(queue_length),
    };

    INIT.call_once(|| {
    //TODO: none of this works, we must submit an issue and example..
        // //set the enviornment variable t0 127
        // std::env::set_var("ASYNC_GLOBAL_EXECUTOR_THREADS", "127");
        // std::env::set_var("BLOCKING_MAX_THREADS", "127");
        //
        // nuclei::init_with_config(
        //
        //     nuclei::GlobalExecutorConfig::default()
        //         .with_env_var("ASYNC_GLOBAL_EXECUTOR_THREADS")
        //         .with_min_threads(3)
        //         .with_max_threads(1000),
        // );

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
            })
                .detach();
        }
    });
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
pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
    nuclei::block_on(future) // Block until the future is resolved
}



#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // fn dummy_waker() -> Waker {
    //     fn noop_clone(_: *const ()) -> RawWaker {
    //         dummy_raw_waker()
    //     }
    //
    //     fn noop(_: *const ()) {}
    //
    //     fn dummy_raw_waker() -> RawWaker {
    //         RawWaker::new(std::ptr::null(), &RawWakerVTable::new(noop_clone, noop, noop, noop))
    //     }
    //
    //     unsafe { Waker::from_raw(dummy_raw_waker()) }
    // }

    // #[test]
    // fn test_infinite_sleep() {
    //     let mut sleep_future = InfiniteSleep;
    //     let waker = dummy_waker();
    //     let mut cx = Context::from_waker(&waker);
    //
    //     // Poll the InfiniteSleep future
    //     let result = Pin::new(&mut sleep_future).poll(&mut cx);
    //
    //     // Ensure the future returns Poll::Pending
    //     assert!(matches!(result, Poll::Pending));
    // }

    #[test]
    fn test_init_without_driver() {
        let config = ProactorConfig::InterruptDriven;

        // Initialize the executor without the IOUring driver
        init(false, config, 256);

        // Ensure the INIT Once is initialized
        //assert!(INIT.is_completed());
    }

    #[test]
    fn test_init_with_driver() {
        let config = ProactorConfig::InterruptDriven;

        // Initialize the executor with the IOUring driver
        init(true, config,256);

        // Wait for a moment to let the driver start
        thread::sleep(Duration::from_millis(100));

        // Ensure the INIT Once is initialized
        //assert!(INIT.is_completed());
    }

    // #[test]
    // fn test_spawn_detached() {
    //     // Use an Arc and Mutex to share state between the main task and the detached task
    //     let flag = Arc::new(Mutex::new(false));
    //     let flag_clone = flag.clone();
    //
    //     let lock = Arc::new(Mutex::new(()));
    //     // Spawn a detached task that sets the flag to true
    //     spawn_detached(lock, async move {
    //         let mut flag = flag_clone.lock().unwrap();
    //         *flag = true;
    //     });
    //
    //     // Wait for a moment to let the detached task run
    //     thread::sleep(Duration::from_millis(100));
    //
    //     // Check if the flag is set to true
    //     assert!(*flag.lock().unwrap());
    // }

    #[test]
    fn test_block_on() {
        // Define a future that returns 42
        let future = async { 42 };

        // Block on the future and get the result
        let result = block_on(future);

        // Ensure the result is 42
        assert_eq!(result, 42);
    }
}
