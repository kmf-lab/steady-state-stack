//! ChannelBuilder `build_*` methods live here so channel_builder.rs stays under the 1,200-line budget.
//! Same inherent impl; callers keep using `ChannelBuilder::build` etc. unchanged.

use super::*;

impl ChannelBuilder {

/**
 * Creates a bundle of channels with the specified number of channels (girth).
 *
 * Consumes the builder to produce a bundle of lazily initialized transmitter and receiver pairs,
 * configured according to the builder’s settings. Resources are allocated only upon first use.
 *
 * # Type Parameters
 *
 * - `T`: Type of data to transmit through the channels.
 * - `GIRTH`: Number of channels in the bundle, specified as a constant.
 *
 * # Returns
 *
 * a tuple of `LazySteadyTxBundle<T, GIRTH>` and `LazySteadyRxBundle<T, GIRTH>` representing the transmitter and receiver bundles.
 *
 * # Panics
 *
 * Panics with an "Internal error, incorrect length" message if the bundle size does not match `GIRTH`.
 */
// ss[impl bundle.girth-const-generic]
pub fn build_channel_bundle<T, const GIRTH: usize>(&self) -> (LazySteadyTxBundle<T, GIRTH>, LazySteadyRxBundle<T, GIRTH>) {
    let mut tx_vec = Vec::with_capacity(GIRTH);
    let mut rx_vec = Vec::with_capacity(GIRTH);

    (0..GIRTH).for_each(|i| {
        let mut indexed_builder = self.clone();
        indexed_builder.bundle_index = Some(i);
        indexed_builder.girth = GIRTH;
        let (t, r) = indexed_builder.build_channel();
        tx_vec.push(t);
        rx_vec.push(r);
    });

    (
        {
            match tx_vec.try_into() {
                Ok(t) => t,
                Err(_) => panic!("Internal error, incorrect length")
            }
        },
        {
            match rx_vec.try_into() {
                Ok(t) => t,
                Err(_) => panic!("Internal error, incorrect length")
            }
        }
        ,
    )
}

/**
 * Creates a bundle of stream channels with the specified number of channels (girth).
 *
 * Similar to `build_channel_bundle`, but tailored for stream channels handling data in a streaming fashion.
 * Channels are lazily initialized, with resources allocated only upon first use.
 *
 * # Type Parameters
 *
 * - `T`: Type of data to transmit, must implement `StreamControlItem`.
 * - `GIRTH`: Number of stream channels in the bundle, specified as a constant.
 *
 * # Arguments
 *
 * - `bytes_per_item`: Number of bytes per item, used to calculate payload channel capacity.
 *
 * # Returns
 *
 * a tuple of `LazySteadyStreamTxBundle<T, GIRTH>` and `LazySteadyStreamRxBundle<T, GIRTH>` representing the transmitter and receiver bundles.
 *
 * # Panics
 *
 * Panics with an "Internal error, incorrect length" message if the bundle size does not match `GIRTH`.
 */
// ss[related channel.lazy.defer-allocation]
pub fn build_stream_bundle<T: StreamControlItem, const GIRTH: usize>(&self
                                                                     , bytes_per_item: usize
) -> (LazySteadyStreamTxBundle<T, GIRTH>, LazySteadyStreamRxBundle<T, GIRTH>) {
    let mut tx_vec = Vec::with_capacity(GIRTH); //pre-allocate, we know the size now
    let mut rx_vec = Vec::with_capacity(GIRTH); //pre-allocate, we know the size now

    let payload_channel_builder = &self.with_capacity(self.capacity*bytes_per_item);
    (0..GIRTH).for_each(|i| { //TODO: later add custom builders for items vs payload
        let mut indexed_builder = self.clone();
        indexed_builder.bundle_index = Some(i);
        indexed_builder.girth = GIRTH;
        let mut indexed_payload_builder = payload_channel_builder.clone();
        indexed_payload_builder.bundle_index = Some(i);
        indexed_payload_builder.girth = GIRTH;

        let lazy = Arc::new(LazyStream::new(&indexed_builder, &indexed_payload_builder));
        tx_vec.push(LazyStreamTx::<T>::new(lazy.clone()));
        rx_vec.push(LazyStreamRx::<T>::new(lazy.clone()));
    });

    (
        match tx_vec.try_into() {
            Ok(t) => t,
            Err(_) => panic!("Internal error, incorrect length")
        }
        ,
        match rx_vec.try_into() {
            Ok(t) => t,
            Err(_) => panic!("Internal error, incorrect length")
        }
        ,
    )
}

/**
 * Creates a single stream channel with the specified configuration.
 *
 * Produces a lazily initialized stream channel for handling data streams, with capacity adjusted based on item size.
 *
 * # Type Parameters
 *
 * - `T`: Type of data to transmit, must implement `StreamControlItem`.
 *
 * # Arguments
 *
 * - `bytes_per_item`: Number of bytes per item, used to calculate payload channel capacity.
 *
 * # Returns
 *
 * a tuple of `LazyStreamTx<T>` and `LazyStreamRx<T>` representing the transmitter and receiver.
 */
// ss[impl channel.stream-dual-buffer]
pub fn build_stream<T: StreamControlItem>(&self, bytes_per_item: usize) -> (LazyStreamTx<T>, LazyStreamRx<T>) {
    let bytes_capacity = self.capacity*bytes_per_item;
    let lazy_stream = Arc::new(LazyStream::new(self
                                               , &self.with_capacity(bytes_capacity)));
    (LazyStreamTx::<T>::new(lazy_stream.clone()), LazyStreamRx::<T>::new(lazy_stream.clone()))
}

/**
 * Creates a single channel with lazy initialization.
 *
 * Returns lazy wrappers for the transmitter and receiver, deferring resource allocation until first use.
 * Preferred over `eager_build` for resource efficiency in actor systems.
 *
 * # Type Parameters
 *
 * - `T`: Type of data to transmit through the channel.
 *
 * # Returns
 *
 * a tuple of `LazySteadyTx<T>` and `LazySteadyRx<T>` representing the transmitter and receiver.
 */
// ss[impl philosophy.lazy-to-established]
// ss[impl channel.lazy.defer-allocation]
pub fn build_channel<T>(&self) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
    let lazy_channel = Arc::new(LazyChannel::new(self));
    (LazySteadyTx::<T>::new(lazy_channel.clone()), LazySteadyRx::<T>::new(lazy_channel.clone()))
}

/**
 * Alias for `build_channel`, providing a simpler method name.
 *
 * # Type Parameters
 *
 * - `T`: Type of data to transmit through the channel.
 *
 * # Returns
 *
 * a tuple of `LazySteadyTx<T>` and `LazySteadyRx<T>` representing the transmitter and receiver.
 */
// ss[related channel.lazy.defer-allocation]
pub fn build<T>(&self) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
    let lazy_channel = Arc::new(LazyChannel::new(self));
    (LazySteadyTx::<T>::new(lazy_channel.clone()), LazySteadyRx::<T>::new(lazy_channel.clone()))
}
}
