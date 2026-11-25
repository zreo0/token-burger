use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::account_usage::credentials::{CredentialMetadata, CredentialStore};
use crate::account_usage::{AccountUsageError, AccountUsageStatus};

/** Cursor 的本机凭据副本与旧版手填 Cookie 使用不同引用 */
pub(super) const CREDENTIAL_REF: &str = "cursor:default:local";
/** Cursor Safe Storage 的密文格式标记 */
const SEALED_PREFIX: &str = "cursor-safe-storage:v1:";

/**
 * 保存查询用量所需的登录信息，不读取或轮换 Cursor 的 refresh token
 */
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct CursorCredential {
    /** 只进入内存或系统凭据存储，不发送到前端 */
    pub access_token: String,
    /** Cursor 的稳定用户标识，用于会话 Cookie 和账号匹配 */
    pub user_id: String,
    /** 本机缓存的账号邮箱，仅用于展示 */
    pub email: Option<String>,
}

/**
 * 返回当前系统 Cursor 登录数据库的位置，不访问数据库或钥匙串
 */
pub(super) fn database_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let root = dirs::home_dir()?.join("Library/Application Support");
    #[cfg(target_os = "windows")]
    let root = dirs::config_dir()?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = dirs::config_dir()?;
    Some(root.join("Cursor/User/globalStorage/state.vscdb"))
}

/**
 * 按用户交互权限读取本机登录，返回凭据；不会修改 Cursor 数据库
 */
pub(super) fn read_local(allow_interaction: bool) -> Result<CursorCredential, AccountUsageError> {
    let path = database_path().ok_or_else(login_required)?;
    read_database(&path, |stored| open_token(stored, allow_interaction))
}

/**
 * 只读提取登录字段，通过传入的解密函数返回凭据，便于验证不依赖真实钥匙串
 */
fn read_database(
    path: &Path,
    open: impl FnOnce(&str) -> Result<String, AccountUsageError>,
) -> Result<CursorCredential, AccountUsageError> {
    if !path.exists() {
        return Err(login_required());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| auth_error("无法读取 Cursor 登录，请打开 Cursor 后重试"))?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| auth_error("Cursor 登录数据暂不可读，请稍后重试"))?;
    let stored = read_item(&conn, "cursorAuth/accessToken")?.ok_or_else(login_required)?;
    let access_token = normalize_token(&open(&stored)?);
    if access_token.is_empty() {
        return Err(login_required());
    }
    // 账号切换时缓存标识可能滞后，以当前 token 携带的身份为准
    let user_id = token_subject(&access_token)
        .filter(|id| !id.trim().is_empty())
        .or(read_item(&conn, "cursorAuth/stripeMembershipAuthId")?)
        .map(|id| id.rsplit('|').next().unwrap_or(&id).to_string())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| auth_error("Cursor 登录信息不完整，请在 Cursor 重新登录"))?;
    let email = read_item(&conn, "cursorAuth/cachedEmail")?;
    Ok(CursorCredential {
        access_token,
        user_id,
        email,
    })
}

/**
 * 查询指定登录字段，兼容 SQLite TEXT/BLOB 与 JSON 字符串，返回非空文本
 */
fn read_item(conn: &Connection, key: &str) -> Result<Option<String>, AccountUsageError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| auth_error("无法读取 Cursor 登录信息"))?;
    Ok(raw.and_then(|raw| {
        let value = serde_json::from_str::<String>(&raw).unwrap_or(raw);
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    }))
}

/**
 * 将本机凭据副本写入 Token Burger 的系统凭据存储，返回引用
 */
pub(super) fn save(
    store: &CredentialStore,
    credential: &CursorCredential,
    allow_interaction: bool,
) -> Result<CredentialMetadata, AccountUsageError> {
    let secret =
        serde_json::to_string(credential).map_err(|_| auth_error("无法保存 Cursor 授权"))?;
    let store = store.for_local_login();
    let metadata = store
        .save_secret_with_interaction(
            "cursor",
            "default",
            "local",
            &secret,
            credential.email.clone(),
            allow_interaction,
        )
        .map_err(|_| {
            AccountUsageError::new(
                AccountUsageStatus::CredentialUnavailable,
                "已读取 Cursor 登录，但无法保存到当前 TokenBurger 的钥匙串条目，请重新授权",
            )
        })?;
    if allow_interaction {
        // 单次允许读取源凭据不代表保存的副本可供后台使用，确认回读后才报告授权完成
        verify_saved_copy(credential, store.load_secret_silently(CREDENTIAL_REF))?;
    }
    Ok(metadata)
}

