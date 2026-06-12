use super::{
    AgentDataBatch, AgentSource, BehaviorExtractor, DataSource, SqliteCreatedCursor,
    SqliteMessageRow, SqliteRowBatch, TokenExtraction, TokenExtractor, TokenLog, TokenType,
    SQLITE_NULL_SESSION_ID,
};
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};

use crate::behavior;

pub struct OpenCodeAdapter;

impl AgentSource for OpenCodeAdapter {
    fn agent_name(&self) -> &str {
        "opencode"
    }

    fn data_source(&self) -> DataSource {
        let db_path = get_opencode_db_path();
        if db_path.exists() {
            DataSource::Sqlite { db_path }
        } else {
            // fallback 到旧版 JSON
            let home = dirs::home_dir().unwrap_or_default();
            DataSource::Json {
                paths: vec![home
                    .join(".local")
                    .join("share")
                    .join("opencode")
                    .join("storage")
                    .join("message")],
            }
        }
    }

    fn log_paths(&self) -> Vec<String> {
        let home = dirs::home_dir().unwrap_or_default();
        vec![format!(
            "{}/.local/share/opencode/storage/message/**/*.json",
            home.display()
        )]
    }

    fn query_sqlite_rows(
        &self,
        db_path: &Path,
        since: Option<u64>,
    ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
        query_message_rows(db_path, since)
    }

    fn list_sqlite_session_ids(
        &self,
        conn: &rusqlite::Connection,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        list_session_ids(conn)
    }

    fn query_sqlite_rows_by_created_cursor(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
        time_created: i64,
        id: &str,
        limit: usize,
    ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
        query_message_rows_by_created_cursor(conn, session_id, time_created, id, limit)
    }

    fn query_sqlite_rows_by_updated_cursor(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
        since_updated: i64,
        limit: usize,
    ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
        query_message_rows_by_updated_cursor(conn, session_id, since_updated, limit)
    }

    fn query_sqlite_created_cursor_before_watermark(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
        watermark: u64,
    ) -> Result<Option<SqliteCreatedCursor>, Box<dyn std::error::Error>> {
        query_created_cursor_before_watermark(conn, session_id, watermark)
    }
}

impl TokenExtractor for OpenCodeAdapter {
    fn extract_tokens(&self, batch: &AgentDataBatch) -> TokenExtraction {
        match batch {
            AgentDataBatch::JsonDocument { content, .. }
            | AgentDataBatch::JsonlIncrement { content, .. } => {
                TokenExtraction::from_logs(parse_opencode_json_content(content))
            }
            AgentDataBatch::SqliteRows { rows, .. } => {
                let fallback_ts = chrono::Local::now()
                    .format("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string();
                let logs = rows
                    .iter()
                    .flat_map(|row| parse_opencode_message_row(row, &fallback_ts))
                    .collect();

                TokenExtraction::from_logs(logs)
            }
        }
    }
}

impl BehaviorExtractor for OpenCodeAdapter {
    fn extract_behavior(&self, batch: &AgentDataBatch) -> Vec<crate::behavior::AgentBehaviorEvent> {
        let AgentDataBatch::SqliteRows { rows, .. } = batch else {
            return Vec::new();
        };

        rows.iter()
            .filter_map(|row| {
                let row = behavior::opencode::OpenCodeMessageRow {
                    id: row.id.clone(),
                    session_id: row.session_id.clone(),
                    data: row.data.clone(),
                    time_created: row.time_created,
                };
                behavior::opencode::parse_message_row(&row)
            })
            .collect()
    }
}

/// 旧版 OpenCode JSON fallback 解析
pub(crate) fn parse_opencode_json_content(content: &str) -> Vec<TokenLog> {
    let now = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();

    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(val) => parse_opencode_json(&val, &now),
        Err(e) => {
            log::warn!("opencode: JSON 解析失败: {}", e);
            Vec::new()
        }
    }
}

