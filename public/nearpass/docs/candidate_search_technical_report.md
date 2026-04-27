
# Technical Report and Implementation Plan: Parallel Candidate Search Engine in Rust

## 1. Executive summary

The project should be implemented as a single Rust search engine, not as Bash orchestration. Bash can remain useful for launching benchmark runs, setting environment variables, and collecting logs, but it should not own scheduling, checkpointing, cancellation, queueing, or result arbitration.

The uploaded feeder (`lib.rs`) is already a solid deterministic ordered graph enumerator. It generates candidate strings by traversing an edit graph using a `BinaryHeap`, per-depth best-cost maps, and optional global deduplication. However, it is not yet checkpoint-capable in a robust production sense. The current live state is not just the last candidate emitted. It is the whole graph frontier:

```rust
heap: BinaryHeap<QueueItem>
best: Vec<HashMap<Vec<char>, u32>>
finalized_global: HashSet<Vec<char>>
stats: EnumeratorStats
```

Therefore, checkpointing must either serialize this state or checkpoint enough engine-level state to replay safely without skipping candidates.

The recommended design is:

```text
single Rust binary
  ├── deterministic feeder / enumerator
  ├── controller thread owning checkpointable search state
  ├── bounded work queue
  ├── fixed worker pool
  ├── result channel
  ├── atomic checkpoint writer
  └── CLI and benchmark scaffold
```

The first production version should optimize for correctness, bounded memory, resumability, and clean early cancellation. Performance tuning should then be done with benchmarks on the M1 Pro, because more threads are not automatically faster on Apple Silicon.

Recommended default semantics:

```text
first-discovered success wins
```

This gives maximum speed. If the project requires the earliest candidate in the feeder's deterministic ordering, implement an explicit `ordered-first` mode. Do not pretend those two semantics are the same. They are not.

---

## 2. Current feeder assessment

The uploaded feeder defines:

- `DistanceMode`
  - `PerDistanceBestCost`
  - `GlobalMinimumDistance`
- `EditOps`
  - delete
  - insert
  - replace
  - swap
- `SearchConfig`
- `Candidate`
- `QueueItem`
- `EnumeratorStats`
- `PipelinedOrderedCandidateEnumerator`

The iterator emits candidates ordered by:

```text
distance ascending, then cost ascending, then lexical Vec<char> ascending
```

This is implemented through reversed `Ord` for `QueueItem` because Rust's `BinaryHeap` is a max-heap.

### 2.1 What is already good

The feeder already has several properties that are valuable for the larger search engine:

1. **Deterministic ordering**

   Candidate ordering is stable given the same normalized configuration.

2. **Bounded distance band**

   `min_distance` and `max_distance` define the search band.

3. **Operation-level deduplication**

   `one_edit_neighbors()` deduplicates neighbors generated through multiple equivalent edits.

4. **Cost-aware relaxation**

   `best[depth][chars] = best known cost` prevents inferior duplicate paths from dominating the frontier.

5. **Unicode correctness**

   The use of `Vec<char>` avoids invalid UTF-8 slicing bugs.

6. **Existing tests**

   The file already contains useful tests for ordering, golden examples, duplicate handling, Unicode behavior, distance modes, edge cases, and invariants.

### 2.2 What is missing for the larger project

The feeder currently lacks:

1. **Candidate ordinals**

   Each emitted candidate needs a stable `ordinal: u64`.

2. **Serialization support**

   Core config, candidate, stats, and snapshot types need `serde` support.

3. **Config hashing**

   A checkpoint must be rejected when used with a different search configuration.

4. **Snapshot/restore API**

   The enumerator must expose a way to serialize and reconstruct its internal frontier.

5. **Engine-level checkpointing**

   A feeder snapshot alone is not enough once candidates are queued or in-flight in worker threads. The engine checkpoint must also include pending jobs.

6. **Explicit success semantics**

   The code currently has deterministic feeder order, but a parallel worker pool can return a later candidate before an earlier candidate has finished testing.

7. **Performance instrumentation**

   The feeder has stats, but the full engine needs throughput, queue depth, worker utilization, checkpoint cost, and time-to-result metrics.

---

## 3. Core architecture

### 3.1 Recommended runtime model

Use a single Rust process with multiple threads.

```text
+---------------------+
| CLI / Main          |
+----------+----------+
           |
           v
+---------------------+
| Search Controller   |
| owns enumerator     |
| owns pending jobs   |
| writes checkpoints  |
+-----+----------+----+
      |          ^
      | jobs     | results
      v          |
+---------------------+
| Bounded Work Queue  |
+----------+----------+
           |
           v
+---------------------+
| Worker Pool         |
| N Rust threads      |
+----------+----------+
           |
           v
+---------------------+
| Existing predicate  |
| candidate -> bool   |
+---------------------+
```

The controller should be the only owner of:

- the enumerator,
- the pending-job map,
- checkpoint-writing decisions,
- result arbitration,
- stop reason.

Workers should be simple and disposable:

```text
receive candidate -> run predicate -> send result
```

They should not write checkpoints, mutate global search state, or decide final correctness.

### 3.2 Why not Bash

Bash is the wrong layer for this problem.

Do not use Bash for:

- worker scheduling,
- early cancellation,
- checkpoint consistency,
- queue backpressure,
- result ordering,
- concurrency control.

The overhead and failure modes are unnecessary. Rust threads are cheaper, safer, and easier to coordinate for this workload.

Bash may be used only for wrapper scripts such as:

```bash
#!/usr/bin/env bash
set -euo pipefail
RUSTFLAGS="-C target-cpu=native" cargo run --release -- run --config search.toml --resume
```

### 3.3 Why not multiple processes initially

Multiple processes add IPC, serialization, process management, and duplicated memory. On an M1 Pro laptop, this is usually not justified for a CPU-bound pure Rust predicate.

Start with threads. Move to processes only if:

- the worker function is not thread-safe,
- process isolation is required,
- the predicate can crash the process,
- or external tools must be executed per candidate.

### 3.4 `rayon` versus custom worker pool

`rayon` is excellent for bulk parallel iteration, but this project needs precise control over:

- early stop,
- checkpoint state,
- bounded pending jobs,
- in-flight candidates,
- progress reporting,
- ordered-first semantics if needed.

Use a custom controller plus `crossbeam-channel` for the first production version.

