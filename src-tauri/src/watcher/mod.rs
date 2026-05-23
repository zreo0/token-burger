pub mod notify_strategy;
pub mod offset;
pub mod polling_strategy;
pub mod sqlite_strategy;

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::adapters::{AgentAdapter, DataSource};
use crate::db::WriteRequest;
use crate::types::ColdStartProgress;

fn sqlite_offset_key(db_path: &std::path::Path) -> String {
    format!("sqlite:{}", db_path.to_string_lossy())
}

fn insert_logs_and_update_offset(
    write_tx: &Sender<WriteRequest>,
    logs: Vec<crate::adapters::TokenLog>,
    file_path: String,
    offset: u64,
) -> bool {
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    if let Err(e) = write_tx.send(WriteRequest::InsertTokenLogsAndUpdateOffset {
        logs,
        file_path,
        offset,
        result_tx,
    }) {
        log::warn!("发送日志与 offset 原子写入请求失败: {}", e);
        return false;
    }

    match result_rx.recv() {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            log::warn!("日志与 offset 原子写入失败，将在下轮重试: {}", e);
            false
        }
        Err(e) => {
            log::warn!("等待日志与 offset 原子写入结果失败: {}", e);
            false
        }
    }
}

fn mark_cold_start_complete(cold_start_complete: &Arc<AtomicBool>) {
    cold_start_complete.store(true, Ordering::Release);
}

/// Watcher 配置
pub struct WatcherConfig {
    pub watch_mode: String,
    pub polling_interval_secs: u32,
    pub keep_days: u32,
}

