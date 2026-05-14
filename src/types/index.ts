// 与 Rust 后端 token_logs 表一致的数据模型
// timestamp 格式：RFC3339（如 2026-04-18T10:00:00+08:00）
export interface TokenLog {
    id?: number;
    agent_name: string;
    provider: string;
    model_id: string;
    token_type: 'input' | 'cache_create' | 'cache_read' | 'output';
    token_count: number;
    session_id?: string;
    request_id?: string;
    latency_ms?: number;
    is_error: boolean;
    metadata?: string;
    cost?: number;
    timestamp: string;
}

// Token 按类型的细分统计
export interface TokenBreakdown {
    input: number;
    cache_create: number;
    cache_read: number;
    output: number;
    agent_cost: number;
}

// Token 汇总（IPC 传输用）
export interface TokenSummary {
    input: number;
    cache_create: number;
    cache_read: number;
    output: number;
    total: number;
    agent_cost: number;
    by_agent: Record<string, TokenBreakdown>;
    by_model: Record<string, TokenBreakdown>;
}

// Agent 信息
export interface AgentInfo {
    name: string;
    enabled: boolean;
    available: boolean;
    source_type: string;
}

// 应用设置
export interface AppSettings {
    enabled_agents: string[];
    watch_mode: string;
    keep_days: number;
    polling_interval_secs: number;
    language: string;
    color_theme: string;
}

// 当前运行平台
export interface PlatformInfo {
    platform: string;
    display_name: string;
}

// 模型价格信息
export interface ModelPrice {
    input_cost_per_token: number;
    output_cost_per_token: number;
    cache_creation_input_token_cost: number;
    cache_read_input_token_cost: number;
}

// 价格表（模型名 → 价格）
export type PricingTable = Record<string, ModelPrice>;

// 冷启动进度
export interface ColdStartProgress {
    agent: string;
    done: boolean;
    total: number;
    completed: number;
}

// 时间范围
export type TimeRange = 'today' | '7d' | '30d';

export type AccountUsageCapability =
    | 'local_tokens' | 'account_usage' | 'account_quota'
    | 'cost_estimate' | 'multi_account' | 'official_api'
    | 'internal_api' | 'cookie_required' | 'auth_file_required'
    | 'token_required';

export type AccountUsageSource =
    | 'local_logs' | 'auth_file' | 'official_api'
    | 'internal_api' | 'user_credential' | 'unsupported';

export type AccountUsageConfidence = 'high' | 'medium' | 'low';

export type AccountUsageStatus =
    | 'ok' | 'stale' | 'auth_required' | 'forbidden'
    | 'unsupported' | 'rate_limited' | 'network'
    | 'schema_changed' | 'credential_unavailable' | 'error';

export type AccountUsageMetricScope =
    | 'personal' | 'account' | 'workspace'
    | 'organization' | 'team' | 'enterprise' | 'local';

export interface CredentialRequirement {
    key: string;
    label: string;
    secret: boolean;
    required: boolean;
    description: string;
}

export interface AccountUsageProviderInfo {
    id: string;
    display_name: string;
    enabled: boolean;
    available: boolean;
    source: AccountUsageSource;
    confidence: AccountUsageConfidence;
    capabilities: AccountUsageCapability[];
    credential_requirements: CredentialRequirement[];
    experimental: boolean;
    default_refresh_interval_secs: number;
    refresh_interval_secs: number;
}

export interface AccountUsageError {
    code: AccountUsageStatus;
    message: string;
    retry_after_secs?: number;
}

export interface AccountUsageMetric {
    metric_key: string;
    label: string;
    unit: string;
    scope: AccountUsageMetricScope;
    used?: number | null;
    limit?: number | null;
    remaining?: number | null;
    percentage?: number | null;
    reset_at?: string | null;
}

export interface AccountUsageSnapshot {
    provider_id: string;
    account_key: string;
    account_label?: string | null;
    plan?: string | null;
    status: AccountUsageStatus;
    source: AccountUsageSource;
    confidence: AccountUsageConfidence;
    observed_at: string;
    period_start?: string | null;
    period_end?: string | null;
    reset_at?: string | null;
    stale: boolean;
    error?: AccountUsageError;
    metrics: AccountUsageMetric[];
}

export interface AccountUsageProviderState {
    provider_id: string;
    enabled: boolean;
    refresh_interval_secs: number;
    last_refresh_at?: string;
    retry_after_until?: string;
    credential_ref?: string;
    credential_label?: string;
    auto_discovery_enabled: boolean;
}
