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
    timestamp: string;
}
