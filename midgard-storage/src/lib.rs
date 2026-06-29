use async_trait::async_trait;
use midgard_agent::{
    AgentMessage, AgentRole, AgentRunStatus, AgentSession, AgentToolCall, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult};
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
}

pub type SharedAgentSessionStore = Arc<dyn AgentSessionStore>;

#[derive(Default)]
pub struct MemoryAgentSessionStore {
    sessions: Mutex<BTreeMap<Uuid, AgentSession>>,
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
        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .insert(session.id, session.clone());

        Ok(session)
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
    toasty::models!(StoredAgentSession, StoredAgentMessage)
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

fn optional_tool_calls_json(tool_calls: &[AgentToolCall]) -> MidgardResult<Option<String>> {
    if tool_calls.is_empty() {
        return Ok(None);
    }

    serde_json::to_string(tool_calls)
        .map(Some)
        .map_err(|err| MidgardError::Storage(format!("serialize tool calls: {err}")))
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
}
