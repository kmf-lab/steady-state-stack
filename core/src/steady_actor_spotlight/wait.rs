//! Wait helpers for `SteadyActorSpotlight`.
//! Trait methods in `mod.rs` delegate here so the spotlight file stays under the 1,200-line budget.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use futures_timer::Delay;
use futures_util::select;
use std::time::Duration;
use crate::monitor::{CALL_OTHER, CALL_WAIT};
use crate::{yield_now, RxCore, TxCore, RxCoreBundle, TxCoreBundle};
use crate::steady_actor::{index_wait_avoid_repeat_lane, next_index_wait_start};

// ss[related actor.shadow-spotlight]
pub(super) async fn wait_avail<const RX_LEN: usize, const TX_LEN: usize, T: RxCore>(spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>, this: &mut T, count: usize) -> bool {
    let _guard = spotlight.start_profile(CALL_OTHER);

    let count = this.shared_validate_capacity_items(count);

    if this.shared_avail_items_count() >= count {
        true
    } else {
        if spotlight.telemetry.is_dirty() {
            let remaining_micros = spotlight.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                yield_now().await; //Important to avoid tight loops
                false
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = this.shared_wait_closed_or_avail_units(count);
                select! {
                    _ = spotlight.oneshot_shutdown.clone().fuse() => false,
                    _ = dur.fuse() => false,
                    x = wat.fuse() => x
                }
            }
        } else {
            select! {
                _ = spotlight.oneshot_shutdown.clone().fuse() => false,
                x = this.shared_wait_closed_or_avail_units(count).fuse() => x,
            }
        }
    }

}

// ss[related actor.shadow-spotlight]
pub(super) async fn wait_vacant<const RX_LEN: usize, const TX_LEN: usize, T: TxCore>(spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>, this: &mut T, size: T::MsgSize) -> bool {
    let _guard = spotlight.start_profile(CALL_WAIT);
    if this.shared_vacant_units_for(size) {
        true
    } else {

        if spotlight.telemetry.is_dirty() {
            let remaining_micros = spotlight.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                yield_now().await; //Important to avoid tight loops
                false //immediate return to do telemetry will be back later
            } else {
                    let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                    let wat = this.shared_wait_shutdown_or_vacant_units(size);
                    select! {
                        _ = spotlight.oneshot_shutdown.clone().fuse() => false,
                        _ = dur.fuse() => false,
                        x = wat.fuse() => x
                    }
            }
        } else {
            select! {
                _ = spotlight.oneshot_shutdown.clone().fuse() => false,
                x = this.shared_wait_shutdown_or_vacant_units(size).fuse() => x,
            }
        }
    }
}

// ss[related actor.shadow-spotlight]
pub(super) async fn wait_shutdown<const RX_LEN: usize, const TX_LEN: usize>(spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>) -> bool {
    let _guard = spotlight.start_profile(CALL_OTHER);
    if spotlight.telemetry.is_dirty() {
        let remaining_micros = spotlight.telemetry_remaining_micros();
        if remaining_micros <= 0 && spotlight.is_liveliness_running() {
            false
        } else {
            let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
            let mut shut = spotlight.oneshot_shutdown.clone().fuse();
            select! {
                _ = shut => true,
                _ = dur.fuse() => false,
            }
        }
    } else {
        spotlight.internal_wait_shutdown().await
    }
}

#[allow(deprecated)]
// ss[related actor.shadow-spotlight]
pub(super) async fn wait_vacant_bundle<const RX_LEN: usize, const TX_LEN: usize, T: TxCore>(spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>, this: &mut TxCoreBundle<'_, T>, size: T::MsgSize, ready_channels: usize) -> bool {
    let _guard = spotlight.start_profile(CALL_OTHER);
    let count_down = ready_channels.min(this.len());
    let result = Arc::new(AtomicBool::new(true));
    let mut futures = FuturesUnordered::new();   //TODO: optimize this similar to wait_vacant if possible
    for tx in this.iter_mut().take(count_down) {
        let local_r = result.clone();
        futures.push(async move {
            let bool_result = tx.shared_wait_shutdown_or_vacant_units(size).await;
            if !bool_result {
                local_r.store(false, Ordering::Relaxed);
            }
        });
    }
    let mut completed = 0;
    loop {
        if completed >= count_down {
            break;
        }
        select! {
            _ = spotlight.oneshot_shutdown.clone().fuse() => {
                result.store(false, Ordering::Relaxed);
                break;
            }
            next = futures.next() => {
                if next.is_some() {
                    completed += 1;
                } else {
                    break;
                }
            }
        }
    }
    result.load(Ordering::Relaxed)
}

