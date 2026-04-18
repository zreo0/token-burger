use super::{AgentAdapter, DataSource, TokenLog, TokenType};

pub struct ClaudeCodeAdapter;

impl AgentAdapter for ClaudeCodeAdapter {
    fn agent_name(&self) -> &str {
        "claude-code"
    }

    fn data_source(&self) -> DataSource {
        let home = dirs::home_dir().unwrap_or_default();
        let base = home.join(".claude").join("projects");
        DataSource::Jsonl { paths: vec![base] }
    }

    fn log_paths(&self) -> Vec<String> {
        let home = dirs::home_dir().unwrap_or_default();
        vec![format!("{}/.claude/projects/**/*.jsonl", home.display())]
    }

    fn parse_content(&self, content: &str) -> Vec<TokenLog> {
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
}

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

    let request_id = val
        .get("uuid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let timestamp = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_ts)
        .to_string();

    let base = |tt: TokenType, count: i64| TokenLog {
        id: None,
        agent_name: "claude-code".into(),
        provider: "Anthropic".into(),
        model_id: model.clone(),
        token_type: tt,
        token_count: count,
        session_id: session_id.clone(),
        request_id: request_id.as_ref().map(|r| format!("{}-{}", r, "input")),
        latency_ms: None,
        is_error: false,
        metadata: None,
        cost: None,
        timestamp: timestamp.clone(),
    };

    let mut logs = Vec::new();
    if input > 0 {
        let mut log = base(TokenType::Input, input);
        log.request_id = request_id.as_ref().map(|r| format!("{}-input", r));
        logs.push(log);
    }
    if cache_create > 0 {
        let mut log = base(TokenType::CacheCreate, cache_create);
        log.request_id = request_id.as_ref().map(|r| format!("{}-cache_create", r));
        logs.push(log);
    }
    if cache_read > 0 {
        let mut log = base(TokenType::CacheRead, cache_read);
        log.request_id = request_id.as_ref().map(|r| format!("{}-cache_read", r));
        logs.push(log);
    }
    if output > 0 {
        let mut log = base(TokenType::Output, output);
        log.request_id = request_id.as_ref().map(|r| format!("{}-output", r));
        logs.push(log);
    }

    if logs.is_empty() {
        None
    } else {
        Some(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_assistant_event() {
        let line = r#"{"type":"assistant","uuid":"abc-123","conversationId":"conv-1","message":{"model":"claude-3-7-sonnet-20250219","usage":{"input_tokens":1000,"cache_creation_input_tokens":200,"cache_read_input_tokens":50,"output_tokens":500}}}"#;
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.parse_content(line);
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
        let logs = adapter.parse_content(line);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_skip_invalid_json() {
        let content = "invalid json\n{\"type\":\"human\"}\n";
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.parse_content(content);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = ClaudeCodeAdapter;
        let logs = adapter.parse_content("");
        assert!(logs.is_empty());
    }
}
