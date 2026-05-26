// ss[related state.lock-init-once]
use std::sync::Arc;
use futures_util::lock::{Mutex, MutexGuard, MappedMutexGuard};
use std::ops::{Deref, DerefMut};
// ss[related state.lock-init-once]
use std::fs::File;
use std::io::{BufReader, Error};
use std::path::{Path, PathBuf};
// ss[related state.lock-init-once]
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
// ss[related state.lock-init-once]
pub struct SteadyState<S> {
    inner: Arc<Mutex<Option<S>>>,
    on_drop: Option<Arc<dyn Fn(&S) + Send + Sync>>,
    on_persist: Option<Arc<dyn Fn(&S) -> Result<(), std::io::Error> + Send + Sync>>,
}

// ss[impl state.clone-shared]
impl<S> Clone for SteadyState<S> {

    /// Creates a new reference to the same underlying state.
    ///
    /// This method clones the `Arc`, allowing multiple references to the same state.
    // ss[related state.lock-init-once]
    fn clone(&self) -> Self {
        SteadyState {
            inner: self.inner.clone(),
            on_drop: self.on_drop.clone(),
            on_persist: self.on_persist.clone(),
        }
    }
}

// ss[related state.lock-init-once]
impl<S> Default for SteadyState<S> {
    /// new simple state creation
    fn default() -> Self {
        new_state()
    }
}

// ss[related state.lock-init-once]
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
    // ss[related state.lock-init-once]
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
            on_persist: self.on_persist.clone(),
        }
    }

    /// Lock state to review or modify its values after it has been created or initialized.
    /// This is most helpful in testing and in main after actors have shutdown to determine what
    /// was the final state of the SteadyState.
    // ss[impl state.try-lock-sync]
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
                    on_persist: self.on_persist.clone(),
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
// ss[impl state.steady-state-persistence]
// ss[impl state.lock-init-once]
pub fn new_state<S>() -> SteadyState<S> {
    SteadyState {
        inner: Arc::new(Mutex::new(None)),
        on_drop: None,
        on_persist: None,

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
// ss[impl state.save-on-drop]
// ss[impl state.persistent-load]
pub fn new_persistent_state<S, P>(file_path: P) -> SteadyState<S>
where
    P: AsRef<Path>,
    S: Serialize + DeserializeOwned + Send + 'static,
{
    let file_path: PathBuf = file_path.as_ref().to_path_buf();


    let state = File::open(&file_path)
        .ok()
        .and_then(|file| {
            let reader = BufReader::new(file);
            serde_json::from_reader(reader).ok()
        });

    let drop_file = file_path.clone();
    // ss[impl state.on-drop-hook]
    let on_drop = move |s: &S| {
        let _ = write_file(&drop_file, s);
    };
    let persist_file = file_path.clone();
    let on_persist = move |s: &S| {
        write_file(&persist_file, s)
    };

    SteadyState {
        inner: Arc::new(Mutex::new(state)),
        on_drop: Some(Arc::new(on_drop)),
        on_persist: Some(Arc::new(on_persist)),
    }
}

// ss[related state.lock-init-once]
fn write_file<S: serde::Serialize>(file_path: &PathBuf, s: &S) -> Result<(), std::io::Error> {
    if let Ok(file) = File::create(&file_path) {
        let result = serde_json::to_writer_pretty(file, s);
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::from(e)),
        }
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to create file"))
    }
}

///
/// Protect state access while the actor needs to use it. State reverts to lock when dropped.
// ss[related state.lock-init-once]
pub struct StateGuard<'a, S> {
    guard: MappedMutexGuard<'a, Option<S>, S>,
    on_drop: Option<Arc<dyn Fn(&S) + Send + Sync>>,
    on_persist: Option<Arc<dyn Fn(&S) -> Result<(), std::io::Error> + Send + Sync>>,
}

// ss[related state.lock-init-once]
impl<'a, S> Deref for StateGuard<'a, S> {
    type Target = S;

