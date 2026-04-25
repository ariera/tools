use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, RecvTimeoutError};
use serde::{Deserialize, Serialize};

use crate::{
    config_hash, Candidate, CandidatePredicate, EnumeratorSnapshot, PipelinedOrderedCandidateEnumerator,
    SearchConfig, SnapshotError,
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SuccessSemantics {
    /// Stop as soon as any worker reports success. Fastest.
    FirstDiscovered,
    /// Stop only when the lowest-ordinal success is confirmed (i.e., all
    /// candidates with a smaller ordinal have returned false). Guarantees
    /// the result with the shortest edit path.
    OrderedFirst,
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub workers: usize,
    pub max_pending: usize,
    pub checkpoint_path: Option<PathBuf>,
    pub checkpoint_every: Duration,
    pub progress_every: Duration,
    pub success_semantics: SuccessSemantics,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            workers: num_cpus(),
            max_pending: 256,
            checkpoint_path: None,
            checkpoint_every: Duration::from_secs(60),
            progress_every: Duration::from_secs(10),
            success_semantics: SuccessSemantics::FirstDiscovered,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    Found,
    Exhausted,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchReport {
    pub stop_reason: StopReason,
    pub winning_candidate: Option<Candidate>,
    pub generated: u64,
    pub tested: u64,
    pub elapsed_secs: f64,
}

#[derive(Debug)]
pub enum RunError {
    ConfigError(crate::ConfigError),
    CheckpointLoad(String),
    CheckpointWrite(String),
    SnapshotError(SnapshotError),
    WorkerPanic,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigError(e) => write!(f, "config error: {e}"),
            Self::CheckpointLoad(msg) => write!(f, "checkpoint load error: {msg}"),
            Self::CheckpointWrite(msg) => write!(f, "checkpoint write error: {msg}"),
            Self::SnapshotError(e) => write!(f, "snapshot error: {e}"),
            Self::WorkerPanic => write!(f, "a worker thread panicked"),
        }
    }
}

impl std::error::Error for RunError {}

// ---------------------------------------------------------------------------
// Checkpoint
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EngineCheckpoint {
    schema_version: u32,
    config_hash: String,
    success_semantics: SuccessSemantics,
    enumerator_snapshot: EnumeratorSnapshot,
    /// Candidates dispatched to workers but not yet resolved at checkpoint time.
    /// Re-tested on resume (predicate must be pure).
    pending: Vec<Candidate>,
    generated_count: u64,
    completed_false_count: u64,
    best_success: Option<Candidate>,
    next_required_ordinal: u64,
    /// Ordinals already confirmed false but blocked by a gap in the frontier.
    completed_false_ordinals: Vec<u64>,
}

