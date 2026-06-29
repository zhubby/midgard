use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use midgard_core::{MidgardError, MidgardResult, RiskLevel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[ts(type = "unknown")]
    pub parameters_schema: Value,
    pub risk_level: RiskLevel,
    pub requires_approval: bool,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters_schema: Value,
        risk_level: RiskLevel,
    ) -> Self {
        let requires_approval = risk_level.requires_approval();

        Self {
            name: name.into(),
            description: description.into(),
            parameters_schema,
            risk_level,
            requires_approval,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub should_continue: bool,
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            should_continue: true,
            is_error: false,
        }
    }

    pub fn error(output: impl Into<String>) -> Self {
        Self {
            success: false,
            output: output.into(),
            should_continue: true,
            is_error: true,
        }
    }

    pub fn complete(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            should_continue: false,
            is_error: false,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn call(&self, arguments: Value) -> ToolResult;
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register<T>(&mut self, tool: T)
    where
        T: Tool + 'static,
    {
        self.tools
            .insert(tool.definition().name.clone(), Arc::new(tool));
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }

    pub fn definition(&self, name: &str) -> Option<ToolDefinition> {
        self.tools.get(name).map(|tool| tool.definition())
    }

    pub async fn call(&self, name: &str, arguments: Value) -> MidgardResult<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| MidgardError::Tool(format!("tool not found: {name}")))?;

        Ok(tool.call(arguments).await)
    }
}
