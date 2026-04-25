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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder: Option<LayerBuilderCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerBuilderCheckpoint {
    pub source_distance: usize,
    pub parent_index: usize,
    pub next_layer_best: Vec<(Vec<char>, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandidateAdvance {
    Candidate(String),
    Building,
    Finished,
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
pub struct DiscoveryCandidateEnumerator {
    config: SearchConfig,
    visited: FxHashSet<Vec<char>>,
    current_layer: Vec<(Vec<char>, u32)>,
    current_distance: usize,
    pending_seed: Option<String>,
    layer_builder: Option<DiscoveryLayerBuilder>,
    finished: bool,
}

#[derive(Clone, Debug)]
pub struct CandidateEnumerator {
    config: SearchConfig,
    visited: FxHashSet<Vec<char>>,
    current_layer: Vec<(Vec<char>, u32)>,
    current_distance: usize,
    layer_output: Vec<String>,
    output_index: usize,
    phase: EnumerationPhase,
}

#[derive(Clone, Debug)]
enum EnumerationPhase {
    EmittingCurrentLayer,
    BuildingNextLayer(LayerBuilder),
    Finished,
}

#[derive(Clone, Debug)]
struct LayerBuilder {
    source_distance: usize,
    parent_index: usize,
    next_layer_best: FxHashMap<Vec<char>, u32>,
}

#[derive(Clone, Debug)]
struct DiscoveryLayerBuilder {
    source_distance: usize,
    parent_index: usize,
    neighbor_cursor: Option<OneEditNeighborCursor>,
    next_layer: Vec<(Vec<char>, u32)>,
    next_layer_positions: FxHashMap<Vec<char>, usize>,
}

#[derive(Clone, Debug)]
struct OneEditNeighborCursor {
    operation: NeighborOperation,
    index: usize,
    alphabet_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NeighborOperation {
    Delete,
    Insert,
    Replace,
    Swap,
    Done,
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
            phase: EnumerationPhase::EmittingCurrentLayer,
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
            builder,
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
        if !finished
            && !current_layer
                .iter()
                .all(|(candidate, _)| visited.contains(candidate))
        {
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
        if finished && builder.is_some() {
            return Err("finished checkpoint cannot include a layer builder".into());
        }

        let phase = if finished {
            EnumerationPhase::Finished
        } else if let Some(builder) = builder {
            if builder.source_distance != current_distance {
                return Err(
                    "checkpoint builder source_distance must match current_distance".into(),
                );
            }
            if output_index != layer_output.len() {
                return Err(
                    "checkpoint builder requires the current layer output to be exhausted".into(),
                );
            }
            EnumerationPhase::BuildingNextLayer(LayerBuilder::from_checkpoint(
                builder,
                current_layer.len(),
                &visited,
            )?)
        } else {
            EnumerationPhase::EmittingCurrentLayer
        };

        Ok(Self {
            config: config.clone(),
            visited,
            current_layer,
            current_distance,
            layer_output,
            output_index,
            phase,
        })
    }

    pub fn checkpoint(&self) -> CandidateCheckpoint {
        let mut visited: Vec<Vec<char>> = self.visited.iter().cloned().collect();
        visited.sort_unstable();

        CandidateCheckpoint {
            finished: matches!(self.phase, EnumerationPhase::Finished),
            current_distance: self.current_distance,
            output_index: self.output_index,
            current_layer: self.current_layer.clone(),
            visited,
            builder: match &self.phase {
                EnumerationPhase::BuildingNextLayer(builder) => Some(builder.checkpoint()),
                EnumerationPhase::EmittingCurrentLayer | EnumerationPhase::Finished => None,
            },
        }
    }

    pub fn checkpoint_file(&self) -> SearchCheckpointFile {
        SearchCheckpointFile {
            format_version: CHECKPOINT_FORMAT_VERSION,
            config: SearchConfigSnapshot::from_config(&self.config),
            checkpoint: self.checkpoint(),
        }
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

    pub fn advance_work(&mut self, budget: usize) -> CandidateAdvance {
        if self.output_index < self.layer_output.len() {
            let candidate = self.layer_output[self.output_index].clone();
            self.output_index += 1;
            return CandidateAdvance::Candidate(candidate);
        }

        let mut remaining_budget = budget;
        loop {
            match &mut self.phase {
                EnumerationPhase::Finished => return CandidateAdvance::Finished,
                EnumerationPhase::EmittingCurrentLayer => {
                    if self.current_distance >= self.config.max_distance {
                        self.phase = EnumerationPhase::Finished;
                        return CandidateAdvance::Finished;
                    }
                    self.phase = EnumerationPhase::BuildingNextLayer(LayerBuilder::new(
                        self.current_distance,
                    ));
                }
                EnumerationPhase::BuildingNextLayer(builder) => {
                    let previous_parent_index = builder.parent_index;
                    let completed = builder.advance(
                        &self.current_layer,
                        &self.visited,
                        &self.config,
                        remaining_budget,
                    );
                    let expanded = builder.parent_index - previous_parent_index;
                    remaining_budget = remaining_budget.saturating_sub(expanded);
                    if !completed {
                        return CandidateAdvance::Building;
                    }

                    let builder = match std::mem::replace(
                        &mut self.phase,
                        EnumerationPhase::EmittingCurrentLayer,
                    ) {
                        EnumerationPhase::BuildingNextLayer(builder) => builder,
                        _ => unreachable!("phase changed while completing layer builder"),
                    };

                    self.current_distance = builder.source_distance + 1;
                    self.current_layer = builder.finish();
                    self.output_index = 0;
                    self.layer_output = self.rebuild_current_output();

                    for (candidate, _) in &self.current_layer {
                        self.visited.insert(candidate.clone());
                    }

                    if self.current_layer.is_empty() {
                        self.phase = EnumerationPhase::Finished;
                        return CandidateAdvance::Finished;
                    }

                    if self.output_index < self.layer_output.len() {
                        let candidate = self.layer_output[self.output_index].clone();
                        self.output_index += 1;
                        return CandidateAdvance::Candidate(candidate);
                    }

                    if remaining_budget == 0 {
                        return CandidateAdvance::Building;
                    }
                }
            }
        }
    }
}

impl LayerBuilder {
    fn new(source_distance: usize) -> Self {
        Self {
            source_distance,
            parent_index: 0,
            next_layer_best: FxHashMap::default(),
        }
    }

    fn from_checkpoint(
        checkpoint: LayerBuilderCheckpoint,
        current_layer_len: usize,
        visited: &FxHashSet<Vec<char>>,
    ) -> Result<Self, String> {
        if checkpoint.parent_index > current_layer_len {
            return Err("checkpoint builder parent_index exceeds current layer length".into());
        }

        let best_len = checkpoint.next_layer_best.len();
        let next_layer_best: FxHashMap<Vec<char>, u32> =
            checkpoint.next_layer_best.into_iter().collect();
        if next_layer_best.len() != best_len {
            return Err("checkpoint builder next_layer_best contains duplicates".into());
        }
        if next_layer_best
            .keys()
            .any(|candidate| visited.contains(candidate))
        {
            return Err("checkpoint builder next_layer_best overlaps visited set".into());
        }

        Ok(Self {
            source_distance: checkpoint.source_distance,
            parent_index: checkpoint.parent_index,
            next_layer_best,
        })
    }

    fn checkpoint(&self) -> LayerBuilderCheckpoint {
        let mut next_layer_best: Vec<(Vec<char>, u32)> = self
            .next_layer_best
            .iter()
            .map(|(candidate, cost)| (candidate.clone(), *cost))
            .collect();
        next_layer_best.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });

        LayerBuilderCheckpoint {
            source_distance: self.source_distance,
            parent_index: self.parent_index,
            next_layer_best,
        }
    }

    fn advance(
        &mut self,
        current_layer: &[(Vec<char>, u32)],
        visited: &FxHashSet<Vec<char>>,
        config: &SearchConfig,
        budget: usize,
    ) -> bool {
        let mut expanded = 0usize;
        while self.parent_index < current_layer.len() && expanded < budget {
            let (candidate, accumulated_cost) = &current_layer[self.parent_index];
            let accumulated_cost = *accumulated_cost;
            for_each_one_edit_neighbor(candidate, config, |neighbor_chars, cost| {
                if visited.contains(neighbor_chars) {
                    return;
                }
                let total_cost = accumulated_cost + cost;
                match self.next_layer_best.get_mut(neighbor_chars) {
                    Some(existing) => {
                        if total_cost < *existing {
                            *existing = total_cost;
                        }
                    }
                    None => {
                        self.next_layer_best
                            .insert(neighbor_chars.to_vec(), total_cost);
                    }
                }
            });
            self.parent_index += 1;
            expanded += 1;
        }

        self.parent_index == current_layer.len()
    }

    fn finish(self) -> Vec<(Vec<char>, u32)> {
        let mut next_layer: Vec<(Vec<char>, u32)> = self.next_layer_best.into_iter().collect();
        next_layer.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });
        next_layer
    }
}

impl DiscoveryCandidateEnumerator {
    pub fn new(config: &SearchConfig) -> Self {
        let seed_chars: Vec<char> = config.seed_chars.iter().copied().collect();
        let mut visited: FxHashSet<Vec<char>> = FxHashSet::default();
        visited.insert(seed_chars.clone());

        Self {
            config: config.clone(),
            visited,
            current_layer: vec![(seed_chars, 0)],
            current_distance: 0,
            pending_seed: (config.min_distance == 0).then(|| config.seed.clone()),
            layer_builder: None,
            finished: false,
        }
    }
}

impl Iterator for DiscoveryCandidateEnumerator {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(seed) = self.pending_seed.take() {
            return Some(seed);
        }
        if self.finished {
            return None;
        }

        loop {
            if self.current_distance >= self.config.max_distance {
                self.finished = true;
                return None;
            }

            if self.layer_builder.is_none() {
                self.layer_builder = Some(DiscoveryLayerBuilder::new(self.current_distance));
            }

            let builder = self
                .layer_builder
                .as_mut()
                .expect("discovery layer builder should be initialized");
            if let Some(candidate) =
                builder.next_candidate(&self.current_layer, &mut self.visited, &self.config)
            {
                return Some(candidate);
            }

            let builder = self
                .layer_builder
                .take()
                .expect("discovery layer builder should still be present");
            self.current_distance = builder.source_distance + 1;
            self.current_layer = builder.next_layer;

            if self.current_layer.is_empty() {
                self.finished = true;
                return None;
            }
        }
    }
}

