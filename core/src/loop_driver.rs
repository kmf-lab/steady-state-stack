use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// This macro waits for all the provided futures to complete.
/// It returns a boolean indicating if all futures returned true.
///
/// # Arguments
///
/// * `$t:expr` - A list of futures to wait for.
///

#[macro_export]
macro_rules! wait_for_all {
    ($($t:expr),*) => {
        async {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let _ = futures::join!($( wrap_bool_future(flag.clone(), $t) ),*);
            flag.load(std::sync::atomic::Ordering::Relaxed)
        }
    };
}

/// This macro waits for either the first future to complete, or all of the rest to complete.
/// It returns a boolean indicating if all futures (or the rest of the futures if the first completes) returned true.
///
/// # Arguments
///
/// * `$first_future:expr` - The first future to wait for.
/// * `$($rest_futures:expr),*` - The rest of the futures to wait for.
///

#[macro_export]
macro_rules! wait_for_all_or_proceed_upon {
    ($first_future:expr, $($rest_futures:expr),* $(,)?) => {
        async {
            use futures::future::FutureExt;
            use futures::pin_mut;
            use std::sync::Arc;
            use std::sync::atomic::{AtomicBool, Ordering};

            let flag = Arc::new(AtomicBool::new(true));

            // Fuse the first future and pin it
            let first = wrap_bool_future(flag.clone(), $first_future).fuse();
            pin_mut!(first);

            // Create the combined future for the rest and pin it
            let rest = async {
                futures::join!($(wrap_bool_future(flag.clone(), $rest_futures)),*)
            }.fuse();
            pin_mut!(rest);

            futures::select! {
                _ = first => {},
                _ = rest => {},
            };

            flag.load(Ordering::Relaxed)
        }
    };
}

/// Wraps a future that returns a boolean into one that updates a shared flag if it returns false.
///
/// # Arguments
///
/// * `flag` - An `Arc` of an `AtomicBool` shared between all wrapped futures.
/// * `fut` - The future to wrap.
///
/// # Returns
///
/// * `impl Future<Output = ()>` - A future that updates the flag if the input future returns false.
///
/// # Usage
///
/// This function is used internally by the macros `wait_for_all` and `wait_for_all_or_proceed_upon`.
///
/// ```rust
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicBool, Ordering};
/// use steady_state::wrap_bool_future;
/// let flag = Arc::new(AtomicBool::new(true));
/// let wrapped_future = wrap_bool_future(flag.clone(), async { true });
/// wrapped_future; //.await;
/// assert!(flag.load(Ordering::Relaxed));
/// ```
pub fn wrap_bool_future<F>(flag: Arc<AtomicBool>, fut: F) -> impl Future<Output = ()>
    where
        F: Future<Output = bool>,
{
    let flag = flag.clone();
    async move {
        match fut.await {
            true => {}
            false => {
                flag.store(false, Ordering::Relaxed);
            }
        };
    }
}

