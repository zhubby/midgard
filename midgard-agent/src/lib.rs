use async_trait::async_trait;
use midgard_core::{CompletionStatus, LlmConfig, RiskLevel};
use midgard_tools::{Tool, ToolDefinition, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentMessage {
    pub role: AgentRole,
    pub content: String,
}

impl AgentMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: AgentRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: AgentRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSession {
    pub id: Uuid,
    pub messages: Vec<AgentMessage>,
    pub iteration_count: usize,
}

impl AgentSession {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            messages: vec![AgentMessage::user(goal)],
            iteration_count: 0,
        }
    }
}

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    config: LlmConfig,
    api_key: String,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: LlmConfig, api_key: impl Into<String>) -> Self {
        Self {
            config,
            api_key: api_key.into(),
        }
    }

    pub fn chat_completions_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn authorization_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }
}

pub struct CompleteTaskTool;

#[async_trait]
impl Tool for CompleteTaskTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "complete_task",
            "Signal that the agent task is complete, partial, or blocked",
            json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["success", "partial", "blocked"]
                    },
                    "summary": {"type": "string"}
                },
                "required": ["summary"]
            }),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        let status = arguments
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(CompletionStatus::Success.as_str());
        let summary = arguments
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("task completed");

        ToolResult::complete(format!("{status}: {summary}"))
    }
}
