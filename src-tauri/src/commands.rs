use std::path::PathBuf;
use std::sync::Mutex;

use tauri::State;

use crate::adapters;
use crate::db;
use crate::types::{AgentInfo, AppSettings, PricingTable, TokenSummary};
use crate::watcher;

/// Tauri 共享状态
pub struct AppState {
    pub db_path: String,
    pub pricing: PricingTable,
    pub watcher: Mutex<Option<watcher::WatcherEngine>>,
    pub write_tx: Mutex<std::sync::mpsc::Sender<db::WriteRequest>>,
}

fn db_path_from(state: &AppState) -> PathBuf {
    PathBuf::from(&state.db_path)
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

    let enabled_str = db::queries::get_setting(&conn, "enabled_agents").unwrap_or(None);
    let enabled_agents: Vec<String> = match enabled_str {
        Some(s) => serde_json::from_str(&s).unwrap_or(defaults.enabled_agents),
        None => defaults.enabled_agents,
    };
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

    *watcher_guard = Some(new_watcher);
}

#[tauri::command]
pub fn get_token_summary(range: String, state: State<AppState>) -> Result<TokenSummary, String> {
    let conn = db::open_readonly(&db_path_from(&state)).map_err(|e| e.to_string())?;
    let summary = db::queries::get_token_summary(&conn, &range).map_err(|e| e.to_string())?;
    Ok(summary)
}

#[tauri::command]
pub fn get_agent_list(state: State<AppState>) -> Result<Vec<AgentInfo>, String> {
    let conn = db::open_readonly(&db_path_from(&state)).map_err(|e| e.to_string())?;
    let defaults = AppSettings::default();
    let enabled_str = db::queries::get_setting(&conn, "enabled_agents").unwrap_or(None);
    let enabled: Vec<String> = match enabled_str {
        Some(s) => serde_json::from_str(&s).unwrap_or(defaults.enabled_agents),
        None => defaults.enabled_agents,
    };

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
    state: State<AppState>,
) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&db_path_from(&state)).map_err(|e| e.to_string())?;
    let defaults = AppSettings::default();
    let current_str = db::queries::get_setting(&conn, "enabled_agents").unwrap_or(None);
    let mut current: Vec<String> = match current_str {
        Some(s) => serde_json::from_str(&s).unwrap_or(defaults.enabled_agents),
        None => defaults.enabled_agents,
    };

    if enabled {
        if !current.contains(&agent_name) {
            current.push(agent_name);
        }
    } else {
        current.retain(|a| a != &agent_name);
    }

    let json = serde_json::to_string(&current).map_err(|e| e.to_string())?;
    db::queries::set_setting(&conn, "enabled_agents", &json).map_err(|e| e.to_string())?;
    drop(conn);

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

    Ok(AppSettings {
        enabled_agents,
        watch_mode,
        keep_days,
        polling_interval_secs,
        language,
    })
}

#[tauri::command]
pub fn update_settings(key: String, value: String, state: State<AppState>) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&db_path_from(&state)).map_err(|e| e.to_string())?;
    db::queries::set_setting(&conn, &key, &value).map_err(|e| e.to_string())?;
    drop(conn);

    // watch_mode / polling_interval / enabled_agents 变更后重启 Watcher
    if matches!(
        key.as_str(),
        "watch_mode" | "polling_interval_secs" | "enabled_agents"
    ) {
        restart_watcher(&state);
    }
    Ok(())
}

#[tauri::command]
pub fn get_pricing(state: State<AppState>) -> Result<PricingTable, String> {
    Ok(state.pricing.clone())
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
}
