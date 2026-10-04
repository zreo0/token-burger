pub mod queries;

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use chrono::{Datelike, LocalResult, TimeZone};
use rusqlite::{Connection, OpenFlags};
use tauri::{AppHandle, Emitter, Manager};

use crate::adapters::{ExternalSqliteCursor, TokenLog};
use crate::types::TokenSummary;

const MIDNIGHT_REFRESH_GRACE_SECS: u32 = 1;

pub(crate) const SCHEMA_SQL: &str = "
PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS token_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_name TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    token_type TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    session_id TEXT,
    request_id TEXT,
    latency_ms INTEGER,
    is_error INTEGER DEFAULT 0,
    metadata TEXT,
    cost REAL,  -- Agent 自带的花费（美元），NULL 表示需要前端计算
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(request_id, token_type)
);

CREATE INDEX IF NOT EXISTS idx_query_main ON token_logs(timestamp, agent_name, model_id);
CREATE INDEX IF NOT EXISTS idx_session ON token_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_request_dedup ON token_logs(request_id);

CREATE TABLE IF NOT EXISTS file_offsets (
    file_path TEXT PRIMARY KEY,
    last_offset INTEGER NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS external_sqlite_cursors (
    source_key TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_time INTEGER NOT NULL DEFAULT 0,
    created_id TEXT NOT NULL DEFAULT '',
    updated_time INTEGER NOT NULL DEFAULT 0,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(source_key, session_id)
);
";

/// 获取数据库路径，dev/prod 条件编译隔离
pub fn get_db_path(app_handle: &AppHandle) -> PathBuf {
    let mut path = app_handle
        .path()
        .app_data_dir()
        .expect("无法获取应用数据目录");
    if !path.exists() {
        std::fs::create_dir_all(&path).expect("无法创建应用数据目录");
    }
    #[cfg(debug_assertions)]
    {
        path.push("tokenburger_dev.sqlite");
    }
    #[cfg(not(debug_assertions))]
    {
        path.push("tokenburger_prod.sqlite");
    }
    path
}

/// 初始化数据库（WAL + Schema）
pub fn init_db(db_path: &PathBuf) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch(SCHEMA_SQL)?;
    crate::account_usage::store::init_schema(&conn)?;
    ensure_token_logs_cost_column(&conn)?;
    Ok(conn)
}

fn ensure_token_logs_cost_column(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(token_logs)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_cost = false;

    for row in rows {
        if row? == "cost" {
            has_cost = true;
            break;
        }
    }

    if !has_cost {
        conn.execute("ALTER TABLE token_logs ADD COLUMN cost REAL", [])?;
    }

    Ok(())
}

fn duration_until_next_local_day(now: chrono::DateTime<chrono::Local>) -> std::time::Duration {
    let next_date = match now.date_naive().succ_opt() {
        Some(date) => date,
        None => return std::time::Duration::from_secs(60),
    };

    let target = match chrono::Local.with_ymd_and_hms(
        next_date.year(),
        next_date.month(),
        next_date.day(),
        0,
        0,
        MIDNIGHT_REFRESH_GRACE_SECS,
    ) {
        LocalResult::Single(time) => time,
        LocalResult::Ambiguous(earliest, _) => earliest,
        LocalResult::None => now + chrono::Duration::days(1),
    };

    target
        .signed_duration_since(now)
        .to_std()
        .unwrap_or_else(|_| std::time::Duration::from_secs(1))
}

/**
 * 在后台读取标题数据，再将托盘获取、更新和释放全部调度到主线程
 * 参数为应用、数据库连接和总量，无返回值
 */
pub(crate) fn update_main_tray_title(app_handle: &AppHandle, conn: &Connection, total: i64) {
    let language = queries::get_setting(conn, "language")
        .unwrap_or(None)
        .unwrap_or_else(|| crate::types::AppSettings::default().language);
    let token_title = crate::commands::main_tray_token_title(
        &language,
        total,
        crate::commands::is_cold_start_complete(app_handle),
    );
    let items = crate::tray_usage::account_usage_menu_bar_items(conn);
    let suffix = items
        .iter()
        .map(|item| item.usage_title())
        .collect::<Vec<_>>()
        .join(" ");
    let fallback_title = if suffix.is_empty() {
        token_title.clone()
    } else {
        format!("{token_title} {suffix}")
    };
    let app = app_handle.clone();
    // Tauri 2.10 的 TrayIcon 内部持有 Rc，连 tray_by_id 的克隆也不能在后台执行
    if let Err(error) = app_handle.run_on_main_thread(move || {
        let Some(tray) = app.tray_by_id("main") else {
            return;
        };
        #[cfg(target_os = "macos")]
        if crate::tray_usage::set_main_tray_usage_title(&tray, token_title, items).is_ok() {
            return;
        }
        let _ = tray.set_title(Some(&fallback_title));
    }) {
        log::warn!("更新托盘标题失败: {}", error);
    }
}

