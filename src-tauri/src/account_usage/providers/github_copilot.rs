use std::path::PathBuf;
use std::process::Command;
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

const COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const GITHUB_EMAILS_URL: &str = "https://api.github.com/user/emails";
const EDITOR_PLUGIN_VERSION: &str = "GitHubCopilotChat/0.26.7";
const EDITOR_VERSION: &str = "vscode/1.96.2";
const GITHUB_API_VERSION: &str = "2025-04-01";

pub struct GithubCopilotUsageProvider;

impl AccountUsageProvider for GithubCopilotUsageProvider {
    fn id(&self) -> &'static str {
        "github-copilot"
    }

    fn default_refresh_interval_secs(&self) -> u64 {
        5 * 60
    }

    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "GitHub Copilot".to_string(),
            enabled: state.enabled,
            available: state.credential_ref.is_some() || discover_developer_token().is_some(),
            source: AccountUsageSource::OfficialApi,
            confidence: AccountUsageConfidence::Medium,
            capabilities: vec![
                AccountUsageCapability::AccountUsage,
                AccountUsageCapability::AccountQuota,
                AccountUsageCapability::OfficialApi,
                AccountUsageCapability::TokenRequired,
            ],
            credential_requirements: vec![CredentialRequirement {
                key: "github_token".to_string(),
                label: "GitHub token".to_string(),
                secret: true,
                required: true,
                description: "具备 Copilot 访问权限的 GitHub token".to_string(),
            }],
            experimental: false,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    fn detect(&self) -> bool {
        discover_developer_token().is_some()
    }

    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let token = load_configured_token(&context).or_else(discover_developer_token);
        let token = token.ok_or_else(|| {
            AccountUsageError::new(AccountUsageStatus::AuthRequired, "GitHub token 未配置")
        })?;
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(EDITOR_PLUGIN_VERSION)
            .build()
            .map_err(|error| {
                AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
            })?;

        fetch_copilot_usage(&client, &token)
    }
}

fn fetch_copilot_usage(client: &Client, token: &str) -> AccountUsageResult {
    let copilot_url =
        std::env::var("COPILOT_USER_URL").unwrap_or_else(|_| COPILOT_USER_URL.to_string());
    let user_url = std::env::var("GITHUB_USER_URL").unwrap_or_else(|_| GITHUB_USER_URL.to_string());
    let emails_url =
        std::env::var("GITHUB_EMAILS_URL").unwrap_or_else(|_| GITHUB_EMAILS_URL.to_string());
    let payload = fetch_github_json(client, &copilot_url, token)?;
    let user = fetch_github_json(client, &user_url, token).ok();
    let emails = fetch_github_json(client, &emails_url, token).ok();
    Ok(vec![parse_copilot_snapshot(
        &payload,
        user.as_ref(),
        emails.as_ref(),
    )?])
}

fn fetch_github_json(client: &Client, url: &str, token: &str) -> Result<Value, AccountUsageError> {
    let response = client
        .get(url)
        .header("accept", "application/json")
        .header("authorization", format!("token {token}"))
        .header("editor-plugin-version", EDITOR_PLUGIN_VERSION)
        .header("editor-version", EDITOR_VERSION)
        .header("x-github-api-version", GITHUB_API_VERSION)
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
            "GitHub token 无效或缺少 Copilot 访问权限",
        ));
    }
    if !status.is_success() {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Error,
            redact_secret_text(&format!("GitHub Copilot 请求失败: {status}: {body}")),
        ));
    }
    serde_json::from_str(&body).map_err(|error| {
        AccountUsageError::new(AccountUsageStatus::SchemaChanged, error.to_string())
    })
}

fn parse_copilot_snapshot(
    payload: &Value,
    user: Option<&Value>,
    emails: Option<&Value>,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let login = string_at(payload, &["login"])
        .or_else(|| user.and_then(|value| string_at(value, &["login"])));
    let email = string_at(payload, &["email"])
        .or_else(|| user.and_then(|value| string_at(value, &["email"])))
        .or_else(|| primary_email(emails));
    let account_key = login.clone().or_else(|| email.clone()).ok_or_else(|| {
        AccountUsageError::new(AccountUsageStatus::SchemaChanged, "缺少 GitHub 账号身份")
    })?;
    let quota_reset = string_at(payload, &["quota_reset_date_utc"])
        .or_else(|| string_at(payload, &["quota_reset_date"]));
    let plan = resolve_plan(payload);
    let quotas = normalize_quota_snapshots(payload);

    let mut metrics = Vec::new();
    if let Some(quota) = quotas.premium_interactions.as_ref() {
        metrics.push(quota_metric(
            "copilot.premium_interactions",
            "Premium interactions",
            quota,
            quota_reset.clone(),
        ));
    }
    if let Some(quota) = quotas.chat.as_ref() {
        metrics.push(quota_metric(
            "copilot.chat",
            "Chat",
            quota,
            quota_reset.clone(),
        ));
    }
    if let Some(quota) = quotas.completions.as_ref() {
        metrics.push(quota_metric(
            "copilot.completions",
            "Completions",
            quota,
            quota_reset.clone(),
        ));
    }
    append_scoped_metrics(payload, &mut metrics, quota_reset.clone());

    Ok(AccountUsageSnapshot {
        provider_id: "github-copilot".to_string(),
        account_key: format!("github:{account_key}"),
        account_label: login.or(email),
        plan,
        status: AccountUsageStatus::Ok,
        source: AccountUsageSource::OfficialApi,
        confidence: AccountUsageConfidence::Medium,
        observed_at: now_rfc3339(),
        period_start: None,
        period_end: quota_reset.clone(),
        reset_at: quota_reset,
        stale: false,
        error: None,
        metrics,
    })
}

