//! Utilities for composing and awaiting multiple futures in SteadyState.
//!
//! This module provides macros and functions for waiting on multiple asynchronous
//! operations, supporting both "wait for all" and "wait for any" semantics, as well as
//! more advanced patterns such as "wait for all except if the first completes".
//!
//! These utilities are designed to simplify concurrent actor and channel orchestration
//! in async Rust code, especially in graph-based or event-driven systems.

// ss[related philosophy.single-wake-up]
use futures_util::FutureExt;
// ss[related philosophy.structural-hierarchy]
pub use futures::future::Future;
// ss[related philosophy.structural-hierarchy]
pub use futures::select;
// ss[related philosophy.single-wake-up]
pub use futures::pin_mut;
// ss[related philosophy.structural-hierarchy]
use futures_util::future::FusedFuture;

// ss[impl philosophy.single-wake-up]
/// Waits for all provided futures to complete, returning `true` only if all complete with `true`.
///
/// This macro is useful for synchronizing multiple asynchronous operations where all must succeed.
/// The result is a boolean indicating whether all futures returned `true`.
///
/// # Clean vs dirty wakes
///
/// Every `bool` fed into these macros is a **wake result**: `true` means the awaited
/// condition was met, `false` means the wait was interrupted (shutdown). The AND of the
/// set is the cleanliness of this reactor tick:
///
/// * **Clean wake (`true`)** — every condition is genuinely satisfied; proceed to
///   peek/take/send with confidence.
/// * **Dirty wake (`false`)** — at least one wait was interrupted. Do **not** assume
///   readiness. Drain whatever partial work is legitimately available, then let the
///   `is_running(...)` loop condition exit the actor.
///
/// Macro results cannot carry `#[must_use]`, so this convention is contractual: always
/// bind the result (`let clean = await_for_all!(...)`) and gate readiness on it.
#[macro_export]
// ss[related philosophy.single-wake-up]
macro_rules! await_for_all {
($($t:expr),*) => {
        async {
            $crate::yield_now().await; //ensure we play nice in troupes
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
///
/// # Clean vs dirty wakes
///
/// The first arm is typically the work trigger (e.g. data availability); the remaining
/// arms are supporting conditions. If the first arm completes first, its wake result is
/// returned and the other futures are dropped mid-flight — treat a `false` as an
/// interrupted (dirty) wake: drain what is legitimately available, do not assume the
/// dropped conditions were met, and let `is_running(...)` exit on shutdown. Macro results
/// cannot carry `#[must_use]`; always bind the result (`let clean = ...`) and gate
/// readiness on it.
#[macro_export]
// ss[related philosophy.single-wake-up]
macro_rules! await_for_all_or_proceed_upon {
    ($first:expr, $second:expr $(,)?) => {{
        $crate::yield_now().await; //ensure we play nice in troupes
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        $crate::steady_await_for_all_or_proceed_upon_two(fut1,fut2).await
    }};
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        $crate::yield_now().await; //ensure we play nice in troupes
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        let fut3 = $crate::steady_fuse_future($third);
        $crate::steady_await_for_all_or_proceed_upon_three(fut1,fut2,fut3).await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        $crate::yield_now().await; //ensure we play nice in troupes
        let fut1 = $crate::steady_fuse_future($first);
        let fut2 = $crate::steady_fuse_future($second);
        let fut3 = $crate::steady_fuse_future($third);
        let fut4 = $crate::steady_fuse_future($fourth);
        $crate::steady_await_for_all_or_proceed_upon_four(fut1,fut2,fut3,fut4).await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr, $fifth:expr $(,)?) => {{
        $crate::yield_now().await; //ensure we play nice in troupes
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
///
/// # Clean vs dirty wakes
///
/// Only the winning future's wake result is returned; the losers are dropped mid-flight.
/// A `true` result vouches **only** for the lane that won — the other conditions are
/// unknown, so never treat "any" as "all". A `false` result means the winning wait was
/// interrupted (dirty wake): drain what is legitimately available, do not assume
/// readiness, and let `is_running(...)` exit on shutdown. Macro results cannot carry
/// `#[must_use]`; always bind the result (`let clean = ...`) and gate readiness on it.
#[macro_export]
// ss[related philosophy.single-wake-up]
macro_rules! await_for_any {
    ($first:expr $(,)?) => {{
        async {
            $crate::yield_now().await; //ensure we play nice in troupes
            $first.await
        }.await
    }};
    ($first:expr, $second:expr $(,)?) => {{
        async {
            $crate::yield_now().await; //ensure we play nice in troupes
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            $crate::steady_select_two(fut1, fut2).await
        }.await
    }};
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        async {
            $crate::yield_now().await; //ensure we play nice in troupes
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            $crate::steady_select_three(fut1, fut2, fut3).await
        }.await
    }};
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        async {
            $crate::yield_now().await; //ensure we play nice in troupes
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            let fut4 = $crate::steady_fuse_future($fourth);
            $crate::steady_select_four(fut1, fut2, fut3, fut4).await
        }.await
    }};
   ($first:expr, $second:expr, $third:expr, $fourth:expr, $fifth:expr $(,)?) => {{
        async {
            $crate::yield_now().await; //ensure we play nice in troupes
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
// ss[related philosophy.single-wake-up]
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

/// Wraps an `Option<usize>`-returning index wait method into a `bool`-returning future,
/// storing the resulting index into a mutable target.
///
/// This bridges methods like [`wait_avail_index`](crate::SteadyActor::wait_avail_index),
/// [`wait_vacant_index`](crate::SteadyActor::wait_vacant_index), and
/// [`wait_avail_vacant_index`](crate::SteadyActor::wait_avail_vacant_index) into the `bool`-returning macro
/// ecosystem (`await_for_any!`, `await_for_all!`, etc.).
///
/// # Example
///
/// ```rust,ignore
/// let mut ready_idx: Option<usize> = None;
///
/// let _clean = await_for_any!(
///     actor.wait_periodic(Duration::from_millis(500)),
///     wait_for_index!(actor.wait_avail_index(&mut rx, &counts) => ready_idx)
/// );
///
/// if let Some(i) = ready_idx {
///     process(&mut rx[i]);
/// }
/// ```
// ss[impl bundle.wait-for-index-macro]
#[macro_export]
// ss[related philosophy.structural-hierarchy]
macro_rules! wait_for_index {
    ($call:expr => $target:expr) => {
        async {
            match $call.await {
                Some(idx) => {
                    $target = Some(idx);
                    true
                }
                None => false,
            }
        }
    };
}

/// Waits for the first of two futures to complete, returning its result.
///
/// This function pins the provided futures and uses `select!` to await the first to finish.
/// It is useful for racing two asynchronous operations.
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
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
// ss[related philosophy.single-wake-up]
#[path = "loop_driver_tests.rs"]
mod loop_driver_tests;

