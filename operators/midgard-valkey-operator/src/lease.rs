pub use midgard_operator::lease::{LeaseConfig, LeaseDecision, LeaseGuard, decide_lease};

pub const DEFAULT_LOCK_NAMESPACE: &str = "valkey-operator-system";
pub const DEFAULT_LOCK_NAME: &str = "midgard-valkey-operator";

pub fn default_config() -> LeaseConfig {
    LeaseConfig {
        namespace: DEFAULT_LOCK_NAMESPACE.to_string(),
        name: DEFAULT_LOCK_NAME.to_string(),
        ..LeaseConfig::default()
    }
}
