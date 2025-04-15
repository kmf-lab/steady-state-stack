use futures_util::FutureExt;
pub use futures::future::Future;
pub use futures::select;
pub use futures::pin_mut;
use futures_util::future::FusedFuture;

/// This macro waits for all the provided futures to complete.
/// It returns a boolean indicating if all futures returned true.
///
/// # Arguments
///
/// * `$t:expr` - A list of futures to wait for.
///
#[macro_export]
macro_rules! await_for_all {
    ($($t:expr),*) => {
        async {
            let mut flag = true;
            $(
                flag &= $t.await;
            )*
            flag
        }.await
    };
}


// pub async fn wait_for_all<F, const LEN: usize>(futures: &mut [F; LEN]) -> bool
// where
//     F: Future<Output = bool>,
// {
//     let mut flag = true;
//     for i in 0..LEN {
//         // Use unsafe because F is not Unpin.
//         let pinned_fut: Pin<&mut F> = unsafe { Pin::new_unchecked(&mut futures[i]) };
//         flag &= pinned_fut.await;
//     }
//     flag
// }


/// This macro waits for either the first future to complete, or all of the rest to complete.
/// It returns a boolean indicating if all futures (or the rest of the futures if the first completes) returned true.
///
/// # Arguments
///
/// * `$first_future:expr` - The first future to wait for.
/// * `$($rest_futures:expr),*` - The rest of the futures to wait for.
///
#[macro_export]
macro_rules! await_for_all_or_proceed_upon {
    ($first_future:expr, $($rest_futures:expr),* $(,)?) => {
        async {
            use futures_util::FutureExt;
            use futures::pin_mut;

            // Fuse the first future and pin it
            let first = $first_future.fuse();
            pin_mut!(first);

            // Create the combined future for the rest and pin it
            let rest = async {
                let mut flag = true;
                $(
                    let next = $rest_futures.fuse();
                    pin_mut!(next);
                    flag = flag & next.await;
                )*
               flag
            }.fuse();
            pin_mut!(rest);

            futures::select! {
                b = first => {b},
                b = rest => {b},
            }
        }.await
    };
}


// pub async fn wait_for_all_or_proceed_upon<F>(first_future: F, rest_futures: &mut [F]) -> bool
// where
//     F: Future<Output = bool> + Unpin,
// {
//     // Fuse and pin the first future
//     let first = first_future.fuse();
//     pin_mut!(first);
//
//     // Create and fuse the combined future for the rest
//     let rest = async {
//         let mut flag = true;
//         for fut in rest_futures.iter_mut() {
//             let next = fut.fuse();  // Fuse<&mut F>
//             pin_mut!(next);
//             flag &= next.await;
//         }
//         flag
//     }.fuse();
//     pin_mut!(rest);
//
//     // Race the first future against the combined rest
//     select! {
//         b = first => b,
//         b = rest => b,
//     }
// }


pub fn steady_fuse_future<F>(fut: F) -> futures_util::future::Fuse<F>
where
    F: Future,
{
    fut.fuse()
}

// This function pins the futures and runs select! on them.
// It takes fused futures that are not Unpin, pins them locally, and selects.
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

/// This macro waits for any of the provided futures to complete.
/// It returns a boolean indicating if any one of the futures returned true.
///
/// # Arguments
///
/// * `$($t:expr),*` - A list of futures to wait for.
///
#[macro_export]
macro_rules! await_for_any {
    // Case: Single future
    ($first:expr $(,)?) => {{
        async {
            $first.await
        }.await
    }};
    // Case: Two futures
    ($first:expr, $second:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            $crate::steady_select_two(fut1, fut2).await
        }.await
    }};
    // Case: Three futures
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            $crate::steady_select_three(fut1, fut2, fut3).await
        }.await
    }};
    // Case: Four futures
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        async {
            let fut1 = $crate::steady_fuse_future($first);
            let fut2 = $crate::steady_fuse_future($second);
            let fut3 = $crate::steady_fuse_future($third);
            let fut4 = $crate::steady_fuse_future($fourth);
            $crate::steady_select_four(fut1, fut2, fut3, fut4).await
        }.await
    }};
    // Case: Five futures
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
    // Add more cases as needed
}

// pub async fn wait_for_any<F>(futures: &mut [F]) -> bool
// where
//     F: Future<Output = bool> + 'static,
// {
//     let mut futs = FuturesUnordered::new();
//     for fut in futures.iter_mut() {
//         futs.push(fut);
//     }
//     match futs.next().await {
//         Some(result) => result,
//         None => unreachable!("futures slice should not be empty"),
//     }
// }


#[cfg(test)]
mod await_for_tests {
    use futures::future::ready;
    use std::time::Duration;
    use futures_timer::Delay;
    //
    // #[async_std::test]
    // async fn test_wait_for_all_true() {
    //     let future1 = ready(true);
    //     let future2 = ready(true);
    //     let future3 = ready(true);
    //
    //     let result = wait_for_all(&mut [future1, future2, future3]).await;
    //     assert!(result);
    // }
    //
    // #[async_std::test]
    // async fn test_wait_for_all_false() {
    //     let future1 = ready(true);
    //     let future2 = ready(false);
    //     let future3 = ready(true);
    //
    //     let result = wait_for_all(&mut [future1, future2, future3]).await;
    //     assert!(!result);
    // }

    // async fn test_wait_for_any_true() {
    //     let future1 = ready(true);
    //     let future2 = ready(true);
    //     let future3 = ready(true);
    //
    //     let result = wait_for_any(&mut [future1, future2, future3]).await;
    //     assert!(result);
    // }

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

        let result = await_for_all_or_proceed_upon!(future1, future2, future3);
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

        let result = await_for_all_or_proceed_upon!(future1, future2, future3);
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

        let result = await_for_all_or_proceed_upon!(future1, future2, future3);
        assert!(!result);
    }
}
