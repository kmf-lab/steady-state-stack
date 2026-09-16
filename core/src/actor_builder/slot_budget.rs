//! CPU slot budget for dynamic troupe packing at `Graph::start`.
//!
//! Prefer cgroup CPU quota (containers / Kubernetes) over host `available_parallelism`
//! so we do not spawn host-CPU-count threads inside a 2-vCPU cgroup.
//!
//! Tests MUST inject slots via [`SlotBudget::Pinned`] — never call the host detector
//! from packing property tests.

// ss[impl troupe.dynamic-slot-budget]
// ss[impl philosophy.startup-schedule]
use log::warn;
// ss[related philosophy.structural-hierarchy]
use std::thread::available_parallelism;

/// How many OS threads the packer may create for packed actors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// ss[impl troupe.dynamic-slot-budget]
pub enum SlotBudget {
    /// Explicit pin (`with_pack_slots` / `SS_PACK_SLOTS`). Used by tests and demos.
    Pinned(usize),
    /// Detect from this process’s environment (cgroup quota or `available_parallelism`).
    Detect,
}

// ss[impl troupe.dynamic-slot-budget]
impl SlotBudget {
    /// Resolve to a concrete slot count after subtracting threads already reserved.
    ///
    /// `reserved` = SoloAct OS threads + normal troupe OS threads + telemetry threads (if any).
    // ss[impl troupe.dynamic-slot-budget]
    pub fn resolve(self, reserved: usize) -> usize {
        let detected = match self {
            SlotBudget::Pinned(n) => n.max(1),
            SlotBudget::Detect => detect_parallelism(),
        };
        detected.saturating_sub(reserved).max(1)
    }
}

/// Parse cgroup v2 `cpu.max` quota/period pair into a core count.
// ss[impl troupe.dynamic-slot-budget]
pub(crate) fn parse_cgroup_v2_quota(quota: &str, period: &str) -> Option<usize> {
    if quota == "max" {
        return None;
    }
    let q: i64 = quota.parse().ok()?;
    let p: i64 = period.parse().ok()?;
    if q > 0 && p > 0 {
        let cores = ((q as f64) / (p as f64)).ceil() as usize;
        return Some(cores.max(1));
    }
    None
}

/// Parse cgroup v1 `cpu.cfs_quota_us` / `cpu.cfs_period_us` into a core count.
// ss[impl troupe.dynamic-slot-budget]
pub(crate) fn parse_cgroup_v1_quota_us(quota: &str, period: &str) -> Option<usize> {
    let q: i64 = quota.trim().parse().ok()?;
    let p: i64 = period.trim().parse().ok()?;
    if q < 0 {
        return None;
    }
    if q > 0 && p > 0 {
        let cores = ((q as f64) / (p as f64)).ceil() as usize;
        return Some(cores.max(1));
    }
    None
}

/// Detect usable parallelism for this process.
///
/// Order: `SS_PACK_SLOTS` env (ops override) → Linux cgroup quota → `available_parallelism`.
// ss[impl troupe.dynamic-slot-budget]
pub fn detect_parallelism() -> usize {
    if let Ok(s) = std::env::var("SS_PACK_SLOTS") {
        if let Ok(n) = s.parse::<usize>() {
            return n.max(1);
        }
        warn!("SS_PACK_SLOTS={s:?} is not a positive integer; ignoring");
    }
    if let Some(n) = cgroup_cpu_quota_cores() {
        return n.max(1);
    }
    available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1)
}

/// Parse Linux cgroup v2 `cpu.max` or v1 `cpu.cfs_quota_us` / `cpu.cfs_period_us`.
/// Returns `None` when not in a constrained cgroup or when quota is unlimited.
// ss[impl troupe.dynamic-slot-budget]
fn cgroup_cpu_quota_cores() -> Option<usize> {
    // cgroup v2: /sys/fs/cgroup/cpu.max → "max 100000" or "200000 100000"
    if let Ok(contents) = std::fs::read_to_string("/sys/fs/cgroup/cpu.max") {
        let mut parts = contents.split_whitespace();
        let quota = parts.next()?;
        let period = parts.next().unwrap_or("100000");
        return parse_cgroup_v2_quota(quota, period);
    }
    // cgroup v1
    let quota =
        std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_quota_us").ok()?;
    let period =
        std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_period_us").ok()?;
    parse_cgroup_v1_quota_us(&quota, &period)
}

