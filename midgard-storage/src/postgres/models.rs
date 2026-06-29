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

#[derive(Debug, Clone, toasty::Model)]
#[table = "users"]
pub struct StoredAuthUser {
    #[key]
    pub id: Uuid,
    #[unique]
    pub email_lower: String,
    pub display_name: String,
    pub role: String,
    pub password_hash: String,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "auth_sessions"]
pub struct StoredAuthSession {
    #[key]
    pub id: Uuid,
    #[index]
    pub user_id: Uuid,
    #[unique]
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "auth_audit_events"]
pub struct StoredAuthAuditEvent {
    #[key]
    pub id: Uuid,
    #[index]
    pub user_id: Option<Uuid>,
    pub event_type: String,
    pub email_lower: Option<String>,
    pub occurred_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub detail_json: Option<String>,
}

pub fn storage_models() -> toasty::ModelSet {
    toasty::models!(
        StoredAgentSession,
        StoredAgentMessage,
        StoredAgentApprovalRecord,
        StoredAuthUser,
        StoredAuthSession,
        StoredAuthAuditEvent
    )
}