/**
 * 验证授权副本可被静默读取且与刚导入的账号一致，失败时明确指出保存后的访问阶段
 */
fn verify_saved_copy(
    credential: &CursorCredential,
    saved: Result<String, AccountUsageError>,
) -> Result<(), AccountUsageError> {
    let matches = saved
        .ok()
        .and_then(|secret| serde_json::from_str::<CursorCredential>(&secret).ok())
        .is_some_and(|saved| &saved == credential);
    if matches {
        Ok(())
    } else {
        Err(AccountUsageError::new(AccountUsageStatus::CredentialUnavailable,
            "Cursor 登录已保存，但无法无交互读取 TokenBurger 的凭据副本；请解锁钥匙串并重新授权，需要持续访问时选择“始终允许”"))
    }
}

/**
 * 从 Token Burger 的加密存储无交互加载凭据，拒绝把旧 Cookie 当成本机授权
 */
pub(super) fn load(store: &CredentialStore) -> Result<CursorCredential, AccountUsageError> {
    let secret = store
        .for_local_login()
        .load_secret_silently(CREDENTIAL_REF)
        .map_err(|error| {
            AccountUsageError::new(
                error.code,
                "当前 TokenBurger 的 Cursor 凭据尚未授权或无法静默读取，请点击重新授权",
            )
        })?;
    serde_json::from_str::<CursorCredential>(&secret)
        .ok()
        .filter(|value| !value.access_token.is_empty() && !value.user_id.is_empty())
        .ok_or_else(|| auth_error("Cursor 授权已过期，请点击重新授权"))
}

/**
 * 从 JWT 的公开声明获取用户标识；仅用于匹配账号，不作为签名验证
 */
fn token_subject(token: &str) -> Option<String> {
    if let Some((user, _)) = token.split_once("::") {
        return Some(user.to_string());
    }
    let payload = token.split('.').nth(1)?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("sub")?.as_str().map(str::to_string)
}

/**
 * 规范化已解密的 token，兼容旧版带包装前缀的 JWT
 */
fn normalize_token(stored: &str) -> String {
    let stored = stored.trim();
    if stored.starts_with("eyJ") || stored.contains("::") {
        return stored.to_string();
    }
    let Some(tail) = stored.splitn(3, ':').nth(2) else {
        return stored.to_string();
    };
    if tail.starts_with("eyJ") {
        return tail.to_string();
    }
    general_purpose::STANDARD
        .decode(tail)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|value| value.starts_with("eyJ"))
        .unwrap_or_else(|| stored.to_string())
}

/**
 * 根据系统与交互权限解密 token，未加密的旧版凭据直接返回
 */
fn open_token(stored: &str, allow_interaction: bool) -> Result<String, AccountUsageError> {
    if !stored.starts_with(SEALED_PREFIX) {
        return Ok(stored.to_string());
    }
    #[cfg(target_os = "macos")]
    {
        use crate::account_usage::credentials::with_keychain_interaction;
        use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
        let password = with_keychain_interaction(allow_interaction, || {
            ItemSearchOptions::new()
                .class(ItemClass::generic_password())
                .service("Cursor Safe Storage")
                .account("Cursor Key")
                .load_data(true)
                .skip_authenticated_items(!allow_interaction)
                .search()
                .ok()
                .and_then(|items| {
                    items.into_iter().find_map(|item| match item {
                        SearchResult::Data(bytes) => Some(bytes),
                        _ => None,
                    })
                })
                .ok_or_else(|| {
                    AccountUsageError::new(
                    AccountUsageStatus::CredentialUnavailable,
                    "尚未获得 Cursor 登录读取权限，请点击重新授权，并允许访问 Cursor Safe Storage",
                )
                })
        })?;
        decrypt_token(stored, &password)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = allow_interaction;
        Err(AccountUsageError::new(
            AccountUsageStatus::Unsupported,
            "当前系统暂不支持读取 Cursor 的加密登录",
        ))
    }
}

/**
 * 按 macOS Electron v10 格式解密，参数为密文和钥匙串口令，返回明文 token
 */
#[cfg(target_os = "macos")]
fn decrypt_token(stored: &str, password: &[u8]) -> Result<String, AccountUsageError> {
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    let encoded = stored
        .strip_prefix(SEALED_PREFIX)
        .ok_or_else(|| auth_error("Cursor 登录密文格式不受支持"))?;
    let encrypted = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| auth_error("Cursor 登录密文格式不受支持，请更新 Cursor 后重新授权"))?;
    let cipher = encrypted
        .strip_prefix(b"v10")
        .ok_or_else(|| auth_error("Cursor 登录密文版本不受支持"))?;
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, &mut key);
    let result = cbc::Decryptor::<aes::Aes128>::new(&key.into(), &[0x20; 16].into())
        .decrypt_padded_vec_mut::<Pkcs7>(cipher)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| auth_error("无法解密 Cursor 登录，请重新授权"));
    key.fill(0);
    result
}

