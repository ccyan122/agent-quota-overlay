use super::AuthError;
use serde::Deserialize;

#[derive(Deserialize)] struct Credential { token: Token }
#[derive(Deserialize)] struct Token { access_token: String }
pub(crate) struct Credentials { pub access_token: String }

/// Reads the existing native credential in memory only. It never writes, refreshes, or logs it.
pub(crate) fn read() -> Result<Credentials, AuthError> {
    let raw = read_native_credential()?;
    let parsed: Credential = serde_json::from_slice(&raw).map_err(|_| AuthError::Malformed)?;
    if parsed.token.access_token.trim().is_empty() { return Err(AuthError::Missing); }
    Ok(Credentials { access_token: parsed.token.access_token })
}
pub fn credentials_present() -> bool { read().is_ok() }

#[cfg(windows)]
fn read_native_credential() -> Result<Vec<u8>, AuthError> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let name: Vec<u16> = "gemini:antigravity".encode_utf16().chain(Some(0)).collect();
    let mut pointer: *mut CREDENTIALW = null_mut();
    let ok = unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) };
    if ok == 0 || pointer.is_null() { return Err(AuthError::Missing); }
    let bytes = unsafe {
        let credential = &*pointer;
        if credential.CredentialBlobSize == 0 || credential.CredentialBlob.is_null() { Vec::new() }
        else { std::slice::from_raw_parts(credential.CredentialBlob, credential.CredentialBlobSize as usize).to_vec() }
    };
    unsafe { CredFree(pointer.cast()); }
    if bytes.is_empty() { Err(AuthError::Missing) } else { Ok(bytes) }
}
#[cfg(not(windows))]
fn read_native_credential() -> Result<Vec<u8>, AuthError> { Err(AuthError::Missing) }
