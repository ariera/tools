# Worker Adapter Design
**Date:** 2026-04-25  
**Status:** Approved

## Overview

Adapt the existing KeePass database worker from the research prototype into the pipelined crate as a reusable `CandidatePredicate` implementation that the Rust orchestrator can invoke with passwords from the feeder.

## Goals

1. Copy the worker prototype without modification
2. Wrap it to implement the `CandidatePredicate` trait
3. Enable the orchestrator to test passwords in parallel
4. Keep the worker pure and stateless (except for fixed database path)

## Architecture

### Module Structure
```
pipelined/src/
  ├── lib.rs
  ├── candidate_search.rs (existing enumerator)
  └── worker.rs (new)
      ├── pub struct KeePassWorker
      ├── impl CandidatePredicate for KeePassWorker
      └── Error types from research prototype
```

### KeePassWorker Adapter

```rust
pub struct KeePassWorker {
    db_path: PathBuf,
}

impl KeePassWorker {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }
}

impl CandidatePredicate for KeePassWorker {
    fn test(&self, candidate: &str) -> bool {
        can_open_database(&self.db_path, candidate)
    }
}
```

### Worker Interface

**Trait** (defined in main or a traits module):
```rust
pub trait CandidatePredicate: Send + Sync + 'static {
    fn test(&self, candidate: &str) -> bool;
}
```

**Contract:**
- Input: candidate string (password)
- Output: `true` if database opens, `false` otherwise
- Panics: not expected; `can_open_database` returns `Result` which is mapped to bool
- Thread-safety: `Send + Sync` required; worker holds immutable reference to path

### Error Handling

The existing `OpenError` enum maps all failures to `false`:
- `WrongPassword` → `false`
- `CorruptDatabase` → `false`
- `UnsupportedFormat` → `false`
- `Io(_)` → `false`
- `Other` → `false`

This is safe because:
1. Predicate is pure: same candidate always returns same result
2. Failures are at-least-once retestable (per the technical report)
3. Database state is read-only from the worker's perspective

### Invariants

- **Fixed path:** Database path is set once at worker creation, never changes
- **No side effects:** The predicate only reads; does not modify database or external state
- **Idempotent:** Retesting the same candidate after resume is safe
- **Thread-safe:** Multiple worker threads can share the same `KeePassWorker` via `Arc<KeePassWorker>`

## Source Code Origin

Worker logic is copied from:
```
/Users/mainar/dev/personal/research/keepass-secrets-vault-approaches/src/
```

Files to copy/adapt:
- `open.rs` → adapt into `worker.rs`
- `error.rs` → adapt error enum into `worker.rs`

## Integration Points

1. **Orchestrator setup** (future `engine.rs`):
   ```rust
   let worker = KeePassWorker::new(db_path);
   let worker = Arc::new(worker);
   // pass to spawn_worker() in controller loop
   ```

2. **Worker pool** (future `worker_loop` function):
   ```rust
   fn worker_loop<P>(
       predicate: Arc<P>,
       job_rx: ...,
       result_tx: ...,
   ) where
       P: CandidatePredicate,
   {
       while let Ok(candidate) = job_rx.recv_timeout(...) {
           let success = predicate.test(&candidate.text);
           result_tx.send(WorkerResult { candidate, success, ... });
       }
   }
   ```

## Testing Strategy

1. **Unit test:** `KeePassWorker::test()` on known passwords
2. **Property test:** true candidates stay true, false stay false across threads
3. **Integration:** orchestrator finds a known successful password using worker

## Non-Goals (Out of Scope)

- Modifying the research prototype code in place
- Supporting multiple worker types in this phase
- Custom error reporting per candidate
- Batching strategies

## Acceptance Criteria

1. Worker code copied and compiles
2. `KeePassWorker` implements `CandidatePredicate`
3. Fixed-path invariant is enforced by type (not comments)
4. Orchestrator can pass `Arc<KeePassWorker>` to worker threads
5. Unit tests pass; known passwords are found, unknown passwords fail
