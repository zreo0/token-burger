use std::path::Path;

use serde_json::Value;

use super::{AgentBehaviorEvent, AgentBehaviorKind};

const SESSION_META_SCAN_LINES: usize = 8;

/// 从 Codex JSONL 增量内容中解析行为事件
pub fn parse_events(content: &str, session_hint: &str) -> Vec<AgentBehaviorEvent> {
    parse_events_with_context(content, session_hint, None)
}

/// 从 Codex JSONL 增量内容中解析行为事件，并用完整上下文识别会话类型
pub fn parse_events_with_context(
    content: &str,
    session_hint: &str,
    context: Option<&str>,
) -> Vec<AgentBehaviorEvent> {
    if context.is_some_and(is_auto_review_session) || is_auto_review_session(content) {
        return Vec::new();
    }

    let session_id = compact_session_id(session_hint);
    let fallback_ts = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    let mut events = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(line) else {
            log::debug!("codex behavior: 跳过无法解析的行");
            continue;
        };

        if let Some(event) = parse_line(&value, &session_id, &fallback_ts) {
            events.push(event);
        }
    }

    events
}

fn is_auto_review_session(content: &str) -> bool {
    content
        .lines()
        .take(SESSION_META_SCAN_LINES)
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .any(|value| {
            value.get("type").and_then(Value::as_str) == Some("session_meta")
                && value
                    .get("payload")
                    .is_some_and(session_meta_is_auto_review)
        })
}

fn session_meta_is_auto_review(payload: &Value) -> bool {
    payload.get("thread_source").and_then(Value::as_str) == Some("subagent")
        && source_subagent_name(payload) == Some("guardian")
}

fn source_subagent_name(payload: &Value) -> Option<&str> {
    let subagent = payload.get("source")?.get("subagent")?;

    subagent
        .as_str()
        .or_else(|| subagent.get("other").and_then(Value::as_str))
}

fn parse_line(value: &Value, session_id: &str, fallback_ts: &str) -> Option<AgentBehaviorEvent> {
    let timestamp = timestamp(value, fallback_ts);
    let line_type = value.get("type").and_then(Value::as_str);
    let payload = value.get("payload")?;

    match line_type {
        Some("event_msg") => parse_event_msg(payload, session_id, timestamp),
        Some("response_item") => parse_response_item(payload, session_id, timestamp),
        _ => None,
    }
}

fn parse_event_msg(
    payload: &Value,
    session_id: &str,
    timestamp: String,
) -> Option<AgentBehaviorEvent> {
    let payload_type = payload.get("type").and_then(Value::as_str)?;
    let turn_id = turn_id(payload);

    match payload_type {
        "task_started" => Some(AgentBehaviorEvent::new(
            "codex",
            session_id,
            AgentBehaviorKind::TurnStarted,
            timestamp,
            turn_id,
            None,
            "A new turn started",
        )),
        "task_complete" => Some(AgentBehaviorEvent::new(
            "codex",
            session_id,
            AgentBehaviorKind::RunCompleted,
            timestamp,
            turn_id,
            None,
            "Codex finished the current turn",
        )),
        "turn_aborted" => Some(AgentBehaviorEvent::new(
            "codex",
            session_id,
            AgentBehaviorKind::RunAborted,
            timestamp,
            turn_id,
            None,
            abort_summary(payload),
        )),
        _ => None,
    }
}

fn parse_response_item(
    payload: &Value,
    session_id: &str,
    timestamp: String,
) -> Option<AgentBehaviorEvent> {
    let payload_type = payload.get("type").and_then(Value::as_str)?;
    let call_id = call_id(payload)?;
    let turn_id = turn_id(payload);

    match payload_type {
        "function_call"
            if payload.get("name").and_then(Value::as_str) == Some("exec_command")
                && arguments_require_escalation(payload) =>
        {
            Some(AgentBehaviorEvent::new(
                "codex",
                session_id,
                AgentBehaviorKind::PermissionRequested,
                timestamp,
                turn_id,
                Some(call_id),
                "Codex is waiting for permission",
            ))
        }
        "function_call_output" => Some(AgentBehaviorEvent::new(
            "codex",
            session_id,
            AgentBehaviorKind::PermissionResolved,
            timestamp,
            turn_id,
            Some(call_id),
            "Permission request was handled",
        )),
        _ => None,
    }
}

