use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{config_hash, ConfigError, DistanceMode, EditOps, SearchConfig, SnapshotError};

// ---------------------------------------------------------------------------
// Constants (costs, matching the graph enumerator)
// ---------------------------------------------------------------------------

const DELETE_COST: u32 = 2;
const INSERT_COST: u32 = 2;
const KEYBOARD_REPLACE_COST: u32 = 1;
const NORMAL_REPLACE_COST: u32 = 3;

// ---------------------------------------------------------------------------
// Score
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        Self { edits, cost: self.cost.saturating_add(cost_delta) }
    }
}

fn better(a: Score, b: Score) -> Score {
    if (a.edits, a.cost) <= (b.edits, b.cost) { a } else { b }
}

// ---------------------------------------------------------------------------
// DP helpers
// ---------------------------------------------------------------------------

fn initial_row(seed: &[char], config: &SearchConfig) -> Vec<Score> {
    let seed_len = seed.len();
    let mut row = vec![Score::inf(); seed_len + 1];
    row[0] = Score::zero();
    for j in 1..=seed_len {
        if config.ops.delete {
            row[j] = row[j - 1].add(1, DELETE_COST, config.max_distance);
        }
    }
    row
}

fn is_keyboard_neighbor(config: &SearchConfig, from: char, to: char) -> bool {
    config.keyboard_neighbors.get(&from).is_some_and(|s| s.contains(&to))
}

fn step_row(
    seed: &[char],
    prev: &[Score],
    ch: char,
    ch_is_in_raw_alphabet: bool,
    config: &SearchConfig,
) -> Vec<Score> {
    let n = seed.len();
    let mut curr = vec![Score::inf(); n + 1];

    // Insert candidate char ch against empty seed prefix.
    if config.ops.insert && ch_is_in_raw_alphabet {
        curr[0] = prev[0].add(1, INSERT_COST, config.max_distance);
    }

    for j in 1..=n {
        let mut best = Score::inf();

        // Delete seed[j-1]
        if config.ops.delete {
            best = better(best, curr[j - 1].add(1, DELETE_COST, config.max_distance));
        }

        // Match or replace seed[j-1] with ch
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

        // Insert candidate char ch (seed prefix length stays j)
        if config.ops.insert && ch_is_in_raw_alphabet {
            best = better(best, prev[j].add(1, INSERT_COST, config.max_distance));
        }

        curr[j] = best;
    }

    curr
}

fn length_lower_bound(
    remaining_seed: usize,
    remaining_candidate: usize,
    ops: EditOps,
) -> Option<usize> {
    match (ops.insert, ops.delete) {
        (true, true) => Some(remaining_seed.abs_diff(remaining_candidate)),
        (true, false) => {
            if remaining_candidate >= remaining_seed {
                Some(remaining_candidate - remaining_seed)
            } else {
                None
            }
        }
        (false, true) => {
            if remaining_seed >= remaining_candidate {
                Some(remaining_seed - remaining_candidate)
            } else {
                None
            }
        }
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

// ---------------------------------------------------------------------------
// Public stats / snapshot types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamingEnumeratorStats {
    pub prefixes_visited: u64,
    pub prefixes_pruned: u64,
    pub leaves_checked: u64,
    pub emitted: u64,
    pub rows_computed: u64,
}

/// A serializable DFS-stack snapshot for the streaming enumerator.
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
    row: Vec<Score>,
    next_symbol_index: usize,
}

// ---------------------------------------------------------------------------
// Internal frame
// ---------------------------------------------------------------------------

struct StreamingFrame {
    row: Vec<Score>,
    next_symbol_index: usize,
}

// ---------------------------------------------------------------------------
// Main enumerator
// ---------------------------------------------------------------------------