#[cfg(test)]
// ss[related troupe.dynamic-slot-budget]
mod slot_budget_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    use proptest::prelude::*;

    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    fn pinned_minus_reserved_floors_at_one() {
        assert_eq!(SlotBudget::Pinned(4).resolve(0), 4);
        assert_eq!(SlotBudget::Pinned(4).resolve(3), 1);
        assert_eq!(SlotBudget::Pinned(4).resolve(10), 1);
        assert_eq!(SlotBudget::Pinned(0).resolve(0), 1);
    }

    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    fn detect_resolve_floors_at_one() {
        let slots = SlotBudget::Detect.resolve(0);
        assert!(slots >= 1);
        let slots_reserved = SlotBudget::Detect.resolve(1_000_000);
        assert_eq!(slots_reserved, 1);
    }

    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    fn ss_pack_slots_env_override() {
        // SAFETY: test-only env mutation; single-threaded test runner.
        unsafe { std::env::set_var("SS_PACK_SLOTS", "12") };
        assert_eq!(detect_parallelism(), 12);
        unsafe { std::env::remove_var("SS_PACK_SLOTS") };
    }

    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    fn ss_pack_slots_invalid_env_ignored() {
        unsafe { std::env::set_var("SS_PACK_SLOTS", "not-a-number") };
        let n = detect_parallelism();
        assert!(n >= 1);
        unsafe { std::env::remove_var("SS_PACK_SLOTS") };
    }

    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    fn cgroup_v2_parse_cases() {
        assert_eq!(parse_cgroup_v2_quota("max", "100000"), None);
        assert_eq!(parse_cgroup_v2_quota("200000", "100000"), Some(2));
        assert_eq!(parse_cgroup_v2_quota("150000", "100000"), Some(2));
        assert_eq!(parse_cgroup_v2_quota("0", "100000"), None);
    }

    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    fn cgroup_v1_parse_cases() {
        assert_eq!(parse_cgroup_v1_quota_us("-1", "100000"), None);
        assert_eq!(parse_cgroup_v1_quota_us("200000", "100000"), Some(2));
        assert_eq!(parse_cgroup_v1_quota_us("0", "100000"), None);
    }

    ss_proptest! {
        /// Property: pinned resolve never returns zero and never exceeds pin when reserved=0.
        #[test]
        // ss[verify troupe.dynamic-slot-budget]
        // ss[verify verify.process.proptest]
        fn proptest_pinned_resolve(
            pin in 0usize..64,
            reserved in 0usize..64,
        ) {
            let slots = SlotBudget::Pinned(pin).resolve(reserved);
            prop_assert!(slots >= 1);
            if reserved == 0 {
                prop_assert_eq!(slots, pin.max(1));
            } else {
                prop_assert!(slots <= pin.max(1));
            }
        }

        /// Property: cgroup v2 quota parsing is monotonic in quota for fixed period.
        #[test]
        // ss[verify troupe.dynamic-slot-budget]
        // ss[verify verify.process.proptest]
        fn proptest_cgroup_v2_quota_monotonic(
            quota in 1i64..500_000,
            period in 1i64..200_000,
        ) {
            let cores = parse_cgroup_v2_quota(&quota.to_string(), &period.to_string());
            if let Some(c) = cores {
                prop_assert!(c >= 1);
                let double = parse_cgroup_v2_quota(&(quota * 2).to_string(), &period.to_string());
                if let Some(d) = double {
                    prop_assert!(d >= c);
                }
            }
        }

        /// Property: cgroup v1 negative quota is unlimited (None).
        #[test]
        // ss[verify troupe.dynamic-slot-budget]
        // ss[verify verify.process.proptest]
        fn proptest_cgroup_v1_negative_is_unlimited(
            period in 1i64..200_000,
        ) {
            let neg = -1 - (period % 100);
            prop_assert_eq!(
                parse_cgroup_v1_quota_us(&neg.to_string(), &period.to_string()),
                None
            );
        }
    }
}
