//! OS thread spawning abstraction for Steady State actors.
//!
//! This module provides a runtime-agnostic way to spawn OS threads for actors,
//! ensuring consistent behavior across different executor backends
//! (async-std, nuclei, tokio, etc.). It allows actors to run on dedicated
//! OS threads while maintaining compatibility with the existing actor model.

use std::sync::Arc;
use std::io::Result;

/// Function type for spawning OS threads.
///
/// This type is used to allow customization of the thread spawning behavior,
/// such as for testing or custom scheduling.
pub type OsThreadSpawnFn = dyn Fn(String, Box<dyn FnOnce() + Send>) -> Result<()> + Send + Sync;

/// Global override for OS thread spawning (e.g., for testing or custom schedulers).
static mut SPAWN_FN: Option<Arc<OsThreadSpawnFn>> = None;

/// Sets a custom OS thread spawn function (e.g., for simulation or monitoring).
///
/// This allows replacing the default thread spawning behavior, which can be useful
/// for testing, custom scheduling, or sandboxing.
///
/// # Arguments
///
/// * `f` - The custom spawn function to use
///
/// # Safety
///
/// This function is unsafe because it modifies a global static variable.
/// It should only be called during initialization, before any threads are spawned.
pub unsafe fn set_spawn_fn(f: Arc<OsThreadSpawnFn>) {
    SPAWN_FN = Some(f);
}

/// Spawns an OS thread with the given name and closure.
///
/// Uses the global override if set; otherwise falls back to `std::thread::Builder`.
/// This ensures consistent thread spawning behavior across all executor backends.
///
/// # Arguments
///
/// * `name` - The name to give the spawned thread
/// * `f` - The closure to run in the spawned thread
///
/// # Returns
///
/// * `Ok(())` if the thread was successfully spawned
/// * `Err` if thread spawning failed
///
/// # Examples
///
/// ```
/// use std::sync::mpsc::channel;
///
/// let (tx, rx) = channel();
/// steady_state::os_thread::spawn("test-thread", Box::new(move || {
///     tx.send(42).unwrap();
/// }));
/// assert_eq!(rx.recv().unwrap(), 42);
/// ```
pub fn spawn(name: &str, f: Box<dyn FnOnce() + Send>) -> Result<()> {
    unsafe {
        if let Some(ref spawn_fn) = SPAWN_FN {
            return spawn_fn(name.to_string(), f);
        }
    }

    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(|| f())?
        .join()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "thread panicked"))?;

    Ok(())
}
