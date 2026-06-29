mod codec;
mod models;

use async_trait::async_trait;
use midgard_agent::{
    AgentMessage, AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult};
use toasty::{sql, stmt, Db, Executor};
use uuid::Uuid;

use crate::store::AgentSessionStore;

use codec::{
    agent_message_from_row, approval_record_from_row, i64_from_value, json_string, optional_json,
    optional_pending_approval, optional_string_from_value, optional_tool_calls_json,
    risk_level_to_storage, role_to_storage, status_from_storage, status_to_storage, storage_error,
    string_from_value, uuid_from_value,
};

pub use models::{
    storage_models, StoredAgentApprovalRecord, StoredAgentMessage, StoredAgentSession,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
