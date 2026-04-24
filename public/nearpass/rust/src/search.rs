use std::fs;
use std::io::Write;
use std::path::Path;

use crate::config::SearchConfig;
use crate::keyboard::{KeyboardNeighborSnapshot, KeyboardNeighbors};
use crate::mutations::for_each_one_edit_neighbor;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

const CHECKPOINT_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateCheckpoint {
    pub finished: bool,
    pub current_distance: usize,
    pub output_index: usize,
    pub current_layer: Vec<(Vec<char>, u32)>,
    pub visited: Vec<Vec<char>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchConfigSnapshot {
    pub seed: String,
    pub alphabet: Vec<char>,
    pub min_distance: usize,
    pub max_distance: usize,
    pub keyboard_neighbors: KeyboardNeighborSnapshot,
    pub enabled_operations: crate::config::EnabledOperations,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCheckpointFile {
    pub format_version: u32,
    pub config: SearchConfigSnapshot,
    pub checkpoint: CandidateCheckpoint,
}

#[derive(Clone, Debug)]
pub struct CandidateEnumerator {
    config: SearchConfig,
    visited: FxHashSet<Vec<char>>,
    current_layer: Vec<(Vec<char>, u32)>,
    current_distance: usize,
    layer_output: Vec<String>,
    output_index: usize,
    finished: bool,
}

impl CandidateEnumerator {
    pub fn new(config: &SearchConfig) -> Self {
        let seed_chars: Vec<char> = config.seed_chars.iter().copied().collect();
        let mut visited: FxHashSet<Vec<char>> = FxHashSet::default();
        visited.insert(seed_chars.clone());

        let current_layer: Vec<(Vec<char>, u32)> = vec![(seed_chars, 0)];
        let layer_output = if config.min_distance == 0 {
            vec![config.seed.clone()]
        } else {
            Vec::new()
        };

        Self {
            config: config.clone(),
            visited,
            current_layer,
            current_distance: 0,
            layer_output,
            output_index: 0,
            finished: false,
        }
    }

    pub fn from_checkpoint(
        config: &SearchConfig,
        checkpoint: CandidateCheckpoint,
    ) -> Result<Self, String> {
        let CandidateCheckpoint {
            finished,
            current_distance,
            output_index,
            current_layer,
            visited,
        } = checkpoint;

        if current_distance > config.max_distance {
            return Err("checkpoint current_distance exceeds max_distance".into());
        }
        if visited.is_empty() && !finished {
            return Err("checkpoint visited set must not be empty before completion".into());
        }

        let visited_len = visited.len();
        let visited: FxHashSet<Vec<char>> = visited.into_iter().collect();
        if visited.len() != visited_len {
            return Err("checkpoint visited set contains duplicates".into());
        }

        if !finished && current_layer.is_empty() {
            return Err("checkpoint current_layer must not be empty before completion".into());
        }
        if !finished && !current_layer.iter().all(|(candidate, _)| visited.contains(candidate)) {
            return Err("checkpoint current_layer must already be present in visited".into());
        }

        let layer_output = if finished {
            Vec::new()
        } else if current_distance >= config.min_distance {
            current_layer
                .iter()
                .map(|(candidate, _)| candidate.iter().collect())
                .collect()
        } else {
            Vec::new()
        };

        if output_index > layer_output.len() {
            return Err("checkpoint output_index exceeds current layer output".into());
        }

        Ok(Self {
            config: config.clone(),
            visited,
            current_layer,
            current_distance,
            layer_output,
            output_index,
            finished,
        })
    }

    pub fn checkpoint(&self) -> CandidateCheckpoint {
        let mut visited: Vec<Vec<char>> = self.visited.iter().cloned().collect();
        visited.sort_unstable();

        CandidateCheckpoint {
            finished: self.finished,
            current_distance: self.current_distance,
            output_index: self.output_index,
            current_layer: self.current_layer.clone(),
            visited,
        }
    }

    pub fn checkpoint_file(&self) -> SearchCheckpointFile {
        SearchCheckpointFile {
            format_version: CHECKPOINT_FORMAT_VERSION,
            config: SearchConfigSnapshot::from_config(&self.config),
            checkpoint: self.checkpoint(),
        }
    }

    fn build_next_layer(&self) -> Vec<(Vec<char>, u32)> {
        let mut next_layer_best: FxHashMap<Vec<char>, u32> = FxHashMap::with_capacity_and_hasher(
            self.current_layer.len() * 16,
            Default::default(),
        );

        for (candidate, accumulated_cost) in &self.current_layer {
            let accumulated_cost = *accumulated_cost;
            for_each_one_edit_neighbor(candidate, &self.config, |neighbor_chars, cost| {
                if self.visited.contains(neighbor_chars) {
                    return;
                }
                let total_cost = accumulated_cost + cost;
                match next_layer_best.get_mut(neighbor_chars) {
                    Some(existing) => {
                        if total_cost < *existing {
                            *existing = total_cost;
                        }
                    }
                    None => {
                        next_layer_best.insert(neighbor_chars.to_vec(), total_cost);
                    }
                }
            });
        }

        let mut next_layer: Vec<(Vec<char>, u32)> = next_layer_best.into_iter().collect();
        next_layer.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });
        next_layer
    }

    fn rebuild_current_output(&self) -> Vec<String> {
        if self.current_distance < self.config.min_distance {
            Vec::new()
        } else {
            self.current_layer
                .iter()
                .map(|(candidate, _)| candidate.iter().collect())
                .collect()
        }
    }
}