fn emit_token_summary(app_handle: &AppHandle, conn: &Connection, summary: &TokenSummary) {
    let _ = app_handle.emit("token-updated", summary);
    update_main_tray_title(app_handle, conn, summary.total);
}

pub(crate) fn query_and_emit_today_summary(app_handle: &AppHandle, conn: &Connection) {
    let enabled_agents = queries::get_enabled_agents(conn);
    match queries::get_token_summary_for_agents(conn, "today", &enabled_agents) {
        Ok(summary) => {
            emit_token_summary(app_handle, conn, &summary);
        }
        Err(e) => {
            log::error!("查询今日汇总失败: {}", e);
        }
    }
}

fn start_midnight_refresh_thread(db_path: PathBuf, app_handle: AppHandle) {
    thread::spawn(move || {
        let mut last_local_date = chrono::Local::now().date_naive();

        loop {
            let now = chrono::Local::now();
            let local_date = now.date_naive();

            if local_date != last_local_date {
                let conn = match open_readonly(&db_path) {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::error!("零点刷新无法打开数据库: {}", e);
                        let delay = duration_until_next_local_day(now);
                        thread::sleep(delay.min(std::time::Duration::from_secs(60)));
                        continue;
                    }
                };

                query_and_emit_today_summary(&app_handle, &conn);
                last_local_date = local_date;
            }

            let delay = duration_until_next_local_day(now);
            thread::sleep(delay.min(std::time::Duration::from_secs(60)));
        }
    });
}

/// 创建只读连接
pub fn open_readonly(db_path: &PathBuf) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(conn)
}

/// 写请求类型
pub enum WriteRequest {
    /** 确认此前排队的写入已处理，并返回期间的写入错误 */
    Flush(mpsc::Sender<Result<(), String>>),
    /// 批量插入 token logs
    InsertTokenLogs(Vec<TokenLog>),
    /** 同一事务内保存文件统计与断点，失败时不推进断点 */
    InsertFileLogs {
        logs: Vec<TokenLog>,
        file_path: String,
        offset: u64,
    },
    /**
     * 重算单个 Claude 文件并确认持久化，避免失败时提前推进解析版本
     */
    ReindexClaudeFile {
        logs: Vec<TokenLog>,
        file_path: String,
        offset: u64,
        result_tx: mpsc::Sender<Result<(), String>>,
    },
    /// 批量插入 token logs，并在同一事务内更新外部 SQLite cursor
    InsertTokenLogsAndUpdateSqliteCursors {
        logs: Vec<TokenLog>,
        cursors: Vec<ExternalSqliteCursor>,
        result_tx: mpsc::Sender<Result<(), String>>,
    },
    /// 清理数据（keep_days 为 None 表示清空全部）
    #[allow(dead_code)]
    ClearData(Option<u32>),
    /// 更新文件偏移量
    UpdateOffset { file_path: String, offset: u64 },
}

/// 数据库管理器，持有写通道和数据库路径
pub struct DbManager {
    pub write_tx: mpsc::Sender<WriteRequest>,
    #[allow(dead_code)]
    pub db_path: PathBuf,
}

