use async_trait::async_trait;
use midgard_agent::{AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval};
use midgard_core::MidgardResult;
use std::sync::Arc;
use uuid::Uuid;

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
