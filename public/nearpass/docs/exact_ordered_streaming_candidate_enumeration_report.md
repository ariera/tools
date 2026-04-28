# Exact Ordered Streaming Candidate Enumeration

**Technical report and implementation plan for coding-agent handoff**

## 1. Objective

Build an enumerator that preserves the existing exact ordering contract:

```text
1. edit distance ascending
2. accumulated likelihood cost ascending
3. lexical candidate order ascending
```

while avoiding the current large pause between distance layers.

The proposed implementation is a **pipelined ordered layer-relaxation enumerator**.

It does **not** emit candidates on discovery. It emits only after a candidate is finalized under the ordering rules. The key improvement is that it incrementally builds distance `d + 1` while emitting finalized candidates from distance `d`, instead of waiting until all of distance `d` has been emitted and then building/sorting the entire next layer in one blocking operation.

---

## 2. Critical Semantic Decisions

### 2.1 Distance means script depth unless explicitly changed

The current compatibility behavior appears to mean:

```text
distance d = reachable by exactly d edit operations
```

This is not always the same as minimum edit distance.

Example:

```text
seed = "a"
alphabet = ['a', 'b']
replace enabled

depth 0: "a"
depth 1: "b"
depth 2: "a"    // replace a -> b -> a
```

So `"a"` can appear again at distance `2` under exact script-depth semantics.

This report assumes the compatibility mode is:

```rust
DistanceMode::PerDistanceBestCost
```

Meaning:

```text
For each depth d, emit each candidate at most once,
using the best accumulated likelihood cost known at that depth.
```

Do **not** silently change this to global deduplication unless the product contract is changed.

### 2.2 Optional global minimum-distance mode

A second mode is useful, but it must be explicit:

```rust
DistanceMode::GlobalMinimumDistance
```

Meaning:

```text
Emit each candidate string only at its first/minimum reachable edit depth.
Never emit it again at later depths.
```

This is closer to a true bounded edit-distance neighborhood.

The two modes have different outputs. Treat them as different contracts.

---

## 3. High-Level Algorithm

Use a priority queue over layered graph nodes.

A node is:

```text
(depth, candidate_string)
```

An edge is:

```text
(depth, parent) --one edit--> (depth + 1, child)
```

Each edge has likelihood cost:

```text
delete:  2
insert:  2
replace: 1 if keyboard neighbor, else 3
swap:    1
```

The priority queue key is:

```text
(depth ascending, accumulated_cost ascending, candidate_string lexical ascending)
```

The enumerator repeatedly:

```text
1. Pop the smallest heap item.
2. Ignore it if it is stale.
3. Finalize it.
4. Expand it into depth + 1.
5. Emit it if depth >= min_distance.
```

The important detail is step 4:

```text
Expand before returning the candidate.
```

This spreads the expensive construction of the next layer across many calls to `next()`.

---

## 4. Graph Structure

The search graph is not just strings. It is a **layered graph**.

For seed `"a"`:

```text
depth 0:
  "a"

depth 1:
  ""
  "aa"
  "ab"
  "ba"
  "b"

depth 2:
  ...
```

A string may appear at multiple depths:

```text
(depth 0, "a")
(depth 2, "a")
(depth 4, "a")
...
```

Those are distinct graph nodes under per-distance semantics.

This distinction is essential.

Do **not** use only `HashSet<String>` as the main visited set in compatibility mode. That would incorrectly suppress valid later-depth candidates.

The compatibility dedup key is:

```text
(depth, candidate_string)
```

not just:

```text
candidate_string
```

---

## 5. Correctness Invariant

The core invariant is:

> Before any node at depth `d` is emitted, every finalized node at depth `d - 1` has already been expanded.

This is guaranteed because the heap sorts by `depth` first.

Every possible edge into depth `d` comes from depth `d - 1`. Therefore, once all depth `d - 1` nodes have been popped and expanded, the heap contains every reachable candidate at depth `d`, with the best cost discovered so far.

Since no depth-`d` node can generate another depth-`d` node, a depth-`d` candidate’s cost cannot improve after depth `d` emission begins.

Then the heap ordering gives:

```text
distance ascending,
then cost ascending,
then lexical ascending.
```

This exactly matches the ordered contract.

---

## 6. Why This Reduces Pauses

The existing exact enumerator does this:

```text
emit layer d
build all of layer d + 1
deduplicate all of layer d + 1
sort all of layer d + 1
emit first item from layer d + 1
```

That causes a large silent gap.

The proposed enumerator does this:

```text
pop one item from layer d
expand that item into layer d + 1
emit the item
repeat
```

For the example from the brief:

```text
seed length n = 6
alphabet size A ~= 91
distance-1 candidates ~= 1.2k
distance-2 raw neighbor events ~= 1.5M
```

The old exact enumerator performs most of those `~1.5M` events in one opaque block.

The new enumerator distributes the work over the roughly `1.2k` depth-1 emissions.

Total work is still large. The improvement is latency and perceived responsiveness, not asymptotic magic.

---

## 7. Constraints and Non-Negotiable Rules

### 7.1 Unicode

