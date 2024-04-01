use futures::FutureExt;
use futures_util::future::FusedFuture;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[macro_export]
macro_rules! wait_for_all {
    // This pattern matches against any number of arguments
    ($($t:expr),*) => {{
            use std::sync::Arc;
            use std::sync::atomic::{AtomicBool, Ordering};

            let flag = Arc::new(AtomicBool::new(true));
            futures::join!($( OutcomeTracker::wrap_future(flag.clone(),$t) ),*);
            flag.load(Ordering::Relaxed)

    }};
}

#[macro_export]
macro_rules! wait_for_all_or_proceed_upon {
    ($first_future:expr, $($rest_futures:expr),* $(,)?) => {{
        use futures::future::FutureExt;
        use futures::pin_mut;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let flag = Arc::new(AtomicBool::new(true));

        // Fuse the first future and pin it
        let first = OutcomeTracker::wrap_future(flag.clone(),$first_future).fuse();
        pin_mut!(first);

        // Create the combined future for the rest and pin it
        let rest = async {
            futures::join!($(OutcomeTracker::wrap_future(flag.clone(),$rest_futures)),*)
        }.fuse();
        pin_mut!(rest);

        futures::select! {
            _ = first => {},
            _ = rest => {},
        };

        flag.load(Ordering::Relaxed)

    }};
}

pub struct OutcomeTracker;

impl OutcomeTracker {
    pub async fn wrap_future<F>(flag: Arc<AtomicBool>, fut: F) -> impl Future<Output = ()>
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
}
