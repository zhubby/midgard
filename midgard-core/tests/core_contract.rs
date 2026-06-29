use midgard_core::{CapabilityDescriptor, CompletionStatus, LlmConfig, PlatformConfig, RiskLevel};

#[test]
fn capability_descriptor_marks_approval_for_high_risk_operations() {
    let capability =
        CapabilityDescriptor::new("redis.restart", "Restart Redis workload", RiskLevel::High);

    assert!(capability.requires_approval);
    assert_eq!(capability.risk_level, RiskLevel::High);
}

#[test]
fn platform_config_uses_openai_compatible_defaults() {
    let config = PlatformConfig::for_development();

    assert_eq!(config.llm.base_url, "https://api.openai.com/v1");
    assert_eq!(config.llm.model, "gpt-4o-mini");
}

#[test]
fn llm_config_accepts_custom_openai_compatible_gateway() {
    let config = LlmConfig::new("http://gateway.local/v1", "qwen-max");

    assert_eq!(config.base_url, "http://gateway.local/v1");
    assert_eq!(config.model, "qwen-max");
}

#[test]
fn completion_status_serializes_to_stable_wire_values() {
    assert_eq!(CompletionStatus::Success.as_str(), "success");
    assert_eq!(CompletionStatus::Partial.as_str(), "partial");
    assert_eq!(CompletionStatus::Blocked.as_str(), "blocked");
}
