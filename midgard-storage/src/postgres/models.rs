use uuid::Uuid;

#[derive(Debug, Clone, toasty::Model)]
#[table = "agent_sessions"]
pub struct StoredAgentSession {
    #[key]
    pub id: Uuid,
    pub iteration_count: i64,
    pub status: String,
    pub pending_approval_json: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "agent_messages"]
pub struct StoredAgentMessage {
    #[key]
    #[auto]
    pub id: u64,
    #[index]
    pub session_id: Uuid,
    pub sequence: i64,
    pub role: String,
    pub content: String,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "agent_approval_records"]
pub struct StoredAgentApprovalRecord {
    #[key]
    pub id: Uuid,
    #[index]
    pub session_id: Uuid,
    pub tool_call_json: String,
    pub risk_level: String,
    pub status: String,
    pub requested_at: String,
    pub decided_at: Option<String>,
    pub actor: Option<String>,
    pub reason: Option<String>,
}

pub fn storage_models() -> toasty::ModelSet {
    toasty::models!(
        StoredAgentSession,
        StoredAgentMessage,
        StoredAgentApprovalRecord
    )
}