#[derive(Debug, Default)]
struct QuotaSnapshots {
    premium_interactions: Option<Value>,
    chat: Option<Value>,
    completions: Option<Value>,
}

fn normalize_quota_snapshots(payload: &Value) -> QuotaSnapshots {
    let mut result = QuotaSnapshots::default();
    if let Some(snapshots) = payload.get("quota_snapshots").and_then(Value::as_object) {
        for (key, value) in snapshots {
            let normalized = key
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            match normalized.as_str() {
                "premiuminteractions" => result.premium_interactions = Some(value.clone()),
                "chat" => result.chat = Some(value.clone()),
                "completions" => result.completions = Some(value.clone()),
                _ => {}
            }
        }
    }
    if result.premium_interactions.is_none()
        && result.chat.is_none()
        && result.completions.is_none()
    {
        if let (Some(monthly), Some(limited)) = (
            payload.get("monthly_quotas").and_then(Value::as_object),
            payload
                .get("limited_user_quotas")
                .and_then(Value::as_object),
        ) {
            if let (Some(entitlement), Some(remaining)) =
                (number_field(monthly, "chat"), number_field(limited, "chat"))
            {
                result.chat = Some(quota_value(entitlement, remaining, false));
            }
            if let (Some(entitlement), Some(remaining)) = (
                number_field(monthly, "completions"),
                number_field(limited, "completions"),
            ) {
                result.completions = Some(quota_value(entitlement, remaining, false));
            }
        }
    }
    if result.premium_interactions.is_none() && result.completions.is_some() {
        result.premium_interactions = result.completions.take();
    }
    result
}

fn quota_value(entitlement: f64, remaining: f64, unlimited: bool) -> Value {
    serde_json::json!({
        "entitlement": entitlement,
        "remaining": remaining,
        "percent_remaining": if entitlement > 0.0 { remaining / entitlement * 100.0 } else { 0.0 },
        "unlimited": unlimited,
    })
}

fn quota_metric(
    key: &str,
    label: &str,
    quota: &Value,
    reset_at: Option<String>,
) -> AccountUsageMetric {
    let unlimited = quota
        .get("unlimited")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let entitlement = number_at(quota, &["entitlement"]).unwrap_or(0.0);
    let remaining = number_at(quota, &["remaining"])
        .or_else(|| number_at(quota, &["quota_remaining"]))
        .unwrap_or(0.0);
    let percent_remaining = if unlimited {
        100.0
    } else {
        number_at(quota, &["percent_remaining"])
            .unwrap_or_else(|| {
                if entitlement > 0.0 {
                    remaining / entitlement * 100.0
                } else {
                    0.0
                }
            })
            .clamp(0.0, 100.0)
    };
    let used = if unlimited {
        0.0
    } else {
        (entitlement - remaining).max(0.0)
    };
    AccountUsageMetric {
        metric_key: key.to_string(),
        label: label.to_string(),
        unit: "interaction".to_string(),
        scope: AccountUsageMetricScope::Personal,
        used: Some(used),
        limit: (!unlimited && entitlement > 0.0).then_some(entitlement),
        remaining: Some(remaining),
        percentage: Some(if unlimited {
            0.0
        } else {
            100.0 - percent_remaining
        }),
        reset_at,
    }
}

fn append_scoped_metrics(
    payload: &Value,
    metrics: &mut Vec<AccountUsageMetric>,
    reset_at: Option<String>,
) {
    for (field, scope) in [
        ("organization_quotas", AccountUsageMetricScope::Organization),
        ("team_quotas", AccountUsageMetricScope::Team),
        ("enterprise_quotas", AccountUsageMetricScope::Enterprise),
    ] {
        let Some(object) = payload.get(field).and_then(Value::as_object) else {
            continue;
        };
        for (key, value) in object {
            let mut metric = quota_metric(
                &format!("copilot.{field}.{key}"),
                &format!("{field} {key}"),
                value,
                reset_at.clone(),
            );
            metric.scope = scope.clone();
            metrics.push(metric);
        }
    }
}

