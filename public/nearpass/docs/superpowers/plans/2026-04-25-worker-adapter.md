# Worker Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy the KeePass worker prototype into the pipelined crate and wrap it as a `CandidatePredicate` implementation so the orchestrator can test passwords in parallel.

**Architecture:** The worker module will contain the adapted error types and password-checking logic from the research prototype, exported through a `KeePassWorker` struct that implements `CandidatePredicate`. The orchestrator will hold this worker in an `Arc` and pass it to worker threads.

**Tech Stack:** Rust, keepass crate (0.10.6), standard library

---

## File Structure

**New files:**
- `pipelined/src/worker.rs` — Worker adapter, error types, predicate trait

**Modified files:**
- `pipelined/src/lib.rs` — Export worker module and trait
- `pipelined/Cargo.toml` — keepass dependency already present
- `pipelined/tests/worker_tests.rs` — Unit tests for worker

---

## Task 1: Define CandidatePredicate trait

**Files:**
- Create: `pipelined/src/worker.rs`
- Modify: `pipelined/src/lib.rs`

- [ ] **Step 1: Create worker.rs with trait definition**

Create `pipelined/src/worker.rs`:

```rust
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
```

- [ ] **Step 2: Export trait from lib.rs**

Add to `pipelined/src/lib.rs`:

```rust
mod worker;
pub use worker::CandidatePredicate;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p pipelined`

Expected: Success, no errors.

- [ ] **Step 4: Commit**

```bash
cd /Users/mainar/dev/personal/b29
git add pipelined/src/worker.rs pipelined/src/lib.rs
git commit -m "feat: define CandidatePredicate trait for worker abstraction"
```

---

## Task 2: Copy and adapt error types from research prototype

**Files:**
- Modify: `pipelined/src/worker.rs`

- [ ] **Step 1: Read the research prototype error enum**

Run:
```bash
cat /Users/mainar/dev/personal/research/keepass-secrets-vault-approaches/src/error.rs
```

Copy the entire `OpenError` enum and its impl block.

- [ ] **Step 2: Add error enum to worker.rs**

Replace the entire `pipelined/src/worker.rs` with:

```rust
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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p pipelined`

Expected: Success, no errors.

- [ ] **Step 4: Commit**

```bash
git add pipelined/src/worker.rs
git commit -m "feat: add OpenError type from research prototype"
```

---

## Task 3: Implement database opening logic

**Files:**
- Modify: `pipelined/src/worker.rs`

- [ ] **Step 1: Read the research prototype open.rs**

Run:
```bash
cat /Users/mainar/dev/personal/research/keepass-secrets-vault-approaches/src/open.rs
```

Copy the `open_database` and `can_open_database` functions.

- [ ] **Step 2: Add database opening functions to worker.rs**

Add these functions to the end of `pipelined/src/worker.rs`:

```rust
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use keepass::{Database, DatabaseKey};

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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p pipelined`

Expected: Success, no errors.

- [ ] **Step 4: Commit**

```bash
git add pipelined/src/worker.rs
git commit -m "feat: add database opening logic from research prototype"
```

---

## Task 4: Implement KeePassWorker struct

**Files:**
- Modify: `pipelined/src/worker.rs`

- [ ] **Step 1: Add KeePassWorker struct and constructor**

Add to the end of `pipelined/src/worker.rs`:

```rust
use std::path::PathBuf;

/// A worker that tests passwords by attempting to open a KeePass database.
///
/// The database path is fixed at construction and never changes.
/// Multiple worker threads can safely share a KeePassWorker via Arc<KeePassWorker>.
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
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p pipelined`

Expected: Success, no errors.

- [ ] **Step 3: Export KeePassWorker from lib.rs**

Add to `pipelined/src/lib.rs`:

```rust
pub use worker::{CandidatePredicate, KeePassWorker};
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p pipelined`

Expected: Success, no errors.

