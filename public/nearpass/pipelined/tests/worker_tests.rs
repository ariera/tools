use pipelined::OpenError;

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
