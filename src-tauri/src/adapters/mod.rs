pub mod claude_code;
pub mod codex;
pub mod gemini_cli;
pub mod opencode;

use std::path::{Path, PathBuf};

use crate::behavior::AgentBehaviorEvent;

/// Agent 日志数据源类型
#[derive(Debug, Clone)]
pub enum DataSource {
    /// JSONL 文件，支持 offset 增量读取
    Jsonl { paths: Vec<PathBuf> },
    /// JSON 文件，全量解析 + mtime 缓存
    Json { paths: Vec<PathBuf> },
    /// 外部 SQLite 数据库，定时查询
    Sqlite { db_path: PathBuf },
}

/// Token 类型枚举，与前端 TokenLog.token_type 联合类型一致
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    Input,
    CacheCreate,
    CacheRead,
    Output,
}

/// 统一的 Token 日志结构（与 SQLite token_logs 表对应）
/// timestamp 格式：RFC3339（如 2026-04-18T10:00:00+08:00）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenLog {
    pub id: Option<i64>,
    pub agent_name: String,
    pub provider: String,
    pub model_id: String,
    pub token_type: TokenType,
    pub token_count: i64,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub latency_ms: Option<i64>,
    pub is_error: bool,
    pub metadata: Option<String>,
    /// Agent 自带的花费（美元），None 表示需要前端用价格表计算
    pub cost: Option<f64>,
    pub timestamp: String,
}

/// 外部 SQLite 查询得到的最小 row
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteMessageRow {
    pub id: String,
    pub session_id: Option<String>,
    pub data: String,
    pub time_created: i64,
    pub watermark: i64,
}

/// 外部 SQLite source 的一次增量查询结果
#[derive(Debug, Clone, Default)]
pub struct SqliteRowBatch {
    pub rows: Vec<SqliteMessageRow>,
    pub high_watermark: Option<u64>,
}

/// Watcher 已读取的一批 Agent 数据
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AgentDataBatch {
    JsonlIncrement {
        agent_name: String,
        source_key: String,
        path: PathBuf,
        content: String,
        token_context: Option<String>,
        initial_model: Option<String>,
        previous_offset: u64,
        next_offset: u64,
    },
    JsonDocument {
        agent_name: String,
        source_key: String,
        path: PathBuf,
        content: String,
        mtime: u64,
    },
    SqliteRows {
        agent_name: String,
        source_key: String,
        db_path: PathBuf,
        rows: Vec<SqliteMessageRow>,
        previous_watermark: Option<u64>,
        next_watermark: Option<u64>,
    },
}

impl AgentDataBatch {
    /// 读取 batch 所属的 Agent 名称
    #[allow(dead_code)]
    pub fn agent_name(&self) -> &str {
        match self {
            Self::JsonlIncrement { agent_name, .. }
            | Self::JsonDocument { agent_name, .. }
            | Self::SqliteRows { agent_name, .. } => agent_name,
        }
    }

    /// 读取 batch 对应的 source key
    pub fn source_key(&self) -> &str {
        match self {
            Self::JsonlIncrement { source_key, .. }
            | Self::JsonDocument { source_key, .. }
            | Self::SqliteRows { source_key, .. } => source_key,
        }
    }

    /// 读取行为解析应消费的新增文本内容
    pub fn behavior_content(&self) -> Option<&str> {
        match self {
            Self::JsonlIncrement { content, .. } => Some(content),
            Self::JsonDocument { content, .. } => Some(content),
            Self::SqliteRows { .. } => None,
        }
    }

    /// 读取 token 解析应消费的文本内容
    pub fn token_content(&self) -> Option<&str> {
        match self {
            Self::JsonlIncrement {
                content,
                token_context,
                ..
            } => token_context.as_deref().or(Some(content)),
            Self::JsonDocument { content, .. } => Some(content),
            Self::SqliteRows { .. } => None,
        }
    }
}

/// Token 解析输出
#[derive(Debug, Clone, Default)]
pub struct TokenExtraction {
    pub logs: Vec<TokenLog>,
    pub final_model: Option<String>,
}

impl TokenExtraction {
    /// 从 token logs 创建一个无额外解析状态的输出
    pub fn from_logs(logs: Vec<TokenLog>) -> Self {
        Self {
            logs,
            final_model: None,
        }
    }
}

/// Agent 数据源描述 Trait
pub trait AgentSource: Send + Sync {
    fn agent_name(&self) -> &str;
    fn data_source(&self) -> DataSource;
    /// 返回需要监听的日志路径模式列表（支持 glob）
    fn log_paths(&self) -> Vec<String>;

    /// 查询外部 SQLite 数据库，since 为上次查询的源数据水位线
    fn query_sqlite_rows(
        &self,
        _db_path: &Path,
        _since: Option<u64>,
    ) -> Result<SqliteRowBatch, Box<dyn std::error::Error>> {
        Err("此 Agent source 不支持 SQLite 查询".into())
    }
}

/// Token 解析器 Trait
pub trait TokenExtractor: Send + Sync {
    fn extract_tokens(&self, batch: &AgentDataBatch) -> TokenExtraction;
}

/// 行为解析器 Trait
pub trait BehaviorExtractor: Send + Sync {
    fn extract_behavior(&self, _batch: &AgentDataBatch) -> Vec<AgentBehaviorEvent> {
        Vec::new()
    }
}

