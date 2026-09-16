//! Host bridge for warm Lambda-style request/response injects.
//!
//! The host thread owns [`LambdaHost`]; actors park on [`LambdaIngress::wait`] and
//! reply via [`LambdaEgress::send`]. This is **not** StageManager / `testing_send_all`.

use parking_lot::{Condvar, Mutex};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

struct IngressSlot<In> {
    msg: Option<In>,
    waker: Option<Waker>,
}

struct EgressSlot<Out> {
    msg: Option<Out>,
}

struct BridgeInner<In, Out> {
    ingress: Mutex<IngressSlot<In>>,
    egress: Mutex<EgressSlot<Out>>,
    egress_cv: Condvar,
    closed: AtomicBool,
}

/// Host-side handle: inject requests and wait for responses on the Runtime API thread.
// ss[impl graph.lambda.host-bridge]
// ss[impl graph.lambda.host-not-executor]
pub struct LambdaHost<In, Out> {
    inner: Arc<BridgeInner<In, Out>>,
}

/// Actor-side ingress: async wait for the next host inject (does not block the OS thread).
// ss[impl graph.lambda.host-bridge]
pub struct LambdaIngress<In, Out> {
    inner: Arc<BridgeInner<In, Out>>,
}

/// Actor-side egress: send a response back to the waiting host.
// ss[impl graph.lambda.host-bridge]
pub struct LambdaEgress<In, Out> {
    inner: Arc<BridgeInner<In, Out>>,
}

/// Build a typed host bridge (used by [`crate::Graph::lambda_bridge`]).
// ss[impl graph.lambda.host-bridge]
// ss[impl graph.lambda.warm-graph]
pub fn lambda_bridge<In, Out>() -> (LambdaHost<In, Out>, LambdaIngress<In, Out>, LambdaEgress<In, Out>)
where
    In: Send + 'static,
    Out: Send + 'static,
{
    let inner = Arc::new(BridgeInner {
        ingress: Mutex::new(IngressSlot {
            msg: None,
            waker: None,
        }),
        egress: Mutex::new(EgressSlot { msg: None }),
        egress_cv: Condvar::new(),
        closed: AtomicBool::new(false),
    });
    (
        LambdaHost {
            inner: inner.clone(),
        },
        LambdaIngress {
            inner: inner.clone(),
        },
        LambdaEgress { inner },
    )
}

impl<In, Out> LambdaHost<In, Out> {
    /// Inject one request for the next actor `wait`. Replaces any unread prior inject.
    // ss[impl graph.lambda.host-bridge]
    pub fn inject(&self, msg: In) {
        let mut g = self.inner.ingress.lock();
        g.msg = Some(msg);
        if let Some(w) = g.waker.take() {
            w.wake();
        }
    }

    /// Block the **host** thread until an egress send or `timeout`.
    // ss[impl graph.lambda.host-bridge]
    // ss[impl graph.lambda.host-not-executor]
    pub fn wait_output(&self, timeout: Duration) -> Option<Out> {
        let deadline = std::time::Instant::now() + timeout;
        let mut g = self.inner.egress.lock();
        loop {
            if let Some(msg) = g.msg.take() {
                return Some(msg);
            }
            if self.inner.closed.load(Ordering::SeqCst) {
                return None;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let timed_out = self
                .inner
                .egress_cv
                .wait_for(&mut g, remaining)
                .timed_out();
            if timed_out {
                return g.msg.take();
            }
        }
    }

    /// Close the bridge so actor waits return `None` and host waiters wake.
    // ss[impl graph.lambda.warm-graph]
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        {
            let mut g = self.inner.ingress.lock();
            if let Some(w) = g.waker.take() {
                w.wake();
            }
        }
        self.inner.egress_cv.notify_all();
    }
}

impl<In, Out> Clone for LambdaIngress<In, Out> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<In, Out> Clone for LambdaEgress<In, Out> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<In, Out> LambdaIngress<In, Out> {
    /// Async wait for the next host inject. Parks only this actor future (troupe-safe).
    // ss[impl graph.lambda.host-bridge]
    // ss[impl graph.lambda.event-driven]
    pub fn wait(&self) -> IngressWait<In, Out> {
        IngressWait {
            inner: self.inner.clone(),
        }
    }
}

impl<In, Out> LambdaEgress<In, Out> {
    /// Deliver one response to the waiting host.
    // ss[impl graph.lambda.host-bridge]
    pub fn send(&self, msg: Out) {
        let mut g = self.inner.egress.lock();
        g.msg = Some(msg);
        self.inner.egress_cv.notify_one();
    }

    /// Signal no more responses (optional; host [`LambdaHost::close`] is preferred).
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        {
            let mut g = self.inner.ingress.lock();
            if let Some(w) = g.waker.take() {
                w.wake();
            }
        }
        self.inner.egress_cv.notify_all();
    }
}

/// Future returned by [`LambdaIngress::wait`].
pub struct IngressWait<In, Out> {
    inner: Arc<BridgeInner<In, Out>>,
}

