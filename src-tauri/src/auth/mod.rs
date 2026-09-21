pub mod antigravity;
pub mod claude;
pub mod codex;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("credentials are unavailable")]
    Missing,
    #[error("credentials could not be read")]
    Unreadable,
    #[error("credentials are malformed")]
    Malformed,
}

pub(crate) fn home_dir() -> Result<PathBuf, AuthError> {
    std::env::var_os("USERPROFILE").map(PathBuf::from).ok_or(AuthError::Missing)
}