/// Agent source pipeline Trait
pub trait AgentPipeline: AgentSource + TokenExtractor + BehaviorExtractor {}

impl<T> AgentPipeline for T where T: AgentSource + TokenExtractor + BehaviorExtractor {}

/// 创建所有内置 Agent pipeline 实例
pub fn all_agents() -> Vec<Box<dyn AgentPipeline>> {
    vec![
        Box::new(claude_code::ClaudeCodeAdapter),
        Box::new(codex::CodexAdapter),
        Box::new(gemini_cli::GeminiCliAdapter),
        Box::new(opencode::OpenCodeAdapter),
    ]
}

/// 按设置过滤已启用的 Agent pipeline
pub fn filter_enabled_agents(
    agents: Vec<Box<dyn AgentPipeline>>,
    enabled_agents: &[String],
) -> Vec<Box<dyn AgentPipeline>> {
    agents
        .into_iter()
        .filter(|agent| enabled_agents.contains(&agent.agent_name().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token_log() -> TokenLog {
        TokenLog {
            id: None,
            agent_name: "claude-code".into(),
            provider: "Anthropic".into(),
            model_id: "claude-3-7-sonnet".into(),
            token_type: TokenType::Input,
            token_count: 1024,
            session_id: Some("sess-001".into()),
            request_id: Some("req-001".into()),
            latency_ms: Some(350),
            is_error: false,
            metadata: None,
            cost: None,
            timestamp: "2026-04-18T10:00:00+08:00".into(),
        }
    }

    fn jsonl_batch() -> AgentDataBatch {
        AgentDataBatch::JsonlIncrement {
            agent_name: "codex".into(),
            source_key: "/tmp/session.jsonl".into(),
            path: "/tmp/session.jsonl".into(),
            content: "{}\n".into(),
            token_context: None,
            initial_model: Some("gpt-test".into()),
            previous_offset: 1,
            next_offset: 4,
        }
    }

    #[test]
    fn token_log_serialize_deserialize() {
        let log = make_token_log();
        let json = serde_json::to_string(&log).expect("序列化失败");
        let parsed: TokenLog = serde_json::from_str(&json).expect("反序列化失败");

        assert_eq!(parsed.agent_name, "claude-code");
        assert_eq!(parsed.token_count, 1024);
        assert_eq!(parsed.token_type, TokenType::Input);
        assert!(!parsed.is_error);
    }

    #[test]
    fn token_log_optional_fields() {
        // 验证可选字段为 None 时的序列化/反序列化
        let log = TokenLog {
            id: Some(1),
            agent_name: "codex".into(),
            provider: "OpenAI".into(),
            model_id: "codex-mini".into(),
            token_type: TokenType::Output,
            token_count: 512,
            session_id: None,
            request_id: None,
            latency_ms: None,
            is_error: true,
            metadata: None,
            cost: None,
            timestamp: "2026-04-18T11:00:00+08:00".into(),
        };
        let json = serde_json::to_string(&log).expect("序列化失败");
        let parsed: TokenLog = serde_json::from_str(&json).expect("反序列化失败");

        assert!(parsed.session_id.is_none());
        assert!(parsed.request_id.is_none());
        assert!(parsed.latency_ms.is_none());
        assert!(parsed.is_error);
    }

    #[test]
    fn token_type_serde_snake_case() {
        // 验证 TokenType 序列化为 snake_case，与前端联合类型一致
        assert_eq!(
            serde_json::to_string(&TokenType::Input).unwrap(),
            "\"input\""
        );
        assert_eq!(
            serde_json::to_string(&TokenType::CacheCreate).unwrap(),
            "\"cache_create\""
        );
        assert_eq!(
            serde_json::to_string(&TokenType::CacheRead).unwrap(),
            "\"cache_read\""
        );
        assert_eq!(
            serde_json::to_string(&TokenType::Output).unwrap(),
            "\"output\""
        );

        // 反序列化
        assert_eq!(
            serde_json::from_str::<TokenType>("\"cache_create\"").unwrap(),
            TokenType::CacheCreate
        );
    }

    #[test]
    fn agent_data_batch_exposes_source_key_and_content() {
        let batch = jsonl_batch();

        assert_eq!(batch.agent_name(), "codex");
        assert_eq!(batch.source_key(), "/tmp/session.jsonl");
        assert_eq!(batch.token_content(), Some("{}\n"));
        assert_eq!(batch.behavior_content(), Some("{}\n"));
    }

    #[test]
    fn sqlite_row_batch_defaults_to_empty() {
        let batch = SqliteRowBatch::default();

        assert!(batch.rows.is_empty());
        assert_eq!(batch.high_watermark, None);
    }

    #[test]
    fn unsupported_behavior_extractor_returns_empty_events() {
        let extractor = gemini_cli::GeminiCliAdapter;

        assert!(extractor.extract_behavior(&jsonl_batch()).is_empty());
    }

    #[test]
    fn filter_enabled_agents_keeps_only_requested_sources() {
        let enabled = vec!["codex".to_string()];
        let agents = filter_enabled_agents(all_agents(), &enabled);

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_name(), "codex");
    }
}
