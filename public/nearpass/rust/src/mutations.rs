use crate::config::SearchConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeighborCandidate {
    pub candidate: Vec<char>,
    pub likelihood_cost: u32,
}

pub fn for_each_one_edit_neighbor<F>(seed: &[char], config: &SearchConfig, mut emit: F)
where
    F: FnMut(&[char], u32),
{
    let mut scratch: Vec<char> = Vec::with_capacity(seed.len() + 1);
    let ops = config.enabled_operations;

    if ops.delete {
        for index in 0..seed.len() {
            scratch.clear();
            scratch.extend_from_slice(&seed[..index]);
            scratch.extend_from_slice(&seed[index + 1..]);
            emit(&scratch, 2);
        }
    }

    if ops.insert {
        for index in 0..=seed.len() {
            for &ch in config.alphabet.iter() {
                scratch.clear();
                scratch.extend_from_slice(&seed[..index]);
                scratch.push(ch);
                scratch.extend_from_slice(&seed[index..]);
                emit(&scratch, 2);
            }
        }
    }

    if ops.replace {
        for index in 0..seed.len() {
            let original = seed[index];
            for &ch in config.alphabet.iter() {
                if ch == original {
                    continue;
                }
                scratch.clear();
                scratch.extend_from_slice(seed);
                scratch[index] = ch;
                let likelihood_cost =
                    if config.keyboard_neighbors.contains_neighbor(original, ch) {
                        1
                    } else {
                        3
                    };
                emit(&scratch, likelihood_cost);
            }
        }
    }

    if ops.swap {
        for index in 0..seed.len().saturating_sub(1) {
            if seed[index] == seed[index + 1] {
                continue;
            }
            scratch.clear();
            scratch.extend_from_slice(seed);
            scratch.swap(index, index + 1);
            emit(&scratch, 1);
        }
    }
}

pub fn one_edit_neighbors(seed: &[char], config: &SearchConfig) -> Vec<NeighborCandidate> {
    let mut out = Vec::new();
    for_each_one_edit_neighbor(seed, config, |chars, cost| {
        out.push(NeighborCandidate {
            candidate: chars.to_vec(),
            likelihood_cost: cost,
        });
    });
    out
}
