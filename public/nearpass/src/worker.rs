//! KeePass database worker adapter.
//!
//! This module provides a worker implementation that tests passwords by attempting
//! to open a KeePass database. It is designed to be used with a parallel search
//! orchestrator that generates candidate passwords and tests them in parallel.
//!
//! # Example
//!
//! ```ignore
//! use nearpass::{CandidatePredicate, KeePassWorker};
//! use std::path::PathBuf;
//! use std::sync::Arc;
//!
//! let worker = Arc::new(KeePassWorker::new(PathBuf::from("database.kdbx")));
//! let result = worker.test("password");
//! assert!(!result); // password is wrong
//! ```
//!
//! # Thread Safety
//!
//! `KeePassWorker` is `Send + Sync` and can be safely shared across multiple
//! threads via `Arc<KeePassWorker>`. The orchestrator passes the worker to
//! parallel worker threads, which call `test()` concurrently.
//!
//! # Purity
//!
//! The `test()` method is pure: the same password always produces the same result.
//! This ensures that retesting after a resume (due to checkpoint recovery) is safe
//! and will produce identical results.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use keepass::error::{DatabaseKeyError, DatabaseOpenError, DatabaseVersionParseError};
use keepass::{Database, DatabaseKey};

/// Errors that can occur when opening a KeePass database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    WrongPassword,
    CorruptDatabase,
    UnsupportedFormat,
    Io(std::io::ErrorKind),
    Other,
}

impl OpenError {
    /// Return a machine-readable error code suitable for CLI output.
    pub fn as_cli_code(&self) -> &'static str {
        match self {
            Self::WrongPassword => "wrong-password",
            Self::CorruptDatabase => "corrupt-db",
            Self::UnsupportedFormat => "unsupported-format",
            Self::Io(_) => "io-error",
            Self::Other => "other-error",
        }
    }

    /// Convert a keepass crate error into our OpenError.
    pub fn from_keepass_error(error: DatabaseOpenError) -> Self {
        match error {
            DatabaseOpenError::Io(inner) => Self::Io(inner.kind()),
            DatabaseOpenError::UnexpectedEof => Self::CorruptDatabase,
            DatabaseOpenError::UnsupportedVersion => Self::UnsupportedFormat,
            DatabaseOpenError::VersionParse(DatabaseVersionParseError::InvalidKDBXVersion {
                ..
            }) => Self::UnsupportedFormat,
            DatabaseOpenError::VersionParse(DatabaseVersionParseError::InvalidKDBXIdentifier)
            | DatabaseOpenError::VersionParse(DatabaseVersionParseError::UnexpectedEof) => {
                Self::CorruptDatabase
            }
            DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => Self::WrongPassword,
            DatabaseOpenError::Format(_) => Self::CorruptDatabase,
            DatabaseOpenError::Key(_) | DatabaseOpenError::Cryptography(_) => Self::Other,
        }
    }
}

/// A predicate that tests whether a candidate string succeeds.
///
/// Implementations must be thread-safe and pure: the same candidate
/// must always return the same result, and no side effects are allowed.
pub trait CandidatePredicate: Send + Sync + 'static {
    /// Test whether this candidate succeeds.
    /// Returns true if the candidate is successful, false otherwise.
    fn test(&self, candidate: &str) -> bool;
}

/// Blanket impl for function pointers for simple use cases.
impl<F> CandidatePredicate for F
where
    F: Fn(&str) -> bool + Send + Sync + 'static,
{
    fn test(&self, candidate: &str) -> bool {
        self(candidate)
    }
}

/// Attempt to open a KeePass database with the given password.
///
/// Returns Ok(()) if the database opens successfully with this password,
/// or an error describing why it failed.
pub fn open_database(path: &Path, password: &str) -> Result<(), OpenError> {
    let file = File::open(path).map_err(|error| OpenError::Io(error.kind()))?;
    let mut reader = BufReader::new(file);
    let key = DatabaseKey::new().with_password(password);

    Database::get_xml(&mut reader, key)
        .map(|_| ())
        .map_err(OpenError::from_keepass_error)
}

/// Test whether a password opens the database at the given path.
///
/// Returns true if the database opens, false otherwise (including on any error).
/// This is the pure predicate used by the worker.
pub fn can_open_database(path: &Path, password: &str) -> bool {
    open_database(path, password).is_ok()
}

/// A worker that tests passwords by attempting to open a KeePass database.
///
/// The database path is fixed at construction and never changes.
/// Multiple worker threads can safely share a KeePassWorker via `Arc<KeePassWorker>`.
#[derive(Clone)]
pub struct KeePassWorker {
    db_path: PathBuf,
}

impl KeePassWorker {
    /// Create a new worker for the given database path.
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Get the database path (for testing/debugging).
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

impl CandidatePredicate for KeePassWorker {
    fn test(&self, candidate: &str) -> bool {
        can_open_database(&self.db_path, candidate)
    }
}
