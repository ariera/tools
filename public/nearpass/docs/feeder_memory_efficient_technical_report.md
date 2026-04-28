# Technical Report and Implementation Plan: Memory-Efficient Feeder Enumeration

## 1. Scope

This report analyzes the uploaded Rust feeder library file `lib.rs` and proposes a memory-efficient implementation plan for large edit-distance searches such as:

```text
seed length:      8 characters
edit distance:   exactly 3, or range including 3
alphabet:        alphanumeric plus special symbols, commonly around 62 to 94 characters
operation set:   delete, insert, replace, optionally swap
worker:          password-candidate consumer, likely KeePass-oriented
```

Only `lib.rs` was available in the upload. The `worker` and `engine` modules are referenced by `lib.rs`, but their source files were not included in the uploaded data. The engine and worker sections below are therefore integration guidance based on the public exports and the feeder design visible in `lib.rs`.

The main recommendation is to stop using the current heap/hash-map graph search for large searches. Keep it for small exact tests and exact ordered mode, but add a streaming edit-distance enumerator for production-sized searches. The streaming enumerator should generate one candidate, hand it to the worker, and then discard it.

Target memory behavior:

```text
Current graph search:     O(number_of_reachable_candidates)
Proposed streaming mode:  O(seed_length * max_distance + max_candidate_length * seed_length + queue_size)
```

For the problematic case, this is the difference between potentially many gigabytes and a small, bounded amount of memory.

---

## 2. Executive summary

The feeder currently implements a layered weighted graph search. Each string reached by edits is stored in multiple places:

- `BinaryHeap<QueueItem>` frontier.
- `best: Vec<HashMap<Vec<char>, u32>>` for best cost by depth and candidate.
- `finalized_global: HashSet<Vec<char>>` in `GlobalMinimumDistance` mode.
- `Candidate { text: String, chars: Vec<char>, ... }` duplicates the emitted string representation.
- `snapshot()` clones the entire heap, all best maps, and the finalized set.

This design is exact and well tested for small spaces, but it is not appropriate for an 8-character seed at edit distance 3 over a large alphabet. The number of candidates is genuinely huge. For example, with a 94-character alphabet, replacements alone at exactly distance 3 create:

```text
C(8, 3) * (94 - 1)^3
= 56 * 93^3
= 45,043,992 candidates
```

That excludes insertions, deletions, swaps, and mixed edit scripts. It is enough by itself to exhaust memory when each candidate may be stored as owned `Vec<char>` keys in hash maps and heap entries.

The recommended architecture is:

1. Add a new `StreamingLevenshteinCandidateEnumerator` for large searches.
2. Use it only with minimum-distance semantics, equivalent to `DistanceMode::GlobalMinimumDistance`.
3. Initially support insert, delete, and replace exactly; treat swap separately.
4. Preserve the current `PipelinedOrderedCandidateEnumerator` for small exact ordered searches and regression tests.
5. Add a memory guard so the graph enumerator cannot accidentally run huge searches.
6. Ensure the engine uses a bounded channel or bounded batch size so the feeder cannot outrun the worker and accumulate candidates.
7. Avoid full-frontier snapshots for large searches; streaming snapshots should store only a DFS stack, current prefix, current target length, and ordinal.

---

## 3. Current implementation analysis

### 3.1 Relevant structures in `lib.rs`

The uploaded code contains these key structures:

```rust
pub enum DistanceMode {
    PerDistanceBestCost,
    GlobalMinimumDistance,
}

pub struct Candidate {
    pub ordinal: u64,
    pub text: String,
    pub chars: Vec<char>,
    pub distance: usize,
    pub cost: u32,
}

struct QueueItem {
    depth: usize,
    cost: u32,
    chars: Vec<char>,
}

pub struct PipelinedOrderedCandidateEnumerator {
    config: SearchConfig,
    heap: BinaryHeap<QueueItem>,
    best: Vec<HashMap<Vec<char>, u32>>,
    finalized_global: HashSet<Vec<char>>,
    stats: EnumeratorStats,
}
```

Important code locations in the uploaded file:

```text
Candidate struct:                         around line 95
QueueItem struct:                         around line 105
EnumeratorSnapshot:                       around line 236
PipelinedOrderedCandidateEnumerator:      around line 250
snapshot():                               around line 345
one_edit_neighbors():                     around line 396
relax():                                  around line 456
Iterator implementation:                  around line 491
```

### 3.2 Current graph model

The current enumerator models the search as a layered directed graph.

A graph node is:

```text
(depth, candidate_string)
```

An edge is one edit operation:

```text
(depth, parent_string) -> (depth + 1, child_string)
```

The edge has a weighted cost:

```text
swap:                         1
keyboard-neighbor replace:    1
delete:                       2
insert:                       2
normal replace:               3
```

The heap is ordered by:

```text
depth ascending,
then cost ascending,
then lexical candidate ascending
```

This is visible in `impl Ord for QueueItem`, which reverses ordering because Rust's `BinaryHeap` is a max heap.

### 3.3 Deduplication behavior

There are two modes:

#### `PerDistanceBestCost`

A candidate can appear once per depth. This mode intentionally allows the same candidate string to reappear at a later edit depth.

Example already tested in the uploaded code:

```rust
// seed: "a", replace-only, alphabet ['a', 'b'], distance 0..2
// output includes:
// ("a", distance 0, cost 0)
// ("b", distance 1, cost 3)
// ("a", distance 2, cost 6)
```

This is expensive and usually not desirable for a password worker, because testing the same password multiple times wastes CPU.

#### `GlobalMinimumDistance`

A candidate string is emitted only once, at its first reachable depth. This is closer to what a password-recovery feeder should do.

However, the current implementation still stores large amounts of state to discover and suppress duplicates.

### 3.4 Branching factor

For a parent string of length `L` and alphabet size `A`, `one_edit_neighbors()` can generate approximately:

```text
delete:    L
insert:    (L + 1) * A
replace:   L * (A - 1)
swap:      L - 1
```