impl DiscoveryLayerBuilder {
    fn new(source_distance: usize) -> Self {
        Self {
            source_distance,
            parent_index: 0,
            neighbor_cursor: None,
            next_layer: Vec::new(),
            next_layer_positions: FxHashMap::default(),
        }
    }

    fn next_candidate(
        &mut self,
        current_layer: &[(Vec<char>, u32)],
        visited: &mut FxHashSet<Vec<char>>,
        config: &SearchConfig,
    ) -> Option<String> {
        while self.parent_index < current_layer.len() {
            let (parent, accumulated_cost) = &current_layer[self.parent_index];
            let cursor = self
                .neighbor_cursor
                .get_or_insert_with(OneEditNeighborCursor::new);

            while let Some((neighbor, cost)) = cursor.next(parent, config) {
                let total_cost = *accumulated_cost + cost;
                if let Some(index) = self.next_layer_positions.get(&neighbor).copied() {
                    if total_cost < self.next_layer[index].1 {
                        self.next_layer[index].1 = total_cost;
                    }
                    continue;
                }
                if visited.contains(&neighbor) {
                    continue;
                }

                visited.insert(neighbor.clone());
                self.next_layer_positions
                    .insert(neighbor.clone(), self.next_layer.len());
                self.next_layer.push((neighbor.clone(), total_cost));

                if self.source_distance + 1 >= config.min_distance {
                    return Some(neighbor.iter().collect());
                }
            }

            self.neighbor_cursor = None;
            self.parent_index += 1;
        }

        None
    }
}

