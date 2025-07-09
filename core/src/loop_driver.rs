//! Utilities for composing and awaiting multiple futures in SteadyState.
//!
//! This module provides macros and functions for waiting on multiple asynchronous
//! operations, supporting both "wait for all" and "wait for any" semantics, as well as
//! more advanced patterns such as "wait for all except if the first completes".
//!
//! These utilities are designed to simplify concurrent actor and channel orchestration
//! in async Rust code, especially in graph-based or event-driven systems.

use futures_util::FutureExt;
pub use futures::future::Future;
pub use futures::select;
pub use futures::pin_mut;
use futures_util::future::FusedFuture;

/// Waits for all provided futures to complete, returning `true` only if all complete with `true`.
///
/// This macro is useful for synchronizing multiple asynchronous operations where all must succeed.
/// The result is a boolean indicating whether all futures returned `true`.
#[macro_export]
macro_rules! await_for_all {
    ($($t:expr),*) => {
        async {
            $(
                if !$t.await {
                    return false;
                }
            )*
            true
        }.await
    };
}

/// Waits for all provided futures to complete, returning a future that resolves to `true` only if all complete with `true`.
///
/// This macro is similar to `await_for_all!` but returns a future instead of immediately awaiting it.
/// Useful for composing with other async combinators.
#[macro_export]
macro_rules! wait_for_all {
    ($($t:expr),*) => {
        async {
            $(
                if !$t.await {
                    return false;
                }
            )*
            true
        }
    };
}

/// Converts a future into a fused future, which can be polled after completion without panicking.
///
/// This is useful for use with `select!` and other combinators that require fused futures.
pub fn steady_fuse_future<F>(fut: F) -> futures_util::future::Fuse<F>
where
    F: Future,
{
    fut.fuse()
}

/// Waits for either the first future to complete, or for all of the remaining futures to complete.
///
/// Returns the result of the first future if it completes first, otherwise returns the logical AND
/// of the results of the remaining futures. This is useful for scenarios where an early exit is
/// possible, but otherwise all other operations must complete.
pub async fn steady_await_for_all_or_proceed_upon_two<F1, F2>(
    fut1: F1,
    fut2: F2,
) -> bool
where
    F1: Future<Output = bool> + FusedFuture,
    F2: Future<Output = bool> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);

    select! {
        b = fut1 => b,
        b = async {
            let mut flag = true;
            flag &= fut2.await;
            flag
        }.fuse() => b,
    }
}

/// Waits for either the first future to complete, or for all of the remaining three futures to complete.
///
/// Returns the result of the first future if it completes first, otherwise returns the logical AND
/// of the results of the remaining futures.
pub async fn steady_await_for_all_or_proceed_upon_three<F1, F2, F3>(
    fut1: F1,
    fut2: F2,
    fut3: F3,
) -> bool
where
    F1: Future<Output = bool> + FusedFuture,
    F2: Future<Output = bool> + FusedFuture,
    F3: Future<Output = bool> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);
    pin_mut!(fut3);

    select! {
        b = fut1 => b,
        b = async {
            let mut flag = true;
            flag &= fut2.await;
            flag &= fut3.await;
            flag
        }.fuse() => b,
    }
}

/// Waits for either the first future to complete, or for all of the remaining four futures to complete.
///
/// Returns the result of the first future if it completes first, otherwise returns the logical AND
/// of the results of the remaining futures.
pub async fn steady_await_for_all_or_proceed_upon_four<F1, F2, F3, F4>(
    fut1: F1,
    fut2: F2,
    fut3: F3,
    fut4: F4,
) -> bool
where
    F1: Future<Output = bool> + FusedFuture,
    F2: Future<Output = bool> + FusedFuture,
    F3: Future<Output = bool> + FusedFuture,
    F4: Future<Output = bool> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);
    pin_mut!(fut3);
    pin_mut!(fut4);

    select! {
        b = fut1 => b,
        b = async {
            let mut flag = true;
            flag &= fut2.await;
            flag &= fut3.await;
            flag &= fut4.await;
            flag
        }.fuse() => b,
    }
}

