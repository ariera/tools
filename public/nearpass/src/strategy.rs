use serde::{Deserialize, Serialize};

use crate::config::{ConfigError, SearchConfig};
use crate::enumerator::{
    Candidate, EnumeratorSnapshot, PipelinedOrderedCandidateEnumerator, SnapshotError,
};
use crate::streaming::{StreamingEnumeratorSnapshot, StreamingLevenshteinCandidateEnumerator};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnumeratorStrategy {
    Auto,
    OrderedGraph,
    StreamingLevenshtein,
}

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

impl CandidateEnumerator {
    pub fn next_ordinal(&self) -> u64 {
        match self {
            Self::Ordered(e) => e.next_ordinal(),
            Self::Streaming(e) => e.next_ordinal(),
        }
    }

    pub fn snapshot_any(&self) -> AnyEnumeratorSnapshot {
        match self {
            Self::Ordered(e) => AnyEnumeratorSnapshot::Graph(e.snapshot()),
            Self::Streaming(e) => AnyEnumeratorSnapshot::Streaming(e.snapshot()),
        }
    }

    pub fn from_any_snapshot(
        config: SearchConfig,
        snap: AnyEnumeratorSnapshot,
    ) -> Result<Self, SnapshotError> {
        match snap {
            AnyEnumeratorSnapshot::Graph(s) => {
                Ok(Self::Ordered(PipelinedOrderedCandidateEnumerator::from_snapshot(config, s)?))
            }
            AnyEnumeratorSnapshot::Streaming(s) => {
                Ok(Self::Streaming(StreamingLevenshteinCandidateEnumerator::from_snapshot(config, s)?))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnyEnumeratorSnapshot {
    Graph(EnumeratorSnapshot),
    Streaming(StreamingEnumeratorSnapshot),
}

// ---------------------------------------------------------------------------
// Graph-size guard helpers
// ---------------------------------------------------------------------------

const MAX_GRAPH_STATES: u64 = 5_000_000;

fn binom_coeff(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    if k == 0 || k == n {
        return 1;
    }
    let k = k.min(n - k);
    let mut result: u64 = 1;
    for i in 0..k {
        result = result.saturating_mul((n - i) as u64);
        result /= (i + 1) as u64;
    }
    result
}

fn replacement_lower_bound(seed_len: usize, alphabet_len: usize, max_distance: usize) -> u64 {
    // sum over k in 1..=min(max_distance, seed_len) of C(seed_len, k) * (alphabet_len-1)^k
    let choices = alphabet_len.saturating_sub(1) as u64;
    let mut total: u64 = 0;
    for k in 1..=max_distance.min(seed_len) {
        let combinations = binom_coeff(seed_len, k);
        let replacements = choices.saturating_pow(k as u32);
        total = total.saturating_add(combinations.saturating_mul(replacements));
    }
    total
}

fn graph_estimate_is_safe(config: &SearchConfig) -> bool {
    replacement_lower_bound(
        config.seed.chars().count(),
        config.alphabet.len(),
        config.max_distance,
    ) < MAX_GRAPH_STATES
}

fn validate_graph_size(config: &SearchConfig) -> Result<(), ConfigError> {
    let est = replacement_lower_bound(
        config.seed.chars().count(),
        config.alphabet.len(),
        config.max_distance,
    );
    if est >= MAX_GRAPH_STATES {
        Err(ConfigError::EstimatedSearchTooLarge { estimated: est })
    } else {
        Ok(())
    }
}

/// Factory that creates the appropriate enumerator based on strategy.
///
/// `Auto` picks `OrderedGraph` when the estimated state count is below
/// `MAX_GRAPH_STATES` (5 million); otherwise falls back to `StreamingLevenshtein`.
pub fn make_candidate_enumerator(
    config: SearchConfig,
    strategy: EnumeratorStrategy,
) -> Result<CandidateEnumerator, ConfigError> {
    match strategy {
        EnumeratorStrategy::OrderedGraph => {
            validate_graph_size(&config)?;
            Ok(CandidateEnumerator::Ordered(PipelinedOrderedCandidateEnumerator::new(config)?))
        }
        EnumeratorStrategy::StreamingLevenshtein => {
            Ok(CandidateEnumerator::Streaming(
                StreamingLevenshteinCandidateEnumerator::new(config)?,
            ))
        }
        EnumeratorStrategy::Auto => {
            if graph_estimate_is_safe(&config) {
                Ok(CandidateEnumerator::Ordered(PipelinedOrderedCandidateEnumerator::new(config)?))
            } else {
                Ok(CandidateEnumerator::Streaming(
                    StreamingLevenshteinCandidateEnumerator::new(config)?,
                ))
            }
        }
    }
}
