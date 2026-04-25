# Handoff: String Neighborhood Search


## Goal

Build a simple but fast feeder/worker setup in Rust:

- The feeder generates a large search space of candidate strings.
- The worker is already a Rust function.
- Multiple CPU cores should be used to search in parallel.
- The run should stop as soon as the first candidate succeeds.
- Previously checked work should not be repeated on later runs.

The current direction is to keep the coordination in Rust rather than Bash or extra infrastructure.

## Current State

The feeder has already been refactored to support lazy iteration and in-memory checkpointing.

- `CandidateEnumerator` lazily yields candidates one at a time.
- `CandidateCheckpoint` captures enough state to resume an interrupted run without replaying already emitted candidates.
- `SearchCheckpointFile` serializes the search configuration and checkpoint state to disk and restores it later.
- The CLI now streams candidates instead of building a full `Vec<String>` first, and flushes each candidate when stdout is a terminal.
- The CLI also supports `--count` for reporting a closed-form count of the simplified insert/delete/replace model without listing candidates.
- The existing `enumerate_candidates(&SearchConfig)` API still exists as a compatibility wrapper that collects from the iterator.
- The test suite passes after these changes.

## Important Files

- [`src/search.rs`](/Users/mainar/dev/personal/b29/rust/src/search.rs)
  - Core search logic.
  - Contains `CandidateEnumerator`, `CandidateCheckpoint`, `SearchCheckpointFile`, and the compatibility wrapper.
- [`src/bin/enumerate.rs`](/Users/mainar/dev/personal/b29/rust/src/bin/enumerate.rs)
  - CLI entry point.
  - Currently streams output from the enumerator and supports `--count`.
- [`src/config.rs`](/Users/mainar/dev/personal/b29/rust/src/config.rs)
  - `SearchConfig` and `EnabledOperations`.
- [`src/mutations.rs`](/Users/mainar/dev/personal/b29/rust/src/mutations.rs)
  - One-edit neighborhood generation.
- [`src/lib.rs`](/Users/mainar/dev/personal/b29/rust/src/lib.rs)
  - Public re-exports.
- [`tests/search_tests.rs`](/Users/mainar/dev/personal/b29/rust/tests/search_tests.rs)
  - Behavior tests for ordering, deduplication, Unicode handling, and resume support.

## How The Search Works Today

`CandidateEnumerator` owns the search state and advances it layer by layer.

- It starts from the seed string.
- It tracks a `visited` set so candidates are not repeated.
- For each distance layer, it generates one-edit neighbors, deduplicates them, sorts them by likelihood, and emits them in deterministic order.
- It keeps the current layer and output index so the iteration can pause and resume within a layer.
- It is not fully incremental inside a distance layer. A large layer must be built and sorted before the first candidate in that layer can be emitted.

The checkpoint currently captures:

- whether enumeration is finished,
- the current distance,
- the output index within the current layer,
- the current layer contents,
- the visited set.

That is enough for in-memory resume.

`SearchCheckpointFile` now persists the same state together with the search configuration so a run can be restored from disk.

## What Is Still Missing

The next pieces are the ones that matter for the final feeder/worker integration:

1. Connect the enumerator to the worker function.
2. Add parallel execution so multiple candidates can be evaluated at once.
3. Stop the whole search immediately after the first success.
4. Decide how frequently to persist checkpoints during a long run.
5. Decide whether the count mode should stay simplified or grow into a model that matches the full enumerator, including swap and deduplication.
6. Decide whether to redesign layer generation if checkpoints need progress inside a layer build.

## Design Notes

The cleanest next step is usually:

- Keep candidate generation deterministic.
- Persist a compact checkpoint file.
- Run a bounded worker pool.
- Share a cancellation flag so the feeder and workers can stop quickly.

For this project, the simplest path is still preferred over adding a queueing system or process orchestration layer.

## Build And Test

From `/Users/mainar/dev/personal/b29/rust`:

```bash
cargo test
cargo run --bin enumerate -- --help
cargo run --bin enumerate -- abc --count
```

## Implementation Constraint

If you change the search state shape, keep the compatibility wrapper or update all call sites together. The existing tests and CLI still expect the older `enumerate_candidates` entry point to work.
