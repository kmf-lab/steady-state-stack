// ss[related bundle.split-macro]
use crate::{LazySteadyRx, LazySteadyRxBundle, LazySteadyTx, LazySteadyTxBundle, SteadyRx, SteadyTx, SteadyActor, SteadyTxBundle, SteadyRxBundle};
use crate::simulate_edge::IntoSimRunner;
use crate::distributed::aqueduct_stream::{SteadyStreamRx, SteadyStreamTx, StreamControlItem};
// ss[related bundle.split-macro]
use async_ringbuf::Arc;
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;

/// Trait to allow uniform flattening of channels and bundles into simulation runners.
// ss[related bundle.split-macro]
pub trait SimIndexable<C: SteadyActor + 'static> {
    /// Pushes references to simulation runners into the provided vector.
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>);
}

// ss[related bundle.split-macro]
impl<T, C> SimIndexable<C> for SteadyRx<T>
    where SteadyRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

// ss[related bundle.split-macro]
impl<T, C> SimIndexable<C> for SteadyTx<T>
    where SteadyTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

// ss[related bundle.split-macro]
impl<T, C> SimIndexable<C> for SteadyStreamRx<T>
    where SteadyStreamRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

// ss[related bundle.split-macro]
impl<T, C> SimIndexable<C> for SteadyStreamTx<T>
    where SteadyStreamTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

// ss[related bundle.split-macro]
impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyRx<T>; N]>
    where SteadyRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

// ss[related bundle.split-macro]
impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyTx<T>; N]>
    where SteadyTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

// ss[related bundle.split-macro]
impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyStreamRx<T>; N]>
    where SteadyStreamRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

// ss[related bundle.split-macro]
impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyStreamTx<T>; N]>
    where SteadyStreamTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

/// Trait to allow uniform indexing into metadata collections (bundles or single channels).
/// This is used by the `rx_meta_data!` and `tx_meta_data!` macros to handle mixed types.
// ss[related bundle.split-macro]
pub trait MetaIndexable<T: ?Sized> {
    /// Returns a reference to the metadata at the specified index.
    fn meta_at(&self, index: usize) -> &T;
    /// Returns the number of metadata items in the collection.
    // ss[related bundle.split-macro]
    fn meta_len(&self) -> usize;
}

// Implementation for bundles (Arc of array of providers)
// ss[related bundle.split-macro]
impl<T: RxMetaDataProvider + 'static, const N: usize> MetaIndexable<dyn RxMetaDataProvider> for Arc<[T; N]> {
    fn meta_at(&self, index: usize) -> &(dyn RxMetaDataProvider + 'static) { &self[index] }
    fn meta_len(&self) -> usize { N }
}

// ss[related bundle.split-macro]
impl<T: TxMetaDataProvider + 'static, const N: usize> MetaIndexable<dyn TxMetaDataProvider> for Arc<[T; N]> {
    fn meta_at(&self, index: usize) -> &(dyn TxMetaDataProvider + 'static) { &self[index] }
    fn meta_len(&self) -> usize { N }
}

// Implementation for single channels or already-extracted metadata
// ss[related bundle.split-macro]
impl<M: RxMetaDataProvider + 'static> MetaIndexable<dyn RxMetaDataProvider> for M {
    fn meta_at(&self, _index: usize) -> &(dyn RxMetaDataProvider + 'static) { self }
    fn meta_len(&self) -> usize { 1 }
}

// ss[related bundle.split-macro]
impl<M: TxMetaDataProvider + 'static> MetaIndexable<dyn TxMetaDataProvider> for M {
    fn meta_at(&self, _index: usize) -> &(dyn TxMetaDataProvider + 'static) { self }
    fn meta_len(&self) -> usize { 1 }
}

// Implementation for raw arrays of references (output of bundle.meta_data())
// ss[related bundle.split-macro]
impl<'a, T: ?Sized + 'a, const N: usize> MetaIndexable<T> for [&'a T; N] {
    fn meta_at(&self, index: usize) -> &T { self[index] }
    fn meta_len(&self) -> usize { N }
}

/// Internal helper for recursive offset calculation within metadata arrays.
/// This macro navigates through multiple arrays to find the element at a specific global offset.
/// Do not call this directly.
#[macro_export]
// ss[related bundle.split-macro]
macro_rules! __concat_meta_impl {
    ($offset:expr; $trait:ty; $last:expr) => {{
        use $crate::macros::MetaIndexable;
        $last.meta_at($offset)
    }};

    ($offset:expr; $trait:ty; $head:expr, $($tail:expr),+) => {{
        use $crate::macros::MetaIndexable;
        let len = $head.meta_len();
        if $offset < len {
            $head.meta_at($offset)
        } else {
            $crate::__concat_meta_impl!($offset - len; $trait; $($tail),+)
        }
    }};
}

