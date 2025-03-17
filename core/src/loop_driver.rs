use std::pin::{Pin};
use futures::future::Future;
use futures::future::FutureExt;
use futures::select;
use futures::StreamExt;
use futures::pin_mut;

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


pub async fn wait_for_all<F, const LEN: usize>(futures: &mut [F; LEN]) -> bool
where
    F: Future<Output = bool>,
{
    let mut flag = true;
    for i in 0..LEN {
        // Use unsafe because F is not Unpin.
        let pinned_fut: Pin<&mut F> = unsafe { Pin::new_unchecked(&mut futures[i]) };
        flag &= pinned_fut.await;
    }
    flag
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
macro_rules! await_for_all_or_proceed_upon {
    ($first_future:expr, $($rest_futures:expr),* $(,)?) => {
        async {
            use futures::future::FutureExt;
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


pub async fn wait_for_all_or_proceed_upon<F>(first_future: F, rest_futures: &mut [F]) -> bool
where
    F: Future<Output = bool> + Unpin,
{
    // Fuse and pin the first future
    let first = first_future.fuse();
    pin_mut!(first);

    // Create and fuse the combined future for the rest
    let rest = async {
        let mut flag = true;
        for fut in rest_futures.iter_mut() {
            let next = fut.fuse();  // Fuse<&mut F>
            pin_mut!(next);
            flag &= next.await;
        }
        flag
    }.fuse();
    pin_mut!(rest);

    // Race the first future against the combined rest
    select! {
        b = first => b,
        b = rest => b,
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
            use futures::future::FutureExt;
            use futures::pin_mut;
            use futures::select;
            let fut1 = $first.fuse();
            let fut2 = $second.fuse();
            pin_mut!(fut1);
            pin_mut!(fut2);
            select! {
                b = fut1 => {b},
                b = fut2 => {b},
            }
        }.await
    }};
    // Case: Three futures
    ($first:expr, $second:expr, $third:expr $(,)?) => {{
        async {
            use futures::future::FutureExt;
            use futures::pin_mut;
            use futures::select;
            let fut1 = $first.fuse();
            let fut2 = $second.fuse();
            let fut3 = $third.fuse();
            pin_mut!(fut1);
            pin_mut!(fut2);
            pin_mut!(fut3);
            select! {
                b = fut1 => {b},
                b = fut2 => {b},
                b = fut3 => {b},
            }

        }.await
    }};
     // Case: Four futures
    ($first:expr, $second:expr, $third:expr, $fourth:expr $(,)?) => {{
        async {
            use futures::future::FutureExt;
            use futures::pin_mut;
            use futures::select;
            let fut1 = $first.fuse();
            let fut2 = $second.fuse();
            let fut3 = $third.fuse();
            let fut4 = $fourth.fuse();
            pin_mut!(fut1);
            pin_mut!(fut2);
            pin_mut!(fut3);
            pin_mut!(fut4);
            select! {
                b = fut1 => {b},
                b = fut2 => {b},
                b = fut3 => {b},
                b = fut4 => {b},
            }
        }.await
    }};
    // Case: Five futures
    ($first:expr, $second:expr, $third:expr, $fourth:expr, $fifth:expr $(,)?) => {{
        async {
            use futures::future::FutureExt;
            use futures::pin_mut;
            use futures::select;
            let fut1 = $first.fuse();
            let fut2 = $second.fuse();
            let fut3 = $third.fuse();
            let fut4 = $fourth.fuse();
            let fut5 = $fifth.fuse();
            pin_mut!(fut1);
            pin_mut!(fut2);
            pin_mut!(fut3);
            pin_mut!(fut4);
            pin_mut!(fut5);
            select! {
                b = fut1 => {b},
                b = fut2 => {b},
                b = fut3 => {b},
                b = fut4 => {b},
                b = fut5 => {b},
            }
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
