use serde_json::Value;

use super::{AgentBehaviorEvent, AgentBehaviorKind};

/// OpenCode message 表的最小行为解析输入
#[derive(Debug, Clone)]
pub struct OpenCodeMessageRow {
    pub id: String,
    pub session_id: Option<String>,
    pub data: String,
    pub time_created: i64,
}

/// 从 OpenCode message row 中解析第一版完成事件
pub fn parse_message_row(row: &OpenCodeMessageRow) -> Option<AgentBehaviorEvent> {
    let data = serde_json::from_str::<Value>(&row.data).ok()?;
    if data.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    if data.get("finish").and_then(Value::as_str) != Some("stop") {
        return None;
    }

    let session_id = row
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(&row.id)
        .to_string();
    let timestamp = chrono::DateTime::from_timestamp(row.time_created / 1000, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        })
        .unwrap_or_else(|| {
            chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        });

    Some(AgentBehaviorEvent::new(
        "opencode",
        session_id,
        AgentBehaviorKind::RunCompleted,
        timestamp,
        Some(row.id.clone()),
        None,
        "OpenCode finished the current turn",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::AgentBehaviorKind;

    #[test]
    fn parses_opencode_finish_stop() {
        let row = OpenCodeMessageRow {
            id: "msg-1".to_string(),
            session_id: Some("session-1".to_string()),
            data: r#"{"role":"assistant","finish":"stop","tokens":{"input":1}}"#.to_string(),
            time_created: 1_780_000_000_000,
        };

        let event = parse_message_row(&row).unwrap();

        assert_eq!(event.agent_name, "opencode");
        assert_eq!(event.kind, AgentBehaviorKind::RunCompleted);
        assert_eq!(event.session_id, "session-1");
    }

    #[test]
    fn skips_opencode_non_stop_or_non_assistant() {
        let user_row = OpenCodeMessageRow {
            id: "msg-1".to_string(),
            session_id: None,
            data: r#"{"role":"user","finish":"stop"}"#.to_string(),
            time_created: 1_780_000_000_000,
        };
        let running_row = OpenCodeMessageRow {
            id: "msg-2".to_string(),
            session_id: None,
            data: r#"{"role":"assistant","finish":"tool"}"#.to_string(),
            time_created: 1_780_000_000_000,
        };

        assert!(parse_message_row(&user_row).is_none());
        assert!(parse_message_row(&running_row).is_none());
    }
}
