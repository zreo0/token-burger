use super::{
    AgentDataBatch, AgentSource, BehaviorExtractor, DataSource, SqliteMessageRow, SqliteRowBatch,
    TokenExtraction, TokenExtractor, TokenLog, TokenType,
};
use std::path::{Path, PathBuf};

use crate::behavior;

pub struct MiMoCodeAdapter;

impl AgentSource for MiMoCodeAdapter {
    fn agent_name(&self) -> &str {
        "mimocode"
    }

    fn data_source(&self) -> DataSource {
        DataSource::Sqlite {
            db_path: get_mimocode_db_path(),
        }
    }

    fn log_paths(&self) -> Vec<String> {
        Vec::new()
    }

    fn query_sqlite_rows(
        &self,
        db_path: &Path,
        since: Option<u64>,
    ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
        query_message_rows(db_path, since)
    }
}

impl TokenExtractor for MiMoCodeAdapter {
    fn extract_tokens(&self, batch: &AgentDataBatch) -> TokenExtraction {
        let AgentDataBatch::SqliteRows { rows, .. } = batch else {
            return TokenExtraction::default();
        };

        let fallback_ts = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        let logs = rows
            .iter()
            .flat_map(|row| parse_mimocode_message_row(row, &fallback_ts))
            .collect();

        TokenExtraction::from_logs(logs)
    }
}

impl BehaviorExtractor for MiMoCodeAdapter {
    fn extract_behavior(&self, batch: &AgentDataBatch) -> Vec<crate::behavior::AgentBehaviorEvent> {
        let AgentDataBatch::SqliteRows { rows, .. } = batch else {
            return Vec::new();
        };

        rows.iter()
            .filter_map(|row| {
                let row = behavior::mimocode::MiMoCodeMessageRow {
                    id: row.id.clone(),
                    session_id: row.session_id.clone(),
                    data: row.data.clone(),
                    time_created: row.time_created,
                };
                behavior::mimocode::parse_message_row(&row)
            })
            .collect()
    }
}

/// 查询 MiMoCode SQLite message row
pub(crate) fn query_message_rows(
    db_path: &Path,
    since: Option<u64>,
) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let watermark_column = mimocode_watermark_column(&conn)?;
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

fn mimocode_watermark_column(conn: &rusqlite::Connection) -> Result<&'static str, rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(message)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;

    for row in rows {
        if row? == "time_updated" {
            return Ok("time_updated");
        }
    }

    Ok("time_created")
}

fn parse_mimocode_message_row(row: &SqliteMessageRow, fallback_ts: &str) -> Vec<TokenLog> {
    let mut logs = Vec::new();
    let ts_str = chrono::DateTime::from_timestamp(row.time_created / 1000, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        })
        .unwrap_or_else(|| fallback_ts.to_string());
    let data = match serde_json::from_str::<serde_json::Value>(&row.data) {
        Ok(data) => data,
        Err(e) => {
            log::warn!("mimocode: message data JSON 解析失败: {}", e);
            return logs;
        }
    };

    if data.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return logs;
    }

    let Some(tokens) = data.get("tokens") else {
        log::warn!("mimocode: assistant message 缺少 tokens 字段: {}", row.id);
        return logs;
    };

    let input = tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
    let output = tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
    let reasoning = tokens
        .get("reasoning")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output_total = output.saturating_add(reasoning);
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
        .unwrap_or("MiMoCode")
        .to_string();
    let cost = data
        .get("cost")
        .and_then(|v| v.as_f64())
        .filter(|&c| c > 0.0);

    let make = |tt: TokenType,
                count: i64,
                suffix: &str,
                log_cost: Option<f64>,
                metadata: Option<String>|
     -> TokenLog {
        TokenLog {
            id: None,
            agent_name: "mimocode".into(),
            provider: provider.clone(),
            model_id: model.clone(),
            token_type: tt,
            token_count: count,
            session_id: row.session_id.clone(),
            request_id: Some(format!("{}-{}", row.id, suffix)),
            latency_ms: None,
            is_error: false,
            metadata,
            cost: log_cost,
            timestamp: ts_str.clone(),
        }
    };

    if input > 0 {
        logs.push(make(TokenType::Input, input, "input", cost, None));
    }
    if output_total > 0 {
        let metadata =
            (reasoning > 0).then(|| serde_json::json!({ "reasoning": reasoning }).to_string());
        logs.push(make(
            TokenType::Output,
            output_total,
            "output",
            None,
            metadata,
        ));
    }
    if cache_read > 0 {
        logs.push(make(
            TokenType::CacheRead,
            cache_read,
            "cache_read",
            None,
            None,
        ));
    }
    if cache_write > 0 {
        logs.push(make(
            TokenType::CacheCreate,
            cache_write,
            "cache_create",
            None,
            None,
        ));
    }

    logs
}