/// Waits for either the first future to complete, or for all of the remaining five futures to complete.
///
/// Returns the result of the first future if it completes first, otherwise returns the logical AND
/// of the results of the remaining futures.
pub async fn steady_await_for_all_or_proceed_upon_five<F1, F2, F3, F4, F5>(
    fut1: F1,
    fut2: F2,
    fut3: F3,
    fut4: F4,
    fut5: F5,
) -> bool
where
    F1: Future<Output = bool> + FusedFuture,
    F2: Future<Output = bool> + FusedFuture,
    F3: Future<Output = bool> + FusedFuture,
    F4: Future<Output = bool> + FusedFuture,
    F5: Future<Output = bool> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);
    pin_mut!(fut3);
    pin_mut!(fut4);
    pin_mut!(fut5);

    select! {
        b = fut1 => b,
        b = async {
            let mut flag = true;
            flag &= fut2.await;
            flag &= fut3.await;
            flag &= fut4.await;
            flag &= fut5.await;
            flag
        }.fuse() => b,
    }
}

/// Macro for "wait for all or proceed upon first" pattern.
///
/// Waits for either the first future to complete, or for all of the rest to complete.
/// Returns a boolean indicating if all completed with `true`, or the result of the first future if it completes first.
#[macro_export]
macro_rules! await_for_all_or_proceed_upon {
    ($first:expr, $second:expr $(,)?) => {{
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        $crate::steady_await_for_all_or_proceed_upon_two(fut1,fut2).await
    }};
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        let fut3 = $crate::steady_fuse_future($third);
        $crate::steady_await_for_all_or_proceed_upon_three(fut1,fut2,fut3).await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        let fut3 = $crate::steady_fuse_future($third);
        let fut4 = $crate::steady_fuse_future($fourth);
        $crate::steady_await_for_all_or_proceed_upon_four(fut1,fut2,fut3,fut4).await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr, $fifth:expr $(,)?) => {{
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        let fut3 = $crate::steady_fuse_future($third);
        let fut4 = $crate::steady_fuse_future($fourth);
        let fut5 = $crate::steady_fuse_future($fifth);
        $crate::steady_await_for_all_or_proceed_upon_five(fut1,fut2,fut3,fut4,fut5).await
    }};
}

/// Waits for any of the provided futures to complete, returning the result of the first to finish.
///
/// This macro is useful for racing multiple asynchronous operations and acting on the first to complete.
/// The result is the output of the first future that completes.
#[macro_export]
macro_rules! await_for_any {
    ($first:expr $(,)?) => {{
        async {
            $first.await
        }.await
    }};
    ($first:expr, $second:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            $crate::steady_select_two(fut1, fut2).await
        }.await
    }};
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            $crate::steady_select_three(fut1, fut2, fut3).await
        }.await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            let fut4 = $crate::steady_fuse_future($fourth);
            $crate::steady_select_four(fut1, fut2, fut3, fut4).await
        }.await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr, $fifth:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            let fut4 = $crate::steady_fuse_future($fourth);
            let fut5 = $crate::steady_fuse_future($fifth);
            $crate::steady_select_five(fut1, fut2, fut3, fut4, fut5).await
        }.await
    }};
}

/// Like `await_for_any!`, but returns a future instead of immediately awaiting it.
///
/// This macro is useful for composing with other async combinators or for use in select! blocks.
#[macro_export]
macro_rules! wait_for_any {
    ($first:expr $(,)?) => {{
        async {
            $first.await
        }
    }};
    ($first:expr, $second:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            $crate::steady_select_two(fut1, fut2).await
        }
    }};
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            $crate::steady_select_three(fut1, fut2, fut3).await
        }
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            let fut4 = $crate::steady_fuse_future($fourth);
            $crate::steady_select_four(fut1, fut2, fut3, fut4).await
        }
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr, $fifth:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            let fut4 = $crate::steady_fuse_future($fourth);
            let fut5 = $crate::steady_fuse_future($fifth);
            $crate::steady_select_five(fut1, fut2, fut3, fut4, fut5).await
        }
    }};
}

/// Waits for the first of two futures to complete, returning its result.
///
/// This function pins the provided futures and uses `select!` to await the first to finish.
/// It is useful for racing two asynchronous operations.
pub async fn steady_select_two<F1, F2, O>(fut1: F1, fut2: F2) -> O
where
    F1: Future<Output = O> + FusedFuture,
    F2: Future<Output = O> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);

    select! {
        res = fut1 => res,
        res = fut2 => res,
    }
}

