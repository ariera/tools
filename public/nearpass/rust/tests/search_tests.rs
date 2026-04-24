use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use string_neighborhood_kata::{
    count_candidates, enumerate_candidates, CandidateEnumerator, EnabledOperations,
    KeyboardNeighbors, SearchCheckpointFile, SearchConfig,
};

fn temp_checkpoint_path(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{stamp}.json"))
}

#[test]
fn returns_seed_when_distance_band_is_zero() {
    let config = SearchConfig::new(
        "abc",
        vec!['a', 'b', 'c'],
        0,
        0,
        KeyboardNeighbors::empty(),
    )
    .unwrap();

    let result = enumerate_candidates(&config).unwrap();

    assert_eq!(result, vec!["abc".to_string()]);
}

#[test]
fn rejects_min_distance_greater_than_max_distance() {
    let result = SearchConfig::new(
        "abc",
        vec!['a', 'b', 'c'],
        2,
        1,
        KeyboardNeighbors::empty(),
    );
    assert!(result.is_err());
}

#[test]
fn rejects_seed_with_characters_outside_alphabet() {
    let result = SearchConfig::new(
        "abd",
        vec!['a', 'b', 'c'],
        0,
        1,
        KeyboardNeighbors::empty(),
    );
    assert!(result.is_err());
}

#[test]
fn generates_insert_delete_replace_and_swap_neighbors() {
    let config = SearchConfig::new(
        "ab",
        vec!['a', 'b', 'c'],
        1,
        1,
        KeyboardNeighbors::from_pairs(&[('a', &['b']), ('b', &['a'])]),
    )
    .unwrap();

    let seed_chars: Vec<char> = config.seed.chars().collect();
    let neighbors = string_neighborhood_kata::one_edit_neighbors(&seed_chars, &config);

    assert!(neighbors.iter().any(|item| item.candidate == vec!['a']));
    assert!(neighbors
        .iter()
        .any(|item| item.candidate == vec!['a', 'b', 'c']));
    assert!(neighbors
        .iter()
        .any(|item| item.candidate == vec!['b', 'a']));
    assert!(neighbors
        .iter()
        .any(|item| item.candidate == vec!['b', 'b']));
}

#[test]
fn keyboard_neighbor_replace_costs_less_than_arbitrary_replace() {
    let config = SearchConfig::new(
        "ab",
        vec!['a', 'b', 'c'],
        1,
        1,
        KeyboardNeighbors::from_pairs(&[('a', &['b'])]),
    )
    .unwrap();

    let seed_chars: Vec<char> = config.seed.chars().collect();
    let neighbors = string_neighborhood_kata::one_edit_neighbors(&seed_chars, &config);
    let keyboard_cost = neighbors
        .iter()
        .find(|item| item.candidate == vec!['b', 'b'])
        .unwrap()
        .likelihood_cost;
    let arbitrary_cost = neighbors
        .iter()
        .find(|item| item.candidate == vec!['c', 'b'])
        .unwrap()
        .likelihood_cost;

    assert!(keyboard_cost < arbitrary_cost);
}

#[test]
fn identical_adjacent_swap_does_not_emit_seed() {
    let config = SearchConfig::new("aa", vec!['a'], 1, 1, KeyboardNeighbors::empty()).unwrap();

    let seed_chars: Vec<char> = config.seed.chars().collect();
    let neighbors = string_neighborhood_kata::one_edit_neighbors(&seed_chars, &config);

    assert!(!neighbors
        .iter()
        .any(|item| item.candidate == vec!['a', 'a']));
}

#[test]
fn excludes_seed_when_min_distance_is_one() {
    let config = SearchConfig::new("ab", vec!['a', 'b'], 1, 1, KeyboardNeighbors::empty()).unwrap();
    let result = enumerate_candidates(&config).unwrap();
    assert!(!result.contains(&"ab".to_string()));
}

#[test]
fn orders_distance_before_likelihood() {
    let config = SearchConfig::new(
        "ab",
        vec!['a', 'b', 'c'],
        1,
        2,
        KeyboardNeighbors::from_pairs(&[('a', &['b'])]),
    )
    .unwrap();

    let result = enumerate_candidates(&config).unwrap();
    let one_edit_index = result.iter().position(|item| item == "bb").unwrap();
    let two_edit_index = result.iter().position(|item| item == "cbc").unwrap();

    assert!(one_edit_index < two_edit_index);
}

#[test]
fn deduplicates_candidates_reachable_by_multiple_paths() {
    let config = SearchConfig::new("aa", vec!['a', 'b'], 1, 2, KeyboardNeighbors::empty()).unwrap();
    let result = enumerate_candidates(&config).unwrap();
    let count = result.iter().filter(|item| *item == "a").count();
    assert_eq!(count, 1);
}