Use:

```rust
Vec<char>
```

as the internal representation.

Do **not** mutate strings by byte offset.

Wrong:

```rust
s.remove(byte_index);
s.insert(byte_index, ch);
```

Correct:

```rust
let chars: Vec<char> = seed.chars().collect();
```

All edit positions are char positions.

Important caveat: `char` correctness is not the same as grapheme-cluster correctness. For example:

```text
"é"
"e\u{301}"
```

are different `Vec<char>` sequences. If grapheme-level behavior is required later, that is a separate design change.

### 7.2 Alphabet

Normalize the alphabet once during config initialization:

```rust
alphabet.sort_unstable();
alphabet.dedup();
```

The alphabet controls inserted and replacement characters.

Seed characters may appear in outputs even if they are not in the alphabet, because existing seed characters can survive edits.

### 7.3 Replacement

Replacement must skip replacing a char with itself:

```rust
if replacement == original {
    continue;
}
```

Replacement cost:

```text
1 if replacement is a keyboard neighbor of original
3 otherwise
```

Keyboard-neighbor lookup should be treated as directed unless the config explicitly normalizes it to be symmetric.

That means:

```text
a -> b
```

does not automatically imply:

```text
b -> a
```

unless you explicitly add both entries.

### 7.4 Swap

Swap only adjacent distinct chars:

```rust
if chars[i] == chars[i + 1] {
    continue;
}
```

Swapping identical adjacent chars must not emit the same string as a cost-1 candidate.

### 7.5 `min_distance`

Do not skip expanding nodes below `min_distance`.

Wrong:

```rust
if depth < min_distance {
    continue;
}
```

That prevents the enumerator from reaching the minimum requested layer.

Correct:

```text
Always expand finalized nodes while depth < max_distance.
Only suppress emission while depth < min_distance.
```

### 7.6 `max_distance`

Never expand a node at `max_distance`.

```rust
if item.depth < config.max_distance {
    expand(item)
}
```

### 7.7 Determinism

Output order must not depend on:

```text
HashMap iteration order
HashSet iteration order
operation generation order
heap insertion order
```

The heap key must be a total order:

```text
(depth, cost, candidate chars)
```

If two heap items have exactly the same key, they represent the same logical node and should not both be inserted.

---

## 8. Data Model

### 8.1 Candidate

```rust
pub struct Candidate {
    pub text: String,
    pub chars: Vec<char>,
    pub distance: usize,
    pub cost: u32,
}
```

### 8.2 Queue item

```rust
struct QueueItem {
    depth: usize,
    cost: u32,
    chars: Vec<char>,
}
```

The priority queue is a `BinaryHeap<QueueItem>`, with reversed ordering because Rust’s `BinaryHeap` is a max-heap.

### 8.3 Best-cost map

Use one map per depth:

```rust
Vec<HashMap<Vec<char>, u32>>
```

Meaning:

```text
best[depth][candidate] = best known cost for this candidate at this depth
```

This is required to:

```text
1. deduplicate candidates within a depth
2. keep the best known cost if multiple paths reach the same candidate
3. skip stale heap entries
```

### 8.4 Optional global finalized set

Only for global minimum-distance mode:

```rust
HashSet<Vec<char>>
```

Meaning:

```text
candidate strings already finalized at a lower/equal depth
```

In global mode, once a candidate string has been finalized, later occurrences should be skipped and not expanded.

---

## 9. Rust Baseline Implementation

This is a complete baseline implementation intended to be easy to copy, test, and adapt.

It uses only the Rust standard library.

