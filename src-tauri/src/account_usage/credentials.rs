use crate::account_usage::{AccountUsageError, AccountUsageStatus};

/** 保留既有手填凭据的存储位置，本机导入凭据使用独立应用命名空间 */
const LEGACY_SERVICE_NAME: &str = "dev.token-burger.account-usage";

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
    /** 由实际应用标识和构建模式决定，隔离正式版与开发版的本机凭据副本 */
    local_service_name: String,
}

impl CredentialStore {
    /**
     * 根据应用标识创建凭据存储，既有手填凭据保持兼容，本机登录副本单独隔离
     */
    pub fn new(application_identifier: &str) -> Self {
        Self {
            service_name: LEGACY_SERVICE_NAME.to_string(),
            local_service_name: local_service_name(application_identifier, cfg!(debug_assertions)),
        }
    }

    /**
     * 返回本机登录副本的独立存储，不读取或迁移旧的共享钥匙串条目
     */
    pub(crate) fn for_local_login(&self) -> Self {
        Self {
            service_name: self.local_service_name.clone(),
            ..self.clone()
        }
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
 * 生成本机凭据服务名；即使 tauri dev 没有加载 dev 配置，也不与正式版共用
 */
fn local_service_name(application_identifier: &str, development: bool) -> String {
    let mode = if development { "dev" } else { "release" };
    format!("{application_identifier}.account-usage.{mode}")
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

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * 正式版、开发版及不同应用标识的本机副本相互隔离，旧手填凭据不自动迁移
     */
    #[test]
    fn local_login_namespaces_are_isolated() {
        let release = local_service_name("com.tokenburger.desktop", false);
        let dev = local_service_name("com.tokenburger.desktop", true);
        let configured_dev = local_service_name("com.tokenburger.desktop.dev", true);
        assert_eq!(release, "com.tokenburger.desktop.account-usage.release");
        assert_ne!(release, dev);
        assert_ne!(release, configured_dev);
        assert_ne!(dev, configured_dev);
        let store = CredentialStore::new("com.tokenburger.desktop");
        let local = store.for_local_login();
        assert_ne!(local.service_name, LEGACY_SERVICE_NAME);
        assert_eq!(
            local.service_name,
            local_service_name("com.tokenburger.desktop", cfg!(debug_assertions))
        );
        assert_eq!(local.for_local_login().service_name, local.service_name);
        assert_eq!(store.service_name, LEGACY_SERVICE_NAME);
    }

    /**
     * 静默操作持锁期间阻止可交互操作进入，不读取任何真实凭据或请求系统弹窗
     */
    #[test]
    #[cfg(target_os = "macos")]
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
