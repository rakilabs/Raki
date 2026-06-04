//! Test-support doubles, shared across crate test suites. No IO.

use crate::clock::Clock;

/// A clock that always returns a fixed timestamp.
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}
