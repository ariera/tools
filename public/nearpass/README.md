# nearpass

A Rust tool for recovering forgotten KeePass passwords via edit-distance search.

Given a seed (your best guess at the password), `nearpass` enumerates every string within a bounded edit-distance neighborhood of that seed and tries each one against the database in parallel, stopping as soon as one opens it.

## Binaries

| Binary | Purpose |
|--------|---------|
| `crack` | Search a `.kdbx` database for a password near a seed |
| `enumerate` | Explore the candidate search space without a database |

Before running `crack` on a large search, it is worth running `enumerate` first to count how many candidates you are about to try:

```bash
# How many candidates does distance-2 with letters-numbers produce?
enumerate myseed --max 2 --preset letters-numbers --quiet | wc -l
```

If the number is in the millions, consider whether your search is actually feasible or whether a tighter preset or lower `--max` is more appropriate.

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

### Arguments and flags

| Argument / Flag | Default | Description |
|-----------------|---------|-------------|
| `<DB_PATH>` | — | Path to the `.kdbx` file |
| `<SEED>` | — | Your best guess at the password |
| `--min <N>` | `1` | Minimum edit distance to search (inclusive) |
| `--max <N>` | `2` | Maximum edit distance to search (inclusive) |
| `--preset <PRESET>` | `lowercase` | Predefined alphabet — see [Alphabet presets](#alphabet-presets). **Mutually exclusive with `--alphabet`** |
| `--alphabet <CHARS>` | — | Custom alphabet string. **Mutually exclusive with `--preset`** — passing both is an error |
| `--qwerty` | off | Weight keyboard-neighbor substitutions as cost 1 instead of cost 3 |
| `--mode <MODE>` | `per-distance` | Deduplication mode — see [Deduplication modes](#deduplication-modes) |
| `--strategy <S>` | `auto` | Candidate generation strategy — see [Strategies](#strategies) |
| `--workers <N>` | logical CPUs | Number of parallel worker threads |
| `--max-pending <N>` | `256` | Max candidates in-flight at once (bounds channel memory) |
| `--semantics <S>` | `first-discovered` | Stop condition — see [Success semantics](#success-semantics) |
| `--checkpoint <PATH>` | — | File for checkpoint/resume state. Enables periodic checkpointing when set |
| `--resume` | off | Resume from an existing checkpoint at `--checkpoint PATH`. **Requires `--checkpoint`** |
| `--checkpoint-every <N>` | `60` | Write a checkpoint every N **seconds** |
| `--progress-every <N>` | `10` | Print progress to stderr every N **seconds** |
| `--quiet` | off | Suppress all progress and summary output to stderr |

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

| Value | Behaviour | Checkpointable |
|-------|-----------|----------------|
| `first-discovered` (default) | Stops the moment any worker returns a hit. Fastest; may not return the closest match if multiple candidates succeed. | Yes |
| `ordered-first` | Stops only after confirming the lowest-cost hit across all candidates enumerated so far. Guarantees the result closest to the seed. Slower because workers may run past the first hit. | Yes |

**When to use `first-discovered`:** This is almost always the right choice. You just want the database open — it doesn't matter whether the winning password was edit-distance 1 or 2 from your seed. Use this unless you have a specific reason to care about which match is returned.

**When to use `ordered-first`:** Use this when you want to understand *how* your memory of the password was wrong. For example, if you want to confirm it was a single transposition rather than a substitution, `ordered-first` guarantees you get the candidate with the minimum edit cost, not just whichever worker happened to finish first. It is also useful when multiple near-identical candidates could succeed and you want the most likely one. Expect it to be meaningfully slower for large searches.

Both semantics are fully checkpointable. The semantics value is saved into the checkpoint file and **must match** on resume — passing a different `--semantics` when using `--resume` is an error.

### Checkpoint / resume

```bash
# Start a long search with checkpointing
./target/release/crack vault.kdbx hunter2 --max 3 --checkpoint ckpt.json

# Resume after interruption
./target/release/crack vault.kdbx hunter2 --max 3 --checkpoint ckpt.json --resume
```

`--resume` requires `--checkpoint`. On resume the engine validates that the seed, alphabet, distance bounds, and semantics all match the saved checkpoint; a mismatch is an error. Candidates that were in-flight at checkpoint time are re-tested (the KeePass predicate is pure, so this is safe).

The search configuration hash (covering seed, alphabet, distance bounds, and deduplication mode) is also saved and checked on resume — changing any of those flags alongside `--resume` will be rejected.

**When to use checkpointing:** Any search you expect to run for more than a few minutes. At `--max 3` with a realistic alphabet the candidate count is in the millions; a single Ctrl-C without a checkpoint discards all progress. Set `--checkpoint ckpt.json` by default for anything non-trivial, and reduce `--checkpoint-every` if you are on an unstable machine.

---

## `enumerate` — explore the search space

```
enumerate <SEED> [OPTIONS]
```

### Arguments and flags

| Flag | Default | Description |
|------|---------|-------------|
| `<SEED>` | — | Seed string to explore around |
| `--min <N>` | `1` | Minimum edit distance to emit (inclusive) |
| `--max <N>` | `1` | Maximum edit distance to emit (inclusive) |
| `--preset <PRESET>` | `lowercase` | Predefined alphabet — see [Alphabet presets](#alphabet-presets). **Mutually exclusive with `--alphabet`** |
| `--alphabet <CHARS>` | — | Custom alphabet string. **Mutually exclusive with `--preset`** — passing both is an error |
| `--qwerty` | off | Enable QWERTY keyboard-neighbor likelihood scoring |
| `--limit <N>` | `0` | Stop after N candidates (0 = no limit) |
| `--mode <MODE>` | `per-distance` | Deduplication mode — see [Deduplication modes](#deduplication-modes) |
| `--strategy <S>` | `auto` | Candidate generation strategy — see [Strategies](#strategies) |
| `--verbose` | off | Print `distance`, `cost`, `text` columns (tab-separated) |
| `--stats` | off | Print enumeration statistics to stderr after finishing |
| `--quiet` | off | Suppress the trailing "N candidates" line on stderr |

Candidates are emitted in strict **(distance, cost, lexical)** order without blocking to sort an entire layer first — output starts immediately.

### Examples

```bash
# All distance-1 neighbors of "patter" with keyboard scoring
enumerate patter --max 1 --qwerty --verbose | head -10

# Count distance-2 candidates before committing to a crack run
enumerate patter --max 2 --preset letters-numbers --quiet | wc -l

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

`--alphabet` and `--preset` are **mutually exclusive** — passing both is a hard error.

**Choosing a preset:** The candidate count grows roughly as `alphabet_size^distance`, so alphabet choice matters enormously. Start narrow. If you know your password used only lowercase letters, `lowercase` (26 chars) produces far fewer candidates than `letters-numbers` (63 chars) at the same distance. Widen the preset only after the narrow search comes up empty.

Use `--alphabet` when you have specific knowledge about the character set — for example, if you know your password was a word with one digit appended, `--alphabet abcdefghijklmnopqrstuvwxyz0123456789` avoids testing uppercase and symbols entirely.

```bash
enumerate abc --alphabet abcde
```

## Deduplication modes

### `per-distance` (default)

Each string may appear **once per distance layer** at the best (lowest) cost for that depth. A string reachable at both distance 1 and distance 2 is emitted twice — once in each layer.

**When to use:** Analysing the search space with `enumerate --verbose`, or any situation where you want to see a candidate's cost at each depth independently. Also the right choice when you are not concerned about trying the same candidate twice.

### `global-minimum`

Each string is emitted **only once**, at its minimum reachable distance. Later paths to an already-emitted string are suppressed.

**When to use:** Always use this when feeding `crack`, or when piping `enumerate` output to a downstream tool. It avoids retrying the same password, which is wasted work. Also required when using `--strategy streaming`.

## Strategies

| Value | Description | Constraints |
|-------|-------------|-------------|
| `auto` (default) | Uses `ordered-graph` for small searches, `streaming` for large ones | None |
| `ordered-graph` | Heap-based graph search; emits candidates in exact (distance, cost, lexical) order | None |
| `streaming` | Streaming DFS; bounded memory regardless of search size | Requires `--mode global-minimum`; swap operations are disabled |

**When to use `auto`:** Almost always. Let the tool pick.

**When to force `ordered-graph`:** If you need exact (distance, cost, lexical) ordering guaranteed regardless of search size, or if you are debugging enumeration behaviour and want the deterministic heap-based path. For small-to-medium searches it is also slightly faster than streaming because it avoids the DFS overhead.

**When to force `streaming`:** Only if you are hitting memory pressure during a very large search (high `--max`, large alphabet) and `auto` has not already chosen it. Streaming uses bounded memory at the cost of disabling swap-operation candidates and requiring `--mode global-minimum`. If memory is not a concern, prefer `auto`.

## Edit costs

| Operation | Cost |
|-----------|------|
| Delete a character | 2 |
| Insert a character | 2 |
| Replace with a keyboard neighbor (`--qwerty`) | 1 |
| Replace with a non-neighbor character | 3 |
| Swap two adjacent distinct characters | 1 |

Cost determines enumeration order: lower-cost candidates are tried first. Without `--qwerty`, all substitutions cost 3, so swaps (cost 1) come before deletions/insertions (cost 2), which come before substitutions (cost 3).

**When to use `--qwerty`:** When you typed the password on a US QWERTY keyboard and suspect the mistake was a finger-slip to an adjacent key — for example `passworf` instead of `password` (d/f are neighbours). With `--qwerty`, those keyboard-adjacent substitutions cost 1 (same as swaps), so they float to the front of the search and are tried before less-likely character replacements. Skip it if you are not confident the error was keyboard-related; it does not remove any candidates, it only reorders them.

## Test assets

`assets/qwerty.kdbx` is a KeePass 4 database created for smoke-testing. Password: `qwerty`.

```bash
./target/release/crack assets/qwerty.kdbx qwerty --min 0 --max 0
```
