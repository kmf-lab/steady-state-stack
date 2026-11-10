// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use futures_timer::Delay;
// ss[related philosophy.single-wake-up]
use futures::future::ready;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicBool, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.single-wake-up]
use std::time::Duration;

// Helper function to create a future that returns a boolean after a delay
// ss[related philosophy.single-wake-up]
async fn delayed_bool(value: bool, ms: u64) -> bool {
    Delay::new(Duration::from_millis(ms)).await;
    value
}

// Helper function to create a controlled future that completes when signaled
// ss[related philosophy.single-wake-up]
fn controlled_bool(value: bool, signal: Arc<AtomicBool>) -> impl Future<Output = bool> + FusedFuture {
    let signal_clone = signal.clone();
    async move {
        while !signal_clone.load(Ordering::SeqCst) {
            Delay::new(Duration::from_millis(100)).await;
        }
        value
    }
        .fuse()
}

// Tests for await_for_all! macro
// ss[verify philosophy.single-wake-up]
// ss[verify philosophy.mechanical-sympathy]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_all_true() {
crate::core_exec::block_on(async {

    let result = await_for_all!(
        ready(true),
        ready(true),
        ready(true)
    );
    assert!(result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_one_false() {
crate::core_exec::block_on(async {

    let result = await_for_all!(
        ready(true),
        ready(false),
        ready(true)
    );
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_empty() {
crate::core_exec::block_on(async {

    let result = await_for_all!();
    assert!(result); // Empty case returns true
    });
}

// Tests for wait_for_all! macro
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn wait_for_all_all_true() {
crate::core_exec::block_on(async {

    let fut = wait_for_all!(
        ready(true),
        ready(true),
        ready(true)
    );
    assert!(fut.await);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn wait_for_all_one_false() {
crate::core_exec::block_on(async {

    let fut = wait_for_all!(
        ready(true),
        ready(false),
        ready(true)
    );
    assert!(!fut.await);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn wait_for_all_empty() {
crate::core_exec::block_on(async {

    let fut = wait_for_all!();
    assert!(fut.await);
    });
}

// Test for steady_fuse_future function
#[test]
// ss[verify philosophy.single-wake-up]
fn steady_fuse_future_works() {
crate::core_exec::block_on(async {

    let fut = ready(true);
    let fused = steady_fuse_future(fut);
    assert!(!fused.is_terminated());
    assert!(fused.await);
    });
}

// Tests for await_for_all_or_proceed_upon! macro with two futures
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_two_first_completes() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(false));
    let fut1 = delayed_bool(true, 100);
    let fut2 = controlled_bool(true, signal.clone());
    let result = await_for_all_or_proceed_upon!(fut1, fut2);
    assert!(result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_two_others_complete() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(true));
    let fut1 = controlled_bool(true, signal.clone());
    let fut2 = delayed_bool(false, 100);
    let result = await_for_all_or_proceed_upon!(fut1, fut2);
    assert!(result);
    signal.store(true, Ordering::SeqCst); // Ensure cleanup
    });
}

// Tests for await_for_all_or_proceed_upon! macro with three futures
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_three_first_completes() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(false));
    let fut1 = delayed_bool(false, 100);
    let fut2 = controlled_bool(true, signal.clone());
    let fut3 = controlled_bool(true, signal.clone());
    let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3);
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_three_others_complete() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(true));
    let fut1 = controlled_bool(true, signal.clone());
    let fut2 = delayed_bool(true, 100);
    let fut3 = delayed_bool(false, 100);
    let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3);
    assert!(result);
    signal.store(true, Ordering::SeqCst);
    });
}

// Tests for await_for_all_or_proceed_upon! macro with four futures
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_four_first_completes() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(false));
    let fut1 = delayed_bool(true, 100);
    let fut2 = controlled_bool(true, signal.clone());
    let fut3 = controlled_bool(true, signal.clone());
    let fut4 = controlled_bool(true, signal.clone());
    let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4);
    assert!(result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_four_others_complete() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(true));
    let fut1 = controlled_bool(true, signal.clone());
    let fut2 = delayed_bool(true, 100);
    let fut3 = delayed_bool(true, 100);
    let fut4 = delayed_bool(false, 100);
    let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4);
    assert!(result);
    signal.store(true, Ordering::SeqCst);
    });
}

// Tests for await_for_all_or_proceed_upon! macro with five futures
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_five_first_completes() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(false));
    let fut1 = delayed_bool(false, 100);
    let fut2 = controlled_bool(true, signal.clone());
    let fut3 = controlled_bool(true, signal.clone());
    let fut4 = controlled_bool(true, signal.clone());
    let fut5 = controlled_bool(true, signal.clone());
    let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4, fut5);
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_all_or_proceed_upon_five_others_complete() {
crate::core_exec::block_on(async {

    let signal = Arc::new(AtomicBool::new(true));
    let fut1 = controlled_bool(true, signal.clone());
    let fut2 = delayed_bool(true, 100);
    let fut3 = delayed_bool(true, 100);
    let fut4 = delayed_bool(true, 100);
    let fut5 = delayed_bool(false, 100);
    let result = await_for_all_or_proceed_upon!(fut1, fut2, fut3, fut4, fut5);
    assert!(result);
    signal.store(true, Ordering::SeqCst);
    });
}

