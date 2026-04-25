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