For `L = 8` and `A = 94`:

```text
delete:    8
insert:    9 * 94  = 846
replace:   8 * 93  = 744
swap:      7
raw total: 1605 one-edit neighbors from the seed
```

At depth 3, the number of reachable strings is not `1605^3` because of deduplication and collisions, but it is still very large. Replacement-only at exact distance 3 is already 45,043,992 candidates.

### 3.5 Why the crash happens

The crash is not a small bug; it is a consequence of the algorithmic shape.

For `min_distance = max_distance = 3`, the iterator cannot emit depth-3 candidates until it has popped and expanded the lower-depth frontier. During that process it keeps inserting depth-3 states into `best[3]` and `heap`.

The memory pressure is severe because each candidate string is owned repeatedly:

```text
best[depth] key:             Vec<char>
heap item:                   Vec<char>
finalized_global key:        Vec<char>, in global mode
local neighbor map key:      Vec<char>, temporarily per expansion
Candidate output:            String plus Vec<char>
snapshot, if called:         clones nearly everything
```

`Vec<char>` is especially expensive for mostly ASCII password candidates because a Rust `char` is 4 bytes, and each `Vec` has its own allocation and capacity metadata. Hash-map and heap overhead add substantially more.

A rough lower-bound memory estimate for replacement-only distance-3 candidates with a 94-character alphabet:

```text
45,043,992 strings * 100 bytes/string = about 4.5 GB
45,043,992 strings * 200 bytes/string = about 9.0 GB
```

Those figures are plausible before including insertions, mixed operations, allocator fragmentation, the heap duplicate, snapshot cloning, or worker buffering.

---

## 4. Design goals

### 4.1 Functional goals

The new feeder mode should:

1. Generate candidate strings within a configured edit-distance range.
2. Use `GlobalMinimumDistance` semantics: emit each candidate at most once, at its minimum edit distance.
3. Support delete, insert, and replace exactly in the first implementation.
4. Preserve weighted replacement cost enough to report `Candidate.cost` for a minimum-edit path.
5. Remain deterministic across runs for the same config.
6. Work with the existing worker without requiring the worker to know the generation algorithm.
7. Keep the existing exact ordered graph enumerator for small tests, exact ordering, snapshots, and compatibility.

### 4.2 Non-goals for the first streaming implementation

Do not attempt these in the first implementation:

1. Do not preserve the exact current output order of `depth, cost, lexical` for huge searches.
2. Do not support `PerDistanceBestCost` in streaming mode.
3. Do not support exact adjacent-swap semantics in the first streaming implementation.
4. Do not implement disk-backed external sorting unless exact ordering is explicitly required.
5. Do not use a global `HashSet` for all emitted candidates in the large-search path.

### 4.3 Hard constraints for the coding agent

The coding agent should follow these constraints strictly:

- Do not call `.collect()` on a large candidate iterator in production paths.
- Do not store all generated candidates in `Vec`, `HashMap`, `HashSet`, `BinaryHeap`, or any unbounded queue.
- Do not silently ignore `ops.swap`; either reject streaming mode with a clear error, route to a documented approximate/hybrid swap mode, or use the exact graph mode only when the estimate is safe.
- Do not silently change `DistanceMode::PerDistanceBestCost` to global semantics. If streaming is requested with `PerDistanceBestCost`, return an explicit unsupported-mode error.
- Do not create snapshots by cloning a huge frontier in the large-search path.
- Do not duplicate candidate storage as both `String` and `Vec<char>` unless compatibility requires it at the public boundary.
- Do not let the feeder outrun the worker through an unbounded channel.
- Do use deterministic alphabet normalization.
- Do keep all current small-case tests passing for the existing graph enumerator.
- Do add cross-check tests comparing the streaming enumerator to the graph enumerator on small no-swap configurations.

---

## 5. Proposed architecture

### 5.1 Keep the existing graph enumerator, but make it safe

Keep:

```rust
PipelinedOrderedCandidateEnumerator
```

Use it for:

- Small exact tests.
- Existing snapshot/restore tests.
- Exact current ordering.
- Configurations where the estimated state count is below a conservative threshold.

Add a guard so large searches cannot accidentally use it:

```rust
pub enum EnumeratorStrategy {
    Auto,
    OrderedGraph,
    StreamingLevenshtein,
}
```

Add a factory:

```rust
pub enum CandidateEnumerator {
    Ordered(PipelinedOrderedCandidateEnumerator),
    Streaming(StreamingLevenshteinCandidateEnumerator),
}

impl Iterator for CandidateEnumerator {
    type Item = Candidate;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Ordered(e) => e.next(),
            Self::Streaming(e) => e.next(),
        }
    }
}
```

Recommended behavior:

```text
strategy = OrderedGraph:
    run the existing enumerator only if the estimate is below a configured hard limit;
    otherwise return ConfigError::EstimatedSearchTooLarge.

strategy = StreamingLevenshtein:
    require DistanceMode::GlobalMinimumDistance;
    require ops.swap == false for MVP;
    then use the streaming enumerator.

strategy = Auto:
    if safe for graph, use graph;
    otherwise, if compatible with streaming, use streaming;
    otherwise return a clear error explaining which config field blocks streaming.
```

### 5.2 Add a streaming edit-distance enumerator

Add:

```rust
pub struct StreamingLevenshteinCandidateEnumerator {
    config: SearchConfig,
    seed: Vec<char>,
    raw_alphabet: HashSet<char>,
    generation_alphabet: Vec<char>,
    min_distance: usize,
    max_distance: usize,
    min_target_len: usize,
    max_target_len: usize,
    target_len: usize,
    prefix: Vec<char>,
    stack: Vec<StreamingFrame>,
    emitted: u64,
    stats: StreamingEnumeratorStats,
}

struct StreamingFrame {
    row: Vec<Score>,
    next_symbol_index: usize,
}
```

This is an iterative depth-first search over candidate prefixes. It does not store previously emitted candidates because each candidate string has exactly one path through the prefix tree.

