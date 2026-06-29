use async_trait::async_trait;
use midgard_agent::{
    AgentMessage, AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult};
use std::{collections::BTreeMap, sync::Mutex};
use uuid::Uuid;

use crate::store::AgentSessionStore;

#[derive(Default)]
pub struct MemoryAgentSessionStore {
    sessions: Mutex<BTreeMap<Uuid, AgentSession>>,
    approval_records: Mutex<BTreeMap<Uuid, Vec<ApprovalRecord>>>,
}

impl MemoryAgentSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn upsert_pending_approval_record(&self, session: &AgentSession) -> MidgardResult<()> {
        let Some(approval) = &session.pending_approval else {
            return Ok(());
        };
        if approval.approved.is_some() {
            return Ok(());
        }
        let mut approval_records = self
            .approval_records
            .lock()
            .map_err(|_| MidgardError::Storage("approval store poisoned".to_string()))?;
        let records = approval_records.entry(session.id).or_default();
        if !records.iter().any(|record| record.id == approval.id) {
            records.push(ApprovalRecord::pending(session.id, approval));
        }

        Ok(())
    }
}

#[async_trait]
impl AgentSessionStore for MemoryAgentSessionStore {
    async fn create_session(&self, goal: String) -> MidgardResult<AgentSession> {
        let session = AgentSession::new(goal);
        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .insert(session.id, session.clone());

        Ok(session)
    }

    async fn append_user_message(&self, id: Uuid, message: String) -> MidgardResult<AgentSession> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?;
        let session = sessions.entry(id).or_insert_with(|| {
            let mut session = AgentSession::new("resumed session");
            session.id = id;
            session
        });

        session.messages.push(AgentMessage::user(message));

        Ok(session.clone())
    }

    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .get(&id)
            .cloned())
    }

    async fn save_session(&self, session: AgentSession) -> MidgardResult<AgentSession> {
        self.upsert_pending_approval_record(&session)?;
        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .insert(session.id, session.clone());

        Ok(session)
    }

    async fn list_approval_records(&self, session_id: Uuid) -> MidgardResult<Vec<ApprovalRecord>> {
        Ok(self
            .approval_records
            .lock()
            .map_err(|_| MidgardError::Storage("approval store poisoned".to_string()))?
            .get(&session_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn record_approval_decision(
        &self,
        session_id: Uuid,
        approval: PendingApproval,
        decision: ApprovalDecision,
        actor: String,
        reason: Option<String>,
    ) -> MidgardResult<ApprovalRecord> {
        let mut approval_records = self
            .approval_records
            .lock()
            .map_err(|_| MidgardError::Storage("approval store poisoned".to_string()))?;
        let records = approval_records.entry(session_id).or_default();
        let index = match records.iter().position(|record| record.id == approval.id) {
            Some(index) => index,
            None => {
                records.push(ApprovalRecord::pending(session_id, &approval));
                records.len() - 1
            }
        };

        records[index].record_decision(decision, actor, reason);

        Ok(records[index].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midgard_agent::{AgentRole, AgentToolCall, ApprovalStatus};
    use midgard_core::RiskLevel;

    #[tokio::test]
    async fn memory_store_creates_session_with_user_goal() {
        let store = MemoryAgentSessionStore::new();

        let session = store
            .create_session("inspect redis".to_string())
            .await
            .unwrap();

        assert_eq!(session.iteration_count, 0);
        assert_eq!(session.messages[0].role, AgentRole::User);
        assert_eq!(session.messages[0].content, "inspect redis");
        assert_eq!(store.load_session(session.id).await.unwrap(), Some(session));
    }

    #[tokio::test]
    async fn memory_store_appends_user_message_in_order() {
        let store = MemoryAgentSessionStore::new();
        let session = store
            .create_session("inspect redis".to_string())
            .await
            .unwrap();

        let session = store
            .append_user_message(session.id, "list pods".to_string())
            .await
            .unwrap();

        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].role, AgentRole::User);
        assert_eq!(session.messages[1].content, "list pods");
    }

    #[tokio::test]
    async fn memory_store_preserves_resumed_session_behavior_for_missing_id() {
        let store = MemoryAgentSessionStore::new();
        let id = Uuid::new_v4();

        let session = store
            .append_user_message(id, "continue".to_string())
            .await
            .unwrap();

        assert_eq!(session.messages[0].content, "resumed session");
        assert_eq!(session.messages[1].content, "continue");
    }

    #[tokio::test]
    async fn memory_store_saves_structured_tool_trace() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("inspect redis");
        session
            .messages
            .push(AgentMessage::assistant_with_tool_calls(
                "",
                vec![AgentToolCall::from_raw(
                    "call_1",
                    "redis_describe",
                    r#"{"namespace":"default","name":"cache"}"#,
                )],
            ));

        let saved = store.save_session(session).await.unwrap();
        let loaded = store.load_session(saved.id).await.unwrap().unwrap();

        assert_eq!(loaded.messages[1].tool_calls[0].name, "redis_describe");
        assert_eq!(
            loaded.messages[1].tool_calls[0].arguments["namespace"],
            "default"
        );
    }

    #[tokio::test]
    async fn pending_approval_save_creates_approval_record() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        let approval = pending_restart_approval();
        session.pending_approval = Some(approval.clone());

        store.save_session(session.clone()).await.unwrap();
        let records = store.list_approval_records(session.id).await.unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, approval.id);
        assert_eq!(records[0].status, ApprovalStatus::Pending);
        assert_eq!(records[0].actor, None);
    }

    #[tokio::test]
    async fn repeated_pending_approval_save_does_not_duplicate_record() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        session.pending_approval = Some(pending_restart_approval());

        store.save_session(session.clone()).await.unwrap();
        store.save_session(session.clone()).await.unwrap();
        let records = store.list_approval_records(session.id).await.unwrap();

        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn approval_decision_updates_existing_record() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        let approval = pending_restart_approval();
        session.pending_approval = Some(approval.clone());
        store.save_session(session.clone()).await.unwrap();

        let record = store
            .record_approval_decision(
                session.id,
                approval,
                ApprovalDecision::Approve,
                "operator@example.com".to_string(),
                Some("maintenance window".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(record.status, ApprovalStatus::Approved);
        assert_eq!(record.actor.as_deref(), Some("operator@example.com"));
        assert_eq!(record.reason.as_deref(), Some("maintenance window"));
        assert!(record.decided_at.is_some());
    }

    #[tokio::test]
    async fn approval_history_remains_after_pending_approval_is_cleared() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        let approval = pending_restart_approval();
        session.pending_approval = Some(approval.clone());
        store.save_session(session.clone()).await.unwrap();
        store
            .record_approval_decision(
                session.id,
                approval,
                ApprovalDecision::Reject,
                "operator@example.com".to_string(),
                None,
            )
            .await
            .unwrap();

        session.pending_approval = None;
        store.save_session(session.clone()).await.unwrap();
        let records = store.list_approval_records(session.id).await.unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ApprovalStatus::Rejected);
    }

    fn pending_restart_approval() -> PendingApproval {
        PendingApproval::new(
            AgentToolCall::from_raw(
                "call_1",
                "redis_restart",
                r#"{"namespace":"default","name":"cache"}"#,
            ),
            RiskLevel::High,
        )
    }
}
