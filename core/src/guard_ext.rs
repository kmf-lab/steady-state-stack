//! Guard-first acquisition for Steady channel handles.
//!
//! [`SteadyRx`] and [`SteadyTx`] are shared handles over an async fair queue. Calling
//! [`SteadyChannelExt::acquire_guard`] binds the channel guard to this actor's instance —
//! the Steady vocabulary for what `.lock().await` does on the underlying
//! `futures::lock::Mutex`. We say **acquire the guard**, not "lock", because this is not
//! a mutex critical section:
//!
//! * The guard is meant to be **held across `.await`** for the life of the actor
//!   (guard-first, bind-all-at-entry). Every actor instance holds its own guard on its
//!   own handle; guards never serialize actors against each other.
//! * On panic the guard is simply **dropped** and the ring retains its messages —
//!   nothing is poisoned or lost.
//!
//! Re-exported at the crate root so a glob import (`use steady_state::*;`) brings the
//! method into scope next to [`SteadyRx`] / [`SteadyTx`].

use std::future::Future;

use futures_util::lock::MutexGuard;

use crate::steady_rx::Rx;
use crate::steady_tx::Tx;
use crate::{SteadyRx, SteadyTx};

/// Guard-first acquisition for [`SteadyRx`] / [`SteadyTx`] handles.
///
/// `acquire_guard().await` is the preferred spelling of `.lock().await` on Steady
/// channel handles. It yields the same guard; only the vocabulary changes. See the
/// [module documentation](self) for why Steady says "guard" and not "lock".
pub trait SteadyChannelExt {
    /// The guard type bound to this actor instance for the borrow of the handle.
    type Guard<'a>
    where
        Self: 'a;

    /// Binds this actor instance's channel guard, awaiting fairness if another
    /// clone of this same handle is mid-operation.
    ///
    /// Bind once at actor entry, keep the guard for the actor's lifetime, and
    /// guard every `wait_*` with it. On shutdown or panic, drop the guard — the
    /// ring keeps its messages.
    fn acquire_guard(&self) -> impl Future<Output = Self::Guard<'_>> + Send + '_;
}

impl<T: Send> SteadyChannelExt for SteadyRx<T> {
    type Guard<'a>
        = MutexGuard<'a, Rx<T>>
    where
        Self: 'a;

    fn acquire_guard(&self) -> impl Future<Output = Self::Guard<'_>> + Send + '_ {
        self.lock()
    }
}

impl<T: Send> SteadyChannelExt for SteadyTx<T> {
    type Guard<'a>
        = MutexGuard<'a, Tx<T>>
    where
        Self: 'a;

    fn acquire_guard(&self) -> impl Future<Output = Self::Guard<'_>> + Send + '_ {
        self.lock()
    }
}

#[cfg(test)]
mod guard_ext_tests {
    use super::*;
    use crate::*;

    /// The alias must yield a guard over the same live channel state as `.lock().await`.
    // ss[verify actor.lock-first.channels]
    #[test]
    fn acquire_guard_equivalent_to_lock_single_channel() {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = graph.channel_builder();
        let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        let (tx, rx) = (tx_lazy.clone(), rx_lazy.clone());

        core_exec::block_on(async {
            {
                let mut rx_guard = rx.lock().await;
                assert!(rx_guard.is_empty());
                assert_eq!(0, rx_guard.avail_units());
            }
            {
                let mut rx_guard = rx.acquire_guard().await;
                assert!(rx_guard.is_empty());
                assert_eq!(0, rx_guard.avail_units());

                // State written through one acquisition path is visible through the other.
                let mut tx_guard = tx.acquire_guard().await;
                assert!(tx_guard.mark_closed());
                drop(tx_guard);
                assert!(rx_guard.is_closed_and_empty());
            }
        });
    }

    /// Bundle trait alias must bind every lane, same as `.lock().await`.
    // ss[verify actor.lock-first.channels]
    #[test]
    fn acquire_guard_equivalent_to_lock_bundle() {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = graph.channel_builder();
        let (_tx_lazy, rx_lazy) = builder.build_channel_bundle::<String, 3>();
        let bundle = SteadyRxBundle::new([rx_lazy[0].clone(), rx_lazy[1].clone(), rx_lazy[2].clone()]);

        core_exec::block_on(async {
            {
                let guards = bundle.lock().await;
                assert_eq!(3, guards.len());
            }
            {
                let mut guards = bundle.acquire_guard().await;
                assert_eq!(3, guards.len());
                for lane in guards.iter_mut() {
                    assert_eq!(0, lane.avail_units());
                }
            }
        });
    }

    /// `SteadyState::acquire_guard` initializes once, exactly like `lock`.
    // ss[verify state.lock-init-once]
    #[test]
    fn acquire_guard_equivalent_to_lock_steady_state() {
        let state: SteadyState<u64> = new_state();
        core_exec::block_on(async {
            {
                let guard = state.acquire_guard(|| 42).await;
                assert_eq!(42, *guard);
            }
            {
                // Second acquisition must not re-run init.
                let guard = state.acquire_guard(|| 0).await;
                assert_eq!(42, *guard);
            }
            {
                let guard = state.lock(|| 0).await;
                assert_eq!(42, *guard);
            }
        });
    }

    /// Builder-context (`NonSendWrapper`) alias guards the same value as `lock`.
    // ss[verify actor.regeneration-survives]
    #[test]
    fn acquire_guard_equivalent_to_lock_builder_context() {
        let wrapped = crate::actor_builder::NonSendWrapper::new(7u32);
        core_exec::block_on(async {
            {
                let mut guard = wrapped.acquire_guard().await;
                assert_eq!(7, *guard);
                *guard = 9;
            }
            {
                let guard = wrapped.lock().await;
                assert_eq!(9, *guard);
            }
        });
    }
}