impl OneEditNeighborCursor {
    fn new() -> Self {
        Self {
            operation: NeighborOperation::Delete,
            index: 0,
            alphabet_index: 0,
        }
    }

    fn next(&mut self, seed: &[char], config: &SearchConfig) -> Option<(Vec<char>, u32)> {
        loop {
            match self.operation {
                NeighborOperation::Delete => {
                    if !config.enabled_operations.delete {
                        self.advance_operation(NeighborOperation::Insert);
                        continue;
                    }
                    if self.index < seed.len() {
                        let index = self.index;
                        self.index += 1;
                        let mut candidate = Vec::with_capacity(seed.len().saturating_sub(1));
                        candidate.extend_from_slice(&seed[..index]);
                        candidate.extend_from_slice(&seed[index + 1..]);
                        return Some((candidate, 2));
                    }
                    self.advance_operation(NeighborOperation::Insert);
                }
                NeighborOperation::Insert => {
                    if !config.enabled_operations.insert {
                        self.advance_operation(NeighborOperation::Replace);
                        continue;
                    }
                    if self.index <= seed.len() {
                        if self.alphabet_index < config.alphabet.len() {
                            let ch = config.alphabet[self.alphabet_index];
                            self.alphabet_index += 1;
                            let mut candidate = Vec::with_capacity(seed.len() + 1);
                            candidate.extend_from_slice(&seed[..self.index]);
                            candidate.push(ch);
                            candidate.extend_from_slice(&seed[self.index..]);
                            return Some((candidate, 2));
                        }
                        self.index += 1;
                        self.alphabet_index = 0;
                        continue;
                    }
                    self.advance_operation(NeighborOperation::Replace);
                }
                NeighborOperation::Replace => {
                    if !config.enabled_operations.replace {
                        self.advance_operation(NeighborOperation::Swap);
                        continue;
                    }
                    if self.index < seed.len() {
                        while self.alphabet_index < config.alphabet.len() {
                            let ch = config.alphabet[self.alphabet_index];
                            self.alphabet_index += 1;
                            let original = seed[self.index];
                            if ch == original {
                                continue;
                            }

                            let mut candidate = seed.to_vec();
                            candidate[self.index] = ch;
                            let likelihood_cost =
                                if config.keyboard_neighbors.contains_neighbor(original, ch) {
                                    1
                                } else {
                                    3
                                };
                            return Some((candidate, likelihood_cost));
                        }
                        self.index += 1;
                        self.alphabet_index = 0;
                        continue;
                    }
                    self.advance_operation(NeighborOperation::Swap);
                }
                NeighborOperation::Swap => {
                    if !config.enabled_operations.swap {
                        self.advance_operation(NeighborOperation::Done);
                        continue;
                    }
                    while self.index < seed.len().saturating_sub(1) {
                        let index = self.index;
                        self.index += 1;
                        if seed[index] == seed[index + 1] {
                            continue;
                        }

                        let mut candidate = seed.to_vec();
                        candidate.swap(index, index + 1);
                        return Some((candidate, 1));
                    }
                    self.advance_operation(NeighborOperation::Done);
                }
                NeighborOperation::Done => return None,
            }
        }
    }

