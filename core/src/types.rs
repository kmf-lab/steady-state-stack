//! Thread-safe channel type aliases and guard bundles.

// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;

// ss[related philosophy.structural-hierarchy]
use futures::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::MutexGuard;

// ss[related philosophy.structural-hierarchy]
use crate::core_rx::RxCore;
// ss[related philosophy.structural-hierarchy]
use crate::core_tx::TxCore;
// ss[related philosophy.structural-hierarchy]
use crate::steady_rx::Rx;
// ss[related philosophy.structural-hierarchy]
use crate::steady_tx::Tx;

/// Type alias for a thread-safe transmitter wrapped in an `Arc` and `Mutex`.
///
/// Simplifies the usage of a transmitter that can be shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
// ss[related philosophy.structural-hierarchy]
pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;

/// Type alias for an array of thread-safe transmitters with a fixed size, wrapped in an `Arc`.
///
/// Simplifies the usage of a bundle of transmitters shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
/// - `GIRTH`: The fixed size of the transmitter array.
// ss[related philosophy.structural-hierarchy]
pub type SteadyTxBundle<T, const GIRTH: usize> = Arc<[SteadyTx<T>; GIRTH]>;

/// Type alias for a thread-safe receiver wrapped in an `Arc` and `Mutex`.
///
/// Simplifies the usage of a receiver that can be shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being received.
// ss[impl channel.internal-behavior-no-lazy]
pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;

/// Type alias for an array of thread-safe receivers with a fixed size, wrapped in an `Arc`.
///
/// Simplifies the usage of a bundle of receivers shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being received.
/// - `GIRTH`: The fixed size of the receiver array.
// ss[related philosophy.structural-hierarchy]
pub type SteadyRxBundle<T, const GIRTH: usize> = Arc<[SteadyRx<T>; GIRTH]>;

/// Type alias for a vector of `MutexGuard` references to transmitters.
///
/// Simplifies batch operations over multiple transmitter guards.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
// ss[related philosophy.structural-hierarchy]
pub type TxBundle<'a, T> = Vec<MutexGuard<'a, Tx<T>>>;

/// Type alias for a vector of `MutexGuard` references to receivers.
///
/// Simplifies batch operations over multiple receiver guards.
///
/// # Type Parameters
/// - `T`: The type of data being received.
// ss[related philosophy.structural-hierarchy]
pub type RxBundle<'a, T> = Vec<MutexGuard<'a, Rx<T>>>;

/// Bundle of `TxCore` guards for batch locking of transmitters.
///
/// Represents a collection of locked transmitter guards for batch operations.
///
/// # Type Parameters
/// - `T`: The type implementing `TxCore`.
#[allow(type_alias_bounds)]
// ss[related philosophy.structural-hierarchy]
pub type TxCoreBundle<'a, T: TxCore> = Vec<MutexGuard<'a, T>>;

/// Bundle of `RxCore` guards for batch locking of receivers.
///
/// Represents a collection of locked receiver guards for batch operations.
///
/// # Type Parameters
/// - `T`: The type implementing `RxCore`.
#[allow(type_alias_bounds)]
// ss[related philosophy.structural-hierarchy]
pub type RxCoreBundle<'a, T: RxCore> = Vec<MutexGuard<'a, T>>;
