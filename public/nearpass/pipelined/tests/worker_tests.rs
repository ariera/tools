use pipelined::{OpenError, CandidatePredicate, KeePassWorker};
use std::path::PathBuf;

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
