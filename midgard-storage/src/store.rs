use async_trait::async_trait;
use midgard_agent::{AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval};
use midgard_core::MidgardResult;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::{
    AuthSession, AuthUser, AuthUserRecord, AuthUserUpdate, NewAuthAuditEvent, NewAuthSession,
    NewUser,
};

#[async_trait]
pub trait AgentSessionStore: Send + Sync {
    async fn create_session(&self, goal: String) -> MidgardResult<AgentSession>;
    async fn append_user_message(&self, id: Uuid, message: String) -> MidgardResult<AgentSession>;
    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>>;
    async fn save_session(&self, session: AgentSession) -> MidgardResult<AgentSession>;
    async fn list_approval_records(&self, session_id: Uuid) -> MidgardResult<Vec<ApprovalRecord>>;
    async fn record_approval_decision(
        &self,
        session_id: Uuid,
        approval: PendingApproval,
        decision: ApprovalDecision,
        actor: String,
        reason: Option<String>,
    ) -> MidgardResult<ApprovalRecord>;
}

pub type SharedAgentSessionStore = Arc<dyn AgentSessionStore>;

#[async_trait]
pub trait AuthStore: Send + Sync {
    async fn create_user(&self, user: NewUser) -> MidgardResult<AuthUser>;
    async fn list_users(&self) -> MidgardResult<Vec<AuthUser>>;
    async fn load_user_by_id(&self, id: Uuid) -> MidgardResult<Option<AuthUser>>;
    async fn load_user_by_email(&self, email_lower: &str) -> MidgardResult<Option<AuthUserRecord>>;
    async fn update_user(
        &self,
        id: Uuid,
        update: AuthUserUpdate,
    ) -> MidgardResult<Option<AuthUser>>;
    async fn create_auth_session(&self, session: NewAuthSession) -> MidgardResult<AuthSession>;
    async fn load_user_by_session(
        &self,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> MidgardResult<Option<AuthUser>>;
    async fn revoke_auth_session(&self, token_hash: &str, revoked_at: String) -> MidgardResult<()>;
    async fn record_auth_audit_event(&self, event: NewAuthAuditEvent) -> MidgardResult<()>;
}

pub type SharedAuthStore = Arc<dyn AuthStore>;