fn timestamp(value: &Value, fallback_ts: &str) -> String {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_ts)
        .to_string()
}

fn turn_id(payload: &Value) -> Option<String> {
    payload
        .get("turn_id")
        .or_else(|| payload.get("turnId"))
        .or_else(|| payload.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn call_id(payload: &Value) -> Option<String> {
    payload
        .get("call_id")
        .or_else(|| payload.get("callId"))
        .or_else(|| payload.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn arguments_require_escalation(payload: &Value) -> bool {
    let Some(arguments) = payload
        .get("arguments")
        .or_else(|| payload.get("args"))
        .or_else(|| payload.get("input"))
    else {
        return false;
    };

    if argument_value_requires_escalation(arguments) {
        return true;
    }

    arguments
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .as_ref()
        .is_some_and(argument_value_requires_escalation)
}

fn argument_value_requires_escalation(value: &Value) -> bool {
    value
        .get("sandbox_permissions")
        .or_else(|| value.get("sandboxPermissions"))
        .and_then(Value::as_str)
        == Some("require_escalated")
}

fn abort_summary(payload: &Value) -> String {
    payload
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
        .map(|reason| format!("Codex stopped the current turn: {reason}"))
        .unwrap_or_else(|| "Codex stopped the current turn".to_string())
}

fn compact_session_id(session_hint: &str) -> String {
    let path = Path::new(session_hint);
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(session_hint)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::AgentBehaviorKind;

    #[test]
    fn parses_codex_turn_and_completion_events() {
        let content = r#"{"timestamp":"2026-06-01T10:00:00Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1"}}
{"timestamp":"2026-06-01T10:01:00Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;

        let events = parse_events(content, "/tmp/session.jsonl");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, AgentBehaviorKind::TurnStarted);
        assert_eq!(events[0].session_id, "session.jsonl");
        assert_eq!(events[1].kind, AgentBehaviorKind::RunCompleted);
        assert_eq!(events[1].turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn parses_codex_permission_request_and_resolved() {
        let content = r#"{"timestamp":"2026-06-01T10:00:00Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call-1","turn_id":"turn-1","arguments":"{\"sandbox_permissions\":\"require_escalated\",\"cmd\":\"secret\"}"}}
{"timestamp":"2026-06-01T10:00:10Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call-1","turn_id":"turn-1","output":"hidden"}}"#;

        let events = parse_events(content, "session");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, AgentBehaviorKind::PermissionRequested);
        assert_eq!(events[0].call_id.as_deref(), Some("call-1"));
        assert_eq!(events[0].summary, "Codex is waiting for permission");
        assert!(!serde_json::to_string(&events[0])
            .unwrap()
            .contains("secret"));
        assert_eq!(events[1].kind, AgentBehaviorKind::PermissionResolved);
    }

    #[test]
    fn skips_auto_review_guardian_session_events() {
        let content = r#"{"timestamp":"2026-06-01T10:00:00Z","type":"session_meta","payload":{"thread_source":"subagent","source":{"subagent":{"other":"guardian"}}}}
{"timestamp":"2026-06-01T10:00:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1"}}
{"timestamp":"2026-06-01T10:00:02Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;

        let events = parse_events(content, "guardian-session");

        assert!(events.is_empty());
    }

    #[test]
    fn skips_auto_review_events_when_meta_is_only_in_context() {
        let context = r#"{"timestamp":"2026-06-01T10:00:00Z","type":"session_meta","payload":{"thread_source":"subagent","source":{"subagent":"guardian"}}}"#;
        let content = r#"{"timestamp":"2026-06-01T10:00:02Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;

        let events = parse_events_with_context(content, "guardian-session", Some(context));

        assert!(events.is_empty());
    }

    #[test]
    fn skips_invalid_json_and_non_escalated_exec() {
        let content = r#"nope
{"timestamp":"2026-06-01T10:00:00Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call-1","arguments":{"sandbox_permissions":"use_default"}}}"#;

        let events = parse_events(content, "session");

        assert!(events.is_empty());
    }
}