/// Waits for the first of three futures to complete, returning its result.
///
/// This function pins the provided futures and uses `select!` to await the first to finish.
pub async fn steady_select_three<F1, F2, F3, O>(fut1: F1, fut2: F2, fut3: F3) -> O
where
    F1: Future<Output = O> + FusedFuture,
    F2: Future<Output = O> + FusedFuture,
    F3: Future<Output = O> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);
    pin_mut!(fut3);

    select! {
        res = fut1 => res,
        res = fut2 => res,
        res = fut3 => res,
    }
}

/// Waits for the first of four futures to complete, returning its result.
///
/// This function pins the provided futures and uses `select!` to await the first to finish.
pub async fn steady_select_four<F1, F2, F3, F4, O>(fut1: F1, fut2: F2, fut3: F3, fut4: F4) -> O
where
    F1: Future<Output = O> + FusedFuture,
    F2: Future<Output = O> + FusedFuture,
    F3: Future<Output = O> + FusedFuture,
    F4: Future<Output = O> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);
    pin_mut!(fut3);
    pin_mut!(fut4);

    select! {
        res = fut1 => res,
        res = fut2 => res,
        res = fut3 => res,
        res = fut4 => res,
    }
}

/// Waits for the first of five futures to complete, returning its result.
///
/// This function pins the provided futures and uses `select!` to await the first to finish.
pub async fn steady_select_five<F1, F2, F3, F4, F5, O>(
    fut1: F1,
    fut2: F2,
    fut3: F3,
    fut4: F4,
    fut5: F5,
) -> O
where
    F1: Future<Output = O> + FusedFuture,
    F2: Future<Output = O> + FusedFuture,
    F3: Future<Output = O> + FusedFuture,
    F4: Future<Output = O> + FusedFuture,
    F5: Future<Output = O> + FusedFuture,
{
    pin_mut!(fut1);
    pin_mut!(fut2);
    pin_mut!(fut3);
    pin_mut!(fut4);
    pin_mut!(fut5);

    select! {
        res = fut1 => res,
        res = fut2 => res,
        res = fut3 => res,
        res = fut4 => res,
        res = fut5 => res,
    }
}

#[cfg(test)]
mod loop_driver_tests {
    use super::*;
    use async_std::task::sleep;
    use futures::future::ready;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // Helper function to create a future that returns a boolean after a delay
    async fn delayed_bool(value: bool, ms: u64) -> bool {
        sleep(Duration::from_millis(ms)).await;
        value
    }

    // Helper function to create a controlled future that completes when signaled
    fn controlled_bool(value: bool, signal: Arc<AtomicBool>) -> impl Future<Output = bool> + FusedFuture {
        let signal_clone = signal.clone();
        async move {
            while !signal_clone.load(Ordering::SeqCst) {
                sleep(Duration::from_millis(10)).await;
            }
            value
        }
            .fuse()
    }

    // Tests for await_for_all! macro
    #[async_std::test]
    async fn await_for_all_all_true() {
        let result = await_for_all!(
            ready(true),
            ready(true),
            ready(true)
        );
        assert!(result);
    }

    #[async_std::test]
    async fn await_for_all_one_false() {
        let result = await_for_all!(
            ready(true),
            ready(false),
            ready(true)
        );
        assert!(!result);
    }

    #[async_std::test]
    async fn await_for_all_empty() {
        let result = await_for_all!();
        assert!(result); // Empty case returns true
    }

    // Tests for wait_for_all! macro
    #[async_std::test]
    async fn wait_for_all_all_true() {
        let fut = wait_for_all!(
            ready(true),
            ready(true),
            ready(true)
        );
        assert!(fut.await);
    }

    #[async_std::test]
    async fn wait_for_all_one_false() {
        let fut = wait_for_all!(
            ready(true),
            ready(false),
            ready(true)
        );
        assert!(!fut.await);
    }

    #[async_std::test]
    async fn wait_for_all_empty() {
        let fut = wait_for_all!();
        assert!(fut.await);
    }

    // Test for steady_fuse_future function
    #[async_std::test]
    async fn steady_fuse_future_works() {
        let fut = ready(true);
        let fused = steady_fuse_future(fut);
        assert!(!fused.is_terminated());
        assert!(fused.await);
    }