// Tests for await_for_any! macro
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_any_one() {
crate::core_exec::block_on(async {

    let result = await_for_any!(ready(42));
    assert_eq!(result, 42);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_any_two_first_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 100);
    let fut2 = delayed_bool(false, 200);
    let result = await_for_any!(fut1, fut2);
    assert!(result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_any_two_second_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(false, 100);
    let result = await_for_any!(fut1, fut2);
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_any_three_third_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(true, 200);
    let fut3 = delayed_bool(false, 100);
    let result = await_for_any!(fut1, fut2, fut3);
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_any_four_fourth_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(true, 200);
    let fut3 = delayed_bool(true, 200);
    let fut4 = delayed_bool(false, 100);
    let result = await_for_any!(fut1, fut2, fut3, fut4);
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn await_for_any_five_fifth_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(true, 200);
    let fut3 = delayed_bool(true, 200);
    let fut4 = delayed_bool(true, 200);
    let fut5 = delayed_bool(false, 100);
    let result = await_for_any!(fut1, fut2, fut3, fut4, fut5);
    assert!(!result);
    });
}

// Tests for wait_for_any! macro
#[test]
// ss[verify philosophy.single-wake-up]
fn wait_for_any_one() {
crate::core_exec::block_on(async {

    let fut = wait_for_any!(ready(42));
    assert_eq!(fut.await, 42);
    });
}

#[test]
// ss[verify philosophy.single-wake-up]
fn wait_for_any_two_first_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 100);
    let fut2 = delayed_bool(false, 200);
    let fut = wait_for_any!(fut1, fut2);
    assert!(fut.await);
    });
}

#[test]
// ss[verify philosophy.single-wake-up]
fn wait_for_any_three_second_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(false, 100);
    let fut3 = delayed_bool(true, 200);
    let fut = wait_for_any!(fut1, fut2, fut3);
    assert!(!fut.await);
    });
}

#[test]
// ss[verify philosophy.single-wake-up]
fn wait_for_any_four_third_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(true, 200);
    let fut3 = delayed_bool(false, 100);
    let fut4 = delayed_bool(true, 200);
    let fut = wait_for_any!(fut1, fut2, fut3, fut4);
    assert!(!fut.await);
    });
}

#[test]
// ss[verify philosophy.single-wake-up]
fn wait_for_any_five_fourth_completes() {
crate::core_exec::block_on(async {

    let fut1 = delayed_bool(true, 200);
    let fut2 = delayed_bool(true, 200);
    let fut3 = delayed_bool(true, 200);
    let fut4 = delayed_bool(false, 100);
    let fut5 = delayed_bool(true, 200);
    let fut = wait_for_any!(fut1, fut2, fut3, fut4, fut5);
    assert!(!fut.await);
    });
}

// Tests for steady_select_* functions
// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn steady_select_two_first_completes() {
crate::core_exec::block_on(async {

    let fut1 = steady_fuse_future(delayed_bool(true, 100));
    let fut2 = steady_fuse_future(delayed_bool(false, 200));
    let result = steady_select_two(fut1, fut2).await;
    assert!(result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn steady_select_two_second_completes() {
crate::core_exec::block_on(async {

    let fut1 = steady_fuse_future(delayed_bool(true, 200));
    let fut2 = steady_fuse_future(delayed_bool(false, 100));
    let result = steady_select_two(fut1, fut2).await;
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn steady_select_three_third_completes() {
crate::core_exec::block_on(async {

    let fut1 = steady_fuse_future(delayed_bool(true, 200));
    let fut2 = steady_fuse_future(delayed_bool(true, 200));
    let fut3 = steady_fuse_future(delayed_bool(false, 100));
    let result = steady_select_three(fut1, fut2, fut3).await;
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn steady_select_four_fourth_completes() {
crate::core_exec::block_on(async {

    let fut1 = steady_fuse_future(delayed_bool(true, 200));
    let fut2 = steady_fuse_future(delayed_bool(true, 200));
    let fut3 = steady_fuse_future(delayed_bool(true, 200));
    let fut4 = steady_fuse_future(delayed_bool(false, 100));
    let result = steady_select_four(fut1, fut2, fut3, fut4).await;
    assert!(!result);
    });
}

// ss[verify philosophy.single-wake-up]
#[test]
// ss[related philosophy.structural-hierarchy]
fn steady_select_five_fifth_completes() {
crate::core_exec::block_on(async {

    let fut1 = steady_fuse_future(delayed_bool(true, 200));
    let fut2 = steady_fuse_future(delayed_bool(true, 200));
    let fut3 = steady_fuse_future(delayed_bool(true, 200));
    let fut4 = steady_fuse_future(delayed_bool(true, 200));
    let fut5 = steady_fuse_future(delayed_bool(false, 100));
    let result = steady_select_five(fut1, fut2, fut3, fut4, fut5).await;
    assert!(!result);
    });
}

/// A dirty wake (`false`) must stop the consolidated wait. Later conditions
/// are not polled, so a shutdown interrupt cannot be papered over by a
/// sibling that would have returned ready.
#[test]
// ss[verify philosophy.single-wake-up]
fn await_for_all_short_circuits_on_dirty_wake() {
    crate::core_exec::block_on(async {
        let polled = AtomicBool::new(false);
        let result = await_for_all!(
            ready(false),
            async {
                polled.store(true, Ordering::SeqCst);
                true
            }
        );
        assert!(!result);
        assert!(
            !polled.load(Ordering::SeqCst),
            "dirty wake must not poll later await_for_all conditions"
        );
    });
}
