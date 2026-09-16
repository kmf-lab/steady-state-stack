//! Dirty-bit / persist-error properties (JSON persist is covered in `state_management.rs`).

use super::*;
use proptest::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

ss_proptest! {
    /// Property: persist hook `Err` leaves the guard dirty.
    #[test]
    // ss[verify state.dirty-at-park]
    // ss[verify state.persist-hooks]
    // ss[verify verify.process.proptest]
    fn proptest_persist_err_keeps_dirty(value in -1_000i32..1_000) {
        crate::core_exec::block_on(async {
            let state = new_persistent_state_with(
                || None,
                |_| Err(std::io::Error::other("persist failed")),
            );
            let mut guard = state.lock(|| 0i32).await;
            *guard = value;
            prop_assert!(guard.is_dirty());
            prop_assert!(guard.persist().await.is_err());
            prop_assert!(guard.is_dirty());
            Ok(())
        })?;
    }

    /// Property: successful persist then park under strict does not panic.
    #[test]
    // ss[verify state.dirty-at-park]
    // ss[verify state.persist-before-park]
    // ss[verify verify.process.proptest]
    fn proptest_persist_success_clears_dirty_for_strict_park(value in -500i32..500) {
        set_thread_strict_persist(true);
        let result = std::panic::catch_unwind(|| {
            crate::core_exec::block_on(async {
                let store = Arc::new(parking_lot::Mutex::new(None::<i32>));
                let store_p = store.clone();
                let state = new_persistent_state_with(
                    || None,
                    move |s: &i32| {
                        *store_p.lock() = Some(*s);
                        Ok(())
                    },
                );
                let mut guard = state.lock(|| 0i32).await;
                *guard = value;
                guard.persist().await.expect("persist");
                assert!(!guard.is_dirty());
                check_dirty_at_park();
            });
        });
        set_thread_strict_persist(false);
        prop_assert!(result.is_ok(), "strict park after persist must not panic");
    }

    /// Property: dirty-at-park is a no-op when the thread is not strict.
    #[test]
    // ss[verify state.dirty-at-park]
    // ss[verify verify.process.proptest]
    fn proptest_check_dirty_at_park_skips_when_not_strict(value in 0i32..100) {
        set_thread_strict_persist(false);
        crate::core_exec::block_on(async {
            let state = new_persistent_state_with(|| None, |_| Ok(()));
            let mut guard = state.lock(|| 0i32).await;
            *guard = value;
            prop_assert!(guard.is_dirty());
            check_dirty_at_park();
            Ok(())
        })?;
    }

    /// Property: persist-hook `Err` on `new_persistent_state_with` is returned to the caller.
    #[test]
    // ss[verify state.persist-hooks]
    // ss[verify verify.process.proptest]
    fn proptest_persist_hook_err_counts(calls in 1u8..8) {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_p = hits.clone();
        crate::core_exec::block_on(async {
            let state = new_persistent_state_with(
                || Some(0i32),
                move |_| {
                    hits_p.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::other("hook"))
                },
            );
            let guard = state.lock(|| 0i32).await;
            for _ in 0..calls {
                prop_assert!(guard.persist().await.is_err());
            }
            Ok(())
        })?;
        prop_assert_eq!(hits.load(Ordering::SeqCst), calls as usize);
    }
}
