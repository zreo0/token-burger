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
        self.save_secret_with_interaction(
            provider_id,
            account_key,
            secret_kind,
            secret,
            label,
            true,
        )
    }

    /**
     * 按交互权限保存系统凭据，返回公开引用，所有系统存取共享同一把锁
     */
    pub(crate) fn save_secret_with_interaction(
        &self,
        provider_id: &str,
        account_key: &str,
        secret_kind: &str,
        secret: &str,
        label: Option<String>,
        allow_interaction: bool,
    ) -> Result<CredentialMetadata, AccountUsageError> {
        let credential_ref = Self::credential_ref(provider_id, account_key, secret_kind);
        with_keychain_interaction(allow_interaction, || {
            write_os_secret(&self.service_name, &credential_ref, secret)
        })?;
        Ok(CredentialMetadata {
            provider_id: provider_id.to_string(),
            account_key: account_key.to_string(),
            secret_kind: secret_kind.to_string(),
            credential_ref,
            label,
        })
    }

    pub fn load_secret(&self, credential_ref: &str) -> Result<String, AccountUsageError> {
        with_keychain_interaction(true, || read_os_secret(&self.service_name, credential_ref))
    }

    /**
     * 无交互读取指定凭据，系统需要授权时返回错误而不弹窗
     */
    pub fn load_secret_silently(&self, credential_ref: &str) -> Result<String, AccountUsageError> {
        with_keychain_interaction(false, || read_os_secret(&self.service_name, credential_ref))
    }

    pub fn delete_secret(&self, credential_ref: &str) -> Result<(), AccountUsageError> {
        with_keychain_interaction(true, || {
            delete_os_secret(&self.service_name, credential_ref)
        })
    }
}

/**
 * 按交互权限执行凭据操作，返回操作结果；macOS 上串行保护进程级弹窗开关
 */
pub(crate) fn with_keychain_interaction<T>(
    allow_interaction: bool,
    operation: impl FnOnce() -> Result<T, AccountUsageError>,
) -> Result<T, AccountUsageError> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::os::macos::keychain::SecKeychain;
        static KEYCHAIN_ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = KEYCHAIN_ACCESS.lock().map_err(|_| {
            AccountUsageError::new(
                AccountUsageStatus::CredentialUnavailable,
                "无法读取系统凭据",
            )
        })?;
        let _interaction = if allow_interaction {
            None
        } else {
            Some(SecKeychain::disable_user_interaction().map_err(|_| {
                AccountUsageError::new(
                    AccountUsageStatus::CredentialUnavailable,
                    "系统凭据需要授权，请点击重新授权",
                )
            })?)
        };
        operation()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = allow_interaction;
        operation()
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /**
     * 静默操作持锁期间阻止可交互操作进入，不读取任何真实凭据或请求系统弹窗
     */
    #[test]
    fn silent_keychain_access_serializes_interactive_operations() {
        use security_framework::os::macos::keychain::SecKeychain;
        use std::sync::mpsc;
        let (silent_tx, silent_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (interactive_tx, interactive_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let silent = scope.spawn(move || {
                with_keychain_interaction(false, || {
                    assert!(!SecKeychain::user_interaction_allowed().unwrap());
                    silent_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    assert!(!SecKeychain::user_interaction_allowed().unwrap());
                    Ok(())
                })
            });
            silent_rx.recv().unwrap();
            let interactive = scope.spawn(move || {
                with_keychain_interaction(true, || {
                    interactive_tx
                        .send(SecKeychain::user_interaction_allowed().unwrap())
                        .unwrap();
                    Ok(())
                })
            });
            let completed_early = interactive_rx.recv_timeout(std::time::Duration::from_millis(30));
            release_tx.send(()).unwrap();
            assert!(completed_early.is_err());
            assert!(interactive_rx.recv().unwrap());
            silent.join().unwrap().unwrap();
            interactive.join().unwrap().unwrap();
        });
    }
}
