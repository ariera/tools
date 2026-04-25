use std::collections::{HashMap, HashSet};
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use pipelined::{
    DistanceMode, EditOps, PipelinedOrderedCandidateEnumerator, SearchConfig,
};

/// Enumerate strings in a bounded edit-distance neighborhood of a seed.
///
/// Candidates are emitted in (distance, cost, lexical) order without the
/// full-layer blocking pause of a traditional layer-sort enumerator.
#[derive(Parser, Debug)]
#[command(name = "enumerate", about, long_about = None)]
struct Cli {
    /// Seed string to explore around
    seed: String,

    /// Minimum edit distance (inclusive)
    #[arg(long, default_value_t = 1)]
    min: usize,

    /// Maximum edit distance (inclusive)
    #[arg(long, default_value_t = 1)]
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

    /// Print at most N candidates (0 = no limit)
    #[arg(long, default_value_t = 0)]
    limit: usize,

    /// Deduplication mode: per-distance (compatibility) or global-minimum
    #[arg(long, value_enum, default_value_t = Mode::PerDistance)]
    mode: Mode,

    /// Also print distance and cost columns (tab-separated: distance\tcost\ttext)
    #[arg(long)]
    verbose: bool,

    /// Print enumeration stats to stderr after finishing
    #[arg(long)]
    stats: bool,