pub struct StreamingLevenshteinCandidateEnumerator {
    config: SearchConfig,
    seed: Vec<char>,
    /// Characters that are legal to insert or use as a replacement target.
    raw_alphabet: HashSet<char>,
    /// Sorted dedup(alphabet ∪ seed chars) — the universe of characters used
    /// to build candidates in the prefix tree.
    generation_alphabet: Vec<char>,
    #[allow(dead_code)]
    min_target_len: usize,
    max_target_len: usize,
    target_len: usize,
    prefix: Vec<char>,
    stack: Vec<StreamingFrame>,
    emitted: u64,
    stats: StreamingEnumeratorStats,
}

impl StreamingLevenshteinCandidateEnumerator {
    pub fn new(mut config: SearchConfig) -> Result<Self, ConfigError> {
        // Validate: swap is unsupported in streaming mode.
        if config.ops.swap {
            return Err(ConfigError::UnsupportedForStreaming {
                reason: "streaming Levenshtein MVP supports insert/delete/replace, not swap"
                    .to_string(),
            });
        }

        // Validate: only GlobalMinimumDistance is supported.
        if config.distance_mode != DistanceMode::GlobalMinimumDistance {
            return Err(ConfigError::UnsupportedForStreaming {
                reason: "streaming mode requires GlobalMinimumDistance".to_string(),
            });
        }

        // Validate distance band.
        if config.min_distance > config.max_distance {
            return Err(ConfigError::InvalidDistanceBand {
                min: config.min_distance,
                max: config.max_distance,
            });
        }

        // Normalize alphabet.
        config.alphabet.sort_unstable();
        config.alphabet.dedup();

        let seed: Vec<char> = config.seed.chars().collect();
        let seed_len = seed.len();

        let raw_alphabet: HashSet<char> = config.alphabet.iter().copied().collect();

        // generation_alphabet = sorted_dedup(alphabet ∪ seed chars)
        let mut gen_alpha: Vec<char> = config.alphabet.clone();
        for &ch in &seed {
            if !raw_alphabet.contains(&ch) {
                gen_alpha.push(ch);
            }
        }
        gen_alpha.sort_unstable();
        gen_alpha.dedup();

        let min_target_len = if config.ops.delete {
            seed_len.saturating_sub(config.max_distance)
        } else {
            seed_len
        };

        let max_target_len = if config.ops.insert {
            seed_len + config.max_distance
        } else {
            seed_len
        };

        let target_len = min_target_len;

        let mut enumerator = Self {
            config,
            seed,
            raw_alphabet,
            generation_alphabet: gen_alpha,
            min_target_len,
            max_target_len,
            target_len,
            prefix: Vec::new(),
            stack: Vec::new(),
            emitted: 0,
            stats: StreamingEnumeratorStats::default(),
        };

        enumerator.init_target_len();
        Ok(enumerator)
    }

    fn init_target_len(&mut self) {
        self.prefix.clear();
        self.stack.clear();
        let root_row = initial_row(&self.seed, &self.config);
        self.stats.rows_computed += 1;
        self.stack.push(StreamingFrame { row: root_row, next_symbol_index: 0 });
    }

    fn backtrack_one(&mut self) {
        self.stack.pop();
        if !self.prefix.is_empty() {
            self.prefix.pop();
        }
    }

    pub fn snapshot(&self) -> StreamingEnumeratorSnapshot {
        let stack: Vec<StreamingFrameSnapshot> = self
            .stack
            .iter()
            .map(|f| StreamingFrameSnapshot {
                row: f.row.clone(),
                next_symbol_index: f.next_symbol_index,
            })
            .collect();
        StreamingEnumeratorSnapshot {
            schema_version: 1,
            config_hash: config_hash(&self.config),
            target_len: self.target_len,
            prefix: self.prefix.clone(),
            stack,
            emitted: self.emitted,
            stats: self.stats.clone(),
        }
    }

