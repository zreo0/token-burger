use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use crate::account_usage::credentials::{CredentialMetadata, CredentialStore};
use crate::account_usage::providers;
use crate::account_usage::{
    now_rfc3339, redact_secret_text, AccountUsageError, AccountUsageProviderInfo,
    AccountUsageProviderState, AccountUsageRefreshContext, AccountUsageResult,
    AccountUsageSnapshot, AccountUsageStatus, SharedAccountUsageProvider,
};

const DEFAULT_REFRESH_TIMEOUT_SECS: u64 = 30;

pub struct AccountUsageManager {
    db_path: PathBuf,
    providers: Vec<SharedAccountUsageProvider>,
    refresh_locks: Mutex<HashSet<String>>,
    credentials: CredentialStore,
    refresh_timeout: Duration,
}

/// Provider 刷新结果，区分退避缓存和真实拉取结果
enum ProviderRefreshResult {
    Cached(Vec<AccountUsageSnapshot>),
    Fetched(AccountUsageResult),
}

impl AccountUsageManager {
    pub fn new(db_path: PathBuf) -> Self {
        let providers = providers::all_providers()
            .into_iter()
            .map(Arc::from)
            .collect::<Vec<SharedAccountUsageProvider>>();
        Self {
            db_path,
            providers,
            refresh_locks: Mutex::new(HashSet::new()),
            credentials: CredentialStore::new(),
            refresh_timeout: Duration::from_secs(DEFAULT_REFRESH_TIMEOUT_SECS),
        }
    }

    pub fn initialize_provider_states(&self) -> Result<(), String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        for provider in &self.providers {
            crate::account_usage::store::ensure_provider_state(
                &conn,
                provider.id(),
                provider.default_refresh_interval_secs(),
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn provider_infos(&self) -> Result<Vec<AccountUsageProviderInfo>, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        self.providers
            .iter()
            .map(|provider| {
                let state = self.state_or_default(
                    &conn,
                    provider.id(),
                    provider.default_refresh_interval_secs(),
                )?;
                Ok(provider.info(&state))
            })
            .collect()
    }

    pub fn latest_snapshots(&self) -> Result<Vec<AccountUsageSnapshot>, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        crate::account_usage::store::latest_snapshots(&conn).map_err(|error| error.to_string())
    }