The candidate prefix tree is not the edit graph. It is simply:

```text
root = empty string
edge = append one character from generation_alphabet
leaf = one complete candidate string of target_len
```

At each prefix, keep a dynamic-programming row that represents the cheapest way to transform a prefix of the seed into the current candidate prefix.

### 5.3 Candidate universe

The current graph search can produce strings containing original seed characters even if those characters are not present in `config.alphabet`, because unchanged seed characters survive deletes/replaces/inserts around them.

Therefore, for exact insert/delete/replace streaming semantics, use:

```text
generation_alphabet = sorted_dedup(config.alphabet union seed.chars())
```

But operation legality must still use the raw configured alphabet:

```text
insert candidate char ch:     allowed only if ch is in config.alphabet
replace seed char -> ch:      allowed only if ch is in config.alphabet
match seed char == ch:        allowed even if ch is not in config.alphabet
```

This preserves the semantics of the current graph for insert/delete/replace while still generating all possible reachable strings.

### 5.4 Target length bounds

For seed length `n` and max distance `d`:

```rust
let min_target_len = if config.ops.delete {
    n.saturating_sub(d)
} else {
    n
};

let max_target_len = if config.ops.insert {
    n + d
} else {
    n
};
```

If both insert and delete are disabled, the only possible target length is `n`.

### 5.5 Weighted minimum-distance score

The current graph tracks two concepts:

```text
depth = number of edit operations
cost  = weighted cost of those operations
```

The streaming mode should compute, for every prefix pair, the best score ordered by:

```text
edit count first, then weighted cost
```

That gives the minimum edit distance and the cheapest cost among minimum-edit scripts.

Use a score type:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Score {
    edits: u16,
    cost: u32,
}

impl Score {
    fn zero() -> Self {
        Self { edits: 0, cost: 0 }
    }

    fn inf() -> Self {
        Self { edits: u16::MAX, cost: u32::MAX }
    }

    fn add(self, edit_delta: u16, cost_delta: u32, max_distance: usize) -> Self {
        if self.edits == u16::MAX {
            return Self::inf();
        }

        let edits = self.edits.saturating_add(edit_delta);
        if edits as usize > max_distance {
            return Self::inf();
        }

        Self {
            edits,
            cost: self.cost.saturating_add(cost_delta),
        }
    }
}

fn better(a: Score, b: Score) -> Score {
    if (a.edits, a.cost) <= (b.edits, b.cost) { a } else { b }
}
```

Operation costs should stay consistent with the current graph search:

```rust
const DELETE_COST: u32 = 2;
const INSERT_COST: u32 = 2;
const SWAP_COST: u32 = 1;
const KEYBOARD_REPLACE_COST: u32 = 1;
const NORMAL_REPLACE_COST: u32 = 3;
```

`SWAP_COST` is listed for consistency, but swap is not part of the first streaming DP implementation.

---

## 6. Streaming dynamic-programming algorithm

### 6.1 Initial row

The initial row represents converting seed prefixes to the empty candidate prefix.

```rust
fn initial_row(seed_len: usize, config: &SearchConfig) -> Vec<Score> {
    let mut row = vec![Score::inf(); seed_len + 1];
    row[0] = Score::zero();

    for j in 1..=seed_len {
        if config.ops.delete {
            row[j] = row[j - 1].add(1, DELETE_COST, config.max_distance);
        }
    }

    row
}
```

### 6.2 Row transition for one appended candidate character

This computes the next DP row after appending `ch` to the candidate prefix.

```rust
fn step_row(
    seed: &[char],
    prev: &[Score],
    ch: char,
    ch_is_in_raw_alphabet: bool,
    config: &SearchConfig,
) -> Vec<Score> {
    let n = seed.len();
    let mut curr = vec![Score::inf(); n + 1];

    // Insert candidate char ch while matching an empty seed prefix.
    if config.ops.insert && ch_is_in_raw_alphabet {
        curr[0] = prev[0].add(1, INSERT_COST, config.max_distance);
    }

    for j in 1..=n {
        let mut best = Score::inf();

        // Delete seed[j - 1].
        if config.ops.delete {
            best = better(best, curr[j - 1].add(1, DELETE_COST, config.max_distance));
        }

        // Match or replace seed[j - 1] with ch.
        if seed[j - 1] == ch {
            best = better(best, prev[j - 1]);
        } else if config.ops.replace && ch_is_in_raw_alphabet {
            let replace_cost = if is_keyboard_neighbor(config, seed[j - 1], ch) {
                KEYBOARD_REPLACE_COST
            } else {
                NORMAL_REPLACE_COST
            };
            best = better(best, prev[j - 1].add(1, replace_cost, config.max_distance));
        }

        // Insert candidate char ch while seed prefix length stays j.
        if config.ops.insert && ch_is_in_raw_alphabet {
            best = better(best, prev[j].add(1, INSERT_COST, config.max_distance));
        }

        curr[j] = best;
    }

    curr
}
```

### 6.3 Lower-bound pruning

A prefix can be abandoned if no completion can reach `max_distance`.

This pruning uses a safe lower bound based on the remaining seed length and remaining candidate length. It may keep some impossible branches, but it must never discard a valid candidate.

```rust
fn length_lower_bound(
    remaining_seed: usize,
    remaining_candidate: usize,
    ops: EditOps,
) -> Option<usize> {
    match (ops.insert, ops.delete) {
        (true, true) => Some(remaining_seed.abs_diff(remaining_candidate)),

        // Without delete, the remaining candidate must be at least as long as
        // the remaining seed; extra candidate chars can be insertions.
        (true, false) => {
            if remaining_candidate >= remaining_seed {
                Some(remaining_candidate - remaining_seed)
            } else {
                None
            }
        }

        // Without insert, the remaining seed must be at least as long as the
        // remaining candidate; extra seed chars can be deletions.
        (false, true) => {
            if remaining_seed >= remaining_candidate {
                Some(remaining_seed - remaining_candidate)
            } else {
                None
            }
        }

        // Without insert or delete, lengths must match exactly.
        (false, false) => {
            if remaining_seed == remaining_candidate { Some(0) } else { None }
        }
    }
}

