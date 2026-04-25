# Pipelined Ordered Candidate Enumerator

A Rust implementation of a streaming edit-distance neighborhood enumerator.

Candidates are emitted in strict **(distance, cost, lexical)** order — the same ordering contract as a traditional layer-sort enumerator — but without the blocking pause that comes from building and sorting an entire distance layer before emitting the first result. Work is distributed one parent-expansion per emission, so output starts immediately and stays responsive throughout.

## Build

```bash
cargo build --release
```

The binary is placed at:

```
target/release/enumerate
```

Run directly during development with:

```bash
cargo run --bin enumerate -- [OPTIONS] <SEED>
```

## Usage

```
enumerate [OPTIONS] <SEED>
```

`SEED` is the string to explore around. All other arguments are optional.

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--min <N>` | `1` | Minimum edit distance to emit (inclusive) |
| `--max <N>` | `1` | Maximum edit distance to emit (inclusive) |
| `--preset <PRESET>` | `lowercase` | Predefined alphabet (see below) |
| `--alphabet <CHARS>` | — | Custom alphabet string; overrides `--preset` |
| `--qwerty` | off | Enable QWERTY keyboard-neighbor likelihood scoring |
| `--limit <N>` | `0` | Stop after N candidates (0 = no limit) |
| `--mode <MODE>` | `per-distance` | Deduplication mode (see below) |
| `--verbose` | off | Print `distance`, `cost`, and `text` columns (tab-separated) |
| `--stats` | off | Print enumeration statistics to stderr after finishing |
| `--quiet` | off | Suppress the trailing "N candidates" line on stderr |

### Alphabet presets

| Preset | Contents | Size |
|--------|----------|------|
| `lowercase` | `a–z` | 26 |
| `letters` | `a–z`, `A–Z` | 52 |
| `letters-numbers` | `a–z`, `A–Z`, `0–9`, space | 63 |
| `letters-numbers-symbols` | letters, digits, space, common symbols | ~79 |
| `full-ascii` | All printable ASCII (32–126) | 95 |

Use `--alphabet` to pass an arbitrary character set:

```bash
enumerate abc --alphabet abcde
```

## Modes

The `--mode` flag controls how candidates that are reachable at multiple edit depths are handled. For most seeds and alphabets, the same string can be reached at distance 1 (e.g. delete a character) and again at distance 2 (e.g. delete then reinsert). The two modes differ in whether that second occurrence is emitted.

### `per-distance` (default)

Each candidate string may appear **once per distance layer**, using the best (lowest) accumulated cost path known at that depth. A string reachable at both distance 1 and distance 2 is emitted twice — once in each layer.

Use this mode when you need results that match a traditional layer-by-layer enumerator, or when the cost of reaching a string at each specific depth is meaningful to you.

```bash
enumerate a --min 0 --max 2 --alphabet ab --mode per-distance --verbose --quiet
```

```
0	0	a
1	2	
1	2	aa
1	2	ab
1	2	ba
1	3	b
2	3	ab      ← "ab" also appeared at distance 1
2	3	ba      ← "ba" also appeared at distance 1
2	4	a       ← "a" reappears at distance 2 (delete + reinsert, cost 4)
2	4	aaa
...
```

### `global-minimum`

Each candidate string is emitted **only once**, at its minimum reachable distance. Any path that reaches an already-emitted string at a later depth is suppressed.

Use this mode when you want a clean set of distinct candidate strings for downstream processing — for example, feeding a worker that should try each string exactly once.

```bash
enumerate a --min 0 --max 2 --alphabet ab --mode global-minimum --verbose --quiet
```

```
0	0	a
1	2	
1	2	aa
1	2	ab
1	2	ba
1	3	b
2	4	aaa
2	4	aab     ← strings like "ab", "ba", "a" are absent: already emitted at distance 1 or 0
2	4	aba
...
```

### Which mode to choose

| Situation | Recommended mode |
|-----------|-----------------|
| Feeding a worker that should try each string once | `global-minimum` |
| Studying how cost accumulates across depths | `per-distance` |
| Matching the output of a traditional layer-sort enumerator | `per-distance` |
| Minimising total number of emitted candidates | `global-minimum` |

## Likelihood costs

Edit costs reflect how likely a given mutation is as a typing error:

| Operation | Cost |
|-----------|------|
| Delete a character | 2 |
| Insert a character | 2 |
| Replace with a keyboard neighbor (`--qwerty`) | 1 |
| Replace with a non-neighbor character | 3 |
| Swap two adjacent distinct characters | 1 |

Candidates are sorted by accumulated cost within each distance layer, so keyboard-plausible typos surface before unlikely character replacements.

## Output format

By default, one candidate per line:

```
hello
helo
hlelo
```

With `--verbose`, columns are tab-separated `distance`, `cost`, `text`:

```
1	1	hlelo
1	2	ehllo
1	2	helo
```

The trailing status line is written to **stderr**, not stdout, so piped output contains only candidates:

```bash
enumerate patter --max 2 --quiet | wc -l
```

## Examples

### Distance-1 neighbors with a small alphabet

```bash
enumerate a --min 1 --max 1 --alphabet ab
```

```

aa
ab
ba
b
5 candidates
```

### Sorted by cost — swap before delete/insert/replace

```bash
enumerate ab --min 1 --max 1 --alphabet ab --verbose
```

```
1	1	ba
1	2	a
1	2	aab
1	2	aba
1	2	abb
1	2	b
1	2	bab
1	3	aa
1	3	bb
9 candidates
```

The swap (`ab` → `ba`, cost 1) sorts before all cost-2 and cost-3 results.

### Keyboard-aware scoring

```bash
enumerate patter --max 1 --qwerty --verbose | head -10
```

Keyboard neighbors of each character get cost 1, all other replacements get cost 3, so near-keyboard typos appear first.

### Distance-2 with stats

```bash
enumerate patter --max 2 --qwerty --stats --quiet | wc -l
```

```
51348
stats: popped=51442 stale_skipped=93 global_dup_skipped=0 expanded=336 raw_neighbors=124040 local_unique=121582 relaxed_new=51348 relaxed_improved=93 relaxed_not_better=70141 emitted=51348
```

### Show only distance-2 candidates

```bash
enumerate patter --min 2 --max 2 --qwerty --quiet | head -5
```

### Stop after a fixed number of results

```bash
enumerate patter --max 2 --limit 20
```

### Pipe-friendly (buffered output)

```bash
enumerate patter --max 2 --quiet > candidates.txt
```

When stdout is not a terminal the writer is automatically buffered. When stdout is a terminal each candidate is flushed immediately for responsive streaming.
