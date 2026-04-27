use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use pipelined::{
    engine::{EngineConfig, StopReason, SuccessSemantics},
    run, DistanceMode, EditOps, EnumeratorStrategy, KeePassWorker, SearchConfig,
};

/// Search a KeePass database for a password in an edit-distance neighborhood of a seed.
///
/// Exit codes: 0 = password found, 1 = not found or cancelled, 2 = error.
#[derive(Parser, Debug)]
#[command(name = "crack", about, long_about = None)]
struct Cli {
    /// Path to the KeePass database file (.kdbx)
    db_path: PathBuf,

    /// Seed string to build the search neighborhood around
    seed: String,

    // --- Search space ---
    /// Minimum edit distance (inclusive)
    #[arg(long, default_value_t = 1)]
    min: usize,

    /// Maximum edit distance (inclusive)
    #[arg(long, default_value_t = 2)]
    max: usize,

    /// Predefined alphabet; overridden by --alphabet
    #[arg(long, value_enum, default_value_t = AlphabetPreset::Lowercase)]
    preset: AlphabetPreset,

    /// Custom alphabet as a string of characters; overrides --preset
    #[arg(long, conflicts_with = "preset")]
    alphabet: Option<String>,

    /// Enable QWERTY (US) keyboard-neighbor likelihood ranking
    #[arg(long)]
    qwerty: bool,

    /// Deduplication mode: per-distance or global-minimum
    #[arg(long, value_enum, default_value_t = Mode::PerDistance)]
    mode: Mode,

    // --- Engine ---
    /// Number of worker threads (default: number of logical CPUs)
    #[arg(long)]
    workers: Option<usize>,

    /// Maximum candidates in-flight at once
    #[arg(long, default_value_t = 256)]
    max_pending: usize,

    /// Path for checkpoint file (enables checkpointing when set)
    #[arg(long)]
    checkpoint: Option<PathBuf>,

    /// Resume from checkpoint at --checkpoint path if it exists
    #[arg(long, requires = "checkpoint")]
    resume: bool,

    /// Save a checkpoint every N seconds
    #[arg(long, default_value_t = 60)]
    checkpoint_every: u64,

    /// Print progress every N seconds
    #[arg(long, default_value_t = 10)]
    progress_every: u64,

    /// Success semantics: first-discovered or ordered-first
    #[arg(long, value_enum, default_value_t = Semantics::FirstDiscovered)]
    semantics: Semantics,

    /// Suppress progress output
    #[arg(long)]
    quiet: bool,

