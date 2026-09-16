// ss[related graph.for-testing]
use super::deps::*;
// ss[related philosophy.structural-hierarchy]
use super::builder::GraphBuilder;
// ss[related philosophy.structural-hierarchy]
use super::identity::ActorIdentity;
// ss[related graph.for-testing]
use super::lambda_bridge::{self, LambdaEgress, LambdaHost, LambdaIngress};
// ss[related graph.for-testing]
use super::liveliness::GraphLiveliness;
// ss[related philosophy.structural-hierarchy]
use super::shutdown::{effective_block_until_stopped_timeout, watch_shutdown};
// ss[related philosophy.structural-hierarchy]
use super::state::GraphLivelinessState;
// ss[related graph.for-testing]
use super::testing_guard::StageManagerGuard;
// ss[related philosophy.structural-hierarchy]
use log::{debug, error, trace};

/// Represents the graph of actors and manages their execution and lifecycle.
///
/// This struct orchestrates the actors within the SteadyState framework, handling their startup,
/// execution, telemetry, and shutdown processes.
// ss[impl philosophy.explicit-ownership]
pub struct Graph {
    /// THE arguments passed to the graph, stored in a thread-safe manner.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    /// A shared counter for the number of channels in the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) channel_count: Arc<AtomicUsize>,
    /// A shared counter for the number of actors in the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_count: Arc<AtomicUsize>,
    /// A mutex for synchronizing thread operations.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) thread_lock: Arc<Mutex<()>>,
    /// A shared counter for the number of actor troupes.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) team_count: Arc<AtomicUsize>,
    /// Indicates whether the graph is configured for testing.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) is_for_testing: bool,
    /// A collection of telemetry receivers for monitoring the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// THE shared liveliness state of the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    /// A shared vector of oneshot senders for shutdown notifications.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// An optional backplane for testing side-channel communications.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) backplane: Arc<Mutex<Option<StageManager>>>,
    /// THE rate at which telemetry data is produced, in milliseconds.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) telemetry_production_rate_ms: u64,
    /// An optional hex color for the telemetry top bar.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) telemetry_colors: Option<(String, String)>,
    /// A lazily initialized reference to the Aeron media driver.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) aeron: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// An optional barrier for synchronizing actor shutdown.
    pub shutdown_barrier: Option<Arc<Barrier>>,
    /// Default stack size for all actors in the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) default_stack_size: Option<usize>,
    /// Minimum size for bundles.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) bundle_floor_size: usize,
    /// Names of actors that use `internal_behavior` in test graphs (pipeline processors).
    // ss[related philosophy.structural-hierarchy]
    pub(crate) test_pipeline_internal_names: Arc<HashSet<&'static str>>,
    /// Univeral list of all actor identifiers
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_catalog: Arc<RwLock<Vec<ActorIdentity>>>,
    /// Dynamic troupe bags awaiting packing at `start` (shared with TroupeGuard).
    // ss[impl troupe.dynamic-is-a-bag]
    // ss[impl troupe.dynamic-finalize-at-start]
    pub(crate) pending_dynamic_troupes:
        Arc<parking_lot::Mutex<Vec<Arc<parking_lot::Mutex<Option<crate::actor_builder::Troupe>>>>>>,
    /// Optional pin for packed OS thread count (`with_pack_slots` / tests).
    // ss[impl troupe.dynamic-slot-budget]
    // ss[impl troupe.dynamic-for-testing]
    pub(crate) pack_slots: Option<usize>,
    /// SoloAct OS threads already spawned (for slot budget).
    // ss[impl troupe.dynamic-slot-budget]
    pub(crate) solo_thread_count: Arc<AtomicUsize>,
    /// Normal troupe OS threads already spawned (for slot budget).
    // ss[impl troupe.dynamic-slot-budget]
    pub(crate) normal_troupe_thread_count: Arc<AtomicUsize>,
    /// Build-time packing incidence (channel endpoints). Filled in Phase 2+.
    // ss[impl graph.pack.incidence-before-start]
    pub(crate) pack_incidence: Arc<parking_lot::Mutex<Vec<crate::actor_builder::pack::PackEdge>>>,
    /// Per-actor pack ports recorded at build (actor id → rx/tx channel ids).
    // ss[impl graph.pack.incidence-before-start]
    pub(crate) pack_ports: Arc<parking_lot::Mutex<std::collections::HashMap<usize, PackPorts>>>,
    /// channel_id → (capacity, byte_footprint, girth, optional bundle family key).
    // ss[impl graph.pack.incidence-before-start]
    pub(crate) channel_pack_meta:
        Arc<parking_lot::Mutex<std::collections::HashMap<usize, (usize, usize, usize, Option<usize>)>>>,
    /// When true, dirty persistent state at park is a defect.
    // ss[impl graph.for-lambda]
    // ss[impl state.dirty-at-park]
    pub(crate) strict_persist: bool,
}

