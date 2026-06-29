use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type MidgardResult<T> = Result<T, MidgardError>;

#[derive(Debug, Error)]
pub enum MidgardError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("controller error: {0}")]
    Controller(String),
    #[error("agent error: {0}")]
    Agent(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn requires_approval(&self) -> bool {
        matches!(self, RiskLevel::High | RiskLevel::Critical)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityDescriptor {
    pub id: String,
    pub name: String,
    pub risk_level: RiskLevel,
    pub requires_approval: bool,
}

impl CapabilityDescriptor {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        risk_level: RiskLevel,
    ) -> Self {
        let requires_approval = risk_level.requires_approval();

        Self {
            id: id.into(),
            name: name.into(),
            risk_level,
            requires_approval,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStatus {
    Success,
    Partial,
    Blocked,
}

impl CompletionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompletionStatus::Success => "success",
            CompletionStatus::Partial => "partial",
            CompletionStatus::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
}

impl LlmConfig {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlatformConfig {
    pub llm: LlmConfig,
}

impl PlatformConfig {
    pub fn for_development() -> Self {
        Self {
            llm: LlmConfig::new("https://api.openai.com/v1", "gpt-4o-mini"),
        }
    }
}

