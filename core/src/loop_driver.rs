//use futures::FutureExt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[macro_export]
macro_rules! wait_for_all {
    // This pattern matches against any number of arguments
    ($($t:expr),*) => {
        async {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
             let _ = futures::join!($( wrap_bool_future(flag.clone(),$t) ),*);
            flag.load(std::sync::atomic::Ordering::Relaxed)
        }
    };
}

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
            let first = wrap_bool_future(flag.clone(),$first_future).fuse();
            pin_mut!(first);

            // Create the combined future for the rest and pin it
            let rest = async {
                futures::join!($(wrap_bool_future(flag.clone(),$rest_futures)),*)
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
