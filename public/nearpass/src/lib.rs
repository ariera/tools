mod config;
pub use config::{config_hash, ConfigError, DistanceMode, EditOps, SearchConfig};

mod enumerator;
pub use enumerator::{
    Candidate, EnumeratorSnapshot, EnumeratorStats, PipelinedOrderedCandidateEnumerator,
    SnapshotError,
};

mod strategy;
pub use strategy::{
    make_candidate_enumerator, AnyEnumeratorSnapshot, CandidateEnumerator, EnumeratorStrategy,
};

mod streaming;
pub use streaming::{
    StreamingEnumeratorSnapshot, StreamingEnumeratorStats, StreamingLevenshteinCandidateEnumerator,
};

mod worker;
pub use worker::{CandidatePredicate, KeePassWorker, OpenError};

pub mod engine;
pub use engine::{run, EngineConfig, RunError, SearchReport, StopReason, SuccessSemantics};
