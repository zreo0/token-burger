use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::Value;

use crate::account_usage::{
    now_rfc3339, redact_secret_text, AccountUsageCapability, AccountUsageConfidence,
    AccountUsageError, AccountUsageMetric, AccountUsageMetricScope, AccountUsageProvider,
    AccountUsageProviderInfo, AccountUsageProviderState, AccountUsageRefreshContext,
    AccountUsageResult, AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus,
    CredentialRequirement,
};

const CURSOR_USAGE_SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";
const CURSOR_AUTH_ME_URL: &str = "https://cursor.com/api/auth/me";
const CURSOR_USAGE_URL: &str = "https://cursor.com/api/usage";

pub struct CursorUsageProvider;

impl AccountUsageProvider for CursorUsageProvider {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn default_refresh_interval_secs(&self) -> u64 {
        10 * 60
    }

    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "Cursor".to_string(),
            enabled: state.enabled,
            show_in_menu_bar: state.show_in_menu_bar,
            available: state.credential_ref.is_some(),
            source: AccountUsageSource::InternalApi,
            confidence: AccountUsageConfidence::Low,
            capabilities: vec![
                AccountUsageCapability::AccountUsage,
                AccountUsageCapability::AccountQuota,
                AccountUsageCapability::InternalApi,
                AccountUsageCapability::CookieRequired,
            ],
            credential_requirements: vec![CredentialRequirement {
                key: "session".to_string(),
                label: "Cursor session/cookie".to_string(),
                secret: true,
                required: true,
                description: "显式提供 Cursor Web session 或 cookie".to_string(),
            }],
            experimental: true,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    fn detect(&self) -> bool {
        false
    }

    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let conn = rusqlite::Connection::open(&context.db_path).map_err(|error| {
            AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
        })?;
        let state = crate::account_usage::store::get_provider_state(&conn, self.id())?.ok_or_else(
            || AccountUsageError::new(AccountUsageStatus::AuthRequired, "Cursor 凭据未配置"),
        )?;
        let credential_ref = state.credential_ref.ok_or_else(|| {
            AccountUsageError::new(AccountUsageStatus::AuthRequired, "Cursor 凭据未配置")
        })?;
        let cookie = context.credentials.load_secret(&credential_ref)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(cursor_user_agent())
            .build()
            .map_err(|error| {
                AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
            })?;

        fetch_cursor_usage(&client, &cookie)
    }
}

fn fetch_cursor_usage(client: &Client, cookie: &str) -> AccountUsageResult {
    let summary_url = std::env::var("CURSOR_USAGE_SUMMARY_URL")
        .unwrap_or_else(|_| CURSOR_USAGE_SUMMARY_URL.to_string());
    let auth_me_url =
        std::env::var("CURSOR_AUTH_ME_URL").unwrap_or_else(|_| CURSOR_AUTH_ME_URL.to_string());
    let usage_url =
        std::env::var("CURSOR_USAGE_URL").unwrap_or_else(|_| CURSOR_USAGE_URL.to_string());

    let summary = fetch_cursor_json(client, &summary_url, cookie)?;
    let user_info = fetch_cursor_json(client, &auth_me_url, cookie).ok();
    let request_usage = user_info
        .as_ref()
        .and_then(|value| string_at(value, &["sub"]))
        .and_then(|user_id| {
            fetch_cursor_json(client, &format!("{usage_url}?user={user_id}"), cookie).ok()
        });

    Ok(vec![parse_cursor_snapshot(
        &summary,
        user_info.as_ref(),
        request_usage.as_ref(),
    )?])
}

fn fetch_cursor_json(client: &Client, url: &str, cookie: &str) -> Result<Value, AccountUsageError> {
    let response = client
        .get(url)
        .header("cookie", cookie)
        .header("accept", "application/json")
        .send()
        .map_err(|error| {
            AccountUsageError::new(
                AccountUsageStatus::Network,
                redact_secret_text(&error.to_string()),
            )
        })?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "Cursor session 已失效或无权限",
        ));
    }
    if !status.is_success() {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Error,
            redact_secret_text(&format!("Cursor 请求失败: {status}: {body}")),
        ));
    }
    serde_json::from_str(&body).map_err(|error| {
        AccountUsageError::new(AccountUsageStatus::SchemaChanged, error.to_string())
    })
}