/// Concatenates multiple RX metadata sources into a single array of trait objects.
///
/// Accepts any combination of single receivers or receiver bundles.
/// Enforces that the resulting trait objects implement `RxMetaDataProvider`.
#[macro_export]
// ss[related bundle.split-macro]
macro_rules! rx_meta_data {
    ($len:expr; $($item:expr),+ $(,)?) => {{
        let result: [&dyn $crate::steady_rx::RxMetaDataProvider; $len] =
            std::array::from_fn(|i| {
                $crate::__concat_meta_impl!(i; dyn $crate::steady_rx::RxMetaDataProvider; $($item),+)
            });
        result
    }};
    ($($item:expr),+ $(,)?) => {{
        std::array::from_fn(|i| {
            $crate::__concat_meta_impl!(i; dyn $crate::steady_rx::RxMetaDataProvider; $($item),+)
        })
    }};
}

/// Concatenates multiple TX metadata sources into a single array of trait objects.
///
/// Accepts any combination of single transmitters or transmitter bundles.
/// Enforces that the resulting trait objects implement `TxMetaDataProvider`.
#[macro_export]
// ss[related bundle.split-macro]
macro_rules! tx_meta_data {
    ($len:expr; $($item:expr),+ $(,)?) => {{
        let result: [&dyn $crate::steady_tx::TxMetaDataProvider; $len] =
            std::array::from_fn(|i| {
                $crate::__concat_meta_impl!(i; dyn $crate::steady_tx::TxMetaDataProvider; $($item),+)
            });
        result
    }};
    ($($item:expr),+ $(,)?) => {{
        std::array::from_fn(|i| {
            $crate::__concat_meta_impl!(i; dyn $crate::steady_tx::TxMetaDataProvider; $($item),+)
        })
    }};
}

/// Concatenates multiple channels or bundles into a Vec of simulation runners.
///
/// Automatically flattens bundles and handles trait object casting for `simulated_behavior`.
#[macro_export]
// ss[related bundle.split-macro]
macro_rules! sim_runners {
    ($($item:expr),+ $(,)?) => {{
        let mut runners = Vec::new();
        $(
            $crate::macros::SimIndexable::push_to(&$item, &mut runners);
        )+
        runners
    }};
}

/////////////////////////////////////////////////////////////////////////////////

