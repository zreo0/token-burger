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

/**
 * 趋势时间桶，保存模型细分以复用前端定价规则
 */
#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenTrendBucket {
    /// 时间桶起点，Unix 秒
    pub start: i64,
    /// 时间桶终点，Unix 秒
    pub end: i64,
    /// 桶内模型用量
    pub by_model: HashMap<String, TokenBreakdown>,
}

/**
 * 本地日志趋势及可比较的上一周期
 */
#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenTrend {
    /// 包含零用量区间的时间桶
    pub buckets: Vec<TokenTrendBucket>,
    /// 上一周期模型用量
    pub previous_by_model: HashMap<String, TokenBreakdown>,
    /// 已有日志是否覆盖上一周期起点
    pub comparison_available: bool,
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
    pub behavior_tips_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enabled_agents: vec![
                "claude-code".into(),
                "codex".into(),
                "opencode".into(),
                "mimocode".into(),
            ],
            watch_mode: "realtime".into(),
            keep_days: 90,
            polling_interval_secs: 10,
            language: "en".into(),
            color_theme: "warm".into(),
            behavior_tips_enabled: false,
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

/// 模型价格刷新结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PricingRefreshResult {
    /// 是否实际获取并应用了新价格表
    pub updated: bool,
    /// 当前价格表包含的模型数量
    pub model_count: usize,
}

/// 冷启动进度
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ColdStartProgress {
    /** 新增用量已纳入启动期采集，不代表历史完整 */
    pub live: bool,
    /** 状态修订号，避免页面初始查询覆盖较新的事件 */
    pub revision: u64,
    /** recent / history / writing / complete */
    pub phase: String,
    /** 累计检查的文件数，包含无需重读的文件 */
    pub files_checked: u32,
    /** 读取或持久化失败次数 */
    pub errors: u32,
    pub agent: String,
    pub done: bool,
    /** 历史文件与 SQLite 会话任务总数 */
    pub total: u32,
    /** 已处理任务数，失败次数单独显示 */
    pub completed: u32,
}

/// 平台信息（IPC 传输用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlatformInfo {
    pub platform: String,
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证未配置的新用户使用预期默认设置
    #[test]
    fn app_settings_uses_expected_defaults() {
        let settings = AppSettings::default();

        assert_eq!(settings.keep_days, 90);
        assert!(!settings.behavior_tips_enabled);
    }
}