`rayon` can be revisited later for local CPU-heavy subroutines inside the predicate, not for top-level orchestration.

---

## 4. Search graph model

The feeder is traversing a directed weighted graph.

```text
node  = candidate string represented as Vec<char>
edge  = one edit operation
weight = edit cost
level = number of edit operations from seed
```

Supported edges:

```text
delete:  cost 2
insert:  cost 2
replace: cost 1 if keyboard neighbor, otherwise 3
swap:    cost 1
```

The priority key is:

```text
(depth, accumulated_cost, lexical_chars)
```

This is not a pure Dijkstra search over cost. Depth dominates cost. That appears intentional: the search first considers edit distance, then weighted cost inside each distance layer.

The current graph traversal state is:

```text
heap                 frontier of candidate states still to pop
best[depth][chars]   cheapest accumulated cost found for candidate at exact depth
finalized_global     globally finalized strings in GlobalMinimumDistance mode
stats                operational counters
```

### 4.1 Complexity

Let:

```text
L = current candidate length
A = alphabet size
D = max_distance
F = heap/frontier size
S = number of reached states
```

One expansion generates at most approximately:

```text
delete:  L
insert:  (L + 1) * A
replace: L * (A - 1)
swap:    L - 1
```

So the number of raw one-edit neighbors is:

```text
O(L * A)
```

However, the current implementation copies a `Vec<char>` for most children, so the actual expansion cost is closer to:

```text
O(L^2 * A)
```

because each neighbor construction copies O(L) characters.

Heap operations add:

```text
O(log F)
```

per relaxed state.

Hashing `Vec<char>` also costs O(L).

Total search space grows combinatorially with distance. Even with deduplication, the effective number of states can become very large. This reinforces the need for:

- bounded queues,
- periodic checkpoints,
- benchmark-driven worker counts,
- and no unbounded pre-generation.

---

## 5. Required semantic decision: what means “first true”?

This is the most important correctness decision.

Parallelism makes “first” ambiguous.

### 5.1 Option A: first-discovered success

A worker finds any successful candidate and the run stops immediately.

```text
candidate 5000 succeeds before candidate 3000 finishes
=> candidate 5000 wins
```

Advantages:

- fastest,
- simplest,
- minimum wasted work,
- best match for “stop as soon as anything works”.

Disadvantage:

- does not guarantee the earliest candidate in feeder order.

This should be the default mode for performance.

### 5.2 Option B: ordered-first success

The engine returns the lowest-ordinal successful candidate.

```text
candidate 5000 succeeds
candidate 3000 is still running
=> cannot stop yet
=> wait until all ordinals < 5000 are known false
```

Advantages:

- deterministic best result according to feeder order,
- useful when order encodes quality or likelihood.

Disadvantages:

- slower,
- more bookkeeping,
- weaker early termination,
- more in-flight work after a success has already been seen.

### 5.3 Recommendation

Implement both, but make the default:

```text
first-discovered
```

Expose this as a CLI option:

```bash
candidate-search run --semantics first-discovered
candidate-search run --semantics ordered-first
```

The coding agent must not conflate these modes.

---

## 6. Checkpointing design

### 6.1 Why saving only the last emitted string is insufficient

The feeder is not a simple counter. It is a frontier-based graph search.

At any point, the future sequence depends on:

- heap contents,
- best-cost maps,
- finalized global strings,
- stats/emitted count,
- normalized config.

Saving only this:

```text
last_string = "abc"
```

is not enough to reconstruct the future sequence efficiently or correctly.

### 6.2 Why saving only the feeder snapshot is also insufficient

Once candidates are sent to worker threads, the enumerator has advanced. If the program checkpoints only the enumerator state after dispatching jobs, then a crash may skip candidates that were generated but not yet tested.

Example:

```text
1. controller generates candidate ordinal 100
2. controller sends it to a worker
3. enumerator state advances to ordinal 101
4. checkpoint saves only enumerator snapshot
5. process crashes before candidate 100 returns false
6. resume starts from ordinal 101
7. candidate 100 is skipped
```

That is unacceptable.

Therefore, an engine checkpoint must include both:

```text
enumerator snapshot
pending jobs not yet known false
```

### 6.3 Recommended checkpoint semantics

Use at-least-once candidate evaluation.

Guarantee:

```text
no candidate is skipped
```

Allow:

```text
some pending candidates may be re-tested after resume
```

This is the right tradeoff if the worker predicate is pure:

```rust
fn test_candidate(candidate: &str) -> bool
```

If the worker has side effects, stop and redesign. Exactly-once semantics are much harder and probably not worth it for this search engine.

### 6.4 Engine checkpoint structure

Recommended checkpoint schema:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineCheckpoint {
    pub schema_version: u32,
    pub engine_version: String,
    pub run_id: String,
    pub config_hash: String,
    pub success_semantics: SuccessSemantics,
    pub enumerator: EnumeratorSnapshot,
    pub pending: Vec<Candidate>,
    pub generated_count: u64,
    pub completed_false_count: u64,
    pub created_at_unix_ms: u128,
}
```

Important details:

- `pending` must include every candidate generated but not yet known false.
- A candidate that was already tested false before checkpoint time should not remain in `pending`.
- A candidate still queued or actively running must remain in `pending`.
- On resume, test `pending` first, then continue from `enumerator`.
- Retesting pending jobs is acceptable.

### 6.5 Atomic checkpoint writing

Never overwrite a checkpoint file in place.

Use:

```text
checkpoint.json.tmp
write contents
flush file
fsync file
rename checkpoint.json.tmp -> checkpoint.json
fsync parent directory where supported
```

Rust sketch:

```rust
pub fn write_checkpoint_atomic<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = path.with_extension("tmp");

    {
        let mut file = std::fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp, path)?;

    #[cfg(unix)]
    {
        if let Ok(dir_file) = std::fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }

    Ok(())
}
```

For large checkpoints, switch from pretty JSON to one of:

```text
bincode + zstd
postcard + zstd
serde_json compact + zstd
```

But start with JSON until correctness is proven.

---

## 7. Required changes to the feeder

### 7.1 Add dependencies

In `Cargo.toml`:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
blake3 = "1"
crossbeam-channel = "0.5"
ctrlc = "3"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"

[dev-dependencies]
tempfile = "3"
```

Optional later:

