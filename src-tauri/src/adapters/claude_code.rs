use super::{
    AgentDataBatch, AgentSource, BehaviorExtractor, DataSource, TokenExtraction, TokenExtractor,
    TokenLog, TokenType,
};
use std::path::{Path, PathBuf};

/**
 * 解析 Claude 配置目录，空环境变量回退到用户目录
 */
pub(crate) fn config_root() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

/**
 * 根据配置目录和用户目录生成日志模式，涵盖共享日志及桌面端嵌套日志
 */
fn log_patterns(config: &Path, home: &Path, desktop: bool) -> Vec<String> {
    let mut patterns = vec![format!(
        "{}/projects/**/*.jsonl",
        glob::Pattern::escape(&config.to_string_lossy())
    )];
    if desktop {
        // 自定义 CLI profile 不应遮住桌面端仍写入默认目录的共享日志
        let shared = home.join(".claude");
        if config != shared {
            patterns.push(format!(
                "{}/projects/**/*.jsonl",
                glob::Pattern::escape(&shared.to_string_lossy())
            ));
        }
        for directory in ["claude-code-sessions", "local-agent-mode-sessions"] {
            let root = home
                .join("Library/Application Support/Claude")
                .join(directory);
            let root = glob::Pattern::escape(&root.to_string_lossy());
            // 限制发现深度，避免扫描桌面会话检出的仓库及依赖目录
            for depth in 0..=4 {
                patterns.push(format!(
                    "{root}/{}.claude/projects/**/*.jsonl",
                    "*/".repeat(depth)
                ));
            }
        }
    }
    patterns
}

pub struct ClaudeCodeAdapter;

impl AgentSource for ClaudeCodeAdapter {
    fn agent_name(&self) -> &str {
        "claude-code"
    }

    /**
     * 返回 Claude JSONL 数据源，实际文件范围由 log_paths 提供
     */
    fn data_source(&self) -> DataSource {
        let base = config_root().join("projects");
        DataSource::Jsonl { paths: vec![base] }
    }

    /**
     * 返回当前配置及桌面端的日志发现模式
     */
    fn log_paths(&self) -> Vec<String> {
        let home = dirs::home_dir().unwrap_or_default();
        log_patterns(&config_root(), &home, cfg!(target_os = "macos"))
    }
}

impl TokenExtractor for ClaudeCodeAdapter {
    fn extract_tokens(&self, batch: &AgentDataBatch) -> TokenExtraction {
        let Some(content) = batch.token_content() else {
            return TokenExtraction::default();
        };

        TokenExtraction::from_logs(parse_claude_content(content))
    }
}

impl BehaviorExtractor for ClaudeCodeAdapter {}

/**
 * 解析一批 Claude JSONL，返回按响应身份标记的各类 token 记录
 */
pub(crate) fn parse_claude_content(content: &str) -> Vec<TokenLog> {
    let mut logs = Vec::new();
    let now = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(val) => {
                if let Some(parsed) = parse_claude_line(&val, &now) {
                    logs.extend(parsed);
                }
            }
            Err(e) => {
                log::warn!("claude-code: 跳过无法解析的行: {}", e);
            }
        }
    }
    logs
}

/**
 * 从 assistant 行提取累计 usage，缺少时间时使用 fallback_ts，非用量事件返回 None
 */
