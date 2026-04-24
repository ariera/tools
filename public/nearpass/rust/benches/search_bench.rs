use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use string_neighborhood_kata::{enumerate_candidates, KeyboardNeighbors, SearchConfig};

fn benchmark_medium_search(c: &mut Criterion) {
    let alphabet: Vec<char> = ('a'..='z').collect();
    let config = SearchConfig::new(
        "pattern",
        alphabet,
        1,
        2,
        KeyboardNeighbors::from_pairs(&[
            ('a', &['s', 'q', 'w', 'z']),
            ('s', &['a', 'w', 'e', 'd', 'x']),
            ('p', &['o', 'l']),
        ]),
    )
    .unwrap();

    c.bench_function("enumerate pattern distance 1..2", |b| {
        b.iter(|| {
            let result = enumerate_candidates(black_box(&config)).unwrap();
            black_box(result.len())
        })
    });
}

criterion_group!(benches, benchmark_medium_search);
criterion_main!(benches);
