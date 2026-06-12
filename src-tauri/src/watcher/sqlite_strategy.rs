use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use rusqlite::OpenFlags;

use crate::adapters::{
    all_agents, AgentDataBatch, AgentPipeline, ExternalSqliteCursor, SqliteMessageRow,
};
use crate::db::{queries, WriteRequest};
use crate::watcher::BehaviorRuntime;

const CREATED_BATCH_LIMIT: usize = 500;
const UPDATED_BATCH_LIMIT: usize = 500;
const UPDATED_RECONCILE_SESSIONS_PER_POLL: usize = 5;
const SQLITE_CURSOR_OVERLAP_MS: i64 = 120_000;

/// 定时 SQLite 轮询策略（用于 OpenCode/MiMoCode 等外部 DB 适配器）
pub fn run_sqlite_polling(
    agent_name: String,
    db_path: PathBuf,
    local_db_path: PathBuf,
    write_tx: Sender<WriteRequest>,
    stop_flag: Arc<AtomicBool>,
    poll_interval_secs: u32,
    initial_offset: Option<u64>,
    behavior_runtime: Option<BehaviorRuntime>,
) {
    let agents = all_agents();
    let agent = match agents.iter().find(|a| a.agent_name() == agent_name) {
        Some(a) => a,
        None => return,
    };

    let source_key = super::sqlite_offset_key(&db_path);
    let mut reconcile_index = 0usize;
    let mut external_conn = None;
    let mut cursors = None;

    loop {
        // 可中断的等待
        let deadline = std::time::Instant::now() + Duration::from_secs(poll_interval_secs as u64);
        while std::time::Instant::now() < deadline {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        if !db_path.exists() {
            external_conn = None;
            cursors = None;
            continue;
        }

        if external_conn.is_none() {
            external_conn = match open_external_readonly(&db_path) {
                Ok(conn) => Some(conn),
                Err(e) => {
                    log::warn!("{}: SQLite 只读连接失败: {}", agent_name, e);
                    continue;
                }
            };
        }
        let conn = external_conn.as_ref().expect("external SQLite conn exists");

        if cursors.is_none() {
            cursors = match load_or_bootstrap_cursors(
                agent.as_ref(),
                conn,
                &local_db_path,
                &source_key,
                initial_offset,
                &write_tx,
            ) {
                Ok(cursors) => Some(cursors),
                Err(e) => {
                    log::warn!("{}: SQLite cursor 初始化失败: {}", agent_name, e);
                    continue;
                }
            };
        } else if let Err(e) =
            sync_known_sessions(agent.as_ref(), conn, &source_key, cursors.as_mut().unwrap())
        {
            log::warn!("{}: SQLite session 刷新失败: {}", agent_name, e);
            continue;
        }

        let cursors = match cursors.as_mut() {
            Some(cursors) => cursors,
            None => {
                continue;
            }
        };

        if let Err(e) = process_created_rows(
            agent.as_ref(),
            conn,
            &db_path,
            &source_key,
            &write_tx,
            cursors,
            behavior_runtime.as_ref(),
        ) {
            log::warn!("{}: SQLite created cursor 轮询出错: {}", agent_name, e);
            continue;
        }

        if let Err(e) = process_updated_rows(
            agent.as_ref(),
            conn,
            &db_path,
            &source_key,
            &write_tx,
            cursors,
            &mut reconcile_index,
        ) {
            log::warn!("{}: SQLite updated cursor 校准出错: {}", agent_name, e);
        }
    }
}

fn open_external_readonly(db_path: &Path) -> Result<rusqlite::Connection, rusqlite::Error> {
    let conn = rusqlite::Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(conn)
}

/// 冷启动处理外部 SQLite source，并通过 per-session cursor 完成历史追赶。
pub(crate) fn cold_start_sqlite_source(
    agent: &dyn AgentPipeline,
    db_path: &Path,
    local_db_path: &Path,
    write_tx: &Sender<WriteRequest>,
    old_global_watermark: Option<u64>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let source_key = super::sqlite_offset_key(db_path);
    let conn = open_external_readonly(db_path)?;
    let mut cursors = load_or_bootstrap_cursors(
        agent,
        &conn,
        local_db_path,
        &source_key,
        old_global_watermark,
        write_tx,
    )?;
    let mut total_rows = 0usize;

    loop {
        let processed = process_created_rows(
            agent,
            &conn,
            db_path,
            &source_key,
            write_tx,
            &mut cursors,
            None,
        )?;
        if processed == 0 {
            break;
        }
        total_rows += processed;
    }

    Ok(total_rows)
}

fn sync_known_sessions(
    agent: &dyn AgentPipeline,
    conn: &rusqlite::Connection,
    source_key: &str,
    cursors: &mut Vec<ExternalSqliteCursor>,
) -> Result<(), Box<dyn std::error::Error>> {
    let known = cursors
        .iter()
        .map(|cursor| cursor.session_id.clone())
        .collect::<BTreeSet<_>>();
    for session_id in agent.list_sqlite_session_ids(conn)? {
        if !known.contains(&session_id) {
            cursors.push(ExternalSqliteCursor::empty(source_key, &session_id));
        }
    }
    cursors.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(())
}

fn load_or_bootstrap_cursors(
    agent: &dyn AgentPipeline,
    conn: &rusqlite::Connection,
    local_db_path: &Path,
    source_key: &str,
    old_global_watermark: Option<u64>,
    write_tx: &Sender<WriteRequest>,
) -> Result<Vec<ExternalSqliteCursor>, Box<dyn std::error::Error>> {
    let existing = load_local_cursors(local_db_path, source_key)?;
    let session_ids = agent.list_sqlite_session_ids(conn)?;
    if !existing.is_empty() {
        return Ok(merge_with_known_sessions(source_key, existing, session_ids));
    }

    if let Some(watermark) = old_global_watermark {
        let cursors = build_bootstrap_cursors(agent, conn, source_key, &session_ids, watermark)?;
        if !cursors.is_empty() && !write_sqlite_cursors(write_tx, cursors.clone()) {
            return Err("写入 bootstrap SQLite cursor 失败".into());
        }
        return Ok(cursors);
    }

    Ok(session_ids
        .iter()
        .map(|session_id| ExternalSqliteCursor::empty(source_key, session_id))
        .collect())
}

fn load_local_cursors(
    local_db_path: &Path,
    source_key: &str,
) -> Result<Vec<ExternalSqliteCursor>, rusqlite::Error> {
    let conn = rusqlite::Connection::open_with_flags(
        local_db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    queries::get_external_sqlite_cursors(&conn, source_key)
}

fn merge_with_known_sessions(
    source_key: &str,
    existing: Vec<ExternalSqliteCursor>,
    session_ids: Vec<String>,
) -> Vec<ExternalSqliteCursor> {
    let mut cursors: HashMap<String, ExternalSqliteCursor> = existing
        .into_iter()
        .map(|cursor| (cursor.session_id.clone(), cursor))
        .collect();
    for session_id in session_ids {
        cursors
            .entry(session_id.clone())
            .or_insert_with(|| ExternalSqliteCursor::empty(source_key, &session_id));
    }

    let mut values = cursors.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    values
}

fn build_bootstrap_cursors(
    agent: &dyn AgentPipeline,
    conn: &rusqlite::Connection,
    source_key: &str,
    session_ids: &[String],
    old_global_watermark: u64,
) -> Result<Vec<ExternalSqliteCursor>, Box<dyn std::error::Error>> {
    let updated_time = old_global_watermark
        .min(i64::MAX as u64)
        .saturating_sub(SQLITE_CURSOR_OVERLAP_MS as u64) as i64;
    let mut cursors = Vec::with_capacity(session_ids.len());

    for session_id in session_ids {
        let created_cursor = agent.query_sqlite_created_cursor_before_watermark(
            conn,
            session_id,
            old_global_watermark,
        )?;
        let mut cursor = ExternalSqliteCursor::empty(source_key, session_id);
        if let Some(created_cursor) = created_cursor {
            cursor.created_time = created_cursor.time_created;
            cursor.created_id = created_cursor.id;
        }
        cursor.updated_time = updated_time;
        cursors.push(cursor);
    }

    Ok(cursors)
}

fn process_created_rows(
    agent: &dyn AgentPipeline,
    conn: &rusqlite::Connection,
    db_path: &Path,
    source_key: &str,
    write_tx: &Sender<WriteRequest>,
    cursors: &mut [ExternalSqliteCursor],
    behavior_runtime: Option<&BehaviorRuntime>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut processed = 0usize;

    for cursor in cursors {
        let row_batch = agent.query_sqlite_rows_by_created_cursor(
            conn,
            &cursor.session_id,
            cursor.created_time,
            &cursor.created_id,
            CREATED_BATCH_LIMIT,
        )?;
        if row_batch.rows.is_empty() {
            continue;
        }

        let next_cursor = advance_created_cursor(cursor, &row_batch.rows);
        let batch = AgentDataBatch::SqliteRows {
            agent_name: agent.agent_name().to_string(),
            source_key: source_key.to_string(),
            db_path: db_path.to_path_buf(),
            rows: row_batch.rows,
            previous_watermark: None,
            next_watermark: row_batch.high_watermark,
        };
        let logs = agent.extract_tokens(&batch).logs;
        let behavior_events = if behavior_runtime.is_some_and(BehaviorRuntime::is_enabled) {
            agent.extract_behavior(&batch)
        } else {
            Vec::new()
        };
        let row_count = match &batch {
            AgentDataBatch::SqliteRows { rows, .. } => rows.len(),
            _ => 0,
        };

        if !insert_logs_and_update_sqlite_cursors(write_tx, logs, vec![next_cursor.clone()]) {
            return Err("写入 created cursor 处理结果失败".into());
        }

        if let Some(runtime) = behavior_runtime {
            if runtime.is_enabled() {
                for event in behavior_events {
                    runtime.dispatcher.handle_event(event);
                }
            }
        }

        *cursor = next_cursor;
        processed += row_count;
    }

    Ok(processed)
}

fn process_updated_rows(
    agent: &dyn AgentPipeline,
    conn: &rusqlite::Connection,
    db_path: &Path,
    source_key: &str,
    write_tx: &Sender<WriteRequest>,
    cursors: &mut [ExternalSqliteCursor],
    reconcile_index: &mut usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    if cursors.is_empty() {
        return Ok(0);
    }

    let mut processed = 0usize;
    let session_count = cursors.len();
    let mut seen = BTreeSet::new();

    for _ in 0..UPDATED_RECONCILE_SESSIONS_PER_POLL.min(session_count) {
        let idx = *reconcile_index % session_count;
        *reconcile_index = (*reconcile_index + 1) % session_count;
        if !seen.insert(idx) {
            continue;
        }

        let cursor = &mut cursors[idx];
        let since_updated = cursor
            .updated_time
            .saturating_sub(SQLITE_CURSOR_OVERLAP_MS)
            .max(0);
        let row_batch = agent.query_sqlite_rows_by_updated_cursor(
            conn,
            &cursor.session_id,
            since_updated,
            UPDATED_BATCH_LIMIT,
        )?;
        let next_updated_time = next_updated_cursor_time(cursor.updated_time, &row_batch);
        let mut next_cursor = cursor.clone();
        next_cursor.updated_time = next_updated_time;

        if row_batch.rows.is_empty() {
            // 空校准只推进内存 cursor，避免每轮轮询都写入本地 DB
            // 真正读到 row 时再和 TokenLog 同事务持久化，重启后重复校准不会漏消息
            *cursor = next_cursor;
            continue;
        }

        let row_count = row_batch.rows.len();
        let batch = AgentDataBatch::SqliteRows {
            agent_name: agent.agent_name().to_string(),
            source_key: source_key.to_string(),
            db_path: db_path.to_path_buf(),
            rows: row_batch.rows,
            previous_watermark: None,
            next_watermark: row_batch.high_watermark,
        };
        let logs = agent.extract_tokens(&batch).logs;
        if !insert_logs_and_update_sqlite_cursors(write_tx, logs, vec![next_cursor.clone()]) {
            return Err("写入 updated cursor 校准结果失败".into());
        }

        *cursor = next_cursor;
        processed += row_count;
    }

    Ok(processed)
}

fn advance_created_cursor(
    cursor: &ExternalSqliteCursor,
    rows: &[SqliteMessageRow],
) -> ExternalSqliteCursor {
    let mut next = cursor.clone();
    for row in rows {
        if row.time_created > next.created_time
            || (row.time_created == next.created_time && row.id > next.created_id)
        {
            next.created_time = row.time_created;
            next.created_id = row.id.clone();
        }
        if row.watermark > next.updated_time {
            next.updated_time = row.watermark;
        }
    }
    next
}

fn next_updated_cursor_time(current: i64, row_batch: &crate::adapters::SqliteRowBatch) -> i64 {
    let max_seen = row_batch
        .high_watermark
        .map(|value| value.min(i64::MAX as u64) as i64)
        .unwrap_or(current);

    if row_batch.rows.len() < UPDATED_BATCH_LIMIT {
        current.max(max_seen).max(current_epoch_millis())
    } else {
        current.max(max_seen)
    }
}

fn current_epoch_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn write_sqlite_cursors(
    write_tx: &Sender<WriteRequest>,
    cursors: Vec<ExternalSqliteCursor>,
) -> bool {
    insert_logs_and_update_sqlite_cursors(write_tx, Vec::new(), cursors)
}

fn insert_logs_and_update_sqlite_cursors(
    write_tx: &Sender<WriteRequest>,
    logs: Vec<crate::adapters::TokenLog>,
    cursors: Vec<ExternalSqliteCursor>,
) -> bool {
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    if let Err(e) = write_tx.send(WriteRequest::InsertTokenLogsAndUpdateSqliteCursors {
        logs,
        cursors,
        result_tx,
    }) {
        log::warn!("发送日志与 SQLite cursor 原子写入请求失败: {}", e);
        return false;
    }

    match result_rx.recv() {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            log::warn!("日志与 SQLite cursor 原子写入失败，将在下轮重试: {}", e);
            false
        }
        Err(e) => {
            log::warn!("等待日志与 SQLite cursor 原子写入结果失败: {}", e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{
        AgentSource, BehaviorExtractor, DataSource, SqliteCreatedCursor, SqliteRowBatch,
        TokenExtraction, TokenExtractor, TokenLog, TokenType,
    };

    struct BootstrapAgent {
        sessions: Vec<String>,
        cursors: HashMap<String, SqliteCreatedCursor>,
    }

    impl AgentSource for BootstrapAgent {
        fn agent_name(&self) -> &str {
            "bootstrap"
        }

        fn data_source(&self) -> DataSource {
            DataSource::Sqlite {
                db_path: "/tmp/bootstrap.db".into(),
            }
        }

        fn log_paths(&self) -> Vec<String> {
            Vec::new()
        }

        fn list_sqlite_session_ids(
            &self,
            _conn: &rusqlite::Connection,
        ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
            Ok(self.sessions.clone())
        }

        fn query_sqlite_created_cursor_before_watermark(
            &self,
            _conn: &rusqlite::Connection,
            session_id: &str,
            _watermark: u64,
        ) -> Result<Option<SqliteCreatedCursor>, Box<dyn std::error::Error>> {
            Ok(self.cursors.get(session_id).cloned())
        }
    }

    impl TokenExtractor for BootstrapAgent {
        fn extract_tokens(&self, _batch: &AgentDataBatch) -> TokenExtraction {
            TokenExtraction::default()
        }
    }

    impl BehaviorExtractor for BootstrapAgent {}

    struct FirstCatchUpAgent;

    impl AgentSource for FirstCatchUpAgent {
        fn agent_name(&self) -> &str {
            "first-catch-up"
        }

        fn data_source(&self) -> DataSource {
            DataSource::Sqlite {
                db_path: "/tmp/first-catch-up.db".into(),
            }
        }

        fn log_paths(&self) -> Vec<String> {
            Vec::new()
        }

        fn list_sqlite_session_ids(
            &self,
            _conn: &rusqlite::Connection,
        ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
            Ok(vec!["session-1".to_string()])
        }

        fn query_sqlite_rows_by_created_cursor(
            &self,
            _conn: &rusqlite::Connection,
            _session_id: &str,
            time_created: i64,
            id: &str,
            _limit: usize,
        ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
            if time_created != 0 || !id.is_empty() {
                return Ok(SqliteRowBatch::default());
            }

            Ok(SqliteRowBatch {
                rows: vec![SqliteMessageRow {
                    id: "msg-1".to_string(),
                    session_id: Some("session-1".to_string()),
                    data: "{}".to_string(),
                    time_created: 1000,
                    watermark: 1200,
                }],
                high_watermark: Some(1200),
            })
        }
    }

    impl TokenExtractor for FirstCatchUpAgent {
        fn extract_tokens(&self, _batch: &AgentDataBatch) -> TokenExtraction {
            TokenExtraction::from_logs(vec![TokenLog {
                id: None,
                agent_name: "first-catch-up".to_string(),
                provider: "test".to_string(),
                model_id: "test-model".to_string(),
                token_type: TokenType::Input,
                token_count: 1,
                session_id: Some("session-1".to_string()),
                request_id: Some("msg-1-input".to_string()),
                latency_ms: None,
                is_error: false,
                metadata: None,
                cost: None,
                timestamp: "2026-01-01T00:00:00+00:00".to_string(),
            }])
        }
    }

    impl BehaviorExtractor for FirstCatchUpAgent {}

    struct EmptyUpdatedAgent;

    impl AgentSource for EmptyUpdatedAgent {
        fn agent_name(&self) -> &str {
            "empty-updated"
        }

        fn data_source(&self) -> DataSource {
            DataSource::Sqlite {
                db_path: "/tmp/empty-updated.db".into(),
            }
        }

        fn log_paths(&self) -> Vec<String> {
            Vec::new()
        }

        fn query_sqlite_rows_by_updated_cursor(
            &self,
            _conn: &rusqlite::Connection,
            _session_id: &str,
            _since_updated: i64,
            _limit: usize,
        ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
            Ok(SqliteRowBatch::default())
        }
    }

    impl TokenExtractor for EmptyUpdatedAgent {
        fn extract_tokens(&self, _batch: &AgentDataBatch) -> TokenExtraction {
            TokenExtraction::default()
        }
    }

    impl BehaviorExtractor for EmptyUpdatedAgent {}

    #[test]
    fn bootstrap_cursors_use_old_watermark_without_reading_rows() {
        let agent = BootstrapAgent {
            sessions: vec!["session-1".to_string(), "session-2".to_string()],
            cursors: HashMap::from([(
                "session-1".to_string(),
                SqliteCreatedCursor {
                    time_created: 2000,
                    id: "msg-2".to_string(),
                },
            )]),
        };
        let conn = rusqlite::Connection::open_in_memory().unwrap();

        let cursors = build_bootstrap_cursors(
            &agent,
            &conn,
            "sqlite:/tmp/bootstrap.db",
            &agent.sessions,
            10_000,
        )
        .unwrap();

        assert_eq!(cursors.len(), 2);
        assert_eq!(cursors[0].created_time, 2000);
        assert_eq!(cursors[0].created_id, "msg-2");
        assert_eq!(cursors[0].updated_time, 0);
        assert_eq!(cursors[1].created_time, 0);
        assert_eq!(cursors[1].created_id, "");
    }

    #[test]
    fn merge_existing_cursors_adds_new_sessions_without_rebootstrap() {
        let existing = vec![ExternalSqliteCursor {
            source_key: "sqlite:/tmp/opencode.db".to_string(),
            session_id: "session-1".to_string(),
            created_time: 1000,
            created_id: "msg-1".to_string(),
            updated_time: 1200,
        }];

        let cursors = merge_with_known_sessions(
            "sqlite:/tmp/opencode.db",
            existing,
            vec!["session-1".to_string(), "session-2".to_string()],
        );

        assert_eq!(cursors.len(), 2);
        assert_eq!(cursors[0].session_id, "session-1");
        assert_eq!(cursors[0].created_id, "msg-1");
        assert_eq!(
            cursors[1],
            ExternalSqliteCursor::empty("sqlite:/tmp/opencode.db", "session-2")
        );
    }

    #[test]
    fn cold_start_without_old_watermark_establishes_cursor_after_write() {
        let dir = tempfile::tempdir().unwrap();
        let local_db_path = dir.path().join("local.db");
        let external_db_path = dir.path().join("first-catch-up.db");
        let conn = rusqlite::Connection::open(&local_db_path).unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        drop(conn);
        let external_conn = rusqlite::Connection::open(&external_db_path).unwrap();
        drop(external_conn);

        let (write_tx, write_rx) = std::sync::mpsc::channel();
        let (capture_tx, capture_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = write_rx.recv() {
                let WriteRequest::InsertTokenLogsAndUpdateSqliteCursors {
                    logs,
                    cursors,
                    result_tx,
                } = request
                else {
                    panic!("unexpected write request");
                };
                capture_tx.send((logs.len(), cursors)).unwrap();
                result_tx.send(Ok(())).unwrap();
            }
        });

        let agent = FirstCatchUpAgent;
        let count =
            cold_start_sqlite_source(&agent, &external_db_path, &local_db_path, &write_tx, None)
                .unwrap();
        let (log_count, cursors) = capture_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        assert_eq!(count, 1);
        assert_eq!(log_count, 1);
        assert_eq!(cursors.len(), 1);
        assert_eq!(cursors[0].created_time, 1000);
        assert_eq!(cursors[0].created_id, "msg-1");
        assert_eq!(cursors[0].updated_time, 1200);
    }

    #[test]
    fn advance_created_cursor_uses_time_created_and_id_tie_breaker() {
        let cursor = ExternalSqliteCursor::empty("sqlite:/tmp/mimocode.db", "session-1");
        let rows = vec![
            SqliteMessageRow {
                id: "msg-a".to_string(),
                session_id: Some("session-1".to_string()),
                data: "{}".to_string(),
                time_created: 1000,
                watermark: 1100,
            },
            SqliteMessageRow {
                id: "msg-b".to_string(),
                session_id: Some("session-1".to_string()),
                data: "{}".to_string(),
                time_created: 1000,
                watermark: 1300,
            },
        ];

        let next = advance_created_cursor(&cursor, &rows);

        assert_eq!(next.created_time, 1000);
        assert_eq!(next.created_id, "msg-b");
        assert_eq!(next.updated_time, 1300);
    }

    #[test]
    fn updated_cursor_advances_to_now_after_complete_sweep() {
        let batch = SqliteRowBatch::default();
        let next = next_updated_cursor_time(1000, &batch);

        assert!(next >= 1000);
    }

    #[test]
    fn empty_updated_reconciliation_does_not_persist_cursor() {
        let agent = EmptyUpdatedAgent;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let (write_tx, write_rx) = std::sync::mpsc::channel();
        let (capture_tx, capture_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            if let Ok(request) = write_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                let WriteRequest::InsertTokenLogsAndUpdateSqliteCursors {
                    logs,
                    cursors,
                    result_tx,
                } = request
                else {
                    panic!("unexpected write request");
                };
                capture_tx.send((logs.len(), cursors.len())).unwrap();
                result_tx.send(Ok(())).unwrap();
            }
        });
        let mut cursors = vec![ExternalSqliteCursor {
            source_key: "sqlite:/tmp/empty-updated.db".to_string(),
            session_id: "session-1".to_string(),
            created_time: 0,
            created_id: String::new(),
            updated_time: 1000,
        }];
        let mut reconcile_index = 0usize;

        let processed = process_updated_rows(
            &agent,
            &conn,
            Path::new("/tmp/empty-updated.db"),
            "sqlite:/tmp/empty-updated.db",
            &write_tx,
            &mut cursors,
            &mut reconcile_index,
        )
        .unwrap();

        drop(write_tx);
        handle.join().unwrap();
        assert_eq!(processed, 0);
        assert!(cursors[0].updated_time >= 1000);
        assert!(capture_rx.try_recv().is_err());
    }
}