fn resolve_plan(payload: &Value) -> Option<String> {
    string_at(payload, &["copilot_plan"])
        .or_else(|| string_at(payload, &["access_type_sku"]))
        .map(|value| {
            value
                .split(['_', '-'])
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
        })
}

fn primary_email(emails: Option<&Value>) -> Option<String> {
    let emails = emails?.as_array()?;
    emails
        .iter()
        .find(|email| {
            email
                .get("primary")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && email
                    .get("verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .or_else(|| {
            emails.iter().find(|email| {
                email
                    .get("verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        })
        .and_then(|email| string_at(email, &["email"]))
}

fn load_configured_token(context: &AccountUsageRefreshContext) -> Option<String> {
    let conn = rusqlite::Connection::open(&context.db_path).ok()?;
    let state = crate::account_usage::store::get_provider_state(&conn, "github-copilot")
        .ok()
        .flatten()?;
    context
        .credentials
        .load_secret(state.credential_ref.as_ref()?)
        .ok()
}

fn discover_developer_token() -> Option<String> {
    run_gh_auth_token()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .or_else(|| std::env::var("GH_TOKEN").ok())
        .or_else(read_gh_hosts_token)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn run_gh_auth_token() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn read_gh_hosts_token() -> Option<String> {
    let home = dirs::home_dir()?;
    for path in [
        home.join(".config").join("gh").join("hosts.yml"),
        home.join(".config").join("gh").join("hosts.yaml"),
    ] {
        if let Some(token) = read_token_from_hosts_file(path) {
            return Some(token);
        }
    }
    None
}

fn read_token_from_hosts_file(path: PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut in_github_com = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') && trimmed.ends_with(':') {
            let host = trimmed.trim_end_matches(':').trim_matches(['\'', '"']);
            in_github_com = host == "github.com";
            continue;
        }
        if !in_github_com {
            continue;
        }
        for prefix in ["oauth_token:", "token:"] {
            if let Some(value) = trimmed.strip_prefix(prefix) {
                let value = value
                    .split(" #")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .trim_matches(['\'', '"']);
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn number_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key)
        .and_then(|value| value.as_f64().or_else(|| value.as_i64().map(|v| v as f64)))
}

fn number_at(value: &Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_copilot_personal_quota() {
        let payload = serde_json::json!({
            "login": "octo",
            "copilot_plan": "individual",
            "quota_reset_date_utc": "2026-06-01T00:00:00Z",
            "quota_snapshots": {
                "premium_interactions": { "entitlement": 300, "remaining": 250, "percent_remaining": 83.33 },
                "chat": { "unlimited": true, "remaining": 0 },
                "completions": { "entitlement": 1000, "quota_remaining": 900 }
            }
        });
        let snapshot = parse_copilot_snapshot(&payload, None, None).unwrap();

        assert_eq!(snapshot.account_label.as_deref(), Some("octo"));
        assert!(snapshot.metrics.iter().any(|metric| {
            metric.metric_key == "copilot.premium_interactions"
                && metric.scope == AccountUsageMetricScope::Personal
                && metric.remaining == Some(250.0)
        }));
    }

    #[test]
    fn test_parse_copilot_org_scoped_metrics() {
        let payload = serde_json::json!({
            "login": "octo",
            "organization_quotas": {
                "acme": { "entitlement": 100, "remaining": 80 }
            }
        });
        let snapshot = parse_copilot_snapshot(&payload, None, None).unwrap();

        assert!(snapshot.metrics.iter().any(|metric| {
            metric.scope == AccountUsageMetricScope::Organization && metric.remaining == Some(80.0)
        }));
    }

    #[test]
    fn test_fallback_quota_formats() {
        let payload = serde_json::json!({
            "login": "octo",
            "monthly_quotas": { "chat": 100, "completions": 200 },
            "limited_user_quotas": { "chat": 25, "completions": 150 }
        });
        let snapshot = parse_copilot_snapshot(&payload, None, None).unwrap();

        assert!(snapshot.metrics.iter().any(|metric| {
            metric.metric_key == "copilot.chat" && metric.remaining == Some(25.0)
        }));
    }

    #[test]
    fn test_missing_account_identity_is_schema_error() {
        let payload = serde_json::json!({});
        let error = parse_copilot_snapshot(&payload, None, None).unwrap_err();

        assert_eq!(error.code, AccountUsageStatus::SchemaChanged);
    }

    #[test]
    fn test_hosts_file_token_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hosts.yml");
        std::fs::write(
            &path,
            "enterprise.example.com:\n  oauth_token: 'enterprise-token'\ngithub.com:\n  oauth_token: 'gho_test'\n",
        )
        .unwrap();

        assert_eq!(
            read_token_from_hosts_file(path).as_deref(),
            Some("gho_test")
        );
    }
}