fn parse_cursor_snapshot(
    summary: &Value,
    user_info: Option<&Value>,
    request_usage: Option<&Value>,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let individual = summary.get("individualUsage");
    let plan = individual.and_then(|value| value.get("plan"));
    let membership_type = string_at(summary, &["membershipType"]);
    let account_label = user_info
        .and_then(|value| string_at(value, &["email"]))
        .or_else(|| user_info.and_then(|value| string_at(value, &["name"])));
    let subject = user_info.and_then(|value| string_at(value, &["sub"]));
    let account_key = subject
        .clone()
        .or_else(|| account_label.clone())
        .unwrap_or_else(|| "default".to_string());
    let billing_end = string_at(summary, &["billingCycleEnd"]);

    let plan_used = number_at(plan, &["used"]);
    let plan_limit = number_at(plan, &["limit"]);
    let auto_percent = number_at(plan, &["autoPercentUsed"]).map(clamp_percent);
    let api_percent = number_at(plan, &["apiPercentUsed"]).map(clamp_percent);
    let total_percent = number_at(plan, &["totalPercentUsed"]).map(clamp_percent);
    let plan_percent = total_percent.or_else(|| match (auto_percent, api_percent) {
        (Some(auto), Some(api)) => Some((auto + api) / 2.0),
        (Some(auto), None) => Some(auto),
        (None, Some(api)) => Some(api),
        _ => match (plan_used, plan_limit) {
            (Some(used), Some(limit)) if limit > 0.0 => Some(clamp_percent(used / limit * 100.0)),
            _ => None,
        },
    });

    let legacy_percent = parse_legacy_request_percent(request_usage);
    let primary_percent = legacy_percent.or(plan_percent).ok_or_else(|| {
        AccountUsageError::new(
            AccountUsageStatus::SchemaChanged,
            "Cursor 响应缺少核心用量字段",
        )
    })?;
    let on_demand_used = number_at(
        individual.and_then(|value| value.get("onDemand")),
        &["used"],
    );

    let mut metrics = vec![percent_metric(
        "cursor.plan",
        "Plan usage",
        primary_percent,
        billing_end.clone(),
    )];
    if let Some(auto_percent) = auto_percent {
        metrics.push(percent_metric(
            "cursor.auto",
            "Auto / Composer usage",
            auto_percent,
            billing_end.clone(),
        ));
    }
    if let Some(api_percent) = api_percent {
        metrics.push(percent_metric(
            "cursor.api",
            "API usage",
            api_percent,
            billing_end.clone(),
        ));
    }
    if plan_used.is_some() || plan_limit.is_some() {
        metrics.push(money_metric(
            "cursor.included_plan",
            "Included plan",
            plan_used.unwrap_or(0.0),
            plan_limit.unwrap_or(0.0),
        ));
    }
    if let Some(on_demand_used) = on_demand_used {
        metrics.push(money_metric(
            "cursor.on_demand",
            "On-demand used",
            on_demand_used,
            0.0,
        ));
    }

    Ok(AccountUsageSnapshot {
        provider_id: "cursor".to_string(),
        account_key: format!("cursor:{account_key}"),
        account_label,
        plan: membership_type.map(format_membership),
        status: AccountUsageStatus::Ok,
        source: AccountUsageSource::InternalApi,
        confidence: AccountUsageConfidence::Low,
        observed_at: now_rfc3339(),
        period_start: None,
        period_end: billing_end.clone(),
        reset_at: billing_end,
        stale: false,
        error: None,
        metrics,
    })
}

fn parse_legacy_request_percent(request_usage: Option<&Value>) -> Option<f64> {
    let gpt4 = request_usage?.get("gpt-4")?;
    let used = number_at(Some(gpt4), &["numRequestsTotal"])
        .or_else(|| number_at(Some(gpt4), &["numRequests"]))?;
    let limit = number_at(Some(gpt4), &["maxRequestUsage"])?;
    (limit > 0.0).then(|| clamp_percent(used / limit * 100.0))
}