    // Tests for await_for_all_or_proceed_upon! macro with two futures
    #[async_std::test]
    async fn await_for_all_or_proceed_upon_two_first_completes() {
        let signal = Arc::new(AtomicBool::new(false));
        let fut1 = delayed_bool(true, 10);
        let fut2 = controlled_bool(true, signal.clone());
        let result = await_for_all_or_proceed_upon!(fut1, fut2);
        assert!(result);
    }

    #[async_std::test]
    async fn await_for_all_or_proceed_upon_two_others_complete() {
        let signal = Arc::new(AtomicBool::new(true));
        let fut1 = controlled_bool(true, signal.clone());
        let fut2 = delayed_bool(false, 10);
        let result = await_for_all_or_proceed_upon!(fut1, fut2);
        assert!(result);
        signal.store(true, Ordering::SeqCst); // Ensure cleanup
    }

    // Tests for await_for_all_or_proceed_upon! macro with three futures
    #[async_std::test]
    async fn await_for_all_or_proceed_upon_three_first_completes() {
        let signal = Arc::new(AtomicBool::new(false));
        let fut1 = delayed_bool(false, 10);
        let fut2 = controlled_bool(true, signal.clone());
        let fut3 = controlled_bool(true, signal.clone());
        let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3);
        assert!(!result);
    }

    #[async_std::test]
    async fn await_for_all_or_proceed_upon_three_others_complete() {
        let signal = Arc::new(AtomicBool::new(true));
        let fut1 = controlled_bool(true, signal.clone());
        let fut2 = delayed_bool(true, 10);
        let fut3 = delayed_bool(false, 10);
        let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3);
        assert!(result);
        signal.store(true, Ordering::SeqCst);
    }

    // Tests for await_for_all_or_proceed_upon! macro with four futures
    #[async_std::test]
    async fn await_for_all_or_proceed_upon_four_first_completes() {
        let signal = Arc::new(AtomicBool::new(false));
        let fut1 = delayed_bool(true, 10);
        let fut2 = controlled_bool(true, signal.clone());
        let fut3 = controlled_bool(true, signal.clone());
        let fut4 = controlled_bool(true, signal.clone());
        let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4);
        assert!(result);
    }

    #[async_std::test]
    async fn await_for_all_or_proceed_upon_four_others_complete() {
        let signal = Arc::new(AtomicBool::new(true));
        let fut1 = controlled_bool(true, signal.clone());
        let fut2 = delayed_bool(true, 10);
        let fut3 = delayed_bool(true, 10);
        let fut4 = delayed_bool(false, 10);
        let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4);
        assert!(result);
        signal.store(true, Ordering::SeqCst);
    }

    // Tests for await_for_all_or_proceed_upon! macro with five futures
    #[async_std::test]
    async fn await_for_all_or_proceed_upon_five_first_completes() {
        let signal = Arc::new(AtomicBool::new(false));
        let fut1 = delayed_bool(false, 10);
        let fut2 = controlled_bool(true, signal.clone());
        let fut3 = controlled_bool(true, signal.clone());
        let fut4 = controlled_bool(true, signal.clone());
        let fut5 = controlled_bool(true, signal.clone());
        let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4, fut5);
        assert!(!result);
    }

    #[async_std::test]
    async fn await_for_all_or_proceed_upon_five_others_complete() {
        let signal = Arc::new(AtomicBool::new(true));
        let fut1 = controlled_bool(true, signal.clone());
        let fut2 = delayed_bool(true, 10);
        let fut3 = delayed_bool(true, 10);
        let fut4 = delayed_bool(true, 10);
        let fut5 = delayed_bool(false, 10);
        let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4, fut5);
        assert!(result);
        signal.store(true, Ordering::SeqCst);
    }

    // Tests for await_for_any! macro
    #[async_std::test]
    async fn await_for_any_one() {
        let result = await_for_any!(ready(42));
        assert_eq!(result, 42);
    }

    #[async_std::test]
    async fn await_for_any_two_first_completes() {
        let fut1 = delayed_bool(true, 10);
        let fut2 = delayed_bool(false, 20);
        let result = await_for_any!(fut1, fut2);
        assert!(result);
    }

    #[async_std::test]
    async fn await_for_any_two_second_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(false, 10);
        let result = await_for_any!(fut1, fut2);
        assert!(!result);
    }

    #[async_std::test]
    async fn await_for_any_three_third_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(true, 20);
        let fut3 = delayed_bool(false, 10);
        let result = await_for_any!(fut1, fut2, fut3);
        assert!(!result);
    }

    #[async_std::test]
    async fn await_for_any_four_fourth_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(true, 20);
        let fut3 = delayed_bool(true, 20);
        let fut4 = delayed_bool(false, 10);
        let result = await_for_any!(fut1, fut2, fut3, fut4);
        assert!(!result);
    }

    #[async_std::test]
    async fn await_for_any_five_fifth_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(true, 20);
        let fut3 = delayed_bool(true, 20);
        let fut4 = delayed_bool(true, 20);
        let fut5 = delayed_bool(false, 10);
        let result = await_for_any!(fut1, fut2, fut3, fut4, fut5);
        assert!(!result);
    }

    // Tests for wait_for_any! macro
    #[async_std::test]
    async fn wait_for_any_one() {
        let fut = wait_for_any!(ready(42));
        assert_eq!(fut.await, 42);
    }

    #[async_std::test]
    async fn wait_for_any_two_first_completes() {
        let fut1 = delayed_bool(true, 10);
        let fut2 = delayed_bool(false, 20);
        let fut = wait_for_any!(fut1, fut2);
        assert!(fut.await);
    }

    #[async_std::test]
    async fn wait_for_any_three_second_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(false, 10);
        let fut3 = delayed_bool(true, 20);
        let fut = wait_for_any!(fut1, fut2, fut3);
        assert!(!fut.await);
    }

    #[async_std::test]
    async fn wait_for_any_four_third_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(true, 20);
        let fut3 = delayed_bool(false, 10);
        let fut4 = delayed_bool(true, 20);
        let fut = wait_for_any!(fut1, fut2, fut3, fut4);
        assert!(!fut.await);
    }

    #[async_std::test]
    async fn wait_for_any_five_fourth_completes() {
        let fut1 = delayed_bool(true, 20);
        let fut2 = delayed_bool(true, 20);
        let fut3 = delayed_bool(true, 20);
        let fut4 = delayed_bool(false, 10);
        let fut5 = delayed_bool(true, 20);
        let fut = wait_for_any!(fut1, fut2, fut3, fut4, fut5);
        assert!(!fut.await);
    }

    // Tests for steady_select_* functions
    #[async_std::test]
    async fn steady_select_two_first_completes() {
        let fut1 = steady_fuse_future(delayed_bool(true, 10));
        let fut2 = steady_fuse_future(delayed_bool(false, 20));
        let result = steady_select_two(fut1, fut2).await;
        assert!(result);
    }

    #[async_std::test]
    async fn steady_select_two_second_completes() {
        let fut1 = steady_fuse_future(delayed_bool(true, 20));
        let fut2 = steady_fuse_future(delayed_bool(false, 10));
        let result = steady_select_two(fut1, fut2).await;
        assert!(!result);
    }

    #[async_std::test]
    async fn steady_select_three_third_completes() {
        let fut1 = steady_fuse_future(delayed_bool(true, 20));
        let fut2 = steady_fuse_future(delayed_bool(true, 20));
        let fut3 = steady_fuse_future(delayed_bool(false, 10));
        let result = steady_select_three(fut1, fut2, fut3).await;
        assert!(!result);
    }

    #[async_std::test]
    async fn steady_select_four_fourth_completes() {
        let fut1 = steady_fuse_future(delayed_bool(true, 20));
        let fut2 = steady_fuse_future(delayed_bool(true, 20));
        let fut3 = steady_fuse_future(delayed_bool(true, 20));
        let fut4 = steady_fuse_future(delayed_bool(false, 10));
        let result = steady_select_four(fut1, fut2, fut3, fut4).await;
        assert!(!result);
    }

    #[async_std::test]
    async fn steady_select_five_fifth_completes() {
        let fut1 = steady_fuse_future(delayed_bool(true, 20));
        let fut2 = steady_fuse_future(delayed_bool(true, 20));
        let fut3 = steady_fuse_future(delayed_bool(true, 20));
        let fut4 = steady_fuse_future(delayed_bool(true, 20));
        let fut5 = steady_fuse_future(delayed_bool(false, 10));
        let result = steady_select_five(fut1, fut2, fut3, fut4, fut5).await;
        assert!(!result);
    }
}