/// 查询 OpenCode SQLite message row
#[allow(dead_code)]
pub(crate) fn query_message_rows(
    db_path: &Path,
    since: Option<u64>,
) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let watermark_column = opencode_watermark_column(&conn)?;
    let since_ts = since.unwrap_or(0).min(i64::MAX as u64) as i64;
    let sql = format!(
        "SELECT id, session_id, data, time_created, {watermark_column} FROM message
         WHERE {watermark_column} > ?1
         ORDER BY {watermark_column} ASC"
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map(rusqlite::params![since_ts], |row| {
        Ok(SqliteMessageRow {
            id: row.get::<_, String>(0)?,
            session_id: row.get::<_, Option<String>>(1)?,
            data: row.get::<_, String>(2)?,
            time_created: row.get::<_, i64>(3)?,
            watermark: row.get::<_, i64>(4)?,
        })
    })?;

    let mut message_rows = Vec::new();
    let mut high_watermark = None;

    for row in rows {
        let row = row?;
        if row.watermark >= 0 {
            let ts = row.watermark as u64;
            high_watermark = Some(high_watermark.map_or(ts, |prev: u64| prev.max(ts)));
        }
        message_rows.push(row);
    }

    Ok(SqliteRowBatch {
        rows: message_rows,
        high_watermark,
    })
}

/// 列出 OpenCode message 表中已知 session id
pub(crate) fn list_session_ids(
    conn: &rusqlite::Connection,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if let Ok(mut stmt) = conn.prepare("SELECT id FROM session ORDER BY id ASC") {
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
    }

    let mut stmt = conn.prepare(
        "SELECT DISTINCT COALESCE(session_id, ?1)
         FROM message
         ORDER BY 1 ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![SQLITE_NULL_SESSION_ID], |row| {
        row.get::<_, String>(0)
    })?;

    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// 按 session created cursor 查询 OpenCode message row
pub(crate) fn query_message_rows_by_created_cursor(
    conn: &rusqlite::Connection,
    session_id: &str,
    time_created: i64,
    id: &str,
    limit: usize,
) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
    let watermark_column = opencode_watermark_column(conn)?;
    let limit = limit.min(i64::MAX as usize) as i64;

    let mut message_rows = Vec::new();
    let mut high_watermark = None;

    if session_id == SQLITE_NULL_SESSION_ID {
        let sql = format!(
            "SELECT id, session_id, data, time_created, {watermark_column} FROM message
             WHERE session_id IS NULL
               AND (time_created > ?1 OR (time_created = ?2 AND id > ?3))
             ORDER BY time_created ASC, id ASC
             LIMIT ?4"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params![time_created, time_created, id, limit],
            |row| {
                Ok(SqliteMessageRow {
                    id: row.get::<_, String>(0)?,
                    session_id: row.get::<_, Option<String>>(1)?,
                    data: row.get::<_, String>(2)?,
                    time_created: row.get::<_, i64>(3)?,
                    watermark: row.get::<_, i64>(4)?,
                })
            },
        )?;
        collect_rows(rows, &mut message_rows, &mut high_watermark)?;
    } else {
        let sql = format!(
            "SELECT id, session_id, data, time_created, {watermark_column} FROM message
             WHERE session_id = ?1
               AND (time_created > ?2 OR (time_created = ?3 AND id > ?4))
             ORDER BY time_created ASC, id ASC
             LIMIT ?5"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params![session_id, time_created, time_created, id, limit],
            |row| {
                Ok(SqliteMessageRow {
                    id: row.get::<_, String>(0)?,
                    session_id: row.get::<_, Option<String>>(1)?,
                    data: row.get::<_, String>(2)?,
                    time_created: row.get::<_, i64>(3)?,
                    watermark: row.get::<_, i64>(4)?,
                })
            },
        )?;
        collect_rows(rows, &mut message_rows, &mut high_watermark)?;
    }

    Ok(SqliteRowBatch {
        rows: message_rows,
        high_watermark,
    })
}

