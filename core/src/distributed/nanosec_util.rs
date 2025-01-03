
#[cfg(any(
    feature = "aeron_driver_systemd",
    feature = "aeron_driver_sidecar",
    feature = "aeron_driver_external"
))]
pub(crate) fn precise_sleep(nanos: i64) { //Add 100 microseconds (100_000 nanoseconds)
    use libc::{clock_gettime, clock_nanosleep, timespec, CLOCK_MONOTONIC, TIMER_ABSTIME};

    let mut next_tick = timespec { //libc::unix::timespec
        tv_sec: 0,  // time_t in rust is i64
        tv_nsec: 0, // c_long in Rust matches the C long type (i64 on most 64-bit platforms, i32 on most 32-bit platforms)
    };

    unsafe {
        clock_gettime(CLOCK_MONOTONIC, &mut next_tick);
    }

    next_tick.tv_nsec += nanos;
    if next_tick.tv_nsec >= 1_000_000_000 {
        next_tick.tv_nsec -= 1_000_000_000;
        next_tick.tv_sec += 1;
    }

    unsafe {
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next_tick, std::ptr::null_mut());
    }
}


/*
pub struct NanosecondTickScheduler {
    interval_ns: i64,
    next_tick: timespec,
}

impl NanosecondTickScheduler {
    /// Create a new `NanosecondTickScheduler` with a specified interval.
    pub fn new(interval: Duration) -> Self {
        let interval_ns = interval.as_nanos() as i64;
        let mut next_tick = timespec { tv_sec: 0, tv_nsec: 0 };

        unsafe {
            clock_gettime(CLOCK_MONOTONIC, &mut next_tick);
        }

        Self { interval_ns, next_tick }
    }

    /// Pin the current thread to a specific core (e.g., core 0 by default).
    pub fn pin_to_core(core_index: usize) {
        let mut cpuset = libc::cpu_set_t { __bits: [0; 16] };
        unsafe {
            libc::CPU_ZERO(&mut cpuset);
            libc::CPU_SET(core_index, &mut cpuset);
            libc::sched_setaffinity(0, std::mem::size_of_val(&cpuset), &cpuset);
        }

    /// Wait for the next tick. This method allows custom work to be done between ticks.
    pub fn wait_for_next_tick(&mut self) {
        // Calculate the next tick
        self.next_tick.tv_nsec += self.interval_ns;

        // Normalize the timespec if nanoseconds overflow
        if self.next_tick.tv_nsec >= 1_000_000_000 {
            self.next_tick.tv_nsec -= 1_000_000_000;
            self.next_tick.tv_sec += 1;
        }
        //TODO: need higher priority for this thread to ensure we can swap qickly on this event.
        // Sleep until the next tick
        unsafe {
            let res = clock_nanosleep(CLOCK_MONOTONIC, 1, &self.next_tick, std::ptr::null_mut());
            if res != 0 {
                eprintln!("clock_nanosleep error: {}", res);
                sched_yield(); // Yield the CPU in case of an error
            }
        }
    }


}

fn main() {
    // Example usage of `NanosecondTickScheduler`

    // Pin the thread to a specific core (optional, but can improve timing consistency)
    NanosecondTickScheduler::pin_to_core(0);

    // Create a scheduler with a 10ms interval
    let mut scheduler = NanosecondTickScheduler::new(Duration::from_millis(10));

    // Example loop with custom work and waiting for the next tick
    loop {
        // Perform the work for this tick
        println!("Doing work at {:?}", std::time::Instant::now());

        // Wait for the next tick
        scheduler.wait_for_next_tick();
    }
}
//  */