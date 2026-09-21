use std::{fs, path::{Path, PathBuf}};
use serde::Deserialize;
use super::{home_dir, AuthError};

#[derive(Deserialize)] struct CredentialsFile { #[serde(rename = "claudeAiOauth")] claude_ai_oauth: Option<OAuth> }
#[derive(Deserialize)] struct OAuth { #[serde(rename = "accessToken")] access_token: String }
pub(crate) struct Credentials { pub access_token: String }

pub(crate) fn default_path() -> Result<PathBuf, AuthError> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) { return Ok(PathBuf::from(dir).join(".credentials.json")); }
    Ok(home_dir()?.join(".claude").join(".credentials.json"))
}
pub(crate) fn read() -> Result<Credentials, AuthError> {
    if let Some(token) = std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").filter(|v| !v.is_empty()) {
        return Ok(Credentials { access_token: token.to_string_lossy().into_owned() });
    }
    read_path(&default_path()?)
}
pub fn credentials_present() -> bool { read().is_ok() }
pub(crate) fn read_path(path: &Path) -> Result<Credentials, AuthError> {
    let raw = fs::read(path).map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { AuthError::Missing } else { AuthError::Unreadable })?;
    let parsed: CredentialsFile = serde_json::from_slice(&raw).map_err(|_| AuthError::Malformed)?;
    let oauth = parsed.claude_ai_oauth.ok_or(AuthError::Missing)?;
    if oauth.access_token.trim().is_empty() { return Err(AuthError::Missing); }
    Ok(Credentials { access_token: oauth.access_token })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_the_claude_code_credentials_shape() {
        let parsed: CredentialsFile = serde_json::from_str(r#"{"claudeAiOauth":{"accessToken":"fixture-token"}}"#).unwrap();
        assert_eq!(parsed.claude_ai_oauth.unwrap().access_token, "fixture-token");
    }
    #[test] fn missing_oauth_object_is_not_a_credential() {
        let parsed: CredentialsFile = serde_json::from_str(r#"{}"#).unwrap();
        assert!(parsed.claude_ai_oauth.is_none());
    }
}
