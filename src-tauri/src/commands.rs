use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::{
    menu::{Menu, MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};

use crate::account_usage::manager::AccountUsageManager;
use crate::account_usage::{
    redact_account_usage_snapshots, AccountUsageProviderInfo, AccountUsageProviderState,
    AccountUsageSnapshot,
};
use crate::adapters;
use crate::db;
use crate::types::{AgentInfo, AppSettings, PlatformInfo, PricingTable, TokenSummary};
use crate::watcher;

const ACCOUNT_USAGE_IDLE_POLL_SECS: u64 = 30;
const ACCOUNT_USAGE_SLEEP_SLICE_MS: u64 = 500;

/// Tauri 共享状态
pub struct AppState {
    pub db_path: String,
    pub pricing: PricingTable,
    pub watcher: Mutex<Option<watcher::WatcherEngine>>,
    pub write_tx: Mutex<std::sync::mpsc::Sender<db::WriteRequest>>,
    pub account_usage: AccountUsageManager,
    pub(crate) account_usage_refresher: Mutex<Option<AccountUsageRefreshWorker>>,
    pub cold_start_complete: Arc<AtomicBool>,
}

/// 账号用量后台刷新线程
pub(crate) struct AccountUsageRefreshWorker {
    stop_flag: Arc<AtomicBool>,
    wake_tx: Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl AccountUsageRefreshWorker {
    /// 启动后台刷新线程，按启用 Provider 的最短间隔刷新
    fn start(app: AppHandle) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (wake_tx, wake_rx) = mpsc::channel();
        let thread_stop_flag = stop_flag.clone();
        let handle = thread::spawn(move || loop {
            if thread_stop_flag.load(Ordering::Relaxed) {
                return;
            }

            let interval = match account_usage_refresh_interval(&app) {
                Some(interval) => interval,
                None => {
                    if wait_for_account_usage_signal(
                        &thread_stop_flag,
                        &wake_rx,
                        Duration::from_secs(ACCOUNT_USAGE_IDLE_POLL_SECS),
                    ) {
                        return;
                    }
                    continue;
                }
            };

            refresh_account_usage_from_background(&app);

            if wait_for_account_usage_signal(&thread_stop_flag, &wake_rx, interval) {
                return;
            }
        });

        Self {
            stop_flag,
            wake_tx,
            handle: Some(handle),
        }
    }

    /// 唤醒后台线程，使其重新读取 Provider 刷新间隔
    fn wake(&self) {
        let _ = self.wake_tx.send(());
    }
}

impl Drop for AccountUsageRefreshWorker {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.wake_tx.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct SaveAccountUsageCredentialRequest {
    pub provider_id: String,
    pub account_key: Option<String>,
    pub secret_kind: String,
    pub secret: String,
    pub label: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SetAccountUsageProviderEnabledRequest {
    pub provider_id: String,
    pub enabled: bool,
    pub refresh_interval_secs: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SetAccountUsageProviderMenuBarVisibleRequest {
    pub provider_id: String,
    pub show_in_menu_bar: bool,
}

pub fn tray_menu_labels(language: &str) -> (&'static str, &'static str) {
    if language == "zh-CN" {
        ("设置", "退出")
    } else {
        ("Settings", "Quit")
    }
}

pub fn main_tray_scanning_title(language: &str) -> &'static str {
    if language == "zh-CN" {
        "扫描中..."
    } else {
        "Scanning..."
    }
}

pub fn main_tray_token_title(language: &str, total: i64, cold_start_complete: bool) -> String {
    if cold_start_complete {
        format_token_count(total)
    } else {
        main_tray_scanning_title(language).to_string()
    }
}

pub(crate) fn is_cold_start_complete(app: &AppHandle) -> bool {
    app.try_state::<AppState>()
        .map(|state| state.cold_start_complete.load(Ordering::Acquire))
        .unwrap_or(true)
}

pub fn build_tray_menu(app: &AppHandle, language: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let (settings_label, quit_label) = tray_menu_labels(language);
    let settings_item = MenuItemBuilder::with_id("settings", settings_label).build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", quit_label).build(app)?;

    MenuBuilder::new(app)
        .item(&settings_item)
        .separator()
        .item(&quit_item)
        .build()
}

/// 启动账号用量后台刷新线程
pub(crate) fn start_account_usage_refresher(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut refresher = state.account_usage_refresher.lock().unwrap();
    if refresher.is_some() {
        return;
    }

    *refresher = Some(AccountUsageRefreshWorker::start(app.clone()));
}

/// 唤醒账号用量后台刷新线程，刷新 Provider 状态变更后的调度
fn wake_account_usage_refresher(state: &AppState) {
    if let Ok(refresher) = state.account_usage_refresher.lock() {
        if let Some(refresher) = refresher.as_ref() {
            refresher.wake();
        }
    }
}

/// 等待后台刷新下一轮执行，等待期间可响应停止或唤醒信号
fn wait_for_account_usage_signal(
    stop_flag: &Arc<AtomicBool>,
    wake_rx: &Receiver<()>,
    duration: Duration,
) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop_flag.load(Ordering::Relaxed) {
            return true;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        match wake_rx
            .recv_timeout(remaining.min(Duration::from_millis(ACCOUNT_USAGE_SLEEP_SLICE_MS)))
        {
            Ok(()) => return false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return true,
        }
    }

    stop_flag.load(Ordering::Relaxed)
}

/// 读取当前启用 Provider 的最短刷新间隔
fn account_usage_refresh_interval(app: &AppHandle) -> Option<Duration> {
    let state = app.try_state::<AppState>()?;
    match state.account_usage.enabled_refresh_interval_secs() {
        Ok(Some(interval)) => Some(Duration::from_secs(interval)),
        Ok(None) => None,
        Err(error) => {
            log::warn!("读取账号用量后台刷新间隔失败: {}", error);
            None
        }
    }
}

/// 后台刷新账号用量，并广播给前端与菜单栏
fn refresh_account_usage_from_background(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    if let Err(error) = refresh_account_usage_and_emit(app, &state) {
        log::warn!("账号用量后台刷新失败: {}", error);
    }
}

/// 执行账号用量刷新，统一处理事件广播和菜单栏同步
fn refresh_account_usage_and_emit(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<AccountUsageSnapshot>, String> {
    let snapshots = redact_account_usage_snapshots(state.account_usage.refresh_all_enabled()?);
    let _ = app.emit("account-usage-updated", &snapshots);
    sync_account_usage_tray_items(app);
    Ok(snapshots)
}

fn update_tray_menu_language(app: &AppHandle, language: &str) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id("main") {
        let menu = build_tray_menu(app, language).map_err(|e| e.to_string())?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn db_path_from(state: &AppState) -> PathBuf {
    PathBuf::from(&state.db_path)
}

pub(crate) fn sync_account_usage_tray_items(app: &AppHandle) {
    let state = app.state::<AppState>();
    let conn = match db::open_readonly(&db_path_from(&state)) {
        Ok(conn) => conn,
        Err(error) => {
            log::warn!("读取菜单栏账号用量状态失败: {}", error);
            return;
        }
    };
    let enabled_agents = db::queries::get_enabled_agents(&conn);
    if let Ok(summary) = db::queries::get_token_summary_for_agents(&conn, "today", &enabled_agents)
    {
        db::update_main_tray_title(app, &conn, summary.total);
    }
}

/// 重启 Watcher 引擎（根据当前数据库中的设置重新创建）
fn restart_watcher(state: &AppState) {
    let mut watcher_guard = state.watcher.lock().unwrap();
    if let Some(mut w) = watcher_guard.take() {
        w.stop(); // 阻塞等待旧线程退出
    }

    let db_path_buf = db_path_from(state);
    let conn = match db::open_readonly(&db_path_buf) {
        Ok(c) => c,
        Err(e) => {
            log::error!("重启 watcher 时无法读取数据库: {}", e);
            return;
        }
    };
    let defaults = AppSettings::default();
    let enabled_agents = db::queries::get_enabled_agents(&conn);
    let watch_mode = db::queries::get_setting(&conn, "watch_mode")
        .unwrap_or(None)
        .unwrap_or(defaults.watch_mode);
    let polling_interval_secs = db::queries::get_setting(&conn, "polling_interval_secs")
        .unwrap_or(None)
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.polling_interval_secs);
    let keep_days = db::queries::get_setting(&conn, "keep_days")
        .unwrap_or(None)
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.keep_days);
    drop(conn);

    let all = adapters::all_adapters();
    let active_adapters: Vec<Box<dyn adapters::AgentAdapter>> = all
        .into_iter()
        .filter(|a| enabled_agents.contains(&a.agent_name().to_string()))
        .collect();

    let config = watcher::WatcherConfig {
        watch_mode,
        polling_interval_secs,
        keep_days,
    };

    let write_tx = state.write_tx.lock().unwrap().clone();
    let new_watcher =
        watcher::WatcherEngine::start_monitoring(active_adapters, write_tx, config, db_path_buf);
    state.cold_start_complete.store(true, Ordering::Release);

    *watcher_guard = Some(new_watcher);
}

#[tauri::command]
pub fn get_token_summary(range: String, state: State<AppState>) -> Result<TokenSummary, String> {
    let conn = db::open_readonly(&db_path_from(&state)).map_err(|e| e.to_string())?;
    let enabled_agents = db::queries::get_enabled_agents(&conn);
    let summary = db::queries::get_token_summary_for_agents(&conn, &range, &enabled_agents)
        .map_err(|e| e.to_string())?;
    Ok(summary)
}

#[tauri::command]
pub fn get_agent_list(state: State<AppState>) -> Result<Vec<AgentInfo>, String> {
    let conn = db::open_readonly(&db_path_from(&state)).map_err(|e| e.to_string())?;
    let enabled = db::queries::get_enabled_agents(&conn);

    let all = adapters::all_adapters();
    let agents =
        all.iter()
            .map(|a| {
                let name = a.agent_name().to_string();
                // 检查数据源路径/目录/数据库是否存在（而非已有日志文件）
                let available = match a.data_source() {
                    adapters::DataSource::Jsonl { paths }
                    | adapters::DataSource::Json { paths } => paths.iter().any(|p| p.exists()),
                    adapters::DataSource::Sqlite { db_path } => db_path.exists(),
                };
                let source_type = match a.data_source() {
                    adapters::DataSource::Jsonl { .. } => "jsonl",
                    adapters::DataSource::Json { .. } => "json",
                    adapters::DataSource::Sqlite { .. } => "sqlite",
                }
                .to_string();
                AgentInfo {
                    enabled: enabled.contains(&name),
                    name,
                    available,
                    source_type,
                }
            })
            .collect();
    Ok(agents)
}

#[tauri::command]
pub fn toggle_agent(
    agent_name: String,
    enabled: bool,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let conn = rusqlite::Connection::open(db_path_from(&state)).map_err(|e| e.to_string())?;
    let defaults = AppSettings::default();
    let mut current = db::queries::get_enabled_agents(&conn);
    let keep_days = db::queries::get_setting(&conn, "keep_days")
        .unwrap_or(None)
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.keep_days);
    let mut changed = false;

    if enabled {
        if !current.contains(&agent_name) {
            current.push(agent_name.clone());
            changed = true;
        }
    } else {
        let before = current.len();
        current.retain(|a| a != &agent_name);
        changed = current.len() != before;
    }

    let json = serde_json::to_string(&current).map_err(|e| e.to_string())?;
    db::queries::set_setting(&conn, "enabled_agents", &json).map_err(|e| e.to_string())?;
    db::query_and_emit_today_summary(&app, &conn);
    drop(conn);

    if enabled && changed {
        let adapters: Vec<Box<dyn adapters::AgentAdapter>> = adapters::all_adapters()
            .into_iter()
            .filter(|a| a.agent_name() == agent_name)
            .collect();
        let write_tx = state.write_tx.lock().unwrap().clone();
        watcher::catch_up_adapters(adapters, write_tx, keep_days, db_path_from(&state));
    }

    // Agent 变更后重启 Watcher
    restart_watcher(&state);
    Ok(())
}

#[tauri::command]
pub fn clear_data(keep_days: Option<u32>, state: State<AppState>) -> Result<(), String> {
    // 通过写线程执行清理（写线程会在清理后广播新汇总并更新 tray）
    let tx = state.write_tx.lock().unwrap();
    tx.send(db::WriteRequest::ClearData(keep_days))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<AppSettings, String> {
    let conn = db::open_readonly(&db_path_from(&state)).map_err(|e| e.to_string())?;

    let defaults = AppSettings::default();

    let enabled_agents_str = db::queries::get_setting(&conn, "enabled_agents").unwrap_or(None);
    let enabled_agents = match enabled_agents_str {
        Some(s) => serde_json::from_str(&s).unwrap_or(defaults.enabled_agents),
        None => defaults.enabled_agents,
    };
    let watch_mode = db::queries::get_setting(&conn, "watch_mode")
        .unwrap_or(None)
        .unwrap_or(defaults.watch_mode);
    let keep_days = db::queries::get_setting(&conn, "keep_days")
        .unwrap_or(None)
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.keep_days);
    let polling_interval_secs = db::queries::get_setting(&conn, "polling_interval_secs")
        .unwrap_or(None)
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.polling_interval_secs);
    let language = db::queries::get_setting(&conn, "language")
        .unwrap_or(None)
        .unwrap_or(defaults.language);
    let color_theme = db::queries::get_setting(&conn, "color_theme")
        .unwrap_or(None)
        .unwrap_or(defaults.color_theme);

    Ok(AppSettings {
        enabled_agents,
        watch_mode,
        keep_days,
        polling_interval_secs,
        language,
        color_theme,
    })
}