/**
 * 返回未登录状态，指导用户先完成 Cursor 自身登录
 */
fn login_required() -> AccountUsageError {
    auth_error("未检测到 Cursor 登录，请先打开 Cursor 并登录，再点击授权")
}

/**
 * 将固定说明转换为授权错误，不携带凭据或数据库内容
 */
fn auth_error(message: &str) -> AccountUsageError {
    AccountUsageError::new(AccountUsageStatus::AuthRequired, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * 保存后必须能静默读回同一份凭据，否则保持授权失败且错误中不包含密钥
     */
    #[test]
    fn saved_copy_must_be_silently_readable() {
        let credential = CursorCredential {
            access_token: "test-secret".into(),
            user_id: "user-test".into(),
            email: None,
        };
        assert!(
            verify_saved_copy(&credential, Ok(serde_json::to_string(&credential).unwrap())).is_ok()
        );
        let other = CursorCredential {
            user_id: "another-user".into(),
            ..credential.clone()
        };
        for result in [
            Err(AccountUsageError::new(
                AccountUsageStatus::CredentialUnavailable,
                "拒绝静默访问",
            )),
            Ok(serde_json::to_string(&other).unwrap()),
            Ok("invalid".to_string()),
        ] {
            let error = verify_saved_copy(&credential, result).unwrap_err();
            assert_eq!(error.code, AccountUsageStatus::CredentialUnavailable);
            assert!(error.message.contains("已保存"));
            assert!(!error.message.contains(&credential.access_token));
        }
    }

    /**
     * 验证只读导入兼容 BLOB 和 JSON 字符串，且不改变源数据库
     */
    #[test]
    fn reads_login_without_writing_source() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.vscdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value BLOB);
            INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', '"new-token"');
            INSERT INTO ItemTable VALUES ('cursorAuth/stripeMembershipAuthId', 'workos|user-1');
            INSERT INTO ItemTable VALUES ('cursorAuth/cachedEmail', 'test@example.com');"#,
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let credential = read_database(&path, |value| Ok(value.to_string())).unwrap();
        assert_eq!(credential.access_token, "new-token");
        assert_eq!(credential.user_id, "user-1");
        assert_eq!(credential.email.as_deref(), Some("test@example.com"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /**
     * 无数据库时不得访问钥匙串，也不得创建 Cursor 数据文件
     */
    #[test]
    fn missing_login_does_not_request_access() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing.vscdb");
        let error = read_database(&path, |_| panic!("不应读取钥匙串"))
            .err()
            .unwrap();
        assert_eq!(error.code, AccountUsageStatus::AuthRequired);
        assert!(!path.exists());
    }

    /**
     * 切换账号后以 token 身份为准，无法解析的旧 token 才使用缓存标识
     */
    #[test]
    fn token_identity_overrides_stale_membership_cache() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.vscdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value TEXT);
            INSERT INTO ItemTable VALUES ('cursorAuth/stripeMembershipAuthId', 'workos|old-user');
            INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', 'placeholder');",
        )
        .unwrap();
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"workos|new-user"}"#);
        for (token, expected) in [
            (format!("eyJ0eXAiOiJKV1QifQ.{payload}.test"), "new-user"),
            ("pair-user::test".to_string(), "pair-user"),
            ("opaque-token".to_string(), "old-user"),
        ] {
            conn.execute(
                "UPDATE ItemTable SET value = ?1 WHERE key = 'cursorAuth/accessToken'",
                [&token],
            )
            .unwrap();
            let credential = read_database(&path, |value| Ok(value.to_string())).unwrap();
            assert_eq!(credential.user_id, expected);
        }
    }

    /**
     * 使用公开测试向量验证实际解密，并拒绝错误口令
     */
    #[cfg(target_os = "macos")]
    #[test]
    fn decrypts_electron_v10() {
        let sealed = "cursor-safe-storage:v1:djEwWYhmSnp1q+qiTrBMIfHTGqxKxLmInuo8HX2UP86Z4BiIQadRbuQ680bP9VNiVxNHjA4ytuvGrbXz3Gw+8zuoOQ==";
        assert_eq!(
            decrypt_token(sealed, b"test-password").unwrap(),
            "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ1c2VyX3Rlc3QifQ.c2ln"
        );
        assert!(decrypt_token(sealed, b"wrong-password").is_err());
    }
}
