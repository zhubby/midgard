use async_trait::async_trait;
use midgard_agent::{
    AgentMessage, AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult};
use std::{collections::BTreeMap, sync::Mutex};
use uuid::Uuid;

use crate::{
    auth::{
        normalize_email, parse_rfc3339_utc, utc_now_rfc3339, AuthSession, AuthUser, AuthUserRecord,
        AuthUserUpdate, NewAuthAuditEvent, NewAuthSession, NewUser,
    },
    store::{AgentSessionStore, AuthStore},
};

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

#[derive(Default)]
pub struct MemoryAuthStore {
    users: Mutex<BTreeMap<Uuid, AuthUserRecord>>,
    sessions: Mutex<BTreeMap<String, AuthSession>>,
    audit_events: Mutex<Vec<NewAuthAuditEvent>>,
}

impl MemoryAuthStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn audit_event_count(&self) -> MidgardResult<usize> {
        Ok(self
            .audit_events
            .lock()
            .map_err(|_| MidgardError::Storage("auth audit store poisoned".to_string()))?
            .len())
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

#[async_trait]
impl AuthStore for MemoryAuthStore {
    async fn create_user(&self, user: NewUser) -> MidgardResult<AuthUser> {
        let email_lower = normalize_email(&user.email);
        if email_lower.is_empty() {
            return Err(MidgardError::Storage("user email is required".to_string()));
        }

        let mut users = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?;
        if users
            .values()
            .any(|record| record.user.email == email_lower)
        {
            return Err(MidgardError::Storage(format!(
                "user already exists: {email_lower}"
            )));
        }

        let now = utc_now_rfc3339();
        let auth_user = AuthUser {
            id: Uuid::new_v4(),
            email: email_lower,
            display_name: user.display_name.trim().to_string(),
            role: user.role,
            active: user.active,
            created_at: now.clone(),
            updated_at: now,
            last_login_at: None,
        };
        users.insert(
            auth_user.id,
            AuthUserRecord {
                user: auth_user.clone(),
                password_hash: user.password_hash,
            },
        );

        Ok(auth_user)
    }

    async fn list_users(&self) -> MidgardResult<Vec<AuthUser>> {
        let mut users = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .values()
            .map(|record| record.user.clone())
            .collect::<Vec<_>>();
        users.sort_by(|left, right| left.email.cmp(&right.email));
        Ok(users)
    }

    async fn load_user_by_id(&self, id: Uuid) -> MidgardResult<Option<AuthUser>> {
        Ok(self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .get(&id)
            .map(|record| record.user.clone()))
    }

    async fn load_user_by_email(&self, email_lower: &str) -> MidgardResult<Option<AuthUserRecord>> {
        let email_lower = normalize_email(email_lower);
        Ok(self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .values()
            .find(|record| record.user.email == email_lower)
            .cloned())
    }

    async fn update_user(
        &self,
        id: Uuid,
        update: AuthUserUpdate,
    ) -> MidgardResult<Option<AuthUser>> {
        let mut users = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?;
        let Some(record) = users.get_mut(&id) else {
            return Ok(None);
        };

        if let Some(display_name) = update.display_name {
            record.user.display_name = display_name.trim().to_string();
        }
        if let Some(role) = update.role {
            record.user.role = role;
        }
        if let Some(password_hash) = update.password_hash {
            record.password_hash = password_hash;
        }
        if let Some(active) = update.active {
            record.user.active = active;
        }
        record.user.updated_at = utc_now_rfc3339();

        Ok(Some(record.user.clone()))
    }

    async fn create_auth_session(&self, session: NewAuthSession) -> MidgardResult<AuthSession> {
        let auth_session = AuthSession {
            id: Uuid::new_v4(),
            user_id: session.user_id,
            token_hash: session.token_hash,
            created_at: session.created_at,
            expires_at: session.expires_at,
            revoked_at: None,
            user_agent: session.user_agent,
            ip_address: session.ip_address,
        };

        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("auth session store poisoned".to_string()))?
            .insert(auth_session.token_hash.clone(), auth_session.clone());

