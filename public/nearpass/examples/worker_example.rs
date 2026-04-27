use nearpass::{CandidatePredicate, KeePassWorker};
use std::path::PathBuf;

fn main() {
    // This example demonstrates creating a KeePassWorker and using it as a predicate.
    let db_path = PathBuf::from("/path/to/database.kdbx");

    let worker = KeePassWorker::new(db_path);

    // Test some candidate passwords.
    let candidates = vec!["password", "test123", "admin", "letmein"];

    for candidate in &candidates {
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
