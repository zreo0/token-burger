use super::{
    AgentDataBatch, AgentSource, BehaviorExtractor, DataSource, TokenExtraction, TokenExtractor,
    TokenLog, TokenType,
};

pub(crate) const DEFAULT_CODEX_MODEL: &str = "codex";

pub struct CodexAdapter;

pub(crate) struct CodexParseResult {
    pub logs: Vec<TokenLog>,
    pub final_model: String,
    pub replay_pending: bool,
}

impl AgentSource for CodexAdapter {
    fn agent_name(&self) -> &str {
        "codex"
    }

    fn data_source(&self) -> DataSource {
        let home = dirs::home_dir().unwrap_or_default();
        DataSource::Jsonl {
            paths: vec![home.join(".codex").join("sessions")],
        }
    }

    fn log_paths(&self) -> Vec<String> {
        let home = dirs::home_dir().unwrap_or_default();
        // Codex 按日期分子目录存储：~/.codex/sessions/2026/04/18/*.jsonl
        vec![format!("{}/.codex/sessions/**/*.jsonl", home.display())]
    }
}

impl TokenExtractor for CodexAdapter {
    fn extract_tokens(&self, batch: &AgentDataBatch) -> TokenExtraction {
        let Some(content) = batch.token_content() else {
            return TokenExtraction::default();
        };
        let initial_model = match batch {
            AgentDataBatch::JsonlIncrement { initial_model, .. } => {
                initial_model.as_deref().unwrap_or(DEFAULT_CODEX_MODEL)
            }
            _ => DEFAULT_CODEX_MODEL,
        };
        // 增量可能不含 session_meta，复用 Watcher 已读取的文件头，不依赖项目目录
        let context = match batch {
            AgentDataBatch::JsonlIncrement {
                behavior_context, ..
            } => behavior_context.as_deref().unwrap_or(content),
            _ => content,
        };
        let session_id = parse_session_id(context).unwrap_or_else(|| {
            // 旧日志缺少元数据时使用 rollout 文件名，目录移动不改变标识
            std::path::Path::new(batch.source_key())
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
        let parsed = parse_content_with_model(content, initial_model, &session_id);

        TokenExtraction {
            logs: parsed.logs,
            // 重放结束前不缓存模型，确保下一次增量继续带上完整文件上下文
            final_model: (!parsed.replay_pending).then_some(parsed.final_model),
        }
    }
}

impl BehaviorExtractor for CodexAdapter {
    fn extract_behavior(&self, batch: &AgentDataBatch) -> Vec<crate::behavior::AgentBehaviorEvent> {
        match batch {
            AgentDataBatch::JsonlIncrement {
                content,
                behavior_context,
                token_context,
                ..
            } => crate::behavior::codex::parse_events_with_context(
                content,
                batch.source_key(),
                behavior_context.as_deref().or(token_context.as_deref()),
            ),
            _ => batch
                .behavior_content()
                .map(|content| crate::behavior::codex::parse_events(content, batch.source_key()))
                .unwrap_or_default(),
        }
    }
}

/**
 * 从已读取的文本返回首个会话元数据的 ID，避免采用子代理回放中的父会话 ID
 */
fn parse_session_id(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("type")?.as_str()? == "session_meta").then_some(value)
        })?
        .get("payload")?
        .get("id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/**
 * 按稳定会话 ID 解析用量文本和初始模型，返回日志、最终模型及回放状态
 */
pub(crate) fn parse_content_with_model(
    content: &str,
    initial_model: &str,
    session_id: &str,
) -> CodexParseResult {
    let mut logs = Vec::new();
    let mut saw_session_meta = false;
    let mut replay_pending = false;
    let mut current_model = if initial_model.is_empty() {
        DEFAULT_CODEX_MODEL.to_string()
    } else {
        initial_model.to_string()
    };
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
                if !saw_session_meta {
                    if let Some(is_thread_spawn) = parse_thread_spawn_session(&val) {
                        saw_session_meta = true;
                        replay_pending = is_thread_spawn;
                    }
                }

                if let Some(model) = parse_turn_model(&val) {
                    current_model = model.to_string();
                    continue;
                }

                if replay_pending {
                    if val.get("type").and_then(|v| v.as_str())
                        == Some("inter_agent_communication_metadata")
                    {
                        replay_pending = false;
                    }
                    continue;
                }

                if let Some(parsed) = parse_codex_line(&val, &now, &current_model, session_id) {
                    logs.extend(parsed);
                }
            }
            Err(e) => {
                log::warn!("codex: 跳过无法解析的行: {}", e);
            }
        }
    }

    CodexParseResult {
        logs,
        final_model: current_model,
        replay_pending,
    }
}

