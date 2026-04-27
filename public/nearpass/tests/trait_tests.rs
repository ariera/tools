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