impl DbManager {
    /// 启动专用写线程，返回 DbManager
    pub fn start(db_path: PathBuf, app_handle: AppHandle) -> Self {
        let (write_tx, write_rx) = mpsc::channel::<WriteRequest>();

        start_midnight_refresh_thread(db_path.clone(), app_handle.clone());

        let writer_db_path = db_path.clone();
        thread::spawn(move || {
            let conn = match init_db(&writer_db_path) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("写线程无法打开数据库: {}", e);
                    return;
                }
            };

            let refresh_interval = std::time::Duration::from_secs(2);
            let mut last_summary = std::time::Instant::now();
            let mut summary_dirty = false;
            let mut write_error: Option<String> = None;
            loop {
                let req = match write_rx.recv_timeout(refresh_interval) {
                    Ok(req) => req,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if summary_dirty {
                            query_and_emit_today_summary(&app_handle, &conn);
                            summary_dirty = false;
                            last_summary = std::time::Instant::now();
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                match req {
                    WriteRequest::Flush(result_tx) => {
                        let _ = result_tx.send(write_error.take().map_or(Ok(()), Err));
                    }
                    WriteRequest::ReindexClaudeFile {
                        logs,
                        file_path,
                        offset,
                        result_tx,
                    } => {
                        let result = queries::reindex_claude_file(&conn, &logs, &file_path, offset)
                            .map_err(|error| error.to_string());
                        match &result {
                            Ok(()) => summary_dirty = true,
                            Err(error) => write_error = Some(error.clone()),
                        }
                        let _ = result_tx.send(result);
                    }
                    WriteRequest::InsertFileLogs {
                        logs,
                        file_path,
                        offset,
                    } => {
                        match queries::batch_insert_token_logs_and_update_offset(
                            &conn, &logs, &file_path, offset,
                        ) {
                            Ok(()) => summary_dirty |= !logs.is_empty(),
                            Err(error) => {
                                write_error = Some(error.to_string());
                                log::error!(
                                    "文件统计写入失败，保留旧断点 {}: {}",
                                    file_path,
                                    error
                                );
                            }
                        }
                    }
                    WriteRequest::InsertTokenLogs(logs) => {
                        let count = logs.len();
                        let total_tokens: i64 = logs.iter().map(|l| l.token_count).sum();
                        let total_cost: f64 = logs.iter().filter_map(|l| l.cost).sum();
                        // 提取涉及的 agent 列表
                        let agents: Vec<&str> = logs
                            .iter()
                            .map(|l| l.agent_name.as_str())
                            .collect::<std::collections::HashSet<_>>()
                            .into_iter()
                            .collect();
                        log::info!(
                            "[db] 写入 {} 条记录, agents={:?}, {} tokens, agent_cost=${:.4}",
                            count,
                            agents,
                            total_tokens,
                            total_cost
                        );
                        if let Err(e) = queries::batch_insert_token_logs(&conn, &logs) {
                            write_error = Some(e.to_string());
                            log::error!("批量插入失败: {}", e);
                            continue;
                        }
                        summary_dirty = true;
                    }
                    WriteRequest::InsertTokenLogsAndUpdateSqliteCursors {
                        logs,
                        cursors,
                        result_tx,
                    } => {
                        let count = logs.len();
                        let total_tokens: i64 = logs.iter().map(|l| l.token_count).sum();
                        let total_cost: f64 = logs.iter().filter_map(|l| l.cost).sum();
                        let agents: Vec<&str> = logs
                            .iter()
                            .map(|l| l.agent_name.as_str())
                            .collect::<std::collections::HashSet<_>>()
                            .into_iter()
                            .collect();
                        log::info!(
                            "[db] 写入 {} 条记录并更新 {} 个 SQLite cursor, agents={:?}, {} tokens, agent_cost=${:.4}",
                            count,
                            cursors.len(),
                            agents,
                            total_tokens,
                            total_cost
                        );
                        if let Err(e) = queries::batch_insert_token_logs_and_update_sqlite_cursors(
                            &conn, &logs, &cursors,
                        ) {
                            let message = e.to_string();
                            write_error = Some(message.clone());
                            log::error!("批量插入并更新 SQLite cursor 失败: {}", message);
                            let _ = result_tx.send(Err(message));
                            continue;
                        }
                        let _ = result_tx.send(Ok(()));

                        summary_dirty |= !logs.is_empty();
                    }
                    WriteRequest::ClearData(keep_days) => {
                        if let Err(e) = queries::clear_data(&conn, keep_days) {
                            log::error!("清理数据失败: {}", e);
                            continue;
                        }
                        // 清理后查询今日汇总并广播（刷新 tray 和前端）
                        query_and_emit_today_summary(&app_handle, &conn);
                    }
                    WriteRequest::UpdateOffset { file_path, offset } => {
                        if let Err(e) = queries::update_offset(&conn, &file_path, offset) {
                            write_error = Some(e.to_string());
                            log::error!("更新 offset 失败: {}", e);
                        }
                    }
                }
                // 持续入库时最多每两秒汇总一次，空闲后也要发布最后一批变化
                if summary_dirty && last_summary.elapsed() >= refresh_interval {
                    query_and_emit_today_summary(&app_handle, &conn);
                    summary_dirty = false;
                    last_summary = std::time::Instant::now();
                }
            }
        });

        DbManager { write_tx, db_path }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_init() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();

        // 验证基础表存在
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_offsets", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM app_settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM external_sqlite_cursors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
    }

    #[test]
    fn test_midnight_refresh_delay_targets_next_local_day() {
        let now = chrono::Local::now();
        let delay = super::duration_until_next_local_day(now);

        assert!(delay >= std::time::Duration::from_secs(1));
        assert!(delay <= std::time::Duration::from_secs(25 * 60 * 60));
    }
}