fn can_still_reach(
    row: &[Score],
    seed_len: usize,
    remaining_candidate: usize,
    ops: EditOps,
    max_distance: usize,
) -> bool {
    row.iter().enumerate().any(|(j, score)| {
        if score.edits == u16::MAX {
            return false;
        }

        let remaining_seed = seed_len - j;
        let Some(lb) = length_lower_bound(remaining_seed, remaining_candidate, ops) else {
            return false;
        };

        score.edits as usize + lb <= max_distance
    })
}
```

### 6.4 Leaf emission

At a leaf, the final edit distance and cost are stored in:

```rust
let final_score = row[seed.len()];
```

Emit only if:

```rust
min_distance <= final_score.edits <= max_distance
```

Candidate construction:

```rust
let text: String = prefix.iter().collect();

Candidate {
    ordinal,
    text,
    chars: prefix.clone(),
    distance: final_score.edits as usize,
    cost: final_score.cost,
}
```

Compatibility note: `chars: prefix.clone()` preserves the current public `Candidate` API. Later, consider replacing `Candidate` with a lighter type that does not store both `String` and `Vec<char>`.

### 6.5 Iterative DFS skeleton

Use an iterative stack rather than recursion so snapshots are easy and stack overflow is impossible.

```rust
impl StreamingLevenshteinCandidateEnumerator {
    fn init_target_len(&mut self) {
        self.prefix.clear();
        self.stack.clear();
        self.stack.push(StreamingFrame {
            row: initial_row(self.seed.len(), &self.config),
            next_symbol_index: 0,
        });
    }

    fn backtrack_one(&mut self) {
        self.stack.pop();
        if !self.prefix.is_empty() {
            self.prefix.pop();
        }
    }
}

impl Iterator for StreamingLevenshteinCandidateEnumerator {
    type Item = Candidate;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.target_len > self.max_target_len {
                return None;
            }

            if self.stack.is_empty() {
                self.target_len += 1;
                if self.target_len > self.max_target_len {
                    return None;
                }
                self.init_target_len();
            }

            if self.prefix.len() == self.target_len {
                let row = &self.stack.last().unwrap().row;
                let final_score = row[self.seed.len()];

                self.backtrack_one();

                let edits = final_score.edits as usize;
                if final_score.edits != u16::MAX
                    && edits >= self.min_distance
                    && edits <= self.max_distance
                {
                    let ordinal = self.emitted;
                    self.emitted += 1;

                    let text: String = self.prefix.iter().collect();

                    // Important: in real code, capture the prefix before backtracking.
                    // This skeleton intentionally shows the control flow; the final
                    // implementation should clone the leaf prefix before backtrack_one().
                    return Some(Candidate {
                        ordinal,
                        text,
                        chars: self.prefix.clone(),
                        distance: edits,
                        cost: final_score.cost,
                    });
                }

                continue;
            }

            let top = self.stack.last_mut().unwrap();
            if top.next_symbol_index >= self.generation_alphabet.len() {
                self.backtrack_one();
                continue;
            }

            let ch = self.generation_alphabet[top.next_symbol_index];
            top.next_symbol_index += 1;

            let parent_row = &top.row;
            let ch_is_in_raw_alphabet = self.raw_alphabet.contains(&ch);
            let next_row = step_row(
                &self.seed,
                parent_row,
                ch,
                ch_is_in_raw_alphabet,
                &self.config,
            );

            let remaining_after_push = self.target_len - self.prefix.len() - 1;
            if !can_still_reach(
                &next_row,
                self.seed.len(),
                remaining_after_push,
                self.config.ops,
                self.max_distance,
            ) {
                continue;
            }

            self.prefix.push(ch);
            self.stack.push(StreamingFrame {
                row: next_row,
                next_symbol_index: 0,
            });
        }
    }
}
```

The skeleton above intentionally flags one subtle bug: do not backtrack before cloning the leaf prefix. A correct implementation should capture `leaf_prefix = self.prefix.clone()` before `backtrack_one()` and build `Candidate` from `leaf_prefix`.

Correct leaf section:

```rust
if self.prefix.len() == self.target_len {
    let row = &self.stack.last().unwrap().row;
    let final_score = row[self.seed.len()];
    let leaf_prefix = self.prefix.clone();

    self.backtrack_one();

    let edits = final_score.edits as usize;
    if final_score.edits != u16::MAX
        && edits >= self.min_distance
        && edits <= self.max_distance
    {
        let ordinal = self.emitted;
        self.emitted += 1;
        let text: String = leaf_prefix.iter().collect();

        return Some(Candidate {
            ordinal,
            text,
            chars: leaf_prefix,
            distance: edits,
            cost: final_score.cost,
        });
    }

    continue;
}
```

---

## 7. Ordering semantics

The current graph enumerator guarantees:

```text
distance ascending,
then weighted cost ascending,
then lexical candidate ascending
```

The streaming DP enumerator should not promise this order. It should promise:

```text
deterministic order,
each candidate at most once,
minimum-distance semantics,
bounded memory
```

A practical deterministic order is:

```text
target length ascending,
then prefix-tree lexical order by normalized generation_alphabet
```

If distance-first order is desired without storing everything, the streaming enumerator can loop over accepted distance bands:

```text
for wanted_distance in min_distance..=max_distance:
    run streaming traversal and emit only candidates whose final distance == wanted_distance