```rust
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceMode {
    /// Compatibility mode:
    /// A candidate may appear once per distance layer.
    /// Deduplication key is (depth, candidate).
    PerDistanceBestCost,

    /// True minimum-distance mode:
    /// A candidate string is emitted only once, at its first reachable depth.
    /// Deduplication key is candidate.
    GlobalMinimumDistance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditOps {
    pub delete: bool,
    pub insert: bool,
    pub replace: bool,
    pub swap: bool,
}

impl EditOps {
    pub fn all() -> Self {
        Self {
            delete: true,
            insert: true,
            replace: true,
            swap: true,
        }
    }

    pub fn none() -> Self {
        Self {
            delete: false,
            insert: false,
            replace: false,
            swap: false,
        }
    }

    pub fn delete_only() -> Self {
        Self {
            delete: true,
            insert: false,
            replace: false,
            swap: false,
        }
    }

    pub fn insert_only() -> Self {
        Self {
            delete: false,
            insert: true,
            replace: false,
            swap: false,
        }
    }

    pub fn replace_only() -> Self {
        Self {
            delete: false,
            insert: false,
            replace: true,
            swap: false,
        }
    }

    pub fn swap_only() -> Self {
        Self {
            delete: false,
            insert: false,
            replace: false,
            swap: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchConfig {
    pub seed: String,
    pub alphabet: Vec<char>,
    pub min_distance: usize,
    pub max_distance: usize,
    pub ops: EditOps,

    /// Directed keyboard-neighbor map.
    ///
    /// If keyboard_neighbors['a'] contains 'b', then replacing a -> b costs 1.
    /// This does not imply b -> a unless that entry is also present.
    pub keyboard_neighbors: HashMap<char, HashSet<char>>,

    pub distance_mode: DistanceMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    InvalidDistanceBand { min: usize, max: usize },
    MaxDistanceTooLarge { max: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub chars: Vec<char>,
    pub distance: usize,
    pub cost: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueueItem {
    depth: usize,
    cost: u32,
    chars: Vec<char>,
}

/// Rust BinaryHeap is a max-heap.
/// This Ord implementation reverses comparison so that the smallest logical key
/// is popped first:
///
///   depth ascending,
///   cost ascending,
///   lexical chars ascending.
impl Ord for QueueItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .depth
            .cmp(&self.depth)
            .then_with(|| other.cost.cmp(&self.cost))
            .then_with(|| other.chars.cmp(&self.chars))
    }
}

impl PartialOrd for QueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Default)]
pub struct EnumeratorStats {
    pub popped: u64,
    pub stale_skipped: u64,
    pub global_duplicate_skipped: u64,
    pub expanded: u64,
    pub raw_neighbors_generated: u64,
    pub local_unique_neighbors: u64,
    pub relaxed_new: u64,
    pub relaxed_improved: u64,
    pub relaxed_not_better: u64,
    pub emitted: u64,
}

pub struct PipelinedOrderedCandidateEnumerator {
    config: SearchConfig,
    heap: BinaryHeap<QueueItem>,

    /// best[depth][chars] = best known cost at this exact depth.
    best: Vec<HashMap<Vec<char>, u32>>,

    /// Used only for DistanceMode::GlobalMinimumDistance.
    finalized_global: HashSet<Vec<char>>,

    stats: EnumeratorStats,
}

impl PipelinedOrderedCandidateEnumerator {
    pub fn new(mut config: SearchConfig) -> Result<Self, ConfigError> {
        if config.min_distance > config.max_distance {
            return Err(ConfigError::InvalidDistanceBand {
                min: config.min_distance,
                max: config.max_distance,
            });
        }

        // With the current cost model, max accumulated cost is <= 3 * max_distance.
        // This keeps u32 costs safe.
        if config.max_distance > (u32::MAX as usize / 3) {
            return Err(ConfigError::MaxDistanceTooLarge {
                max: config.max_distance,
            });
        }

        config.alphabet.sort_unstable();
        config.alphabet.dedup();

        let seed_chars: Vec<char> = config.seed.chars().collect();

        let mut best = Vec::with_capacity(config.max_distance + 1);
        for _ in 0..=config.max_distance {
            best.push(HashMap::new());
        }

        best[0].insert(seed_chars.clone(), 0);

        let mut heap = BinaryHeap::new();
        heap.push(QueueItem {
            depth: 0,
            cost: 0,
            chars: seed_chars,
        });

        Ok(Self {
            config,
            heap,
            best,
            finalized_global: HashSet::new(),
            stats: EnumeratorStats::default(),
        })
    }

    pub fn stats(&self) -> &EnumeratorStats {
        &self.stats
    }

    fn is_keyboard_neighbor(&self, from: char, to: char) -> bool {
        self.config
            .keyboard_neighbors
            .get(&from)
            .map_or(false, |neighbors| neighbors.contains(&to))
    }

    fn record_local_neighbor(
        local: &mut HashMap<Vec<char>, u32>,
        child: Vec<char>,
        delta_cost: u32,
    ) {
        match local.entry(child) {
            Entry::Vacant(v) => {
                v.insert(delta_cost);
            }
            Entry::Occupied(mut o) => {
                if delta_cost < *o.get() {
                    *o.get_mut() = delta_cost;
                }
            }
        }
    }

    /// Generate all unique one-edit neighbors of `parent`.
    ///
    /// The returned map stores the cheapest single-edit cost per child.
    ///
    /// This local deduplication is not strictly required for correctness because
    /// global relaxation also deduplicates, but it avoids unnecessary heap/map work
    /// for repeated chars and repeated insertion positions.
    fn one_edit_neighbors(&mut self, parent: &[char]) -> HashMap<Vec<char>, u32> {
        let mut local: HashMap<Vec<char>, u32> = HashMap::new();
        let len = parent.len();

        if self.config.ops.delete {
            for i in 0..len {
                let mut child = Vec::with_capacity(len.saturating_sub(1));
                child.extend_from_slice(&parent[..i]);
                child.extend_from_slice(&parent[i + 1..]);

                self.stats.raw_neighbors_generated += 1;
                Self::record_local_neighbor(&mut local, child, 2);
            }
        }

        if self.config.ops.insert {
            for pos in 0..=len {
                for &ch in &self.config.alphabet {
                    let mut child = Vec::with_capacity(len + 1);
                    child.extend_from_slice(&parent[..pos]);
                    child.push(ch);
                    child.extend_from_slice(&parent[pos..]);

                    self.stats.raw_neighbors_generated += 1;
                    Self::record_local_neighbor(&mut local, child, 2);
                }
            }
        }

        if self.config.ops.replace {
            for i in 0..len {
                let original = parent[i];

                for &replacement in &self.config.alphabet {
                    if replacement == original {
                        continue;
                    }

                    let mut child = parent.to_vec();
                    child[i] = replacement;

                    let delta_cost = if self.is_keyboard_neighbor(original, replacement) {
                        1
                    } else {
                        3
                    };

                    self.stats.raw_neighbors_generated += 1;
                    Self::record_local_neighbor(&mut local, child, delta_cost);
                }
            }
        }

        if self.config.ops.swap && len >= 2 {
            for i in 0..(len - 1) {
                if parent[i] == parent[i + 1] {
                    continue;
                }

                let mut child = parent.to_vec();
                child.swap(i, i + 1);

                self.stats.raw_neighbors_generated += 1;
                Self::record_local_neighbor(&mut local, child, 1);
            }
        }

        self.stats.local_unique_neighbors += local.len() as u64;
        local
    }

    fn relax(&mut self, depth: usize, chars: Vec<char>, cost: u32) {
        if depth > self.config.max_distance {
            return;
        }

        if self.config.distance_mode == DistanceMode::GlobalMinimumDistance
            && self.finalized_global.contains(&chars)
        {
            self.stats.global_duplicate_skipped += 1;
            return;
        }

        match self.best[depth].entry(chars.clone()) {
            Entry::Vacant(v) => {
                v.insert(cost);
                self.heap.push(QueueItem { depth, cost, chars });
                self.stats.relaxed_new += 1;
            }
            Entry::Occupied(mut o) => {
                if cost < *o.get() {
                    *o.get_mut() = cost;
                    self.heap.push(QueueItem { depth, cost, chars });
                    self.stats.relaxed_improved += 1;
                } else {
                    self.stats.relaxed_not_better += 1;
                }
            }
        }
    }

    fn chars_to_string(chars: &[char]) -> String {
        chars.iter().collect()
    }
}

impl Iterator for PipelinedOrderedCandidateEnumerator {
    type Item = Candidate;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(item) = self.heap.pop() {
            self.stats.popped += 1;

            let best_cost = self.best[item.depth].get(&item.chars).copied();

            // Stale entry left in the heap after a cheaper path was discovered.
            if best_cost != Some(item.cost) {
                self.stats.stale_skipped += 1;
                continue;
            }

            if self.config.distance_mode == DistanceMode::GlobalMinimumDistance {
                if self.finalized_global.contains(&item.chars) {
                    self.stats.global_duplicate_skipped += 1;
                    continue;
                }

                // Mark all finalized strings, even below min_distance.
                // This prevents the seed from being emitted again at distance 2
                // when min_distance > 0.
                self.finalized_global.insert(item.chars.clone());
            }

            if item.depth < self.config.max_distance {
                self.stats.expanded += 1;

                let child_depth = item.depth + 1;
                let children = self.one_edit_neighbors(&item.chars);

                for (child, delta_cost) in children {
                    let next_cost = item
                        .cost
                        .checked_add(delta_cost)
                        .expect("cost overflow despite max_distance validation");

                    self.relax(child_depth, child, next_cost);
                }
            }

            if item.depth >= self.config.min_distance {
                self.stats.emitted += 1;

                return Some(Candidate {
                    text: Self::chars_to_string(&item.chars),
                    chars: item.chars,
                    distance: item.depth,
                    cost: item.cost,
                });
            }
        }

        None
    }
}
```