    pub fn from_snapshot(
        mut config: SearchConfig,
        snapshot: StreamingEnumeratorSnapshot,
    ) -> Result<Self, SnapshotError> {
        if snapshot.schema_version != 1 {
            return Err(SnapshotError::UnsupportedVersion { got: snapshot.schema_version });
        }

        // Normalize before hashing.
        config.alphabet.sort_unstable();
        config.alphabet.dedup();

        let expected_hash = config_hash(&config);
        if snapshot.config_hash != expected_hash {
            return Err(SnapshotError::ConfigHashMismatch {
                expected: expected_hash,
                got: snapshot.config_hash,
            });
        }

        let seed: Vec<char> = config.seed.chars().collect();
        let seed_len = seed.len();
        let raw_alphabet: HashSet<char> = config.alphabet.iter().copied().collect();

        let mut gen_alpha: Vec<char> = config.alphabet.clone();
        for &ch in &seed {
            if !raw_alphabet.contains(&ch) {
                gen_alpha.push(ch);
            }
        }
        gen_alpha.sort_unstable();
        gen_alpha.dedup();

        let min_target_len = if config.ops.delete {
            seed_len.saturating_sub(config.max_distance)
        } else {
            seed_len
        };

        let max_target_len = if config.ops.insert {
            seed_len + config.max_distance
        } else {
            seed_len
        };

        let stack: Vec<StreamingFrame> = snapshot
            .stack
            .into_iter()
            .map(|f| StreamingFrame { row: f.row, next_symbol_index: f.next_symbol_index })
            .collect();

        Ok(Self {
            config,
            seed,
            raw_alphabet,
            generation_alphabet: gen_alpha,
            min_target_len,
            max_target_len,
            target_len: snapshot.target_len,
            prefix: snapshot.prefix,
            stack,
            emitted: snapshot.emitted,
            stats: snapshot.stats,
        })
    }

    pub fn next_ordinal(&self) -> u64 {
        self.emitted
    }

    pub fn stats(&self) -> &StreamingEnumeratorStats {
        &self.stats
    }
}

impl Iterator for StreamingLevenshteinCandidateEnumerator {
    type Item = crate::Candidate;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If stack is empty, advance to the next target length.
            if self.stack.is_empty() {
                self.target_len += 1;
                if self.target_len > self.max_target_len {
                    return None;
                }
                self.init_target_len();
            }

            // Also check upfront (handles the initial target_len > max_target_len case).
            if self.target_len > self.max_target_len {
                return None;
            }

            // Leaf node: full candidate string assembled.
            if self.prefix.len() == self.target_len {
                let final_score = self.stack.last().unwrap().row[self.seed.len()];
                let leaf_prefix = self.prefix.clone();
                self.stats.leaves_checked += 1;
                self.backtrack_one();

                let edits = final_score.edits as usize;
                if final_score.edits != u16::MAX
                    && edits >= self.config.min_distance
                    && edits <= self.config.max_distance
                {
                    let ordinal = self.emitted;
                    self.emitted += 1;
                    self.stats.emitted += 1;
                    let text: String = leaf_prefix.iter().collect();
                    return Some(crate::Candidate {
                        ordinal,
                        text,
                        chars: leaf_prefix,
                        distance: edits,
                        cost: final_score.cost,
                    });
                }

                continue;
            }

            // Interior node: try next symbol.
            let next_sym_idx = self.stack.last().unwrap().next_symbol_index;
            if next_sym_idx >= self.generation_alphabet.len() {
                self.backtrack_one();
                continue;
            }

            // Advance the symbol index on the current frame (borrow ends here).
            self.stack.last_mut().unwrap().next_symbol_index += 1;

            let ch = self.generation_alphabet[next_sym_idx];
            let ch_in_raw = self.raw_alphabet.contains(&ch);

            // Compute next DP row.
            let next_row = {
                let parent_row = &self.stack.last().unwrap().row;
                step_row(&self.seed, parent_row, ch, ch_in_raw, &self.config)
            };
            self.stats.rows_computed += 1;

            let remaining_after_push = self.target_len - self.prefix.len() - 1;

            if !can_still_reach(
                &next_row,
                self.seed.len(),
                remaining_after_push,
                self.config.ops,
                self.config.max_distance,
            ) {
                self.stats.prefixes_pruned += 1;
                continue;
            }