```toml
bincode = "1"
zstd = "0.13"
ahash = "0.8"
criterion = "0.5"
```

Do not add `ahash` until benchmarks show hashing is a bottleneck.

### 7.2 Add serialization derives

Current types need serde support.

```rust
use serde::{Deserialize, Serialize};
```

Recommended derives:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMode { ... }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditOps { ... }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchConfig { ... }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate { ... }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct QueueItem { ... }

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnumeratorStats { ... }
```

### 7.3 Add candidate ordinals

Change `Candidate` from:

```rust
pub struct Candidate {
    pub text: String,
    pub chars: Vec<char>,
    pub distance: usize,
    pub cost: u32,
}
```

to:

```rust
pub struct Candidate {
    pub ordinal: u64,
    pub text: String,
    pub chars: Vec<char>,
    pub distance: usize,
    pub cost: u32,
}
```

Then update `next()`:

```rust
if item.depth >= self.config.min_distance {
    let ordinal = self.stats.emitted;
    self.stats.emitted += 1;

    return Some(Candidate {
        ordinal,
        text: Self::chars_to_string(&item.chars),
        chars: item.chars,
        distance: item.depth,
        cost: item.cost,
    });
}
```

This preserves the meaning of `stats.emitted` as the number of emitted candidates and gives every candidate a stable zero-based ordinal.

### 7.4 Add config normalization and canonical hash

Do not hash raw `SearchConfig` directly. It contains `HashMap` and `HashSet`, whose iteration order is not stable.

Create a canonical representation:

```rust
#[derive(Serialize)]
struct CanonicalSearchConfig {
    seed: String,
    alphabet: Vec<char>,
    min_distance: usize,
    max_distance: usize,
    ops: EditOps,
    keyboard_neighbors: Vec<(char, Vec<char>)>,
    distance_mode: DistanceMode,
}

fn canonical_config(config: &SearchConfig) -> CanonicalSearchConfig {
    let mut alphabet = config.alphabet.clone();
    alphabet.sort_unstable();
    alphabet.dedup();

    let mut keyboard_neighbors: Vec<(char, Vec<char>)> = config
        .keyboard_neighbors
        .iter()
        .map(|(&from, tos)| {
            let mut tos: Vec<char> = tos.iter().copied().collect();
            tos.sort_unstable();
            tos.dedup();
            (from, tos)
        })
        .collect();

    keyboard_neighbors.sort_by_key(|(from, _)| *from);

    CanonicalSearchConfig {
        seed: config.seed.clone(),
        alphabet,
        min_distance: config.min_distance,
        max_distance: config.max_distance,
        ops: config.ops,
        keyboard_neighbors,
        distance_mode: config.distance_mode,
    }
}

pub fn config_hash(config: &SearchConfig) -> String {
    let canonical = canonical_config(config);
    let bytes = serde_json::to_vec(&canonical)
        .expect("canonical SearchConfig should serialize");
    blake3::hash(&bytes).to_hex().to_string()
}
```

Also consider adding:

```rust
impl SearchConfig {
    pub fn normalize(&mut self) {
        self.alphabet.sort_unstable();
        self.alphabet.dedup();
    }

    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}
```

### 7.5 Add enumerator snapshot type

Keep snapshot internals private if possible. External modules should not depend on the exact heap representation.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumeratorSnapshot {
    schema_version: u32,
    config_hash: String,
    heap: Vec<QueueItem>,
    best: Vec<Vec<(Vec<char>, u32)>>,
    finalized_global: Vec<Vec<char>>,
    stats: EnumeratorStats,
}
```

Snapshot method:

```rust
impl PipelinedOrderedCandidateEnumerator {
    pub fn snapshot(&self) -> EnumeratorSnapshot {
        EnumeratorSnapshot {
            schema_version: 1,
            config_hash: config_hash(&self.config),
            heap: self.heap.clone().into_vec(),
            best: self.best
                .iter()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
                .collect(),
            finalized_global: self.finalized_global.iter().cloned().collect(),
            stats: self.stats.clone(),
        }
    }

    pub fn next_ordinal(&self) -> u64 {
        self.stats.emitted
    }
}
```

Restore method:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    UnsupportedVersion { got: u32 },
    ConfigHashMismatch { expected: String, got: String },
    InvalidBestDepthCount { expected: usize, got: usize },
}

impl PipelinedOrderedCandidateEnumerator {
    pub fn from_snapshot(
        mut config: SearchConfig,
        snapshot: EnumeratorSnapshot,
    ) -> Result<Self, SnapshotError> {
        config.alphabet.sort_unstable();
        config.alphabet.dedup();

        if snapshot.schema_version != 1 {
            return Err(SnapshotError::UnsupportedVersion { got: snapshot.schema_version });
        }

        let expected = config_hash(&config);
        if snapshot.config_hash != expected {
            return Err(SnapshotError::ConfigHashMismatch {
                expected,
                got: snapshot.config_hash,
            });
        }

        let expected_best_len = config.max_distance + 1;
        if snapshot.best.len() != expected_best_len {
            return Err(SnapshotError::InvalidBestDepthCount {
                expected: expected_best_len,
                got: snapshot.best.len(),
            });
        }

        let best = snapshot
            .best
            .into_iter()
            .map(|entries| entries.into_iter().collect())
            .collect();

        Ok(Self {
            config,
            heap: BinaryHeap::from(snapshot.heap),
            best,
            finalized_global: snapshot.finalized_global.into_iter().collect(),
            stats: snapshot.stats,
        })
    }
}
```

Do not use `assert_eq!` for production checkpoint validation. Return errors.

---

## 8. Engine implementation plan

### 8.1 Project structure

Recommended crate layout:

```text
candidate-search/
  Cargo.toml
  src/
    lib.rs                  # public library API
    feeder.rs               # current enumerator, moved from uploaded lib.rs
    config.rs               # config loading, normalization, hashing
    checkpoint.rs           # EngineCheckpoint and atomic IO
    engine.rs               # controller loop
    worker.rs               # worker threads and result types
    metrics.rs              # counters and progress snapshots
    bin/
      candidate-search.rs   # CLI entrypoint
  tests/
    feeder_snapshot.rs
    engine_resume.rs
    engine_semantics.rs
    checkpoint_io.rs
  benches/
    feeder_bench.rs
    engine_bench.rs
