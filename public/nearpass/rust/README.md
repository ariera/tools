# String Neighborhood Search

This crate is a Rust implementation of a string search feeder. The goal is to generate a large space of candidate strings efficiently, feed them to a worker function, and stop as soon as one candidate succeeds.

For the fuller project context and current implementation state, start with [`HANDOFF.md`](/Users/mainar/dev/personal/b29/rust/HANDOFF.md).

## Current Shape

- The search code lives in `src/search.rs`.
- Candidate generation is now lazy through `CandidateEnumerator`.
- The enumerator can produce and restore a `CandidateCheckpoint`, so a run can be paused and resumed without rechecking already processed candidates.
- `SearchCheckpointFile` can serialize that state and the search configuration to disk for later resumption.
- The CLI in `src/bin/enumerate.rs` now streams candidates instead of collecting the full result set first, and flushes each candidate when stdout is a terminal.
- The CLI also supports `--count` to print a closed-form count for the simplified insert/delete/replace model without emitting candidates.

## What Is Still Missing

- A small runner that saves progress and resumes from the last checkpoint on startup.
- Worker integration that consumes candidates until the first success.
- Finer-grained generation within a distance layer. The current enumerator must build, deduplicate, and sort each distance layer before emitting that layer.

## Status

The lazy enumerator, in-memory resume support, on-disk checkpoint serialization, and closed-form simplified count mode are implemented and tested. The next step is to wire the feeder into the worker loop and decide how checkpoint persistence should be scheduled during long runs.
