//! The egress policy: the single gate deciding whether data may leave the device.

use raki_domain::Locality;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EgressPolicy {
    /// No network model calls allowed.
    LocalOnly,
    /// Cloud providers permitted (user consented).
    AllowCloud,
}

impl EgressPolicy {
    pub fn permits(&self, locality: Locality) -> bool {
        match self {
            EgressPolicy::LocalOnly => locality == Locality::Local,
            EgressPolicy::AllowCloud => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::Locality;

    #[test]
    fn local_only_blocks_cloud() {
        assert!(EgressPolicy::LocalOnly.permits(Locality::Local));
        assert!(!EgressPolicy::LocalOnly.permits(Locality::Cloud));
    }

    #[test]
    fn allow_cloud_permits_everything() {
        assert!(EgressPolicy::AllowCloud.permits(Locality::Local));
        assert!(EgressPolicy::AllowCloud.permits(Locality::Cloud));
    }
}
