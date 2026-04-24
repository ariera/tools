use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardNeighborSnapshot {
    pub pairs: Vec<(char, Vec<char>)>,
}

#[derive(Clone, Debug, Default)]
pub struct KeyboardNeighbors {
    by_key: FxHashMap<char, Box<[char]>>,
}

impl KeyboardNeighbors {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_pairs(pairs: &[(char, &[char])]) -> Self {
        let mut by_key = FxHashMap::default();
        for (key, neighbors) in pairs {
            by_key.insert(*key, neighbors.to_vec().into_boxed_slice());
        }
        Self { by_key }
    }

    pub fn to_snapshot(&self) -> KeyboardNeighborSnapshot {
        let mut pairs: Vec<(char, Vec<char>)> = self
            .by_key
            .iter()
            .map(|(key, neighbors)| (*key, neighbors.to_vec()))
            .collect();
        pairs.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        KeyboardNeighborSnapshot { pairs }
    }

    pub fn from_snapshot(snapshot: &KeyboardNeighborSnapshot) -> Self {
        let mut by_key = FxHashMap::default();
        for (key, neighbors) in &snapshot.pairs {
            by_key.insert(*key, neighbors.clone().into_boxed_slice());
        }
        Self { by_key }
    }

    pub fn contains_neighbor(&self, source: char, target: char) -> bool {
        self.by_key
            .get(&source)
            .map(|neighbors| neighbors.contains(&target))
            .unwrap_or(false)
    }
}
