use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use string_neighborhood_kata::{
    count_candidates, CandidateEnumerator, KeyboardNeighbors, SearchConfig,
};

/// Enumerate strings in a bounded edit-distance neighborhood of a seed.
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

    /// Print only the total number of combinations in the simplified insert/delete/replace model
    #[arg(long)]
    count: bool,

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

fn common_symbols() -> &'static [char] {
    &[
        '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+', '-', '=', '[', ']', '{', '}',
        '|', ';', ':', ',', '.', '<', '>', '?', '/', '~',
    ]
}

fn qwerty_neighbors() -> KeyboardNeighbors {
    KeyboardNeighbors::from_pairs(&[
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
    ])
}

#[cfg(test)]
mod tests {
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
    fn count_flag_parses() {
        let cli = Cli::try_parse_from(["enumerate", "abc", "--count"]).unwrap();
        assert!(cli.count);
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let alphabet: Vec<char> = match cli.alphabet.as_deref() {
        Some(s) => s.chars().collect(),
        None => cli.preset.chars(),
    };

    let keyboard = if cli.qwerty {
        qwerty_neighbors()
    } else {
        KeyboardNeighbors::empty()
    };

    let config = match SearchConfig::new(&cli.seed, alphabet, cli.min, cli.max, keyboard) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    if cli.count {
        let count = match count_candidates(&config) {
            Ok(count) => count,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(1);
            }
        };
        println!("{count}");
        return ExitCode::SUCCESS;
    }

    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let mut enumerator = CandidateEnumerator::new(&config);
    let mut printed = 0usize;
    while let Some(candidate) = enumerator.next() {
        if cli.limit != 0 && printed >= cli.limit {
            break;
        }
        let _ = writeln!(out, "{candidate}");
        printed += 1;
    }
    let _ = out.flush();

    if !cli.quiet {
        if cli.limit != 0 {
            eprintln!("{} candidates (showing first {})", printed, printed);
        } else {
            eprintln!("{} candidates", printed);
        }
    }

    ExitCode::SUCCESS
}
