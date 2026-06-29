use midgard_agent::{
    AgentMessage, AgentRole, AgentRunStatus, AgentToolCall, ApprovalRecord, ApprovalStatus,
    PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult, RiskLevel};
use toasty::stmt;
use uuid::Uuid;

pub(crate) fn agent_message_from_row(row: stmt::Value) -> MidgardResult<AgentMessage> {
    let record = row.into_record();
    let role = string_from_value(&record[0])?;
    let content = string_from_value(&record[1])?;
    let tool_calls = optional_tool_calls(&record[2])?;
    let tool_call_id = optional_string_from_value(&record[3])?;

    Ok(AgentMessage {
        role: role_from_storage(role)?,
        content: content.to_string(),
        tool_calls,
        tool_call_id,
    })
}

pub(crate) fn approval_record_from_row(row: stmt::Value) -> MidgardResult<ApprovalRecord> {
    let record = row.into_record();
    let id = uuid_from_value(&record[0])?;
    let session_id = uuid_from_value(&record[1])?;
    let tool_call_json = string_from_value(&record[2])?;
    let risk_level = risk_level_from_storage(string_from_value(&record[3])?)?;
    let status = approval_status_from_storage(string_from_value(&record[4])?)?;
    let requested_at = string_from_value(&record[5])?.to_string();
    let decided_at = optional_string_from_value(&record[6])?;
    let actor = optional_string_from_value(&record[7])?;
    let reason = optional_string_from_value(&record[8])?;

    Ok(ApprovalRecord {
        id,
        session_id,
        tool_call: serde_json::from_str(tool_call_json)
            .map_err(|err| MidgardError::Storage(format!("deserialize tool call: {err}")))?,
        risk_level,
        status,
        requested_at,
        decided_at,
        actor,
        reason,
    })
}

pub(crate) fn role_to_storage(role: &AgentRole) -> &'static str {
    match role {
        AgentRole::System => "system",
        AgentRole::User => "user",
        AgentRole::Assistant => "assistant",
        AgentRole::Tool => "tool",
    }
}

fn role_from_storage(role: &str) -> MidgardResult<AgentRole> {
    match role {
        "system" => Ok(AgentRole::System),
        "user" => Ok(AgentRole::User),
        "assistant" => Ok(AgentRole::Assistant),
        "tool" => Ok(AgentRole::Tool),
        other => Err(MidgardError::Storage(format!(
            "unknown stored agent role: {other}"
        ))),
    }
}

pub(crate) fn status_to_storage(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Running => "running",
        AgentRunStatus::Completed => "completed",
        AgentRunStatus::AwaitingApproval => "awaiting_approval",
        AgentRunStatus::Responded => "responded",
        AgentRunStatus::MaxIterations => "max_iterations",
        AgentRunStatus::Failed => "failed",
    }
}

pub(crate) fn status_from_storage(status: &str) -> MidgardResult<AgentRunStatus> {
    match status {
        "running" => Ok(AgentRunStatus::Running),
        "completed" => Ok(AgentRunStatus::Completed),
        "awaiting_approval" => Ok(AgentRunStatus::AwaitingApproval),
        "responded" => Ok(AgentRunStatus::Responded),
        "max_iterations" => Ok(AgentRunStatus::MaxIterations),
        "failed" => Ok(AgentRunStatus::Failed),
        other => Err(MidgardError::Storage(format!(
            "unknown stored agent status: {other}"
        ))),
    }
}

fn approval_status_from_storage(status: &str) -> MidgardResult<ApprovalStatus> {
    match status {
        "pending" => Ok(ApprovalStatus::Pending),
        "approved" => Ok(ApprovalStatus::Approved),
        "rejected" => Ok(ApprovalStatus::Rejected),
        other => Err(MidgardError::Storage(format!(
            "unknown stored approval status: {other}"
        ))),
    }
}

pub(crate) fn risk_level_to_storage(risk_level: &RiskLevel) -> &'static str {
    match risk_level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

