// ss[related actor.regeneration-survives]

/// A helper struct for managing CPU core allocation to balance actor distribution across available cores.
///
/// `CoreBalancer` tracks the usage of each core and allocates actors to the least utilized cores, respecting any
/// exclusions specified in the `ActorBuilder`.
#[derive(Clone)]
// ss[related actor.regeneration-survives]
pub struct CoreBalancer {
    /// A vector where each element represents the number of actors assigned to that core.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) core_usage: Vec<usize>,
}

// ss[related actor.regeneration-survives]
impl CoreBalancer {
    /// Allocates a core for an actor, choosing the least utilized core that is not excluded.
    ///
    /// # Arguments
    ///
    /// * `excluded_cores` - A slice of core indices to exclude from allocation.
    ///
    /// # Returns
    ///
    /// THE index of the allocated core.
    // ss[related actor.regeneration-survives]
    pub(crate) fn allocate_core(&mut self, excluded_cores: &[usize]) -> usize {
        let core = self
            .core_usage
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded_cores.contains(i))
            .min_by_key(|(_, count)| *count)
            .map(|(core, _)| core)
            .expect("No available cores");
        self.core_usage[core] += 1;
        core
    }
}

/// Retrieves the number of available CPU cores on Unix systems.
///
/// # Returns
///
/// THE number of CPU cores available.
#[cfg(feature = "core_affinity")]
#[cfg(unix)]
// ss[related actor.regeneration-survives]
fn get_num_cores() -> usize {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as usize }
}

/// Pins the current thread to a specific CPU core.
///
/// # Arguments
///
/// * `_core_id` - THE index of the core to pin the thread to.
///
/// # Returns
///
/// A `Result` indicating success or an error message if pinning fails.
#[cfg(feature = "core_affinity")]
// ss[related actor.regeneration-survives]
pub(crate) fn pin_thread_to_core(_core_id: usize) -> Result<(), String> {
    #[cfg(unix)]
    {
        let num_cores = get_num_cores();
        let core_id = _core_id % num_cores;
        let mut cpu_set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::CPU_ZERO(&mut cpu_set);
            libc::CPU_SET(core_id, &mut cpu_set);
            let thread_id = libc::pthread_self();
            let result = libc::pthread_setaffinity_np(
                thread_id,
                std::mem::size_of::<libc::cpu_set_t>(),
                &cpu_set,
            );
            if result != 0 {
                return Err(format!("Failed to set thread affinity: {}", result));
            }
        }
    }
    // #[cfg(windows)]
    // {
    //     unsafe {
    //         let thread = winapi::um::processthreadsapi::GetCurrentThread();
    //         let mask = 1usize << core_id; //TODO: this logic is wrong we need to think
    //         winapi::um::winbase::SetThreadAffinityMask(thread, mask);
    //     }
    // }
    Ok(())
}

/// No-op when `core_affinity` is disabled (CI / default test features).
#[cfg(not(feature = "core_affinity"))]
// ss[related actor.regeneration-survives]
pub(crate) fn pin_thread_to_core(_core_id: usize) -> Result<(), String> {
    Ok(())
}


#[cfg(test)]
// ss[related actor.regeneration-survives]
mod affinity_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use proptest::prelude::*;

    #[test]
    // ss[verify actor.regeneration-survives]
    fn test_core_balancer() {
        let mut cb = CoreBalancer {
            core_usage: vec![0, 0, 0],
        };
        assert_eq!(cb.allocate_core(&[]), 0);
        assert_eq!(cb.allocate_core(&[]), 1);
        assert_eq!(cb.allocate_core(&[0]), 2);
        assert_eq!(cb.allocate_core(&[]), 0);
        assert_eq!(cb.core_usage, vec![2, 1, 1]);
    }

    ss_proptest! {

        /// Property: repeated allocation picks the least-used non-excluded core (round-robin balance).
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_core_balancer_round_robin(
            num_cores in 1usize..8,
            allocations in 1usize..64,
            excluded in prop::collection::vec(0usize..8, 0..4),
        ) {
            let mut cb = CoreBalancer {
                core_usage: vec![0; num_cores],
            };
            let excluded: Vec<usize> = excluded
                .into_iter()
                .filter(|c| *c < num_cores)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            prop_assume!(excluded.len() < num_cores);

            for _ in 0..allocations {
                let before = cb.core_usage.clone();
                let core = cb.allocate_core(&excluded);
                prop_assert!(core < num_cores);
                prop_assert!(!excluded.contains(&core));
                let min_usage = before
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !excluded.contains(i))
                    .map(|(_, c)| *c)
                    .min()
                    .unwrap_or(0);
                prop_assert_eq!(before[core], min_usage);
                prop_assert_eq!(cb.core_usage[core], before[core] + 1);
            }
        }
    }
}
