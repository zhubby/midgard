pub mod conditions;
pub mod control;
pub mod controller;
pub mod finalizers;
pub mod lease;
pub mod probe;
pub mod traits;

use thiserror::Error;

pub type OperatorResult<T> = Result<T, OperatorError>;

#[derive(Debug, Error)]
pub enum OperatorError {
    #[error("kubernetes API error: {0}")]
    Kube(#[from] kube::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("invalid operator configuration: {0}")]
    InvalidConfig(String),

    #[error("invalid operator state: {0}")]
    InvalidState(String),

    #[error("operator lease is held by {0}")]
    LeaseHeld(String),

    #[error("operator lease was lost: {0}")]
    LeaseLost(String),
}
