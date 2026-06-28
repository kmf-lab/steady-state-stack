//! Monitoring utilities for inspecting channel and actor metrics at runtime.
//!
//! The `monitor` module provides types and traits for gathering and representing runtime metadata
//! about channels and actors, enabling integration with telemetry systems and health checks.

mod helpers;
mod metadata;
mod telemetry_iface;

pub use metadata::{
    ActorMetaData, ActorStatus, ChannelMetaData, RxMetaData, RxMetaDataHolder, ThreadInfo,
    TxMetaData, TxMetaDataHolder,
};
pub use telemetry_iface::RxTel;

pub(crate) use helpers::{DriftCountIterator, FinallyRollupProfileGuard, find_my_index};
pub(crate) use metadata::{
    CALL_BATCH_READ, CALL_BATCH_WRITE, CALL_OTHER, CALL_SINGLE_READ, CALL_SINGLE_WRITE, CALL_WAIT,
    METADATA_REGISTRY,
};

pub(crate) use crate::graph_liveliness::ActorIdentity;