    /// Candidate generation strategy: auto, ordered-graph, or streaming
    #[arg(long, value_enum, default_value_t = Strategy::Auto)]
    strategy: Strategy,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum AlphabetPreset {
    /// Lowercase letters a-z (26 chars)
    Lowercase,
    /// Letters a-z and A-Z (52 chars)
    Letters,
    /// Letters, digits, and space (63 chars)
    LettersNumbers,
    /// Letters, digits, space, and common symbols (~79 chars)
    LettersNumbersSymbols,
    /// All printable ASCII (95 chars)
    FullAscii,
}

impl AlphabetPreset {
    fn chars(self) -> Vec<char> {
        match self {
            Self::Lowercase => ('a'..='z').collect(),
            Self::Letters => ('a'..='z').chain('A'..='Z').collect(),
            Self::LettersNumbers => ('a'..='z')
                .chain('A'..='Z')
                .chain('0'..='9')
                .chain(std::iter::once(' '))
                .collect(),
            Self::LettersNumbersSymbols => ('a'..='z')
                .chain('A'..='Z')
                .chain('0'..='9')
                .chain(std::iter::once(' '))
                .chain(common_symbols().iter().copied())
                .collect(),
            Self::FullAscii => (32u8..127).map(|b| b as char).collect(),
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Mode {
    /// Each candidate emitted once per distance layer (default)
    PerDistance,
    /// Each candidate string emitted only at its minimum reachable distance
    GlobalMinimum,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Semantics {
    /// Stop as soon as any worker finds a match (fastest)
    FirstDiscovered,
    /// Stop only when the lowest-ordinal (shortest edit path) match is confirmed
    OrderedFirst,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Strategy {
    /// Use ordered graph for small searches, streaming for large
    Auto,
    /// Ordered graph: exact (distance, cost, lexical) order
    OrderedGraph,
    /// Streaming DFS: bounded memory; requires global-minimum mode, no swap
    Streaming,
}

impl Strategy {
    fn to_enumerator_strategy(self) -> EnumeratorStrategy {
        match self {
            Self::Auto => EnumeratorStrategy::Auto,
            Self::OrderedGraph => EnumeratorStrategy::OrderedGraph,
            Self::Streaming => EnumeratorStrategy::StreamingLevenshtein,
        }
    }
}

fn common_symbols() -> &'static [char] {
    &[
        '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+', '-', '=', '[', ']', '{', '}',
        '|', ';', ':', ',', '.', '<', '>', '?', '/', '~',
    ]
}

fn qwerty_neighbors() -> HashMap<char, HashSet<char>> {
    let pairs: &[(char, &[char])] = &[
        ('q', &['w', 'a']),
        ('w', &['q', 'e', 'a', 's']),
        ('e', &['w', 'r', 's', 'd']),
        ('r', &['e', 't', 'd', 'f']),
        ('t', &['r', 'y', 'f', 'g']),
        ('y', &['t', 'u', 'g', 'h']),
        ('u', &['y', 'i', 'h', 'j']),
        ('i', &['u', 'o', 'j', 'k']),
        ('o', &['i', 'p', 'k', 'l']),
        ('p', &['o', 'l']),
        ('a', &['q', 'w', 's', 'z']),
        ('s', &['a', 'w', 'e', 'd', 'z', 'x']),
        ('d', &['s', 'e', 'r', 'f', 'x', 'c']),
        ('f', &['d', 'r', 't', 'g', 'c', 'v']),
        ('g', &['f', 't', 'y', 'h', 'v', 'b']),
        ('h', &['g', 'y', 'u', 'j', 'b', 'n']),
        ('j', &['h', 'u', 'i', 'k', 'n', 'm']),
        ('k', &['j', 'i', 'o', 'l', 'm']),
        ('l', &['k', 'o', 'p']),
        ('z', &['a', 's', 'x']),
        ('x', &['z', 's', 'd', 'c']),
        ('c', &['x', 'd', 'f', 'v']),
        ('v', &['c', 'f', 'g', 'b']),
        ('b', &['v', 'g', 'h', 'n']),
        ('n', &['b', 'h', 'j', 'm']),
        ('m', &['n', 'j', 'k']),
    ];

    let mut map: HashMap<char, HashSet<char>> = HashMap::new();
    for &(from, neighbors) in pairs {
        map.entry(from).or_default().extend(neighbors.iter().copied());
    }
    map
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Build search config.
    let alphabet: Vec<char> = match cli.alphabet.as_deref() {
        Some(s) => s.chars().collect(),
        None => cli.preset.chars(),
    };

    let keyboard_neighbors = if cli.qwerty { qwerty_neighbors() } else { HashMap::new() };

    let distance_mode = match cli.mode {
        Mode::PerDistance => DistanceMode::PerDistanceBestCost,
        Mode::GlobalMinimum => DistanceMode::GlobalMinimumDistance,
    };

    let search_config = SearchConfig {
        seed: cli.seed.clone(),
        alphabet,
        min_distance: cli.min,
        max_distance: cli.max,
        ops: EditOps::all(),
        keyboard_neighbors,
        distance_mode,
    };

    let success_semantics = match cli.semantics {
        Semantics::FirstDiscovered => SuccessSemantics::FirstDiscovered,
        Semantics::OrderedFirst => SuccessSemantics::OrderedFirst,
    };

    let default_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let engine_config = EngineConfig {
        workers: cli.workers.unwrap_or(default_workers),
        max_pending: cli.max_pending,
        checkpoint_path: cli.checkpoint.clone(),
        checkpoint_every: Duration::from_secs(cli.checkpoint_every),
        progress_every: if cli.quiet {
            Duration::from_secs(u64::MAX)
        } else {
            Duration::from_secs(cli.progress_every)
        },
        success_semantics,
        strategy: cli.strategy.to_enumerator_strategy(),
    };

    if !cli.quiet {
        eprintln!(
            "crack: seed={:?} min={} max={} workers={} semantics={:?}",
            cli.seed, cli.min, cli.max, engine_config.workers, success_semantics
        );
    }

    let predicate = Arc::new(KeePassWorker::new(cli.db_path));

    match run(search_config, predicate, engine_config, cli.resume) {
        Ok(report) => {
            if !cli.quiet {
                eprintln!(
                    "done: reason={:?} generated={} tested={} elapsed={:.1}s",
                    report.stop_reason,
                    report.generated,
                    report.tested,
                    report.elapsed_secs,
                );
            }
            match report.stop_reason {
                StopReason::Found => {
                    if let Some(winner) = &report.winning_candidate {
                        println!("{}", winner.text);
                    }
                    ExitCode::SUCCESS
                }
                StopReason::Exhausted | StopReason::Cancelled => ExitCode::from(1),
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(2)
        }
    }
}
