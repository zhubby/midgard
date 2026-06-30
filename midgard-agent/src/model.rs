use chrono::{SecondsFormat, Utc};
use midgard_core::{MidgardError, MidgardResult, RiskLevel};
use midgard_tools::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    #[ts(type = "unknown")]
    pub arguments: Value,
    pub raw_arguments: String,
}

impl AgentToolCall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
        raw_arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
            raw_arguments: raw_arguments.into(),
        }
    }

    pub fn from_raw(
        id: impl Into<String>,
        name: impl Into<String>,
        raw_arguments: impl Into<String>,
    ) -> Self {
        let raw_arguments = raw_arguments.into();
        let arguments = serde_json::from_str(&raw_arguments).unwrap_or(Value::Null);

        Self::new(id, name, arguments, raw_arguments)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct AgentMessage {
    pub role: AgentRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AgentToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl AgentMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(AgentRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(AgentRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(AgentRole::Assistant, content)
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<AgentToolCall>,
    ) -> Self {
        Self {
            role: AgentRole::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: AgentRole::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    fn new(role: AgentRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Running,
    Completed,
    AwaitingApproval,
    #[default]
    Responded,
    MaxIterations,
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct PendingApproval {
    #[ts(type = "string")]
    pub id: Uuid,
    pub tool_call: AgentToolCall,
    pub risk_level: RiskLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
}

impl PendingApproval {
    pub fn new(tool_call: AgentToolCall, risk_level: RiskLevel) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool_call,
            risk_level,
            approved: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

impl ApprovalDecision {
    pub fn approved(&self) -> bool {
        matches!(self, ApprovalDecision::Approve)
    }

    pub fn status(&self) -> ApprovalStatus {
        match self {
            ApprovalDecision::Approve => ApprovalStatus::Approved,
            ApprovalDecision::Reject => ApprovalStatus::Rejected,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalStatus::Pending => "pending",
            ApprovalStatus::Approved => "approved",
            ApprovalStatus::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct ApprovalRecord {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub session_id: Uuid,
    pub tool_call: AgentToolCall,
    pub risk_level: RiskLevel,
    pub status: ApprovalStatus,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ApprovalRecord {
    pub fn pending(session_id: Uuid, approval: &PendingApproval) -> Self {
        Self {
            id: approval.id,
            session_id,
            tool_call: approval.tool_call.clone(),
            risk_level: approval.risk_level.clone(),
            status: ApprovalStatus::Pending,
            requested_at: utc_now_rfc3339(),
            decided_at: None,
            actor: None,
            reason: None,
        }
    }

    pub fn record_decision(
        &mut self,
        decision: ApprovalDecision,
        actor: impl Into<String>,
        reason: Option<String>,
    ) {
        self.status = decision.status();
        self.decided_at = Some(utc_now_rfc3339());
        self.actor = Some(actor.into());
        self.reason = reason;
    }
}

fn utc_now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct AgentSession {
    #[ts(type = "string")]
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "string | null")]
    pub workspace_id: Option<Uuid>,
    pub messages: Vec<AgentMessage>,
    pub iteration_count: usize,
    #[serde(default)]
    pub status: AgentRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<PendingApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl AgentSession {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id: None,
            messages: vec![AgentMessage::user(goal)],
            iteration_count: 0,
            status: AgentRunStatus::Responded,
            pending_approval: None,
            last_error: None,
        }
    }

    pub fn record_approval_decision(
        &mut self,
        decision: ApprovalDecision,
    ) -> MidgardResult<PendingApproval> {
        let mut approval = self.pending_approval.clone().ok_or_else(|| {
            MidgardError::Agent("session does not have a pending approval".to_string())
        })?;
        approval.approved = Some(decision.approved());
        self.pending_approval = Some(approval.clone());
        self.status = AgentRunStatus::Running;

        Ok(approval)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentRunEvent {
    ModelDelta {
        content: String,
    },
    AssistantMessage {
        message: AgentMessage,
    },
    ToolCallRequested {
        tool_call: AgentToolCall,
    },
    ToolResult {
        tool_call_id: String,
        name: String,
        result: ToolResult,
    },
    ApprovalRequired {
        approval: PendingApproval,
    },
    Completed {
        status: AgentRunStatus,
        output: String,
    },
    Failed {
        error: String,
    },
}