fn risk_level_from_storage(risk_level: &str) -> MidgardResult<RiskLevel> {
    match risk_level {
        "low" => Ok(RiskLevel::Low),
        "medium" => Ok(RiskLevel::Medium),
        "high" => Ok(RiskLevel::High),
        "critical" => Ok(RiskLevel::Critical),
        other => Err(MidgardError::Storage(format!(
            "unknown stored risk level: {other}"
        ))),
    }
}

pub(crate) fn optional_json<T>(value: &Option<T>) -> MidgardResult<Option<String>>
where
    T: serde::Serialize,
{
    value
        .as_ref()
        .map(|value| {
            serde_json::to_string(value)
                .map_err(|err| MidgardError::Storage(format!("serialize JSON: {err}")))
        })
        .transpose()
}

pub(crate) fn json_string<T>(value: &T) -> MidgardResult<String>
where
    T: serde::Serialize,
{
    serde_json::to_string(value)
        .map_err(|err| MidgardError::Storage(format!("serialize JSON: {err}")))
}

pub(crate) fn optional_tool_calls_json(
    tool_calls: &[AgentToolCall],
) -> MidgardResult<Option<String>> {
    if tool_calls.is_empty() {
        return Ok(None);
    }

    json_string(&tool_calls).map(Some)
}

fn optional_tool_calls(value: &stmt::Value) -> MidgardResult<Vec<AgentToolCall>> {
    let Some(json) = optional_string_from_value(value)? else {
        return Ok(Vec::new());
    };

    serde_json::from_str(&json)
        .map_err(|err| MidgardError::Storage(format!("deserialize tool calls: {err}")))
}

pub(crate) fn optional_pending_approval(
    value: &stmt::Value,
) -> MidgardResult<Option<PendingApproval>> {
    let Some(json) = optional_string_from_value(value)? else {
        return Ok(None);
    };

    serde_json::from_str(&json)
        .map(Some)
        .map_err(|err| MidgardError::Storage(format!("deserialize pending approval: {err}")))
}

pub(crate) fn uuid_from_value(value: &stmt::Value) -> MidgardResult<Uuid> {
    match value {
        stmt::Value::Uuid(value) => Ok(*value),
        other => Err(MidgardError::Storage(format!(
            "expected uuid, got {other:?}"
        ))),
    }
}

pub(crate) fn i64_from_value(value: &stmt::Value) -> MidgardResult<i64> {
    match value {
        stmt::Value::I64(value) => Ok(*value),
        other => Err(MidgardError::Storage(format!(
            "expected i64, got {other:?}"
        ))),
    }
}

pub(crate) fn string_from_value(value: &stmt::Value) -> MidgardResult<&str> {
    match value {
        stmt::Value::String(value) => Ok(value),
        other => Err(MidgardError::Storage(format!(
            "expected string, got {other:?}"
        ))),
    }
}

pub(crate) fn optional_string_from_value(value: &stmt::Value) -> MidgardResult<Option<String>> {
    match value {
        stmt::Value::String(value) => Ok(Some(value.clone())),
        stmt::Value::Null => Ok(None),
        other => Err(MidgardError::Storage(format!(
            "expected nullable string, got {other:?}"
        ))),
    }
}

pub(crate) fn storage_error(err: toasty::Error) -> MidgardError {
    MidgardError::Storage(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_mapping_restores_agent_message_role_and_content() {
        let row = stmt::Value::record_from_vec(vec![
            stmt::Value::from("assistant"),
            stmt::Value::from("called list_pods"),
            stmt::Value::Null,
            stmt::Value::Null,
        ]);

        let message = agent_message_from_row(row).unwrap();

        assert_eq!(message.role, AgentRole::Assistant);
        assert_eq!(message.content, "called list_pods");
    }

    #[test]
    fn unknown_stored_role_returns_storage_error() {
        let err = role_from_storage("operator").unwrap_err();

        assert!(matches!(err, MidgardError::Storage(_)));
        assert!(err.to_string().contains("unknown stored agent role"));
    }
}