    fn advance_operation(&mut self, operation: NeighborOperation) {
        self.operation = operation;
        self.index = 0;
        self.alphabet_index = 0;
    }
}

impl Iterator for CandidateEnumerator {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.advance_work(usize::MAX) {
                CandidateAdvance::Candidate(candidate) => return Some(candidate),
                CandidateAdvance::Building => continue,
                CandidateAdvance::Finished => return None,
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

pub fn count_candidates(config: &SearchConfig) -> Result<u128, String> {
    count_combinatorial_candidates(config)
}

fn count_combinatorial_candidates(config: &SearchConfig) -> Result<u128, String> {
    let seed_len = config.seed_chars.len();
    let alphabet_len = config.alphabet.len();
    if alphabet_len == 0 {
        return Ok(0);
    }

    let mut total = 0u128;
    for distance in config.min_distance..=config.max_distance {
        let distance_total = count_exact_distance_combinations(
            seed_len,
            alphabet_len,
            distance,
            config.enabled_operations,
        )?;
        total = total
            .checked_add(distance_total)
            .ok_or_else(|| "candidate count overflowed u128".to_string())?;
    }
    Ok(total)
}

fn count_exact_distance_combinations(
    seed_len: usize,
    alphabet_len: usize,
    distance: usize,
    ops: crate::config::EnabledOperations,
) -> Result<u128, String> {
    let mut total = 0u128;

    for deletions in 0..=distance.min(seed_len) {
        if deletions > 0 && !ops.delete {
            continue;
        }

        let remaining_after_deletes = seed_len - deletions;
        for replacements in 0..=distance - deletions {
            if replacements > remaining_after_deletes {
                break;
            }
            if replacements > 0 && !ops.replace {
                continue;
            }

            let insertions = distance - deletions - replacements;
            if insertions > 0 && !ops.insert {
                continue;
            }

            let delete_choices = binomial(seed_len, deletions)?;
            let replacement_choices = binomial(remaining_after_deletes, replacements)?;
            let insertion_choices = binomial(remaining_after_deletes + insertions, insertions)?;
            let replacement_values =
                checked_pow_u128(alphabet_len.saturating_sub(1) as u128, replacements)?;
            let insertion_values = checked_pow_u128(alphabet_len as u128, insertions)?;

            let mut term = delete_choices;
            term = term
                .checked_mul(replacement_choices)
                .ok_or_else(|| "candidate count overflowed u128".to_string())?;
            term = term
                .checked_mul(replacement_values)
                .ok_or_else(|| "candidate count overflowed u128".to_string())?;
            term = term
                .checked_mul(insertion_choices)
                .ok_or_else(|| "candidate count overflowed u128".to_string())?;
            term = term
                .checked_mul(insertion_values)
                .ok_or_else(|| "candidate count overflowed u128".to_string())?;

            total = total
                .checked_add(term)
                .ok_or_else(|| "candidate count overflowed u128".to_string())?;
        }
    }

    Ok(total)
}

fn binomial(n: usize, k: usize) -> Result<u128, String> {
    if k > n {
        return Ok(0);
    }
    let k = k.min(n - k);
    let mut row = vec![0u128; k + 1];
    row[0] = 1;

    for i in 1..=n {
        let upper = i.min(k);
        for j in (1..=upper).rev() {
            row[j] = row[j]
                .checked_add(row[j - 1])
                .ok_or_else(|| format!("binomial coefficient overflow at C({n}, {k})"))?;
        }
    }

    Ok(row[k])
}

fn checked_pow_u128(base: u128, exp: usize) -> Result<u128, String> {
    let mut acc = 1u128;
    for _ in 0..exp {
        acc = acc
            .checked_mul(base)
            .ok_or_else(|| "candidate count overflowed u128".to_string())?;
    }
    Ok(acc)
}
