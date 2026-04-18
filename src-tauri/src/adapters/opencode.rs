use super::{AgentAdapter, DataSource, TokenLog, TokenType};
use std::path::{Path, PathBuf};

pub struct OpenCodeAdapter;

impl AgentAdapter for OpenCodeAdapter {
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

    fn parse_content(&self, content: &str) -> Vec<TokenLog> {
        // 旧版 JSON fallback 解析
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

    fn query_db(
        &self,
        db_path: &Path,
        since: Option<i64>,
    ) -> Result<Vec<TokenLog>, Box<dyn std::error::Error>> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;

        let now = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        // since 是秒级时间戳，time_created 是毫秒级，需要转换
        let since_ts = since.unwrap_or(0) * 1000;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, data, time_created FROM message
             WHERE time_created > ?1
             ORDER BY time_created ASC",
        )?;

        let mut logs = Vec::new();
        let rows = stmt.query_map(rusqlite::params![since_ts], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;

        for row in rows {
            let (msg_id, session_id, data_str, time_created) = row?;
            // time_created 是毫秒级时间戳，需要转换为秒
            let ts_str = chrono::DateTime::from_timestamp(time_created / 1000, 0)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%dT%H:%M:%S%:z")
                        .to_string()
                })
                .unwrap_or_else(|| now.clone());
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&data_str) {
                // role 实际存放在 data JSON 内部，仅保留 assistant 消息
                if data.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                    continue;
                }

                if let Some(tokens) = data.get("tokens") {
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

                    let make =
                        |tt: TokenType, count: i64, suffix: &str, log_cost: Option<f64>| TokenLog {
                            id: None,
                            agent_name: "opencode".into(),
                            provider: provider.clone(),
                            model_id: model.clone(),
                            token_type: tt,
                            token_count: count,
                            session_id: session_id.clone(),
                            request_id: Some(format!("{}-{}", msg_id, suffix)),
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
                }
            }
        }
        Ok(logs)
    }
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

    #[test]
    fn test_parse_opencode_json() {
        let json = r#"{"id":"msg-1","modelID":"gemini-3-flash-preview","providerID":"quotio","tokens":{"input":500,"output":200,"cache":{"read":50,"write":30}}}"#;
        let adapter = OpenCodeAdapter;
        let logs = adapter.parse_content(json);
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
        let logs = adapter.parse_content(json);

        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].cost, Some(0.12));
        assert!(logs[1..].iter().all(|log| log.cost.is_none()));
    }

    #[test]
    fn test_no_tokens_field() {
        let json = r#"{"id":"msg-2","model":"test"}"#;
        let adapter = OpenCodeAdapter;
        let logs = adapter.parse_content(json);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = OpenCodeAdapter;
        assert!(adapter.parse_content("").is_empty());
    }
}