#[test]
fn emits_exact_one_edit_neighborhood_for_small_alphabet() {
    let config = SearchConfig::new("a", vec!['a', 'b'], 1, 1, KeyboardNeighbors::empty()).unwrap();
    let result = enumerate_candidates(&config).unwrap();
    let expected = vec!["", "aa", "ab", "ba", "b"];
    assert_eq!(result, expected);
}

#[test]
fn supports_exact_distance_band() {
    let config = SearchConfig::new("ab", vec!['a', 'b'], 2, 2, KeyboardNeighbors::empty()).unwrap();
    let result = enumerate_candidates(&config).unwrap();
    assert!(result.iter().all(|candidate| candidate != "ab"));
    assert!(!result.is_empty());
}

#[test]
fn handles_unicode_seed_without_invalid_utf8_error() {
    let config =
        SearchConfig::new("é", vec!['é', 'a'], 1, 1, KeyboardNeighbors::empty()).unwrap();

    let result = enumerate_candidates(&config).unwrap();

    assert!(result.iter().any(|candidate| candidate == "a"));
    assert!(result.iter().any(|candidate| candidate == "aé"));
    assert!(result.iter().any(|candidate| candidate == "éa"));
}

#[test]
fn enumeration_treats_multibyte_character_as_single_edit() {
    let config = SearchConfig::new(
        "café",
        vec!['c', 'a', 'f', 'é'],
        1,
        1,
        KeyboardNeighbors::empty(),
    )
    .unwrap();

    let result = enumerate_candidates(&config).unwrap();

    assert!(result.iter().any(|candidate| candidate == "caf"));
    assert!(result.iter().any(|candidate| candidate == "caéf"));
    assert!(result.iter().all(|candidate| candidate != "café"));
}

#[test]
fn disables_insert_operation_when_flag_is_off() {
    let ops = EnabledOperations {
        insert: false,
        ..EnabledOperations::default()
    };
    let config = SearchConfig::new("a", vec!['a', 'b'], 1, 1, KeyboardNeighbors::empty())
        .unwrap()
        .with_enabled_operations(ops);

    let result = enumerate_candidates(&config).unwrap();

    assert!(result.iter().all(|candidate| candidate.chars().count() <= 1));
}

#[test]
fn disables_swap_operation_when_flag_is_off() {
    let ops = EnabledOperations {
        swap: false,
        insert: false,
        delete: false,
        replace: false,
    };
    let config = SearchConfig::new("ab", vec!['a', 'b'], 1, 1, KeyboardNeighbors::empty())
        .unwrap()
        .with_enabled_operations(ops);

    let result = enumerate_candidates(&config).unwrap();

    assert!(result.is_empty());
}

#[test]
fn resumes_from_checkpoint_without_repeating_emitted_candidates() {
    let config = SearchConfig::new("a", vec!['a', 'b'], 1, 1, KeyboardNeighbors::empty()).unwrap();
    let full = enumerate_candidates(&config).unwrap();

    let mut enumerator = CandidateEnumerator::new(&config);
    assert_eq!(enumerator.next(), Some(full[0].clone()));
    assert_eq!(enumerator.next(), Some(full[1].clone()));

    let checkpoint = enumerator.checkpoint();
    let resumed = CandidateEnumerator::from_checkpoint(&config, checkpoint).unwrap();
    let tail: Vec<String> = resumed.collect();

    assert_eq!(tail, full[2..].to_vec());
}

#[test]
fn serializes_checkpoint_to_disk_and_restores_it() {
    let config = SearchConfig::new(
        "ab",
        vec!['a', 'b', 'c'],
        1,
        2,
        KeyboardNeighbors::from_pairs(&[('a', &['b'])]),
    )
    .unwrap();

    let full = enumerate_candidates(&config).unwrap();
    let mut enumerator = CandidateEnumerator::new(&config);
    assert_eq!(enumerator.next(), Some(full[0].clone()));

    let checkpoint = SearchCheckpointFile::from_enumerator(&enumerator);
    let path = temp_checkpoint_path("string-neighborhood-checkpoint");
    checkpoint.save_to_path(&path).unwrap();

    let loaded = SearchCheckpointFile::load_from_path(&path).unwrap();
    let resumed = loaded.to_enumerator().unwrap();
    let tail: Vec<String> = resumed.collect();

    let _ = fs::remove_file(&path);
    assert_eq!(tail, full[1..].to_vec());
}

#[test]
fn counts_candidates_without_collecting_them() {
    let config = SearchConfig::new("a", vec!['a', 'b'], 1, 1, KeyboardNeighbors::empty()).unwrap();
    let count = count_candidates(&config).unwrap();

    assert_eq!(count, 6);
}
