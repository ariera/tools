# String Neighborhood Search

This crate is a Rust implementation of a string search feeder. The goal is to generate a large space of candidate strings efficiently, feed them to a worker function, and stop as soon as one candidate succeeds.

For the fuller project context and current implementation state, start with [`HANDOFF.md`](/Users/mainar/dev/personal/b29/rust/HANDOFF.md).

## Current Shape

- The search code lives in `src/search.rs`.
- Candidate generation is now lazy through `CandidateEnumerator`.
- The enumerator can produce and restore a `CandidateCheckpoint`, so a run can be paused and resumed without rechecking already processed candidates.
- The CLI in `src/bin/enumerate.rs` now streams candidates instead of collecting the full result set first.

## What Is Still Missing

- Checkpoint persistence to disk.
- A small runner that saves progress and resumes from the last checkpoint on startup.
- Worker integration that consumes candidates until the first success.

## Status

The lazy enumerator and in-memory resume support are implemented and tested. The next step is to serialize checkpoints and wire the feeder into the worker loop.
