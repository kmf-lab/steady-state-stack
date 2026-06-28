use super::deps::*;

/// A guard that provides access to the stage manager for testing purposes.
///
/// This struct holds a lock on the backplane, allowing test code to interact with it safely.
// ss[related graph.for-testing]
pub struct StageManagerGuard<'a> {
    /// THE mutex guard holding the lock on the backplane.
    pub(crate) guard: MutexGuard<'a, Option<StageManager>>,
}

// ss[related graph.for-testing]
impl Deref for StageManagerGuard<'_> {
    type Target = StageManager;

    /// Provides immutable access to the underlying stage manager.
    ///
    /// This allows dereferencing the guard to interact with the stage manager directly.
    // ss[related graph.for-testing]
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("SideChannelHub is not initialized")
    }
}

// ss[related graph.for-testing]
impl StageManagerGuard<'_> {
    /// Releases the lock on the stage manager explicitly.
    ///
    /// This method allows the guard to be dropped manually, freeing the lock for other operations.
    // ss[related graph.for-testing]
    pub fn final_bow(self) {
    }
}
