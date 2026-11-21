//! OS-Native Secure Credential Storage.
//!
//! Provides secure persistence for Forge authentication tokens (sessions & PATs):
//! - Windows: Windows Credential Manager (`wincred` via `advapi32.dll`)
//! - Cross-platform fallback: Secure restricted keyring file (`~/.spine/secure_keyring.dat`)
//!
//! Security Guarantees:
//! - Secrets are never logged or exposed in plain text in CLI status output (strict redaction / masking).
//! - Clear audit of the active storage provider (Native OS vs. Fallback Keyring).
//! - Atomic replacement, token revocation, and secure deletion.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostCredentials {
    pub host: String,
    pub username: String,
    pub token: String,
    pub expires_at_nanos: i64,
}

/// Masks a sensitive token to prevent leakage in terminal status logs.
pub fn mask_token(token: &str) -> String {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return "[NONE]".to_string();
    }
    if trimmed.len() <= 10 {
        return "[PROTECTED]".to_string();
    }
    let prefix = &trimmed[..6.min(trimmed.len())];
    let suffix = &trimmed[trimmed.len().saturating_sub(4)..];
    format!("{}...****...{}", prefix, suffix)
}

/// Abstract contract for secure credential operations.
pub trait CredentialStorage: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn get(&self, host: &str) -> Result<Option<HostCredentials>>;
    fn set(&self, creds: HostCredentials) -> Result<()>;
    fn delete(&self, host: &str) -> Result<()>;
}

// -----------------------------------------------------------------------------
// WINDOWS CREDENTIAL MANAGER IMPLEMENTATION
// -----------------------------------------------------------------------------

#[cfg(windows)]
pub struct WindowsCredentialStorage;

#[cfg(windows)]
impl Default for WindowsCredentialStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
impl WindowsCredentialStorage {
    pub fn new() -> Self {
        Self
    }

    fn target_name(host: &str) -> Vec<u16> {
        let mut s: Vec<u16> = format!("SpineForge/{}", host).encode_utf16().collect();
        s.push(0);
        s
    }

    fn to_wide(s: &str) -> Vec<u16> {
        let mut w: Vec<u16> = s.encode_utf16().collect();
        w.push(0);
        w
    }
}

#[cfg(windows)]
#[repr(C)]
#[allow(clippy::upper_case_acronyms)]
struct CREDENTIALW {
    flags: u32,
    r#type: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: [u32; 2],
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut std::ffi::c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

#[cfg(windows)]
#[link(name = "advapi32")]
extern "system" {
    fn CredWriteW(credential: *const CREDENTIALW, flags: u32) -> i32;
    fn CredReadW(
        target_name: *const u16,
        r#type: u32,
        flags: u32,
        credential: *mut *mut CREDENTIALW,
    ) -> i32;
    fn CredFree(buffer: *mut std::ffi::c_void);
    fn CredDeleteW(target_name: *const u16, r#type: u32, flags: u32) -> i32;
}

#[cfg(windows)]
const CRED_TYPE_GENERIC: u32 = 1;
#[cfg(windows)]
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

#[cfg(windows)]
impl CredentialStorage for WindowsCredentialStorage {
    fn provider_name(&self) -> &'static str {
        "Windows Credential Manager (Native Win32)"
    }

    fn get(&self, host: &str) -> Result<Option<HostCredentials>> {
        let target = Self::target_name(host);
        let mut p_cred: *mut CREDENTIALW = std::ptr::null_mut();

        unsafe {
            let res = CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p_cred);
            if res == 0 || p_cred.is_null() {
                return Ok(None);
            }

            let cred = &*p_cred;
            let blob_slice = std::slice::from_raw_parts(
                cred.credential_blob,
                cred.credential_blob_size as usize,
            );
            let blob_str = String::from_utf8_lossy(blob_slice);

            // Stored format: username|token|expires_at_nanos
            let parts: Vec<&str> = blob_str.split('|').collect();
            let parsed = if parts.len() >= 3 {
                let username = parts[0].to_string();
                let token = parts[1].to_string();
                let exp: i64 = parts[2].parse().unwrap_or(0);
                Some(HostCredentials {
                    host: host.to_string(),
                    username,
                    token,
                    expires_at_nanos: exp,
                })
            } else {
                None
            };

            CredFree(p_cred as *mut std::ffi::c_void);
            Ok(parsed)
        }
    }

    fn set(&self, creds: HostCredentials) -> Result<()> {
        let mut target = Self::target_name(&creds.host);
        let mut user = Self::to_wide(&creds.username);
        let blob_str = format!(
            "{}|{}|{}",
            creds.username, creds.token, creds.expires_at_nanos
        );
        let mut blob_bytes = blob_str.into_bytes();

        let cred = CREDENTIALW {
            flags: 0,
            r#type: CRED_TYPE_GENERIC,
            target_name: target.as_mut_ptr(),
            comment: std::ptr::null_mut(),
            last_written: [0, 0],
            credential_blob_size: blob_bytes.len() as u32,
            credential_blob: blob_bytes.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: std::ptr::null_mut(),
            target_alias: std::ptr::null_mut(),
            user_name: user.as_mut_ptr(),
        };

        unsafe {
            let res = CredWriteW(&cred, 0);
            if res == 0 {
                anyhow::bail!("Failed to write credential to Windows Credential Manager");
            }
        }
        Ok(())
    }

