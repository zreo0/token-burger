pub mod codex;
pub mod dispatcher;
pub mod opencode;
pub mod tip_window;

/// Agent 行为事件类型
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBehaviorKind {
    TurnStarted,
    PermissionRequested,
    PermissionResolved,
    RunCompleted,
    RunAborted,
    ToolError,
}

/// Agent 行为事件，表示从同一批 Agent 数据中解析出的短生命周期信号
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentBehaviorEvent {
    pub key: String,
    pub agent_name: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub call_id: Option<String>,
    pub kind: AgentBehaviorKind,
    pub timestamp: String,
    pub title: String,
    pub summary: String,
}

impl AgentBehaviorEvent {
    /// 创建行为事件，并生成稳定去重 key
    pub fn new(
        agent_name: impl Into<String>,
        session_id: impl Into<String>,
        kind: AgentBehaviorKind,
        timestamp: impl Into<String>,
        turn_id: Option<String>,
        call_id: Option<String>,
        summary: impl Into<String>,
    ) -> Self {
        let agent_name = agent_name.into();
        let session_id = session_id.into();
        let timestamp = timestamp.into();
        let summary = summary.into();
        let title = event_title(&kind).to_string();
        let key = behavior_key(
            &agent_name,
            &session_id,
            &kind,
            turn_id.as_deref(),
            call_id.as_deref(),
        );

        Self {
            key,
            agent_name,
            session_id,
            turn_id,
            call_id,
            kind,
            timestamp,
            title,
            summary,
        }
    }
}

/// 当前展示给前端的轻量提示
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BehaviorTip {
    pub key: String,
    pub agent_name: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub call_id: Option<String>,
    pub kind: AgentBehaviorKind,
    pub timestamp: String,
    pub title: String,
    pub summary: String,
    pub auto_hide_ms: Option<u64>,
}

impl From<AgentBehaviorEvent> for BehaviorTip {
    fn from(event: AgentBehaviorEvent) -> Self {
        let auto_hide_ms = match event.kind {
            AgentBehaviorKind::RunCompleted | AgentBehaviorKind::RunAborted => Some(5_000),
            AgentBehaviorKind::ToolError => Some(8_000),
            _ => None,
        };

        Self {
            key: event.key,
            agent_name: event.agent_name,
            session_id: event.session_id,
            turn_id: event.turn_id,
            call_id: event.call_id,
            kind: event.kind,
            timestamp: event.timestamp,
            title: event.title,
            summary: event.summary,
            auto_hide_ms,
        }
    }
}

/// 生成稳定行为事件 key
pub fn behavior_key(
    agent_name: &str,
    session_id: &str,
    kind: &AgentBehaviorKind,
    turn_id: Option<&str>,
    call_id: Option<&str>,
) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        agent_name,
        session_id,
        turn_id.unwrap_or("-"),
        call_id.unwrap_or("-"),
        kind_key(kind)
    )
}

fn kind_key(kind: &AgentBehaviorKind) -> &'static str {
    match kind {
        AgentBehaviorKind::TurnStarted => "turn_started",
        AgentBehaviorKind::PermissionRequested => "permission_requested",
        AgentBehaviorKind::PermissionResolved => "permission_resolved",
        AgentBehaviorKind::RunCompleted => "run_completed",
        AgentBehaviorKind::RunAborted => "run_aborted",
        AgentBehaviorKind::ToolError => "tool_error",
    }
}

fn event_title(kind: &AgentBehaviorKind) -> &'static str {
    match kind {
        AgentBehaviorKind::TurnStarted => "New turn started",
        AgentBehaviorKind::PermissionRequested => "Permission needed",
        AgentBehaviorKind::PermissionResolved => "Permission handled",
        AgentBehaviorKind::RunCompleted => "Run completed",
        AgentBehaviorKind::RunAborted => "Run stopped",
        AgentBehaviorKind::ToolError => "Tool error",
    }
}
