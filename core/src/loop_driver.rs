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
            // we do not check the rest if the early one is blocked since it saves cycles
            $(
                let _ = wrap_bool_future(flag.clone(), $t).await;
            )*
            flag.load(std::sync::atomic::Ordering::Relaxed)
        }.await
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
                $(
                    let next = wrap_bool_future(flag.clone(), $rest_futures).fuse();
                    pin_mut!(next);
                    next.await;
                )*
            }.fuse();
            pin_mut!(rest);

            futures::select! {
                _ = first => {},
                _ = rest => {},
            };

            flag.load(Ordering::Relaxed)
        }.await
    };
}

/// This macro waits for any of the provided futures to complete.
/// It returns a boolean indicating if any one of the futures returned true.
///
/// # Arguments
///
/// * `$($t:expr),*` - A list of futures to wait for.
///
#[macro_export]
macro_rules! wait_for_any {
    ($($t:expr),* $(,)?) => {
        async {
            use futures::future::FutureExt;
            use futures::pin_mut;
            use futures::future::select_all;
            use std::sync::Arc;
            use std::sync::atomic::{AtomicBool, Ordering};

            let flag = Arc::new(AtomicBool::new(false));
            let mut futures_vec = Vec::new();

            $(
                futures_vec.push(wrap_bool_future(flag.clone(), $t).boxed());
            )*

            // Wait for any of the futures to complete
            let (res, _index, _remaining) = select_all(futures_vec).await;
            res;  // This ensures the future has been awaited

            flag.load(Ordering::Relaxed)
        }.await
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



#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::ready;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    use std::future::Future;
    use std::time::Duration;
    use futures::executor::block_on;
    use futures::future::{join};
    use futures_timer::Delay;

    #[test]
    fn test_wrap_bool_future_true() {
        let flag = Arc::new(AtomicBool::new(true));
        let wrapped = wrap_bool_future(flag.clone(), ready(true));

        block_on(wrapped);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn test_wrap_bool_future_false() {
        let flag = Arc::new(AtomicBool::new(true));
        let wrapped = wrap_bool_future(flag.clone(), ready(false));

        block_on(wrapped);
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[async_std::test]
    async fn test_wait_for_all_true() {
        let future1 = ready(true);
        let future2 = ready(true);
        let future3 = ready(true);

        let result = wait_for_all!(future1, future2, future3);
        assert!(result);
    }

    #[async_std::test]
    async fn test_wait_for_all_false() {
        let future1 = ready(true);
        let future2 = ready(false);
        let future3 = ready(true);

        let result = wait_for_all!(future1, future2, future3);
        assert!(!result);
    }

    #[async_std::test]
    async fn test_wait_for_all_or_proceed_upon_first_complete() {
        let future1 = ready(true);
        let future2 = async {
            Delay::new(Duration::from_millis(10)).await;
            true
        };
        let future3 = async {
            Delay::new(Duration::from_millis(10)).await;
            true
        };

        let result = wait_for_all_or_proceed_upon!(future1, future2, future3);
        assert!(result);
    }

    #[async_std::test]
    async fn test_wait_for_all_or_proceed_upon_rest_complete() {
        let future1 = async {
            Delay::new(Duration::from_millis(10)).await;
            true
        };
        let future2 = ready(true);
        let future3 = ready(true);

        let result = wait_for_all_or_proceed_upon!(future1, future2, future3);
        assert!(result);
    }

    #[async_std::test]
    async fn test_wait_for_all_or_proceed_upon_false() {
        let future1 = async {
            Delay::new(Duration::from_millis(10)).await;
            true
        };
        let future2 = ready(false);
        let future3 = ready(true);

        let result = wait_for_all_or_proceed_upon!(future1, future2, future3);
        assert!(!result);
    }
}
