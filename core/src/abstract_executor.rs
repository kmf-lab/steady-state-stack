//! Bare-metal executor: OS threads plus nestable `futures_lite::future::block_on`.
//!
//! Optional `tokio` feature swaps `block_on` for a **current-thread** Tokio runtime on the
//! calling OS thread (SOLO/TROUP). Actors are not spawned onto a Tokio pool and do not
//! gain `Send` bounds. Do not use `futures::executor::block_on` here — it is not nestable
//! and fails graph-build nested awaits with `EnterError`.

pub(crate) mod core_exec {
    // ss[impl platform.executor-features]
    use futures::FutureExt;
    use futures::channel::oneshot;
    use futures_util::future::FusedFuture;
    use log::warn;
    use std::any::Any;
    use std::error::Error;
    use std::future::Future;
    use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
    use std::pin::Pin;
    use std::thread;

    /// Spawns a future on a dedicated OS thread and detaches it.
    // ss[impl platform.executor-features]
    pub fn spawn_detached<F: Future<Output = T> + Send + 'static, T: Send + 'static>(future: F) {
        thread::spawn(move || {
            let _ = block_on(future);
        });
    }

    #[cfg(all(unix, feature = "libc"))]
    // ss[related platform.executor-features]
    fn get_current_core() -> Option<usize> {
        let cpu = unsafe { libc::sched_getcpu() };
        if cpu >= 0 {
            Some(cpu as usize)
        } else {
            None
        }
    }

    #[cfg(all(windows, feature = "winapi"))]
    // ss[related platform.executor-features]
    fn get_current_core() -> Option<usize> {
        let cpu = unsafe { winapi::um::processthreadsapi::GetCurrentProcessorNumber() };
        if cpu != 0xFFFFFFFF {
            Some(cpu as usize)
        } else {
            None
        }
    }

    #[cfg(not(any(all(unix, feature = "libc"), all(windows, feature = "winapi"))))]
    // ss[related platform.executor-features]
    fn get_current_core() -> Option<usize> {
        None
    }

    #[cfg(all(unix, feature = "libc"))]
    // ss[related platform.executor-features]
    fn set_thread_affinity(core: usize) -> std::result::Result<(), Box<dyn Error>> {
        use libc::{cpu_set_t, pthread_self, pthread_setaffinity_np};
        let mut cpu_set: cpu_set_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::CPU_SET(core, &mut cpu_set);
            let res = pthread_setaffinity_np(
                pthread_self(),
                std::mem::size_of::<libc::cpu_set_t>(),
                &cpu_set,
            );
            if res == 0 {
                Ok(())
            } else {
                Err("Unable to set affinity".into())
            }
        }
    }

    #[cfg(all(windows, feature = "winapi"))]
    // ss[related platform.executor-features]
    fn set_thread_affinity(core: usize) -> std::result::Result<(), Box<dyn Error>> {
        use winapi::shared::basetsd::DWORD_PTR;
        use winapi::um::processthreadsapi::GetCurrentThread;

        let mask = 1u64 << core;
        let res = unsafe {
            winapi::um::winbase::SetThreadAffinityMask(GetCurrentThread(), mask as DWORD_PTR)
        };
        if res != 0 {
            Ok(())
        } else {
            Err("unable to set affinity on windows due to mask failure".into())
        }
    }

    #[cfg(not(any(all(unix, feature = "libc"), all(windows, feature = "winapi"))))]
    // ss[related platform.executor-features]
    fn set_thread_affinity(_core: usize) -> std::result::Result<(), Box<dyn Error>> {
        Ok(())
    }

    /// Runs a blocking closure on a new OS thread, optionally pinned to the caller's core.
    // ss[impl platform.executor-features]
    pub fn spawn_blocking<F, T>(f: F) -> Pin<Box<dyn futures::future::FusedFuture<Output = T> + Send>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let current_core = get_current_core();
        let (sender, receiver) =
            oneshot::channel::<std::result::Result<T, Box<dyn Any + Send + 'static>>>();
        thread::spawn(move || {
            if let Some(core) = current_core {
                if let Err(e) = set_thread_affinity(core) {
                    warn!(
                        "Affinity for blocking tasks was enabled but unable to set due to '{:?}', will run blocking task on another core.",
                        e
                    );
                }
            }
            let result = catch_unwind(AssertUnwindSafe(|| f()));
            if let Err(_e) = sender.send(result) {
                warn!("blocking job finished but the receiver is no longer attached");
            }
        });

        Box::pin(
            async move {
                let result = receiver.await.expect("Sender dropped");
                match result {
                    Ok(t) => t,
                    Err(e) => resume_unwind(e),
                }
            }
            .fuse(),
        ) as Pin<Box<dyn FusedFuture<Output = T> + Send>>
    }

    /// Drives `future` to completion on the current OS thread.
    ///
    /// With the `tokio` feature this uses a fresh current-thread Tokio runtime so a panic
    /// in the future drops that runtime and the next restart (SOLO/TROUP) gets a new one.
    // ss[impl platform.executor-features]
    // ss[verify platform.executor-features]
    pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
        #[cfg(feature = "tokio")]
        {
            // Nested block_on during graph build (mutex/oneshot) must not create a second runtime
            // on a thread that already entered Tokio.
            if tokio::runtime::Handle::try_current().is_ok() {
                return futures_lite::future::block_on(future);
            }
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio current-thread runtime")
                .block_on(future)
        }
        #[cfg(not(feature = "tokio"))]
        {
            // Nestable park/unpark (async-std's block_on was nestable; futures::executor is not).
            futures_lite::future::block_on(future)
        }
    }
}

