use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SboxError {
    #[error("Command execution failed: {0}")]
    Execution(String),

    #[error("Command binary '{0}' not found on PATH")]
    BinaryNotFound(String),

    #[error("Landlock filesystem restriction error: {0}")]
    Landlock(String),

    #[error("Network namespace error: {0}")]
    NetworkNamespace(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid path: {0}")]
    InvalidPath(PathBuf),

    #[error("Lock error: {0}")]
    LockError(String),
}

pub type Result<T> = std::result::Result<T, SboxError>;