    /// 返回启用 Provider 的最短刷新间隔秒数
    pub fn enabled_refresh_interval_secs(&self) -> Result<Option<u64>, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let interval = crate::account_usage::store::list_provider_states(&conn)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|state| state.enabled && state.refresh_interval_secs > 0)
            .map(|state| state.refresh_interval_secs)
            .min();

        Ok(interval)
    }

    pub fn refresh_all_enabled(&self) -> Result<Vec<AccountUsageSnapshot>, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let states = crate::account_usage::store::list_provider_states(&conn)
            .map_err(|error| error.to_string())?;
        let state_map = states
            .into_iter()
            .map(|state| (state.provider_id.clone(), state))
            .collect::<HashMap<_, _>>();
        drop(conn);

        let provider_ids = self
            .providers
            .iter()
            .filter(|provider| {
                state_map
                    .get(provider.id())
                    .map(|state| state.enabled)
                    .unwrap_or(false)
            })
            .map(|provider| provider.id().to_string())
            .collect::<Vec<_>>();
        let results = std::thread::scope(|scope| {
            let handles = provider_ids
                .into_iter()
                .map(|provider_id| {
                    let handle_provider_id = provider_id.clone();
                    let handle = scope.spawn(move || self.fetch_provider_usage(&provider_id));
                    (handle_provider_id, handle)
                })
                .collect::<Vec<_>>();

            handles
                .into_iter()
                .filter_map(|(provider_id, handle)| match handle.join() {
                    Ok(result) => Some((provider_id, result)),
                    Err(_) => {
                        log::warn!("账号用量 Provider 刷新线程崩溃: {}", provider_id);
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        let mut snapshots = Vec::new();
        for (provider_id, result) in results {
            match result.and_then(|refresh_result| {
                self.store_provider_refresh_result(&provider_id, refresh_result)
            }) {
                Ok(provider_snapshots) => snapshots.extend(provider_snapshots),
                Err(error) => {
                    log::warn!("账号用量 Provider 刷新失败: {}: {}", provider_id, error);
                }
            }
        }
        Ok(snapshots)
    }

    pub fn refresh_provider(&self, provider_id: &str) -> Result<Vec<AccountUsageSnapshot>, String> {
        let result = self.fetch_provider_usage(provider_id)?;
        self.store_provider_refresh_result(provider_id, result)
    }

    /// 拉取单个 Provider 的最新用量，但不在并行阶段写数据库
    fn fetch_provider_usage(&self, provider_id: &str) -> Result<ProviderRefreshResult, String> {
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .cloned()
            .ok_or_else(|| format!("未知账号用量 Provider: {provider_id}"))?;

        if let Some(snapshots) = self.rate_limit_backoff_snapshots(provider_id)? {
            return Ok(ProviderRefreshResult::Cached(snapshots));
        }

        let guard = match self.try_refresh_guard(provider_id) {
            Ok(guard) => guard,
            Err(error) => {
                // 后台刷新和手动刷新可能同时触发，已有缓存时直接复用，避免前端出现短暂错误
                let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
                let snapshots =
                    crate::account_usage::store::latest_snapshots_by_provider(&conn, provider_id)
                        .map_err(|error| error.to_string())?;
                if snapshots.is_empty() {
                    return Err(error);
                }
                return Ok(ProviderRefreshResult::Cached(snapshots));
            }
        };
        let context = AccountUsageRefreshContext {
            db_path: self.db_path.clone(),
            credentials: self.credentials.clone(),
        };
        let result = self.refresh_with_timeout(provider, context);
        drop(guard);

        Ok(ProviderRefreshResult::Fetched(result))
    }

    /// 顺序写入 Provider 刷新结果，避免多个线程同时写 SQLite
    fn store_provider_refresh_result(
        &self,
        provider_id: &str,
        result: ProviderRefreshResult,
    ) -> Result<Vec<AccountUsageSnapshot>, String> {
        match result {
            ProviderRefreshResult::Cached(snapshots) => Ok(snapshots),
            ProviderRefreshResult::Fetched(Ok(snapshots)) => {
                let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
                for snapshot in &snapshots {
                    crate::account_usage::store::upsert_snapshot(&conn, snapshot)
                        .map_err(|error| error.to_string())?;
                }
                self.update_last_refresh(&conn, provider_id)
                    .map_err(|error| error.to_string())?;
                Ok(snapshots)
            }
            ProviderRefreshResult::Fetched(Err(error)) => {
                self.handle_refresh_error(provider_id, error)
            }
        }
    }

    pub fn save_credential(
        &self,
        provider_id: String,
        account_key: Option<String>,
        secret_kind: String,
        secret: String,
        label: Option<String>,
    ) -> Result<AccountUsageProviderState, String> {
        let account_key = account_key.unwrap_or_else(|| "default".to_string());
        let metadata = self
            .credentials
            .save_secret(&provider_id, &account_key, &secret_kind, &secret, label)
            .map_err(|error| error.message)?;
        self.persist_credential_metadata(metadata)
    }

    pub fn clear_credential(
        &self,
        provider_id: String,
    ) -> Result<AccountUsageProviderState, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let provider = self.provider_by_id(&provider_id)?;
        let mut state = self.state_or_default(
            &conn,
            provider.id(),
            provider.default_refresh_interval_secs(),
        )?;
        if let Some(credential_ref) = &state.credential_ref {
            self.credentials
                .delete_secret(credential_ref)
                .map_err(|error| error.message)?;
        }
        state.credential_ref = None;
        state.credential_label = None;
        crate::account_usage::store::upsert_provider_state(&conn, &state)
            .map_err(|error| error.to_string())?;
        Ok(state)
    }

    pub fn set_provider_enabled(
        &self,
        provider_id: String,
        enabled: bool,
        refresh_interval_secs: Option<u64>,
    ) -> Result<AccountUsageProviderState, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let provider = self.provider_by_id(&provider_id)?;
        let mut state = self.state_or_default(
            &conn,
            provider.id(),
            provider.default_refresh_interval_secs(),
        )?;
        state.enabled = enabled;
        if let Some(interval) = refresh_interval_secs {
            state.refresh_interval_secs = interval;
        }
        crate::account_usage::store::upsert_provider_state(&conn, &state)
            .map_err(|error| error.to_string())?;
        Ok(state)
    }

    pub fn set_provider_menu_bar_visible(
        &self,
        provider_id: String,
        show_in_menu_bar: bool,
    ) -> Result<AccountUsageProviderState, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let provider = self.provider_by_id(&provider_id)?;
        let mut state = self.state_or_default(
            &conn,
            provider.id(),
            provider.default_refresh_interval_secs(),
        )?;
        state.show_in_menu_bar = show_in_menu_bar;
        crate::account_usage::store::upsert_provider_state(&conn, &state)
            .map_err(|error| error.to_string())?;
        Ok(state)
    }

    pub fn provider_state(&self, provider_id: String) -> Result<AccountUsageProviderState, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let provider = self.provider_by_id(&provider_id)?;
        self.state_or_default(
            &conn,
            provider.id(),
            provider.default_refresh_interval_secs(),
        )
    }

    fn provider_by_id(&self, provider_id: &str) -> Result<SharedAccountUsageProvider, String> {
        self.providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .cloned()
            .ok_or_else(|| format!("未知账号用量 Provider: {provider_id}"))
    }

    fn state_or_default(
        &self,
        conn: &Connection,
        provider_id: &str,
        refresh_interval_secs: u64,
    ) -> Result<AccountUsageProviderState, String> {
        crate::account_usage::store::get_provider_state(conn, provider_id)
            .map_err(|error| error.to_string())?
            .map(Ok)
            .unwrap_or_else(|| {
                let state = AccountUsageProviderState {
                    provider_id: provider_id.to_string(),
                    enabled: false,
                    show_in_menu_bar: false,
                    refresh_interval_secs,
                    last_refresh_at: None,
                    retry_after_until: None,
                    credential_ref: None,
                    credential_label: None,
                    auto_discovery_enabled: false,
                };
                crate::account_usage::store::upsert_provider_state(conn, &state)
                    .map_err(|error| error.to_string())?;
                Ok(state)
            })
    }

    fn try_refresh_guard(&self, provider_id: &str) -> Result<RefreshGuard<'_>, String> {
        let mut locks = self
            .refresh_locks
            .lock()
            .map_err(|_| "刷新锁已损坏".to_string())?;
        if !locks.insert(provider_id.to_string()) {
            return Err(format!("Provider 正在刷新: {provider_id}"));
        }
        Ok(RefreshGuard {
            provider_id: provider_id.to_string(),
            locks: &self.refresh_locks,
        })
    }

    fn refresh_with_timeout(
        &self,
        provider: SharedAccountUsageProvider,
        context: AccountUsageRefreshContext,
    ) -> AccountUsageResult {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(provider.refresh(context));
        });
        rx.recv_timeout(self.refresh_timeout).unwrap_or_else(|_| {
            Err(AccountUsageError::new(
                AccountUsageStatus::Network,
                "账号用量刷新超时",
            ))
        })
    }

    fn handle_refresh_error(
        &self,
        provider_id: &str,
        error: AccountUsageError,
    ) -> Result<Vec<AccountUsageSnapshot>, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        if error.retry_after_secs.is_some() {
            self.update_retry_after(&conn, provider_id, error.retry_after_secs)
                .map_err(|error| error.to_string())?;
        }
        crate::account_usage::store::mark_provider_snapshots_stale(&conn, provider_id, &error)
            .map_err(|error| error.to_string())?;
        let snapshots =
            crate::account_usage::store::latest_snapshots_by_provider(&conn, provider_id)
                .map_err(|error| error.to_string())?;
        if snapshots.is_empty() {
            Err(redact_secret_text(&error.message))
        } else {
            Ok(snapshots)
        }
    }

    fn update_last_refresh(
        &self,
        conn: &Connection,
        provider_id: &str,
    ) -> Result<(), rusqlite::Error> {
        if let Some(mut state) = crate::account_usage::store::get_provider_state(conn, provider_id)?
        {
            state.last_refresh_at = Some(now_rfc3339());
            state.retry_after_until = None;
            crate::account_usage::store::upsert_provider_state(conn, &state)?;
        }
        Ok(())
    }

    fn update_retry_after(
        &self,
        conn: &Connection,
        provider_id: &str,
        retry_after_secs: Option<u64>,
    ) -> Result<(), rusqlite::Error> {
        let Some(secs) = retry_after_secs else {
            return Ok(());
        };
        if let Some(mut state) = crate::account_usage::store::get_provider_state(conn, provider_id)?
        {
            let retry_at = chrono::Utc::now() + chrono::Duration::seconds(secs as i64);
            state.retry_after_until = Some(retry_at.to_rfc3339());
            crate::account_usage::store::upsert_provider_state(conn, &state)?;
        }
        Ok(())
    }

    fn rate_limit_backoff_snapshots(
        &self,
        provider_id: &str,
    ) -> Result<Option<Vec<AccountUsageSnapshot>>, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let Some(state) = crate::account_usage::store::get_provider_state(&conn, provider_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let Some(retry_after_until) = state.retry_after_until else {
            return Ok(None);
        };
        let Ok(retry_at) = chrono::DateTime::parse_from_rfc3339(&retry_after_until) else {
            return Ok(None);
        };
        if retry_at.with_timezone(&chrono::Utc) <= chrono::Utc::now() {
            return Ok(None);
        }

        let snapshots =
            crate::account_usage::store::latest_snapshots_by_provider(&conn, provider_id)
                .map_err(|error| error.to_string())?;
        if snapshots.is_empty() {
            return Err(format!("Provider 正在限流退避中: {provider_id}"));
        }
        Ok(Some(snapshots))
    }

    fn persist_credential_metadata(
        &self,
        metadata: CredentialMetadata,
    ) -> Result<AccountUsageProviderState, String> {
        let conn = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let provider = self.provider_by_id(&metadata.provider_id)?;
        let mut state = self.state_or_default(
            &conn,
            provider.id(),
            provider.default_refresh_interval_secs(),
        )?;
        state.credential_ref = Some(metadata.credential_ref);
        state.credential_label = metadata.label;
        crate::account_usage::store::upsert_provider_state(&conn, &state)
            .map_err(|error| error.to_string())?;
        Ok(state)
    }
}

