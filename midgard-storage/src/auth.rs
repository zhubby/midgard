use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use midgard_core::{MidgardError, MidgardResult};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;
use uuid::Uuid;

const PASSWORD_SALT_BYTES: usize = 16;
const SESSION_TOKEN_BYTES: usize = 32;
const MIN_PASSWORD_BYTES: usize = 8;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    Operator,
    Viewer,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::Operator => "operator",
            UserRole::Viewer => "viewer",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "admin" => Ok(Self::Admin),
            "operator" => Ok(Self::Operator),
            "viewer" => Ok(Self::Viewer),
            other => Err(MidgardError::Storage(format!(
                "unknown stored user role: {other}"
            ))),
        }
    }

    pub fn can_operate(&self) -> bool {
        matches!(self, UserRole::Admin | UserRole::Operator)
    }

    pub fn can_manage_users(&self) -> bool {
        matches!(self, UserRole::Admin)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct AuthUser {
    #[ts(type = "string")]
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: UserRole,
    #[ts(type = "string")]
    pub system_role_id: Uuid,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_login_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthUserRecord {
    pub user: AuthUser,
    pub password_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewUser {
    pub email: String,
    pub display_name: String,
    pub role: UserRole,
    pub system_role_id: Option<Uuid>,
    pub password_hash: String,
    pub active: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthUserUpdate {
    pub display_name: Option<String>,
    pub role: Option<UserRole>,
    pub system_role_id: Option<Uuid>,
    pub password_hash: Option<String>,
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewAuthSession {
    pub user_id: Uuid,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewAuthAuditEvent {
    pub user_id: Option<Uuid>,
    pub event_type: String,
    pub email_lower: Option<String>,
    pub occurred_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub detail_json: Option<String>,
}

pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

pub fn utc_now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub fn parse_rfc3339_utc(value: &str) -> MidgardResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| MidgardError::Storage(format!("invalid timestamp {value:?}: {err}")))
}

pub fn hash_password(password: &str) -> MidgardResult<String> {
    if password.len() < MIN_PASSWORD_BYTES {
        return Err(MidgardError::Configuration(format!(
            "password must be at least {MIN_PASSWORD_BYTES} bytes"
        )));
    }

    let mut salt_bytes = [0_u8; PASSWORD_SALT_BYTES];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|err| MidgardError::Storage(format!("encode password salt: {err}")))?;

    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| MidgardError::Storage(format!("hash password: {err}")))
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

pub fn generate_session_token() -> String {
    let mut token = [0_u8; SESSION_TOKEN_BYTES];
    OsRng.fill_bytes(&mut token);
    URL_SAFE_NO_PAD.encode(token)
}

pub fn session_token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hashes_verify_only_matching_passwords() {
        let hash = hash_password("correct-password").unwrap();

        assert!(verify_password("correct-password", &hash));
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn session_tokens_are_not_stored_directly() {
        let token = generate_session_token();
        let hash = session_token_hash(&token);

        assert_ne!(token, hash);
        assert_eq!(hash, session_token_hash(&token));
    }

    #[test]
    fn user_role_capabilities_are_ordered_for_v1() {
        assert!(UserRole::Admin.can_manage_users());
        assert!(UserRole::Admin.can_operate());
        assert!(UserRole::Operator.can_operate());
        assert!(!UserRole::Operator.can_manage_users());
        assert!(!UserRole::Viewer.can_operate());
    }
}