/// Watcher 引擎：管理所有监听策略
pub struct WatcherEngine {
    stop_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl WatcherEngine {
    /// 启动 Watcher 引擎（冷启动 + 正常监听）
    pub fn start(
        adapters: Vec<Box<dyn AgentAdapter>>,
        write_tx: Sender<WriteRequest>,
        app_handle: AppHandle,
        config: WatcherConfig,
        db_path: PathBuf,
        cold_start_complete: Arc<AtomicBool>,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = stop_flag.clone();

        let handle = thread::spawn(move || {
            // 阶段一：冷启动——解析历史数据
            cold_start_complete.store(false, Ordering::Release);
            let mut known_offsets = offset::load_offsets_from_db(&db_path);
            let total = adapters.len() as u32;
            log::info!(target: "token_burger::watcher", "冷启动开始: {} 个 adapter, 已知 {} 个文件 offset", total, known_offsets.len());
            for (idx, adapter) in adapters.iter().enumerate() {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                let updated_offsets = cold_start_adapter(
                    adapter.as_ref(),
                    &write_tx,
                    config.keep_days,
                    &known_offsets,
                );
                for (key, offset) in updated_offsets {
                    known_offsets.insert(key, offset);
                }

                let _ = app_handle.emit(
                    "cold-start-progress",
                    ColdStartProgress {
                        agent: adapter.agent_name().to_string(),
                        done: true,
                        total,
                        completed: (idx + 1) as u32,
                    },
                );
            }

            mark_cold_start_complete(&cold_start_complete);
            match crate::db::open_readonly(&db_path) {
                Ok(conn) => crate::db::query_and_emit_today_summary(&app_handle, &conn),
                Err(e) => log::error!("冷启动完成后刷新汇总失败: {}", e),
            }
            log::info!(target: "token_burger::watcher", "冷启动完成");

            // 阶段二：正常监听模式
            start_watchers(
                &adapters,
                &write_tx,
                &flag,
                &config,
                &db_path,
                &known_offsets,
            );
        });

        WatcherEngine {
            stop_flag,
            handle: Some(handle),
        }
    }

    /// 仅启动监听（跳过冷启动，用于设置变更后重启）
    pub fn start_monitoring(
        adapters: Vec<Box<dyn AgentAdapter>>,
        write_tx: Sender<WriteRequest>,
        config: WatcherConfig,
        db_path: PathBuf,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = stop_flag.clone();

        let handle = thread::spawn(move || {
            let known_offsets = offset::load_offsets_from_db(&db_path);
            start_watchers(
                &adapters,
                &write_tx,
                &flag,
                &config,
                &db_path,
                &known_offsets,
            );
        });

        WatcherEngine {
            stop_flag,
            handle: Some(handle),
        }
    }

    /// 设置停止标志并等待主调度线程退出
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn catch_up_adapters(
    adapters: Vec<Box<dyn AgentAdapter>>,
    write_tx: Sender<WriteRequest>,
    keep_days: u32,
    db_path: PathBuf,
) {
    thread::spawn(move || {
        let mut known_offsets = offset::load_offsets_from_db(&db_path);
        for adapter in adapters {
            let updated_offsets =
                cold_start_adapter(adapter.as_ref(), &write_tx, keep_days, &known_offsets);
            for (key, offset) in updated_offsets {
                known_offsets.insert(key, offset);
            }
        }
    });
}

/// 冷启动：扫描单个 Adapter 的历史文件
fn cold_start_adapter(
    adapter: &dyn AgentAdapter,
    write_tx: &Sender<WriteRequest>,
    keep_days: u32,
    known_offsets: &std::collections::HashMap<String, u64>,
) -> Vec<(String, u64)> {
    let mut updated_offsets = Vec::new();
    let cutoff = chrono::Local::now() - chrono::Duration::days(keep_days as i64);
    let cutoff_ts = cutoff.timestamp();
    let agent = adapter.agent_name();

    match adapter.data_source() {
        DataSource::Jsonl { paths } | DataSource::Json { paths } => {
            let mut total_files = 0u32;
            let mut total_records = 0u32;
            let mut skipped_old = 0u32;
            let mut skipped_known = 0u32;

            for base_path in &paths {
                for pattern in &adapter.log_paths() {
                    let entries = match glob::glob(pattern) {
                        Ok(e) => e,
                        Err(e) => {
                            log::warn!("[冷启动] {}: glob 模式错误 {}: {}", agent, pattern, e);
                            continue;
                        }
                    };
                    for entry in entries.flatten() {
                        // mtime 过滤
                        if let Ok(meta) = std::fs::metadata(&entry) {
                            if let Ok(mtime) = meta.modified() {
                                let mtime_ts = mtime
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                                    as i64;
                                if mtime_ts < cutoff_ts {
                                    skipped_old += 1;
                                    continue;
                                }
                            }
                            // offset 过滤：文件大小未变则跳过
                            let path_str = entry.to_string_lossy().to_string();
                            if let Some(&prev_offset) = known_offsets.get(&path_str) {
                                if meta.len() <= prev_offset {
                                    skipped_known += 1;
                                    continue;
                                }
                            }
                        }
                        if let Ok(content) = std::fs::read_to_string(&entry) {
                            let logs = adapter.parse_content(&content);
                            let count = logs.len() as u32;
                            total_files += 1;
                            total_records += count;
                            if !logs.is_empty() {
                                let _ = write_tx.send(WriteRequest::InsertTokenLogs(logs));
                            }

                            // 冷启动完成后立即落盘 offset，避免下次启动重复扫描。
                            if let Ok(meta) = std::fs::metadata(&entry) {
                                let path_str = entry.to_string_lossy().to_string();
                                let offset = meta.len();
                                let _ = write_tx.send(WriteRequest::UpdateOffset {
                                    file_path: path_str.clone(),
                                    offset,
                                });
                                updated_offsets.push((path_str, offset));
                            }
                        }
                    }
                }
                let _ = base_path;
            }
            log::info!(
                "[冷启动] {} 完成: 扫描 {} 个文件, 解析 {} 条记录, 跳过 {} 个过期文件, 跳过 {} 个已处理文件",
                agent,
                total_files,
                total_records,
                skipped_old,
                skipped_known
            );
        }
        DataSource::Sqlite { db_path } => {
            if db_path.exists() {
                let offset_key = sqlite_offset_key(&db_path);
                let since = known_offsets.get(&offset_key).copied();
                log::info!(
                    "[冷启动] {}: 查询外部 SQLite {} since={:?}",
                    agent,
                    db_path.display(),
                    since
                );
                match adapter.query_db(&db_path, since) {
                    Ok((logs, high_watermark)) => {
                        log::info!("[冷启动] {} 完成: {} 条记录", agent, logs.len());
                        let has_logs = !logs.is_empty();
                        match high_watermark {
                            Some(offset) if has_logs => {
                                if !insert_logs_and_update_offset(
                                    write_tx,
                                    logs,
                                    offset_key.clone(),
                                    offset,
                                ) {
                                    return updated_offsets;
                                }
                                updated_offsets.push((offset_key, offset));
                            }
                            Some(offset) => {
                                let _ = write_tx.send(WriteRequest::UpdateOffset {
                                    file_path: offset_key.clone(),
                                    offset,
                                });
                                updated_offsets.push((offset_key, offset));
                            }
                            None if has_logs => {
                                let _ = write_tx.send(WriteRequest::InsertTokenLogs(logs));
                            }
                            None => {}
                        }
                    }
                    Err(e) => {
                        log::warn!("[冷启动] {}: SQLite 查询失败: {}", agent, e);
                    }
                }
            } else {
                log::info!("[冷启动] {}: 数据库不存在, 跳过", agent);
            }
        }
    }
    updated_offsets
}

/// 启动持续监听（各策略，根据 watch_mode 决定）
fn start_watchers(
    adapters: &[Box<dyn AgentAdapter>],
    write_tx: &Sender<WriteRequest>,
    stop_flag: &Arc<AtomicBool>,
    config: &WatcherConfig,
    _db_path: &std::path::Path,
    known_offsets: &std::collections::HashMap<String, u64>,
) {
    // 分组
    let mut jsonl_adapters: Vec<&dyn AgentAdapter> = Vec::new();
    let mut json_adapters: Vec<&dyn AgentAdapter> = Vec::new();
    let mut sqlite_adapters: Vec<(&dyn AgentAdapter, std::path::PathBuf)> = Vec::new();

    for adapter in adapters {
        match adapter.data_source() {
            DataSource::Jsonl { .. } => jsonl_adapters.push(adapter.as_ref()),
            DataSource::Json { .. } => json_adapters.push(adapter.as_ref()),
            DataSource::Sqlite { db_path } => {
                sqlite_adapters.push((adapter.as_ref(), db_path));
            }
        }
    }

    let is_realtime = config.watch_mode == "realtime";
    let poll_secs = config.polling_interval_secs;

    // JSONL 文件：realtime 用 notify，polling 用定时轮询
    if !jsonl_adapters.is_empty() {
        let tx = write_tx.clone();
        let flag = stop_flag.clone();
        let adapter_names: Vec<String> = jsonl_adapters
            .iter()
            .map(|a| a.agent_name().to_string())
            .collect();
        let log_patterns: Vec<Vec<String>> = jsonl_adapters.iter().map(|a| a.log_paths()).collect();

        if is_realtime {
            let initial_offsets = known_offsets.clone();
            thread::spawn(move || {
                notify_strategy::run_notify_polling(
                    adapter_names,
                    log_patterns,
                    tx,
                    flag,
                    initial_offsets,
                );
            });
        } else {
            thread::spawn(move || {
                polling_strategy::run_polling(adapter_names, log_patterns, tx, flag, poll_secs);
            });
        }
    }

    // JSON 文件：始终用 polling
    if !json_adapters.is_empty() {
        let tx = write_tx.clone();
        let flag = stop_flag.clone();
        let adapter_names: Vec<String> = json_adapters
            .iter()
            .map(|a| a.agent_name().to_string())
            .collect();
        let log_patterns: Vec<Vec<String>> = json_adapters.iter().map(|a| a.log_paths()).collect();

        thread::spawn(move || {
            polling_strategy::run_polling(adapter_names, log_patterns, tx, flag, poll_secs);
        });
    }

    // SQLite 策略
    for (adapter, adapter_db_path) in &sqlite_adapters {
        if adapter_db_path.exists() {
            let tx = write_tx.clone();
            let flag = stop_flag.clone();
            let dp = adapter_db_path.clone();
            let name = adapter.agent_name().to_string();
            let initial_offset = known_offsets
                .get(&sqlite_offset_key(adapter_db_path))
                .copied();

            thread::spawn(move || {
                sqlite_strategy::run_sqlite_polling(name, dp, tx, flag, poll_secs, initial_offset);
            });
        }
    }

    // 主线程等待停止信号
    while !stop_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_stop_flag_basic() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(!flag.load(Ordering::Relaxed));
        flag.store(true, Ordering::Relaxed);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn test_mark_cold_start_complete() {
        let complete = Arc::new(AtomicBool::new(false));

        assert!(!complete.load(Ordering::Acquire));
        mark_cold_start_complete(&complete);
        assert!(complete.load(Ordering::Acquire));
    }

    // --- offset 断点续传 ---

    #[test]
    fn test_read_from_offset_full_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();

        let content = notify_strategy::read_from_offset(&path, 0).unwrap();
        assert_eq!(content, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_read_from_offset_incremental() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "line1\nline2\n").unwrap();

        // 模拟首次读到 offset 6（"line1\n" 长度）
        let content = notify_strategy::read_from_offset(&path, 6).unwrap();
        assert_eq!(content, "line2\n");
    }

    #[test]
    fn test_read_from_offset_at_eof() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let data = "line1\n";
        std::fs::write(&path, data).unwrap();

        let content = notify_strategy::read_from_offset(&path, data.len() as u64).unwrap();
        assert_eq!(content, "");
    }