#[allow(deprecated)]
// ss[related actor.shadow-spotlight]
pub(super) async fn wait_avail_bundle<const RX_LEN: usize, const TX_LEN: usize, T: RxCore>(spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>, this: &mut RxCoreBundle<'_, T>, item_count: usize, ready_channels: usize) -> bool {
    let _guard = spotlight.start_profile(CALL_OTHER);
    let count_down = ready_channels.min(this.len());
    let result = Arc::new(AtomicBool::new(true));
    let mut futures = FuturesUnordered::new();
    for rx in this.iter_mut().take(count_down) {
        let local_r = result.clone();
        futures.push(async move {
            let bool_result = rx.shared_wait_closed_or_avail_units(item_count).await;
            if !bool_result {
                local_r.store(false, Ordering::Relaxed);
            }
        });
    }
    let mut completed = 0;
    loop {
        if completed >= count_down {
            break;
        }
        select! {
            _ = spotlight.oneshot_shutdown.clone().fuse() => {
                result.store(false, Ordering::Relaxed);
                break;
            }
            next = futures.next() => {
                if next.is_some() {
                    completed += 1;
                } else {
                    break;
                }
            }
        }
    }
    result.load(Ordering::Relaxed)
}

// ss[related actor.shadow-spotlight]
pub(super) async fn wait_avail_index<const RX_LEN: usize, const TX_LEN: usize, T: RxCore>(
    spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>,
    this: &mut RxCoreBundle<'_, T>,
    counts: &[usize],
) -> Option<usize> {
    debug_assert_eq!(this.len(), counts.len(), "wait_avail_index: bundle and counts length mismatch");

    let len = this.len();
    if len == 0 {
        return None;
    }
    let last_stored = spotlight.index_wait_last_avail.load(Ordering::Relaxed);
    let start = next_index_wait_start(last_stored, len);
    let len_start = len - start;

    // Check if any channel already satisfies its count (round-robin start)
    let (head, tail) = this.split_at_mut(start);
    for (step, rx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
        let i = if step < len_start {
            start + step
        } else {
            step - len_start
        };
        if counts[i] > 0 && rx.shared_avail_items_count() >= counts[i] {
            let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                counts[j] > 0 && this[j].shared_avail_items_count() >= counts[j]
            });
            spotlight.index_wait_last_avail.store(picked, Ordering::Relaxed);
            return Some(picked);
        }
    }

    // Build a FuturesUnordered for channels with count > 0 (rotated order)
    let mut futures = FuturesUnordered::new();
    let (head, tail) = this.split_at_mut(start);
    for (step, rx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
        let i = if step < len_start {
            start + step
        } else {
            step - len_start
        };
        if counts[i] == 0 {
            continue;
        }
        let required = counts[i];
        futures.push(async move {
            if wait_rx_until_avail_items_ready(rx, required).await {
                Some(i)
            } else {
                None
            }
        });
    }

    let _guard = spotlight.start_profile(CALL_OTHER);
    if futures.is_empty() {
        return None;
    }
    loop {
        select! {
            _ = spotlight.oneshot_shutdown.clone().fuse() => return None,
            next = futures.next() => {
                match next {
                    Some(Some(i)) => {
                        drop(futures);
                        let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                            counts[j] > 0 && this[j].shared_avail_items_count() >= counts[j]
                        });
                        spotlight.index_wait_last_avail.store(picked, Ordering::Relaxed);
                        return Some(picked);
                    }
                    Some(None) => {}
                    None => return None,
                }
            }
        }
    }
}

// ss[related actor.shadow-spotlight]
pub(super) async fn wait_vacant_index<const RX_LEN: usize, const TX_LEN: usize, T: TxCore>(
    spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>,
    this: &mut TxCoreBundle<'_, T>,
    counts: &[T::MsgSize],
) -> Option<usize> {
    debug_assert_eq!(this.len(), counts.len(), "wait_vacant_index: bundle and counts length mismatch");

    let len = this.len();
    if len == 0 {
        return None;
    }
    let last_stored = spotlight.index_wait_last_vacant.load(Ordering::Relaxed);
    let start = next_index_wait_start(last_stored, len);

    let _guard = spotlight.start_profile(CALL_OTHER);

    let len_start = len - start;

    // Fast path: already enough vacancy (round-robin start)
    let (head, tail) = this.split_at_mut(start);
    for (step, tx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
        let i = if step < len_start {
            start + step
        } else {
            step - len_start
        };
        if tx.shared_vacant_units_for(counts[i]) {
            let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                this[j].shared_vacant_units_for(counts[j])
            });
            spotlight.index_wait_last_vacant.store(picked, Ordering::Relaxed);
            return Some(picked);
        }
    }

    // Build a FuturesUnordered for all channels (rotated order).
    let mut futures = FuturesUnordered::new();
    let (head, tail) = this.split_at_mut(start);
    for (step, tx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
        let i = if step < len_start {
            start + step
        } else {
            step - len_start
        };
        let required = counts[i];
        futures.push(async move {
            if wait_tx_until_vacant_satisfied(tx, required).await {
                Some(i)
            } else {
                None
            }
        });
    }

    loop {
        select! {
            _ = spotlight.oneshot_shutdown.clone().fuse() => return None,
            next = futures.next() => {
                match next {
                    Some(Some(i)) => {
                        drop(futures);
                        let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                            this[j].shared_vacant_units_for(counts[j])
                        });
                        spotlight.index_wait_last_vacant.store(picked, Ordering::Relaxed);
                        return Some(picked);
                    }
                    Some(None) => {}
                    None => return None,
                }
            }
        }
    }
}