    // ss[related state.lock-init-once]
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

// ss[related state.lock-init-once]
impl<'a, S> DerefMut for StateGuard<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

// ss[impl state.on-drop-hook]
impl<'a, S> Drop for StateGuard<'a, S> {
    fn drop(&mut self) {
        if let Some(on_drop) = &self.on_drop {
            on_drop(&*self.guard);
        }
    }
}

// ss[related state.lock-init-once]
impl<'a, S> StateGuard<'a, S> {

    /// Persists the current state to disk if a persistence function was provided.
    ///
    /// # Returns
    /// - `Result<(), std::io::Error>`: The result of the persistence operation.
    // ss[related state.lock-init-once]
    pub async fn persist(&self) -> Result<(), std::io::Error>
    where
        S: Serialize,
    {
        if let Some(on_persist) = &self.on_persist {
            on_persist(&*self.guard)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
// ss[related state.lock-init-once]
mod state_management_tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    // ss[related state.lock-init-once]
    use std::fs::File;
    use std::io::BufReader;
    use tempfile::tempdir;

    // Define a simple state type for testing persistence
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    // ss[related state.lock-init-once]
    struct MyState {
        value: i32,
    }

    // ss[verify state.lock-init-once]
    // ss[verify state.steady-state-persistence]
    // ss[verify state.try-lock-sync]
    #[async_std::test]
    async fn test_basic_state() {
        let state = new_state::<i32>();
        // Test that try_lock_sync fails before initialization
        assert!(state.try_lock_sync().is_none());
        {
            let guard = state.lock(|| 42).await;
            assert_eq!(*guard, 42);
        }
        {
            let guard = state.try_lock_sync().unwrap();
            assert_eq!(*guard, 42);
        }
    }

    // ss[verify state.clone-shared]
    #[async_std::test]
    async fn test_cloning_shared_state() {
        let state1 = new_state::<i32>();
        {
            let guard = state1.lock(|| 10).await;
            assert_eq!(*guard, 10);
        }
        let state2 = state1.clone();
        {
            let mut guard = state2.lock(|| 0).await; // init closure shouldn't run
            *guard = 20;
        }
        {
            let guard = state1.lock(|| 0).await;
            assert_eq!(*guard, 20);
        }
    }

    // ss[verify state.persistent-load]
    #[async_std::test]
    async fn test_persistent_state_load() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("state.json");
        let initial_state = MyState { value: 100 };
        let file = File::create(&file_path).unwrap();
        serde_json::to_writer(file, &initial_state).unwrap();

        let state = new_persistent_state::<MyState, _>(&file_path);
        {
            let guard = state.lock(|| MyState { value: 0 }).await;
            assert_eq!(*guard, MyState { value: 100 });
        }
    }

    // ss[verify state.save-on-drop]
    // ss[verify state.on-drop-hook]
    #[async_std::test]
    async fn test_persistent_state_save() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("state.json");

        let state = new_persistent_state::<MyState, _>(&file_path);
        {
            let mut guard = state.lock(|| MyState { value: 0 }).await;
            guard.value = 200;
        } // Guard dropped here, should save to file

        let file = File::open(&file_path).unwrap();
        let reader = BufReader::new(file);
        let saved_state: MyState = serde_json::from_reader(reader).unwrap();
        assert_eq!(saved_state, MyState { value: 200 });
    }

    // ss[verify state.persistent-load]
    #[async_std::test]
    async fn test_persistent_state_no_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("nonexistent.json");

        let state = new_persistent_state::<MyState, _>(&file_path);
        {
            let guard = state.lock(|| MyState { value: 50 }).await;
            assert_eq!(*guard, MyState { value: 50 });
        }
    }

    // ss[verify state.persistent-load]
    #[async_std::test]
    async fn test_persistent_state_invalid_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("invalid.json");
        std::fs::write(&file_path, "invalid json").unwrap();

        let state = new_persistent_state::<MyState, _>(&file_path);
        {
            let guard = state.lock(|| MyState { value: 75 }).await;
            assert_eq!(*guard, MyState { value: 75 });
        }
    }
}