```

If you want minimum churn, keep the current code in `src/lib.rs` initially and add modules gradually. But before the project grows, move it into `src/feeder.rs`.

### 8.2 Public API sketch

```rust
pub trait CandidatePredicate: Send + Sync + 'static {
    fn test(&self, candidate: &str) -> bool;
}

impl<F> CandidatePredicate for F
where
    F: Fn(&str) -> bool + Send + Sync + 'static,
{
    fn test(&self, candidate: &str) -> bool {
        self(candidate)
    }
}
```

Engine config:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SuccessSemantics {
    FirstDiscovered,
    OrderedFirst,
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub workers: usize,
    pub max_pending: usize,
    pub checkpoint_path: PathBuf,
    pub checkpoint_every: Duration,
    pub progress_every: Duration,
    pub success_semantics: SuccessSemantics,
}
```

Candidate result:

```rust
#[derive(Clone, Debug)]
pub struct WorkerResult {
    pub candidate: Candidate,
    pub success: bool,
    pub elapsed: Duration,
}
```

Search output:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StopReason {
    Found,
    Exhausted,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchReport {
    pub stop_reason: StopReason,
    pub winner: Option<Candidate>,
    pub generated_count: u64,
    pub completed_false_count: u64,
    pub elapsed_ms: u128,
    pub workers: usize,
    pub config_hash: String,
}
```

### 8.3 Controller loop responsibilities

The controller owns all mutable global state:

```rust
struct ControllerState {
    enumerator: PipelinedOrderedCandidateEnumerator,
    pending: BTreeMap<u64, Candidate>,
    completed_false_count: u64,
    best_success: Option<Candidate>,
    cancelled: Arc<AtomicBool>,
}
```

High-level loop:

```text
start workers
load checkpoint if resume requested
if checkpoint has pending jobs, enqueue pending jobs first
loop:
  receive available worker results
  update pending map
  handle success according to semantics
  generate/enqueue more candidates while pending < max_pending
  write checkpoint if interval elapsed
  print progress if interval elapsed
  stop if found / exhausted / cancelled
write final checkpoint or result file
join workers
return SearchReport
```

Pseudocode:

```rust
while !stop {
    // 1. Drain results without blocking too long.
    while let Ok(result) = result_rx.try_recv() {
        handle_result(result, &mut state);
    }

    // 2. Success arbitration.
    match config.success_semantics {
        SuccessSemantics::FirstDiscovered => {
            if state.best_success.is_some() {
                stop_flag.store(true, Ordering::Release);
                break;
            }
        }
        SuccessSemantics::OrderedFirst => {
            if ordered_first_is_decidable(&state) {
                stop_flag.store(true, Ordering::Release);
                break;
            }
        }
    }

    // 3. Refill work queue, bounded by pending size.
    while state.pending.len() < config.max_pending
        && !stop_flag.load(Ordering::Acquire)
    {
        match state.enumerator.next() {
            Some(candidate) => {
                state.pending.insert(candidate.ordinal, candidate.clone());
                if job_tx.send(candidate).is_err() {
                    stop_flag.store(true, Ordering::Release);
                    break;
                }
            }
            None => {
                state.enumerator_exhausted = true;
                break;
            }
        }
    }

    // 4. Exhaustion condition.
    if state.enumerator_exhausted && state.pending.is_empty() {
        break;
    }

    // 5. Checkpoint.
    if last_checkpoint.elapsed() >= config.checkpoint_every {
        let checkpoint = state.to_checkpoint();
        write_checkpoint_atomic(&config.checkpoint_path, &checkpoint)?;
        last_checkpoint = Instant::now();
    }

    // 6. Avoid busy spin.
    wait_for_result_or_timeout(&result_rx, Duration::from_millis(5));
}
```

### 8.4 Pending-job invariant

This invariant is critical:

```text
Every candidate generated from the enumerator but not yet known false must be present in pending.
```

Implementation rule:

```text
insert into pending before sending to worker queue
remove from pending only after receiving a false result
```

On success:

- `first-discovered`: stop immediately.
- `ordered-first`: retain enough state to know whether any lower ordinal may still succeed.

### 8.5 Worker loop

Worker sketch:

```rust
pub fn worker_loop<P>(
    id: usize,
    predicate: Arc<P>,
    job_rx: crossbeam_channel::Receiver<Candidate>,
    result_tx: crossbeam_channel::Sender<WorkerResult>,
    stop: Arc<AtomicBool>,
)
where
    P: CandidatePredicate,
{
    while !stop.load(Ordering::Acquire) {
        let candidate = match job_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(candidate) => candidate,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        };

        if stop.load(Ordering::Acquire) {
            break;
        }

        let started = Instant::now();
        let success = predicate.test(&candidate.text);
        let elapsed = started.elapsed();

        let _ = result_tx.send(WorkerResult {
            candidate,
            success,
            elapsed,
        });

        if success {
            stop.store(true, Ordering::Release);
            break;
        }
    }
}
```

Important caveat: for `ordered-first`, a worker setting `stop = true` immediately on any success may be too aggressive because lower ordinals may still be unresolved. In that mode, workers should report success but the controller should decide whether to stop. So either:

```text
FirstDiscovered: worker may set stop on success
OrderedFirst: only controller may set stop
```

or simpler:

```text
only controller sets stop in all modes
```

The second option is cleaner and less error-prone.

---

## 9. Ordered-first implementation details

For `ordered-first`, maintain:

```rust
struct OrderedState {
    lowest_unresolved_ordinal: u64,
    successful_candidates: BTreeMap<u64, Candidate>,
    false_ordinals: BTreeSet<u64>,
}
```

But storing every false ordinal can become large. Better: because ordinals are generated contiguously, track a moving frontier:

```rust
next_required_ordinal: u64
completed_false: BTreeSet<u64>
best_success: Option<Candidate>
```

When a false result arrives:

```rust
completed_false.insert(ordinal);
while completed_false.remove(&next_required_ordinal) {
    next_required_ordinal += 1;
}
```

When a success arrives:

```rust
best_success = min_by_ordinal(best_success, candidate)
```

The ordered result is decidable when:

```text
best_success.ordinal == next_required_ordinal
```

or, more generally:

```text
all ordinals lower than best_success.ordinal are known false
```

Once a best success exists, the feeder should stop generating candidates with ordinals greater than that success. There is no value in testing later candidates while earlier success is being validated.

---

## 10. Resume flow

### 10.1 Normal startup

```text
1. Load config.
2. Normalize config.
3. Compute config hash.
4. Create new enumerator.
5. Start workers.
6. Start controller loop.
```

### 10.2 Resume startup

```text
1. Load config.
2. Normalize config.
3. Load checkpoint.
4. Validate schema version.
5. Validate config hash.
6. Restore enumerator from checkpoint.enumerator.
7. Insert checkpoint.pending into pending map.
8. Enqueue pending candidates first.
9. Continue enumeration from restored enumerator.
```

### 10.3 Resume guarantee

The implementation must guarantee:

```text
resume does not skip candidates
```

It may allow:

```text
resume repeats candidates that were pending or completed after the last checkpoint
```

This is acceptable only if the predicate is pure.

---

## 11. CLI design

Recommended CLI:

```bash
candidate-search run \
  --config search.toml \
  --checkpoint checkpoint.json \
  --resume \
  --workers auto \
  --max-pending 4096 \
  --checkpoint-every 5s \
  --progress-every 1s \
  --semantics first-discovered
