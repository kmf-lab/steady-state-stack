//! Gated wrappers for coverage-guided fuzz targets.
//!
//! Enabled only with `--features fuzzing`. Production graphs must not depend on this module.

use bytes::{Bytes, BytesMut};

/// Parse cgroup v2 `cpu.max` quota/period into a core count.
// ss[impl verify.process.fuzz]
// ss[impl troupe.dynamic-slot-budget]
pub fn parse_cgroup_v2_quota(quota: &str, period: &str) -> Option<usize> {
    crate::actor_builder::slot_budget::parse_cgroup_v2_quota(quota, period)
}

/// Parse cgroup v1 `cpu.cfs_quota_us` / `cpu.cfs_period_us` into a core count.
// ss[impl verify.process.fuzz]
// ss[impl troupe.dynamic-slot-budget]
pub fn parse_cgroup_v1_quota_us(quota: &str, period: &str) -> Option<usize> {
    crate::actor_builder::slot_budget::parse_cgroup_v1_quota_us(quota, period)
}

/// Even-split packable actor ids into at most `slots` clusters.
// ss[impl verify.process.fuzz]
// ss[impl troupe.dynamic-no-incidence-fallback]
pub fn even_split(actor_ids: &[usize], slots: usize) -> Vec<Vec<usize>> {
    crate::actor_builder::pack::even_split(actor_ids, slots)
}

/// Pack actor ids; empty incidence falls back to [`even_split`].
// ss[impl verify.process.fuzz]
// ss[impl troupe.dynamic-no-incidence-fallback]
pub fn pack_actors(actor_ids: &[usize], slots: usize) -> Vec<Vec<usize>> {
    crate::actor_builder::pack::pack_actors(actor_ids, &[], slots)
}

/// Read a FAST signed long; `None` on empty or invalid data.
// ss[impl verify.process.fuzz]
// ss[impl stream.control-payload]
pub fn read_long_signed(byte_buffer: &mut Bytes) -> Option<i64> {
    crate::serialize::fast_protocol_packed::read_long_signed(byte_buffer)
}

/// Read a FAST unsigned long; `None` on empty or invalid data.
// ss[impl verify.process.fuzz]
// ss[impl stream.control-payload]
pub fn read_long_unsigned(byte_buffer: &mut Bytes) -> Option<u64> {
    crate::serialize::fast_protocol_packed::read_long_unsigned(byte_buffer)
}

/// Write a FAST signed long.
// ss[impl verify.process.fuzz]
// ss[impl stream.control-payload]
pub fn write_long_signed(value: i64, byte_buffer: &mut BytesMut) {
    crate::serialize::fast_protocol_packed::write_long_signed(value, byte_buffer)
}

/// Write a FAST unsigned long.
// ss[impl verify.process.fuzz]
// ss[impl stream.control-payload]
pub fn write_long_unsigned(value: u64, byte_buffer: &mut BytesMut) {
    crate::serialize::fast_protocol_packed::write_long_unsigned(value, byte_buffer)
}
