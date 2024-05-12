use std::future::Future;
use lazy_static::lazy_static;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
#[allow(unused_imports)]
use log::*;
use parking_lot::Once;


lazy_static! {
    static ref INIT: Once = Once::new();
}

pub(crate) fn init() {

    INIT.call_once(|| {
        //TODO: can I let users choose this and tokio as desired?
        let nuclei_config = NucleiConfig {
            //iouring: IoUringConfiguration::interrupt_driven(1 << 6),
            iouring: IoUringConfiguration::kernel_poll_only(1 << 6),
            //iouring: IoUringConfiguration::low_latency_driven(1 << 6),
            //iouring: IoUringConfiguration::io_poll(1 << 6),
        };
        let _ = nuclei::Proactor::with_config(nuclei_config);

        let config = nuclei::GlobalExecutorConfig::default()
            .with_min_threads(4)
            .with_max_threads(usize::MAX); //disabled this limit
        nuclei::init_with_config(config);
    });
}


pub(crate) fn spawn_detached<F, T>(future: F)
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
{ //only used for supervisors who will be blocked running the actor most of the time.
    nuclei::spawn(async move {
        //TODO: we need to be able to config the stack size for these new threads.

        match nuclei::spawn_more_threads(1).await {
            Ok(_) => {}
            Err(_) => {} //log max thread issues?
        };
        future.await;
    }).detach();  // Spawn an async task with nuclei
}

pub(crate) fn block_on<F, T>(future: F) -> T
    where
        F: Future<Output = T> + Send + 'static,
        T: Send,
{
    nuclei::block_on(future)  // Block until the future is resolved
}


