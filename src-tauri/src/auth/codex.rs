use std::{fs, hash::{Hash, Hasher}, path::{Path, PathBuf}};
use serde::Deserialize;
use super::{home_dir, AuthError};

#[derive(Deserialize)] struct AuthFile { tokens: Option<Tokens> }
#[derive(Deserialize)] struct Tokens { access_token: String, account_id: Option<String> }
pub(crate) struct Credentials { pub access_token: String, pub account_id: Option<String> }

/// A process-local, non-serializable account context. It deliberately never
/// returns the account ID or bearer token, but still separates accounts when
/// an account header is absent.
pub(crate) fn account_context(credentials: &Credentials) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    credentials.account_id.hash(&mut hasher);
    credentials.access_token.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn default_path() -> Result<PathBuf, AuthError> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) { return Ok(PathBuf::from(home).join("auth.json")); }
    Ok(home_dir()?.join(".codex").join("auth.json"))
}

pub(crate) fn read() -> Result<Credentials, AuthError> { read_path(&default_path()?) }
pub fn credentials_present() -> bool { read().is_ok() }
pub(crate) fn read_path(path: &Path) -> Result<Credentials, AuthError> {
    let raw = fs::read(path).map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { AuthError::Missing } else { AuthError::Unreadable })?;
    let parsed: AuthFile = serde_json::from_slice(&raw).map_err(|_| AuthError::Malformed)?;
    let token = parsed.tokens.ok_or(AuthError::Missing)?;
    if token.access_token.trim().is_empty() { return Err(AuthError::Missing); }
    Ok(Credentials { access_token: token.access_token, account_id: token.account_id.filter(|id| !id.trim().is_empty()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn rejects_missing_token_shape() {
        let path = std::env::temp_dir().join("llm-quota-codex-missing-token.json");
        fs::write(&path, br#"{"tokens":{}}"#).unwrap();
        assert!(matches!(read_path(&path), Err(AuthError::Malformed)));
        let _ = fs::remove_file(path);
    }
}