// ss[related actor.shadow-spotlight]
pub(super) async fn wait_avail_vacant_index<const RX_LEN: usize, const TX_LEN: usize, R: RxCore, T: TxCore>(
    spotlight: &SteadyActorSpotlight<RX_LEN, TX_LEN>,
    rx: &mut RxCoreBundle<'_, R>,
    tx: &mut TxCoreBundle<'_, T>,
    avail_counts: &[usize],
    vacant_counts: &[T::MsgSize],
) -> Option<usize> {
    debug_assert_eq!(rx.len(), tx.len(), "wait_avail_vacant_index: rx and tx bundle length mismatch");
    debug_assert_eq!(rx.len(), avail_counts.len(), "wait_avail_vacant_index: rx bundle and avail_counts length mismatch");
    debug_assert_eq!(rx.len(), vacant_counts.len(), "wait_avail_vacant_index: rx bundle and vacant_counts length mismatch");

    let len = rx.len();
    if len == 0 {
        return None;
    }

    let _guard = spotlight.start_profile(CALL_OTHER);

    let last_stored = spotlight.index_wait_last_avail_vacant.load(Ordering::Relaxed);
    let start = next_index_wait_start(last_stored, len);
    let len_start = len - start;

    let (rx_head, rx_tail) = rx.split_at_mut(start);
    let (tx_head, tx_tail) = tx.split_at_mut(start);
    for (step, (rx_i, tx_i)) in rx_tail
        .iter_mut()
        .chain(rx_head.iter_mut())
        .zip(tx_tail.iter_mut().chain(tx_head.iter_mut()))
        .enumerate()
    {
        let i = if step < len_start {
            start + step
        } else {
            step - len_start
        };
        let rx_ok = avail_counts[i] == 0 || rx_i.shared_avail_items_count() >= avail_counts[i];
        let tx_ok = tx_i.shared_vacant_units_for(vacant_counts[i]);
        if rx_ok && tx_ok {
            let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                let rx_ok_j = avail_counts[j] == 0 || rx[j].shared_avail_items_count() >= avail_counts[j];
                let tx_ok_j = tx[j].shared_vacant_units_for(vacant_counts[j]);
                rx_ok_j && tx_ok_j
            });
            spotlight.index_wait_last_avail_vacant.store(picked, Ordering::Relaxed);
            return Some(picked);
        }
    }

    let mut futures = FuturesUnordered::new();
    let (rx_head, rx_tail) = rx.split_at_mut(start);
    let (tx_head, tx_tail) = tx.split_at_mut(start);
    for (step, (rx_i, tx_i)) in rx_tail
        .iter_mut()
        .chain(rx_head.iter_mut())
        .zip(tx_tail.iter_mut().chain(tx_head.iter_mut()))
        .enumerate()
    {
        let i = if step < len_start {
            start + step
        } else {
            step - len_start
        };
        let rx_ok = avail_counts[i] == 0 || rx_i.shared_avail_items_count() >= avail_counts[i];
        let tx_ok = tx_i.shared_vacant_units_for(vacant_counts[i]);
        if rx_ok && tx_ok {
            continue;
        }

        let required_avail = avail_counts[i];
        let required_vacant = vacant_counts[i];
        futures.push(async move {
            if wait_paired_lane_ready(rx_i, tx_i, required_avail, required_vacant).await {
                Some(i)
            } else {
                None
            }
        });
    }

    if futures.is_empty() {
        return None;
    }

    loop {
        select! {
            _ = spotlight.oneshot_shutdown.clone().fuse() => return None,
            next = futures.next() => {
                match next {
                    Some(Some(i)) => {
                        drop(futures);
                        let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                            let rx_ok_j = avail_counts[j] == 0 || rx[j].shared_avail_items_count() >= avail_counts[j];
                            let tx_ok_j = tx[j].shared_vacant_units_for(vacant_counts[j]);
                            rx_ok_j && tx_ok_j
                        });
                        spotlight.index_wait_last_avail_vacant.store(picked, Ordering::Relaxed);
                        return Some(picked);
                    }
                    Some(None) => {}
                    None => return None,
                }
            }
        }
    }
}
