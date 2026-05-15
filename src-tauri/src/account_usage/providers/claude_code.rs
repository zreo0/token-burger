use rusqlite::Connection;

use crate::account_usage::{
    now_rfc3339, AccountUsageCapability, AccountUsageConfidence, AccountUsageError,
    AccountUsageMetric, AccountUsageMetricScope, AccountUsageProvider, AccountUsageProviderInfo,
    AccountUsageProviderState, AccountUsageRefreshContext, AccountUsageResult,
    AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus,
};

pub struct ClaudeCodeUsageProvider;

impl AccountUsageProvider for ClaudeCodeUsageProvider {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn default_refresh_interval_secs(&self) -> u64 {
        60
    }

    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "Claude Code".to_string(),
            enabled: state.enabled,
            show_in_menu_bar: state.show_in_menu_bar,
            available: true,
            source: AccountUsageSource::LocalLogs,
            confidence: AccountUsageConfidence::High,
            capabilities: vec![
                AccountUsageCapability::LocalTokens,
                AccountUsageCapability::CostEstimate,
            ],
            credential_requirements: Vec::new(),
            experimental: false,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    fn detect(&self) -> bool {
        true
    }

    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let conn = Connection::open(&context.db_path).map_err(|error| {
            AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
        })?;
        let metrics = local_usage_metrics(&conn).map_err(|error| {
            AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
        })?;

        Ok(vec![AccountUsageSnapshot {
            provider_id: self.id().to_string(),
            account_key: "local".to_string(),
            account_label: Some("本地 Claude Code 日志".to_string()),
            plan: None,
            status: AccountUsageStatus::Unsupported,
            source: AccountUsageSource::LocalLogs,
            confidence: AccountUsageConfidence::High,
            observed_at: now_rfc3339(),
            period_start: None,
            period_end: None,
            reset_at: None,
            stale: false,
            error: Some(AccountUsageError::new(
                AccountUsageStatus::Unsupported,
                "账号剩余额度不可用，仅展示本地 token 用量",
            )),
            metrics,
        }])
    }
}

fn local_usage_metrics(conn: &Connection) -> Result<Vec<AccountUsageMetric>, rusqlite::Error> {
    let mut metrics = Vec::new();
    for (range, label) in [("today", "今日"), ("7d", "近 7 天"), ("30d", "近 30 天")] {
        let summary = crate::db::queries::get_token_summary_for_agents(
            conn,
            range,
            &["claude-code".to_string()],
        )?;
        metrics.push(AccountUsageMetric {
            metric_key: format!("claude.local_tokens.{range}"),
            label: format!("{label}本地 tokens"),
            unit: "token".to_string(),
            scope: AccountUsageMetricScope::Local,
            used: Some(summary.total as f64),
            limit: None,
            remaining: None,
            percentage: None,
            reset_at: None,
        });
        metrics.push(AccountUsageMetric {
            metric_key: format!("claude.local_cost.{range}"),
            label: format!("{label}本地成本"),
            unit: "usd".to_string(),
            scope: AccountUsageMetricScope::Local,
            used: Some(summary.agent_cost),
            limit: None,
            remaining: None,
            percentage: None,
            reset_at: None,
        });
    }
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{TokenLog, TokenType};

    #[test]
    fn test_info_marks_local_logs() {
        let provider = ClaudeCodeUsageProvider;
        let state = AccountUsageProviderState {
            provider_id: provider.id().to_string(),
            enabled: true,
            show_in_menu_bar: false,
            refresh_interval_secs: provider.default_refresh_interval_secs(),
            last_refresh_at: None,
            retry_after_until: None,
            credential_ref: None,
            credential_label: None,
            auto_discovery_enabled: false,
        };
        let info = provider.info(&state);
        assert_eq!(info.source, AccountUsageSource::LocalLogs);
    }

    #[test]
    fn test_local_usage_summary_and_unsupported_quota() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        crate::db::queries::batch_insert_token_logs(
            &conn,
            &[TokenLog {
                id: None,
                agent_name: "claude-code".to_string(),
                provider: "Anthropic".to_string(),
                model_id: "claude-sonnet".to_string(),
                token_type: TokenType::Input,
                token_count: 100,
                session_id: Some("session-1".to_string()),
                request_id: Some("request-1".to_string()),
                latency_ms: None,
                is_error: false,
                metadata: None,
                cost: Some(0.01),
                timestamp: chrono::Local::now().to_rfc3339(),
            }],
        )
        .unwrap();

        let before_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();
        let metrics = local_usage_metrics(&conn).unwrap();
        let after_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();

        assert_eq!(before_count, after_count);
        assert!(metrics.iter().any(|metric| {
            metric.metric_key == "claude.local_tokens.today" && metric.used == Some(100.0)
        }));
        assert!(metrics.iter().any(|metric| {
            metric.metric_key == "claude.local_cost.today" && metric.used == Some(0.01)
        }));
    }

    #[test]
    fn test_local_usage_empty_data_returns_zero_metrics() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        let metrics = local_usage_metrics(&conn).unwrap();

        assert!(metrics.iter().any(|metric| {
            metric.metric_key == "claude.local_tokens.today" && metric.used == Some(0.0)
        }));
    }
}