/// Macro for creating a LocalMonitor from channels.
///
/// Takes a `SteadyContext` and lists of Rx and Tx channels, returning a `LocalMonitor` for telemetry and Prometheus metrics.
#[macro_export]
// ss[related philosophy.structural-hierarchy]
macro_rules! into_monitor {
    ($self:expr, [$($rx:expr),*], [$($tx:expr),*]) => {{
        #[allow(unused_imports)]
        // ss[related philosophy.structural-hierarchy]
        use $crate::steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use $crate::steady_tx::TxMetaDataProvider;
        let rx_meta = [$($rx.meta_data(),)*];
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, tx_meta)
    }};
    ($self:expr, [$($rx:expr),*], $tx_bundle:expr) => {{
        #[allow(unused_imports)]
        // ss[related philosophy.structural-hierarchy]
        use $crate::steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use $crate::steady_tx::TxMetaDataProvider;
        let rx_meta = [$($rx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, $tx_bundle.meta_data())
    }};
    ($self:expr, $rx_bundle:expr, [$($tx:expr),*]) => {{
        #[allow(unused_imports)]
        // ss[related philosophy.structural-hierarchy]
        use $crate::steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use $crate::steady_tx::TxMetaDataProvider;
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal($rx_bundle.meta_data(), tx_meta)
    }};
    ($self:expr, $rx_bundle:expr, $tx_bundle:expr) => {{
        $self.into_monitor_internal($rx_bundle.meta_data(), $tx_bundle.meta_data())
    }};
    ($self:expr, ($rx_channels_to_monitor:expr, [$($rx:expr),*], $($rx_bundle:expr),* ), ($tx_channels_to_monitor:expr, [$($tx:expr),*], $($tx_bundle:expr),* )) => {{
        #[allow(unused_imports)]
        // ss[related philosophy.structural-hierarchy]
        use $crate::steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use $crate::steady_tx::TxMetaDataProvider;
        let mut rx_count = [$( { $rx; 1 } ),*].len();
        $(
            rx_count += $rx_bundle.meta_data().len();
        )*
        assert_eq!(rx_count, $rx_channels_to_monitor, "Mismatch in RX channel count");

        let mut tx_count = [$( { $tx; 1 } ),*].len();
        $(
            tx_count += $tx_bundle.meta_data().len();
        )*
        assert_eq!(tx_count, $tx_channels_to_monitor, "Mismatch in TX channel count");

        let mut rx_mon = [$crate::monitor::RxMetaData::default(); $rx_channels_to_monitor];
        let mut rx_index = 0;
        $(
            rx_mon[rx_index] = $rx.meta_data();
            rx_index += 1;
        )*
        $(
            for meta in $rx_bundle.meta_data() {
                rx_mon[rx_index] = meta;
                rx_index += 1;
            }
        )*

        let mut tx_mon = [$crate::monitor::TxMetaData::default(); $tx_channels_to_monitor];
        let mut tx_index = 0;
        $(
            tx_mon[tx_index] = $tx.meta_data();
            tx_index += 1;
        )*
        $(
            for meta in $tx_bundle.meta_data() {
                tx_mon[tx_index] = meta;
                tx_index += 1;
            }
        )*

        $self.into_monitor_internal(rx_mon, tx_mon)
    }};
    ($self:expr, ($rx_channels_to_monitor:expr, [$($rx:expr),*]), ($tx_channels_to_monitor:expr, [$($tx:expr),*], $($tx_bundle:expr),* )) => {{
        #[allow(unused_imports)]
        // ss[related philosophy.structural-hierarchy]
        use $crate::steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use $crate::steady_tx::TxMetaDataProvider;
        let mut rx_count = [$( { $rx; 1 } ),*].len();
        assert_eq!(rx_count, $rx_channels_to_monitor, "Mismatch in RX channel count");

        let mut tx_count = [$( { $tx; 1 } ),*].len();
        $(
            tx_count += $tx_bundle.meta_data().len();
        )*
        assert_eq!(tx_count, $tx_channels_to_monitor, "Mismatch in TX channel count");

        let mut rx_mon = [$crate::monitor::RxMetaData::default(); $rx_channels_to_monitor];
        let mut rx_index = 0;
        $(
            rx_mon[rx_index] = $rx.meta_data();
            rx_index += 1;
        )*

        let mut tx_mon = [$crate::monitor::TxMetaData::default(); $tx_channels_to_monitor];
        let mut tx_index = 0;
        $(
            tx_mon[tx_index] = $tx.meta_data();
            tx_index += 1;
        )*
        $(
            for meta in $tx_bundle.meta_data() {
                tx_mon[tx_index] = meta;
                tx_index += 1;
            }
        )*

        $self.into_monitor_internal(rx_mon, tx_mon)
    }};
}

/// Splits a bundle into multiple parts using constants or literals.
///
/// This version supports constant identifiers (e.g., PDF_F) and ensures
/// that the "Capital Allocation" of channels matches the total supply
/// at compile time.
// ss[impl bundle.split-macro]
#[macro_export]
macro_rules! split_bundle {
    ($bundle:expr, $($size:expr),+ $(,)?) => {{
        let bundle = $bundle;

        // Runtime check (because stable can't do `const` sum over const generics).
        let bundle_len = bundle.len();
        let requested_len: usize = 0 $(+ $size)*;
        assert_eq!(
            requested_len,
            bundle_len,
            "split_bundle: requested sizes sum to {}, but bundle len is {}",
            requested_len,
            bundle_len
        );

        let mut it = bundle.into_iter();

        let parts = (
            $({
                let part: [_; $size] = ::std::array::from_fn(|_| {
                    it.next()
                        .expect("split_bundle: not enough elements for part")
                });
                part
            }),+
        );

        // If requested_len == bundle_len, this is redundant; keep as a sanity check.
        debug_assert!(
            it.next().is_none(),
            "split_bundle: sizes did not consume entire bundle"
        );

        parts
    }};
}

// In your existing code file, you can add these to the trait area
// to allow the macro results to be treated as bundles immediately.

/// Helper to convert a raw array of transmitters into a SteadyTxBundle.
// ss[related bundle.split-macro]
pub fn steady_tx_bundle<T, const GIRTH: usize>(
    inner: [LazySteadyTx<T>; GIRTH]
) -> LazySteadyTxBundle<T, GIRTH> {
    inner
}

/// Helper to convert a raw array of receivers into a SteadyRxBundle.
// ss[related bundle.split-macro]
pub fn steady_rx_bundle<T, const GIRTH: usize>(
    inner: [LazySteadyRx<T>; GIRTH]
) -> LazySteadyRxBundle<T, GIRTH> {
    inner
}

/// Helper to convert a raw array of active transmitters into a SteadyTxBundle.
// ss[related bundle.split-macro]
pub fn steady_tx_bundle_active<T, const GIRTH: usize>(
    inner: [SteadyTx<T>; GIRTH]
) -> SteadyTxBundle<T, GIRTH> {
    Arc::new(inner)
}

