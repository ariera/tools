use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceMode {
    /// A candidate may appear once per distance layer.
    /// Deduplication key is (depth, candidate).
    PerDistanceBestCost,

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
        Self { delete: true, insert: true, replace: true, swap: true }
    }

    pub fn none() -> Self {
        Self { delete: false, insert: false, replace: false, swap: false }
    }

    pub fn delete_only() -> Self {
        Self { delete: true, insert: false, replace: false, swap: false }
    }

    pub fn insert_only() -> Self {
        Self { delete: false, insert: true, replace: false, swap: false }
    }

    pub fn replace_only() -> Self {
        Self { delete: false, insert: false, replace: true, swap: false }
    }

    pub fn swap_only() -> Self {
        Self { delete: false, insert: false, replace: false, swap: true }
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
    /// If keyboard_neighbors['a'] contains 'b', replacing a -> b costs 1.
    /// Does not imply b -> a unless that entry is also present.
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

/// Reversed ordering so BinaryHeap (max-heap) pops smallest key first:
///   depth ascending, cost ascending, lexical chars ascending.
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
        if config.max_distance > (u32::MAX as usize / 3) {
            return Err(ConfigError::MaxDistanceTooLarge { max: config.max_distance });
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
        heap.push(QueueItem { depth: 0, cost: 0, chars: seed_chars });

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

    fn record_local_neighbor(local: &mut HashMap<Vec<char>, u32>, child: Vec<char>, delta_cost: u32) {
        match local.entry(child) {
            Entry::Vacant(v) => { v.insert(delta_cost); }
            Entry::Occupied(mut o) => {
                if delta_cost < *o.get() {
                    *o.get_mut() = delta_cost;
                }
            }
        }
    }

    /// Generate all unique one-edit neighbors of `parent`, returning cheapest single-edit cost per child.
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
                    let delta_cost = if self.is_keyboard_neighbor(original, replacement) { 1 } else { 3 };
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

            // Skip stale entries left in the heap after a cheaper path was discovered.
            if best_cost != Some(item.cost) {
                self.stats.stale_skipped += 1;
                continue;
            }

            if self.config.distance_mode == DistanceMode::GlobalMinimumDistance {
                if self.finalized_global.contains(&item.chars) {
                    self.stats.global_duplicate_skipped += 1;
                    continue;
                }
                // Mark finalized even below min_distance to prevent re-emission at later depths.
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

    // --- Heap ordering ---

    #[test]
    fn heap_ordering_pops_in_correct_order() {
        let mut heap = BinaryHeap::new();
        heap.push(QueueItem { depth: 1, cost: 2, chars: vec!['b', 'a'] });
        heap.push(QueueItem { depth: 1, cost: 2, chars: vec!['a', 'a'] });
        heap.push(QueueItem { depth: 0, cost: 0, chars: vec!['s', 'e', 'e', 'd'] });
        heap.push(QueueItem { depth: 1, cost: 1, chars: vec!['z', 'z'] });

        let first = heap.pop().unwrap();
        assert_eq!(first.depth, 0);

        let second = heap.pop().unwrap();
        assert_eq!((second.depth, second.cost), (1, 1));

        let third = heap.pop().unwrap();
        assert_eq!(third.chars, vec!['a', 'a']);

        let fourth = heap.pop().unwrap();
        assert_eq!(fourth.chars, vec!['b', 'a']);
    }

    // --- Golden examples ---

    #[test]
    fn exact_distance_one_matches_brief_example() {
        let cfg = config("a", vec!['a', 'b'], 1, 1, EditOps::all(), DistanceMode::PerDistanceBestCost);
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
        let cfg = config("ab", vec!['a', 'b'], 1, 1, EditOps::all(), DistanceMode::PerDistanceBestCost);
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

    // --- Deduplication ---

    #[test]
    fn duplicate_deletes_are_deduplicated_within_layer() {
        let cfg = config("aa", vec![], 1, 1, EditOps::delete_only(), DistanceMode::PerDistanceBestCost);
        let got = collect_triples(cfg);
        assert_eq!(got, vec![("a".to_string(), 1, 2)]);
    }

    #[test]
    fn duplicate_insertions_are_deduplicated_within_layer() {
        let cfg = config("a", vec!['a'], 1, 1, EditOps::insert_only(), DistanceMode::PerDistanceBestCost);
        let got = collect_triples(cfg);
        assert_eq!(got, vec![("aa".to_string(), 1, 2)]);
    }

    // --- Edge operations ---

    #[test]
    fn identical_adjacent_swap_is_not_emitted() {
        let cfg = config("aa", vec!['a'], 1, 1, EditOps::swap_only(), DistanceMode::PerDistanceBestCost);
        let got = collect_triples(cfg);
        assert!(got.is_empty());
    }

    #[test]
    fn replacement_to_same_char_is_not_an_edit() {
        let cfg = config("a", vec!['a'], 1, 1, EditOps::replace_only(), DistanceMode::PerDistanceBestCost);
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
        assert_eq!(got, vec![("b".to_string(), 1, 1), ("c".to_string(), 1, 3)]);
    }

    // --- Unicode ---

    #[test]
    fn min_distance_zero_emits_seed() {
        let cfg = config("é", vec![], 0, 0, EditOps::none(), DistanceMode::PerDistanceBestCost);
        let got = collect_triples(cfg);
        assert_eq!(got, vec![("é".to_string(), 0, 0)]);
    }

    #[test]
    fn unicode_delete_uses_char_boundaries_not_bytes() {
        let cfg = config("éa", vec![], 1, 1, EditOps::delete_only(), DistanceMode::PerDistanceBestCost);
        let got = collect_triples(cfg);
        assert_eq!(
            got,
            vec![
                ("a".to_string(), 1, 2),
                ("é".to_string(), 1, 2),
            ]
        );
    }

    // --- Alphabet normalization ---

    #[test]
    fn unsorted_duplicate_alphabet_is_normalized() {
        let cfg = config("a", vec!['b', 'a', 'b'], 1, 1, EditOps::all(), DistanceMode::PerDistanceBestCost);
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

    // --- Distance modes ---

    #[test]
    fn per_distance_mode_allows_reappearance_at_later_depth() {
        let cfg = config("a", vec!['a', 'b'], 0, 2, EditOps::replace_only(), DistanceMode::PerDistanceBestCost);
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
        let cfg = config("a", vec!['a', 'b'], 0, 2, EditOps::replace_only(), DistanceMode::GlobalMinimumDistance);
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
        let cfg = config("a", vec!['a', 'b'], 1, 2, EditOps::replace_only(), DistanceMode::GlobalMinimumDistance);
        let got = collect_triples(cfg);
        assert_eq!(got, vec![("b".to_string(), 1, 3)]);
    }

    // --- Edge cases ---

    #[test]
    fn empty_seed_insert_only() {
        let cfg = config("", vec!['b', 'a'], 0, 1, EditOps::insert_only(), DistanceMode::PerDistanceBestCost);
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
            EditOps { delete: true, insert: false, replace: false, swap: true },
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

    #[test]
    fn max_distance_zero_emits_only_seed() {
        let cfg = config("hello", vec!['a', 'b'], 0, 0, EditOps::all(), DistanceMode::PerDistanceBestCost);
        let got = collect_triples(cfg);
        assert_eq!(got, vec![("hello".to_string(), 0, 0)]);
    }

    #[test]
    fn invalid_distance_band_returns_error() {
        let cfg = config("a", vec!['a'], 3, 1, EditOps::all(), DistanceMode::PerDistanceBestCost);
        let result = PipelinedOrderedCandidateEnumerator::new(cfg);
        assert!(matches!(result, Err(ConfigError::InvalidDistanceBand { min: 3, max: 1 })));
    }

    // --- Invariant checks over multiple configs ---

    struct InvariantChecker {
        seen: HashSet<(usize, Vec<char>)>,
        seen_global: HashSet<Vec<char>>,
        last_key: Option<(usize, u32, Vec<char>)>,
        min_distance: usize,
        max_distance: usize,
        mode: DistanceMode,
    }

    impl InvariantChecker {
        fn new(min_distance: usize, max_distance: usize, mode: DistanceMode) -> Self {
            Self {
                seen: HashSet::new(),
                seen_global: HashSet::new(),
                last_key: None,
                min_distance,
                max_distance,
                mode,
            }
        }

        fn check(&mut self, c: &Candidate) {
            // Distance is within band.
            assert!(c.distance >= self.min_distance, "distance {} < min {}", c.distance, self.min_distance);
            assert!(c.distance <= self.max_distance, "distance {} > max {}", c.distance, self.max_distance);

            // Sorted order is correct.
            let key = (c.distance, c.cost, c.chars.clone());
            if let Some(ref prev) = self.last_key {
                assert!(
                    *prev <= key,
                    "out of order: prev={:?} curr={:?}",
                    prev,
                    key
                );
            }
            self.last_key = Some(key);

            // No duplicates within the same distance.
            let per_depth_key = (c.distance, c.chars.clone());
            assert!(
                self.seen.insert(per_depth_key),
                "duplicate (distance, chars) for {:?} at distance {}",
                c.text,
                c.distance
            );

            // No global duplicates in global mode.
            if self.mode == DistanceMode::GlobalMinimumDistance {
                assert!(
                    self.seen_global.insert(c.chars.clone()),
                    "global duplicate {:?}",
                    c.text
                );
            }

            // Valid UTF-8.
            assert!(!c.text.is_empty() || c.chars.is_empty());
            let rebuilt: String = c.chars.iter().collect();
            assert_eq!(rebuilt, c.text, "text/chars mismatch");
        }
    }

    fn check_invariants(cfg: SearchConfig) {
        let min = cfg.min_distance;
        let max = cfg.max_distance;
        let mode = cfg.distance_mode;
        let mut checker = InvariantChecker::new(min, max, mode);

        PipelinedOrderedCandidateEnumerator::new(cfg)
            .unwrap()
            .for_each(|c| checker.check(&c));
    }

    #[test]
    fn invariants_hold_for_matrix_of_small_configs() {
        let seeds = ["", "a", "b", "aa", "ab", "aba"];
        let alphabets: &[&[char]] = &[
            &[],
            &['a'],
            &['a', 'b'],
            &['b', 'a', 'b'],
        ];
        let bands = [(0, 0), (0, 1), (1, 1), (0, 2), (1, 2)];
        let ops_set = [
            EditOps::none(),
            EditOps::delete_only(),
            EditOps::insert_only(),
            EditOps::replace_only(),
            EditOps::swap_only(),
            EditOps::all(),
        ];
        let modes = [DistanceMode::PerDistanceBestCost, DistanceMode::GlobalMinimumDistance];

        for seed in &seeds {
            for &alphabet in alphabets {
                for &(min, max) in &bands {
                    for &ops in &ops_set {
                        for &mode in &modes {
                            let mut neighbors = HashMap::new();
                            neighbors.insert('a', HashSet::from(['b']));

                            let cfg = SearchConfig {
                                seed: seed.to_string(),
                                alphabet: alphabet.to_vec(),
                                min_distance: min,
                                max_distance: max,
                                ops,
                                keyboard_neighbors: neighbors,
                                distance_mode: mode,
                            };
                            check_invariants(cfg);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn stats_are_consistent() {
        let cfg = config("ab", vec!['a', 'b'], 0, 2, EditOps::all(), DistanceMode::PerDistanceBestCost);
        let mut enumerator = PipelinedOrderedCandidateEnumerator::new(cfg).unwrap();
        let results: Vec<_> = enumerator.by_ref().collect();
        let stats = enumerator.stats();

        assert_eq!(stats.emitted, results.len() as u64);
        assert!(stats.popped >= stats.emitted);
        assert!(stats.popped >= stats.stale_skipped + stats.emitted);
        assert!(stats.raw_neighbors_generated >= stats.local_unique_neighbors);
        assert_eq!(stats.relaxed_new + stats.relaxed_improved + stats.relaxed_not_better,
                   stats.local_unique_neighbors);
    }
}