```

This uses bounded memory but repeats work. It still does not sort by weighted cost unless candidates are externally sorted.

### Exact order option: external sorting

If exact current order is mandatory for a large run, the memory-efficient solution is disk-backed external sorting:

1. Stream candidates into bounded-size chunks.
2. Sort each chunk by `(distance, cost, chars)`.
3. Write sorted runs to disk.
4. K-way merge the sorted runs.
5. Deduplicate during merge.

This can preserve ordering while bounding RAM, but it uses substantial disk space and is slower. For a password worker, this is usually the wrong tradeoff because immediate streaming keeps the worker busy and avoids huge intermediate files.

---

## 8. Swap handling

The uploaded config supports adjacent swaps:

```rust
pub struct EditOps {
    pub delete: bool,
    pub insert: bool,
    pub replace: bool,
    pub swap: bool,
}
```

The current graph search handles swap as a one-edit neighbor with cost 1.

### Recommended MVP behavior

For `StreamingLevenshteinCandidateEnumerator` MVP:

```text
if config.ops.swap == true:
    return ConfigError::UnsupportedForStreaming { reason: "swap is not supported by streaming MVP" }
```

Do not silently ignore swap.

### Why not implement swap immediately?

Adjacent transposition can be added to edit-distance DP, but the exact semantics matter:

- Optimal String Alignment distance handles restricted adjacent transpositions.
- Full Damerau-Levenshtein handles broader transposition behavior.
- The current graph permits repeated adjacent swaps as separate graph steps, which can generate permutations through sequences of swaps.

Matching the current graph exactly in a streaming automaton is more complex than insert/delete/replace and should not block the memory fix.

### Practical phase-2 options

Option A: Disable swap for large password-recovery runs.

This is the simplest and safest option.

Option B: Hybrid approximate swap pass.

1. Generate swap-only variants of the seed up to a small swap count.
2. For each swap variant, run the streaming insert/delete/replace generator with remaining edit budget.
3. Accept possible duplicates, or use a small bounded duplicate cache/Bloom filter if duplicate worker checks are acceptable.

This is not exact, but may be useful operationally.

Option C: Exact weighted Damerau streaming.

Implement a streaming Damerau-Levenshtein automaton. This should be a separate project phase with its own tests, because correctness is subtle.

---

## 9. Memory and complexity comparison

Let:

```text
n = seed length
A = alphabet size
D = max edit distance
M = max candidate length, approximately n + D
```

### Current graph search

Approximate branching factor from length `L`:

```text
B(L) = delete_enabled * L
     + insert_enabled * (L + 1) * A
     + replace_enabled * L * (A - 1)
     + swap_enabled * max(L - 1, 0)
```

Memory complexity:

```text
O(number_of_unique_reachable_strings_up_to_depth_D)
```

The constant factor is high because strings are owned by multiple containers.

### Streaming DP search

Each stack frame stores one DP row of length `n + 1`. Stack depth is at most `M + 1`.

Memory complexity:

```text
O(M * n + A + queue_size)
```

For `n = 8`, `D = 3`, `M = 11`, the generator's DP stack is tiny:

```text
about 12 rows * 9 scores/row = 108 scores
```

Even with Rust `Vec` overhead, this is negligible compared with millions of hash-map entries.

Time complexity is still large because the search space is large. The new algorithm prevents memory exhaustion; it does not make tens or hundreds of millions of candidate checks disappear.

---

## 10. Implementation plan

### Phase 0: Add safety guard to the current graph enumerator

Goal: prevent accidental crashes while the streaming implementation is being added.

Add config fields either to `EngineConfig` or a new enumerator factory config:

```rust
pub struct EnumeratorLimits {
    pub max_graph_states: u64,
    pub max_graph_estimated_bytes: u64,
    pub large_search_policy: LargeSearchPolicy,
}

pub enum LargeSearchPolicy {
    Error,
    UseStreamingIfCompatible,
}
```

Add a conservative estimate before constructing `PipelinedOrderedCandidateEnumerator`.

Do not rely on a perfect estimate. It only needs to catch obvious blow-ups.

Example lower-bound estimate from replacements:

```rust
fn replacement_only_lower_bound(seed_len: usize, alphabet_len: usize, max_distance: usize) -> u128 {
    let mut total = 0u128;
    let choices = alphabet_len.saturating_sub(1) as u128;

    for k in 0..=max_distance.min(seed_len) {
        total += binom(seed_len, k) as u128 * choices.pow(k as u32);
    }

    total
}
```

For the user's 8-character, 94-symbol, distance-3 case, this lower bound is 45,286,909 for replacement distances 0 through 3. If this lower bound already exceeds a threshold, graph search must not run.

Acceptance criteria:

- A large graph config returns a clear error or chooses streaming in `Auto` mode.
- Existing graph tests still pass.
- No production code path can instantiate the graph enumerator for obviously huge searches without opting in.

### Phase 1: Add strategy selection

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnumeratorStrategy {
    Auto,
    OrderedGraph,
    StreamingLevenshtein,
}
```

Factory function:

```rust
pub fn make_candidate_enumerator(
    config: SearchConfig,
    strategy: EnumeratorStrategy,
    limits: EnumeratorLimits,
) -> Result<CandidateEnumerator, ConfigError> {
    match strategy {
        EnumeratorStrategy::OrderedGraph => {
            validate_graph_limits(&config, &limits)?;
            Ok(CandidateEnumerator::Ordered(PipelinedOrderedCandidateEnumerator::new(config)?))
        }
        EnumeratorStrategy::StreamingLevenshtein => {
            validate_streaming_supported(&config)?;
            Ok(CandidateEnumerator::Streaming(StreamingLevenshteinCandidateEnumerator::new(config)?))
        }
        EnumeratorStrategy::Auto => {
            if graph_estimate_is_safe(&config, &limits) {
                Ok(CandidateEnumerator::Ordered(PipelinedOrderedCandidateEnumerator::new(config)?))
            } else {
                validate_streaming_supported(&config)?;
                Ok(CandidateEnumerator::Streaming(StreamingLevenshteinCandidateEnumerator::new(config)?))
            }
        }
    }
}
```

Validation for streaming MVP:

