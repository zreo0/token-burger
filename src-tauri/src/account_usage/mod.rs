pub mod credentials;
pub mod manager;
pub mod providers;
pub mod store;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageCapability {
    LocalTokens,
    AccountUsage,
    AccountQuota,
    CostEstimate,
    MultiAccount,
    OfficialApi,
    InternalApi,
    CookieRequired,
    AuthFileRequired,
    TokenRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageSource {
    LocalLogs,
    AuthFile,
    OfficialApi,
    InternalApi,
    UserCredential,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageStatus {
    Ok,
    Stale,
    AuthRequired,
    Forbidden,
    Unsupported,
    RateLimited,
    Network,
    SchemaChanged,
    CredentialUnavailable,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageMetricScope {
    Personal,
    Account,
    Workspace,
    Organization,
    Team,
    Enterprise,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRequirement {
    pub key: String,
    pub label: String,
    pub secret: bool,
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageProviderInfo {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub available: bool,
    pub source: AccountUsageSource,
    pub confidence: AccountUsageConfidence,
    pub capabilities: Vec<AccountUsageCapability>,
    pub credential_requirements: Vec<CredentialRequirement>,
    pub experimental: bool,
    pub default_refresh_interval_secs: u64,
    pub refresh_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageError {
    pub code: AccountUsageStatus,
    pub message: String,
    pub retry_after_secs: Option<u64>,
}

impl AccountUsageError {
    pub fn new(code: AccountUsageStatus, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after_secs: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageMetric {
    pub metric_key: String,
    pub label: String,
    pub unit: String,
    pub scope: AccountUsageMetricScope,
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub percentage: Option<f64>,
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageSnapshot {
    pub provider_id: String,
    pub account_key: String,
    pub account_label: Option<String>,
    pub plan: Option<String>,
    pub status: AccountUsageStatus,
    pub source: AccountUsageSource,
    pub confidence: AccountUsageConfidence,
    pub observed_at: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub reset_at: Option<String>,
    pub stale: bool,
    pub error: Option<AccountUsageError>,
    pub metrics: Vec<AccountUsageMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageProviderState {
    pub provider_id: String,
    pub enabled: bool,
    pub refresh_interval_secs: u64,
    pub last_refresh_at: Option<String>,
    pub retry_after_until: Option<String>,
    pub credential_ref: Option<String>,
    pub credential_label: Option<String>,
    pub auto_discovery_enabled: bool,
}

#[derive(Clone)]
pub struct AccountUsageRefreshContext {
    pub db_path: PathBuf,
    pub credentials: credentials::CredentialStore,
}

pub type AccountUsageResult = Result<Vec<AccountUsageSnapshot>, AccountUsageError>;

pub trait AccountUsageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn default_refresh_interval_secs(&self) -> u64;
    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo;
    fn detect(&self) -> bool;
    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult;
}

pub type SharedAccountUsageProvider = Arc<dyn AccountUsageProvider>;

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub(crate) fn enum_to_db<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "error".to_string())
}

pub(crate) fn enum_from_db<T>(value: &str, fallback: T) -> T
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(serde_json::Value::String(value.to_string())).unwrap_or(fallback)
}

pub fn redact_secret_text(input: &str) -> String {
    let mut output = input.to_string();
    for marker in [
        "authorization",
        "bearer",
        "cookie",
        "set-cookie",
        "access_token",
        "refresh_token",
        "id_token",
        "api_key",
        "token",
        "session",
    ] {
        output = redact_marker_values(&output, marker);
    }
    redact_bare_secret_tokens(&output)
}

pub fn redact_account_usage_snapshots(
    mut snapshots: Vec<AccountUsageSnapshot>,
) -> Vec<AccountUsageSnapshot> {
    for snapshot in &mut snapshots {
        redact_snapshot_error(snapshot);
    }
    snapshots
}

pub(crate) fn redact_snapshot_error(snapshot: &mut AccountUsageSnapshot) {
    if let Some(error) = &mut snapshot.error {
        error.message = redact_secret_text(&error.message);
    }
}

fn redact_marker_values(input: &str, marker: &str) -> String {
    let mut remaining = input;
    let mut redacted = String::with_capacity(input.len());

    loop {
        let lower = remaining.to_lowercase();
        let Some(index) = lower.find(marker) else {
            redacted.push_str(remaining);
            break;
        };

        redacted.push_str(&remaining[..index]);
        redacted.push_str(&remaining[index..index + marker.len()]);

        let rest = &remaining[index + marker.len()..];
        let prefix_len = rest
            .find(|ch: char| !(ch.is_whitespace() || ch == ':' || ch == '=' || ch == '"'))
            .unwrap_or(rest.len());
        redacted.push_str(&rest[..prefix_len]);

        let value = &rest[prefix_len..];
        if value.is_empty() {
            remaining = value;
            continue;
        }

        let split_at = value
            .find([',', '\n', '&', ';', '}', ']'])
            .unwrap_or(value.len());
        redacted.push_str("<redacted>");
        remaining = &value[split_at..];
    }

    redacted
}

fn redact_bare_secret_tokens(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut token = String::new();

    for ch in input.chars() {
        if is_token_char(ch) {
            token.push(ch);
            continue;
        }

        flush_redacted_token(&mut output, &mut token);
        output.push(ch);
    }
    flush_redacted_token(&mut output, &mut token);

    output
}

fn flush_redacted_token(output: &mut String, token: &mut String) {
    if token.is_empty() {
        return;
    }

    if is_known_secret_token(token) {
        output.push_str("<redacted>");
    } else {
        output.push_str(token);
    }
    token.clear();
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn is_known_secret_token(token: &str) -> bool {
    let known_prefix = [
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "sk-",
        "xoxb-",
        "xoxp-",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix) && token.len() > prefix.len() + 6);

    known_prefix || looks_like_jwt(token)
}

fn looks_like_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| part.len() >= 8)
        && parts.iter().all(|part| {
            part.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        })
}
