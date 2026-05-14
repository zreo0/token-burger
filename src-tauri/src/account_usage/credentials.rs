use crate::account_usage::{AccountUsageError, AccountUsageStatus};

const SERVICE_NAME: &str = "dev.token-burger.account-usage";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CredentialMetadata {
    pub provider_id: String,
    pub account_key: String,
    pub secret_kind: String,
    pub credential_ref: String,
    pub label: Option<String>,
}

#[derive(Clone)]
pub struct CredentialStore {
    service_name: String,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self {
            service_name: SERVICE_NAME.to_string(),
        }
    }
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn credential_ref(provider_id: &str, account_key: &str, secret_kind: &str) -> String {
        format!("{}:{}:{}", provider_id, account_key, secret_kind)
    }

    pub fn save_secret(
        &self,
        provider_id: &str,
        account_key: &str,
        secret_kind: &str,
        secret: &str,
        label: Option<String>,
    ) -> Result<CredentialMetadata, AccountUsageError> {
        let credential_ref = Self::credential_ref(provider_id, account_key, secret_kind);
        write_os_secret(&self.service_name, &credential_ref, secret)?;
        Ok(CredentialMetadata {
            provider_id: provider_id.to_string(),
            account_key: account_key.to_string(),
            secret_kind: secret_kind.to_string(),
            credential_ref,
            label,
        })
    }

    pub fn load_secret(&self, credential_ref: &str) -> Result<String, AccountUsageError> {
        read_os_secret(&self.service_name, credential_ref)
    }

    pub fn delete_secret(&self, credential_ref: &str) -> Result<(), AccountUsageError> {
        delete_os_secret(&self.service_name, credential_ref)
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn entry(service: &str, credential_ref: &str) -> Result<keyring::Entry, AccountUsageError> {
    keyring::Entry::new(service, credential_ref).map_err(map_keyring_error)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn write_os_secret(
    service: &str,
    credential_ref: &str,
    secret: &str,
) -> Result<(), AccountUsageError> {
    entry(service, credential_ref)?
        .set_password(secret)
        .map_err(map_keyring_error)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn read_os_secret(service: &str, credential_ref: &str) -> Result<String, AccountUsageError> {
    entry(service, credential_ref)?
        .get_password()
        .map_err(map_keyring_error)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn delete_os_secret(service: &str, credential_ref: &str) -> Result<(), AccountUsageError> {
    match entry(service, credential_ref)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(map_keyring_error(error)),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn map_keyring_error(error: keyring::Error) -> AccountUsageError {
    let code = match error {
        keyring::Error::NoEntry => AccountUsageStatus::AuthRequired,
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            AccountUsageStatus::CredentialUnavailable
        }
        _ => AccountUsageStatus::Error,
    };
    AccountUsageError::new(code, super::redact_secret_text(&error.to_string()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn write_os_secret(
    _service: &str,
    _credential_ref: &str,
    _secret: &str,
) -> Result<(), AccountUsageError> {
    Err(credential_unavailable())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn read_os_secret(_service: &str, _credential_ref: &str) -> Result<String, AccountUsageError> {
    Err(credential_unavailable())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn delete_os_secret(_service: &str, _credential_ref: &str) -> Result<(), AccountUsageError> {
    Err(credential_unavailable())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn credential_unavailable() -> AccountUsageError {
    AccountUsageError::new(
        AccountUsageStatus::CredentialUnavailable,
        "当前平台不支持系统凭据存储",
    )
}