        if let Some(record) = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .get_mut(&auth_session.user_id)
        {
            record.user.last_login_at = Some(auth_session.created_at.clone());
            record.user.updated_at = auth_session.created_at.clone();
        }

        Ok(auth_session)
    }

    async fn load_user_by_session(
        &self,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> MidgardResult<Option<AuthUser>> {
        let session = self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("auth session store poisoned".to_string()))?
            .get(token_hash)
            .cloned();
        let Some(session) = session else {
            return Ok(None);
        };
        if session.revoked_at.is_some() || parse_rfc3339_utc(&session.expires_at)? <= now {
            return Ok(None);
        }

        let user = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .get(&session.user_id)
            .map(|record| record.user.clone())
            .filter(|user| user.active);

        Ok(user)
    }

    async fn revoke_auth_session(&self, token_hash: &str, revoked_at: String) -> MidgardResult<()> {
        if let Some(session) = self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("auth session store poisoned".to_string()))?
            .get_mut(token_hash)
        {
            session.revoked_at = Some(revoked_at);
        }

        Ok(())
    }

    async fn record_auth_audit_event(&self, event: NewAuthAuditEvent) -> MidgardResult<()> {
        self.audit_events
            .lock()
            .map_err(|_| MidgardError::Storage("auth audit store poisoned".to_string()))?
            .push(event);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{hash_password, session_token_hash, UserRole};
    use chrono::{Duration, Utc};
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

    #[tokio::test]
    async fn memory_auth_store_creates_and_loads_user_by_email() {
        let store = MemoryAuthStore::new();
        let user = store
            .create_user(NewUser {
                email: "Operator@Example.com ".to_string(),
                display_name: "Operator".to_string(),
                role: UserRole::Operator,
                password_hash: hash_password("valid-password").unwrap(),
                active: true,
            })
            .await
            .unwrap();

        let loaded = store
            .load_user_by_email("operator@example.com")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(loaded.user, user);
        assert_eq!(loaded.user.email, "operator@example.com");
    }

    #[tokio::test]
    async fn memory_auth_store_rejects_duplicate_email() {
        let store = MemoryAuthStore::new();
        let first = NewUser {
            email: "operator@example.com".to_string(),
            display_name: "Operator".to_string(),
            role: UserRole::Operator,
            password_hash: hash_password("valid-password").unwrap(),
            active: true,
        };
        store.create_user(first.clone()).await.unwrap();

        let err = store.create_user(first).await.unwrap_err();

        assert!(matches!(err, MidgardError::Storage(_)));
        assert!(err.to_string().contains("user already exists"));
    }

    #[tokio::test]
    async fn memory_auth_store_rejects_revoked_and_expired_sessions() {
        let store = MemoryAuthStore::new();
        let user = store
            .create_user(NewUser {
                email: "operator@example.com".to_string(),
                display_name: "Operator".to_string(),
                role: UserRole::Operator,
                password_hash: hash_password("valid-password").unwrap(),
                active: true,
            })
            .await
            .unwrap();
        let token_hash = session_token_hash("session-token");
        let now = Utc::now();
        store
            .create_auth_session(NewAuthSession {
                user_id: user.id,
                token_hash: token_hash.clone(),
                created_at: now.to_rfc3339(),
                expires_at: (now + Duration::hours(1)).to_rfc3339(),
                user_agent: None,
                ip_address: None,
            })
            .await
            .unwrap();

        assert!(store
            .load_user_by_session(&token_hash, now)
            .await
            .unwrap()
            .is_some());

        store
            .revoke_auth_session(&token_hash, now.to_rfc3339())
            .await
            .unwrap();

        assert!(store
            .load_user_by_session(&token_hash, now)
            .await
            .unwrap()
            .is_none());
    }
}
