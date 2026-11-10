// ss[related state.lock-init-once]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::{Mutex, MutexGuard, MappedMutexGuard};
// ss[related philosophy.structural-hierarchy]
use std::ops::{Deref, DerefMut};
// ss[related state.lock-init-once]
use std::fs::File;
// ss[related philosophy.structural-hierarchy]
use std::io::{BufReader, Error};
// ss[related philosophy.structural-hierarchy]
use std::path::{Path, PathBuf};
// ss[related state.lock-init-once]
use serde::{Serialize};
// ss[related philosophy.structural-hierarchy]
use serde::de::DeserializeOwned;
// ss[related philosophy.structural-hierarchy]
use serde_json;
// ss[impl state.dirty-at-park]
use std::cell::RefCell;
// ss[impl state.dirty-at-park]
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    /// When true, dirty watched guards at park are a defect (`for_lambda` / strict_persist).
    // ss[impl state.dirty-at-park]
    static STRICT_PERSIST: RefCell<bool> = const { RefCell::new(false) };
    /// Dirty flags for persistent guards held on this OS thread.
    // ss[impl state.dirty-at-park]
    static WATCHED_DIRTY: RefCell<Vec<Arc<AtomicBool>>> = const { RefCell::new(Vec::new()) };
}

/// Enable or disable dirty-at-park checks on this OS thread (set by actors / tests).
// ss[impl state.dirty-at-park]
// ss[impl graph.for-lambda]
pub fn set_thread_strict_persist(enabled: bool) {
    STRICT_PERSIST.with(|c| *c.borrow_mut() = enabled);
}

/// Returns whether this OS thread currently enforces dirty-at-park.
// ss[impl state.dirty-at-park]
pub fn thread_strict_persist() -> bool {
    STRICT_PERSIST.with(|c| *c.borrow())
}

fn register_watched_dirty(flag: Arc<AtomicBool>) {
    WATCHED_DIRTY.with(|v| v.borrow_mut().push(flag));
}

fn unregister_watched_dirty(flag: &Arc<AtomicBool>) {
    WATCHED_DIRTY.with(|v| {
        v.borrow_mut().retain(|f| !Arc::ptr_eq(f, flag));
    });
}

/// Fail if any watched persistent guard is dirty while strict persist is active.
///
/// Called from actor wake points (`is_running` / `yield_now` under strict graphs).
// ss[impl state.dirty-at-park]
// ss[impl state.persist-before-park]
pub fn check_dirty_at_park() {
    if !thread_strict_persist() {
        return;
    }
    let dirty = WATCHED_DIRTY.with(|v| {
        v.borrow()
            .iter()
            .any(|f| f.load(Ordering::Relaxed))
    });
    if dirty {
        panic!(
            "dirty persistent state at park: call StateGuard::persist() before awaiting (strict persist / for_lambda)"
        );
    }
}



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
    // ss[related philosophy.structural-hierarchy]
    fn default() -> Self {
        new_state()
    }
}

// ss[related state.lock-init-once]
impl<S> SteadyState<S> {

    /// Asynchronously acquires the state guard, initializing it if absent.
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
        // ss[impl state.dirty-at-park]
        // ss[impl state.persist-before-park]
        let dirty = Arc::new(AtomicBool::new(false));
        let watched = self.on_persist.is_some();
        if watched {
            register_watched_dirty(dirty.clone());
        }
        StateGuard {
            guard: mapped,
            on_drop: self.on_drop.clone(),
            on_persist: self.on_persist.clone(),
            dirty,
            watched,
        }
    }

    /// Guard-first alias for [`SteadyState::lock`] — the preferred spelling.
    ///
    /// Acquires the state guard, initializing it if absent. Identical semantics to
    /// `lock(init)`; only the vocabulary changes. Steady says "acquire the guard",
    /// not "lock", because the guard is held across `.await` for the actor's work —
    /// it is not a mutex critical section.
    // ss[related state.lock-init-once]
    pub async fn acquire_guard<F>(&self, init: F) -> StateGuard<'_, S>
    where
        F: FnOnce() -> S,
        S: Send,
    {
        self.lock(init).await
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
                    dirty: Arc::new(AtomicBool::new(false)),
                    watched: false, // try_lock_sync is post-run inspection; not park-watched
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

/// Creates a `SteadyState` with user-supplied load and persist hooks.
///
/// `load` runs once at construction (optional prior value). `persist` is invoked by
/// [`StateGuard::persist`] and may target DynamoDB / S3 / EFS via blocking I/O in the app.
/// Process-local files (e.g. `/tmp`) MUST NOT be assumed to outlive a Lambda sandbox.
// ss[impl state.persist-hooks]
// ss[impl state.persist-before-park]
pub fn new_persistent_state_with<S, L, P>(load: L, persist: P) -> SteadyState<S>
where
    L: FnOnce() -> Option<S>,
    P: Fn(&S) -> Result<(), std::io::Error> + Send + Sync + 'static,
    S: Send + 'static,
{
    let state = load();
    SteadyState {
        inner: Arc::new(Mutex::new(state)),
        on_drop: None,
        on_persist: Some(Arc::new(persist)),
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
    /// Set on [`DerefMut`]; cleared by successful [`Self::persist`].
    // ss[impl state.dirty-at-park]
    dirty: Arc<AtomicBool>,
    /// True when `on_persist` is set — participates in dirty-at-park under strict mode.
    // ss[impl state.dirty-at-park]
    watched: bool,
}

// ss[related state.lock-init-once]
impl<'a, S> Deref for StateGuard<'a, S> {
    // ss[related philosophy.structural-hierarchy]
    type Target = S;

    // ss[related state.lock-init-once]
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

// ss[impl state.dirty-at-park]
impl<'a, S> DerefMut for StateGuard<'a, S> {
    // ss[related philosophy.structural-hierarchy]
    fn deref_mut(&mut self) -> &mut Self::Target {
        if self.watched {
            self.dirty.store(true, Ordering::Relaxed);
        }
        &mut self.guard
    }
}

// ss[impl state.on-drop-hook]
impl<'a, S> Drop for StateGuard<'a, S> {
    // ss[related philosophy.structural-hierarchy]
    fn drop(&mut self) {
        if self.watched {
            unregister_watched_dirty(&self.dirty);
        }
        if let Some(on_drop) = &self.on_drop {
            on_drop(&*self.guard);
        }
    }
}

// ss[related state.lock-init-once]
impl<'a, S> StateGuard<'a, S> {

    /// Persists the current state via the configured hook and clears the dirty bit.
    ///
    /// Under `for_lambda` / strict persist, call this on every path back to the wait.
    ///
    /// # Returns
    /// - `Result<(), std::io::Error>`: The result of the persistence operation.
    // ss[impl state.persist-before-park]
    // ss[impl state.dirty-at-park]
    pub async fn persist(&self) -> Result<(), std::io::Error> {
        if let Some(on_persist) = &self.on_persist {
            on_persist(&*self.guard)?;
        }
        self.dirty.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Whether this guard is dirty (mutated since last successful persist).
    // ss[impl state.dirty-at-park]
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
}


#[cfg(test)]
// ss[related state.lock-init-once]
#[path = "state_management_tests.rs"]
mod state_management_tests;


#[cfg(test)]
#[path = "state_dirty_proptest.rs"]
// ss[related state.dirty-at-park]
mod state_dirty_proptest;
