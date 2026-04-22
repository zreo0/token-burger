use std::collections::HashMap;

/// Token 按类型的细分统计
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenBreakdown {
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
    /// Agent 自带的花费汇总（美元）
    pub agent_cost: f64,
}

/// Token 汇总（IPC 传输用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSummary {
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
    pub total: i64,
    /// Agent 自带的花费汇总（美元）
    pub agent_cost: f64,
    pub by_agent: HashMap<String, TokenBreakdown>,
    pub by_model: HashMap<String, TokenBreakdown>,
}

/// Agent 信息（IPC 传输用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub enabled: bool,
    pub available: bool,
    pub source_type: String,
}

/// 应用设置（IPC 传输用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    pub enabled_agents: Vec<String>,
    pub watch_mode: String,
    pub keep_days: u32,
    pub polling_interval_secs: u32,
    pub language: String,
    pub color_theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enabled_agents: vec!["claude-code".into(), "codex".into(), "opencode".into()],
            watch_mode: "realtime".into(),
            keep_days: 365,
            polling_interval_secs: 10,
            language: "en".into(),
            color_theme: "warm".into(),
        }
    }
}

/// 模型价格信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelPrice {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    #[serde(default)]
    pub cache_creation_input_token_cost: f64,
    #[serde(default)]
    pub cache_read_input_token_cost: f64,
}

/// 价格表（模型名 → 价格）
pub type PricingTable = HashMap<String, ModelPrice>;

/// 冷启动进度
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ColdStartProgress {
    pub agent: String,
    pub done: bool,
    pub total: u32,
    pub completed: u32,
}

/// 平台信息（IPC 传输用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlatformInfo {
    pub platform: String,
    pub display_name: String,
}