```rust
fn validate_streaming_supported(config: &SearchConfig) -> Result<(), ConfigError> {
    if config.distance_mode != DistanceMode::GlobalMinimumDistance {
        return Err(ConfigError::UnsupportedForStreaming {
            reason: "streaming mode requires GlobalMinimumDistance".to_string(),
        });
    }

    if config.ops.swap {
        return Err(ConfigError::UnsupportedForStreaming {
            reason: "streaming Levenshtein MVP supports insert/delete/replace, not swap".to_string(),
        });
    }

    Ok(())
}
```

The exact error shape can differ; the important point is that the error must be explicit.

### Phase 2: Implement streaming insert/delete/replace enumerator

Files to modify:

```text
src/lib.rs, or preferably a new src/streaming.rs module
src/engine.rs, if the engine constructs the enumerator directly
```

Suggested new module:

```rust
mod streaming;
pub use streaming::{StreamingLevenshteinCandidateEnumerator, StreamingEnumeratorStats};
```

Implementation tasks:

1. Normalize `config.alphabet` as existing code does.
2. Build `raw_alphabet: HashSet<char>` from normalized config alphabet.
3. Build `generation_alphabet = sorted_dedup(config.alphabet union seed.chars())`.
4. Compute target length range.
5. Initialize `target_len = min_target_len`.
6. Initialize iterative DFS stack with `initial_row()`.
7. Implement `step_row()`.
8. Implement `can_still_reach()` pruning.
9. Implement `Iterator<Item = Candidate>`.
10. Add stats comparable to current stats but streaming-specific.

Recommended stats:

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamingEnumeratorStats {
    pub prefixes_visited: u64,
    pub prefixes_pruned: u64,
    pub leaves_checked: u64,
    pub emitted: u64,
    pub rows_computed: u64,
}
```

Acceptance criteria:

- Streaming enumerator produces expected candidates for hand-written small examples.
- Streaming enumerator does not allocate memory proportional to emitted candidates.
- Streaming enumerator works when `min_distance > 0`; it must not emit seed unless distance 0 is in range.
- Streaming enumerator works with empty seed.
- Streaming enumerator works when seed contains a char absent from raw alphabet.

### Phase 3: Integrate with engine and worker

Because `engine.rs` and `worker.rs` were not uploaded, the following is a required integration review rather than exact code.

The engine should consume candidates like this:

```rust
let mut enumerator = make_candidate_enumerator(config, strategy, limits)?;

while let Some(candidate) = enumerator.next() {
    worker.check(candidate)?;
}
```

Or, for parallel workers:

```rust
let (tx, rx) = crossbeam_channel::bounded::<Candidate>(buffer_size);
```

Hard rule:

```text
The channel must be bounded.
```

Suggested defaults:

```text
candidate_buffer_size: 1024 to 16384
```

Do not use:

```rust
crossbeam_channel::unbounded()
std::sync::mpsc::channel()  // unbounded semantics
Vec<Candidate>              // large batch accumulation
```

If batching is required for worker throughput, use a fixed maximum batch size:

```rust
const MAX_BATCH_SIZE: usize = 4096;
```

Acceptance criteria:

- The feeder blocks when the worker is slower than generation.
- Resident memory stays roughly flat during long runs.
- Stopping early on success does not require draining or freeing a massive queue.

### Phase 4: Snapshot and resume for streaming mode

Do not reuse `EnumeratorSnapshot`, because it stores graph-specific frontier data.

Add a new snapshot type:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamingEnumeratorSnapshot {
    schema_version: u32,
    config_hash: String,
    target_len: usize,
    prefix: Vec<char>,
    stack: Vec<StreamingFrameSnapshot>,
    emitted: u64,
    stats: StreamingEnumeratorStats,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StreamingFrameSnapshot {
    row: Vec<ScoreSnapshot>,
    next_symbol_index: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ScoreSnapshot {
    edits: u16,
    cost: u32,
}
```

This snapshot size is bounded by:

```text
O(max_candidate_length * seed_length)
```

Do not serialize any global candidate set.

Acceptance criteria:

- Snapshot after N candidates, restore, and continue equals uninterrupted streaming output.
- Snapshot size remains small for large searches.
- Snapshot restore rejects config hash mismatch.

### Phase 5: Reduce candidate representation overhead

The current public `Candidate` duplicates content:

```rust
pub text: String,
pub chars: Vec<char>,
```

For compatibility, keep this initially. Later, add a lighter internal type:

```rust
pub struct CandidateText {
    pub ordinal: u64,
    pub text: String,
    pub distance: usize,
    pub cost: u32,
}
```

Or for ASCII-only worker paths:

```rust
pub struct CandidateBytes {
    pub ordinal: u64,
    pub bytes: Vec<u8>,
    pub distance: usize,
    pub cost: u32,
}
```

If most passwords are ASCII, a later optimization should use:

```rust
smallvec::SmallVec<[u8; 16]>
```

for short candidates. This avoids many heap allocations.

Do not do this before the streaming algorithm is correct. The algorithmic fix matters more than the representation fix.

### Phase 6: Optional swap support

After streaming insert/delete/replace is stable, choose one of these explicitly:

1. Keep swap unsupported in streaming mode.
2. Add a documented approximate hybrid swap mode.
3. Implement exact Damerau-style streaming support.

Do not merge partial swap support without tests comparing it to the graph enumerator on small exhaustive cases.

---

## 11. Test plan

### 11.1 Existing tests that must continue passing

All current tests in `lib.rs` should continue to pass for `PipelinedOrderedCandidateEnumerator`, including:

- heap ordering
- golden examples
- duplicate deletes
- duplicate insertions
- swap behavior
- replacement-to-same-char behavior
- keyboard neighbor costs
- Unicode char boundary behavior
- alphabet normalization
- both distance modes
- empty seed
- empty alphabet
- max distance zero
- invalid distance band
- ordinals
- config hashing
- snapshot restore
- invariant matrix
- stats consistency

### 11.2 New unit tests for DP score helpers

Add tests for `Score::add()`:

```rust
#[test]
fn score_add_caps_at_max_distance() {
    let s = Score { edits: 2, cost: 4 };
    assert_eq!(s.add(1, 2, 3), Score { edits: 3, cost: 6 });
    assert_eq!(s.add(2, 2, 3), Score::inf());
}
```

