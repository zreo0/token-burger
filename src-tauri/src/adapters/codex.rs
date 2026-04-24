use super::{AgentAdapter, DataSource, TokenLog, TokenType};

pub(crate) const DEFAULT_CODEX_MODEL: &str = "codex";

pub struct CodexAdapter;

pub(crate) struct CodexParseResult {
    pub logs: Vec<TokenLog>,
    pub final_model: String,
}

impl AgentAdapter for CodexAdapter {
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

    fn parse_content(&self, content: &str) -> Vec<TokenLog> {
        parse_content_with_model(content, DEFAULT_CODEX_MODEL).logs
    }
}

pub(crate) fn parse_content_with_model(content: &str, initial_model: &str) -> CodexParseResult {
    let mut logs = Vec::new();
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
                if let Some(model) = parse_turn_model(&val) {
                    current_model = model.to_string();
                    continue;
                }

                if let Some(parsed) = parse_codex_line(&val, &now, &current_model) {
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
    }
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

fn parse_codex_line(
    val: &serde_json::Value,
    fallback_ts: &str,
    current_model: &str,
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

    let input = usage
        .get("input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cached_input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        + usage
            .get("reasoning_output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

    if input == 0 && cache_read == 0 && output == 0 {
        return None;
    }

    let timestamp = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_ts)
        .to_string();
    let request_id = format!("codex-{}", timestamp);
    let model = current_model.to_string();

    let make_log = |token_type: TokenType, token_count: i64, suffix: &str| TokenLog {
        id: None,
        agent_name: "codex".into(),
        provider: "OpenAI".into(),
        model_id: model.clone(),
        token_type,
        token_count,
        session_id: None,
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

    #[test]
    fn test_parse_codex_token_count_usage() {
        let line = r#"{"type":"turn_context","payload":{"model":"gpt-5.5"}}
{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7851,"cached_input_tokens":0,"output_tokens":101,"reasoning_output_tokens":64,"total_tokens":7952},"last_token_usage":{"input_tokens":8079,"cached_input_tokens":7808,"output_tokens":48,"reasoning_output_tokens":64,"total_tokens":8191},"model_context_window":258400}}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.parse_content(line);

        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 8079);
        assert_eq!(logs[0].provider, "OpenAI");
        assert_eq!(logs[0].model_id, "gpt-5.5");
        assert_eq!(logs[1].token_type, TokenType::CacheRead);
        assert_eq!(logs[1].token_count, 7808);
        assert_eq!(logs[2].token_type, TokenType::Output);
        assert_eq!(logs[2].token_count, 112);
        assert_eq!(
            logs[2].request_id.as_deref(),
            Some("codex-2026-01-14T07:23:24.629Z-output")
        );
    }

    #[test]
    fn test_skip_non_token_count_event() {
        let line = r#"{"type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.parse_content(line);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_parse_codex_token_count_without_turn_context_falls_back_to_codex() {
        let line = r#"{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1}}}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.parse_content(line);

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
        let logs = adapter.parse_content(line);

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].model_id, "gpt-5.4");
        assert_eq!(logs[1].model_id, "gpt-5.5");
    }

    #[test]
    fn test_skip_null_last_token_usage() {
        let line = r#"{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        let adapter = CodexAdapter;
        let logs = adapter.parse_content(line);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = CodexAdapter;
        assert!(adapter.parse_content("").is_empty());
    }
}
