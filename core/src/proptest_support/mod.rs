//! Shared proptest strategies and harness helpers for `steady_state` unit tests.

// ss[impl verify.process.proptest]
use proptest::prelude::*;

/// Unified property-test case count for all Tier-0 modules.
// ss[impl verify.process.proptest]
pub const SS_PROPCASES: u32 = crate::SS_PROPCASES;

/// Default case count for all property tests (2048 cases).
// ss[impl verify.process.proptest]
pub fn default_config() -> ProptestConfig {
    ProptestConfig::with_cases(SS_PROPCASES)
}

/// Alias for `default_config()` — all properties use the same 2048-case space.
// ss[impl verify.process.proptest]
pub fn critical_config() -> ProptestConfig {
    default_config()
}

/// For proptests that call `ChannelBuilder::eager_build` per case (registry + channel setup).
// ss[impl verify.process.proptest]
pub fn telemetry_eager_config() -> ProptestConfig {
    ProptestConfig::with_cases(64)
}

/// Percent inputs that may be in or out of the valid [0, 100] range.
// ss[impl verify.process.proptest]
pub fn percent_f32() -> impl Strategy<Value = f32> {
    -10.0f32..110.0f32
}

/// Percent inputs that may be in or out of the valid [0, 100] range (f64).
// ss[impl verify.process.proptest]
pub fn percent_f64() -> impl Strategy<Value = f64> {
    -10.0f64..110.0f64
}

/// Valid channel capacity for property tests.
// ss[impl verify.process.proptest]
pub fn capacity() -> impl Strategy<Value = usize> {
    1usize..128
}

/// Lane count for bundle/index-wait properties.
// ss[impl verify.process.proptest]
pub fn lane_count() -> impl Strategy<Value = usize> {
    1usize..16
}

/// Random message vector capped for channel FIFO properties.
// ss[impl verify.process.proptest]
pub fn message_vec<T: Arbitrary>() -> impl Strategy<Value = Vec<T>> {
    prop::collection::vec(any::<T>(), 0..64)
}

/// Vote matrix: `voters` rows of yes/no votes for property tests.
// ss[impl verify.process.proptest]
pub fn vote_matrix(voters: usize) -> impl Strategy<Value = Vec<Vec<bool>>> {
    prop::collection::vec(prop::collection::vec(any::<bool>(), voters), 1..8)
}

/// Random bit mask for `len` lanes (index `i` is ready when bit i is set).
// ss[impl verify.process.proptest]
pub fn lane_mask(len: usize) -> impl Strategy<Value = u16> {
    if len == 0 || len > 16 {
        Just(0u16).boxed()
    } else {
        (0u16..=(u16::MAX >> (16 - len))).boxed()
    }
}

/// Aeron UDP port range used in URI contract property tests.
// ss[impl verify.process.proptest]
pub fn aeron_port() -> impl Strategy<Value = u16> {
    40100u16..41200u16
}

/// Aeron term-length values used in URI contract property tests.
// ss[impl verify.process.proptest]
pub fn aeron_term_length() -> impl Strategy<Value = usize> {
    prop::sample::select(vec![4096usize, 65536, 1_048_576])
}

/// Build a lazy channel, send messages via testing API, drain RX FIFO into a `Vec`.
// ss[impl verify.process.proptest]
pub fn channel_fifo_take<T>(capacity: usize, messages: Vec<T>) -> Vec<T>
where
    T: Clone + std::fmt::Debug + PartialEq,
{
    // ss[impl verify.process.proptest]
    use crate::channel_builder::ChannelBuilder;
    let messages: Vec<T> = messages.into_iter().take(capacity).collect();
    let builder = ChannelBuilder::default().with_capacity(capacity);
    let (tx_lazy, rx_lazy) = builder.build_channel::<T>();
    tx_lazy.testing_send_all(messages.clone(), false);
    let rx = rx_lazy.clone();
    let mut ste_rx = crate::core_exec::block_on(rx.lock());
    let mut taken = Vec::with_capacity(messages.len());
    while let Some(v) = ste_rx.try_take() {
        taken.push(v);
    }
    taken
}