Add tests for `better()`:

```rust
#[test]
fn better_prefers_fewer_edits_then_lower_cost() {
    assert_eq!(
        better(Score { edits: 1, cost: 9 }, Score { edits: 2, cost: 1 }),
        Score { edits: 1, cost: 9 }
    );
    assert_eq!(
        better(Score { edits: 2, cost: 9 }, Score { edits: 2, cost: 1 }),
        Score { edits: 2, cost: 1 }
    );
}
```

### 11.3 New row-transition tests

#### Delete-only

```rust
#[test]
fn streaming_row_delete_only_handles_empty_candidate() {
    let cfg = config("ab", vec![], 2, 2, EditOps::delete_only(), DistanceMode::GlobalMinimumDistance);
    let row = initial_row(2, &cfg);
    assert_eq!(row[2].edits, 2);
    assert_eq!(row[2].cost, 4);
}
```

#### Insert-only

```rust
#[test]
fn streaming_insert_only_from_empty_seed() {
    let cfg = config("", vec!['a'], 1, 1, EditOps::insert_only(), DistanceMode::GlobalMinimumDistance);
    let got: Vec<_> = StreamingLevenshteinCandidateEnumerator::new(cfg).unwrap().collect();
    assert_eq!(got.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["a"]);
    assert_eq!(got[0].distance, 1);
    assert_eq!(got[0].cost, 2);
}
```

#### Replace with keyboard neighbor

```rust
#[test]
fn streaming_keyboard_neighbor_replace_cost_is_one() {
    let mut neighbors = HashMap::new();
    neighbors.insert('a', HashSet::from(['b']));

    let cfg = SearchConfig {
        seed: "a".to_string(),
        alphabet: vec!['b'],
        min_distance: 1,
        max_distance: 1,
        ops: EditOps::replace_only(),
        keyboard_neighbors: neighbors,
        distance_mode: DistanceMode::GlobalMinimumDistance,
    };

    let got: Vec<_> = StreamingLevenshteinCandidateEnumerator::new(cfg).unwrap().collect();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text, "b");
    assert_eq!(got[0].distance, 1);
    assert_eq!(got[0].cost, 1);
}
```

### 11.4 Cross-check tests against current graph enumerator

For small configs with `swap = false` and `GlobalMinimumDistance`, compare the set of emitted triples:

```rust
fn collect_set<I: Iterator<Item = Candidate>>(iter: I) -> BTreeSet<(String, usize, u32)> {
    iter.map(|c| (c.text, c.distance, c.cost)).collect()
}

#[test]
fn streaming_matches_graph_for_small_no_swap_configs() {
    let seeds = ["", "a", "ab", "aa", "aba"];
    let alphabets: &[&[char]] = &[&[], &['a'], &['a', 'b']];
    let bands = [(0, 0), (0, 1), (1, 1), (0, 2), (1, 2), (2, 2)];
    let ops_set = [
        EditOps::none(),
        EditOps::delete_only(),
        EditOps::insert_only(),
        EditOps::replace_only(),
        EditOps { delete: true, insert: true, replace: true, swap: false },
    ];

    for seed in seeds {
        for alphabet in alphabets {
            for (min, max) in bands {
                for ops in ops_set {
                    let cfg = SearchConfig {
                        seed: seed.to_string(),
                        alphabet: alphabet.to_vec(),
                        min_distance: min,
                        max_distance: max,
                        ops,
                        keyboard_neighbors: HashMap::new(),
                        distance_mode: DistanceMode::GlobalMinimumDistance,
                    };

                    let graph = collect_set(PipelinedOrderedCandidateEnumerator::new(cfg.clone()).unwrap());
                    let streaming = collect_set(StreamingLevenshteinCandidateEnumerator::new(cfg).unwrap());
                    assert_eq!(streaming, graph);
                }
            }
        }
    }
}
```

Important: compare sets, not order. Streaming order is allowed to differ.

### 11.5 Alphabet edge case test

This test ensures a seed character not in the raw alphabet can survive by matching, but cannot be inserted or used as a replacement unless it is in the raw alphabet.

```rust
#[test]
fn streaming_seed_char_outside_alphabet_can_match_but_not_insert() {
    let cfg = SearchConfig {
        seed: "x".to_string(),
        alphabet: vec!['a'],
        min_distance: 0,
        max_distance: 1,
        ops: EditOps { delete: true, insert: true, replace: true, swap: false },
        keyboard_neighbors: HashMap::new(),
        distance_mode: DistanceMode::GlobalMinimumDistance,
    };

    let got = collect_set(StreamingLevenshteinCandidateEnumerator::new(cfg.clone()).unwrap());
    let graph = collect_set(PipelinedOrderedCandidateEnumerator::new(cfg).unwrap());
    assert_eq!(got, graph);
}
```

### 11.6 Large-search memory smoke test

This test should be ignored by default or run as a benchmark/integration test. It should not collect all candidates.

```rust
#[test]
#[ignore]
fn streaming_large_search_does_not_grow_memory_while_taking_prefix() {
    let alphabet: Vec<char> = (33u8..=126u8).map(char::from).collect();
    let cfg = SearchConfig {
        seed: "abcd1234".to_string(),
        alphabet,
        min_distance: 3,
        max_distance: 3,
        ops: EditOps { delete: true, insert: true, replace: true, swap: false },
        keyboard_neighbors: HashMap::new(),
        distance_mode: DistanceMode::GlobalMinimumDistance,
    };

    let mut e = StreamingLevenshteinCandidateEnumerator::new(cfg).unwrap();

    for _ in 0..1_000_000 {
        if e.next().is_none() {
            break;
        }
    }

    // In CI, assert stats only. In local profiling, assert RSS externally.
    assert!(e.stats().rows_computed > 0);
}
```

A separate local benchmark should track resident set size over time. It should remain approximately flat.

