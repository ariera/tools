use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMode {
    /// A candidate may appear once per distance layer.
    /// Deduplication key is (depth, candidate).
    PerDistanceBestCost,

    /// A candidate string is emitted only once, at its first reachable depth.
    /// Deduplication key is candidate.
    GlobalMinimumDistance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    UnsupportedForStreaming { reason: String },
    EstimatedSearchTooLarge { estimated: u64 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDistanceBand { min, max } => {
                write!(f, "min_distance ({min}) must not exceed max_distance ({max})")
            }
            Self::MaxDistanceTooLarge { max } => {
                write!(f, "max_distance ({max}) is too large; would cause cost overflow")
            }
            Self::UnsupportedForStreaming { reason } => {
                write!(f, "config is not supported by streaming enumerator: {reason}")
            }
            Self::EstimatedSearchTooLarge { estimated } => {
                write!(
                    f,
                    "estimated search space ({estimated} states) is too large for graph enumerator; \
                     use streaming strategy instead"
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Config hashing
// ---------------------------------------------------------------------------

/// Deterministic representation of SearchConfig for hashing.
///
/// HashMap and HashSet have non-stable iteration order, so the config must be
/// canonicalized (sorted, deduped) before serialization.
#[derive(Serialize)]
struct CanonicalSearchConfig<'a> {
    seed: &'a str,
    alphabet: Vec<char>,
    min_distance: usize,
    max_distance: usize,
    ops: EditOps,
    keyboard_neighbors: Vec<(char, Vec<char>)>,
    distance_mode: DistanceMode,
}

fn canonical_config(config: &SearchConfig) -> CanonicalSearchConfig<'_> {
    let mut alphabet = config.alphabet.clone();
    alphabet.sort_unstable();
    alphabet.dedup();

    let mut keyboard_neighbors: Vec<(char, Vec<char>)> = config
        .keyboard_neighbors
        .iter()
        .map(|(&from, tos)| {
            let mut tos: Vec<char> = tos.iter().copied().collect();
            tos.sort_unstable();
            tos.dedup();
            (from, tos)
        })
        .collect();
    keyboard_neighbors.sort_by_key(|(from, _)| *from);

    CanonicalSearchConfig {
        seed: &config.seed,
        alphabet,
        min_distance: config.min_distance,
        max_distance: config.max_distance,
        ops: config.ops,
        keyboard_neighbors,
        distance_mode: config.distance_mode,
    }
}

/// Return a stable hex string that uniquely identifies a search configuration.
///
/// The hash is stable across runs for the same logical config regardless of
/// alphabet order or keyboard-neighbor map iteration order.
pub fn config_hash(config: &SearchConfig) -> String {
    let canonical = canonical_config(config);
    let bytes =
        serde_json::to_vec(&canonical).expect("canonical SearchConfig is always serializable");
    blake3::hash(&bytes).to_hex().to_string()
}