/// 按 session updated cursor 查询 OpenCode message row
pub(crate) fn query_message_rows_by_updated_cursor(
    conn: &rusqlite::Connection,
    session_id: &str,
    since_updated: i64,
    limit: usize,
) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
    let watermark_column = opencode_watermark_column(conn)?;
    let limit = limit.min(i64::MAX as usize) as i64;

    let mut message_rows = Vec::new();
    let mut high_watermark = None;

    if session_id == SQLITE_NULL_SESSION_ID {
        let sql = format!(
            "SELECT id, session_id, data, time_created, {watermark_column} FROM message
             WHERE session_id IS NULL AND {watermark_column} >= ?1
             ORDER BY {watermark_column} ASC, id ASC
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![since_updated, limit], |row| {
            Ok(SqliteMessageRow {
                id: row.get::<_, String>(0)?,
                session_id: row.get::<_, Option<String>>(1)?,
                data: row.get::<_, String>(2)?,
                time_created: row.get::<_, i64>(3)?,
                watermark: row.get::<_, i64>(4)?,
            })
        })?;
        collect_rows(rows, &mut message_rows, &mut high_watermark)?;
    } else {
        let sql = format!(
            "SELECT id, session_id, data, time_created, {watermark_column} FROM message
             WHERE session_id = ?1 AND {watermark_column} >= ?2
             ORDER BY {watermark_column} ASC, id ASC
             LIMIT ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![session_id, since_updated, limit], |row| {
            Ok(SqliteMessageRow {
                id: row.get::<_, String>(0)?,
                session_id: row.get::<_, Option<String>>(1)?,
                data: row.get::<_, String>(2)?,
                time_created: row.get::<_, i64>(3)?,
                watermark: row.get::<_, i64>(4)?,
            })
        })?;
        collect_rows(rows, &mut message_rows, &mut high_watermark)?;
    }

    Ok(SqliteRowBatch {
        rows: message_rows,
        high_watermark,
    })
}

