pub mod claude_code;
pub mod codex;
pub mod gemini_cli;
pub mod opencode;

use std::path::{Path, PathBuf};

/// Agent 日志数据源类型
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

/// Agent 适配器 Trait
pub trait AgentAdapter: Send + Sync {
    fn agent_name(&self) -> &str;
    fn data_source(&self) -> DataSource;
    /// 返回需要监听的日志路径模式列表（支持 glob）
    fn log_paths(&self) -> Vec<String>;
    /// 解析文本内容（JSONL/JSON），返回统一的 Token 日志集合
    fn parse_content(&self, content: &str) -> Vec<TokenLog>;
    /// 查询外部 SQLite 数据库，since 为上次查询的时间戳
    fn query_db(
        &self,
        _db_path: &Path,
        _since: Option<i64>,
    ) -> Result<Vec<TokenLog>, Box<dyn std::error::Error>> {
        Err("此 Adapter 不支持 SQLite 查询".into())
    }
}

/// 创建所有内置 Adapter 实例
pub fn all_adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(claude_code::ClaudeCodeAdapter),
        Box::new(codex::CodexAdapter),
        Box::new(gemini_cli::GeminiCliAdapter),
        Box::new(opencode::OpenCodeAdapter),
    ]
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
}
