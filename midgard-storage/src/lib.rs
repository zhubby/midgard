use async_trait::async_trait;
use midgard_agent::{AgentMessage, AgentRole, AgentSession};
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
        let session = sessions
            .entry(id)
            .or_insert_with(|| AgentSession::new("resumed session"));

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
}

#[derive(Debug, Clone, toasty::Model)]
#[table = "agent_sessions"]
pub struct StoredAgentSession {
    #[key]
    pub id: Uuid,
    pub iteration_count: i64,
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
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;

        insert_session(&mut tx, session.id, session.iteration_count).await?;
        let first = session.messages.first().ok_or_else(|| {
            MidgardError::Storage("new session has no initial message".to_string())
        })?;
        insert_message(&mut tx, session.id, 0, first).await?;
        tx.commit().await.map_err(storage_error)?;

        Ok(session)
    }

    async fn append_user_message(&self, id: Uuid, message: String) -> MidgardResult<AgentSession> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;
        let mut session = match load_session_with_executor(&mut tx, id).await? {
            Some(session) => session,
            None => {
                insert_session(&mut tx, id, 0).await?;
                let resumed = AgentMessage::user("resumed session");
                insert_message(&mut tx, id, 0, &resumed).await?;

                AgentSession {
                    id,
                    messages: vec![resumed],
                    iteration_count: 0,
                }
            }
        };

        let next_sequence = session.messages.len() as i64;
        let message = AgentMessage::user(message);
        insert_message(&mut tx, id, next_sequence, &message).await?;
        session.messages.push(message);
        tx.commit().await.map_err(storage_error)?;

        Ok(session)
    }

    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>> {
        let mut db = self.db.clone();
        load_session_with_executor(&mut db, id).await
    }
}

async fn insert_session(
    executor: &mut dyn Executor,
    id: Uuid,
    iteration_count: usize,
) -> MidgardResult<()> {
    sql::statement("INSERT INTO agent_sessions (id, iteration_count) VALUES ($1, $2)")
        .bind(id)
        .bind(iteration_count as i64)
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
        "INSERT INTO agent_messages (session_id, sequence, role, content) VALUES ($1, $2, $3, $4)",
    )
    .bind(session_id)
    .bind(sequence)
    .bind(role_to_storage(&message.role))
    .bind(message.content.clone())
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn load_session_with_executor(
    executor: &mut dyn Executor,
    id: Uuid,
) -> MidgardResult<Option<AgentSession>> {
    let session_rows = sql::query("SELECT id, iteration_count FROM agent_sessions WHERE id = $1")
        .bind(id)
        .column_types([stmt::Type::Uuid, stmt::Type::I64])
        .exec(executor)
        .await
        .map_err(storage_error)?;

    let Some(session_row) = session_rows.into_iter().next() else {
        return Ok(None);
    };
    let session_record = session_row.into_record();
    let id = uuid_from_value(&session_record[0])?;
    let iteration_count = i64_from_value(&session_record[1])? as usize;

    let message_rows = sql::query(
        "SELECT role, content FROM agent_messages WHERE session_id = $1 ORDER BY sequence ASC",
    )
    .bind(id)
    .column_types([stmt::Type::String, stmt::Type::String])
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
    }))
}

fn agent_message_from_row(row: stmt::Value) -> MidgardResult<AgentMessage> {
    let record = row.into_record();
    let role = string_from_value(&record[0])?;
    let content = string_from_value(&record[1])?;

    Ok(AgentMessage {
        role: role_from_storage(role)?,
        content: content.to_string(),
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

    #[test]
    fn row_mapping_restores_agent_message_role_and_content() {
        let row = stmt::Value::record_from_vec(vec![
            stmt::Value::from("assistant"),
            stmt::Value::from("called list_pods"),
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