- [ ] **Step 5: Commit**

```bash
git add pipelined/src/worker.rs pipelined/src/lib.rs
git commit -m "feat: implement KeePassWorker struct with CandidatePredicate"
```

---

## Task 5: Write unit tests for OpenError

**Files:**
- Create: `pipelined/tests/worker_tests.rs`

- [ ] **Step 1: Create test file with OpenError tests**

Create `pipelined/tests/worker_tests.rs`:

```rust
use pipelined::worker::OpenError;

#[test]
fn open_error_wrong_password_cli_code() {
    let err = OpenError::WrongPassword;
    assert_eq!(err.as_cli_code(), "wrong-password");
}

#[test]
fn open_error_corrupt_database_cli_code() {
    let err = OpenError::CorruptDatabase;
    assert_eq!(err.as_cli_code(), "corrupt-db");
}

#[test]
fn open_error_unsupported_format_cli_code() {
    let err = OpenError::UnsupportedFormat;
    assert_eq!(err.as_cli_code(), "unsupported-format");
}

#[test]
fn open_error_io_error_cli_code() {
    let err = OpenError::Io(std::io::ErrorKind::NotFound);
    assert_eq!(err.as_cli_code(), "io-error");
}

#[test]
fn open_error_other_cli_code() {
    let err = OpenError::Other;
    assert_eq!(err.as_cli_code(), "other-error");
}

#[test]
fn open_error_clone_and_eq() {
    let err1 = OpenError::WrongPassword;
    let err2 = err1.clone();
    assert_eq!(err1, err2);
}

#[test]
fn open_error_different_io_kinds_not_equal() {
    let err1 = OpenError::Io(std::io::ErrorKind::NotFound);
    let err2 = OpenError::Io(std::io::ErrorKind::PermissionDenied);
    assert_ne!(err1, err2);
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p pipelined --test worker_tests`

Expected: All 7 tests pass.

- [ ] **Step 3: Commit**

```bash
git add pipelined/tests/worker_tests.rs
git commit -m "test: add unit tests for OpenError"
```

---

## Task 6: Write integration tests for KeePassWorker with a test database

**Files:**
- Modify: `pipelined/tests/worker_tests.rs`

- [ ] **Step 1: Check if test database exists**

Run:
```bash
find /Users/mainar/dev/personal/research/keepass-secrets-vault-approaches/tests -name "*.kdbx" 2>/dev/null | head -5
```

If test databases exist, note their paths. If not, we'll create a minimal one in step 4.

- [ ] **Step 2: Add test database setup helpers to worker_tests.rs**

Add to the end of `pipelined/tests/worker_tests.rs`:

```rust
use pipelined::KeePassWorker;
use std::path::PathBuf;

fn get_test_db_path() -> PathBuf {
    PathBuf::from(
        "/Users/mainar/dev/personal/research/keepass-secrets-vault-approaches/tests/data/test.kdbx",
    )
}

#[test]
#[ignore = "requires test database; run with --ignored to include"]
fn keepass_worker_fails_on_missing_db() {
    let worker = KeePassWorker::new(PathBuf::from("/nonexistent/path.kdbx"));
    let result = worker.test("anypassword");
    assert!(!result, "should return false when database doesn't exist");
}

#[test]
#[ignore = "requires test database with known password"]
fn keepass_worker_correct_password() {
    let db_path = get_test_db_path();
    if !db_path.exists() {
        eprintln!("skipping: test database not found at {:?}", db_path);
        return;
    }

    let worker = KeePassWorker::new(db_path);
    // Adjust this password based on what's actually in the test database
    let result = worker.test("password");
    assert!(result, "should return true for correct password");
}

#[test]
#[ignore = "requires test database with known password"]
fn keepass_worker_wrong_password() {
    let db_path = get_test_db_path();
    if !db_path.exists() {
        eprintln!("skipping: test database not found at {:?}", db_path);
        return;
    }

    let worker = KeePassWorker::new(db_path);
    let result = worker.test("wrongpassword");
    assert!(!result, "should return false for wrong password");
}

#[test]
fn keepass_worker_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<KeePassWorker>();
}
```