```

Useful options:

```text
--workers auto|N
--max-pending N
--checkpoint PATH
--resume
--no-checkpoint
--checkpoint-every DURATION
--progress-every DURATION
--semantics first-discovered|ordered-first
--result PATH
--log-level error|warn|info|debug|trace
--dry-run-generate N
--benchmark-workers 1,2,4,6,8,10
```

The `dry-run-generate` mode is valuable for testing feeder throughput without invoking the worker predicate.

Example:

```bash
candidate-search dry-run-generate --config search.toml --limit 1000000
```

---

## 12. Performance plan for M1 Pro

### 12.1 Build settings

Use release builds:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Optional release profile:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
panic = "abort"
```

Use `panic = "abort"` only if you do not need unwind-based recovery.

### 12.2 Worker count

Default:

```rust
let workers = std::thread::available_parallelism()?.get();
```

But expose `--workers`. Benchmark different counts.

Do not assume all cores are optimal. On Apple Silicon, depending on the M1 Pro variant and workload, using all cores may or may not beat using only performance cores. Measure candidate throughput and wall-clock time.

Benchmark matrix:

```text
workers:      1, 2, 4, 6, 8, 10
max_pending:  256, 1024, 4096, 16384
batch_size:   1 initially; later 8, 32, 128 if predicate is cheap
```

### 12.3 Avoid premature core pinning

Do not start with CPU affinity or thread pinning. macOS scheduling is good enough initially, and explicit pinning on macOS is awkward and easy to get wrong.

Only revisit affinity if profiling proves scheduler migration is a material bottleneck.

### 12.4 Allocation pressure

The current feeder allocates heavily because it builds many `Vec<char>` children.

Do not rewrite this before correctness and checkpointing are done. But once the engine works, benchmark these optimizations:

1. Use `Vec<u8>` instead of `Vec<char>` if the search domain is ASCII-only.
2. Use `SmallVec<[char; N]>` for short candidates.
3. Use faster hashers for internal maps, such as `ahash`, if input is trusted.
4. Batch jobs to reduce channel overhead.
5. Avoid building `String` until the candidate is actually dispatched to a worker.
6. Reuse buffers in neighbor generation if feasible.

The highest-impact possible change is likely switching from `Vec<char>` to `Vec<u8>` if Unicode is not required. But that is a semantic change, not a minor optimization. Make it deliberately.

---

## 13. Batching strategy

Start with one candidate per job. This is simpler and easier to checkpoint.

If the worker predicate is very fast, channel overhead may become significant. Then introduce batch jobs:

```rust
pub struct CandidateBatch {
    pub batch_id: u64,
    pub candidates: Vec<Candidate>,
}
```

Worker behavior:

```text
for candidate in batch:
  if stop requested, break
  test candidate
  if success, return success immediately
return all-false summary if none succeeded
```

Batch result:

```rust
pub enum BatchResult {
    AllFalse { batch_id: u64, ordinals: RangeInclusive<u64>, elapsed: Duration },
    Found { candidate: Candidate, elapsed: Duration },
}
```

Only use `RangeInclusive<u64>` if a batch always contains contiguous ordinals. The controller should enforce that.

Do not implement batching before basic single-candidate correctness tests pass.

---

## 14. Logging and metrics

Use `tracing`, not `println!` in worker hot paths.

Progress line every N seconds:

```text
elapsed=123.4s generated=10000000 tested=9870000 pending=4096 rate=81234/s workers=8 heap=123456 best_states=987654 checkpoint_age=4.2s
```

Recommended counters:

```rust
pub struct EngineMetrics {
    pub generated: u64,
    pub completed_false: u64,
    pub successes_seen: u64,
    pub pending: usize,
    pub worker_results: u64,
    pub checkpoint_writes: u64,
    pub checkpoint_write_errors: u64,
    pub last_checkpoint_ms: u128,
    pub candidates_per_sec: f64,
}
```

Expose feeder stats too:

```rust
enumerator.stats()
```

But avoid expensive heap or map size calculations on every progress tick. If needed, add cheap methods:

```rust
pub fn heap_len(&self) -> usize;
pub fn best_total_len(&self) -> usize;
pub fn finalized_global_len(&self) -> usize;
```

---

## 15. Failure handling

### 15.1 Predicate panic

Decide whether a predicate panic should abort the process or be caught.

Recommended first version:

```text
panic aborts the run
```

Reason: catching panics across worker threads complicates safety and may hide real bugs.

If the predicate may panic on malformed candidates, add:

```rust
std::panic::catch_unwind
```

and report a fatal engine error.

### 15.2 Checkpoint write failure

Recommended behavior:

- log the error,
- continue only if user explicitly allows `--checkpoint-best-effort`,
- otherwise stop cleanly with error.

For long expensive searches, silently running without checkpointing is a bad default.

### 15.3 Ctrl-C

Use `ctrlc` to set a shared atomic cancellation flag.

On cancellation:

```text
1. stop generating new candidates
2. let workers finish or stop quickly
3. write final checkpoint
4. exit with StopReason::Cancelled
```

Do not immediately terminate without checkpointing unless the user sends a second interrupt. Optional behavior:

```text
first Ctrl-C: graceful checkpoint and stop
second Ctrl-C: immediate exit
```

---

## 16. Detailed test plan