/// RX/TX channel ids declared for packing (must match `into_spotlight`).
#[derive(Clone, Debug, Default)]
// ss[impl graph.pack.incidence-before-start]
pub struct PackPorts {
    /// Channel ids this actor receives on (must match spotlight RX order).
    pub rx_channel_ids: Vec<usize>,
    /// Channel ids this actor sends on (must match spotlight TX order).
    pub tx_channel_ids: Vec<usize>,
}
// ss[related graph.for-testing]
impl Graph {
    /// Acquires a lock on the stage manager for testing purposes.
    ///
    /// This method provides a guard that allows interaction with the backplane in a test environment.
    ///
    /// # Returns
    ///
    /// A `StageManagerGuard` that holds the lock until dropped.
    // ss[related graph.for-testing]
    pub fn stage_manager(&self) -> StageManagerGuard<'_> {
        let guard = core_exec::block_on(self.backplane.lock());
        StageManagerGuard { guard }
    }

    /// Retrieves the Aeron media driver, initializing it if necessary.
    ///
    /// This method attempts to access or establish the media driver for communication purposes.
    ///
    /// # Returns
    ///
    /// An optional reference to the media driver, or `None` if unavailable.
    // ss[related graph.for-testing]
    pub fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Self::aeron_media_driver_internal(&self.aeron, self.is_for_testing)
    }

    /// Retry budget for [`aeron_context_with_retry`] when the graph or actor was built for testing.
    // ss[related graph.for-testing]
    // ss[impl distributed.media-driver-testing]
    pub(crate) fn aeron_init_timeouts(for_tests: bool) -> (Duration, Duration) {
        // Gate C live-driver tests still use `GraphBuilder::for_testing()` (no telemetry),
        // but a 2s CNC wait is too short after a media-driver restart.
        let gate_c = std::env::var("SS_AERON_GATE_C")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if for_tests && !gate_c {
            (Duration::from_secs(2), Duration::from_millis(100))
        } else {
            (Duration::from_secs(60), Duration::from_millis(50))
        }
    }

    /// Internal helper to retrieve or initialize the Aeron media driver.
    ///
    /// This method manages the lazy initialization of the media driver with retry logic.
    ///
    /// # Arguments
    ///
    /// * `holder` - THE `OnceLock` containing the media driver instance.
    /// * `for_tests` - When true, use a short wait/retry budget suitable for unit tests without a media driver.
    ///
    /// # Returns
    ///
    /// An optional reference to the media driver.
    // ss[related graph.for-testing]
    pub(crate) fn aeron_media_driver_internal(
        holder: &OnceLock<Option<Arc<Mutex<Aeron>>>>,
        for_tests: bool,
    ) -> Option<Arc<Mutex<Aeron>>> {
        let (max_wait, retry_interval) = Self::aeron_init_timeouts(for_tests);
        holder
            .get_or_init(|| aeron_context_with_retry(Context::new(), max_wait, retry_interval))
            .clone()
    }

    /// Sets the logging level for the graph's operations.
    ///
    /// This method configures the verbosity of log output for the graph.
    ///
    /// # Arguments
    ///
    /// * `loglevel` - THE desired logging level to apply.
    // ss[related graph.for-testing]
    pub fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

    /// Attempts to retrieve the graph's arguments cast to a specific type.
    ///
    /// This method allows accessing the arguments provided during graph construction.
    ///
    /// # Type Parameters
    ///
    /// * `A` - THE type to which the arguments should be cast.
    ///
    /// # Returns
    ///
    /// An optional reference to the arguments if the cast succeeds, or `None` if it fails.
    // ss[related graph.for-testing]
    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    /// Creates a test monitor for use in testing scenarios.
    ///
    /// This method constructs a monitor that operates independently of a full graph, intended for testing only.
    ///
    /// # Arguments
    ///
    /// * `name` - THE name to assign to the test monitor.
    ///
    /// # Returns
    ///
    /// A `SteadyActorShadow` instance configured for testing.
    // ss[related graph.for-testing]
    pub fn new_testing_test_monitor(&self, name: &'static str) -> SteadyActorShadow {
        trace!("this is for testing only, never run as part of your release");
        let channel_count = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();
        let oneshot_shutdown = {
            let (send_shutdown_notice, rx) = oneshot::channel();
            let local_vec = self.oneshot_shutdown_vec.clone();
            let runtime_state = self.runtime_state.clone();
            core_exec::block_on(async move {
                let mut v = local_vec.lock().await;
                // If the graph is already in StopRequested state, fire the signal immediately
                // for this new actor instance. This ensures that actors born during the 
                // shutdown window (e.g. after a panic) don't miss the global signal.
                if runtime_state.read().is_in_state(&[GraphLivelinessState::StopRequested]) {
                    let _ = send_shutdown_notice.send(());
                } else {
                    v.push(send_shutdown_notice);
                }
            });
            rx.shared()
        };
        let now = Instant::now();
        SteadyActorShadow {
            channel_count,
            ident: ActorIdentity::new(usize::MAX, name, None),
            args: self.args.clone(),
            is_in_graph: false,
            actor_metadata: Arc::new(ActorMetaData::default()),
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            regeneration: 0,
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown,
            last_periodic_wait: Default::default(),
            actor_start_time: now,
            node_tx_rx: None,
            frame_rate_ms: self.telemetry_production_rate_ms,
            team_id: 0,
            show_thread_info: false,
            aeron_meda_driver: self.aeron.clone(),
            aeron_init_for_tests: true,
            use_internal_behavior: true,
            shutdown_barrier: self.shutdown_barrier.clone(),
            index_wait_last_avail: AtomicUsize::new(usize::MAX),
            index_wait_last_vacant: AtomicUsize::new(usize::MAX),
            index_wait_last_avail_vacant: AtomicUsize::new(usize::MAX),
            expected_pack_ports: None,
            strict_persist: self.strict_persist,
        }
    }

    /// Creates a new `ActorBuilder` for constructing actors within the graph.
    ///
    /// This method provides a builder to define and initialize new actors.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance linked to this graph.
    // ss[related graph.for-testing]
    pub fn actor_builder(&mut self) -> ActorBuilder {
        ActorBuilder::new(self)
    }

    /// Creates a `TroupeGuard` for managing a group of actors that execute together.
    ///
    /// This method sets up a **normal** troupe that will be spawned when the guard is dropped.
    ///
    /// # Returns
    ///
    /// A `TroupeGuard` instance for managing the actor troupe.
    // ss[related graph.for-testing]
    // ss[impl troupe.dynamic-member-of]
    pub fn actor_troupe(&self) -> TroupeGuard {
        TroupeGuard {
            troupe: Some(crate::actor_builder::Troupe::new(self)),
            dynamic_shared: None,
            on_normal_spawned: Some(self.normal_troupe_thread_count.clone()),
        }
    }

    /// Creates a **dynamic** troupe bag for startup packing.
    ///
    /// Actors added with `ScheduleAs::MemberOf` join this bag. No OS thread is created until
    /// [`Graph::start`] / [`Graph::start_with_timeout`] finalizes packing
    /// (`troupe.dynamic-is-a-bag`, `troupe.dynamic-finalize-at-start`).
    ///
    /// # Returns
    ///
    /// A `TroupeGuard` that does **not** spawn on Drop; it shares the bag with this Graph.
    // ss[impl troupe.dynamic-is-a-bag]
    // ss[impl troupe.dynamic-member-of]
    // ss[impl troupe.dynamic-hold-until-start]
    // ss[impl philosophy.startup-schedule]
    pub fn dynamic_troupe(&self) -> TroupeGuard {
        let bag = Arc::new(parking_lot::Mutex::new(Some(
            crate::actor_builder::Troupe::new_dynamic(self),
        )));
        self.pending_dynamic_troupes.lock().push(bag.clone());
        TroupeGuard {
            troupe: None,
            dynamic_shared: Some(bag),
            on_normal_spawned: None,
        }
    }

    /// Applies fail-fast behavior by setting a panic hook that exits immediately on panic.
    ///
    /// This method is active only in debug builds and can be disabled via configuration.
    #[cfg(feature = "disable_actor_restart_on_failure")]
    // ss[related graph.for-testing]
    fn apply_fail_fast(&self) {
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic_info| {
                default_hook(panic_info);
                std::process::exit(-1);
            }));
    }

    /// Starts the graph with a default timeout of 40 seconds for actor registration.
    ///
    /// This method initiates the graph's operation, waiting for actors to register before proceeding.
    // ss[related graph.for-testing]
    pub fn start(&mut self) {
        self.start_with_timeout(Duration::from_secs(20));
    }

    /// Starts the graph with a specified timeout for actor registration.
    ///
    /// This method initiates the graph and waits for all actors to register within the given duration.
    ///
    /// # Arguments
    ///
    /// * `duration` - THE maximum time to wait for actor registration.
    ///
    /// # Returns
    ///
    /// `true` if all actors registered within the timeout, `false` otherwise.
    // ss[related graph.for-testing]
    // ss[impl troupe.dynamic-finalize-at-start]
    // ss[impl philosophy.startup-schedule]
    pub fn start_with_timeout(&mut self, duration: Duration) -> bool {
        trace!("start was called");
        // Pack dynamic troupes onto OS threads BEFORE waiting for registrations.
        // Packable actors must not run until their sub-troupe is spawned.
        self.finalize_dynamic_troupes();
        let mut state = self.runtime_state.write();
        state.wait_for_registrations(duration);
        if !state.is_in_state(&[GraphLivelinessState::Running]) {
            error!("timeout on startup, graph is not in the running state");
            false
        } else {
            true
        }
    }

    /// Drain pending dynamic bags, pack into ≤ slots normal troupes, and spawn them.
    ///
    /// Idempotent: a second call finds an empty pending list.
    // ss[impl troupe.dynamic-finalize-at-start]
    // ss[impl troupe.dynamic-is-a-bag]
    // ss[impl troupe.dynamic-slot-budget]
    // ss[impl troupe.dynamic-no-incidence-fallback]
    // ss[impl troupe.dynamic-hold-until-start]
    // ss[impl troupe.dynamic-channel-establish]
    // ss[impl troupe.dynamic-no-repack]
    pub(crate) fn finalize_dynamic_troupes(&mut self) {
        use crate::actor_builder::pack::{pack_actors, PackEdge};
        use crate::actor_builder::slot_budget::SlotBudget;
        use crate::actor_builder::{Troupe, TroupeKind};
        use log::debug;

        let bags: Vec<_> = {
            let mut pending = self.pending_dynamic_troupes.lock();
            std::mem::take(&mut *pending)
        };
        if bags.is_empty() {
            return;
        }

        // Take troupes out of shared slots (guards still alive see None afterward).
        let mut all_troupes: Vec<Troupe> = Vec::new();
        for bag in bags {
            let mut slot = bag.lock();
            if let Some(t) = slot.take() {
                if t.kind == TroupeKind::Dynamic {
                    all_troupes.push(t);
                }
            }
        }
        if all_troupes.is_empty() {
            return;
        }

        // SoloAct threads (including telemetry actors when enabled) and normal troupes
        // already occupy OS threads; do not double-count telemetry.
        let reserved = self.solo_thread_count.load(Ordering::SeqCst)
            + self.normal_troupe_thread_count.load(Ordering::SeqCst);
        let budget = match self.pack_slots {
            Some(n) => SlotBudget::Pinned(n),
            None if self.is_for_testing => SlotBudget::Pinned(1),
            None => SlotBudget::Detect,
        };
        let slots = budget.resolve(reserved);
        debug!(
            "dynamic troupe packing: reserved_threads={} (solo+normal), pack_slots_budget={}",
            reserved, slots
        );

        let mut flat: Vec<(usize, crate::actor_builder::FutureBuilderType)> = Vec::new();
        for mut t in all_troupes {
            while let Some(fb) = t.future_builder.pop_front() {
                let id = fb.ident().id;
                flat.push((id, fb));
            }
        }
        if flat.is_empty() {
            return;
        }

        let actor_ids: Vec<usize> = flat.iter().map(|(id, _)| *id).collect();
        // Rebuild incidence from pack_ports if present; else empty → even-split.
        let edges: Vec<PackEdge> = self.rebuild_pack_edges();
        if edges.is_empty() {
            debug!(
                "dynamic troupe packing: no incidence; even-split into {} slot(s) for {} actor(s)",
                slots,
                actor_ids.len()
            );
        } else {
            debug!(
                "dynamic troupe packing: Kruskal/even into {} slot(s) for {} actor(s), {} edge(s)",
                slots,
                actor_ids.len(),
                edges.len()
            );
        }
        let clusters = pack_actors(&actor_ids, &edges, slots);

        let mut by_id: std::collections::HashMap<usize, _> =
            flat.into_iter().map(|(id, fb)| (id, fb)).collect();

        for (i, cluster) in clusters.into_iter().enumerate() {
            let mut sub = Troupe::new(self);
            sub.kind = TroupeKind::Normal;
            sub.with_name(&format!("Packed-{}", i));
            for id in cluster {
                if let Some(fb) = by_id.remove(&id) {
                    sub.future_builder.push_back(fb);
                }
            }
            if !sub.future_builder.is_empty() {
                let n = sub.spawn();
                if n > 0 {
                    self.normal_troupe_thread_count
                        .fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    /// Build PackEdge list from recorded pack ports + channel pack meta.
    // ss[impl graph.pack.incidence-before-start]
    pub(crate) fn rebuild_pack_edges(&self) -> Vec<crate::actor_builder::pack::PackEdge> {
        use crate::actor_builder::pack::{BundleFamily, PackEdge};
        let ports = self.pack_ports.lock();
        if ports.is_empty() {
            return self.pack_incidence.lock().clone();
        }
        // Map channel_id → (tx_actor, rx_actors...)
        let mut tx_owner: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut rx_owners: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        for (actor_id, p) in ports.iter() {
            for &cid in &p.tx_channel_ids {
                tx_owner.insert(cid, *actor_id);
            }
            for &cid in &p.rx_channel_ids {
                rx_owners.entry(cid).or_default().push(*actor_id);
            }
        }
        let meta = self.channel_pack_meta.lock();
        let mut edges = Vec::new();
        for (cid, rx_list) in &rx_owners {
            let Some(&from) = tx_owner.get(cid) else {
                continue;
            };
            let (capacity, bytes, girth, bundle_key) = meta
                .get(cid)
                .copied()
                .unwrap_or((64, 512, 1, None));
            for &to in rx_list {
                if from == to {
                    continue;
                }
                let bundle_family = if girth > 1 {
                    bundle_key.map(|family_key| BundleFamily {
                        producer: from,
                        family_key,
                    })
                } else {
                    None
                };
                edges.push(PackEdge {
                    from,
                    to,
                    capacity,
                    byte_footprint: bytes,
                    bundle_family,
                });
            }
        }
        edges
    }

    /// Requests the shutdown of the graph, notifying all actors.
    ///
    /// This method initiates the shutdown process, triggering the voting mechanism among actors.
    // ss[impl graph.request-shutdown]
    pub fn request_shutdown(&mut self) {
        let a = self.runtime_state.clone();
        core_exec::block_on(async move { GraphLiveliness::internal_request_shutdown(a).await });
    }

    /// Blocks the current thread until the graph has fully stopped.
    ///
    /// This method first waits indefinitely for shutdown to be requested (i.e. for the graph
    /// to leave the `Running`/`Building` state). Only once shutdown has been requested does
    /// `clean_shutdown_timeout` begin: it bounds the voting/draining phase during which all
    /// actors must accept the stop.
    ///
    /// # Arguments
    ///
    /// * `clean_shutdown_timeout` - THE maximum duration to wait for a clean shutdown
    ///   after shutdown has been requested.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the shutdown was clean, or an error if it was unclean.
    // ss[impl graph.block-until-stopped]
    pub fn block_until_stopped(self, clean_shutdown_timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
        let timeout = effective_block_until_stopped_timeout(
            clean_shutdown_timeout,
            self.telemetry_production_rate_ms,
        );
        if let Some(wait_on) = {
            self.runtime_state.write().shutdown_timeout = Some(timeout);
            if self.runtime_state.read().is_in_state(&[GraphLivelinessState::Running, GraphLivelinessState::Building]) {
                let (tx, rx) = oneshot::channel();
                let v = self.runtime_state.read().shutdown_one_shot_vec.clone();
                core_exec::block_on(async move {
                    v.lock().await.push(tx);
                });
                Some(rx)
            } else {
                None
            }
        } {
            // Re-check after registration: internal_request_shutdown sets StopRequested
            // BEFORE draining the oneshot vec, so if we observe StopRequested here our
            // tx was either already fired or will never fire - waiting would hang forever.
            // If we still observe Running/Building, our push happened-before any future
            // drain, guaranteeing the oneshot fires.
            if self.runtime_state.read().is_in_state(&[GraphLivelinessState::Running, GraphLivelinessState::Building]) {
                // Wait without a timeout for shutdown to be requested; the clean-shutdown
                // timeout applies only to the voting phase watched below.
                if let Err(dropped) = core_exec::block_on(wait_on) {
                    debug!("shutdown oneshot sender dropped: {:?}", dropped);
                }
            }
        }
        let now = Instant::now();
        let rs = self.runtime_state;
        let tel_prod_rate = Duration::from_millis(self.telemetry_production_rate_ms);
        watch_shutdown(timeout, now, rs, tel_prod_rate)
    }

    /// Constructs a new `Graph` instance based on provided arguments and builder configuration.
    ///
    /// This method initializes the graph with all necessary components for actor execution and management.
    ///
    /// # Type Parameters
    ///
    /// * `A` - THE type of arguments, which must implement `Any`, `Send`, and `Sync`.
    ///
    /// # Arguments
    ///
    /// * `args` - THE arguments to initialize the graph with.
    /// * `builder` - THE `GraphBuilder` providing configuration options.
    ///
    /// # Returns
    ///
    /// A new `Graph` instance ready for use.
    // ss[related graph.for-testing]
    pub fn internal_new<A: Any + Send + Sync>(args: A, builder: GraphBuilder) -> Graph {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let actor_count = Arc::new(AtomicUsize::new(0));
        let actor_catalog = Arc::new(RwLock::new(Vec::new()));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let mut result = Graph {
            args: Arc::new(Box::new(args)),
            channel_count: channel_count.clone(),
            actor_count: actor_count.clone(),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                oneshot_shutdown_vec.clone(),
                actor_count.clone(),
                actor_catalog.clone(),
            ))),
            thread_lock: Arc::new(Mutex::new(())),
            oneshot_shutdown_vec,
            backplane: Arc::new(Mutex::new(builder.backplane)),
            telemetry_production_rate_ms: if builder.telemetry_metric_features {
                builder.telemtry_production_rate_ms
            } else {
                0u64
            },
            telemetry_colors: builder.telemetry_colors,
            team_count: Arc::new(AtomicUsize::new(1)),
            aeron: Default::default(),
            is_for_testing: builder.is_for_testing,
            shutdown_barrier: builder.shutdown_barrier,
            default_stack_size: builder.default_stack_size,
            bundle_floor_size: builder.bundle_floor_size,
            test_pipeline_internal_names: Arc::new(builder.test_pipeline_internal_names.clone()),
            actor_catalog: actor_catalog.clone(),
            pending_dynamic_troupes: Arc::new(parking_lot::Mutex::new(Vec::new())),
            pack_slots: builder.pack_slots,
            solo_thread_count: Arc::new(AtomicUsize::new(0)),
            normal_troupe_thread_count: Arc::new(AtomicUsize::new(0)),
            pack_incidence: Arc::new(parking_lot::Mutex::new(Vec::new())),
            pack_ports: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            channel_pack_meta: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            strict_persist: builder.strict_persist,
        };
        if builder.telemetry_metric_features {
            telemetry::setup::build_telemetry_metric_features(&mut result);
        }
        result
    }

    /// Create a host bridge for warm request/response injects (Lambda Runtime API host thread).
    ///
    /// Not `testing_send_all` / StageManager. Host calls [`LambdaHost::inject`] /
    /// [`LambdaHost::wait_output`]; actors use [`LambdaIngress::wait`] /
    /// [`LambdaEgress::send`].
    // ss[impl graph.lambda.host-bridge]
    // ss[impl graph.lambda.host-not-executor]
    // ss[impl graph.lambda.warm-graph]
    pub fn lambda_bridge<In, Out>(&self) -> (LambdaHost<In, Out>, LambdaIngress<In, Out>, LambdaEgress<In, Out>)
    where
        In: Send + 'static,
        Out: Send + 'static,
    {
        lambda_bridge::lambda_bridge()
    }

    /// Creates a new `ChannelBuilder` for constructing channels within the graph.
    ///
    /// This method provides a builder to define and initialize communication channels.
    ///
    /// # Returns
    ///
    /// A new `ChannelBuilder` instance linked to this graph.
    // ss[related graph.for-testing]
    pub fn channel_builder(&mut self) -> ChannelBuilder {
        let mut builder = ChannelBuilder::new(
            self.channel_count.clone(),
            self.oneshot_shutdown_vec.clone(),
            self.telemetry_production_rate_ms,
        );
        // ss[impl graph.pack.incidence-before-start]
        builder.channel_pack_meta = Some(self.channel_pack_meta.clone());
        builder
    }
}

#[cfg(test)]
#[path = "pack_edges_proptest.rs"]
// ss[related graph.pack.incidence-before-start]
mod pack_edges_proptest;
