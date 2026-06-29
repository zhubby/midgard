mod auth;
mod memory;
mod postgres;
mod store;

pub use auth::{
    generate_session_token, hash_password, normalize_email, parse_rfc3339_utc, session_token_hash,
    utc_now_rfc3339, verify_password, AuthSession, AuthUser, AuthUserRecord, AuthUserUpdate,
    NewAuthAuditEvent, NewAuthSession, NewUser, UserRole,
};
pub use memory::{MemoryAgentSessionStore, MemoryAuthStore};
pub use postgres::{
    connect_database, storage_models, PostgresAgentSessionStore, StoredAgentApprovalRecord,
    StoredAgentMessage, StoredAgentSession, StoredAuthAuditEvent, StoredAuthSession,
    StoredAuthUser,
};
pub use store::{AgentSessionStore, AuthStore, SharedAgentSessionStore, SharedAuthStore};
