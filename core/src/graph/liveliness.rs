// ss[related graph.liveliness-voters]
use super::deps::*;
// ss[related philosophy.structural-hierarchy]
use super::identity::ActorIdentity;
// ss[related philosophy.structural-hierarchy]
use super::state::GraphLivelinessState;
// ss[related graph.liveliness-voters]
use super::vote::{ShutdownVote, VoterStatus};
// ss[related philosophy.structural-hierarchy]
use log::{debug, error, trace, warn};

/// Manages the liveliness state of the graph and coordinates the shutdown voting process.
///
/// This struct oversees the graph's operational state, tracks registered voters, and handles the
/// collection and evaluation of shutdown votes from actors.
// ss[related graph.for-testing]
pub struct GraphLiveliness {
    /// A list of statuses for all registered voters.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) registered_voters: Vec<VoterStatus>,
    /// THE current state of the graph's liveliness.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) state: GraphLivelinessState,
    /// A thread-safe collection of shutdown votes from actors.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) votes: Arc<Box<[Mutex<ShutdownVote>]>>,
    /// THE total number of votes in favor of shutdown.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) vote_in_favor_total: AtomicUsize,
    /// A shared vector of oneshot channels for sending shutdown notifications.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) shutdown_one_shot_vec: Arc<Mutex<Vec<Sender<()>>>>,
    /// THE count of actors currently registered as voters.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) registered_voter_count: AtomicUsize,
    /// A shared count of the total number of actors in the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actors_count: Arc<AtomicUsize>,
    /// An optional timeout duration for the shutdown process.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) shutdown_timeout: Option<Duration>,
    /// Full catalog of all actors
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_catalog: Arc<RwLock<Vec<ActorIdentity>>>
}

// ss[related graph.for-testing]
impl GraphLiveliness {
    /// Creates a new instance of `GraphLiveliness` with an initial building state.
    ///
    /// This method sets up the necessary structures for tracking the graph's state and voter information.
    ///
    /// # Arguments
    ///
    /// * `one_shot_shutdown` - A shared vector of oneshot senders for shutdown signals.
    /// * `actors_count` - A shared counter representing the total number of actors.
    ///
    /// # Returns
    ///
    /// A newly initialized `GraphLiveliness` instance.
    // ss[related graph.for-testing]
    pub(crate) fn new(
        one_shot_shutdown: Arc<Mutex<Vec<Sender<()>>>>,
        actors_count: Arc<AtomicUsize>,
        actors_catalog: Arc<RwLock<Vec<ActorIdentity>>>
    ) -> Self {
        GraphLiveliness {
            actors_count,
            registered_voter_count: AtomicUsize::new(0),
            registered_voters: Vec::new(),
            state: GraphLivelinessState::Building,
            votes: Arc::new(Box::new([])),
            vote_in_favor_total: AtomicUsize::new(0),
            shutdown_one_shot_vec: one_shot_shutdown,
            shutdown_timeout: None,
            actor_catalog: actors_catalog
        }
    }

    // ss[related graph.for-testing]
    pub(crate) fn actor_by_id(&self, id: usize) -> Option<ActorIdentity> {

       let vec = self.actor_catalog.read();
       vec.iter().find(|x| x.id==id).map(|x|x.clone())
    }

    /// Transitions the graph from the building state to the running state.
    ///
    /// This method updates the state to indicate that the graph is now operational.
    ///
    /// # Panics
    ///
    /// Panics if the current state is not `Building`, ensuring a valid state transition.
    // ss[related graph.for-testing]
    pub(crate) fn building_to_running(&mut self) {
        if self.state.eq(&GraphLivelinessState::Building) {
            self.state = GraphLivelinessState::Running;
        } else {
            error!("unexpected state {:?}", self.state);
        }
    }

    /// Marks an actor as dead after it has exited normally.
    ///
    /// This method updates the voter status to exclude the actor from future shutdown votes.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE identity of the actor to be removed from voting.
    // ss[related graph.for-testing]
    pub(crate) fn remove_voter(&mut self, ident: ActorIdentity) {
        if self.registered_voters[ident.id].eq(&VoterStatus::Registered(ident)) {
            self.registered_voters[ident.id] = VoterStatus::Dead(ident);
        }
    }

    /// Registers an actor as a voter in the shutdown process.
    ///
    /// This method adds the actor to the list of voters, enabling it to participate in shutdown decisions.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE identity of the actor to register.
    // ss[impl graph.liveliness-voters]
    pub(crate) fn register_voter(&mut self, ident: ActorIdentity) {
        if ident.id >= self.registered_voters.len() {
            self.registered_voters.resize(ident.id + 1, VoterStatus::None);
        }
        if self.registered_voters[ident.id].eq(&VoterStatus::None) {
            self.registered_voter_count.fetch_add(1, Ordering::SeqCst);
        }
        self.registered_voters[ident.id] = VoterStatus::Registered(ident);
    }

