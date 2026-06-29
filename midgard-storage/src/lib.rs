use async_trait::async_trait;
use midgard_agent::{
    AgentMessage, AgentRole, AgentRunStatus, AgentSession, AgentToolCall, ApprovalDecision,
    ApprovalRecord, ApprovalStatus, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult, RiskLevel};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use toasty::{sql, stmt, Db, Executor};
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

#[derive(Default)]
pub struct MemoryAgentSessionStore {
    sessions: Mutex<BTreeMap<Uuid, AgentSession>>,
    approval_records: Mutex<BTreeMap<Uuid, Vec<ApprovalRecord>>>,
}

impl MemoryAgentSessionStore {
    pub fn new() -> Self {
        Self::default()
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

impl MemoryAgentSessionStore {
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

#[derive(Clone)]
pub struct PostgresAgentSessionStore {
    db: Db,
}

impl PostgresAgentSessionStore {
    pub async fn connect(database_url: &str) -> MidgardResult<Self> {
        let db = connect_database(database_url).await?;
        Ok(Self::new(db))
    }

    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}

pub async fn connect_database(database_url: &str) -> MidgardResult<Db> {
    toasty::Db::builder()
        .models(storage_models())
        .connect(database_url)
        .await
        .map_err(|err| MidgardError::Storage(format!("failed to connect database: {err}")))
}

pub fn storage_models() -> toasty::ModelSet {
    toasty::models!(
        StoredAgentSession,
        StoredAgentMessage,
        StoredAgentApprovalRecord
    )
}

#[async_trait]
impl AgentSessionStore for PostgresAgentSessionStore {
    async fn create_session(&self, goal: String) -> MidgardResult<AgentSession> {
        let session = AgentSession::new(goal);
        self.save_session(session).await
    }

    async fn append_user_message(&self, id: Uuid, message: String) -> MidgardResult<AgentSession> {
        let mut session = match self.load_session(id).await? {
            Some(session) => session,
            None => {
                let mut session = AgentSession::new("resumed session");
                session.id = id;
                session
            }
        };

        session.messages.push(AgentMessage::user(message));
        self.save_session(session).await
    }

    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>> {
        let mut db = self.db.clone();
        load_session_with_executor(&mut db, id).await
    }

    async fn save_session(&self, session: AgentSession) -> MidgardResult<AgentSession> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;

        upsert_session(&mut tx, &session).await?;
        if let Some(approval) = &session.pending_approval {
            if approval.approved.is_none() {
                upsert_pending_approval_record(&mut tx, session.id, approval).await?;
            }
        }
        sql::statement("DELETE FROM agent_messages WHERE session_id = $1")
            .bind(session.id)
            .exec(&mut tx)
            .await
            .map_err(storage_error)?;

        for (sequence, message) in session.messages.iter().enumerate() {
            insert_message(&mut tx, session.id, sequence as i64, message).await?;
        }

        tx.commit().await.map_err(storage_error)?;
        Ok(session)
    }

    async fn list_approval_records(&self, session_id: Uuid) -> MidgardResult<Vec<ApprovalRecord>> {
        let mut db = self.db.clone();
        list_approval_records_with_executor(&mut db, session_id).await
    }

    async fn record_approval_decision(
        &self,
        session_id: Uuid,
        approval: PendingApproval,
        decision: ApprovalDecision,
        actor: String,
        reason: Option<String>,
    ) -> MidgardResult<ApprovalRecord> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;

        upsert_pending_approval_record(&mut tx, session_id, &approval).await?;
        let mut record =
            load_approval_record_with_executor(&mut tx, session_id, approval.id).await?;
        record.record_decision(decision, actor, reason);
        update_approval_record_decision(&mut tx, &record).await?;
        tx.commit().await.map_err(storage_error)?;

        Ok(record)
    }
}

async fn upsert_session(executor: &mut dyn Executor, session: &AgentSession) -> MidgardResult<()> {
    sql::statement(
        "INSERT INTO agent_sessions (id, iteration_count, status, pending_approval_json, last_error)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO UPDATE SET
            iteration_count = EXCLUDED.iteration_count,
            status = EXCLUDED.status,
            pending_approval_json = EXCLUDED.pending_approval_json,
            last_error = EXCLUDED.last_error",
    )
        .bind(session.id)
        .bind(session.iteration_count as i64)
        .bind(status_to_storage(&session.status))
        .bind(optional_json(&session.pending_approval)?)
        .bind(session.last_error.clone())
        .exec(executor)
        .await
        .map_err(storage_error)?;

    Ok(())
}

async fn insert_message(
    executor: &mut dyn Executor,
    session_id: Uuid,
    sequence: i64,
    message: &AgentMessage,
) -> MidgardResult<()> {
    sql::statement(
        "INSERT INTO agent_messages (session_id, sequence, role, content, tool_calls_json, tool_call_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(session_id)
    .bind(sequence)
    .bind(role_to_storage(&message.role))
    .bind(message.content.clone())
    .bind(optional_tool_calls_json(&message.tool_calls)?)
    .bind(message.tool_call_id.clone())
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn upsert_pending_approval_record(
    executor: &mut dyn Executor,
    session_id: Uuid,
    approval: &PendingApproval,
) -> MidgardResult<()> {
    let record = ApprovalRecord::pending(session_id, approval);
    sql::statement(
        "INSERT INTO agent_approval_records
            (id, session_id, tool_call_json, risk_level, status, requested_at, decided_at, actor, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(record.id)
    .bind(record.session_id)
    .bind(json_string(&record.tool_call)?)
    .bind(risk_level_to_storage(&record.risk_level))
    .bind(record.status.as_str())
    .bind(record.requested_at)
    .bind(record.decided_at)
    .bind(record.actor)
    .bind(record.reason)
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn update_approval_record_decision(
    executor: &mut dyn Executor,
    record: &ApprovalRecord,
) -> MidgardResult<()> {
    sql::statement(
        "UPDATE agent_approval_records
         SET status = $1, decided_at = $2, actor = $3, reason = $4
         WHERE id = $5 AND session_id = $6",
    )
    .bind(record.status.as_str())
    .bind(record.decided_at.clone())
    .bind(record.actor.clone())
    .bind(record.reason.clone())
    .bind(record.id)
    .bind(record.session_id)
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn load_session_with_executor(
    executor: &mut dyn Executor,
    id: Uuid,
) -> MidgardResult<Option<AgentSession>> {
    let session_rows = sql::query(
        "SELECT id, iteration_count, status, pending_approval_json, last_error
         FROM agent_sessions WHERE id = $1",
    )
    .bind(id)
    .column_types([
        stmt::Type::Uuid,
        stmt::Type::I64,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    let Some(session_row) = session_rows.into_iter().next() else {
        return Ok(None);
    };
    let session_record = session_row.into_record();
    let id = uuid_from_value(&session_record[0])?;
    let iteration_count = i64_from_value(&session_record[1])? as usize;
    let status = status_from_storage(string_from_value(&session_record[2])?)?;
    let pending_approval = optional_pending_approval(&session_record[3])?;
    let last_error = optional_string_from_value(&session_record[4])?;

    let message_rows = sql::query(
        "SELECT role, content, tool_calls_json, tool_call_id
         FROM agent_messages WHERE session_id = $1 ORDER BY sequence ASC",
    )
    .bind(id)
    .column_types([
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    let mut messages = Vec::with_capacity(message_rows.len());
    for row in message_rows {
        messages.push(agent_message_from_row(row)?);
    }

    Ok(Some(AgentSession {
        id,
        messages,
        iteration_count,
        status,
        pending_approval,
        last_error,
    }))
}

async fn list_approval_records_with_executor(
    executor: &mut dyn Executor,
    session_id: Uuid,
) -> MidgardResult<Vec<ApprovalRecord>> {
    let rows = sql::query(
        "SELECT id, session_id, tool_call_json, risk_level, status, requested_at, decided_at, actor, reason
         FROM agent_approval_records
         WHERE session_id = $1
         ORDER BY requested_at ASC, id ASC",
    )
    .bind(session_id)
    .column_types([
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    rows.into_iter()
        .map(approval_record_from_row)
        .collect::<MidgardResult<Vec<_>>>()
}

async fn load_approval_record_with_executor(
    executor: &mut dyn Executor,
    session_id: Uuid,
    approval_id: Uuid,
) -> MidgardResult<ApprovalRecord> {
    let rows = sql::query(
        "SELECT id, session_id, tool_call_json, risk_level, status, requested_at, decided_at, actor, reason
         FROM agent_approval_records
         WHERE session_id = $1 AND id = $2",
    )
    .bind(session_id)
    .bind(approval_id)
    .column_types([
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    rows.into_iter()
        .next()
        .ok_or_else(|| MidgardError::Storage(format!("approval record not found: {approval_id}")))
        .and_then(approval_record_from_row)
}

fn agent_message_from_row(row: stmt::Value) -> MidgardResult<AgentMessage> {
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

fn approval_record_from_row(row: stmt::Value) -> MidgardResult<ApprovalRecord> {
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

fn role_to_storage(role: &AgentRole) -> &'static str {
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

fn status_to_storage(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Running => "running",
        AgentRunStatus::Completed => "completed",
        AgentRunStatus::AwaitingApproval => "awaiting_approval",
        AgentRunStatus::Responded => "responded",
        AgentRunStatus::MaxIterations => "max_iterations",
        AgentRunStatus::Failed => "failed",
    }
}

fn status_from_storage(status: &str) -> MidgardResult<AgentRunStatus> {
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

fn risk_level_to_storage(risk_level: &RiskLevel) -> &'static str {
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

fn optional_json<T>(value: &Option<T>) -> MidgardResult<Option<String>>
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

fn json_string<T>(value: &T) -> MidgardResult<String>
where
    T: serde::Serialize,
{
    serde_json::to_string(value)
        .map_err(|err| MidgardError::Storage(format!("serialize JSON: {err}")))
}

fn optional_tool_calls_json(tool_calls: &[AgentToolCall]) -> MidgardResult<Option<String>> {
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

fn optional_pending_approval(value: &stmt::Value) -> MidgardResult<Option<PendingApproval>> {
    let Some(json) = optional_string_from_value(value)? else {
        return Ok(None);
    };

    serde_json::from_str(&json)
        .map(Some)
        .map_err(|err| MidgardError::Storage(format!("deserialize pending approval: {err}")))
}

fn uuid_from_value(value: &stmt::Value) -> MidgardResult<Uuid> {
    match value {
        stmt::Value::Uuid(value) => Ok(*value),
        other => Err(MidgardError::Storage(format!(
            "expected uuid, got {other:?}"
        ))),
    }
}

fn i64_from_value(value: &stmt::Value) -> MidgardResult<i64> {
    match value {
        stmt::Value::I64(value) => Ok(*value),
        other => Err(MidgardError::Storage(format!(
            "expected i64, got {other:?}"
        ))),
    }
}

fn string_from_value(value: &stmt::Value) -> MidgardResult<&str> {
    match value {
        stmt::Value::String(value) => Ok(value),
        other => Err(MidgardError::Storage(format!(
            "expected string, got {other:?}"
        ))),
    }
}

fn optional_string_from_value(value: &stmt::Value) -> MidgardResult<Option<String>> {
    match value {
        stmt::Value::String(value) => Ok(Some(value.clone())),
        stmt::Value::Null => Ok(None),
        other => Err(MidgardError::Storage(format!(
            "expected nullable string, got {other:?}"
        ))),
    }
}

fn storage_error(err: toasty::Error) -> MidgardError {
    MidgardError::Storage(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn postgres_integration_uses_test_database_url_when_available() {
        let Ok(database_url) = std::env::var("MIDGARD_TEST_DATABASE_URL") else {
            return;
        };
        let store = PostgresAgentSessionStore::connect(&database_url)
            .await
            .unwrap();
        store.db().push_schema().await.unwrap();

        let session = store
            .create_session("inspect redis".to_string())
            .await
            .unwrap();
        let loaded = store.load_session(session.id).await.unwrap().unwrap();

        assert_eq!(loaded.messages[0].content, "inspect redis");
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
