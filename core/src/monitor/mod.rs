//! Monitoring utilities for inspecting channel and actor metrics at runtime.
//!
//! The `monitor` module provides types and traits for gathering and representing runtime metadata
//! about channels and actors, enabling integration with telemetry systems and health checks.

// ss[related philosophy.single-wake-up]
mod helpers;
// ss[related philosophy.structural-hierarchy]
mod metadata;
// ss[related philosophy.structural-hierarchy]
mod telemetry_iface;

// ss[related philosophy.single-wake-up]
pub use metadata::{
    ActorMetaData, ActorStatus, ChannelMetaData, RxMetaData, RxMetaDataHolder, ThreadInfo,
    TxMetaData, TxMetaDataHolder,
};
// ss[related philosophy.single-wake-up]
pub use telemetry_iface::RxTel;

// ss[related philosophy.structural-hierarchy]
pub(crate) use helpers::{DriftCountIterator, FinallyRollupProfileGuard, find_my_index};
// ss[related philosophy.single-wake-up]
pub(crate) use metadata::{
    CALL_BATCH_READ, CALL_BATCH_WRITE, CALL_OTHER, CALL_SINGLE_READ, CALL_SINGLE_WRITE, CALL_WAIT,
    channel_memory_footprint, METADATA_REGISTRY,
};

// ss[related philosophy.single-wake-up]
pub(crate) use crate::graph_liveliness::ActorIdentity;
