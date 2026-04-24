use std::sync::Arc;

use crate::keyboard::KeyboardNeighbors;

#[derive(Clone, Copy, Debug)]
pub struct EnabledOperations {
    pub insert: bool,
    pub delete: bool,
    pub replace: bool,
    pub swap: bool,
}

impl EnabledOperations {
    pub const fn all() -> Self {
        Self {
            insert: true,
            delete: true,
            replace: true,
            swap: true,
        }
    }
}

impl Default for EnabledOperations {
    fn default() -> Self {
        Self::all()
    }
}

#[derive(Clone, Debug)]
pub struct SearchConfig {
    pub seed: String,
    pub seed_chars: Arc<[char]>,
    pub alphabet: Arc<[char]>,
    pub min_distance: usize,
    pub max_distance: usize,
    pub keyboard_neighbors: KeyboardNeighbors,
    pub enabled_operations: EnabledOperations,
}

impl SearchConfig {
    pub fn new(
        seed: impl Into<String>,
        alphabet: Vec<char>,
        min_distance: usize,
        max_distance: usize,
        keyboard_neighbors: KeyboardNeighbors,
    ) -> Result<Self, String> {
        let seed = seed.into();
        if min_distance > max_distance {
            return Err("min_distance must be <= max_distance".into());
        }
        if alphabet.is_empty() {
            return Err("alphabet must not be empty".into());
        }

        let seed_chars: Vec<char> = seed.chars().collect();
        if !seed_chars.iter().all(|ch| alphabet.contains(ch)) {
            return Err("seed contains characters outside alphabet".into());
        }

        Ok(Self {
            seed,
            seed_chars: Arc::<[char]>::from(seed_chars),
            alphabet: Arc::<[char]>::from(alphabet),
            min_distance,
            max_distance,
            keyboard_neighbors,
            enabled_operations: EnabledOperations::default(),
        })
    }

    pub fn with_enabled_operations(mut self, enabled_operations: EnabledOperations) -> Self {
        self.enabled_operations = enabled_operations;
        self
    }
}