fn percent_metric(
    key: &str,
    label: &str,
    used_percent: f64,
    reset_at: Option<String>,
) -> AccountUsageMetric {
    AccountUsageMetric {
        metric_key: key.to_string(),
        label: label.to_string(),
        unit: "percent".to_string(),
        scope: AccountUsageMetricScope::Personal,
        used: Some(used_percent),
        limit: Some(100.0),
        remaining: Some((100.0 - used_percent).max(0.0)),
        percentage: Some(used_percent),
        reset_at,
    }
}

fn money_metric(key: &str, label: &str, used_cents: f64, limit_cents: f64) -> AccountUsageMetric {
    AccountUsageMetric {
        metric_key: key.to_string(),
        label: label.to_string(),
        unit: "usd".to_string(),
        scope: AccountUsageMetricScope::Personal,
        used: Some(used_cents / 100.0),
        limit: (limit_cents > 0.0).then_some(limit_cents / 100.0),
        remaining: (limit_cents > 0.0).then_some(((limit_cents - used_cents) / 100.0).max(0.0)),
        percentage: (limit_cents > 0.0).then_some(clamp_percent(used_cents / limit_cents * 100.0)),
        reset_at: None,
    }
}

fn number_at(value: Option<&Value>, path: &[&str]) -> Option<f64> {
    let mut current = value?;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_f64()
        .or_else(|| current.as_i64().map(|value| value as f64))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn clamp_percent(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn format_membership(raw: String) -> String {
    raw.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn cursor_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
}

impl From<rusqlite::Error> for AccountUsageError {
    fn from(error: rusqlite::Error) -> Self {
        AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cursor_snapshot_success() {
        let summary = serde_json::json!({
            "membershipType": "pro_monthly",
            "billingCycleEnd": "2026-06-01T00:00:00Z",
            "individualUsage": {
                "plan": {
                    "used": 2500,
                    "limit": 10000,
                    "autoPercentUsed": 30,
                    "apiPercentUsed": 40,
                    "totalPercentUsed": 35
                },
                "onDemand": { "used": 1200 }
            }
        });
        let user = serde_json::json!({ "email": "user@example.com", "sub": "user-1" });
        let snapshot = parse_cursor_snapshot(&summary, Some(&user), None).unwrap();

        assert_eq!(snapshot.provider_id, "cursor");
        assert_eq!(snapshot.account_label.as_deref(), Some("user@example.com"));
        assert_eq!(snapshot.plan.as_deref(), Some("Pro Monthly"));
        assert!(snapshot.metrics.iter().any(|metric| {
            metric.metric_key == "cursor.plan" && metric.percentage == Some(35.0)
        }));
    }

    #[test]
    fn test_parse_cursor_legacy_request_usage() {
        let summary = serde_json::json!({
            "individualUsage": { "plan": { "used": 0, "limit": 0 } }
        });
        let request_usage = serde_json::json!({
            "gpt-4": { "numRequestsTotal": 8, "maxRequestUsage": 10 }
        });
        let snapshot = parse_cursor_snapshot(&summary, None, Some(&request_usage)).unwrap();

        assert_eq!(snapshot.metrics[0].percentage, Some(80.0));
    }

    #[test]
    fn test_cursor_missing_credential_status_metadata() {
        let provider = CursorUsageProvider;
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
        assert!(info.experimental);
        assert!(!info.available);
        assert!(!state.auto_discovery_enabled);
    }

    #[test]
    fn test_parse_cursor_missing_core_usage_is_schema_error() {
        let summary = serde_json::json!({});
        let error = parse_cursor_snapshot(&summary, None, None).unwrap_err();

        assert_eq!(error.code, AccountUsageStatus::SchemaChanged);
    }

    #[test]
    fn test_parse_cursor_missing_used_does_not_default_to_zero_percent() {
        let summary = serde_json::json!({
            "individualUsage": { "plan": { "limit": 10000 } }
        });
        let error = parse_cursor_snapshot(&summary, None, None).unwrap_err();

        assert_eq!(error.code, AccountUsageStatus::SchemaChanged);
    }
}