    /// Suppress the trailing "N candidates" status line on stderr
    #[arg(long)]
    quiet: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum AlphabetPreset {
    /// Lowercase letters a-z (26 chars)
    Lowercase,
    /// Letters a-z and A-Z (52 chars)
    Letters,
    /// Letters, digits, and space a-zA-Z0-9 (63 chars)
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
    /// Each candidate emitted once per distance layer using its best cost (compatibility default)
    PerDistance,
    /// Each candidate string emitted only at its minimum reachable distance
    GlobalMinimum,
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

fn write_candidates<W: Write>(
    enumerator: &mut PipelinedOrderedCandidateEnumerator,
    limit: usize,
    verbose: bool,
    out: &mut W,
    flush_each: bool,
) -> io::Result<usize> {
    let mut printed = 0usize;
    while limit == 0 || printed < limit {
        let Some(candidate) = enumerator.next() else {
            break;
        };
        if verbose {
            writeln!(out, "{}\t{}\t{}", candidate.distance, candidate.cost, candidate.text)?;
        } else {
            writeln!(out, "{}", candidate.text)?;
        }
        printed += 1;
        if flush_each {
            out.flush()?;
        }
    }
    out.flush()?;
    Ok(printed)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use clap::Parser;

    use super::{AlphabetPreset, Cli};

    #[test]
    fn letters_numbers_cli_preset_includes_space() {
        let cli = Cli::try_parse_from(["enumerate", "a b", "--preset", "letters-numbers"]).unwrap();
        let AlphabetPreset::LettersNumbers = cli.preset else {
            panic!("expected letters-numbers preset");
        };
        let alphabet = cli.preset.chars();
        assert!(alphabet.contains(&' '));
    }

    #[test]
    fn mode_flag_parses() {
        let cli =
            Cli::try_parse_from(["enumerate", "abc", "--mode", "global-minimum"]).unwrap();
        assert!(matches!(cli.mode, super::Mode::GlobalMinimum));
    }

    #[test]
    fn verbose_flag_parses() {
        let cli = Cli::try_parse_from(["enumerate", "abc", "--verbose"]).unwrap();
        assert!(cli.verbose);
    }

    #[derive(Default)]
    struct RecordingWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn terminal_streaming_flushes_each_candidate() {
        use pipelined::{DistanceMode, EditOps, PipelinedOrderedCandidateEnumerator, SearchConfig};
        use std::collections::HashMap;

        let cfg = SearchConfig {
            seed: "a".to_string(),
            alphabet: vec!['a', 'b'],
            min_distance: 1,
            max_distance: 1,
            ops: EditOps::all(),
            keyboard_neighbors: HashMap::new(),
            distance_mode: DistanceMode::PerDistanceBestCost,
        };
        let mut enumerator = PipelinedOrderedCandidateEnumerator::new(cfg).unwrap();
        let mut writer = RecordingWriter::default();
        let printed =
            super::write_candidates(&mut enumerator, 0, false, &mut writer, true).unwrap();

        assert!(printed > 0);
        // flush after each candidate plus one final flush
        assert_eq!(writer.flushes, printed + 1);
    }

    #[test]
    fn limit_does_not_pull_extra_candidate() {
        use pipelined::{DistanceMode, EditOps, PipelinedOrderedCandidateEnumerator, SearchConfig};
        use std::collections::HashMap;

        let cfg = SearchConfig {
            seed: "ab".to_string(),
            alphabet: vec!['a', 'b'],
            min_distance: 1,
            max_distance: 1,
            ops: EditOps::all(),
            keyboard_neighbors: HashMap::new(),
            distance_mode: DistanceMode::PerDistanceBestCost,
        };
        let mut enumerator = PipelinedOrderedCandidateEnumerator::new(cfg).unwrap();
        let mut writer = RecordingWriter::default();
        let printed =
            super::write_candidates(&mut enumerator, 1, false, &mut writer, false).unwrap();

        assert_eq!(printed, 1);
    }

    #[test]
    fn verbose_output_includes_distance_and_cost() {
        use pipelined::{DistanceMode, EditOps, PipelinedOrderedCandidateEnumerator, SearchConfig};
        use std::collections::HashMap;

        let cfg = SearchConfig {
            seed: "a".to_string(),
            alphabet: vec!['b'],
            min_distance: 1,
            max_distance: 1,
            ops: EditOps::replace_only(),
            keyboard_neighbors: HashMap::new(),
            distance_mode: DistanceMode::PerDistanceBestCost,
        };
        let mut enumerator = PipelinedOrderedCandidateEnumerator::new(cfg).unwrap();
        let mut writer = RecordingWriter::default();
        super::write_candidates(&mut enumerator, 0, true, &mut writer, false).unwrap();

        let output = String::from_utf8(writer.bytes).unwrap();
        assert_eq!(output, "1\t3\tb\n");
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let alphabet: Vec<char> = match cli.alphabet.as_deref() {
        Some(s) => s.chars().collect(),
        None => cli.preset.chars(),
    };

    let keyboard_neighbors = if cli.qwerty {
        qwerty_neighbors()
    } else {
        HashMap::new()
    };

    let distance_mode = match cli.mode {
        Mode::PerDistance => DistanceMode::PerDistanceBestCost,
        Mode::GlobalMinimum => DistanceMode::GlobalMinimumDistance,
    };

    let config = SearchConfig {
        seed: cli.seed.clone(),
        alphabet,
        min_distance: cli.min,
        max_distance: cli.max,
        ops: EditOps::all(),
        keyboard_neighbors,
        distance_mode,
    };

    let mut enumerator = match PipelinedOrderedCandidateEnumerator::new(config) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    let stdout = io::stdout();
    let printed = if stdout.is_terminal() {
        let mut out = stdout.lock();
        match write_candidates(&mut enumerator, cli.limit, cli.verbose, &mut out, true) {
            Ok(n) => n,
            Err(err) => {
                eprintln!("error: failed to write candidates: {err}");
                return ExitCode::from(1);
            }
        }
    } else {
        let mut out = BufWriter::new(stdout.lock());
        match write_candidates(&mut enumerator, cli.limit, cli.verbose, &mut out, false) {
            Ok(n) => n,
            Err(err) => {
                eprintln!("error: failed to write candidates: {err}");
                return ExitCode::from(1);
            }
        }
    };

    if cli.stats {
        let s = enumerator.stats();
        eprintln!(
            "stats: popped={} stale_skipped={} global_dup_skipped={} expanded={} \
             raw_neighbors={} local_unique={} relaxed_new={} relaxed_improved={} \
             relaxed_not_better={} emitted={}",
            s.popped,
            s.stale_skipped,
            s.global_duplicate_skipped,
            s.expanded,
            s.raw_neighbors_generated,
            s.local_unique_neighbors,
            s.relaxed_new,
            s.relaxed_improved,
            s.relaxed_not_better,
            s.emitted,
        );
    }

    if !cli.quiet {
        if cli.limit != 0 && printed == cli.limit {
            eprintln!("{} candidates (showing first {})", printed, printed);
        } else {
            eprintln!("{} candidates", printed);
        }
    }

    ExitCode::SUCCESS
}
