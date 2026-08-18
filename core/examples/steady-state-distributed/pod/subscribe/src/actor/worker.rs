use steady_state::*;
use std::error::Error;
use std::mem::MaybeUninit;

#[cfg(test)]
const VALUES_PER_HEARTBEAT: usize = 16;
#[cfg(not(test))]
const VALUES_PER_HEARTBEAT: usize = 1_000_000;

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum FizzBuzzMessage {
    #[default]
    FizzBuzz = 15,
    Fizz = 3,
    Buzz = 5,
    Value(u64),
}

impl FizzBuzzMessage {
    pub fn new(value: u64) -> Self {
        match value % 15 {
            0 => FizzBuzzMessage::FizzBuzz,
            3 | 6 | 9 | 12 => FizzBuzzMessage::Fizz,
            5 | 10 => FizzBuzzMessage::Buzz,
            _ => FizzBuzzMessage::Value(value),
        }
    }
}

pub(crate) struct WorkerState {
    pub(crate) heartbeats_processed: u64,
    pub(crate) values_processed: u64,
    pub(crate) audit_position: u64,
}

pub async fn run(
    actor: SteadyActorShadow,
    heartbeat: SteadyRx<u64>,
    generator: SteadyRx<u64>,
    logger: SteadyTx<FizzBuzzMessage>,
    state: SteadyState<WorkerState>,
) -> Result<(), Box<dyn Error>> {
    internal_behavior(
        actor.into_spotlight([&heartbeat, &generator], [&logger]),
        heartbeat,
        generator,
        logger,
        state,
    )
        .await
}

async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    heartbeat: SteadyRx<u64>,
    generator: SteadyRx<u64>,
    logger: SteadyTx<FizzBuzzMessage>,
    state: SteadyState<WorkerState>,
) -> Result<(), Box<dyn Error>> {
    let mut heartbeat = heartbeat.acquire_guard().await;
    let mut generator = generator.acquire_guard().await;
    let mut logger = logger.acquire_guard().await;

    let mut state = state
        .acquire_guard(|| WorkerState {
            heartbeats_processed: 0,
            values_processed: 0,
            audit_position: 0,
        })
        .await;

    //you may find you want different timeouts for testing or release
    let max_latency = if cfg!(test) {
        Duration::from_secs(3)
    } else {
        Duration::from_secs(3600)
    };

    while actor.is_running(|| {
        heartbeat.is_closed_and_empty()
            && generator.is_closed_and_empty()
            && logger.mark_closed()
    }) {
        // Wait for a heartbeat, enough input, and enough output space
        let is_clean = await_for_all_or_proceed_upon!(
            actor.wait_periodic(max_latency),
            actor.wait_avail(&mut heartbeat, 1),
            actor.wait_avail(&mut generator, VALUES_PER_HEARTBEAT),
            actor.wait_vacant(&mut logger, VALUES_PER_HEARTBEAT)
        );

        if !is_clean {
            continue;
        }

        let _ = actor
            .try_take(&mut heartbeat)
            .expect("heartbeat should be available when wait succeeded");
        state.heartbeats_processed += 1;

        // Zero-copy: get slices for input and output
        let (peek_a, peek_b) = actor.peek_slice(&mut generator);    //#!#//
        let (poke_a, poke_b) = actor.poke_slice(&mut logger);    //#!#//

        // We need to process exactly VALUES_PER_HEARTBEAT items
        let mut remaining = VALUES_PER_HEARTBEAT;

        // Helper to process a slice pair
        let mut process = |input: &[u64], output: &mut [MaybeUninit<FizzBuzzMessage>]| {
            let n = input.len().min(output.len()).min(remaining);
            for i in 0..n {
                let value = input[i];
                #[cfg(not(test))] //ensure order when we are running all data
                if state.audit_position != value {
                    panic!(
                        "Sequence error: expected {}, got {}",
                        state.audit_position, value
                    );
                }
                state.audit_position += 1;
                output[i].write(FizzBuzzMessage::new(value));
            }
            remaining -= n;
            n
        };

        // Process in order: peek_a→poke_a, peek_a→poke_b, peek_b→poke_a, peek_b→poke_b
        let mut offset_in = 0;
        let mut offset_out = 0;

        // 1. peek_a → poke_a
        let n1 = process(&peek_a[offset_in..], &mut poke_a[offset_out..]);
        offset_in += n1;
        offset_out += n1;

        // 2. peek_a → poke_b
        let n2 = process(&peek_a[offset_in..], &mut poke_b[..]);
        //offset_in += n2;

        // 3. peek_b → poke_a
        let n3 = process(&peek_b, &mut poke_a[offset_out..]);
        //offset_out += n3;

        // 4. peek_b → poke_b
        let n4 = process(&peek_b[n3..], &mut poke_b[n2..]);

        let total_taken = n1 + n2 + n3 + n4;
        if is_clean {
            #[cfg(not(test))]
            assert_eq!(
                total_taken, VALUES_PER_HEARTBEAT,
                "Should process exactly VALUES_PER_HEARTBEAT"
            );
        }
        #[cfg(not(test))]
        if total_taken !=0 && is_clean {
            assert_eq!(VALUES_PER_HEARTBEAT, total_taken);
        }
        // Advance channel indices
        assert_eq!(
            total_taken,
            actor
                .advance_send_index(&mut logger, total_taken)    //#!#//
                .item_count(),
            "move write position"
        );
        assert_eq!(
            total_taken,
            actor
                .advance_take_index(&mut generator, total_taken)    //#!#//
                .item_count(),
            "move read position"
        );

        state.values_processed += total_taken as u64;

        if 0 == (state.heartbeats_processed & ((1 << 26) - 1)) {
            trace!(
                "Worker: {} heartbeats processed",
                state.heartbeats_processed
            );
        }
    }

    info!(
        "Worker shutting down. Heartbeats: {}, Values: {}",
        state.heartbeats_processed,
        state.values_processed
    );
    Ok(())
}