    // --- 文件轮转检测 ---

    #[test]
    fn test_truncation_detection_logic() {
        // 模拟轮转：文件先大后小，offset 应重置
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        // 写入 20 字节
        std::fs::write(&path, "12345678901234567890").unwrap();
        let prev_offset: u64 = 20;

        // 文件被截断为 5 字节
        std::fs::write(&path, "ABCDE").unwrap();
        let new_size = std::fs::metadata(&path).unwrap().len();

        assert!(new_size < prev_offset, "截断后文件应小于旧 offset");

        // 从 0 重新读取
        let content = notify_strategy::read_from_offset(&path, 0).unwrap();
        assert_eq!(content, "ABCDE");
    }

    #[test]
    fn test_normal_growth_incremental_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        // 初始写入
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"line1\n").unwrap();
        file.flush().unwrap();
        let offset_after_first = 6u64;

        // 追加写入
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"line2\n").unwrap();
        file.flush().unwrap();

        let new_size = std::fs::metadata(&path).unwrap().len();
        assert!(new_size > offset_after_first);

        let content = notify_strategy::read_from_offset(&path, offset_after_first).unwrap();
        assert_eq!(content, "line2\n");
    }

    // --- DB offset 加载 ---

    #[test]
    fn test_load_offsets_from_db_empty() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();

        let offsets = offset::load_offsets_from_db(&db_path);
        assert!(offsets.is_empty());
    }

    #[test]
    fn test_load_offsets_from_db_with_data() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();

        crate::db::queries::update_offset(&conn, "/tmp/a.jsonl", 1024).unwrap();
        crate::db::queries::update_offset(&conn, "/tmp/b.jsonl", 2048).unwrap();
        drop(conn);

        let offsets = offset::load_offsets_from_db(&db_path);
        assert_eq!(offsets.get("/tmp/a.jsonl"), Some(&1024u64));
        assert_eq!(offsets.get("/tmp/b.jsonl"), Some(&2048u64));
    }

    #[test]
    fn test_load_offsets_from_nonexistent_db() {
        let offsets = offset::load_offsets_from_db(std::path::Path::new("/nonexistent/path.db"));
        assert!(offsets.is_empty());
    }

    // --- mtime 过滤 ---

    #[test]
    fn test_mtime_filter_recent_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.jsonl");
        std::fs::write(&path, "test").unwrap();

        let cutoff = chrono::Local::now() - chrono::Duration::days(365);
        let cutoff_ts = cutoff.timestamp();

        let mtime = std::fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // 刚创建的文件 mtime 应大于 365 天前
        assert!(mtime >= cutoff_ts, "新文件应通过 mtime 过滤");
    }
}
