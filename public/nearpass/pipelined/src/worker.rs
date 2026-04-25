use keepass::error::{DatabaseKeyError, DatabaseOpenError, DatabaseVersionParseError};

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
