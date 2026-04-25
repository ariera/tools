# Project
## Goal

Build a simple but fast feeder/worker setup in Rust:

- The feeder generates a large search space of candidate strings (see @pipelined/)
- The worker is already a Rust function.
- Multiple CPU cores should be used to search in parallel.
- The run should stop as soon as the first candidate succeeds.
- Previously checked work should not be repeated on later runs.

The current direction is to keep the coordination in Rust rather than Bash or extra infrastructure.


## What Is Still Missing

The next pieces are the ones that matter for the final feeder/worker integration:

0. bootstrap the worker code inspired for a separate project
1. Connect the enumerator to the worker function.
2. Add parallel execution so multiple candidates can be evaluated at once.
3. Stop the whole search immediately after the first success.
4. Decide how frequently to persist checkpoints during a long run.
5. Decide whether the count mode should stay simplified or grow into a model that matches the full enumerator, including swap and deduplication.

## Design Notes

The cleanest next step is usually:

- Keep candidate generation deterministic.
- Persist a compact checkpoint file.
- Run a bounded worker pool.
- Share a cancellation flag so the feeder and workers can stop quickly.
- Use `CandidateEnumerator::advance_work(budget)` in long-running feeder loops when you need bounded generation work between checkpoint saves.
- Use `DiscoveryCandidateEnumerator` or CLI `--discovery-order` when the user-facing requirement is continuous candidate streaming.

For this project, the simplest path is still preferred over adding a queueing system or process orchestration layer.