### 11.7 Snapshot tests for streaming mode

```rust
#[test]
fn streaming_snapshot_restore_matches_uninterrupted_run() {
    let cfg = SearchConfig {
        seed: "ab".to_string(),
        alphabet: vec!['a', 'b', 'c'],
        min_distance: 0,
        max_distance: 2,
        ops: EditOps { delete: true, insert: true, replace: true, swap: false },
        keyboard_neighbors: HashMap::new(),
        distance_mode: DistanceMode::GlobalMinimumDistance,
    };

    let full: Vec<_> = StreamingLevenshteinCandidateEnumerator::new(cfg.clone()).unwrap().collect();

    let split_at = full.len() / 2;
    let mut first = StreamingLevenshteinCandidateEnumerator::new(cfg.clone()).unwrap();
    let prefix: Vec<_> = first.by_ref().take(split_at).collect();
    let snapshot = first.snapshot();
    let restored = StreamingLevenshteinCandidateEnumerator::from_snapshot(cfg, snapshot).unwrap();
    let suffix: Vec<_> = restored.collect();

    let combined: Vec<_> = prefix.into_iter().chain(suffix).collect();
    assert_eq!(combined, full);
}
```

### 11.8 Engine backpressure test

If the engine uses channels, add a test that creates a slow fake worker and verifies the queue cannot grow past capacity.

Pseudo-code:

```rust
#[test]
fn engine_uses_bounded_candidate_queue() {
    let cfg = large_streaming_config();
    let worker = SlowFakeWorker::new(Duration::from_millis(1));
    let report = run_with_buffer_size(cfg, worker, 8, StopAfterCandidates(100));

    assert!(report.max_observed_queue_len <= 8);
}
```

---

## 12. Acceptance criteria

The implementation is acceptable when all of the following are true:

1. Existing tests for `PipelinedOrderedCandidateEnumerator` still pass.
2. New streaming tests pass.
3. Small no-swap `GlobalMinimumDistance` configs produce the same candidate set as the graph enumerator.
4. Large config with seed length 8, alphabet length around 94, max distance 3, and no swap can start producing candidates without memory growth proportional to total candidate count.
5. Graph enumerator refuses or reroutes obvious large searches according to strategy and limits.
6. Streaming mode rejects unsupported configs clearly, especially `PerDistanceBestCost` and `swap = true` in MVP.
7. Engine/worker integration uses bounded queues or bounded batches.
8. Snapshot for streaming mode is compact and does not clone the full search space.
9. Documentation clearly states that streaming mode does not preserve exact `(distance, cost, lexical)` ordering.
10. The user-facing run report records which enumerator strategy was used.

---

## 13. Recommended default configuration for the user's case

For the case that currently crashes, use:

```rust
SearchConfig {
    seed: seed.to_string(),
    alphabet,
    min_distance: 3,
    max_distance: 3,
    ops: EditOps {
        delete: true,
        insert: true,
        replace: true,
        swap: false, // MVP recommendation for large streaming runs
    },
    keyboard_neighbors,
    distance_mode: DistanceMode::GlobalMinimumDistance,
}
```

And strategy:

```rust
EnumeratorStrategy::StreamingLevenshtein
```

Engine limit defaults:

```rust
EnumeratorLimits {
    max_graph_states: 1_000_000,
    max_graph_estimated_bytes: 256 * 1024 * 1024,
    large_search_policy: LargeSearchPolicy::UseStreamingIfCompatible,
}
```

Worker queue:

```rust
candidate_buffer_size: 4096
```

These values are conservative starting points. The buffer size can be tuned based on worker throughput.

---

## 14. What not to do

Do not try to fix the crash only by changing data structures inside the current graph search. These changes can help small and medium runs, but they will not solve the core issue:

- Replacing `Vec<char>` with `String` in the hash map is not enough.
- Replacing `HashMap` with `BTreeMap` is not enough and may be slower.
- Interning strings is not enough.
- Increasing RAM is not enough for larger alphabets/distances.
- Taking snapshots less often is not enough.
- Using `GlobalMinimumDistance` helps duplicate output but does not stop the graph frontier from growing huge.

The algorithm must stop materializing the reachable edit graph for large searches.

---

## 15. Future optimizations after correctness

After the streaming enumerator is correct and integrated, consider these optimizations:

1. ASCII fast path using `u8` instead of `char`.
2. `SmallVec<[u8; 16]>` for short password candidates.
3. Avoid building `String` until the worker actually needs it.
4. Precompute keyboard-neighbor replacement costs in a table.
5. Replace `HashSet<char>` membership with a compact lookup table for ASCII alphabets.
6. Parallelize by splitting the prefix tree by first character or first two characters.
7. Add checkpoint/resume per prefix shard for distributed runs.
8. Add optional disk-backed external sorting only if exact current output order becomes a hard requirement.

---

## 16. Parallelization note

The streaming prefix tree is easy to shard. For example, with a generation alphabet of size `A`, create one shard per first character:

```text
shard 0: candidates beginning with generation_alphabet[0]
shard 1: candidates beginning with generation_alphabet[1]
...
```

Each shard owns its own DP row after the fixed prefix and can stream independently. This enables multiple feeder threads without shared candidate state.

Do not share a global deduplication set between shards. If the prefix split is by actual candidate prefix, each string belongs to exactly one shard, so no dedup set is needed.

---

## 17. Final recommendation

Use a two-enumerator architecture:

```text
Small/exact searches:
    PipelinedOrderedCandidateEnumerator
    exact current order
    existing snapshot semantics
    memory guarded

Large password-recovery searches:
    StreamingLevenshteinCandidateEnumerator
    GlobalMinimumDistance only
    insert/delete/replace MVP
    deterministic but not cost-sorted
    bounded memory
    bounded worker queue
```

This directly addresses the crash. The current graph search is doing too much bookkeeping for a search space that is far too large to hold in memory. The streaming DP approach changes the core invariant: generate, test, discard.

That is the right model for a password feeder.
