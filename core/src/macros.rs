use crate::{LazySteadyRx, LazySteadyRxBundle, LazySteadyTx, LazySteadyTxBundle, SteadyRx, SteadyTx, SteadyActor};
use crate::simulate_edge::IntoSimRunner;
use crate::distributed::aqueduct_stream::{SteadyStreamRx, SteadyStreamTx, StreamControlItem};
use async_ringbuf::Arc;
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;

/// Trait to allow uniform flattening of channels and bundles into simulation runners.
pub trait SimIndexable<C: SteadyActor + 'static> {
    /// Pushes references to simulation runners into the provided vector.
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>);
}

impl<T, C> SimIndexable<C> for SteadyRx<T> 
    where SteadyRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

impl<T, C> SimIndexable<C> for SteadyTx<T> 
    where SteadyTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

impl<T, C> SimIndexable<C> for SteadyStreamRx<T> 
    where SteadyStreamRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

impl<T, C> SimIndexable<C> for SteadyStreamTx<T> 
    where SteadyStreamTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) { vec.push(self); }
}

impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyRx<T>; N]>
    where SteadyRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyTx<T>; N]>
    where SteadyTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyStreamRx<T>; N]>
    where SteadyStreamRx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

impl<T, C, const N: usize> SimIndexable<C> for Arc<[SteadyStreamTx<T>; N]>
    where SteadyStreamTx<T>: IntoSimRunner<C>, C: SteadyActor + 'static, T: StreamControlItem {
    fn push_to<'a>(&'a self, vec: &mut Vec<&'a dyn IntoSimRunner<C>>) {
        for item in self.iter() { vec.push(item); }
    }
}

/// Trait to allow uniform indexing into metadata collections (bundles or single channels).
/// This is used by the `rx_meta_data!` and `tx_meta_data!` macros to handle mixed types.
pub trait MetaIndexable<T: ?Sized> {
    /// Returns a reference to the metadata at the specified index.
    fn meta_at(&self, index: usize) -> &T;
    /// Returns the number of metadata items in the collection.
    fn meta_len(&self) -> usize;
}

// Implementation for bundles (Arc of array of providers)
impl<T: RxMetaDataProvider + 'static, const N: usize> MetaIndexable<dyn RxMetaDataProvider> for Arc<[T; N]> {
    fn meta_at(&self, index: usize) -> &(dyn RxMetaDataProvider + 'static) { &self[index] }
    fn meta_len(&self) -> usize { N }
}

impl<T: TxMetaDataProvider + 'static, const N: usize> MetaIndexable<dyn TxMetaDataProvider> for Arc<[T; N]> {
    fn meta_at(&self, index: usize) -> &(dyn TxMetaDataProvider + 'static) { &self[index] }
    fn meta_len(&self) -> usize { N }
}

// Implementation for single channels or already-extracted metadata
impl<M: RxMetaDataProvider + 'static> MetaIndexable<dyn RxMetaDataProvider> for M {
    fn meta_at(&self, _index: usize) -> &(dyn RxMetaDataProvider + 'static) { self }
    fn meta_len(&self) -> usize { 1 }
}

impl<M: TxMetaDataProvider + 'static> MetaIndexable<dyn TxMetaDataProvider> for M {
    fn meta_at(&self, _index: usize) -> &(dyn TxMetaDataProvider + 'static) { self }
    fn meta_len(&self) -> usize { 1 }
}

// Implementation for raw arrays of references (output of bundle.meta_data())
impl<'a, T: ?Sized + 'a, const N: usize> MetaIndexable<T> for [&'a T; N] {
    fn meta_at(&self, index: usize) -> &T { self[index] }
    fn meta_len(&self) -> usize { N }
}

/// Internal helper for recursive offset calculation within metadata arrays.
/// This macro navigates through multiple arrays to find the element at a specific global offset.
/// Do not call this directly.
#[macro_export]
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

/// Splits a bundle into multiple parts using constants or literals.
///
/// This version supports constant identifiers (e.g., PDF_F) and ensures
/// that the "Capital Allocation" of channels matches the total supply
/// at compile time.
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
pub fn steady_tx_bundle<T, const GIRTH: usize>(
    inner: [LazySteadyTx<T>; GIRTH]
) -> LazySteadyTxBundle<T, GIRTH> {
    inner
}

/// Helper to convert a raw array of receivers into a SteadyRxBundle.
pub fn steady_rx_bundle<T, const GIRTH: usize>(
    inner: [LazySteadyRx<T>; GIRTH]
) -> LazySteadyRxBundle<T, GIRTH> {
    inner
}