struct RefreshGuard<'a> {
    provider_id: String,
    locks: &'a Mutex<HashSet<String>>,
}

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut locks) = self.locks.lock() {
            locks.remove(&self.provider_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_usage::{
        AccountUsageConfidence, AccountUsageMetric, AccountUsageMetricScope, AccountUsageSource,
    };

    fn setup_manager() -> (tempfile::TempDir, AccountUsageManager) {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("account_usage.sqlite");
        crate::db::init_db(&db_path).unwrap();
        let manager = AccountUsageManager::new(db_path);
        manager.initialize_provider_states().unwrap();
        (temp, manager)
    }

    fn sample_snapshot(provider_id: &str) -> AccountUsageSnapshot {
        AccountUsageSnapshot {
            provider_id: provider_id.to_string(),
            account_key: "account-1".to_string(),
            account_label: Some("user@example.com".to_string()),
            plan: Some("Pro".to_string()),
            status: AccountUsageStatus::Ok,
            source: AccountUsageSource::OfficialApi,
            confidence: AccountUsageConfidence::High,
            observed_at: now_rfc3339(),
            period_start: None,
            period_end: None,
            reset_at: None,
            stale: false,
            error: None,
            metrics: vec![AccountUsageMetric {
                metric_key: "quota".to_string(),
                label: "Quota".to_string(),
                unit: "percent".to_string(),
                scope: AccountUsageMetricScope::Personal,
                used: Some(10.0),
                limit: Some(100.0),
                remaining: Some(90.0),
                percentage: Some(10.0),
                reset_at: None,
            }],
        }
    }

    #[test]
    fn test_refresh_guard_deduplicates_concurrent_provider() {
        let (_temp, manager) = setup_manager();
        let guard = manager.try_refresh_guard("codex").unwrap();

        assert!(manager.try_refresh_guard("codex").is_err());
        drop(guard);
        assert!(manager.try_refresh_guard("codex").is_ok());
    }

    #[test]
    fn test_refresh_provider_reuses_cache_when_refresh_is_running() {
        let (_temp, manager) = setup_manager();
        let conn = Connection::open(&manager.db_path).unwrap();
        crate::account_usage::store::upsert_snapshot(&conn, &sample_snapshot("codex")).unwrap();
        drop(conn);
        let _guard = manager.try_refresh_guard("codex").unwrap();

        let snapshots = manager.refresh_provider("codex").unwrap();

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].provider_id, "codex");
    }

    #[test]
    fn test_enabled_refresh_interval_uses_shortest_enabled_provider() {
        let (_temp, manager) = setup_manager();
        assert_eq!(manager.enabled_refresh_interval_secs().unwrap(), None);

        manager
            .set_provider_enabled("codex".to_string(), true, Some(300))
            .unwrap();
        manager
            .set_provider_enabled("claude-code".to_string(), true, Some(60))
            .unwrap();
        manager
            .set_provider_enabled("github-copilot".to_string(), false, Some(10))
            .unwrap();

        assert_eq!(manager.enabled_refresh_interval_secs().unwrap(), Some(60));
    }

    #[test]
    fn test_rate_limit_backoff_preserves_stale_snapshot() {
        let (_temp, manager) = setup_manager();
        let conn = Connection::open(&manager.db_path).unwrap();
        crate::account_usage::store::upsert_snapshot(&conn, &sample_snapshot("codex")).unwrap();
        drop(conn);

        let snapshots = manager
            .handle_refresh_error(
                "codex",
                AccountUsageError {
                    code: AccountUsageStatus::RateLimited,
                    message: "限流".to_string(),
                    retry_after_secs: Some(60),
                },
            )
            .unwrap();
        assert!(snapshots[0].stale);

        let backoff = manager
            .rate_limit_backoff_snapshots("codex")
            .unwrap()
            .unwrap();
        assert_eq!(backoff.len(), 1);
        let state = manager.provider_state("codex".to_string()).unwrap();
        assert!(state.retry_after_until.is_some());
    }

    #[test]
    fn test_refresh_all_continues_after_provider_error() {
        let (_temp, manager) = setup_manager();
        manager
            .set_provider_enabled("codex".to_string(), true, None)
            .unwrap();
        manager
            .set_provider_enabled("claude-code".to_string(), true, None)
            .unwrap();

        let snapshots = manager.refresh_all_enabled().unwrap();

        assert!(snapshots
            .iter()
            .any(|snapshot| snapshot.provider_id == "claude-code"));
    }
}
