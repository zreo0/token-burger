use super::{AgentAdapter, DataSource, TokenLog, TokenType};

pub struct GeminiCliAdapter;

impl AgentAdapter for GeminiCliAdapter {
    fn agent_name(&self) -> &str {
        "gemini-cli"
    }

    fn data_source(&self) -> DataSource {
        let home = dirs::home_dir().unwrap_or_default();
        DataSource::Json {
            paths: vec![home.join(".gemini").join("tmp")],
        }
    }

    fn log_paths(&self) -> Vec<String> {
        let home = dirs::home_dir().unwrap_or_default();
        vec![format!("{}/.gemini/tmp/*/chats/*.json", home.display())]
    }

    fn parse_content(&self, content: &str) -> Vec<TokenLog> {
        let now = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();

        match serde_json::from_str::<serde_json::Value>(content) {
            Ok(val) => parse_gemini_json(&val, &now),
            Err(e) => {
                log::warn!("gemini-cli: JSON 解析失败: {}", e);
                Vec::new()
            }
        }
    }
}

fn parse_gemini_json(val: &serde_json::Value, fallback_ts: &str) -> Vec<TokenLog> {
    let mut logs = Vec::new();

    let messages = match val.get("messages").and_then(|v| v.as_array()) {
        Some(m) => m,
        None => return logs,
    };

    for (idx, msg) in messages.iter().enumerate() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role != "model" && role != "assistant" {
            continue;
        }

        let tokens = match msg.get("tokens").or_else(|| msg.get("usageMetadata")) {
            Some(t) => t,
            None => continue,
        };

        let input = tokens
            .get("promptTokenCount")
            .or_else(|| tokens.get("input"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let output = tokens
            .get("candidatesTokenCount")
            .or_else(|| tokens.get("output"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        if input == 0 && output == 0 {
            continue;
        }

        let model = val
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gemini-2.5-pro")
            .to_string();

        let session_id = val
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let req_id = format!(
            "gemini-{}-{}",
            session_id.as_deref().unwrap_or("unknown"),
            idx
        );

        if input > 0 {
            logs.push(TokenLog {
                id: None,
                agent_name: "gemini-cli".into(),
                provider: "Google".into(),
                model_id: model.clone(),
                token_type: TokenType::Input,
                token_count: input,
                session_id: session_id.clone(),
                request_id: Some(format!("{}-input", req_id)),
                latency_ms: None,
                is_error: false,
                metadata: None,
                cost: None,
                timestamp: fallback_ts.to_string(),
            });
        }
        if output > 0 {
            logs.push(TokenLog {
                id: None,
                agent_name: "gemini-cli".into(),
                provider: "Google".into(),
                model_id: model,
                token_type: TokenType::Output,
                token_count: output,
                session_id,
                request_id: Some(format!("{}-output", req_id)),
                latency_ms: None,
                is_error: false,
                metadata: None,
                cost: None,
                timestamp: fallback_ts.to_string(),
            });
        }
    }
    logs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_chat() {
        let json = r#"{"id":"chat-1","model":"gemini-2.5-pro","messages":[{"role":"user","content":"hi"},{"role":"model","tokens":{"promptTokenCount":100,"candidatesTokenCount":50}}]}"#;
        let adapter = GeminiCliAdapter;
        let logs = adapter.parse_content(json);
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].token_type, TokenType::Input);
        assert_eq!(logs[0].token_count, 100);
        assert_eq!(logs[0].provider, "Google");
    }

    #[test]
    fn test_skip_user_messages() {
        let json = r#"{"messages":[{"role":"user","tokens":{"promptTokenCount":100}}]}"#;
        let adapter = GeminiCliAdapter;
        let logs = adapter.parse_content(json);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let adapter = GeminiCliAdapter;
        assert!(adapter.parse_content("").is_empty());
    }
}