---

## 10. Test Suite

The tests should do three things:

```text
1. Check explicit golden examples.
2. Compare against the old exact enumerator for many small cases.
3. Check invariants/property-style behavior.
```

Do **not** compare this enumerator against `DiscoveryCandidateEnumerator`. Discovery order is intentionally different.

### 10.1 Unit Test Code

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn config(
        seed: &str,
        alphabet: Vec<char>,
        min_distance: usize,
        max_distance: usize,
        ops: EditOps,
        distance_mode: DistanceMode,
    ) -> SearchConfig {
        SearchConfig {
            seed: seed.to_string(),
            alphabet,
            min_distance,
            max_distance,
            ops,
            keyboard_neighbors: HashMap::new(),
            distance_mode,
        }
    }

    fn collect_triples(config: SearchConfig) -> Vec<(String, usize, u32)> {
        PipelinedOrderedCandidateEnumerator::new(config)
            .unwrap()
            .map(|c| (c.text, c.distance, c.cost))
            .collect()
    }

    #[test]
    fn exact_distance_one_matches_brief_example() {
        let cfg = config(
            "a",
            vec!['a', 'b'],
            1,
            1,
            EditOps::all(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        let expected = vec![
            ("".to_string(), 1, 2),
            ("aa".to_string(), 1, 2),
            ("ab".to_string(), 1, 2),
            ("ba".to_string(), 1, 2),
            ("b".to_string(), 1, 3),
        ];

        assert_eq!(got, expected);
    }

    #[test]
    fn swap_cost_sorts_before_delete_insert_replace() {
        let cfg = config(
            "ab",
            vec!['a', 'b'],
            1,
            1,
            EditOps::all(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        let expected = vec![
            ("ba".to_string(), 1, 1),

            ("a".to_string(), 1, 2),
            ("aab".to_string(), 1, 2),
            ("aba".to_string(), 1, 2),
            ("abb".to_string(), 1, 2),
            ("b".to_string(), 1, 2),
            ("bab".to_string(), 1, 2),

            ("aa".to_string(), 1, 3),
            ("bb".to_string(), 1, 3),
        ];

        assert_eq!(got, expected);
    }

    #[test]
    fn duplicate_deletes_are_deduplicated_within_layer() {
        let cfg = config(
            "aa",
            vec![],
            1,
            1,
            EditOps::delete_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(got, vec![("a".to_string(), 1, 2)]);
    }

    #[test]
    fn duplicate_insertions_are_deduplicated_within_layer() {
        let cfg = config(
            "a",
            vec!['a'],
            1,
            1,
            EditOps::insert_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(got, vec![("aa".to_string(), 1, 2)]);
    }

    #[test]
    fn identical_adjacent_swap_is_not_emitted() {
        let cfg = config(
            "aa",
            vec!['a'],
            1,
            1,
            EditOps::swap_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert!(got.is_empty());
    }

    #[test]
    fn replacement_to_same_char_is_not_an_edit() {
        let cfg = config(
            "a",
            vec!['a'],
            1,
            1,
            EditOps::replace_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert!(got.is_empty());
    }

    #[test]
    fn keyboard_neighbor_replacement_has_lower_cost() {
        let mut neighbors = HashMap::new();
        neighbors.insert('a', HashSet::from(['b']));

        let cfg = SearchConfig {
            seed: "a".to_string(),
            alphabet: vec!['b', 'c'],
            min_distance: 1,
            max_distance: 1,
            ops: EditOps::replace_only(),
            keyboard_neighbors: neighbors,
            distance_mode: DistanceMode::PerDistanceBestCost,
        };

        let got = collect_triples(cfg);

        assert_eq!(
            got,
            vec![
                ("b".to_string(), 1, 1),
                ("c".to_string(), 1, 3),
            ]
        );
    }

    #[test]
    fn min_distance_zero_emits_seed() {
        let cfg = config(
            "é",
            vec![],
            0,
            0,
            EditOps::none(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(got, vec![("é".to_string(), 0, 0)]);
    }

    #[test]
    fn unicode_delete_uses_char_boundaries_not_bytes() {
        let cfg = config(
            "éa",
            vec![],
            1,
            1,
            EditOps::delete_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(
            got,
            vec![
                ("a".to_string(), 1, 2),
                ("é".to_string(), 1, 2),
            ]
        );
    }

    #[test]
    fn unsorted_duplicate_alphabet_is_normalized() {
        let cfg = config(
            "a",
            vec!['b', 'a', 'b'],
            1,
            1,
            EditOps::all(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        let expected = vec![
            ("".to_string(), 1, 2),
            ("aa".to_string(), 1, 2),
            ("ab".to_string(), 1, 2),
            ("ba".to_string(), 1, 2),
            ("b".to_string(), 1, 3),
        ];

        assert_eq!(got, expected);
    }

    #[test]
    fn per_distance_mode_allows_reappearance_at_later_depth() {
        let cfg = config(
            "a",
            vec!['a', 'b'],
            0,
            2,
            EditOps::replace_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(
            got,
            vec![
                ("a".to_string(), 0, 0),
                ("b".to_string(), 1, 3),
                ("a".to_string(), 2, 6),
            ]
        );
    }

    #[test]
    fn global_minimum_distance_suppresses_later_reappearance() {
        let cfg = config(
            "a",
            vec!['a', 'b'],
            0,
            2,
            EditOps::replace_only(),
            DistanceMode::GlobalMinimumDistance,
        );

        let got = collect_triples(cfg);

        assert_eq!(
            got,
            vec![
                ("a".to_string(), 0, 0),
                ("b".to_string(), 1, 3),
            ]
        );
    }

    #[test]
    fn global_mode_marks_seed_seen_even_when_below_min_distance() {
        let cfg = config(
            "a",
            vec!['a', 'b'],
            1,
            2,
            EditOps::replace_only(),
            DistanceMode::GlobalMinimumDistance,
        );

        let got = collect_triples(cfg);

        assert_eq!(got, vec![("b".to_string(), 1, 3)]);
    }

    #[test]
    fn empty_seed_insert_only() {
        let cfg = config(
            "",
            vec!['b', 'a'],
            0,
            1,
            EditOps::insert_only(),
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(
            got,
            vec![
                ("".to_string(), 0, 0),
                ("a".to_string(), 1, 2),
                ("b".to_string(), 1, 2),
            ]
        );
    }

    #[test]
    fn empty_alphabet_still_allows_delete_and_swap() {
        let cfg = config(
            "ab",
            vec![],
            1,
            1,
            EditOps {
                delete: true,
                insert: false,
                replace: false,
                swap: true,
            },
            DistanceMode::PerDistanceBestCost,
        );

        let got = collect_triples(cfg);

        assert_eq!(
            got,
            vec![
                ("ba".to_string(), 1, 1),
                ("a".to_string(), 1, 2),
                ("b".to_string(), 1, 2),
            ]
        );
    }
}
```

---

## 11. Compatibility Tests Against Existing Exact Enumerator

Add a test module that compares the new enumerator against the existing exact ordered `CandidateEnumerator`.

Use small exhaustive inputs only.

Suggested test matrix:

```text
seeds:
  ""
  "a"
  "b"
  "aa"
  "ab"
  "aba"

alphabets:
  []
  ['a']
  ['a', 'b']
  ['b', 'a', 'b']    // unsorted and duplicate

distance bands:
  0..0
  0..1
  1..1
  0..2
  1..2

operation sets:
  none
  delete only
  insert only
  replace only
  swap only
  all

keyboard maps:
  empty
  {'a': {'b'}}
```

For each config:

```rust
let old = old_exact_enumerator.collect::<Vec<_>>();
let new = PipelinedOrderedCandidateEnumerator::new(config)
    .unwrap()
    .collect::<Vec<_>>();

assert_eq!(new, old);
```

Only run this comparison in:

```rust
DistanceMode::PerDistanceBestCost
```

Do not compare global minimum-distance mode against the old exact enumerator unless the old enumerator also globally deduplicates.

---

## 12. Property Tests / Invariant Tests

Even without `proptest`, add deterministic invariant checks.

For every collected output:

### 12.1 Distance is within band

```rust
assert!(candidate.distance >= min_distance);
assert!(candidate.distance <= max_distance);
```

### 12.2 Sorted order is correct

For consecutive outputs:

```rust
let prev_key = (prev.distance, prev.cost, prev.chars.clone());
let curr_key = (curr.distance, curr.cost, curr.chars.clone());

assert!(prev_key <= curr_key);
```

### 12.3 No duplicates within the same distance

For per-distance mode:

```rust
HashSet<(usize, Vec<char>)>
```

must not contain duplicates.

### 12.4 No duplicates globally in global-minimum mode

For global mode:

```rust
HashSet<Vec<char>>
```

must not contain duplicates.

### 12.5 No byte-splitting

For Unicode seeds, ensure candidates are valid strings and char counts match expected edit effects where applicable.

Example:

```text
seed = "éa"
delete only exact distance 1

outputs:
  "a"
  "é"
```

not invalid UTF-8 and not byte fragments.

---

## 13. Algorithmic Complexity

Let:

```text
n       = seed length in chars
A       = alphabet size
D       = max_distance
m       = current candidate length
U_d     = number of unique finalized candidates at depth d
B(m)    = raw one-edit branching factor for length m
H       = heap size
```

For all operations enabled:

```text
delete:       m
insert:       (m + 1) * A
replacement:  up to m * A
swap:         up to m - 1
```

More precisely:

```text
replacement count = Σ_i count(alphabet chars != parent[i])
```

So:

```text
B(m) = O(mA)
```

Since:

```text
m <= n + D
```

the worst local branching is:

```text
B_max = O((n + D)A)
```

Total raw neighbor events through depth `D`:

```text
E_D = Σ_{d=0}^{D-1} Σ_{s ∈ U_d} B(|s|)
```

Baseline implementation cost:

```text
Time:
  O(E_D * ((n + D) + log H))

Memory:
  O(H + Σ_d U_d * average_string_length)
```

The `n + D` factor appears because this baseline copies `Vec<char>` when generating children.

### 13.1 Heap size

Because the heap is depth-first, during processing of depth `d` it primarily contains:

```text
remaining depth d nodes
plus discovered depth d + 1 nodes
plus stale improved-cost entries
```

So practical heap size is closer to:

```text
O(U_d + U_{d+1})
```

rather than all depths at once.

However, because `best` maps are retained for simple stale checks, memory is:

```text
O(Σ_d U_d)
```

unless depth maps are explicitly freed after completion.

### 13.2 Stale heap entries

If a candidate is first discovered with cost `c1` and later with lower cost `c2`, both entries may exist in the heap.

The stale one is skipped when popped:

```rust
if best[item.depth][item.chars] != item.cost {
    continue;
}
```

With this cost model, each candidate’s cost can improve only a limited number of times in practice, but the theoretical upper bound is still tied to the number of relaxations.

This is acceptable for a first implementation.

### 13.3 Cost bucket range

Since each edit costs at most `3`:

```text
cost at depth d ∈ [0, 3d]
```

So each depth has at most:

```text
3d + 1
```

possible cost buckets.

This enables a future optimization using bucket queues instead of a binary heap.

---

## 14. Performance Characteristics

### 14.1 What improves

The proposed enumerator removes the large blocking operation:

```text
build entire next layer + sort entire next layer
```

Instead, work is distributed:

```text
one finalized parent expansion per emitted candidate
```

This should greatly reduce terminal “chunk then silence” behavior.

### 14.2 What does not improve

It does not reduce the total size of the edit neighborhood.

If `D` grows, the number of reachable strings still explodes.

Any exact enumerator must spend at least:

```text
Ω(number of emitted candidates)
```

and usually much more because of duplicate paths and deduplication.

### 14.3 Remaining pauses

There are still unavoidable pauses in some cases.

#### Case 1: `min_distance > 0`

If:

```text
min_distance = 3
```

then depths `0`, `1`, and `2` must be processed before any depth-3 candidate can be safely emitted.

That is inherent to exact distance-first ordering.

#### Case 2: Very large alphabet

One parent expansion costs roughly:

```text
O((n + D)A)
```

If `A` is huge, a single expansion may still be noticeable.

If this becomes a real issue, add a budgeted polling API later.

---

## 15. Optional Budgeted Polling API

For very large alphabets or high `min_distance`, `Iterator::next()` may still run too long before returning.

A more UI-friendly API can be added:

```rust
pub enum PollResult {
    Candidate(Candidate),
    Pending(EnumeratorStats),
    Exhausted,
}

pub fn poll_next_with_budget(&mut self, budget: usize) -> PollResult;
```

Where `budget` means something concrete, such as:

```text
maximum raw neighbor events
```

or:

```text
maximum heap pops / relaxations
```

This requires making neighbor generation resumable, because the current `one_edit_neighbors()` expands a whole parent at once.

Do not implement this first unless profiling proves it is needed. The heap-based pipelined enumerator is the correct first milestone.

---

## 16. Checkpointing Plan

If the current exact enumerator is checkpointable, this one can be too.

Checkpoint only between calls to `next()`.

A checkpoint should contain:

```rust
struct EnumeratorCheckpoint {
    version: u32,
    config: SearchConfig,
    heap_items: Vec<QueueItem>,
    best: Vec<HashMap<Vec<char>, u32>>,
    finalized_global: HashSet<Vec<char>>,
    stats: EnumeratorStats,
}
```

Restore with:

```rust
let heap = BinaryHeap::from(heap_items);
```

Do not assume `heap_items` is sorted. `BinaryHeap::from` rebuilds heap structure.

If using `serde`, derive:

```rust
Serialize
Deserialize
```

for:

```text
SearchConfig
EditOps
DistanceMode
Candidate
QueueItem
EnumeratorStats
EnumeratorCheckpoint
```

Checkpoint compatibility rules:

```text
1. Include a version number.
2. Do not restore checkpoints made with a different cost model.
3. Do not restore checkpoints made with a different alphabet normalization rule.
4. Do not restore checkpoints made with a different distance mode.
```

---

## 17. Implementation Milestones

### Milestone 1: Isolated neighbor generator

Implement and test:

```rust
one_edit_neighbors(parent: &[char]) -> HashMap<Vec<char>, u32>
```

Before integrating the heap.

Tests:

```text
delete duplicates from "aa"
insert duplicates into "a"
replace skips same char
swap skips identical adjacent chars
keyboard-neighbor replacement cost
Unicode char indexing
```

### Milestone 2: Heap ordering

Implement `QueueItem::Ord`.

Test directly:

```rust
let mut heap = BinaryHeap::new();

heap.push(depth 1, cost 2, "ba");
heap.push(depth 1, cost 2, "aa");
heap.push(depth 0, cost 0, "seed");
heap.push(depth 1, cost 1, "zz");

assert_eq!(pop, depth 0 ...);
assert_eq!(pop, depth 1 cost 1 ...);
assert_eq!(pop, depth 1 cost 2 "aa");
assert_eq!(pop, depth 1 cost 2 "ba");
```

This test catches a very common reversed-comparison bug.

### Milestone 3: Per-distance exact enumerator

Implement:

```rust
DistanceMode::PerDistanceBestCost
```

Compare against the existing exact `CandidateEnumerator` for small exhaustive cases.

This is the main compatibility gate.

### Milestone 4: Global minimum-distance mode

Implement:

```rust
DistanceMode::GlobalMinimumDistance
```

Do not enable it by default.

Add tests showing later-depth reappearances are suppressed.

### Milestone 5: Instrumentation

Expose stats:

```rust
enumerator.stats()
```

Use this for debugging and performance comparisons.

At minimum, record:

```text
popped
stale_skipped
expanded
raw_neighbors_generated
local_unique_neighbors
relaxed_new
relaxed_improved
relaxed_not_better
global_duplicate_skipped
emitted
```

### Milestone 6: Benchmarks

Benchmark:

```text
seed = "patter"
alphabet size ~= 91
max_distance = 1
max_distance = 2
```

Compare:

```text
old exact enumerator:
  time to first depth-2 candidate
  total time
  peak memory

new pipelined enumerator:
  time between depth-1 emissions
  time from final depth-1 candidate to first depth-2 candidate
  total time
  peak memory
```

The expected result is not necessarily lower total time. The expected result is much lower layer-boundary latency.

---

## 18. Key Things To Do

### Do preserve the exact key

Output order must be:

```text
(distance, cost, lexical)
```

The heap key must match this.

### Do expand finalized nodes even below `min_distance`

Without this, the enumerator cannot reach the requested band.

### Do store best cost per `(depth, candidate)`

This is the core dedup structure.

### Do skip stale heap entries

A stale heap entry is one whose cost no longer equals the best map.

### Do normalize alphabet once

Sort and deduplicate.

### Do keep keyboard-neighbor behavior explicit

Directed by default. Symmetric only if explicitly normalized.

### Do test against the old exact enumerator

This is the best way to avoid accidental compatibility regressions.

---

## 19. Key Things Not To Do

### Do not emit on discovery in ordered mode

This is the core bug in discovery-style streaming.

A candidate discovered early may later be found with:

```text
lower cost
```

or another candidate may later be discovered with:

```text
same cost but lexically earlier string
```

Discovery order cannot preserve the ordered contract.

### Do not globally deduplicate in compatibility mode

This changes semantics.

Wrong for compatibility:

```rust
seen_strings.insert(candidate)
```

Correct for compatibility:

```rust
best[depth].insert(candidate, cost)
```

### Do not scan lexicographic string space

Do not generate all strings of lengths:

```text
n - D .. n + D
```

over the alphabet and check edit distance.

For alphabet size `91` and lengths `4..8`, this is catastrophic.

### Do not use byte indices

Rust strings are UTF-8. Byte-index mutation will break Unicode correctness.

### Do not replace this with a textbook Levenshtein automaton without validating semantics

A standard Levenshtein or Damerau-Levenshtein automaton may not match your current mutation semantics, especially with:

```text
script depth
weighted likelihood costs
adjacent swaps
multiple paths
per-distance duplicate behavior
```

Automata are a promising later direction, but not a drop-in replacement for compatibility.

### Do not rely on `HashMap` iteration order

Hash maps are for lookup only. Output order must come from the heap key.

### Do not sort full layers

Sorting full layers brings back the pause this design is meant to remove.

---

## 20. Optional Optimization: Cost-Bucketed Queues

Because costs are small integers, a future alternative to the heap is:

```rust
Vec<Vec<BTreeSet<Vec<char>>>>
```

Conceptually:

```text
buckets[depth][cost] = lexically ordered candidates
```

Since:

```text
cost <= 3 * depth
```

the number of buckets per depth is small.

However, if a candidate’s cost improves, the implementation must either:

```text
1. remove it from the old bucket and insert it into the new bucket
```

or:

```text
2. allow stale bucket entries and check best[depth][candidate] before emitting
```

This is slightly more complex than the heap version.

Recommendation:

```text
Implement heap version first.
Only optimize to buckets if profiling shows heap overhead is material.
```

---

## 21. Optional Optimization: Reduce `Vec<char>` Cloning

The baseline implementation copies `Vec<char>` per generated child.

That is simple and safe, but expensive.

Possible later optimizations:

```text
1. Use Arc<[char]> for stored candidates.
2. Intern candidate strings.
3. Use a custom small-vector type for short strings.
4. Use rolling hashes with collision-safe final equality checks.
5. Generate child mutations into reusable buffers.
```

Do not start here. First preserve correctness.

---

## 22. Acceptance Criteria

The implementation is acceptable when all of the following are true:

```text
1. It passes all golden tests in this report.
2. It matches the old exact CandidateEnumerator for small exhaustive compatibility cases.
3. It emits candidates sorted by:
      distance,
      cost,
      lexical candidate.
4. It never emits duplicate candidates within the same distance in compatibility mode.
5. It optionally never emits duplicate candidate strings globally in global-minimum mode.
6. It handles min_distance = 0.
7. It handles max_distance = 0.
8. It handles empty seed.
9. It handles empty alphabet.
10. It handles disabled operations.
11. It does not byte-index strings.
12. It does not emit same-char replacements.
13. It does not emit swaps of identical adjacent chars.
14. It applies keyboard-neighbor replacement costs correctly.
15. It is deterministic across repeated runs.
16. It avoids the old full-layer build/sort pause.
```

---

## 23. Recommended Default Behavior

Use this as the default exact ordered mode:

```rust
DistanceMode::PerDistanceBestCost
```

because it best preserves the current compatibility contract:

```text
best known cost for candidate at that distance,
emit once per distance layer.
```

Expose global minimum-distance behavior only as an explicit option:

```rust
DistanceMode::GlobalMinimumDistance
```

Do not change the default until downstream consumers agree that candidates should be unique globally rather than unique per distance.

---

## 24. Summary for the Coding Agent

Implement:

```rust
PipelinedOrderedCandidateEnumerator
```

using:

```text
layered graph node: (depth, Vec<char>)
heap key:           (depth, cost, Vec<char>)
dedup key:          (depth, Vec<char>) in compatibility mode
optional dedup key: Vec<char> in global minimum-distance mode
```

The enumerator must:

```text
pop finalized candidate,
skip stale entries,
expand into next depth,
then emit if within min/max band.
```

The design is exact, deterministic, Unicode-safe under the stated `Vec<char>` model, and avoids the current layer-sized blocking build/sort operation.

The hardest mistakes to avoid are:

```text
emitting on discovery,
using global dedup in compatibility mode,
forgetting to expand below min_distance,
using byte indices,
and relying on HashMap iteration order.
```