#[tauri::command]
pub fn update_settings(
    key: String,
    value: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let conn = rusqlite::Connection::open(db_path_from(&state)).map_err(|e| e.to_string())?;
    db::queries::set_setting(&conn, &key, &value).map_err(|e| e.to_string())?;
    drop(conn);

    if key == "language" {
        update_tray_menu_language(&app, &value)?;
        sync_account_usage_tray_items(&app);
        let _ = app.emit("settings-language-changed", &value);
    }

    if key == "color_theme" {
        let _ = app.emit("settings-color-theme-changed", &value);
    }

    // watch_mode / polling_interval / enabled_agents 变更后重启 Watcher
    if matches!(
        key.as_str(),
        "watch_mode" | "polling_interval_secs" | "enabled_agents"
    ) {
        restart_watcher(&state);
    }

    if key == "enabled_agents" {
        let conn = db::open_readonly(&db_path_from(&state)).map_err(|e| e.to_string())?;
        db::query_and_emit_today_summary(&app, &conn);
    }
    Ok(())
}

#[tauri::command]
pub fn get_pricing(state: State<AppState>) -> Result<PricingTable, String> {
    Ok(state.pricing.clone())
}

fn current_platform_info() -> PlatformInfo {
    let platform = std::env::consts::OS.to_string();
    let display_name = match platform.as_str() {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
    .to_string();

    PlatformInfo {
        platform,
        display_name,
    }
}

#[tauri::command]
pub fn get_platform_info() -> Result<PlatformInfo, String> {
    Ok(current_platform_info())
}

#[tauri::command]
pub fn list_account_usage_providers(
    state: State<AppState>,
) -> Result<Vec<AccountUsageProviderInfo>, String> {
    state.account_usage.provider_infos()
}

#[tauri::command]
pub fn get_account_usage_snapshots(
    state: State<AppState>,
) -> Result<Vec<AccountUsageSnapshot>, String> {
    state
        .account_usage
        .latest_snapshots()
        .map(redact_account_usage_snapshots)
}

#[tauri::command]
pub fn refresh_account_usage_all(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<AccountUsageSnapshot>, String> {
    refresh_account_usage_and_emit(&app, &state)
}

#[tauri::command]
pub fn refresh_account_usage_provider(
    provider_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<AccountUsageSnapshot>, String> {
    let snapshots =
        redact_account_usage_snapshots(state.account_usage.refresh_provider(&provider_id)?);
    let _ = app.emit("account-usage-updated", &snapshots);
    sync_account_usage_tray_items(&app);
    Ok(snapshots)
}

#[tauri::command]
pub fn save_account_usage_credential(
    request: SaveAccountUsageCredentialRequest,
    app: AppHandle,
    state: State<AppState>,
) -> Result<AccountUsageProviderState, String> {
    let provider_id = request.provider_id.clone();
    let provider_state = state.account_usage.save_credential(
        request.provider_id,
        request.account_key,
        request.secret_kind,
        request.secret,
        request.label,
    )?;
    let snapshots = state
        .account_usage
        .refresh_provider(&provider_id)
        .map(redact_account_usage_snapshots)
        .unwrap_or_default();
    if let Ok(providers) = state.account_usage.provider_infos() {
        let _ = app.emit("account-usage-providers-updated", &providers);
    }
    let _ = app.emit("account-usage-updated", &snapshots);
    sync_account_usage_tray_items(&app);
    Ok(provider_state)
}

#[tauri::command]
pub fn clear_account_usage_credential(
    provider_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<AccountUsageProviderState, String> {
    let provider_state = state.account_usage.clear_credential(provider_id)?;
    let snapshots = state
        .account_usage
        .latest_snapshots()
        .map(redact_account_usage_snapshots)
        .unwrap_or_default();
    if let Ok(providers) = state.account_usage.provider_infos() {
        let _ = app.emit("account-usage-providers-updated", &providers);
    }
    let _ = app.emit("account-usage-updated", &snapshots);
    sync_account_usage_tray_items(&app);
    Ok(provider_state)
}

#[tauri::command]
pub fn get_account_usage_provider_state(
    provider_id: String,
    state: State<AppState>,
) -> Result<AccountUsageProviderState, String> {
    state.account_usage.provider_state(provider_id)
}

#[tauri::command]
pub fn set_account_usage_provider_enabled(
    request: SetAccountUsageProviderEnabledRequest,
    app: AppHandle,
    state: State<AppState>,
) -> Result<AccountUsageProviderState, String> {
    let provider_state = state.account_usage.set_provider_enabled(
        request.provider_id,
        request.enabled,
        request.refresh_interval_secs,
    )?;
    if let Ok(providers) = state.account_usage.provider_infos() {
        let _ = app.emit("account-usage-providers-updated", &providers);
    }
    sync_account_usage_tray_items(&app);
    wake_account_usage_refresher(&state);
    Ok(provider_state)
}

#[tauri::command]
pub fn set_account_usage_provider_menu_bar_visible(
    request: SetAccountUsageProviderMenuBarVisibleRequest,
    app: AppHandle,
    state: State<AppState>,
) -> Result<AccountUsageProviderState, String> {
    let provider_state = state
        .account_usage
        .set_provider_menu_bar_visible(request.provider_id, request.show_in_menu_bar)?;
    if let Ok(providers) = state.account_usage.provider_infos() {
        let _ = app.emit("account-usage-providers-updated", &providers);
    }
    sync_account_usage_tray_items(&app);
    Ok(provider_state)
}

/// 格式化 token 数量为可读字符串（用于 tray title）
pub fn format_token_count(total: i64) -> String {
    if total >= 1_000_000_000 {
        format!("{:.1}B", total as f64 / 1_000_000_000.0)
    } else if total >= 1_000_000 {
        format!("{:.1}M", total as f64 / 1_000_000.0)
    } else if total >= 1_000 {
        format!("{:.1}K", total as f64 / 1_000.0)
    } else {
        total.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_token_count() {
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(1500), "1.5K");
        assert_eq!(format_token_count(1_500_000), "1.5M");
        assert_eq!(format_token_count(1_500_000_000), "1.5B");
        assert_eq!(format_token_count(0), "0");
    }

    #[test]
    fn test_main_tray_scanning_title_uses_language() {
        assert_eq!(main_tray_scanning_title("en"), "Scanning...");
        assert_eq!(main_tray_scanning_title("zh-CN"), "扫描中...");
        assert_eq!(main_tray_scanning_title("fr"), "Scanning...");
    }

    #[test]
    fn test_main_tray_token_title_respects_cold_start_state() {
        assert_eq!(main_tray_token_title("en", 45678, false), "Scanning...");
        assert_eq!(main_tray_token_title("zh-CN", 45678, false), "扫描中...");
        assert_eq!(main_tray_token_title("en", 45678, true), "45.7K");
    }
}
