//! A clock port so timestamps are injectable and tests are deterministic.

pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}
