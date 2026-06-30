use thiserror::Error;

#[derive(Debug, Error)]
pub enum DockerToolError {
    #[error("docker runtime is not available: {0}")]
    Runtime(String),
    #[error("invalid docker tool arguments: {0}")]
    InvalidArguments(String),
    #[error("docker API request failed while {0}: {1}")]
    Api(&'static str, String),
}