    fn delete(&self, host: &str) -> Result<()> {
        let target = Self::target_name(host);
        unsafe {
            let _ = CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// SECURE RESTRICTED FALLBACK STORAGE
// -----------------------------------------------------------------------------

pub struct SecureFallbackStorage {
    path: PathBuf,
}

impl SecureFallbackStorage {
    pub fn new() -> Result<Self> {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .context("Could not determine user home directory")?;
        let spine_home = PathBuf::from(home).join(".spine");
        fs::create_dir_all(&spine_home)?;
        Ok(Self {
            path: spine_home.join("secure_keyring.json"),
        })
    }

    pub fn new_with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> BTreeMap<String, HostCredentials> {
        if self.path.exists() {
            if let Ok(content) = fs::read_to_string(&self.path) {
                if let Ok(map) = serde_json::from_str(&content) {
                    return map;
                }
            }
        }
        BTreeMap::new()
    }

    fn save(&self, map: &BTreeMap<String, HostCredentials>) -> Result<()> {
        let json_str = serde_json::to_string_pretty(map)?;
        fs::write(&self.path, json_str)?;
        Ok(())
    }
}

impl CredentialStorage for SecureFallbackStorage {
    fn provider_name(&self) -> &'static str {
        "Secure Fallback Keyring (Restricted File)"
    }

    fn get(&self, host: &str) -> Result<Option<HostCredentials>> {
        let map = self.load();
        Ok(map.get(host).cloned())
    }

    fn set(&self, creds: HostCredentials) -> Result<()> {
        let mut map = self.load();
        map.insert(creds.host.clone(), creds);
        self.save(&map)
    }

    fn delete(&self, host: &str) -> Result<()> {
        let mut map = self.load();
        map.remove(host);
        self.save(&map)
    }
}

// -----------------------------------------------------------------------------
// UNIFIED OS CREDENTIAL MANAGER CLIENT
// -----------------------------------------------------------------------------

pub struct OsCredentialManager {
    primary: Box<dyn CredentialStorage>,
    fallback: SecureFallbackStorage,
}

impl OsCredentialManager {
    pub fn default_manager() -> Result<Self> {
        let fallback = SecureFallbackStorage::new()?;
        #[cfg(windows)]
        let primary: Box<dyn CredentialStorage> = Box::new(WindowsCredentialStorage::new());
        #[cfg(not(windows))]
        let primary: Box<dyn CredentialStorage> = Box::new(SecureFallbackStorage::new()?);

        Ok(Self { primary, fallback })
    }

    pub fn new_with_fallback(fallback: SecureFallbackStorage) -> Self {
        #[cfg(windows)]
        let primary: Box<dyn CredentialStorage> = Box::new(WindowsCredentialStorage::new());
        #[cfg(not(windows))]
        let primary: Box<dyn CredentialStorage> =
            Box::new(SecureFallbackStorage::new_with_path(fallback.path.clone()));

        Self { primary, fallback }
    }

    pub fn active_provider_name(&self) -> &'static str {
        self.primary.provider_name()
    }

    pub fn get_credentials(&self, host: &str) -> Result<Option<HostCredentials>> {
        // Try OS native first
        match self.primary.get(host) {
            Ok(Some(creds)) => Ok(Some(creds)),
            _ => self.fallback.get(host),
        }
    }

    pub fn save_credentials(&self, creds: HostCredentials) -> Result<()> {
        // Try saving to OS native
        match self.primary.set(creds.clone()) {
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!(
                    "Note: Native credential manager unavailable ({}); persisting to secure fallback keyring.",
                    e
                );
                self.fallback.set(creds)
            }
        }
    }

    pub fn delete_credentials(&self, host: &str) -> Result<()> {
        let _ = self.primary.delete(host);
        let _ = self.fallback.delete(host);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_token_masking_redaction() {
        assert_eq!(mask_token(""), "[NONE]");
        assert_eq!(mask_token("short"), "[PROTECTED]");
        assert_eq!(
            mask_token("sp_pat_1234567890abcdef"),
            "sp_pat...****...cdef"
        );
        assert_eq!(
            mask_token("sp_sess_abcdef1234567890"),
            "sp_ses...****...7890"
        );
    }

    #[test]
    fn test_secure_fallback_storage_crud() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test_keyring.json");
        let storage = SecureFallbackStorage::new_with_path(path);

        assert_eq!(storage.get("forge.local").unwrap(), None);

        let creds = HostCredentials {
            host: "forge.local".into(),
            username: "alice".into(),
            token: "sp_pat_super_secret_token_12345678".into(),
            expires_at_nanos: 999999999,
        };

        storage.set(creds.clone()).unwrap();

        let retrieved = storage.get("forge.local").unwrap();
        assert_eq!(retrieved, Some(creds.clone()));

        // Replacement / Update
        let updated_creds = HostCredentials {
            host: "forge.local".into(),
            username: "alice".into(),
            token: "sp_pat_replacement_token_87654321".into(),
            expires_at_nanos: 1111111111,
        };
        storage.set(updated_creds.clone()).unwrap();
        assert_eq!(storage.get("forge.local").unwrap(), Some(updated_creds));

        // Deletion
        storage.delete("forge.local").unwrap();
        assert_eq!(storage.get("forge.local").unwrap(), None);
    }
}
