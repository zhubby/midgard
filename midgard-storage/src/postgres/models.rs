use uuid::Uuid;

#[derive(Debug, Clone, toasty::Model)]
#[table = "agent_sessions"]
pub struct StoredAgentSession {
    #[key]
    pub id: Uuid,
    #[index]
    pub workspace_id: Uuid,
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
    #[index]
    pub system_role_id: Uuid,
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

#[derive(Debug, Clone, toasty::Model)]
#[table = "organizations"]
pub struct StoredOrganization {
    #[key]
    pub id: Uuid,
    #[unique]
    pub slug: String,
    pub name: String,
    #[index]
    pub created_by_user_id: Uuid,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "organization_memberships"]
pub struct StoredOrganizationMembership {
    #[key]
    pub id: Uuid,
    #[index]
    pub organization_id: Uuid,
    #[index]
    pub user_id: Uuid,
    pub role: String,
    #[index]
    pub role_id: Uuid,
    pub active: bool,
    pub joined_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "workspaces"]
pub struct StoredWorkspace {
    #[key]
    pub id: Uuid,
    #[index]
    pub organization_id: Uuid,
    pub slug: String,
    pub name: String,
    pub runtime_mode: Option<String>,
    pub runtime_config_ciphertext: Option<String>,
    pub runtime_config_summary_json: Option<String>,
    pub runtime_config_status: String,
    pub runtime_config_updated_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "middleware_instances"]
pub struct StoredMiddlewareInstance {
    #[key]
    pub id: Uuid,
    #[index]
    pub workspace_id: Uuid,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub desired_state: String,
    pub status: String,
    pub config_json: String,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "rbac_roles"]
pub struct StoredRbacRole {
    #[key]
    pub id: Uuid,
    #[index]
    pub scope_kind: String,
    #[index]
    pub organization_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    #[index]
    pub builtin_key: Option<String>,
    pub protected: bool,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "rbac_role_permissions"]
pub struct StoredRbacRolePermission {
    #[key]
    #[auto]
    pub id: u64,
    #[index]
    pub role_id: Uuid,
    pub permission_key: String,
}

pub fn storage_models() -> toasty::ModelSet {
    toasty::models!(
        StoredAgentSession,
        StoredAgentMessage,
        StoredAgentApprovalRecord,
        StoredAuthUser,
        StoredAuthSession,
        StoredAuthAuditEvent,
        StoredOrganization,
        StoredOrganizationMembership,
        StoredWorkspace,
        StoredRbacRole,
        StoredRbacRolePermission,
        StoredMiddlewareInstance
    )
}
