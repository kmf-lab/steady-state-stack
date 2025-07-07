use std::sync::Arc;
use futures_util::lock::{Mutex, MutexGuard, MappedMutexGuard};
use std::ops::{Deref, DerefMut};
use std::fs::File;
use serde::{Serialize};
use serde::de::DeserializeOwned;
use serde_json;

/// A thread-safe wrapper for actor state, preserved across restarts.
///
/// The `SteadyState` struct encapsulates an actor's state within an `Arc<Mutex<Option<S>>>`, ensuring thread safety
/// and persistence across restarts or optionally to disk.
///
/// # Type Parameters
/// - `S`: The type of the state being stored.
pub struct SteadyState<S> {
    inner: Arc<Mutex<Option<S>>>,
    on_drop: Option<Arc<dyn Fn(&S) + Send + Sync>>,
}

impl<S> Clone for SteadyState<S> {
    /// Creates a new reference to the same underlying state.
    ///
    /// This method clones the `Arc`, allowing multiple references to the same state.
    fn clone(&self) -> Self {
        SteadyState {
            inner: self.inner.clone(),
            on_drop: self.on_drop.clone(),
        }
    }
}

impl<S> SteadyState<S> {
    /// Asynchronously locks the state, initializing it if absent.
    ///
    /// If the state is `None`, the provided `init` closure is called to create the initial state.
    ///
    /// # Parameters
    /// - `init`: A closure that produces the initial state if it doesn’t exist.
    ///
    /// # Returns
    /// - `StateGuard<'_, S>`: A guard providing mutable access to the state.
    ///
    /// # Type Constraints
    /// - `F: FnOnce() -> S`: The initialization function must produce a value of type `S`.
    /// - `S: Send`: The state must be sendable across threads.
    pub async fn lock<F>(&self, init: F) -> StateGuard<'_, S>
    where
        F: FnOnce() -> S,
        S: Send,
    {
        let mut guard = self.inner.lock().await;
        guard.get_or_insert_with(init);
        let mapped = MutexGuard::map(guard, |opt| opt.as_mut().expect("existing state"));
        StateGuard {
            guard: mapped,
            on_drop: self.on_drop.clone(),
        }
    }

    /// Lock state to review or modify its values after it has been created or initialized.
    /// This is most helpful in testing and in main after actors have shutdown to determine what
    /// was the final state of the SteadyState.
    pub fn try_lock_sync(&self) -> Option<StateGuard<'_, S>>
    where
        S: Send,
    {
        if let Some(guard) = self.inner.try_lock() {
            if let Some(ref _s) = *guard {
                let mapped = MutexGuard::map(guard, |opt| opt.as_mut().expect("existing state"));
                Some(StateGuard {
                    guard: mapped,
                    on_drop: self.on_drop.clone(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// Creates a new `SteadyState` for holding actor state across restarts.
///
/// This function initializes a new `SteadyState` with no initial value, which can be set later via the `lock` method.
///
/// # Type Parameters
/// - `S`: The type of the state to be stored.
///
/// # Returns
/// - `SteadyState<S>`: A new, empty state wrapper.
///
/// # Remarks
/// Should typically be called in `main` when setting up actors.
pub fn new_state<S>() -> SteadyState<S> {
    SteadyState {
        inner: Arc::new(Mutex::new(None)),
        on_drop: None,
    }
}

/// Creates a new `SteadyState` with persistent state stored on disk.
///
/// This function initializes a `SteadyState` that loads its initial state from the specified file path if it exists,
/// and saves the state to that file whenever the guard is dropped.
///
/// # Parameters
/// - `file_path`: The path to the file where the state will be persisted.
///
/// # Type Parameters
/// - `S`: The type of the state, which must implement `Serialize`, `DeserializeOwned`, `Send`, and have a static lifetime.
///
/// # Returns
/// - `SteadyState<S>`: A state wrapper with persistence enabled.
pub fn new_persistent_state<S>(file_path: String) -> SteadyState<S>
where
    S: Serialize + DeserializeOwned + Send + 'static,
{
    let state = File::open(&file_path)
        .ok()
        .and_then(|file| serde_json::from_reader(file).ok());
    let on_drop = move |s: &S| {
        if let Ok(file) = File::create(&file_path) {
            let _ = serde_json::to_writer(file, s);
        }
    };
    SteadyState {
        inner: Arc::new(Mutex::new(state)),
        on_drop: Some(Arc::new(on_drop)),
    }
}

///
/// Protect state access while the actor needs to use it. State reverts to lock when dropped.
pub struct StateGuard<'a, S> {
    guard: MappedMutexGuard<'a, Option<S>, S>,
    on_drop: Option<Arc<dyn Fn(&S) + Send + Sync>>,
}

impl<'a, S> Deref for StateGuard<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<'a, S> DerefMut for StateGuard<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl<'a, S> Drop for StateGuard<'a, S> {
    fn drop(&mut self) {
        if let Some(on_drop) = &self.on_drop {
            on_drop(&*self.guard);
        }
    }
}