/// Helper to convert a raw array of active receivers into a SteadyRxBundle.
// ss[related bundle.split-macro]
pub fn steady_rx_bundle_active<T, const GIRTH: usize>(
    inner: [SteadyRx<T>; GIRTH]
) -> SteadyRxBundle<T, GIRTH> {
    Arc::new(inner)
}

#[cfg(test)]
// ss[related bundle.split-macro]
mod macros_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;

    // ss[verify bundle.split-macro]
    #[test]
    fn test_split_bundle_partitions_lazy_tx_bundle() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (t0, _) = builder.build_channel::<u8>();
        let (t1, _) = builder.build_channel::<u8>();
        let (t2, _) = builder.build_channel::<u8>();
        let (t3, _) = builder.build_channel::<u8>();
        let bundle = steady_tx_bundle([t0, t1, t2, t3]);
        let (a, b) = split_bundle!(bundle, 2, 2);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
    }

    #[test]
    // ss[verify bundle.split-macro]
    fn test_steady_tx_bundle() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx0, _rx0) = builder.build_channel::<u8>();
        let (tx1, _rx1) = builder.build_channel::<u8>();
        let inner: [LazySteadyTx<u8>; 2] = [tx0, tx1];
        let bundle: LazySteadyTxBundle<u8, 2> = steady_tx_bundle(inner);
        assert_eq!(bundle.len(), 2);
    }

    #[test]
    // ss[verify bundle.split-macro]
    fn test_steady_rx_bundle() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (_tx0, rx0) = builder.build_channel::<u8>();
        let (_tx1, rx1) = builder.build_channel::<u8>();
        let inner: [LazySteadyRx<u8>; 2] = [rx0, rx1];
        let bundle: LazySteadyRxBundle<u8, 2> = steady_rx_bundle(inner);
        assert_eq!(bundle.len(), 2);
    }

    #[test]
    // ss[verify bundle.split-macro]
    fn test_steady_tx_bundle_active() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx0, _rx0) = builder.build_channel::<u8>();
        let (tx1, _rx1) = builder.build_channel::<u8>();
        let active0 = tx0.clone();
        let active1 = tx1.clone();
        let inner: [SteadyTx<u8>; 2] = [active0, active1];
        let bundle: SteadyTxBundle<u8, 2> = steady_tx_bundle_active(inner);
        assert_eq!(bundle.len(), 2);
    }

    #[test]
    // ss[verify bundle.split-macro]
    fn test_steady_rx_bundle_active() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (_tx0, rx0) = builder.build_channel::<u8>();
        let (_tx1, rx1) = builder.build_channel::<u8>();
        let active0 = rx0.clone();
        let active1 = rx1.clone();
        let inner: [SteadyRx<u8>; 2] = [active0, active1];
        let bundle: SteadyRxBundle<u8, 2> = steady_rx_bundle_active(inner);
        assert_eq!(bundle.len(), 2);
    }
}

#[cfg(test)]
// ss[related bundle.split-macro]
mod macros_proptest {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::ss_proptest;
    use proptest::prelude::*;

    ss_proptest! {
        /// Property: split_bundle 2+2 on a four-lane lazy TX bundle preserves total length.
        #[test]
        // ss[verify bundle.split-macro]
        // ss[verify verify.process.proptest]
        fn proptest_split_bundle_four_lane_partitions(cap in 1usize..8) {
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (t0, _) = builder.build_channel::<u8>();
            let (t1, _) = builder.build_channel::<u8>();
            let (t2, _) = builder.build_channel::<u8>();
            let (t3, _) = builder.build_channel::<u8>();
            let bundle = steady_tx_bundle([t0, t1, t2, t3]);
            let (a, b) = split_bundle!(bundle, 2, 2);
            prop_assert_eq!(a.len() + b.len(), 4);
            prop_assert_eq!(a.len(), 2);
            prop_assert_eq!(b.len(), 2);
        }

        /// Property: split_bundle 1+3 partitions a four-lane bundle without dropping lanes.
        #[test]
        // ss[verify bundle.split-macro]
        // ss[verify verify.process.proptest]
        fn proptest_split_bundle_one_plus_three(cap in 1usize..8) {
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (t0, _) = builder.build_channel::<u8>();
            let (t1, _) = builder.build_channel::<u8>();
            let (t2, _) = builder.build_channel::<u8>();
            let (t3, _) = builder.build_channel::<u8>();
            let bundle = steady_tx_bundle([t0, t1, t2, t3]);
            let (a, b) = split_bundle!(bundle, 1, 3);
            prop_assert_eq!(a.len(), 1);
            prop_assert_eq!(b.len(), 3);
            prop_assert_eq!(a.len() + b.len(), 4);
        }
    }
}
