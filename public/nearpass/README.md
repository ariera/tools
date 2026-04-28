# nearpass

A Rust tool for recovering forgotten KeePass passwords via edit-distance search.

Given a seed (your best guess at the password), `nearpass` enumerates every string within a bounded edit-distance neighborhood of that seed and tries each one against the database in parallel, stopping as soon as one opens it.

## Binaries

| Binary | Purpose |
|--------|---------|
| `crack` | Search a `.kdbx` database for a password near a seed |
| `enumerate` | Explore the candidate search space without a database |

## Build

```bash
cargo build --release
```

Binaries land at `target/release/crack` and `target/release/enumerate`.

Run during development with:

```bash
cargo run --bin crack -- [OPTIONS] <DB> <SEED>
cargo run --bin enumerate -- [OPTIONS] <SEED>
```

---

## `crack` — recover a KeePass password

```
crack <DB_PATH> <SEED> [OPTIONS]
```

| Argument / Flag | Default | Description |
|-----------------|---------|-------------|
| `<DB_PATH>` | — | Path to the `.kdbx` file |
| `<SEED>` | — | Your best guess at the password |
| `--min <N>` | `1` | Minimum edit distance to search (inclusive) |
| `--max <N>` | `2` | Maximum edit distance to search (inclusive) |
| `--preset <PRESET>` | `lowercase` | Predefined alphabet (see below) |
| `--alphabet <CHARS>` | — | Custom alphabet string; overrides `--preset` |
| `--qwerty` | off | Weight keyboard-neighbor substitutions as more likely |
| `--mode <MODE>` | `per-distance` | Deduplication mode (see below) |
| `--workers <N>` | logical CPUs | Number of parallel worker threads |
| `--max-pending <N>` | `256` | Max candidates in-flight at once |
| `--semantics <S>` | `first-discovered` | Stop condition (see below) |
| `--checkpoint <PATH>` | — | File for checkpoint/resume state |
| `--resume` | off | Resume from an existing checkpoint |
| `--checkpoint-every <N>` | — | Checkpoint after every N candidates tried |
| `--progress-every <N>` | — | Print progress to stderr every N candidates |

Exit codes: `0` = password found (printed to stdout), `1` = not found / cancelled, `2` = error.

### Quick example

```bash
# Test database (password: "qwerty") shipped in assets/
./target/release/crack assets/qwerty.kdbx qwerty --min 0 --max 0
# → qwerty
```

```bash
# More realistic: seed is close but not exact, search distance 1–2
./target/release/crack myvault.kdbx password --max 2 --preset letters-numbers --qwerty
```

### Success semantics

| Value | Behaviour |
|-------|-----------|
| `first-discovered` (default) | Stops the moment any worker returns a hit. Fastest. |
| `ordered-first` | Stops only after confirming the lowest-cost hit. Guarantees the result closest to the seed. |

### Checkpoint / resume

```bash
# Start a long search with checkpointing
./target/release/crack vault.kdbx hunter2 --max 3 --checkpoint ckpt.json

# Resume after interruption
./target/release/crack vault.kdbx hunter2 --max 3 --checkpoint ckpt.json --resume
```

---

## `enumerate` — explore the search space

```
enumerate <SEED> [OPTIONS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--min <N>` | `1` | Minimum edit distance to emit (inclusive) |
| `--max <N>` | `1` | Maximum edit distance to emit (inclusive) |
| `--preset <PRESET>` | `lowercase` | Predefined alphabet (see below) |
| `--alphabet <CHARS>` | — | Custom alphabet string; overrides `--preset` |
| `--qwerty` | off | Enable QWERTY keyboard-neighbor likelihood scoring |
| `--limit <N>` | `0` | Stop after N candidates (0 = no limit) |
| `--mode <MODE>` | `per-distance` | Deduplication mode (see below) |
| `--verbose` | off | Print `distance`, `cost`, `text` columns (tab-separated) |
| `--stats` | off | Print enumeration statistics to stderr |
| `--quiet` | off | Suppress the trailing "N candidates" line on stderr |

Candidates are emitted in strict **(distance, cost, lexical)** order without blocking to sort an entire layer first — output starts immediately.

### Examples

```bash
# All distance-1 neighbors of "patter" with keyboard scoring
enumerate patter --max 1 --qwerty --verbose | head -10

# Count distance-2 candidates
enumerate patter --max 2 --qwerty --quiet | wc -l

# Pipe to crack (global-minimum avoids retrying the same string)
enumerate vault-seed --max 2 --mode global-minimum --quiet > candidates.txt
```

---

## Alphabet presets

| Preset | Contents | Size |
|--------|----------|------|
| `lowercase` | `a–z` | 26 |
| `letters` | `a–z`, `A–Z` | 52 |
| `letters-numbers` | `a–z`, `A–Z`, `0–9`, space | 63 |
| `letters-numbers-symbols` | letters, digits, space, common symbols | ~79 |
| `full-ascii` | All printable ASCII (32–126) | 95 |

Use `--alphabet` for an arbitrary character set:

```bash
enumerate abc --alphabet abcde
```

## Deduplication modes

### `per-distance` (default)

Each string may appear **once per distance layer** at the best (lowest) cost for that depth. A string reachable at both distance 1 and distance 2 is emitted twice.

Use this when you need results that match a traditional layer-sort enumerator, or when the cost at each specific depth matters.

### `global-minimum`

Each string is emitted **only once**, at its minimum reachable distance. Later paths to an already-emitted string are suppressed.

Use this when feeding a worker that should try each candidate exactly once.

## Edit costs

| Operation | Cost |
|-----------|------|
| Delete a character | 2 |
| Insert a character | 2 |
| Replace with a keyboard neighbor (`--qwerty`) | 1 |
| Replace with a non-neighbor character | 3 |
| Swap two adjacent distinct characters | 1 |

## Test assets

`assets/qwerty.kdbx` is a KeePass 4 database created for smoke-testing. Password: `qwerty`.

```bash
./target/release/crack assets/qwerty.kdbx qwerty --min 0 --max 0
```