            self.stats.prefixes_visited += 1;
            self.prefix.push(ch);
            self.stack.push(StreamingFrame { row: next_row, next_symbol_index: 0 });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap, HashSet};

    use super::*;
    use crate::{Candidate, DistanceMode, EditOps, PipelinedOrderedCandidateEnumerator, SearchConfig};

    fn make_config(
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

    fn collect_set<I: Iterator<Item = Candidate>>(iter: I) -> BTreeSet<(String, usize, u32)> {
        iter.map(|c| (c.text, c.distance, c.cost)).collect()
    }

    // -----------------------------------------------------------------------
    // Score unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn score_add_caps_at_max_distance() {
        let s = Score { edits: 2, cost: 4 };
        assert_eq!(s.add(1, 2, 3), Score { edits: 3, cost: 6 });
        assert_eq!(s.add(2, 2, 3), Score::inf());
    }

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

    // -----------------------------------------------------------------------
    // Row transition tests
    // -----------------------------------------------------------------------

    #[test]
    fn streaming_row_delete_only_handles_empty_candidate() {
        let cfg = make_config(
            "ab",
            vec![],
            2,
            2,
            EditOps::delete_only(),
            DistanceMode::GlobalMinimumDistance,
        );
        let seed: Vec<char> = cfg.seed.chars().collect();
        let row = initial_row(&seed, &cfg);
        assert_eq!(row[2].edits, 2);
        assert_eq!(row[2].cost, 4);
    }

    #[test]
    fn streaming_insert_only_from_empty_seed() {
        let cfg = make_config(
            "",
            vec!['a'],
            1,
            1,
            EditOps::insert_only(),
            DistanceMode::GlobalMinimumDistance,
        );
        let got: Vec<_> = StreamingLevenshteinCandidateEnumerator::new(cfg).unwrap().collect();
        assert_eq!(got.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(got[0].distance, 1);
        assert_eq!(got[0].cost, 2);
    }

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

    // -----------------------------------------------------------------------
    // Cross-check against graph enumerator
    // -----------------------------------------------------------------------

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

        for seed in &seeds {
            for &alphabet in alphabets {
                for &(min, max) in &bands {
                    for &ops in &ops_set {
                        let cfg = SearchConfig {
                            seed: seed.to_string(),
                            alphabet: alphabet.to_vec(),
                            min_distance: min,
                            max_distance: max,
                            ops,
                            keyboard_neighbors: HashMap::new(),
                            distance_mode: DistanceMode::GlobalMinimumDistance,
                        };

                        let graph = collect_set(
                            PipelinedOrderedCandidateEnumerator::new(cfg.clone()).unwrap(),
                        );
                        let streaming = collect_set(
                            StreamingLevenshteinCandidateEnumerator::new(cfg.clone()).unwrap(),
                        );
                        assert_eq!(
                            streaming, graph,
                            "mismatch for seed={:?} alphabet={:?} band=({},{}) ops={:?}",
                            seed, alphabet, min, max, ops
                        );
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Seed char outside alphabet
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Snapshot / restore
    // -----------------------------------------------------------------------

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

        let full: Vec<_> =
            StreamingLevenshteinCandidateEnumerator::new(cfg.clone()).unwrap().collect();

        let split_at = full.len() / 2;
        let mut first = StreamingLevenshteinCandidateEnumerator::new(cfg.clone()).unwrap();
        let prefix_half: Vec<_> = first.by_ref().take(split_at).collect();
        let snapshot = first.snapshot();
        let restored =
            StreamingLevenshteinCandidateEnumerator::from_snapshot(cfg, snapshot).unwrap();
        let suffix: Vec<_> = restored.collect();

        let combined: Vec<_> = prefix_half.into_iter().chain(suffix).collect();
        assert_eq!(combined, full);
    }

    // -----------------------------------------------------------------------
    // Large smoke test (ignored by default)
    // -----------------------------------------------------------------------

    #[test]
    #[ignore]
    fn streaming_large_search_does_not_grow_memory() {
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

        assert!(e.stats().rows_computed > 0);
    }
}