### 16.1 Existing feeder tests to keep

Keep all existing tests unless a semantic change is deliberate.

Current coverage is useful for:

- heap ordering,
- exact distance-one output,
- swap cost ordering,
- duplicate deletes,
- duplicate insertions,
- no-op replacement exclusion,
- keyboard neighbor cost,
- Unicode handling,
- alphabet normalization,
- distance modes,
- edge cases,
- matrix invariants,
- stats consistency.

### 16.2 New feeder tests

#### Test: ordinals are contiguous

```rust
#[test]
fn ordinals_are_contiguous() {
    let cfg = config("ab", vec!['a', 'b'], 0, 2, EditOps::all(), DistanceMode::PerDistanceBestCost);
    let got: Vec<_> = PipelinedOrderedCandidateEnumerator::new(cfg).unwrap().collect();

    for (expected, candidate) in got.iter().enumerate() {
        assert_eq!(candidate.ordinal, expected as u64);
    }
}
```

#### Test: snapshot resume matches uninterrupted run

```rust
#[test]
fn snapshot_resume_matches_uninterrupted_run() {
    let cfg = config("ab", vec!['a', 'b'], 0, 2, EditOps::all(), DistanceMode::PerDistanceBestCost);

    let full: Vec<_> = PipelinedOrderedCandidateEnumerator::new(cfg.clone())
        .unwrap()
        .map(|c| (c.ordinal, c.text, c.distance, c.cost))
        .collect();

    let mut e1 = PipelinedOrderedCandidateEnumerator::new(cfg.clone()).unwrap();
    let mut combined = Vec::new();

    for _ in 0..5 {
        let c = e1.next().unwrap();
        combined.push((c.ordinal, c.text, c.distance, c.cost));
    }

    let snapshot = e1.snapshot();
    let e2 = PipelinedOrderedCandidateEnumerator::from_snapshot(cfg, snapshot).unwrap();

    combined.extend(e2.map(|c| (c.ordinal, c.text, c.distance, c.cost)));

    assert_eq!(combined, full);
}
```

#### Test: snapshot preserves global mode finalization

```rust
#[test]
fn snapshot_preserves_global_mode_finalization() {
    let cfg = config("a", vec!['a', 'b'], 0, 2, EditOps::replace_only(), DistanceMode::GlobalMinimumDistance);

    let full: Vec<_> = PipelinedOrderedCandidateEnumerator::new(cfg.clone())
        .unwrap()
        .map(|c| c.text)
        .collect();

    let mut e1 = PipelinedOrderedCandidateEnumerator::new(cfg.clone()).unwrap();
    let first = e1.next().unwrap().text;
    let snapshot = e1.snapshot();

    let rest: Vec<_> = PipelinedOrderedCandidateEnumerator::from_snapshot(cfg, snapshot)
        .unwrap()
        .map(|c| c.text)
        .collect();

    let mut combined = vec![first];
    combined.extend(rest);

    assert_eq!(combined, full);
}
```

#### Test: config mismatch rejects snapshot

```rust
#[test]
fn config_mismatch_rejects_snapshot() {
    let cfg1 = config("ab", vec!['a', 'b'], 0, 2, EditOps::all(), DistanceMode::PerDistanceBestCost);
    let cfg2 = config("ab", vec!['a', 'c'], 0, 2, EditOps::all(), DistanceMode::PerDistanceBestCost);

    let e1 = PipelinedOrderedCandidateEnumerator::new(cfg1).unwrap();
    let snapshot = e1.snapshot();

    let restored = PipelinedOrderedCandidateEnumerator::from_snapshot(cfg2, snapshot);
    assert!(matches!(restored, Err(SnapshotError::ConfigHashMismatch { .. })));
}
```

#### Test: equivalent alphabet hashes identically

```rust
#[test]
fn equivalent_alphabet_hashes_identically() {
    let cfg1 = config("a", vec!['b', 'a', 'b'], 0, 1, EditOps::all(), DistanceMode::PerDistanceBestCost);
    let cfg2 = config("a", vec!['a', 'b'], 0, 1, EditOps::all(), DistanceMode::PerDistanceBestCost);

    assert_eq!(config_hash(&cfg1), config_hash(&cfg2));
}
```

#### Test: exhausted snapshot restores exhausted state

```rust
#[test]
fn exhausted_snapshot_restores_exhausted_state() {
    let cfg = config("a", vec![], 0, 0, EditOps::none(), DistanceMode::PerDistanceBestCost);

    let mut e1 = PipelinedOrderedCandidateEnumerator::new(cfg.clone()).unwrap();
    assert!(e1.next().is_some());
    assert!(e1.next().is_none());

    let snapshot = e1.snapshot();
    let mut e2 = PipelinedOrderedCandidateEnumerator::from_snapshot(cfg, snapshot).unwrap();
    assert!(e2.next().is_none());
}
```

### 16.3 Engine checkpoint tests

#### Test: resume does not skip pending jobs

Use a predicate that succeeds only for a candidate deliberately left pending.

Scenario:

```text
1. generate candidates up to N
2. mark candidate K as pending
3. write checkpoint
4. resume
5. assert candidate K is tested
```

Expected result:

```text
winner == candidate K
```

#### Test: pending false jobs may be re-run

Use a predicate that records invocation counts.

Expected:

```text
no skipped candidates
some repeated candidates allowed only if they were pending at checkpoint time
```

#### Test: checkpoint with no pending resumes exactly

```text
1. run until checkpoint where pending is empty
2. resume
3. compare sequence with uninterrupted run
```

#### Test: corrupted checkpoint fails cleanly

```rust
#[test]
fn corrupted_checkpoint_is_rejected() {
    // Write invalid JSON.
    // Attempt resume.
    // Assert typed checkpoint-read error.
}
```

#### Test: schema mismatch fails cleanly

```text
checkpoint.schema_version = 999
resume must reject it
```

### 16.4 Engine success semantics tests

#### Test: first-discovered may return later ordinal

Use a predicate that sleeps longer for lower ordinals and succeeds for two candidates.

```text
ordinal 10 succeeds after 200ms
ordinal 20 succeeds after 10ms
```

Expected in `first-discovered`:

```text
winner.ordinal == 20
```

#### Test: ordered-first returns lower ordinal

Same setup.

Expected in `ordered-first`:

```text
winner.ordinal == 10
```

