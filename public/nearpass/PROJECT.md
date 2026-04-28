# Project
## Goal

Build a simple but fast feeder/worker setup in Rust:

- The feeder generates a large search space of candidate strings (see `pipelined/`)
- The worker tries each candidate as a KeePass database password
- Multiple CPU cores search in parallel
- The run stops as soon as the first candidate succeeds
- Previously checked work is not repeated on later runs

## Status: Complete

All core pieces are implemented in `pipelined/`:

| Piece | Status |
|---|---|
| Deterministic ordered candidate enumerator | ✅ `PipelinedOrderedCandidateEnumerator` |
| Worker predicate (KeePass) | ✅ `KeePassWorker` / `CandidatePredicate` |
| Parallel worker pool | ✅ crossbeam-channel, bounded jobs + unbounded results |
| Stop on first success | ✅ atomic stop flag, both semantics |
| Checkpoint / resume | ✅ atomic write, `--checkpoint` / `--resume` flags |
| CLI — enumerate | ✅ `enumerate` binary |
| CLI — crack | ✅ `crack` binary |

## Binaries

### `enumerate` — explore the search space

```
enumerate <seed> [--min N] [--max N] [--preset lowercase|letters|...] [--mode per-distance|global-minimum] [--verbose] [--stats]
```

### `crack` — run the parallel search

```
crack <db.kdbx> <seed> [--min N] [--max N] [--preset ...] [--workers N]
      [--semantics first-discovered|ordered-first]
      [--checkpoint path] [--resume]
      [--checkpoint-every N] [--progress-every N]
```

Exit codes: `0` = found (password printed to stdout), `1` = not found / cancelled, `2` = error.

## Success Semantics

- **`first-discovered`** (default) — stops the moment any worker returns true. Fastest.
- **`ordered-first`** — stops only once the lowest-ordinal (shortest edit path) success is confirmed. Guarantees the result closest to the seed.

## What Is Still Open

- **Count mode fidelity** (`enumerate --count`): the simplified counter in the old `rust/` crate does not model swap or deduplication. `pipelined/` does not have a count mode at all — if needed it could be added using the full enumerator.
- **End-to-end smoke test with a real `.kdbx` file**: see below.

## Smoke Test

```sh
# 1. Build
cd pipelined && cargo build --release

# 2. Create a test database with a known password one edit away from the seed
#    (requires keepassxc-cli or any KeePass GUI)
keepassxc-cli db-create --set-password test.kdbx   # enter "passw0rd"

# 3. Crack it — seed "password", distance 1, replace-only approximation
./target/release/crack test.kdbx password --min 1 --max 1 --preset lowercase
# Expected: prints "passw0rd", exits 0
```