impl<In, Out> Future for IngressWait<In, Out> {
    type Output = Option<In>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut g = self.inner.ingress.lock();
        if let Some(msg) = g.msg.take() {
            return Poll::Ready(Some(msg));
        }
        if self.inner.closed.load(Ordering::SeqCst) {
            return Poll::Ready(None);
        }
        g.waker = Some(cx.waker().clone());
        if let Some(msg) = g.msg.take() {
            return Poll::Ready(Some(msg));
        }
        if self.inner.closed.load(Ordering::SeqCst) {
            return Poll::Ready(None);
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod lambda_bridge_tests {
    use super::*;
    use crate::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn inject_then_wait_roundtrip() {
        let (host, ingress, egress) = lambda_bridge::<u32, u32>();
        let h = thread::spawn(move || {
            host.inject(7);
            let out = host.wait_output(Duration::from_secs(2));
            assert_eq!(out, Some(14));
        });
        let ing = thread::spawn(move || {
            let v = crate::core_exec::block_on(ingress.wait());
            assert_eq!(v, Some(7));
            egress.send(14);
        });
        h.join().expect("host");
        ing.join().expect("ingress");
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn close_unblocks_wait() {
        let (host, ingress, _egress) = lambda_bridge::<u32, u32>();
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            host.close();
        });
        let v = crate::core_exec::block_on(ingress.wait());
        assert!(v.is_none());
        h.join().expect("host");
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn wait_output_times_out_when_no_response() {
        let (host, _ingress, _egress) = lambda_bridge::<u32, u32>();
        let out = host.wait_output(Duration::from_millis(30));
        assert!(out.is_none());
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn wait_output_zero_timeout_is_none() {
        let (host, _ingress, _egress) = lambda_bridge::<u32, u32>();
        assert!(host.wait_output(Duration::ZERO).is_none());
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn wait_output_timeout_then_late_send() {
        let (host, _ingress, egress) = lambda_bridge::<u32, u32>();
        assert!(host.wait_output(Duration::from_millis(15)).is_none());
        egress.send(9);
        assert_eq!(host.wait_output(Duration::from_millis(50)), Some(9));
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn egress_close_unblocks_pending_ingress_wait() {
        let (_host, ingress, egress) = lambda_bridge::<u32, u32>();
        let closer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            egress.close();
        });
        let v = crate::core_exec::block_on(ingress.wait());
        assert!(v.is_none());
        closer.join().expect("closer");
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn egress_close_unblocks_host_wait() {
        let (host, ingress, egress) = lambda_bridge::<u32, u32>();
        let h = thread::spawn(move || {
            host.inject(1);
            let out = host.wait_output(Duration::from_secs(2));
            assert!(out.is_none());
        });
        let ing = thread::spawn(move || {
            let v = crate::core_exec::block_on(ingress.wait());
            assert_eq!(v, Some(1));
            egress.close();
        });
        h.join().expect("host");
        ing.join().expect("ingress");
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn inject_replaces_unread_prior_message() {
        let (host, ingress, _egress) = lambda_bridge::<u32, u32>();
        host.inject(1);
        host.inject(2);
        let v = crate::core_exec::block_on(ingress.wait());
        assert_eq!(v, Some(2));
    }

    // ss[verify graph.lambda.host-bridge]
    #[test]
    fn host_close_unblocks_wait_output() {
        let (host, _ingress, egress) = lambda_bridge::<u32, u32>();
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            egress.close();
        });
        let out = host.wait_output(Duration::from_secs(2));
        assert!(out.is_none());
        h.join().expect("closer");
    }

    // ss[verify graph.lambda.warm-graph]
    // ss[verify graph.lambda.host-bridge]
    // ss[verify graph.lambda.host-not-executor]
    // ss[verify graph.lambda.event-driven]
    // ss[verify troupe.dynamic-member-of]
    #[test]
    fn warm_graph_two_injects_same_thread() {
        let mut graph = GraphBuilder::for_testing()
            .with_strict_persist()
            .with_pack_slots(1)
            .build(());
        let (host, ingress, egress) = graph.lambda_bridge::<u64, u64>();
        let thread_ids = Arc::new(parking_lot::Mutex::new(Vec::<std::thread::ThreadId>::new()));
        let tids = thread_ids.clone();
        let invoke_count = Arc::new(AtomicUsize::new(0));
        let count = invoke_count.clone();

        let mut dyn_bag = graph.dynamic_troupe().with_name("LambdaPack");
        graph.actor_builder().with_name("WORKER").build(
            move |ctx| {
                let ingress = ingress.clone();
                let egress = egress.clone();
                let tids = tids.clone();
                let count = count.clone();
                async move {
                    let mut actor = ctx.into_spotlight([], []);
                    while actor.is_running(|| true) {
                        // Event-driven: park on host inject only (no wait_periodic).
                        let req = match ingress.wait().await {
                            Some(v) => v,
                            None => break,
                        };
                        tids.lock().push(thread::current().id());
                        count.fetch_add(1, AtomicOrdering::SeqCst);
                        egress.send(req.saturating_mul(2));
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut dyn_bag),
        );
        drop(dyn_bag);

        graph.start();

        host.inject(3);
        assert_eq!(host.wait_output(Duration::from_secs(2)), Some(6));
        host.inject(5);
        assert_eq!(host.wait_output(Duration::from_secs(2)), Some(10));

        // Same packed OS thread across injects; graph stayed Running (no shutdown between).
        {
            let ids = thread_ids.lock();
            assert_eq!(ids.len(), 2);
            assert_eq!(ids[0], ids[1]);
        }
        assert_eq!(invoke_count.load(AtomicOrdering::SeqCst), 2);

        host.close();
        graph.request_shutdown();
        graph
            .block_until_stopped(Duration::from_secs(2))
            .expect("clean stop");
    }
}