/// 查询旧 global watermark 前最后一条 OpenCode message created cursor
pub(crate) fn query_created_cursor_before_watermark(
    conn: &rusqlite::Connection,
    session_id: &str,
    watermark: u64,
) -> Result<Option<SqliteCreatedCursor>, Box<dyn std::error::Error>> {
    let watermark_column = opencode_watermark_column(&conn)?;
    let watermark = watermark.min(i64::MAX as u64) as i64;

    let result = if session_id == SQLITE_NULL_SESSION_ID {
        let sql = format!(
            "SELECT time_created, id FROM message
             WHERE session_id IS NULL AND {watermark_column} <= ?1
             ORDER BY time_created DESC, id DESC
             LIMIT 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        stmt.query_row(rusqlite::params![watermark], |row| {
            Ok(SqliteCreatedCursor {
                time_created: row.get(0)?,
                id: row.get(1)?,
            })
        })
        .optional()?
    } else {
        let sql = format!(
            "SELECT time_created, id FROM message
             WHERE session_id = ?1 AND {watermark_column} <= ?2
             ORDER BY time_created DESC, id DESC
             LIMIT 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        stmt.query_row(rusqlite::params![session_id, watermark], |row| {
            Ok(SqliteCreatedCursor {
                time_created: row.get(0)?,
                id: row.get(1)?,
            })
        })
        .optional()?
    };

    Ok(result)
}

fn collect_rows<F>(
    rows: rusqlite::MappedRows<'_, F>,
    message_rows: &mut Vec<SqliteMessageRow>,
    high_watermark: &mut Option<u64>,
) -> Result<(), rusqlite::Error>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<SqliteMessageRow>,
{
    for row in rows {
        let row = row?;
        if row.watermark >= 0 {
            let ts = row.watermark as u64;
            *high_watermark = Some((*high_watermark).map_or(ts, |prev| prev.max(ts)));
        }
        message_rows.push(row);
    }

    Ok(())
}

fn opencode_watermark_column(conn: &rusqlite::Connection) -> Result<&'static str, rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(message)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;

    for row in rows {
        if row? == "time_updated" {
            return Ok("time_updated");
        }
    }

    Ok("time_created")
}

fn parse_opencode_message_row(row: &SqliteMessageRow, fallback_ts: &str) -> Vec<TokenLog> {
    let mut logs = Vec::new();
    let ts_str = chrono::DateTime::from_timestamp(row.time_created / 1000, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        })
        .unwrap_or_else(|| fallback_ts.to_string());
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&row.data) else {
        return logs;
    };

    // role 实际存放在 data JSON 内部，仅保留 assistant 消息
    if data.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return logs;
    }

    let Some(tokens) = data.get("tokens") else {
        return logs;
    };

    let input = tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
    let output = tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
    let cache_read = tokens
        .get("cache")
        .and_then(|c| c.get("read"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_write = tokens
        .get("cache")
        .and_then(|c| c.get("write"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let model = data
        .get("modelID")
        .or_else(|| data.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let provider = data
        .get("providerID")
        .and_then(|v| v.as_str())
        .unwrap_or("OpenCode")
        .to_string();
    let cost = data
        .get("cost")
        .and_then(|v| v.as_f64())
        .filter(|&c| c > 0.0);

    let make = |tt: TokenType, count: i64, suffix: &str, log_cost: Option<f64>| TokenLog {
        id: None,
        agent_name: "opencode".into(),
        provider: provider.clone(),
        model_id: model.clone(),
        token_type: tt,
        token_count: count,
        session_id: row.session_id.clone(),
        request_id: Some(format!("{}-{}", row.id, suffix)),
        latency_ms: None,
        is_error: false,
        metadata: None,
        cost: log_cost,
        timestamp: ts_str.clone(),
    };

    if input > 0 {
        logs.push(make(TokenType::Input, input, "input", cost));
    }
    if output > 0 {
        logs.push(make(TokenType::Output, output, "output", None));
    }
    if cache_read > 0 {
        logs.push(make(TokenType::CacheRead, cache_read, "cache_read", None));
    }
    if cache_write > 0 {
        logs.push(make(
            TokenType::CacheCreate,
            cache_write,
            "cache_create",
            None,
        ));
    }

    logs
}

fn get_opencode_db_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db")
}

fn parse_opencode_json(val: &serde_json::Value, fallback_ts: &str) -> Vec<TokenLog> {
    let mut logs = Vec::new();

    let tokens = match val.get("tokens") {
        Some(t) => t,
        None => return logs,
    };

    let input = tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
    let output = tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
    let cache_read = tokens
        .get("cache")
        .and_then(|c| c.get("read"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_write = tokens
        .get("cache")
        .and_then(|c| c.get("write"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
        return logs;
    }

    let model = val
        .get("modelID")
        .or_else(|| val.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let provider = val
        .get("providerID")
        .and_then(|v| v.as_str())
        .unwrap_or("OpenCode")
        .to_string();
    let msg_id = val.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let cost = val
        .get("cost")
        .and_then(|v| v.as_f64())
        .filter(|&c| c > 0.0);

    let make = |tt: TokenType, count: i64, suffix: &str, log_cost: Option<f64>| TokenLog {
        id: None,
        agent_name: "opencode".into(),
        provider: provider.clone(),
        model_id: model.clone(),
        token_type: tt,
        token_count: count,
        session_id: None,
        request_id: Some(format!("{}-{}", msg_id, suffix)),
        latency_ms: None,
        is_error: false,
        metadata: None,
        cost: log_cost,
        timestamp: fallback_ts.to_string(),
    };

    if input > 0 {
        logs.push(make(TokenType::Input, input, "input", cost));
    }
    if output > 0 {
        logs.push(make(TokenType::Output, output, "output", None));
    }
    if cache_read > 0 {
        logs.push(make(TokenType::CacheRead, cache_read, "cache_read", None));
    }
    if cache_write > 0 {
        logs.push(make(
            TokenType::CacheCreate,
            cache_write,
            "cache_create",
            None,
        ));
    }

    logs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_batch(content: &str) -> AgentDataBatch {
        AgentDataBatch::JsonDocument {
            agent_name: "opencode".to_string(),
            source_key: "message.json".to_string(),
            path: "message.json".into(),
            content: content.to_string(),
            mtime: 0,
        }
    }

    fn sqlite_batch(rows: Vec<SqliteMessageRow>, high_watermark: Option<u64>) -> AgentDataBatch {
        AgentDataBatch::SqliteRows {
            agent_name: "opencode".to_string(),
            source_key: "sqlite:/tmp/opencode.db".to_string(),
            db_path: "/tmp/opencode.db".into(),
            rows,
            previous_watermark: None,
            next_watermark: high_watermark,
        }
    }

    #[test]
    fn test_parse_opencode_json() {
        let json = r#"{"id":"msg-1","modelID":"gemini-3-flash-preview","providerID":"quotio","tokens":{"input":500,"output":200,"cache":{"read":50,"write":30}}}"#;
        let adapter = OpenCodeAdapter;
        let logs = adapter.extract_tokens(&json_batch(json)).logs;
        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 500);
        assert_eq!(logs[0].provider, "quotio");
        assert_eq!(logs[0].model_id, "gemini-3-flash-preview");
        assert_eq!(logs[0].cost, None);
    }

    #[test]
    fn test_parse_opencode_cost_only_on_input() {
        let json = r#"{"id":"msg-1","modelID":"gemini-3-flash-preview","providerID":"quotio","cost":0.12,"tokens":{"input":500,"output":200,"cache":{"read":50,"write":30}}}"#;
        let adapter = OpenCodeAdapter;
        let logs = adapter.extract_tokens(&json_batch(json)).logs;

        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].cost, Some(0.12));
        assert!(logs[1..].iter().all(|log| log.cost.is_none()));
    }

    #[test]
    fn test_no_tokens_field() {
        let json = r#"{"id":"msg-2","model":"test"}"#;
        let adapter = OpenCodeAdapter;
        let logs = adapter.extract_tokens(&json_batch(json)).logs;
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = OpenCodeAdapter;
        assert!(adapter.extract_tokens(&json_batch("")).logs.is_empty());
    }

    #[test]
    fn test_query_db_uses_millisecond_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT NOT NULL,
                session_id TEXT,
                data TEXT NOT NULL,
                time_created INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "old",
                "session-1",
                r#"{"role":"assistant","modelID":"gpt-test","providerID":"openai","tokens":{"input":1}}"#,
                1000i64,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "new",
                "session-1",
                r#"{"role":"assistant","modelID":"gpt-test","providerID":"openai","tokens":{"input":2}}"#,
                2500i64,
            ],
        )
        .unwrap();
        drop(conn);

        let adapter = OpenCodeAdapter;
        let rows = adapter.query_sqlite_rows(&db_path, Some(1000)).unwrap();
        let high_watermark = rows.high_watermark;
        let logs = adapter
            .extract_tokens(&sqlite_batch(rows.rows, high_watermark))
            .logs;

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].request_id.as_deref(), Some("new-input"));
        assert_eq!(logs[0].token_count, 2);
        assert_eq!(high_watermark, Some(2500));

        let rows = adapter.query_sqlite_rows(&db_path, Some(2500)).unwrap();
        let high_watermark = rows.high_watermark;
        let logs = adapter
            .extract_tokens(&sqlite_batch(rows.rows, high_watermark))
            .logs;
        assert!(logs.is_empty());
        assert_eq!(high_watermark, None);
    }

    #[test]
    fn test_query_sqlite_rows_uses_time_updated_and_extractors_share_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT NOT NULL,
                session_id TEXT,
                data TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created, time_updated)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "done",
                "session-1",
                r#"{"role":"assistant","finish":"stop","modelID":"gpt-test","providerID":"openai","tokens":{"input":2}}"#,
                2500i64,
                3000i64,
            ],
        )
        .unwrap();
        drop(conn);

        let adapter = OpenCodeAdapter;
        let row_batch = adapter.query_sqlite_rows(&db_path, Some(2500)).unwrap();
        let high_watermark = row_batch.high_watermark;
        let batch = sqlite_batch(row_batch.rows, high_watermark);
        let logs = adapter.extract_tokens(&batch).logs;
        let behavior_events = adapter.extract_behavior(&batch);

        assert_eq!(logs.len(), 1);
        assert_eq!(behavior_events.len(), 1);
        assert_eq!(high_watermark, Some(3000));
        assert_eq!(behavior_events[0].session_id, "session-1");
    }

    #[test]
    fn test_created_cursor_query_uses_session_time_created_id_index() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT NOT NULL,
                session_id TEXT,
                data TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );
            CREATE INDEX message_session_time_created_id_idx
                ON message(session_id, time_created, id);",
        )
        .unwrap();
        for (id, time_created, time_updated) in [
            ("msg-a", 1000i64, 1100i64),
            ("msg-b", 1000i64, 1200i64),
            ("msg-c", 2000i64, 2100i64),
        ] {
            conn.execute(
                "INSERT INTO message (id, session_id, data, time_created, time_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    id,
                    "session-1",
                    r#"{"role":"assistant","tokens":{"input":1}}"#,
                    time_created,
                    time_updated,
                ],
            )
            .unwrap();
        }

        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT id, session_id, data, time_created, time_updated FROM message
                 WHERE session_id = ?1
                   AND (time_created > ?2 OR (time_created = ?3 AND id > ?4))
                 ORDER BY time_created ASC, id ASC
                 LIMIT ?5",
            )
            .unwrap();
        let details = stmt
            .query_map(
                rusqlite::params!["session-1", 1000i64, 1000i64, "msg-a", 10i64],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        assert!(
            details.contains("message_session_time_created_id_idx"),
            "unexpected query plan: {details}"
        );
        drop(stmt);

        let row_batch =
            query_message_rows_by_created_cursor(&conn, "session-1", 1000, "msg-a", 10).unwrap();
        let ids = row_batch
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["msg-b", "msg-c"]);
        assert_eq!(row_batch.high_watermark, Some(2100));
    }
}