impl Iterator for CandidateEnumerator {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            if self.output_index < self.layer_output.len() {
                let candidate = self.layer_output[self.output_index].clone();
                self.output_index += 1;
                return Some(candidate);
            }

            if self.current_distance >= self.config.max_distance {
                self.finished = true;
                return None;
            }

            let next_layer = self.build_next_layer();
            self.current_distance += 1;
            self.current_layer = next_layer;
            self.output_index = 0;
            self.layer_output = self.rebuild_current_output();

            for (candidate, _) in &self.current_layer {
                self.visited.insert(candidate.clone());
            }

            if self.current_layer.is_empty() {
                self.finished = true;
                return None;
            }
        }
    }
}

impl SearchConfigSnapshot {
    pub fn from_config(config: &SearchConfig) -> Self {
        Self {
            seed: config.seed.clone(),
            alphabet: config.alphabet.iter().copied().collect(),
            min_distance: config.min_distance,
            max_distance: config.max_distance,
            keyboard_neighbors: config.keyboard_neighbors.to_snapshot(),
            enabled_operations: config.enabled_operations,
        }
    }

    pub fn to_config(&self) -> Result<SearchConfig, String> {
        let keyboard_neighbors = KeyboardNeighbors::from_snapshot(&self.keyboard_neighbors);
        SearchConfig::new(
            self.seed.clone(),
            self.alphabet.clone(),
            self.min_distance,
            self.max_distance,
            keyboard_neighbors,
        )
        .map(|config| config.with_enabled_operations(self.enabled_operations))
    }
}

impl SearchCheckpointFile {
    pub fn new(config: &SearchConfig, checkpoint: CandidateCheckpoint) -> Self {
        Self {
            format_version: CHECKPOINT_FORMAT_VERSION,
            config: SearchConfigSnapshot::from_config(config),
            checkpoint,
        }
    }

    pub fn from_enumerator(enumerator: &CandidateEnumerator) -> Self {
        enumerator.checkpoint_file()
    }

    pub fn to_enumerator(self) -> Result<CandidateEnumerator, String> {
        if self.format_version != CHECKPOINT_FORMAT_VERSION {
            return Err(format!(
                "unsupported checkpoint format version {}",
                self.format_version
            ));
        }
        let config = self.config.to_config()?;
        CandidateEnumerator::from_checkpoint(&config, self.checkpoint)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let data = serde_json::to_vec_pretty(self)
            .map_err(|err| format!("failed to serialize checkpoint: {err}"))?;
        let tmp_path = path.with_extension("tmp");

        {
            let mut file = fs::File::create(&tmp_path)
                .map_err(|err| format!("failed to create checkpoint temp file: {err}"))?;
            file.write_all(&data)
                .map_err(|err| format!("failed to write checkpoint temp file: {err}"))?;
            file.sync_all()
                .map_err(|err| format!("failed to sync checkpoint temp file: {err}"))?;
        }

        fs::rename(&tmp_path, path)
            .map_err(|err| format!("failed to move checkpoint into place: {err}"))?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let data = fs::read(path)
            .map_err(|err| format!("failed to read checkpoint file {}: {err}", path.display()))?;
        serde_json::from_slice(&data)
            .map_err(|err| format!("failed to parse checkpoint file {}: {err}", path.display()))
    }
}

pub fn enumerate_candidates(config: &SearchConfig) -> Result<Vec<String>, String> {
    Ok(CandidateEnumerator::new(config).collect())
}