    /// Waits for all actors to register as voters within a specified timeout.
    ///
    /// This method blocks until all actors have registered or the timeout is exceeded, then transitions to running.
    ///
    /// # Arguments
    ///
    /// * `timeout` - THE maximum duration to wait for actor registration.
    // ss[related graph.for-testing]
    pub(crate) fn wait_for_registrations(&mut self, timeout: Duration) {
        let expected_count = self.actors_count.load(Ordering::SeqCst);
        if expected_count > 0 {
            trace!("waiting for actors to register: {:?} vs {:?}", self.registered_voter_count.load(Ordering::SeqCst), self.actors_count.load(Ordering::SeqCst));
            let start = Instant::now();
            while self.registered_voter_count.load(Ordering::SeqCst) < self.actors_count.load(Ordering::SeqCst) {
                trace!(" waiting for actors to register: {:?} vs {:?}", self.registered_voter_count.load(Ordering::SeqCst), self.actors_count.load(Ordering::SeqCst));
                let elapsed = start.elapsed();
                if elapsed > timeout {

                    error!("timeout on startup, not all actors registered: {:?} vs {:?}", self.registered_voter_count.load(Ordering::SeqCst), self.actors_count.load(Ordering::SeqCst));
                    error!("if you need more startup time than {:?} use start_with_timeout",timeout);
                    error!("if any of these actors are in a troupe, ensure the troupe is dropped BEFORE calling graph.start()");
                    
                    //find all actors in the None status
                    let missing: Vec<_> = self.registered_voters.iter()
                        .enumerate()
                        .filter(|(_, status)| matches!(status, VoterStatus::None))
                        .map(|(id,_)| self.actor_by_id(id))
                        .collect();


                    error!("missing actors: {:?}",missing);

                    std::process::exit(1); //exit, we did not start up in a reasonable way
                }
                thread::sleep(Duration::from_millis(40));
            }
        } else {
            #[cfg(not(test))]
            warn!("This graph contains no actors.");
        }
        trace!("changed to running state");
        self.building_to_running();
    }

    /// Initiates a shutdown request for the graph and notifies all actors.
    ///
    /// This method transitions the graph to the `StopRequested` state and triggers voting among actors.
    ///
    /// # Arguments
    ///
    /// * `runtime_state` - A shared reference to the graph's liveliness state.
    // ss[impl graph.request-shutdown]
    pub(crate) async fn internal_request_shutdown(runtime_state: Arc<RwLock<GraphLiveliness>>) {
        trace!("starting shutdown one shots");
        if runtime_state.read().state.eq(&GraphLivelinessState::Running) {
            let read = runtime_state.read();
            let votes: Vec<Mutex<ShutdownVote>> = read.registered_voters.iter().enumerate().map(|(i, v)| {
                Mutex::new(ShutdownVote {
                    id: i,
                    signature: None,
                    in_favor: false,
                    voter_status: v.clone(),
                    veto_backtrace: None,
                    veto_reason: None,
                })
            }).collect();
            let local_oss = read.shutdown_one_shot_vec.clone();
            drop(read);
            let mut write = runtime_state.write();
            write.votes = Arc::new(votes.into_boxed_slice());
            write.vote_in_favor_total.store(0, Ordering::SeqCst);
            write.state = GraphLivelinessState::StopRequested;
            drop(write);
            GraphLiveliness::vote_for_the_dead(runtime_state);
            let mut one_shots: MutexGuard<Vec<Sender<_>>> = local_oss.lock().await;
            while let Some(f) = one_shots.pop() {
                let _ignore = f.send(());
                //trace!("send one shot {:?}",_ignore);
            }
            trace!("every actor has had one shot shutdown fired now");
        } else if runtime_state.read().is_in_state(&[GraphLivelinessState::Building]) {
            warn!("request_shutdown should only be called after start");
        }
    }