This test is essential. Without it, the semantics are not really implemented.

### 16.5 Cancellation tests

#### Test: cancellation writes checkpoint

```text
1. start engine with slow predicate
2. trigger cancellation flag
3. assert checkpoint exists
4. resume from checkpoint
5. assert no candidates are skipped
```

#### Test: workers exit after stop

```text
1. predicate succeeds early
2. engine stops
3. join all worker threads
4. assert no worker thread remains alive
```

### 16.6 Backpressure tests

#### Test: pending never exceeds max_pending

Instrument controller state.

Expected:

```text
pending.len() <= max_pending
```

This is non-negotiable. Unbounded pending work defeats the memory model.

### 16.7 Property-style tests

For small configs, compare:

```text
single-thread uninterrupted result
parallel checkpoint/resume result
```

under predicates such as:

```rust
candidate.text == target
candidate.ordinal == target_ordinal
candidate.cost <= threshold && candidate.text.ends_with('x')
```

Run this matrix:

```text
workers: 1, 2, 4
semantics: first-discovered, ordered-first
checkpoint interval: very frequent
max_pending: 1, 2, 8, 64
modes: PerDistanceBestCost, GlobalMinimumDistance
```

---

## 17. Benchmark plan

### 17.1 Feeder-only benchmark

Measure generation rate without worker predicate.

```text
candidates/sec
heap size
best states
allocations if measured
```

### 17.2 Worker-only benchmark

Measure predicate cost on representative candidates.

```text
mean latency
p50 / p95 / p99 latency
CPU utilization
```

### 17.3 End-to-end benchmark

Matrix:

```text
workers:      1, 2, 4, 6, 8, 10
max_pending:  256, 1024, 4096, 16384
semantics:    first-discovered, ordered-first
checkpoint:   off, 5s JSON, 5s compressed
```

Capture:

```text
wall-clock time to success
candidates generated
candidates tested
candidates/sec
checkpoint write duration
RSS memory
CPU utilization
```

### 17.4 Determining bottleneck

Interpret results as follows:

```text
workers idle, queue empty      => feeder bottleneck
queue full, workers busy       => predicate bottleneck
CPU below expected             => synchronization, allocation, or IO bottleneck
checkpoint spikes              => snapshot too large or too frequent
```

---

## 18. Coding-agent constraints

These are hard constraints for implementation.

### 18.1 Must do

1. Implement orchestration in Rust.
2. Use bounded pending work.
3. Add candidate ordinals.
4. Add serde snapshot/restore for the feeder.
5. Add canonical config hashing.
6. Reject incompatible checkpoints.
7. Include pending jobs in engine checkpoints.
8. Write checkpoints atomically.
9. Implement graceful Ctrl-C cancellation.
10. Add tests proving resume does not skip candidates.
11. Add tests distinguishing `first-discovered` from `ordered-first`.
12. Keep the worker predicate pure from the engine's perspective.
13. Keep the feeder itself free of filesystem side effects.
14. Make worker count configurable.
15. Report throughput and checkpoint progress.

### 18.2 Must not do

1. Do not orchestrate workers in Bash.
2. Do not generate the whole search space in advance.
3. Do not use an unbounded channel.
4. Do not checkpoint only the last emitted string.
5. Do not checkpoint only the enumerator state after dispatching jobs unless pending jobs are also saved.
6. Do not let workers mutate checkpoint state.
7. Do not write checkpoint files in place.
8. Do not silently ignore checkpoint write failures.
9. Do not use `assert!` for user-facing checkpoint validation.
10. Do not log per candidate in the hot path.
11. Do not optimize away Unicode support unless the search domain is explicitly ASCII-only.
12. Do not claim earliest-candidate semantics when using first-discovered parallel termination.

---

## 19. Suggested implementation phases

### Phase 1: Refactor and preserve behavior

Goal: prepare the codebase without changing feeder semantics.

Tasks:

1. Move current feeder into `src/feeder.rs`.
2. Add `Cargo.toml` dependencies.
3. Add serde derives.
4. Add `Candidate.ordinal`.
5. Update existing tests to ignore or validate ordinal.
6. Add `SearchConfig::normalize()`.
7. Add canonical `config_hash()`.
8. Run all existing tests.

Acceptance criteria:

```text
cargo test passes
existing candidate order unchanged except Candidate now includes ordinal
```

### Phase 2: Feeder snapshot/restore

Goal: checkpoint the enumerator's graph frontier.

Tasks:

1. Add `EnumeratorSnapshot`.
2. Add `snapshot()`.
3. Add `from_snapshot()`.
4. Add `SnapshotError`.
5. Add tests for snapshot/resume equivalence.
6. Add config mismatch tests.
7. Add exhausted-state tests.
8. Add global-mode snapshot tests.

Acceptance criteria:

```text
snapshot + restore produces exactly the same remaining sequence as uninterrupted enumeration
```

### Phase 3: Controller and worker pool

Goal: run the predicate in parallel with bounded pending work.

Tasks:

1. Add `EngineConfig`.
2. Add worker loop.
3. Add controller loop.
4. Add result channel.
5. Add stop flag.
6. Add `first-discovered` semantics.
7. Add no-success exhaustion behavior.
8. Add basic progress metrics.

Acceptance criteria:

```text
parallel engine finds a known successful candidate
parallel engine exhausts cleanly when no candidate succeeds
all worker threads join cleanly
pending work never exceeds max_pending
```

### Phase 4: Engine checkpoint/resume

Goal: resume safely after interruption without skipping candidates.

Tasks:

1. Add `EngineCheckpoint`.
2. Add atomic checkpoint read/write.
3. Include `enumerator` snapshot and `pending` candidates.
4. On resume, enqueue pending jobs first.
5. Validate config hash.
6. Validate schema version.
7. Add Ctrl-C graceful shutdown.
8. Add resume tests with pending jobs.

Acceptance criteria:

```text
resume never skips pending candidates
incompatible checkpoints are rejected
corrupted checkpoints fail cleanly
Ctrl-C writes a usable checkpoint
```

### Phase 5: Ordered-first semantics

Goal: optionally return the earliest candidate by feeder ordinal.

Tasks:

1. Add `SuccessSemantics` enum.
2. Implement ordered result arbitration.
3. Stop generating candidates beyond known best success.
4. Add tests where first-discovered and ordered-first produce different winners.