/// 识别首个会话元数据是否来自 thread_spawn 子代理
fn parse_thread_spawn_session(val: &serde_json::Value) -> Option<bool> {
    if val.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return None;
    }

    Some(
        val.get("payload")
            .and_then(|payload| payload.get("source"))
            .and_then(|source| source.get("subagent"))
            .and_then(|subagent| subagent.get("thread_spawn"))
            .is_some(),
    )
}

fn parse_turn_model(val: &serde_json::Value) -> Option<&str> {
    if val.get("type").and_then(|v| v.as_str()) != Some("turn_context") {
        return None;
    }

    val.get("payload")
        .and_then(|payload| payload.get("model"))
        .and_then(|v| v.as_str())
        .filter(|model| !model.is_empty())
}

/**
 * 将单个用量事件转换为会话内可去重的 token 日志，缺少时间时使用展示用回退时间
 */
fn parse_codex_line(
    val: &serde_json::Value,
    fallback_ts: &str,
    current_model: &str,
    session_id: &str,
) -> Option<Vec<TokenLog>> {
    // 仅处理 event_msg/token_count 事件，且使用单次请求的 last_token_usage。
    if val.get("type").and_then(|v| v.as_str()) != Some("event_msg") {
        return None;
    }

    let payload = val.get("payload")?;
    if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
        return None;
    }

    let usage = payload
        .get("info")
        .and_then(|info| info.get("last_token_usage"))?;

    let total_input = usage
        .get("input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cached_input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let input = total_input.saturating_sub(cache_read);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let reasoning_output_tokens = usage
        .get("reasoning_output_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output = if total_tokens == total_input + output_tokens + reasoning_output_tokens {
        output_tokens + reasoning_output_tokens
    } else {
        output_tokens
    };

    if input == 0 && cache_read == 0 && output == 0 {
        return None;
    }

    let timestamp = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_ts)
        .to_string();
    // JSON 数组避免字段拼接歧义，用量快照区分同毫秒事件；不使用路径、模型或读取时间
    // 累计用量还能区分连续两次消耗相同的请求，重复上报同一快照则保持同一标识
    let event_key = serde_json::json!([
        session_id,
        val.get("timestamp"),
        payload
            .get("info")
            .and_then(|info| info.get("total_token_usage")),
        usage,
    ]);
    let request_id = format!("codex:{event_key}");
    let model = current_model.to_string();

    let make_log = |token_type: TokenType, token_count: i64, suffix: &str| TokenLog {
        id: None,
        agent_name: "codex".into(),
        provider: "OpenAI".into(),
        model_id: model.clone(),
        token_type,
        token_count,
        session_id: Some(session_id.to_string()),
        request_id: Some(format!("{}-{}", request_id, suffix)),
        latency_ms: None,
        is_error: false,
        metadata: None,
        cost: None,
        timestamp: timestamp.clone(),
    };

    let mut logs = Vec::new();
    if input > 0 {
        logs.push(make_log(TokenType::Input, input, "input"));
    }
    if cache_read > 0 {
        logs.push(make_log(TokenType::CacheRead, cache_read, "cache_read"));
    }
    if output > 0 {
        logs.push(make_log(TokenType::Output, output, "output"));
    }

    Some(logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * 按会话 ID 和累计用量生成同一时间戳的日志，每个事件消耗 10 个输入 token
     */
    fn session_content(session_id: &str, total: i64) -> String {
        format!(
            "{}\n{}\n",
            serde_json::json!({"type": "session_meta", "payload": {"id": session_id, "cwd": "/old/project"}}),
            serde_json::json!({
                "timestamp": "2026-10-13T10:00:00.123Z",
                "type": "event_msg",
                "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": total, "total_tokens": total},
                    "last_token_usage": {"input_tokens": 10, "total_tokens": 10}
                }}
            })
        )
    }

    /**
     * 创建独立内存数据库，用于验证解析结果经过真实写入路径后的去重和汇总
     */
    fn usage_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        conn
    }

    /**
     * 验证相同时刻的并发会话分别入库，项目目录及日志路径变化后重读不会增加总量
     */
    #[test]
    fn concurrent_sessions_survive_dedup_and_project_moves() {
        let conn = usage_db();
        for session_id in ["session-a", "session-b"] {
            let content = session_content(session_id, 10);
            let logs = CodexAdapter.extract_tokens(&jsonl_batch(&content)).logs;
            crate::db::queries::batch_insert_token_logs(&conn, &logs).unwrap();

            let mut moved = jsonl_batch(&content.replace("/old/project", "/new/project"));
            if let AgentDataBatch::JsonlIncrement {
                source_key, path, ..
            } = &mut moved
            {
                *source_key = "/new/project/renamed.jsonl".into();
                *path = source_key.clone().into();
            }
            let reread = CodexAdapter.extract_tokens(&moved).logs;
            assert_eq!(logs[0].request_id, reread[0].request_id);
            assert_eq!(reread[0].session_id.as_deref(), Some(session_id));
            crate::db::queries::batch_insert_token_logs(&conn, &reread).unwrap();
        }
        let result: (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(token_count) FROM token_logs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(result, (2, 20));
    }

    /**
     * 验证同会话同毫秒的不同请求均保留，重复快照和完整重读均不增加统计
     */
    #[test]
    fn same_timestamp_requests_use_usage_snapshots() {
        let conn = usage_db();
        let content = format!(
            "{}{}{}",
            session_content("a", 10),
            session_content("a", 20),
            session_content("a", 20)
        );
        let logs = CodexAdapter.extract_tokens(&jsonl_batch(&content)).logs;
        assert_ne!(logs[0].request_id, logs[1].request_id);
        assert_eq!(logs[1].request_id, logs[2].request_id);
        for _ in 0..2 {
            crate::db::queries::batch_insert_token_logs(&conn, &logs).unwrap();
        }
        let total: i64 = conn
            .query_row("SELECT SUM(token_count) FROM token_logs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(total, 20);
    }

    /**
     * 验证升级重读不新增或修改旧记录，之后同一时刻的两个会话仍能独立入库
     */
    #[test]
    fn legacy_usage_is_preserved_without_blocking_new_concurrent_events() {
        let conn = usage_db();
        let old_content = session_content("a", 10);
        let logs = CodexAdapter.extract_tokens(&jsonl_batch(&old_content)).logs;
        let mut legacy = logs[0].clone();
        legacy.request_id = Some(format!("codex-{}-input", legacy.timestamp));
        legacy.session_id = None;
        legacy.token_count = 99;
        crate::db::queries::batch_insert_token_logs(&conn, &[legacy]).unwrap();
        crate::db::queries::batch_insert_token_logs(&conn, &logs).unwrap();

        for id in ["a", "b"] {
            let content = session_content(id, 20).replace("10:00:00.123", "10:00:01.123");
            let logs = CodexAdapter.extract_tokens(&jsonl_batch(&content)).logs;
            crate::db::queries::batch_insert_token_logs(&conn, &logs).unwrap();
        }
        let result: (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(token_count), SUM(session_id IS NULL) FROM token_logs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(result, (3, 119, 1));
    }

    /**
     * 验证缺失时间戳时重读标识稳定，展示时间变化不影响去重
     */
    #[test]
    fn missing_timestamp_does_not_use_read_time_as_identity() {
        let value = serde_json::json!({"type": "event_msg", "payload": {
            "type": "token_count", "info": {"last_token_usage": {"input_tokens": 10}}
        }});
        let first = parse_codex_line(&value, "2026-10-13T10:00:00Z", "gpt-5.5", "a").unwrap();
        let second = parse_codex_line(&value, "2026-10-13T11:00:00Z", "gpt-5.5", "a").unwrap();
        assert_eq!(first[0].request_id, second[0].request_id);
    }

    fn jsonl_batch(content: &str) -> AgentDataBatch {
        AgentDataBatch::JsonlIncrement {
            agent_name: "codex".to_string(),
            source_key: "session.jsonl".to_string(),
            path: "session.jsonl".into(),
            content: content.to_string(),
            behavior_context: None,
            token_context: None,
            initial_model: None,
            previous_offset: 0,
            next_offset: content.len() as u64,
        }
    }

    #[test]
    fn test_parse_codex_token_count_usage() {
        let line = r#"{"type":"turn_context","payload":{"model":"gpt-5.5"}}
{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7851,"cached_input_tokens":0,"output_tokens":101,"reasoning_output_tokens":64,"total_tokens":7952},"last_token_usage":{"input_tokens":8079,"cached_input_tokens":7808,"output_tokens":48,"reasoning_output_tokens":64,"total_tokens":8191},"model_context_window":258400}}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;

        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 271);
        assert_eq!(logs[0].provider, "OpenAI");
        assert_eq!(logs[0].model_id, "gpt-5.5");
        assert_eq!(logs[1].token_type, TokenType::CacheRead);
        assert_eq!(logs[1].token_count, 7808);
        assert_eq!(logs[2].token_type, TokenType::Output);
        assert_eq!(logs[2].token_count, 112);
        assert!(logs[2].request_id.as_deref().unwrap().starts_with("codex:"));
        assert_eq!(logs[2].session_id.as_deref(), Some("session.jsonl"));
    }

    #[test]
    fn test_parse_codex_token_count_when_output_includes_reasoning() {
        let line = r#"{"type":"turn_context","payload":{"model":"gpt-5.5"}}
{"timestamp":"2026-04-25T17:09:13.596Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":60531,"cached_input_tokens":59776,"output_tokens":514,"reasoning_output_tokens":63,"total_tokens":61045}}}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;

        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 755);
        assert_eq!(logs[1].token_type, TokenType::CacheRead);
        assert_eq!(logs[1].token_count, 59776);
        assert_eq!(logs[2].token_type, TokenType::Output);
        assert_eq!(logs[2].token_count, 514);
    }

    #[test]
    fn test_skip_non_token_count_event() {
        let line = r#"{"type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;
        assert!(logs.is_empty());
    }

    #[test]
    fn test_parse_codex_token_count_without_turn_context_falls_back_to_codex() {
        let line = r#"{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1}}}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].model_id, "codex");
    }

    #[test]
    fn test_parse_codex_model_switches_by_turn_context() {
        let line = r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1}}}}
{"type":"turn_context","payload":{"model":"gpt-5.5"}}
{"timestamp":"2026-01-14T07:24:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":2}}}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].model_id, "gpt-5.4");
        assert_eq!(logs[1].model_id, "gpt-5.5");
    }

    #[test]
    fn test_thread_spawn_skips_replayed_usage() {
        let content = r#"{"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}
{"timestamp":"2026-08-12T09:00:17.609Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":999}}}}
{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"inter_agent_communication_metadata"}
{"timestamp":"2026-08-12T09:00:30.731Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":7}}}}"#;
        let adapter = CodexAdapter;
        let extraction = adapter.extract_tokens(&jsonl_batch(content));

        assert_eq!(extraction.logs.len(), 1);
        assert_eq!(extraction.logs[0].token_count, 7);
        assert_eq!(extraction.logs[0].model_id, "gpt-5.6-sol");
        assert_eq!(extraction.final_model.as_deref(), Some("gpt-5.6-sol"));
    }

    #[test]
    fn test_thread_spawn_without_live_boundary_keeps_model_cache_empty() {
        let content = r#"{"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}
{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"timestamp":"2026-08-12T09:00:17.609Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":999}}}}"#;
        let adapter = CodexAdapter;
        let extraction = adapter.extract_tokens(&jsonl_batch(content));

        assert!(extraction.logs.is_empty());
        assert!(extraction.final_model.is_none());
    }

    #[test]
    fn test_guardian_subagent_counts_usage_without_live_boundary() {
        let content = r#"{"type":"session_meta","payload":{"source":{"subagent":{"other":"guardian"}}}}
{"type":"turn_context","payload":{"model":"codex-auto-review"}}
{"timestamp":"2026-08-12T09:00:17.609Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":5}}}}"#;
        let adapter = CodexAdapter;
        let extraction = adapter.extract_tokens(&jsonl_batch(content));

        assert_eq!(extraction.logs.len(), 1);
        assert_eq!(extraction.logs[0].token_count, 5);
        assert_eq!(extraction.logs[0].model_id, "codex-auto-review");
        assert_eq!(extraction.final_model.as_deref(), Some("codex-auto-review"));
    }

    #[test]
    fn test_skip_null_last_token_usage() {
        let line = r#"{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.extract_tokens(&jsonl_batch(line)).logs;
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = CodexAdapter;
        assert!(adapter.extract_tokens(&jsonl_batch("")).logs.is_empty());
    }
}
