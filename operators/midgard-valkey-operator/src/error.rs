use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("kubernetes API error: {0}")]
    Kube(#[from] kube::Error),

    #[error("redis/valkey error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("operator error: {0}")]
    Operator(#[from] midgard_operator::OperatorError),

    #[error("invalid object state: {0}")]
    InvalidState(String),

    #[error("invalid operator configuration: {0}")]
    InvalidConfig(String),

    #[error("operator lease is held by {0}")]
    LeaseHeld(String),

    #[error("operator lease was lost: {0}")]
    LeaseLost(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("gRPC transport error: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),

    #[error("gRPC status error: {0}")]
    GrpcStatus(#[from] tonic::Status),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