    /// Automatically casts votes in favor of shutdown for actors marked as dead.
    ///
    /// This method ensures that inactive actors do not impede the shutdown process.
    ///
    /// # Arguments
    ///
    /// * `runtime_state` - A shared reference to the graph's liveliness state.
    // ss[related graph.for-testing]
    pub(crate) fn vote_for_the_dead(runtime_state: Arc<RwLock<GraphLiveliness>>) {
        let read = runtime_state.read();
        let the_dead: Vec<(usize, ActorIdentity)> = read.registered_voters.iter().enumerate().flat_map(|(i, v)| {
            if let VoterStatus::Dead(ident) = v {
                let my_ballot = &read.votes[i];
                if let Some(vote) = my_ballot.try_lock() {
                    if !vote.in_favor {
                        Some((i, *ident))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }).collect();
        drop(read);
        if !the_dead.is_empty() {
            let write = runtime_state.write();
            the_dead.iter().for_each(|(i, ident)| {
                let my_ballot = &write.votes[*i];
                if let Some(mut vote) = my_ballot.try_lock() {
                    assert_eq!(vote.id, *i);
                    vote.signature = Some(*ident);
                    vote.in_favor = true;
                    write.vote_in_favor_total.fetch_add(1, Ordering::SeqCst);
                } else {
                    error!("voting integrity error, someone else has my ballot {:?} in_favor of shutdown", ident);
                }
            })
        }
    }

    /// Checks whether the graph has reached a stopped state based on votes and timeout.
    ///
    /// This method evaluates if the shutdown process has completed successfully or timed out.
    ///
    /// # Arguments
    ///
    /// * `now` - THE instant when the shutdown was initiated.
    /// * `timeout` - THE maximum duration allowed for a clean shutdown.
    ///
    /// # Returns
    ///
    /// An optional new state if the graph has stopped, or `None` if still in progress.
    // ss[related graph.for-testing]
    pub fn check_is_stopped(&self, now: Instant, timeout: Duration) -> Option<GraphLivelinessState> {
        if self.is_in_state(&[
            GraphLivelinessState::StopRequested,
            GraphLivelinessState::Stopped,
            GraphLivelinessState::StoppedUncleanly,
        ]) {
            let voters_count = self.votes.len();
            if self.vote_in_favor_total.load(Ordering::SeqCst) == voters_count {
                Some(GraphLivelinessState::Stopped)
            } else if (voters_count>0) && (now.elapsed()>timeout) {
                Some(GraphLivelinessState::StoppedUncleanly)
            } else {
                if voters_count>0 {
                    None
                } else {
                    assert_eq!(0,voters_count);
                    Some(GraphLivelinessState::Stopped)
                }
            }
        } else {
            None
        }
    }

    /// Determines if the graph is currently in one of the specified states.
    ///
    /// This method checks the current state against a list of possible states.
    ///
    /// # Arguments
    ///
    /// * `matches` - A slice of states to check against.
    ///
    /// # Returns
    ///
    /// `true` if the current state matches any of the provided states, `false` otherwise.
    // ss[related graph.for-testing]
    pub fn is_in_state(&self, matches: &[GraphLivelinessState]) -> bool {
        matches.iter().any(|f| f.eq(&self.state))
    }

    /// Checks if all actors except the telemetry system (Collector and Server) have voted 'yes'.
    /// This is used by telemetry actors to ensure they are the last to shut down,
    /// allowing them to capture the final state of all other actors.
    ///
    /// # Panics
    /// Panics if there are fewer than 2 actors, as the telemetry system itself requires
    /// both a Collector and a Server.
    // ss[related graph.for-testing]
    pub fn is_shutdown_telemetry_complete(&self, count: usize) -> bool {
        let total = self.votes.len();
        assert!(total >= count, "Invariant failure: Telemetry system requires at least {:?} actors (ie, Collector and Server)", count);
        let yes_votes = self.vote_in_favor_total.load(Ordering::Relaxed);
        yes_votes >= total - count
    }

    /// Assesses whether an actor should continue running based on the graph's state and its vote.
    ///
    /// This method helps actors determine their operational status during state transitions.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE identity of the actor querying its status.
    /// * `accept_fn` - A closure that determines if the actor accepts the shutdown request.
    ///
    /// # Returns
    ///
    /// `Some(true)` if the actor should keep running, `Some(false)` if it should stop, or `None` if still building.
    // ss[impl philosophy.cooperative-liveliness]
    // ss[impl actor.is-running-loop]
    // ss[impl actor.shutdown-veto]
    // ss[impl graph.shutdown.veto]
    // ss[impl graph.shutdown.accept]
    pub(crate) fn is_running<F: FnMut() -> bool>(&self, ident: ActorIdentity, mut accept_fn: F) -> Option<bool> {
        match self.state {
            GraphLivelinessState::Building => {
                thread::yield_now();
                None
            }
            GraphLivelinessState::Running => Some(true),
            GraphLivelinessState::StopRequested => {
                let my_ballot = &self.votes[ident.id];
                if let Some(mut vote) = my_ballot.try_lock() {


                    debug_assert_eq!(vote.id, ident.id);
                    let in_favor = accept_fn(); //has side effect, must act on results!
                    if in_favor {
                            trace!("now agreed to shutdown: {:?}",&ident);
                    }
                    vote.signature = Some(ident);
                    if in_favor && !vote.in_favor {
                        self.vote_in_favor_total.fetch_add(1, Ordering::SeqCst);
                        vote.veto_backtrace = None;
                        vote.in_favor = in_favor;
                    } else {
                        //if cfg!(debug_assertions) { //TODO: noise!, not the best feature
                        //    vote.veto_backtrace = Some(Backtrace::capture());
                        //}
                        vote.veto_reason = i_take_expression();
                        if vote.in_favor {
                            trace!("already voted in favor! : {:?} {:?} vs {:?}", ident, in_favor, vote.in_favor);
                        }
                    }
                    drop(vote);
                    Some(!in_favor)
                } else {
                 //   error!("2 hello {:?}",&ident);

                    trace!("just try again later, unable to get the lock");
                    Some(true)
                }
            }
            GraphLivelinessState::Stopped | GraphLivelinessState::StoppedUncleanly => Some(false),
        }
    }
}