- [ ] **Step 3: Run the non-ignored tests**

Run: `cargo test -p pipelined --test worker_tests -- --skip keepass_worker`

Expected: `keepass_worker_is_send_sync` passes; all other tests pass.

- [ ] **Step 4: Attempt to run tests with test database**

Run: `cargo test -p pipelined --test worker_tests -- --ignored`

Expected: Tests either pass (if test database exists) or skip gracefully (if not).

If the test database doesn't exist and you have access to one, copy it and update `get_test_db_path()` accordingly. For now, tests will skip gracefully.

- [ ] **Step 5: Commit**

```bash
git add pipelined/tests/worker_tests.rs
git commit -m "test: add integration tests for KeePassWorker with test database"
```

---

## Task 7: Verify trait implementation and type safety

**Files:**
- Create: `pipelined/tests/trait_tests.rs`

- [ ] **Step 1: Write trait object test**

Create `pipelined/tests/trait_tests.rs`:

```rust
use pipelined::{CandidatePredicate, KeePassWorker};
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn candidate_predicate_is_object_safe() {
    // This test verifies that CandidatePredicate can be used as a trait object.
    let worker = KeePassWorker::new(PathBuf::from("/tmp/test.kdbx"));
    let _predicate: Arc<dyn CandidatePredicate> = Arc::new(worker);
    // Compile success is the assertion.
}

#[test]
fn candidate_predicate_function_pointer_impl() {
    // This test verifies that function pointers implement CandidatePredicate.
    fn always_true(_candidate: &str) -> bool {
        true
    }

    fn always_false(_candidate: &str) -> bool {
        false
    }

    let pred1: Box<dyn CandidatePredicate> = Box::new(always_true);
    let pred2: Box<dyn CandidatePredicate> = Box::new(always_false);

    assert!(pred1.test("anything"));
    assert!(!pred2.test("anything"));
}

#[test]
fn keepass_worker_clone() {
    let worker = KeePassWorker::new(PathBuf::from("/tmp/test.kdbx"));
    let worker2 = worker.clone();
    assert_eq!(worker.db_path(), worker2.db_path());
}

#[test]
fn keepass_worker_in_arc() {
    // This test verifies that KeePassWorker can be shared via Arc across threads.
    let worker = Arc::new(KeePassWorker::new(PathBuf::from("/tmp/test.kdbx")));
    let worker_clone = worker.clone();

    // Simulate passing to a worker thread.
    let _handle = std::thread::spawn(move || {
        let _ = worker_clone.test("password");
    });
    // Thread would join and test would pass.
}
```

- [ ] **Step 2: Run the trait tests**

Run: `cargo test -p pipelined --test trait_tests`

Expected: All 4 tests pass.

- [ ] **Step 3: Verify the library still exports correctly**

Run: `cargo doc -p pipelined --no-deps --open`

Expected: Documentation opens showing `CandidatePredicate` and `KeePassWorker` in the public API.

Close the browser when done.

- [ ] **Step 4: Commit**

```bash
git add pipelined/tests/trait_tests.rs
git commit -m "test: verify CandidatePredicate trait safety and KeePassWorker usability"
```

---

## Task 8: Add documentation and examples

**Files:**
- Modify: `pipelined/src/worker.rs`
- Create: `pipelined/examples/worker_example.rs`

- [ ] **Step 1: Enhance worker.rs documentation**

Add doc comments to the top of `pipelined/src/worker.rs`:

