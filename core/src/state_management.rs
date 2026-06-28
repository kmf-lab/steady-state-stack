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

    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;
    use serde::de::DeserializeOwned;

    fn assert_persistent_json_roundtrip<T>(value: T) -> Result<(), TestCaseError>
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug + Clone + Send + 'static,
    {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let expected = value.clone();
        {
            let state = new_persistent_state::<T, _>(&file_path);
            async_std::task::block_on(async {
                let mut guard = state.lock(|| expected.clone()).await;
                *guard = expected.clone();
            });
        }
        let file = File::open(&file_path).expect("persisted file");
        let reader = BufReader::new(file);
        let loaded: T = serde_json::from_reader(reader).expect("valid json");
        prop_assert_eq!(loaded, expected);
        Ok(())
    }

    ss_proptest! {

        /// Property: persistent state round-trips arbitrary i32 values through JSON on disk.
        #[test]
        // ss[verify state.save-on-drop]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_roundtrip_i32(value: i32) {
            assert_persistent_json_roundtrip(value)?;
        }

        /// Property: persistent state round-trips string payloads.
        #[test]
        // ss[verify state.save-on-drop]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_roundtrip_string(s in "\\PC{0,48}") {
            assert_persistent_json_roundtrip(s)?;
        }

        /// Property: persistent state round-trips struct payloads.
        #[test]
        // ss[verify state.save-on-drop]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_roundtrip_struct(value: i32, label in "\\PC{0,16}") {
            #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
            struct Labeled {
                value: i32,
                label: String,
            }
            assert_persistent_json_roundtrip(Labeled { value, label })?;
        }

        /// Property: persistent state round-trips vector payloads.
        #[test]
        // ss[verify state.save-on-drop]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_roundtrip_vec(
            items in prop::collection::vec(-1_000i32..1_000, 0..16),
        ) {
            assert_persistent_json_roundtrip(items)?;
        }

        /// Property: corrupt on-disk JSON falls back to the init closure.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_invalid_json_fallback(
            init_value in -1_000i32..1_000,
            garbage in "\\PC{1,64}",
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            std::fs::write(&file_path, garbage).expect("write garbage");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: `persist` on non-persistent state is a no-op success.
        #[test]
        // ss[verify state.lock-init-once]
        // ss[verify verify.process.proptest]
        fn proptest_non_persistent_persist_is_noop(value: i32) {
            let state = new_state::<MyState>();
            async_std::task::block_on(async {
                let mut guard = state.lock(|| MyState { value: 0 }).await;
                guard.value = value;
                guard.persist().await.expect("persist noop");
            });
        }

        /// Property: structurally valid JSON with wrong shape still falls back to init.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_wrong_shape_fallback(
            init_value in -500i32..500,
            extra in "\\PC{0,24}",
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = format!(r#"{{"not_value":{init_value},"label":"{extra}"}}"#);
            std::fs::write(&file_path, body).expect("write wrong shape");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: clone shares mutations across handles.
        #[test]
        // ss[verify state.clone-shared]
        // ss[verify verify.process.proptest]
        fn proptest_cloned_state_shares_value(a: i32, b: i32) {
            let state1 = new_state::<i32>();
            async_std::task::block_on(async {
                let _ = state1.lock(|| a).await;
            });
            let state2 = state1.clone();
            async_std::task::block_on(async {
                let mut guard = state2.lock(|| 0).await;
                *guard = b;
            });
            let read = async_std::task::block_on(async { *state1.lock(|| 0).await });
            prop_assert_eq!(read, b);
        }

        /// Property: empty file falls back to init closure.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_empty_file_fallback(init_value in -1_000i32..1_000) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            std::fs::write(&file_path, "").expect("write empty");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: truncated JSON falls back to init closure.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_truncated_json_fallback(
            init_value in -500i32..500,
            prefix in "\\PC{1,32}",
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = format!(r#"{{"value":{init_value},"extra":"{prefix}"#);
            std::fs::write(&file_path, body).expect("write truncated");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: valid on-disk JSON loads without calling init.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_valid_json_loads(
            disk_value in -10_000i32..10_000,
            init_value in -10_000i32..10_000,
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let on_disk = MyState { value: disk_value };
            let file = File::create(&file_path).expect("create");
            serde_json::to_writer(file, &on_disk).expect("write");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, disk_value);
        }

        /// Property: explicit `persist` writes current state to disk.
        #[test]
        // ss[verify state.save-on-drop]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_explicit_persist(
            value in -5_000i32..5_000,
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let state = new_persistent_state::<MyState, _>(&file_path);
            async_std::task::block_on(async {
                let mut guard = state.lock(|| MyState { value: 0 }).await;
                guard.value = value;
                guard.persist().await.expect("persist");
            });
            let file = File::open(&file_path).expect("open");
            let reader = BufReader::new(file);
            let loaded: MyState = serde_json::from_reader(reader).expect("json");
            prop_assert_eq!(loaded.value, value);
        }

        /// Property: `try_lock_sync` returns None before initialization.
        #[test]
        // ss[verify state.try-lock-sync]
        // ss[verify verify.process.proptest]
        fn proptest_try_lock_sync_before_init(_seed in 0u8..2) {
            let state = new_state::<MyState>();
            prop_assert!(state.try_lock_sync().is_none());
        }

        /// Property: JSON with the wrong scalar type falls back to the init closure.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_type_mismatch_fallback(
            init_value in -1_000i32..1_000,
            label in "\\PC{0,16}",
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = format!(r#"{{"value":"{label}"}}"#);
            std::fs::write(&file_path, body).expect("write mismatched type");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: JSON `null` document falls back to the init closure.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_null_fallback(init_value in -1_000i32..1_000) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            std::fs::write(&file_path, "null").expect("write null");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: JSON array document falls back to the init closure.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_json_array_fallback(
            init_value in -500i32..500,
            extra in -500i32..500,
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = format!(r#"[{{"value":{init_value}}},{{"value":{extra}}}]"#);
            std::fs::write(&file_path, body).expect("write array");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: persisting to a directory path returns an I/O error.
        #[test]
        // ss[verify state.save-on-drop]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_persist_directory_fails(value in -500i32..500) {
            let dir = tempdir().expect("tempdir");
            let state = new_persistent_state::<MyState, _>(dir.path());
            let err = async_std::task::block_on(async {
                let mut guard = state.lock(|| MyState { value: 0 }).await;
                guard.value = value;
                guard.persist().await
            });
            prop_assert!(err.is_err());
        }

        /// Property: JSON missing required fields falls back to init.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_missing_field_fallback(init_value in -1_000i32..1_000) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            std::fs::write(&file_path, r#"{}"#).expect("write empty object");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: JSON boolean where integer expected falls back to init.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_boolean_type_fallback(init_value in -500i32..500) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            std::fs::write(&file_path, r#"{"value":true}"#).expect("write bool");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: JSON float where integer expected falls back to init.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_float_type_fallback(
            init_value in -500i32..500,
            fractional in 0.1f64..99.9,
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = format!(r#"{{"value":{fractional}}}"#);
            std::fs::write(&file_path, body).expect("write float");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: JSON numeric string where integer expected falls back to init.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_numeric_string_fallback(
            init_value in -1_000i32..1_000,
            disk_digits in "\\d{1,6}",
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = format!(r#"{{"value":"{disk_digits}"}}"#);
            std::fs::write(&file_path, body).expect("write numeric string");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: init_value }).await;
                guard.value
            });
            prop_assert_eq!(got, init_value);
        }

        /// Property: JSON with extra unknown fields still loads valid `MyState` when shape matches.
        #[test]
        // ss[verify state.persistent-load]
        // ss[verify verify.process.proptest]
        fn proptest_persistent_state_extra_fields_ignored(
            disk_value in -10_000i32..10_000,
            extra in "\\PC{0,24}",
        ) {
            let dir = tempdir().expect("tempdir");
            let file_path = dir.path().join("state.json");
            let body = serde_json::json!({
                "value": disk_value,
                "extra": extra,
            })
            .to_string();
            std::fs::write(&file_path, body).expect("write extra fields");
            let state = new_persistent_state::<MyState, _>(&file_path);
            let got = async_std::task::block_on(async {
                let guard = state.lock(|| MyState { value: 0 }).await;
                guard.value
            });
            prop_assert_eq!(got, disk_value);
        }
    }
}