fn parse_claude_line(val: &serde_json::Value, fallback_ts: &str) -> Option<Vec<TokenLog>> {
    // 只处理 type == "assistant" 的事件
    let event_type = val.get("type")?.as_str()?;
    if event_type != "assistant" {
        return None;
    }

    let message = val.get("message")?;
    let usage = message.get("usage")?;

    let input = usage
        .get("input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_create = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let model = message
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let session_id = val
        .get("sessionId")
        .or_else(|| val.get("conversationId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let legacy_id = val.get("uuid").and_then(|v| v.as_str());
    let message_id = message
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let request_id = match (
        message_id,
        val.get("requestId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty()),
        session_id.as_deref(),
    ) {
        (Some(message), Some(request), _) => Some(format!(
            "claude:request:{}",
            serde_json::json!([message, request])
        )),
        (Some(message), None, Some(session)) => Some(format!(
            "claude:session:{}",
            serde_json::json!([session, message])
        )),
        _ => legacy_id.map(|id| format!("claude:uuid:{id}")),
    };
    // 不完整代理响应不能覆盖已完成的用量，保留零值以清除旧版本误计的记录
    let incomplete = message.get("stop_reason").is_some_and(|v| v.is_null())
        && input > 0
        && output == 0
        && usage.get("cache_read_input_tokens").is_none()
        && usage.get("cache_creation_input_tokens").is_none();
    let metadata = serde_json::json!({
        "claude_legacy_id": legacy_id,
        "claude_incomplete": incomplete,
    })
    .to_string();

    let timestamp = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_ts)
        .to_string();

    // 同一响应的四个计数一起更新，避免最终记录归零后残留上一条的计数
    let request_id = request_id?;
    if input <= 0 && cache_create <= 0 && cache_read <= 0 && output <= 0 {
        return None;
    }
    Some(
        [
            (TokenType::Input, "input", input),
            (TokenType::CacheCreate, "cache_create", cache_create),
            (TokenType::CacheRead, "cache_read", cache_read),
            (TokenType::Output, "output", output),
        ]
        .into_iter()
        .map(|(token_type, suffix, count)| TokenLog {
            id: None,
            agent_name: "claude-code".into(),
            provider: "Anthropic".into(),
            model_id: model.clone(),
            token_type,
            token_count: if incomplete { 0 } else { count.max(0) },
            session_id: session_id.clone(),
            request_id: Some(format!("{request_id}-{suffix}")),
            latency_ms: None,
            is_error: false,
            metadata: Some(metadata.clone()),
            cost: None,
            timestamp: timestamp.clone(),
        })
        .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jsonl_batch(content: &str) -> AgentDataBatch {
        AgentDataBatch::JsonlIncrement {
            agent_name: "claude-code".to_string(),
            source_key: "test.jsonl".to_string(),
            path: "test.jsonl".into(),
            content: content.to_string(),
            behavior_context: None,
            token_context: None,
            initial_model: None,
            previous_offset: 0,
            next_offset: content.len() as u64,
        }
    }

    #[test]
    fn test_parse_assistant_event() {
        let line = r#"{"type":"assistant","uuid":"abc-123","conversationId":"conv-1","message":{"model":"claude-3-7-sonnet-20250219","usage":{"input_tokens":1000,"cache_creation_input_tokens":200,"cache_read_input_tokens":50,"output_tokens":500}}}"#;
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;
        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 1000);
        assert_eq!(logs[0].agent_name, "claude-code");
        assert_eq!(logs[0].provider, "Anthropic");
        assert_eq!(logs[3].token_type, TokenType::Output);
        assert_eq!(logs[3].token_count, 500);
    }

    #[test]
    fn test_skip_non_assistant() {
        let line = r#"{"type":"human","message":{"content":"hello"}}"#;
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;
        assert!(logs.is_empty());
    }

    #[test]
    fn test_skip_invalid_json() {
        let content = "invalid json\n{\"type\":\"human\"}\n";
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(content)).logs;
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch("")).logs;
        assert!(logs.is_empty());
    }
    /**
     * 生成同一响应的日志片段，用于模拟分块与重复副本
     */
    fn usage_line(uuid: &str, input: i64, output: i64, timestamp: &str) -> String {
        serde_json::json!({"type":"assistant", "uuid":uuid,"sessionId":"session",
            "timestamp":timestamp,"message":{"id":"message","model":"claude-test",
            "usage":{"input_tokens":input,"output_tokens":output}}})
        .to_string()
    }

    /**
     * 验证跨批次累计用量覆盖、归零字段和旧副本去重
     */
    #[test]
    fn test_cumulative_chunks_and_copies_update_one_response() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        let mut first: serde_json::Value =
            serde_json::from_str(&usage_line("a", 100, 2, "2026-09-24T00:00:00Z")).unwrap();
        first["message"]["usage"]["cache_read_input_tokens"] = 300.into();
        let first = first.to_string();
        let final_line = usage_line("b", 80, 50, "2026-09-24T00:00:01Z");
        for line in [&first, &final_line, &first, &final_line] {
            crate::db::queries::batch_insert_token_logs(&conn, &parse_claude_content(line))
                .unwrap();
        }
        let total: i64 = conn
            .query_row("SELECT SUM(token_count) FROM token_logs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(total, 130);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 4);
    }

    /**
     * 验证历史重算的幂等性及无源文件记录保留
     */
    #[test]
    fn test_reindex_replaces_legacy_rows_and_preserves_unavailable_history() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        let line = usage_line("a", 100, 50, "2026-09-24T00:00:00Z");
        let logs = parse_claude_content(&line);
        let mut old = logs[0].clone();
        old.request_id = Some("a-input".into());
        old.metadata = None;
        let mut unavailable = old.clone();
        unavailable.request_id = Some("unavailable-input".into());
        crate::db::queries::batch_insert_token_logs(&conn, &[old, unavailable]).unwrap();
        for _ in 0..2 {
            crate::db::queries::reindex_claude_file(
                &conn,
                &logs,
                "fixture.jsonl",
                line.len() as u64,
            )
            .unwrap();
        }
        let total: i64 = conn
            .query_row("SELECT SUM(token_count) FROM token_logs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(total, 250);
        assert_eq!(
            crate::db::queries::get_offset(
                &conn,
                &crate::db::queries::claude_parser_marker("fixture.jsonl")
            )
            .unwrap(),
            Some(0)
        );
    }

    /**
     * 验证不完整代理记录不能覆盖已完成的用量
     */
    #[test]
    fn test_incomplete_proxy_does_not_replace_completed_usage() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        let complete = usage_line("a", 100, 50, "2026-09-24T00:00:00Z");
        let mut incomplete: serde_json::Value =
            serde_json::from_str(&usage_line("b", 500, 0, "2026-09-24T00:00:02Z")).unwrap();
        incomplete["message"]["stop_reason"] = serde_json::Value::Null;
        for line in [complete, incomplete.to_string()] {
            crate::db::queries::batch_insert_token_logs(&conn, &parse_claude_content(&line))
                .unwrap();
        }
        let total: i64 = conn
            .query_row("SELECT SUM(token_count) FROM token_logs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(total, 150);
    }

    /**
     * 验证自定义配置、桌面独立目录和默认共享目录同时发现
     */
    #[test]
    fn test_desktop_and_custom_config_log_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let custom = home.join("custom[profile]");
        let cli = custom.join("projects/project/cli.jsonl");
        let shared = home.join(".claude/projects/project/shared.jsonl");
        let desktop = home.join("Library/Application Support/Claude/claude-code-sessions/account/session/.claude/projects/project/desktop.jsonl");
        for file in [&cli, &desktop, &shared] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "").unwrap();
        }
        let found: Vec<_> = log_patterns(&custom, home, true)
            .iter()
            .flat_map(|pattern| glob::glob(pattern).unwrap().flatten())
            .collect();
        assert!(found.contains(&cli));
        assert!(found.contains(&desktop));
        assert!(found.contains(&shared));
        assert_eq!(found.len(), 3);
    }

    /**
     * 验证同一响应不同日志行合并，不同请求保持独立
     */
    #[test]
    fn test_request_identity_separates_retries() {
        let mut first: serde_json::Value =
            serde_json::from_str(&usage_line("a", 100, 50, "2026-09-24T00:00:00Z")).unwrap();
        first["requestId"] = "request-a".into();
        let a = parse_claude_content(&first.to_string());
        first["uuid"] = "b".into();
        assert_eq!(
            a[0].request_id,
            parse_claude_content(&first.to_string())[0].request_id
        );
        first["requestId"] = "request-b".into();
        assert_ne!(
            a[0].request_id,
            parse_claude_content(&first.to_string())[0].request_id
        );
    }

    /**
     * 验证重算失败时旧记录和解析版本均回滚
     */
    #[test]
    fn test_reindex_failure_rolls_back_legacy_deletion_and_marker() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        let logs = parse_claude_content(&usage_line("a", 100, 50, "2026-09-24T00:00:00Z"));
        let mut legacy = logs[0].clone();
        legacy.request_id = Some("a-input".into());
        legacy.metadata = None;
        crate::db::queries::batch_insert_token_logs(&conn, &[legacy]).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_fixture BEFORE INSERT ON token_logs WHEN NEW.request_id LIKE 'claude:%' BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(
            crate::db::queries::reindex_claude_file(&conn, &logs, "fixture.jsonl", 100).is_err()
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM token_logs WHERE request_id = 'a-input'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            crate::db::queries::get_offset(
                &conn,
                &crate::db::queries::claude_parser_marker("fixture.jsonl")
            )
            .unwrap(),
            None
        );
    }
}