fn get_mimocode_db_path() -> PathBuf {
    if let Some(path) = env_path("MIMOCODE_DB") {
        return path;
    }

    if let Some(home) = env_path("MIMOCODE_HOME") {
        return home.join("data").join("mimocode.db");
    }

    let home = dirs::home_dir().unwrap_or_default();
    home.join(".local")
        .join("share")
        .join("mimocode")
        .join("mimocode.db")
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_batch(rows: Vec<SqliteMessageRow>, high_watermark: Option<u64>) -> AgentDataBatch {
        AgentDataBatch::SqliteRows {
            agent_name: "mimocode".to_string(),
            source_key: "sqlite:/tmp/mimocode.db".to_string(),
            db_path: "/tmp/mimocode.db".into(),
            rows,
            previous_watermark: None,
            next_watermark: high_watermark,
        }
    }

    fn message_row(id: &str, data: &str) -> SqliteMessageRow {
        SqliteMessageRow {
            id: id.to_string(),
            session_id: Some("session-1".to_string()),
            data: data.to_string(),
            time_created: 1_780_000_000_000,
            watermark: 1_780_000_000_000,
        }
    }

    #[test]
    fn parses_mimocode_assistant_tokens() {
        let adapter = MiMoCodeAdapter;
        let row = message_row(
            "msg-1",
            r#"{"role":"assistant","modelID":"mimo-v2.5-pro","providerID":"mimo","cost":0.12,"tokens":{"input":500,"output":200,"cache":{"read":50,"write":30}}}"#,
        );
        let logs = adapter.extract_tokens(&sqlite_batch(vec![row], None)).logs;

        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].agent_name, "mimocode");
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 500);
        assert_eq!(logs[0].provider, "mimo");
        assert_eq!(logs[0].model_id, "mimo-v2.5-pro");
        assert_eq!(logs[0].session_id.as_deref(), Some("session-1"));
        assert_eq!(logs[0].request_id.as_deref(), Some("msg-1-input"));
        assert_eq!(logs[0].cost, Some(0.12));
        assert!(logs[1..].iter().all(|log| log.cost.is_none()));
    }

    #[test]
    fn folds_reasoning_into_output_metadata() {
        let adapter = MiMoCodeAdapter;
        let row = message_row(
            "msg-1",
            r#"{"role":"assistant","tokens":{"output":200,"reasoning":128}}"#,
        );
        let logs = adapter.extract_tokens(&sqlite_batch(vec![row], None)).logs;

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].token_type, TokenType::Output);
        assert_eq!(logs[0].token_count, 328);
        assert_eq!(logs[0].metadata.as_deref(), Some(r#"{"reasoning":128}"#));
    }

    #[test]
    fn skips_non_assistant_or_invalid_rows() {
        let adapter = MiMoCodeAdapter;
        let user_row = message_row("user", r#"{"role":"user","tokens":{"input":500}}"#);
        let invalid_row = message_row("invalid", r#"{"role":"assistant","#);
        let missing_tokens = message_row("missing", r#"{"role":"assistant"}"#);
        let logs = adapter
            .extract_tokens(&sqlite_batch(
                vec![user_row, invalid_row, missing_tokens],
                None,
            ))
            .logs;

        assert!(logs.is_empty());
    }

    #[test]
    fn query_db_uses_time_created_when_time_updated_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("mimocode.db");
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
                r#"{"role":"assistant","providerID":"mimo","tokens":{"input":1}}"#,
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
                r#"{"role":"assistant","providerID":"mimo","tokens":{"input":2}}"#,
                2500i64,
            ],
        )
        .unwrap();
        drop(conn);

        let adapter = MiMoCodeAdapter;
        let row_batch = adapter.query_sqlite_rows(&db_path, Some(1000)).unwrap();
        let high_watermark = row_batch.high_watermark;
        let logs = adapter
            .extract_tokens(&sqlite_batch(row_batch.rows, high_watermark))
            .logs;

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].request_id.as_deref(), Some("new-input"));
        assert_eq!(logs[0].token_count, 2);
        assert_eq!(high_watermark, Some(2500));
    }

    #[test]
    fn query_db_prefers_time_updated_and_extracts_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("mimocode.db");
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
                r#"{"role":"assistant","finish":"stop","providerID":"mimo","tokens":{"input":2}}"#,
                2500i64,
                3000i64,
            ],
        )
        .unwrap();
        drop(conn);

        let adapter = MiMoCodeAdapter;
        let row_batch = adapter.query_sqlite_rows(&db_path, Some(2500)).unwrap();
        let high_watermark = row_batch.high_watermark;
        let batch = sqlite_batch(row_batch.rows, high_watermark);
        let logs = adapter.extract_tokens(&batch).logs;
        let behavior_events = adapter.extract_behavior(&batch);

        assert_eq!(logs.len(), 1);
        assert_eq!(behavior_events.len(), 1);
        assert_eq!(high_watermark, Some(3000));
        assert_eq!(behavior_events[0].agent_name, "mimocode");
        assert_eq!(behavior_events[0].session_id, "session-1");
    }
}
