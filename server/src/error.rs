use crate::schema::store::KindRef;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("index not loaded yet")]
    NotReady,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("ambiguous kind '{input}': {} candidates; pass `project` or use `group/version/Kind`", candidates.len())]
    Ambiguous {
        input: String,
        candidates: Vec<KindRef>,
    },
    #[error("timed out after {0}s")]
    Timeout(u64),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::NotReady => "not_ready",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::NotFound(_) => "not_found",
            AppError::Ambiguous { .. } => "ambiguous",
            AppError::Timeout(_) => "timeout",
            AppError::Db(_) => "db",
            AppError::Internal(_) => "internal",
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(anyhow::anyhow!(e))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(anyhow::anyhow!(e))
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(e: tokio::task::JoinError) -> Self {
        AppError::Internal(anyhow::anyhow!("task failed: {e}"))
    }
}