fn write_checkpoint_atomic(path: &Path, checkpoint: &EngineCheckpoint) -> Result<(), RunError> {
    let json = serde_json::to_vec(checkpoint)
        .map_err(|e| RunError::CheckpointWrite(e.to_string()))?;

    let tmp_path = path.with_extension("tmp");

    let mut f = fs::File::create(&tmp_path)
        .map_err(|e| RunError::CheckpointWrite(e.to_string()))?;
    f.write_all(&json)
        .map_err(|e| RunError::CheckpointWrite(e.to_string()))?;
    f.flush()
        .map_err(|e| RunError::CheckpointWrite(e.to_string()))?;
    f.sync_all()
        .map_err(|e| RunError::CheckpointWrite(e.to_string()))?;
    drop(f);

    fs::rename(&tmp_path, path)
        .map_err(|e| RunError::CheckpointWrite(e.to_string()))?;

    // Fsync parent dir so the rename is durable.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

fn load_checkpoint(path: &Path) -> Result<EngineCheckpoint, RunError> {
    let bytes = fs::read(path)
        .map_err(|e| RunError::CheckpointLoad(e.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| RunError::CheckpointLoad(e.to_string()))
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

struct WorkerResult {
    candidate: Candidate,
    success: bool,
}

fn worker_loop(
    predicate: Arc<dyn CandidatePredicate>,
    jobs: crossbeam_channel::Receiver<Candidate>,
    results: crossbeam_channel::Sender<WorkerResult>,
    stop: Arc<AtomicBool>,
) {
    loop {
        if stop.load(AtomicOrdering::Relaxed) {
            break;
        }
        match jobs.recv_timeout(Duration::from_millis(50)) {
            Ok(candidate) => {
                let success = predicate.test(&candidate.text);
                if results.send(WorkerResult { candidate, success }).is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

struct Controller {
    enumerator: PipelinedOrderedCandidateEnumerator,
    /// Candidates dispatched but not yet resolved.
    pending: BTreeMap<u64, Candidate>,
    generated_count: u64,
    completed_false_count: u64,
    best_success: Option<Candidate>,
    enumerator_exhausted: bool,
    /// OrderedFirst: the next ordinal we need a result for before declaring done.
    next_required_ordinal: u64,
    /// OrderedFirst: false results that arrived ahead of the frontier.
    completed_false_ordinals: BTreeSet<u64>,
    /// Candidates to re-send (populated from checkpoint's pending list on resume).
    resend_queue: VecDeque<Candidate>,
}

impl Controller {
    fn new(enumerator: PipelinedOrderedCandidateEnumerator) -> Self {
        Self {
            enumerator,
            pending: BTreeMap::new(),
            generated_count: 0,
            completed_false_count: 0,
            best_success: None,
            enumerator_exhausted: false,
            next_required_ordinal: 0,
            completed_false_ordinals: BTreeSet::new(),
            resend_queue: VecDeque::new(),
        }
    }

    fn from_checkpoint(
        enumerator: PipelinedOrderedCandidateEnumerator,
        cp: EngineCheckpoint,
    ) -> Self {
        let mut ctrl = Self::new(enumerator);
        ctrl.generated_count = cp.generated_count;
        ctrl.completed_false_count = cp.completed_false_count;
        ctrl.best_success = cp.best_success;
        ctrl.next_required_ordinal = cp.next_required_ordinal;
        ctrl.completed_false_ordinals = cp.completed_false_ordinals.into_iter().collect();
        // Re-enqueue pending candidates so they get retested.
        ctrl.resend_queue = cp.pending.into();
        ctrl
    }

    fn snapshot(&self) -> EnumeratorSnapshot {
        self.enumerator.snapshot()
    }

    fn to_checkpoint(
        &self,
        search_config: &SearchConfig,
        success_semantics: SuccessSemantics,
    ) -> EngineCheckpoint {
        EngineCheckpoint {
            schema_version: 1,
            config_hash: config_hash(search_config),
            success_semantics,
            enumerator_snapshot: self.snapshot(),
            pending: self.pending.values().cloned().collect(),
            generated_count: self.generated_count,
            completed_false_count: self.completed_false_count,
            best_success: self.best_success.clone(),
            next_required_ordinal: self.next_required_ordinal,
            completed_false_ordinals: self.completed_false_ordinals.iter().copied().collect(),
        }
    }

    /// Try to advance the ordered-first frontier by draining completed_false_ordinals.
    fn advance_ordered_frontier(&mut self) {
        while self.completed_false_ordinals.contains(&self.next_required_ordinal) {
            self.completed_false_ordinals.remove(&self.next_required_ordinal);
            self.next_required_ordinal += 1;
        }
    }

    fn is_ordered_first_decidable(&self) -> bool {
        if let Some(ref best) = self.best_success {
            self.next_required_ordinal == best.ordinal
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the full search.
///
/// `predicate` is called once per candidate across all worker threads.
/// The caller may supply `resume = true` to load a checkpoint from
/// `engine_config.checkpoint_path` if one exists.
pub fn run(
    search_config: SearchConfig,
    predicate: Arc<dyn CandidatePredicate>,
    engine_config: EngineConfig,
    resume: bool,
) -> Result<SearchReport, RunError> {
    let start = Instant::now();

    // Set up Ctrl-C handler.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop_clone = Arc::clone(&stop);
        ctrlc::set_handler(move || {
            stop_clone.store(true, AtomicOrdering::Relaxed);
        })
        .ok(); // Ignore error if handler already set.
    }

    // Build or restore controller.
    let mut controller = if resume {
        if let Some(ref path) = engine_config.checkpoint_path {
            if path.exists() {
                let cp = load_checkpoint(path)?;
                // Validate schema version.
                if cp.schema_version != 1 {
                    return Err(RunError::CheckpointLoad(format!(
                        "unsupported checkpoint schema version {}",
                        cp.schema_version
                    )));
                }
                // Validate config hash.
                let expected = config_hash(&search_config);
                if cp.config_hash != expected {
                    return Err(RunError::CheckpointLoad(format!(
                        "checkpoint config hash mismatch: expected {expected}, got {}",
                        cp.config_hash
                    )));
                }
                // Validate semantics match.
                if cp.success_semantics != engine_config.success_semantics {
                    return Err(RunError::CheckpointLoad(format!(
                        "checkpoint success semantics {:?} doesn't match engine config {:?}",
                        cp.success_semantics, engine_config.success_semantics
                    )));
                }
                let enumerator = PipelinedOrderedCandidateEnumerator::from_snapshot(
                    search_config.clone(),
                    cp.enumerator_snapshot.clone(),
                )
                .map_err(RunError::SnapshotError)?;
                Controller::from_checkpoint(enumerator, cp)
            } else {
                let enumerator = PipelinedOrderedCandidateEnumerator::new(search_config.clone())
                    .map_err(RunError::ConfigError)?;
                Controller::new(enumerator)
            }
        } else {
            let enumerator = PipelinedOrderedCandidateEnumerator::new(search_config.clone())
                .map_err(RunError::ConfigError)?;
            Controller::new(enumerator)
        }
    } else {
        let enumerator = PipelinedOrderedCandidateEnumerator::new(search_config.clone())
            .map_err(RunError::ConfigError)?;
        Controller::new(enumerator)
    };

    // Spawn worker threads.
    let (job_tx, job_rx) = bounded::<Candidate>(engine_config.max_pending);
    let (result_tx, result_rx) = unbounded::<WorkerResult>();

    let mut handles = Vec::new();
    for _ in 0..engine_config.workers {
        let pred = Arc::clone(&predicate);
        let rx = job_rx.clone();
        let tx = result_tx.clone();
        let stop_clone = Arc::clone(&stop);
        handles.push(std::thread::spawn(move || {
            worker_loop(pred, rx, tx, stop_clone);
        }));
    }
    drop(job_rx); // controller owns the sender, workers own the receiver clones

    let semantics = engine_config.success_semantics;
    let mut last_checkpoint = Instant::now();
    let mut last_progress = Instant::now();

    let stop_reason = 'outer: loop {
        // Check stop flag.
        if stop.load(AtomicOrdering::Relaxed) {
            break 'outer StopReason::Cancelled;
        }

        // Drain results without blocking.
        loop {
            match result_rx.try_recv() {
                Ok(wr) => {
                    controller.pending.remove(&wr.candidate.ordinal);
                    if wr.success {
                        match semantics {
                            SuccessSemantics::FirstDiscovered => {
                                controller.best_success = Some(wr.candidate);
                                stop.store(true, AtomicOrdering::Relaxed);
                                break 'outer StopReason::Found;
                            }
                            SuccessSemantics::OrderedFirst => {
                                // Keep only the best (lowest ordinal) success.
                                let is_better = controller
                                    .best_success
                                    .as_ref()
                                    .map(|b| wr.candidate.ordinal < b.ordinal)
                                    .unwrap_or(true);
                                if is_better {
                                    controller.best_success = Some(wr.candidate);
                                }
                                if controller.is_ordered_first_decidable()
                                    && controller.pending.is_empty()
                                {
                                    stop.store(true, AtomicOrdering::Relaxed);
                                    break 'outer StopReason::Found;
                                }
                            }
                        }
                    } else {
                        controller.completed_false_count += 1;
                        if semantics == SuccessSemantics::OrderedFirst {
                            if wr.candidate.ordinal == controller.next_required_ordinal {
                                controller.next_required_ordinal += 1;
                                controller.advance_ordered_frontier();
                                // Check decidability now that frontier advanced.
                                if controller.is_ordered_first_decidable()
                                    && controller.pending.is_empty()
                                {
                                    stop.store(true, AtomicOrdering::Relaxed);
                                    break 'outer StopReason::Found;
                                }
                            } else {
                                controller
                                    .completed_false_ordinals
                                    .insert(wr.candidate.ordinal);
                            }
                        }
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    break 'outer StopReason::Cancelled;
                }
            }
        }

        // Check exhaustion: if enumerator is done and all pending resolved.
        if controller.enumerator_exhausted && controller.pending.is_empty() {
            break 'outer match semantics {
                SuccessSemantics::FirstDiscovered => StopReason::Exhausted,
                SuccessSemantics::OrderedFirst => {
                    if controller.best_success.is_some() {
                        StopReason::Found
                    } else {
                        StopReason::Exhausted
                    }
                }
            };
        }

        // Feed jobs: drain resend_queue first (checkpoint resume), then enumerator.
        while !stop.load(AtomicOrdering::Relaxed) {
            let candidate = if let Some(c) = controller.resend_queue.pop_front() {
                c
            } else if controller.enumerator_exhausted {
                break;
            } else {
                match controller.enumerator.next() {
                    Some(c) => {
                        controller.generated_count += 1;
                        c
                    }
                    None => {
                        controller.enumerator_exhausted = true;
                        break;
                    }
                }
            };

            controller.pending.insert(candidate.ordinal, candidate.clone());
            match job_tx.try_send(candidate) {
                Ok(()) => {}
                Err(crossbeam_channel::TrySendError::Full(c)) => {
                    // Channel full: put back and wait for results to drain.
                    controller.pending.remove(&c.ordinal);
                    controller.resend_queue.push_front(c);
                    break;
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    break 'outer StopReason::Cancelled;
                }
            }
        }

        // Checkpoint / progress.
        let now = Instant::now();
        if engine_config.checkpoint_path.is_some()
            && now.duration_since(last_checkpoint) >= engine_config.checkpoint_every
        {
            if let Some(ref path) = engine_config.checkpoint_path {
                let cp = controller.to_checkpoint(&search_config, semantics);
                write_checkpoint_atomic(path, &cp)?;
            }
            last_checkpoint = now;
        }

        if now.duration_since(last_progress) >= engine_config.progress_every {
            eprintln!(
                "[progress] generated={} pending={} tested={} elapsed={:.1}s",
                controller.generated_count,
                controller.pending.len(),
                controller.completed_false_count,
                start.elapsed().as_secs_f64(),
            );
            last_progress = now;
        }

        // Yield briefly to avoid spinning when channel is full and no results ready.
        std::thread::yield_now();
    };

    // Signal workers to stop and wait.
    stop.store(true, AtomicOrdering::Relaxed);
    drop(job_tx);
    for h in handles {
        let _ = h.join();
    }

    // Final checkpoint on clean stop.
    if stop_reason != StopReason::Cancelled {
        if let Some(ref path) = engine_config.checkpoint_path {
            let cp = controller.to_checkpoint(&search_config, semantics);
            let _ = write_checkpoint_atomic(path, &cp);
        }
    }

    let winning_candidate = controller.best_success.clone();
    let tested = controller.completed_false_count
        + winning_candidate.as_ref().map(|_| 1).unwrap_or(0);

    Ok(SearchReport {
        stop_reason,
        winning_candidate,
        generated: controller.generated_count,
        tested,
        elapsed_secs: start.elapsed().as_secs_f64(),
    })
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DistanceMode, EditOps, SearchConfig};
    use std::collections::HashMap;

    fn simple_config(seed: &str) -> SearchConfig {
        SearchConfig {
            seed: seed.to_string(),
            alphabet: vec!['a', 'b', 'c'],
            min_distance: 1,
            max_distance: 2,
            ops: EditOps::replace_only(),
            keyboard_neighbors: HashMap::new(),
            distance_mode: DistanceMode::PerDistanceBestCost,
        }
    }

    fn engine_cfg(semantics: SuccessSemantics) -> EngineConfig {
        EngineConfig {
            workers: 2,
            max_pending: 16,
            checkpoint_path: None,
            checkpoint_every: Duration::from_secs(3600),
            progress_every: Duration::from_secs(3600),
            success_semantics: semantics,
        }
    }

    /// Predicate that matches a specific string.
    struct MatchPredicate(String);
    impl CandidatePredicate for MatchPredicate {
        fn test(&self, candidate: &str) -> bool {
            candidate == self.0
        }
    }

    /// Predicate that always returns false.
    struct NeverPredicate;
    impl CandidatePredicate for NeverPredicate {
        fn test(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn first_discovered_finds_match() {
        let cfg = simple_config("a");
        let pred = Arc::new(MatchPredicate("b".to_string()));
        let report = run(cfg, pred, engine_cfg(SuccessSemantics::FirstDiscovered), false).unwrap();
        assert_eq!(report.stop_reason, StopReason::Found);
        assert!(report.winning_candidate.is_some());
        assert_eq!(report.winning_candidate.unwrap().text, "b");
    }

    #[test]
    fn first_discovered_exhausted_when_no_match() {
        let cfg = simple_config("a");
        let pred = Arc::new(NeverPredicate);
        let report = run(cfg, pred, engine_cfg(SuccessSemantics::FirstDiscovered), false).unwrap();
        assert_eq!(report.stop_reason, StopReason::Exhausted);
        assert!(report.winning_candidate.is_none());
    }

    #[test]
    fn ordered_first_finds_lowest_ordinal_match() {
        // seed "a", replace only, alphabet [a,b,c]
        // distance-1: b(cost3), c(cost3)
        // We match "c" — ordered-first should still report "b" if "b" also matches,
        // but here only "c" matches, so it finds "c".
        let cfg = SearchConfig {
            seed: "a".to_string(),
            alphabet: vec!['a', 'b', 'c'],
            min_distance: 1,
            max_distance: 1,
            ops: EditOps::replace_only(),
            keyboard_neighbors: HashMap::new(),
            distance_mode: DistanceMode::PerDistanceBestCost,
        };
        let pred = Arc::new(MatchPredicate("c".to_string()));
        let report = run(cfg, pred, engine_cfg(SuccessSemantics::OrderedFirst), false).unwrap();
        assert_eq!(report.stop_reason, StopReason::Found);
        assert_eq!(report.winning_candidate.unwrap().text, "c");
    }

    #[test]
    fn ordered_first_exhausted_when_no_match() {
        let cfg = simple_config("a");
        let pred = Arc::new(NeverPredicate);
        let report = run(cfg, pred, engine_cfg(SuccessSemantics::OrderedFirst), false).unwrap();
        assert_eq!(report.stop_reason, StopReason::Exhausted);
        assert!(report.winning_candidate.is_none());
    }

    #[test]
    fn ordered_first_prefers_lower_ordinal_success() {
        // Both "b" (ordinal 0) and "c" (ordinal 1) match.
        // ordered-first must return "b".
        let cfg = SearchConfig {
            seed: "a".to_string(),
            alphabet: vec!['a', 'b', 'c'],
            min_distance: 1,
            max_distance: 1,
            ops: EditOps::replace_only(),
            keyboard_neighbors: HashMap::new(),
            distance_mode: DistanceMode::PerDistanceBestCost,
        };
        struct AlwaysMatch;
        impl CandidatePredicate for AlwaysMatch {
            fn test(&self, _: &str) -> bool {
                true
            }
        }
        let pred = Arc::new(AlwaysMatch);
        let report = run(cfg, pred, engine_cfg(SuccessSemantics::OrderedFirst), false).unwrap();
        assert_eq!(report.stop_reason, StopReason::Found);
        // ordinal 0 is "b" (first alphabetically after "a")
        assert_eq!(report.winning_candidate.unwrap().ordinal, 0);
    }

    #[test]
    fn generated_count_increases() {
        let cfg = simple_config("a");
        let pred = Arc::new(NeverPredicate);
        let report = run(cfg, pred, engine_cfg(SuccessSemantics::FirstDiscovered), false).unwrap();
        assert!(report.generated > 0);
    }
}
