pub mod queries;

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use rusqlite::{Connection, OpenFlags};
use tauri::{AppHandle, Emitter, Manager};

use crate::adapters::TokenLog;

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

/// 创建只读连接
pub fn open_readonly(db_path: &PathBuf) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(conn)
}

/// 写请求类型
pub enum WriteRequest {
    /// 批量插入 token logs
    InsertTokenLogs(Vec<TokenLog>),
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

        let writer_db_path = db_path.clone();
        thread::spawn(move || {
            let conn = match init_db(&writer_db_path) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("写线程无法打开数据库: {}", e);
                    return;
                }
            };

            while let Ok(req) = write_rx.recv() {
                match req {
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
                            log::error!("批量插入失败: {}", e);
                            continue;
                        }
                        // 入库后查询今日汇总并广播
                        match queries::get_token_summary(&conn, "today") {
                            Ok(summary) => {
                                let total = summary.total;
                                log::info!(
                                    "[db] 今日汇总: total={}, input={}, output={}, cache_read={}, cache_create={}, agent_cost=${:.2}",
                                    total, summary.input, summary.output, summary.cache_read, summary.cache_create, summary.agent_cost
                                );
                                let _ = app_handle.emit("token-updated", &summary);
                                // 更新 tray title
                                if let Some(tray) = app_handle.tray_by_id("main") {
                                    let formatted = crate::commands::format_token_count(total);
                                    let _ = tray.set_title(Some(&formatted));
                                }
                            }
                            Err(e) => {
                                log::error!("查询汇总失败: {}", e);
                            }
                        }
                    }
                    WriteRequest::ClearData(keep_days) => {
                        if let Err(e) = queries::clear_data(&conn, keep_days) {
                            log::error!("清理数据失败: {}", e);
                            continue;
                        }
                        // 清理后查询今日汇总并广播（刷新 tray 和前端）
                        match queries::get_token_summary(&conn, "today") {
                            Ok(summary) => {
                                let total = summary.total;
                                let _ = app_handle.emit("token-updated", &summary);
                                if let Some(tray) = app_handle.tray_by_id("main") {
                                    let formatted = crate::commands::format_token_count(total);
                                    let _ = tray.set_title(Some(&formatted));
                                }
                            }
                            Err(e) => {
                                log::error!("清理后查询汇总失败: {}", e);
                            }
                        }
                    }
                    WriteRequest::UpdateOffset { file_path, offset } => {
                        if let Err(e) = queries::update_offset(&conn, &file_path, offset) {
                            log::error!("更新 offset 失败: {}", e);
                        }
                    }
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

        // 验证三张表存在
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
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
    }
}