Acceptance criteria:

```text
first-discovered returns fastest successful worker result
ordered-first returns lowest-ordinal successful candidate
```

### Phase 6: Benchmarking and optimization

Goal: tune for M1 Pro.

Tasks:

1. Add benchmark subcommand or Criterion benches.
2. Benchmark worker counts.
3. Benchmark `max_pending`.
4. Measure checkpoint write cost.
5. Identify feeder vs worker bottleneck.
6. Only then consider batching, faster hashers, `Vec<u8>`, or `SmallVec`.

Acceptance criteria:

```text
documented benchmark results identify recommended default workers and queue size for the target machine
```

---

## 20. Minimal controller pseudocode

This is a compact version suitable for the coding agent to expand.

```rust
pub fn run_search<P>(
    config: SearchConfig,
    engine_config: EngineConfig,
    predicate: Arc<P>,
) -> anyhow::Result<SearchReport>
where
    P: CandidatePredicate,
{
    let stop = Arc::new(AtomicBool::new(false));
    install_ctrlc_handler(stop.clone())?;

    let (job_tx, job_rx) = crossbeam_channel::bounded::<Candidate>(engine_config.max_pending);
    let (result_tx, result_rx) = crossbeam_channel::unbounded::<WorkerResult>();

    let mut workers = Vec::new();
    for id in 0..engine_config.workers {
        workers.push(spawn_worker(
            id,
            predicate.clone(),
            job_rx.clone(),
            result_tx.clone(),
            stop.clone(),
        ));
    }
    drop(result_tx);

    let mut state = if engine_config.resume {
        restore_controller_state(&engine_config.checkpoint_path, &config)?
    } else {
        ControllerState::new(PipelinedOrderedCandidateEnumerator::new(config)?)
    };

    enqueue_pending_first(&mut state, &job_tx)?;

    let started = Instant::now();
    let mut last_checkpoint = Instant::now();
    let mut last_progress = Instant::now();

    loop {
        while let Ok(result) = result_rx.try_recv() {
            state.handle_result(result, engine_config.success_semantics);
        }

        if state.should_stop(engine_config.success_semantics) {
            stop.store(true, Ordering::Release);
            break;
        }

        if stop.load(Ordering::Acquire) {
            break;
        }

        while state.pending.len() < engine_config.max_pending {
            match state.enumerator.next() {
                Some(candidate) => {
                    state.pending.insert(candidate.ordinal, candidate.clone());
                    if job_tx.send(candidate).is_err() {
                        stop.store(true, Ordering::Release);
                        break;
                    }
                }
                None => {
                    state.enumerator_exhausted = true;
                    break;
                }
            }
        }

        if state.enumerator_exhausted && state.pending.is_empty() {
            break;
        }

        if last_checkpoint.elapsed() >= engine_config.checkpoint_every {
            write_checkpoint_atomic(&engine_config.checkpoint_path, &state.to_checkpoint())?;
            last_checkpoint = Instant::now();
        }

        if last_progress.elapsed() >= engine_config.progress_every {
            emit_progress(&state, started.elapsed());
            last_progress = Instant::now();
        }

        match result_rx.recv_timeout(Duration::from_millis(5)) {
            Ok(result) => state.handle_result(result, engine_config.success_semantics),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    stop.store(true, Ordering::Release);
    drop(job_tx);

    for worker in workers {
        let _ = worker.join();
    }

    write_checkpoint_atomic(&engine_config.checkpoint_path, &state.to_checkpoint())?;

    Ok(state.to_report(started.elapsed(), engine_config.workers))
}
```

The real implementation should avoid processing a result twice. This pseudocode shows the control flow, not final polished code.

---

## 21. Risk register

### Risk 1: Checkpoints become huge

Cause:

```text
heap + best + finalized_global may contain many states
```

Mitigations:

- checkpoint every few seconds, not every candidate,
- use compact serialization after correctness is proven,
- measure checkpoint size,
- add warning when checkpoint exceeds threshold,
- consider sharded search later.

### Risk 2: Feeder becomes bottleneck

Cause:

```text
single-threaded graph expansion with many Vec<char> allocations
```

Mitigations:

- profile first,
- batch worker jobs,
- optimize representation,
- introduce sharded feeder only if necessary.

### Risk 3: Ordered-first semantics reduce performance

Cause:

```text
must wait for lower ordinals before accepting a success
```

Mitigation:

- default to first-discovered,
- make ordered-first explicit.

### Risk 4: Predicate is not pure

Cause:

```text
retries after resume can repeat side effects
```

Mitigation:

- require pure predicate,
- document at-least-once evaluation,
- if side effects are unavoidable, redesign with durable result logs.

### Risk 5: Unicode support costs too much

Cause:

```text
Vec<char> is larger and costlier than bytes
```

Mitigation:

- keep current Unicode-correct implementation initially,
- add ASCII-only mode later if valid for the real search domain.

---

## 22. Recommended acceptance criteria for the project

The project should be considered ready for real long-running searches only when all of the following are true:

1. Existing feeder tests pass.
2. New snapshot/restore tests pass.
3. Engine resume tests prove no skipped pending jobs.
4. Checkpoint mismatch tests reject incompatible configs.
5. Ctrl-C writes a usable checkpoint.
6. Worker threads exit cleanly after success, exhaustion, or cancellation.
7. `first-discovered` and `ordered-first` are both tested and documented.
8. Bounded pending work is enforced.
9. Progress output shows generated, tested, pending, throughput, and checkpoint status.
10. Benchmarks on the M1 Pro identify sensible defaults for workers and queue size.

---

## 23. Final recommendation

Build the system in Rust as a controller-driven, bounded, checkpointable worker pool around the existing feeder.

The current feeder should not be discarded. It should be upgraded with:

```text
serde derives
candidate ordinals
canonical config hashing
snapshot/restore
snapshot tests
```

The larger engine should then add:

```text
bounded work queue
pending-job tracking
atomic checkpoints
resume logic
first-discovered and ordered-first semantics
Ctrl-C handling
benchmarking
```

The main design constraint is simple:

```text
Never advance durable progress past work that has not been proven false.
```

The practical implementation of that rule is:

```text
checkpoint = enumerator snapshot + pending candidates
```

That gives you the right reliability/performance tradeoff for a high-throughput Rust search engine on your M1 Pro.