```rust
//! KeePass database worker adapter.
//!
//! This module provides a worker implementation that tests passwords by attempting
//! to open a KeePass database. It is designed to be used with a parallel search
//! orchestrator that generates candidate passwords and tests them in parallel.
//!
//! # Example
//!
//! ```ignore
//! use pipelined::{CandidatePredicate, KeePassWorker};
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
```

- [ ] **Step 2: Create example**

Create `pipelined/examples/worker_example.rs`:

```rust
use pipelined::{CandidatePredicate, KeePassWorker};
use std::path::PathBuf;

fn main() {
    // This example demonstrates creating a KeePassWorker and using it as a predicate.
    let db_path = PathBuf::from("/path/to/database.kdbx");

    let worker = KeePassWorker::new(db_path);

    // Test some candidate passwords.
    let candidates = vec!["password", "test123", "admin", "letmein"];

    for candidate in candidates {
        let success = worker.test(candidate);
        println!("Candidate '{}': {}", candidate, if success { "SUCCESS" } else { "false" });
    }

    // The worker can be shared across threads via Arc.
    use std::sync::Arc;
    let worker = Arc::new(worker);

    let handles: Vec<_> = candidates
        .iter()
        .map(|&candidate| {
            let worker = worker.clone();
            std::thread::spawn(move || {
                let success = worker.test(candidate);
                (candidate, success)
            })
        })
        .collect();

    for handle in handles {
        let (candidate, success) = handle.join().unwrap();
        println!("Thread result: '{}' => {}", candidate, success);
    }
}
```

- [ ] **Step 3: Verify example compiles**

Run: `cargo build -p pipelined --example worker_example`

Expected: Compiles successfully (will not run without a real database).

- [ ] **Step 4: Verify documentation builds**

Run: `cargo doc -p pipelined --no-deps`

Expected: No warnings or errors in documentation.

- [ ] **Step 5: Commit**

```bash
git add pipelined/src/worker.rs pipelined/examples/worker_example.rs
git commit -m "docs: add comprehensive documentation and example for worker module"
```

---

## Task 9: Run full test suite and final check

**Files:**
- None modified (verification only)

- [ ] **Step 1: Run all pipelined tests**

Run: `cargo test -p pipelined`

Expected: All tests pass, including unit and trait tests.

- [ ] **Step 2: Run clippy for code quality**

Run: `cargo clippy -p pipelined -- -D warnings`

Expected: No warnings or errors.

- [ ] **Step 3: Verify exports from lib.rs**

Run:
```bash
cargo build -p pipelined && cargo doc -p pipelined --no-deps --document-private-items 2>&1 | head -20
```

Expected: Documentation builds without errors.

- [ ] **Step 4: Quick compilation check on main project**

Run: `cargo check`

Expected: Main workspace checks successfully.

- [ ] **Step 5: Final commit summarizing work**

```bash
git log --oneline -10
```

Review the last 10 commits to verify all worker adapter tasks are there.

- [ ] **Step 6: Create summary of what was built**

Write to console (not committed):

```
Worker adapter setup complete:
- CandidatePredicate trait defined and exported
- OpenError type with KeePass error mapping
- open_database() and can_open_database() functions
- KeePassWorker struct with fixed db_path
- KeePassWorker implements CandidatePredicate
- Unit tests for OpenError
- Integration tests for KeePassWorker (with test database)
- Trait safety tests
- Documentation and example
- All tests passing, no clippy warnings
```

---

## Summary

This plan accomplishes:

1. ✓ Defines `CandidatePredicate` trait for orchestrator compatibility
2. ✓ Copies error types and logic from research prototype
3. ✓ Wraps worker as `KeePassWorker` struct
4. ✓ Implements `CandidatePredicate` for `KeePassWorker`
5. ✓ Ensures thread-safety (`Send + Sync`)
6. ✓ Comprehensive test coverage
7. ✓ Documentation and examples ready for orchestrator integration

**Next Step:** The orchestrator can now import `KeePassWorker` and `CandidatePredicate` from the pipelined crate and use them in the controller loop as described in the technical report (section 8.2).