#[cfg(test)]
// ss[related platform.executor-features]
mod executor_tests {
    use super::core_exec::{block_on, spawn_blocking, spawn_detached};
    use crate::ss_proptest;
    use futures_timer::Delay;
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_flag(flag: &AtomicBool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if flag.load(Ordering::SeqCst) {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        flag.load(Ordering::SeqCst)
    }

    #[test]
    // ss[verify platform.executor-features]
    fn test_block_on() {
        let result = block_on(async { 42 });
        assert_eq!(result, 42);
    }

    #[test]
    // ss[verify platform.executor-features]
    fn test_spawn_detached() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();
        spawn_detached(async move {
            flag_clone.store(true, Ordering::SeqCst);
        });
        assert!(wait_flag(&flag, Duration::from_secs(2)));
    }

    #[test]
    // ss[verify platform.executor-features]
    fn test_spawn_blocking() {
        let result = block_on(async { spawn_blocking(|| 9).await });
        assert_eq!(result, 9);
    }

    #[test]
    // ss[verify platform.executor-features]
    fn test_spawn_blocking_panic_propagates() {
        let result = std::panic::catch_unwind(|| {
            block_on(async { spawn_blocking(|| panic!("blocking panic test")).await })
        });
        assert!(result.is_err());
    }

    ss_proptest! {
        /// Property: `block_on` returns the future's value.
        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_block_on_returns_value(n in 0i32..10_000) {
            let got = block_on(async move { n.wrapping_mul(3).wrapping_add(1) });
            prop_assert_eq!(got, n.wrapping_mul(3).wrapping_add(1));
        }

        /// Property: `spawn_blocking` returns the closure value.
        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_spawn_blocking_returns_value(n in 0i32..10_000) {
            let got = block_on(async { spawn_blocking(move || n * 2).await });
            prop_assert_eq!(got, n * 2);
        }

        /// Property: `spawn_detached` eventually stores the side effect.
        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_spawn_detached_completes(seed in 0u8..=255) {
            let flag = Arc::new(AtomicBool::new(false));
            let flag_clone = flag.clone();
            spawn_detached(async move {
                let _ = seed;
                flag_clone.store(true, Ordering::SeqCst);
            });
            prop_assert!(wait_flag(&flag, Duration::from_secs(3)));
        }

        /// Property: concurrent `block_on` on distinct OS threads does not require a global runtime.
        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_concurrent_block_on(n in 1usize..6) {
            let sum = Arc::new(AtomicUsize::new(0));
            let mut handles = Vec::new();
            for i in 0..n {
                let sum = sum.clone();
                handles.push(thread::spawn(move || {
                    let v = block_on(async move { i + 1 });
                    sum.fetch_add(v, Ordering::SeqCst);
                }));
            }
            for h in handles {
                h.join().expect("worker");
            }
            prop_assert_eq!(sum.load(Ordering::SeqCst), (1..=n).sum::<usize>());
        }

        /// Property: `futures_timer::Delay` completes under `block_on`.
        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_delay_completes_under_block_on(ms in 1u64..20) {
            let start = Instant::now();
            block_on(async move {
                Delay::new(Duration::from_millis(ms)).await;
            });
            prop_assert!(start.elapsed() >= Duration::from_millis(ms));
        }
    }
}
