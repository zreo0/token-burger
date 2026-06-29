use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::account_usage::{
    now_rfc3339, redact_secret_text, AccountUsageCapability, AccountUsageConfidence,
    AccountUsageError, AccountUsageMetric, AccountUsageMetricScope, AccountUsageProvider,
    AccountUsageProviderInfo, AccountUsageProviderState, AccountUsageRefreshContext,
    AccountUsageResult, AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus,
};

const CODEX_REFRESH_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_RESET_CREDITS_ENDPOINT: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";

pub struct CodexUsageProvider;

impl AccountUsageProvider for CodexUsageProvider {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn default_refresh_interval_secs(&self) -> u64 {
        5 * 60
    }

    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "Codex".to_string(),
            enabled: state.enabled,
            show_in_menu_bar: state.show_in_menu_bar,
            available: self.detect(),
            source: AccountUsageSource::AuthFile,
            confidence: AccountUsageConfidence::High,
            capabilities: vec![
                AccountUsageCapability::AccountUsage,
                AccountUsageCapability::AccountQuota,
                AccountUsageCapability::MultiAccount,
                AccountUsageCapability::AuthFileRequired,
            ],
            credential_requirements: Vec::new(),
            experimental: false,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    fn detect(&self) -> bool {
        discover_auth_files().iter().any(|path| path.exists())
    }

    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let auth_files = discover_auth_files();
        if auth_files.is_empty() || !auth_files.iter().any(|path| path.exists()) {
            return Err(AccountUsageError::new(
                AccountUsageStatus::AuthRequired,
                "未找到 Codex auth 文件",
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("TokenBurger/0.1 account-usage")
            .build()
            .map_err(|error| {
                AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
            })?;
        let mut snapshots = Vec::new();
        let mut last_error = None;

        for path in auth_files.into_iter().filter(|path| path.exists()) {
            match refresh_auth_file_usage(&client, &path) {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(error) => last_error = Some(error),
            }
        }

        if snapshots.is_empty() {
            Err(last_error.unwrap_or_else(|| {
                AccountUsageError::new(AccountUsageStatus::AuthRequired, "Codex auth 不可用")
            }))
        } else {
            if let Some(error) = last_error {
                append_stale_codex_snapshots_for_partial_failure(
                    &context.db_path,
                    &mut snapshots,
                    &error,
                );
            }
            Ok(snapshots)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY", default)]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: Option<CodexTokenData>,
    #[serde(default)]
    last_refresh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexTokenData {
    id_token: String,
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct JwtClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<ProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<AuthClaims>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<Value>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexAuthIdentity {
    auth_file: PathBuf,
    access_token: String,
    refresh_token: String,
    account_id: String,
    account_label: Option<String>,
    plan: Option<String>,
    user_id: Option<String>,
    expires_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct RateLimitSnapshot {
    limit_id: String,
    limit_name: Option<String>,
    primary: Option<RateLimitWindow>,
    secondary: Option<RateLimitWindow>,
    credits: Option<CreditsSnapshot>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct RateLimitWindow {
    used_percent: f64,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
struct CreditsSnapshot {
    has_credits: bool,
    unlimited: bool,
    balance: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ResetCreditsSummary {
    available_count: u64,
    next_expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResetCreditsResponse {
    #[serde(default)]
    available_count: Option<u64>,
    #[serde(default)]
    credits: Vec<ResetCredit>,
}

#[derive(Debug, Deserialize)]
struct ResetCredit {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(rename = "expiresAt", default)]
    expires_at_camel: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RateLimitEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    plan_type: Option<Value>,
    #[serde(default)]
    rate_limits: Option<RateLimitEventDetails>,
    #[serde(default)]
    credits: Option<RateLimitEventCredits>,
    #[serde(default)]
    metered_limit_name: Option<String>,
    #[serde(default)]
    limit_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RateLimitEventDetails {
    #[serde(default)]
    primary: Option<RateLimitEventWindow>,
    #[serde(default)]
    secondary: Option<RateLimitEventWindow>,
}

#[derive(Debug, Deserialize)]
struct RateLimitEventWindow {
    used_percent: f64,
    #[serde(default)]
    window_minutes: Option<i64>,
    #[serde(default)]
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RateLimitEventCredits {
    has_credits: bool,
    unlimited: bool,
    #[serde(default)]
    balance: Option<String>,
}

fn discover_auth_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("CODEX_AUTH_FILE") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(dir) = std::env::var("CODEX_AUTH_DIR") {
        paths.extend(auth_files_in_dir(Path::new(&dir)));
    }
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        paths.push(PathBuf::from(dir).join("auth.json"));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".codex").join("auth.json"));
    }
    dedupe_paths(paths)
}

fn auth_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("auth.json"))
        .collect()
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths {
        if !result.contains(&path) {
            result.push(path);
        }
    }
    result
}

fn refresh_auth_file_usage(
    client: &Client,
    auth_file: &Path,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let mut auth = load_codex_auth(auth_file)?;
    let mut identity = parse_codex_identity(auth_file, &auth)?;
    if identity
        .expires_at
        .map(|expires_at| expires_at <= chrono::Utc::now().timestamp() + 60)
        .unwrap_or(false)
    {
        refresh_codex_oauth(client, auth_file, &mut auth, &mut identity)?;
    }
    let response = request_usage(client, &identity);
    let response = match response {
        Ok(response) => response,
        Err(error) if error.code == AccountUsageStatus::AuthRequired => {
            refresh_codex_oauth(client, auth_file, &mut auth, &mut identity)?;
            request_usage(client, &identity)?
        }
        Err(error) => return Err(error),
    };
    let snapshots = parse_rate_limit_response(&response.headers, &response.body)?;
    let reset_credits = request_reset_credits(client, &identity).ok();
    Ok(build_account_snapshot(identity, snapshots, reset_credits))
}

struct UsageResponse {
    headers: HeaderMap,
    body: String,
}

fn request_usage(
    client: &Client,
    identity: &CodexAuthIdentity,
) -> Result<UsageResponse, AccountUsageError> {
    let endpoint = usage_endpoint();
    let response = client
        .get(&endpoint)
        .bearer_auth(&identity.access_token)
        .header("chatgpt-account-id", &identity.account_id)
        .send()
        .map_err(|error| {
            AccountUsageError::new(
                AccountUsageStatus::Network,
                redact_secret_text(&error.to_string()),
            )
        })?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.text().unwrap_or_default();

    if let Some(error) = error_for_usage_status(status, &headers, &body) {
        return Err(error);
    }

    Ok(UsageResponse { headers, body })
}

fn usage_endpoint() -> String {
    std::env::var("CODEX_USAGE_ENDPOINT").unwrap_or_else(|_| CODEX_USAGE_ENDPOINT.to_string())
}

fn request_reset_credits(
    client: &Client,
    identity: &CodexAuthIdentity,
) -> Result<ResetCreditsSummary, AccountUsageError> {
    let endpoint = reset_credits_endpoint();
    let response = client
        .get(&endpoint)
        .bearer_auth(&identity.access_token)
        .header("originator", "Codex Desktop")
        .header("OAI-Product-Sku", "CODEX")
        .header("Accept", "application/json")
        .header("ChatGPT-Account-Id", &identity.account_id)
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
            redact_secret_text(&body),
        ));
    }
    if !status.is_success() {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Error,
            redact_secret_text(&format!("Codex 重置额度请求失败: {status}: {body}")),
        ));
    }

    parse_reset_credits_response(&body)
}

fn reset_credits_endpoint() -> String {
    std::env::var("CODEX_RESET_CREDITS_ENDPOINT")
        .unwrap_or_else(|_| CODEX_RESET_CREDITS_ENDPOINT.to_string())
}

fn error_for_usage_status(
    status: reqwest::StatusCode,
    headers: &HeaderMap,
    body: &str,
) -> Option<AccountUsageError> {
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Some(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            redact_secret_text(body),
        ));
    }
    if status.as_u16() == 429 {
        return Some(AccountUsageError {
            code: AccountUsageStatus::RateLimited,
            message: "Codex 用量接口被限流".to_string(),
            retry_after_secs: parse_retry_after(headers),
        });
    }
    if !status.is_success() {
        return Some(AccountUsageError::new(
            AccountUsageStatus::Error,
            redact_secret_text(&format!("Codex 用量请求失败: {status}: {body}")),
        ));
    }
    None
}

fn load_codex_auth(auth_file: &Path) -> Result<CodexAuthFile, AccountUsageError> {
    let content = std::fs::read_to_string(auth_file).map_err(|error| {
        AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            format!(
                "读取 Codex auth 失败: {}",
                redact_secret_text(&error.to_string())
            ),
        )
    })?;
    serde_json::from_str(&content).map_err(|error| {
        AccountUsageError::new(
            AccountUsageStatus::SchemaChanged,
            format!(
                "解析 Codex auth 失败: {}",
                redact_secret_text(&error.to_string())
            ),
        )
    })
}

fn parse_codex_identity(
    auth_file: &Path,
    auth: &CodexAuthFile,
) -> Result<CodexAuthIdentity, AccountUsageError> {
    let is_api_key = auth.openai_api_key.is_some()
        || auth
            .auth_mode
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case("api_key"))
            .unwrap_or(false);
    if is_api_key {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Unsupported,
            "Codex API key 模式不包含 ChatGPT 账号额度",
        ));
    }

    let tokens = auth.tokens.as_ref().ok_or_else(|| {
        AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "Codex auth 缺少 token 数据",
        )
    })?;
    let id_claims = decode_jwt_claims(&tokens.id_token).unwrap_or_default();
    let access_claims = decode_jwt_claims(&tokens.access_token).unwrap_or_default();
    let auth_claims = id_claims.auth.or(access_claims.auth).unwrap_or_default();
    let profile = id_claims.profile.or(access_claims.profile);
    let email = id_claims
        .email
        .or(access_claims.email)
        .or(profile.and_then(|p| p.email));
    let account_id = tokens
        .account_id
        .clone()
        .or(auth_claims.chatgpt_account_id.clone())
        .ok_or_else(|| {
            AccountUsageError::new(AccountUsageStatus::AuthRequired, "Codex auth 缺少账号 ID")
        })?;

    Ok(CodexAuthIdentity {
        auth_file: auth_file.to_path_buf(),
        access_token: tokens.access_token.clone(),
        refresh_token: tokens.refresh_token.clone(),
        account_id,
        account_label: email,
        plan: auth_claims
            .chatgpt_plan_type
            .as_ref()
            .and_then(plan_value_to_string),
        user_id: auth_claims.chatgpt_user_id.or(auth_claims.user_id),
        expires_at: access_claims.exp.or(id_claims.exp),
    })
}

fn decode_jwt_claims(token: &str) -> Result<JwtClaims, AccountUsageError> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| AccountUsageError::new(AccountUsageStatus::SchemaChanged, "JWT 格式无效"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| {
            AccountUsageError::new(AccountUsageStatus::SchemaChanged, error.to_string())
        })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        AccountUsageError::new(AccountUsageStatus::SchemaChanged, error.to_string())
    })
}

fn refresh_codex_oauth(
    client: &Client,
    auth_file: &Path,
    auth: &mut CodexAuthFile,
    identity: &mut CodexAuthIdentity,
) -> Result<(), AccountUsageError> {
    let endpoint = std::env::var("CODEX_REFRESH_TOKEN_ENDPOINT")
        .unwrap_or_else(|_| CODEX_REFRESH_TOKEN_ENDPOINT.to_string());
    let response = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "client_id": CODEX_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": identity.refresh_token,
        }))
        .send()
        .map_err(|error| {
            AccountUsageError::new(
                AccountUsageStatus::Network,
                redact_secret_text(&error.to_string()),
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            redact_secret_text(&format!("Codex OAuth refresh 失败: {status}: {body}")),
        ));
    }
    let refreshed = response.json::<RefreshResponse>().map_err(|error| {
        AccountUsageError::new(AccountUsageStatus::SchemaChanged, error.to_string())
    })?;
    apply_refreshed_tokens(auth, refreshed)?;
    persist_codex_auth(auth_file, auth)?;
    *identity = parse_codex_identity(auth_file, auth)?;
    Ok(())
}

fn apply_refreshed_tokens(
    auth: &mut CodexAuthFile,
    refreshed: RefreshResponse,
) -> Result<(), AccountUsageError> {
    let tokens = auth.tokens.as_mut().ok_or_else(|| {
        AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "Codex auth 缺少 token 数据",
        )
    })?;
    if let Some(id_token) = refreshed.id_token {
        tokens.id_token = id_token;
    }
    if let Some(access_token) = refreshed.access_token {
        tokens.access_token = access_token;
    }
    if let Some(refresh_token) = refreshed.refresh_token {
        tokens.refresh_token = refresh_token;
    }
    auth.last_refresh = Some(now_rfc3339());
    Ok(())
}

fn persist_codex_auth(auth_file: &Path, auth: &CodexAuthFile) -> Result<(), AccountUsageError> {
    let content = serde_json::to_string_pretty(auth)
        .map_err(|error| AccountUsageError::new(AccountUsageStatus::Error, error.to_string()))?;
    std::fs::write(auth_file, content).map_err(|error| {
        AccountUsageError::new(
            AccountUsageStatus::Error,
            redact_secret_text(&error.to_string()),
        )
    })
}

fn parse_rate_limit_response(
    headers: &HeaderMap,
    body: &str,
) -> Result<Vec<RateLimitSnapshot>, AccountUsageError> {
    // Codex 当前稳定来源是 /wham/usage JSON；旧 event / headers 仅作为互斥兜底。
    let snapshots = parse_wham_usage_response(body)
        .or_else(|| parse_rate_limit_event(body).map(|snapshot| vec![snapshot]))
        .unwrap_or_else(|| parse_all_rate_limits(headers));
    if snapshots.iter().any(has_rate_limit_data) {
        Ok(snapshots)
    } else {
        Err(AccountUsageError::new(
            AccountUsageStatus::SchemaChanged,
            "Codex 响应缺少 rate limit 数据",
        ))
    }
}

fn parse_wham_usage_response(body: &str) -> Option<Vec<RateLimitSnapshot>> {
    let json: Value = serde_json::from_str(body).ok()?;
    let plan_type = json.get("plan_type").and_then(plan_value_to_string);
    let mut snapshots = Vec::new();

    if let Some(snapshot) = parse_wham_limit(
        &json,
        "rate_limit",
        "codex",
        Some("Codex".to_string()),
        plan_type.clone(),
        parse_wham_credits(json.get("credits")),
    ) {
        snapshots.push(snapshot);
    }
    if let Some(snapshot) = parse_wham_limit(
        &json,
        "code_review_rate_limit",
        "codex_code_review",
        Some("Code Review".to_string()),
        plan_type,
        None,
    ) {
        snapshots.push(snapshot);
    }

    (!snapshots.is_empty()).then_some(snapshots)
}

fn parse_wham_limit(
    json: &Value,
    key: &str,
    limit_id: &str,
    limit_name: Option<String>,
    plan_type: Option<String>,
    credits: Option<CreditsSnapshot>,
) -> Option<RateLimitSnapshot> {
    let limit = json.get(key)?;
    let snapshot = RateLimitSnapshot {
        limit_id: limit_id.to_string(),
        limit_name,
        primary: parse_wham_window(limit.get("primary_window")),
        secondary: parse_wham_window(limit.get("secondary_window")),
        credits,
        plan_type,
    };
    has_rate_limit_data(&snapshot).then_some(snapshot)
}

fn parse_wham_window(value: Option<&Value>) -> Option<RateLimitWindow> {
    let window = value?.as_object()?;
    let used_percent = value_f64(window.get("used_percent"))?;
    Some(RateLimitWindow {
        used_percent,
        window_minutes: value_i64(window.get("limit_window_seconds")).map(|seconds| seconds / 60),
        resets_at: value_i64(window.get("reset_at")),
    })
}

fn parse_wham_credits(value: Option<&Value>) -> Option<CreditsSnapshot> {
    let credits = value?.as_object()?;
    Some(CreditsSnapshot {
        has_credits: value_bool(credits.get("has_credits"))?,
        unlimited: value_bool(credits.get("unlimited"))?,
        balance: credits.get("balance").and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        }),
    })
}

fn parse_all_rate_limits(headers: &HeaderMap) -> Vec<RateLimitSnapshot> {
    let mut snapshots = Vec::new();
    snapshots.push(parse_rate_limit_for_limit(headers, "codex", None, true));

    let mut limit_ids = Vec::<String>::new();
    for name in headers.keys() {
        if let Some(limit_id) = header_name_to_limit_id(name.as_str()) {
            if limit_id != "codex" && !limit_ids.contains(&limit_id) {
                limit_ids.push(limit_id);
            }
        }
    }
    for limit_id in limit_ids {
        let header_prefix = limit_id.replace('_', "-");
        let snapshot = parse_rate_limit_for_limit(headers, &limit_id, Some(&header_prefix), false);
        if has_rate_limit_data(&snapshot) {
            snapshots.push(snapshot);
        }
    }
    snapshots
}

fn parse_rate_limit_for_limit(
    headers: &HeaderMap,
    limit_id: &str,
    header_prefix: Option<&str>,
    include_credits: bool,
) -> RateLimitSnapshot {
    let prefix = format!("x-{}", header_prefix.unwrap_or(limit_id));
    RateLimitSnapshot {
        limit_id: normalize_limit_id(limit_id),
        limit_name: header_str(headers, &format!("{prefix}-limit-name")).map(str::to_string),
        primary: parse_rate_limit_window(
            headers,
            &format!("{prefix}-primary-used-percent"),
            &format!("{prefix}-primary-window-minutes"),
            &format!("{prefix}-primary-reset-at"),
        ),
        secondary: parse_rate_limit_window(
            headers,
            &format!("{prefix}-secondary-used-percent"),
            &format!("{prefix}-secondary-window-minutes"),
            &format!("{prefix}-secondary-reset-at"),
        ),
        credits: include_credits
            .then(|| parse_credits_snapshot(headers))
            .flatten(),
        plan_type: None,
    }
}

fn parse_rate_limit_event(payload: &str) -> Option<RateLimitSnapshot> {
    let event: RateLimitEvent = serde_json::from_str(payload).ok()?;
    if event.kind != "codex.rate_limits" {
        return None;
    }
    let (primary, secondary) = event
        .rate_limits
        .map(|details| {
            (
                details.primary.map(event_window_to_window),
                details.secondary.map(event_window_to_window),
            )
        })
        .unwrap_or((None, None));
    Some(RateLimitSnapshot {
        limit_id: event
            .metered_limit_name
            .or(event.limit_name)
            .map(normalize_limit_id)
            .unwrap_or_else(|| "codex".to_string()),
        limit_name: None,
        primary,
        secondary,
        credits: event.credits.map(|credits| CreditsSnapshot {
            has_credits: credits.has_credits,
            unlimited: credits.unlimited,
            balance: credits.balance,
        }),
        plan_type: event.plan_type.as_ref().and_then(plan_value_to_string),
    })
}

fn event_window_to_window(window: RateLimitEventWindow) -> RateLimitWindow {
    RateLimitWindow {
        used_percent: window.used_percent,
        window_minutes: window.window_minutes,
        resets_at: window.reset_at,
    }
}

fn parse_rate_limit_window(
    headers: &HeaderMap,
    used_percent_header: &str,
    window_minutes_header: &str,
    resets_at_header: &str,
) -> Option<RateLimitWindow> {
    let used_percent = header_f64(headers, used_percent_header)?;
    let window_minutes = header_i64(headers, window_minutes_header);
    let resets_at = header_i64(headers, resets_at_header);
    let has_data = used_percent != 0.0
        || window_minutes.is_some_and(|minutes| minutes != 0)
        || resets_at.is_some();
    has_data.then_some(RateLimitWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

fn parse_credits_snapshot(headers: &HeaderMap) -> Option<CreditsSnapshot> {
    Some(CreditsSnapshot {
        has_credits: header_bool(headers, "x-codex-credits-has-credits")?,
        unlimited: header_bool(headers, "x-codex-credits-unlimited")?,
        balance: header_str(headers, "x-codex-credits-balance").map(str::to_string),
    })
}

fn build_account_snapshot(
    identity: CodexAuthIdentity,
    rate_limits: Vec<RateLimitSnapshot>,
    reset_credits: Option<ResetCreditsSummary>,
) -> AccountUsageSnapshot {
    let plan = rate_limits
        .iter()
        .find_map(|snapshot| snapshot.plan_type.clone())
        .or_else(|| identity.plan.clone());
    let reset_at = rate_limits
        .iter()
        .flat_map(|snapshot| [snapshot.primary.as_ref(), snapshot.secondary.as_ref()])
        .flatten()
        .filter_map(|window| window.resets_at)
        .min()
        .and_then(timestamp_to_rfc3339);
    let mut metrics = rate_limits_to_metrics(&rate_limits);
    append_reset_credit_metric(&mut metrics, reset_credits);

    AccountUsageSnapshot {
        provider_id: "codex".to_string(),
        account_key: codex_account_key(&identity),
        account_label: identity.account_label,
        plan,
        status: AccountUsageStatus::Ok,
        source: AccountUsageSource::AuthFile,
        confidence: AccountUsageConfidence::High,
        observed_at: now_rfc3339(),
        period_start: None,
        period_end: None,
        reset_at,
        stale: false,
        error: None,
        metrics,
    }
}

fn append_stale_codex_snapshots_for_partial_failure(
    db_path: &Path,
    snapshots: &mut Vec<AccountUsageSnapshot>,
    error: &AccountUsageError,
) {
    let refreshed_keys = snapshots
        .iter()
        .map(|snapshot| snapshot.account_key.clone())
        .collect::<std::collections::HashSet<_>>();
    let Ok(conn) = rusqlite::Connection::open(db_path) else {
        return;
    };
    let Ok(previous_snapshots) =
        crate::account_usage::store::latest_snapshots_by_provider(&conn, "codex")
    else {
        return;
    };

    snapshots.extend(previous_snapshots.into_iter().filter_map(|mut snapshot| {
        if refreshed_keys.contains(&snapshot.account_key) {
            return None;
        }
        let mut stale_error = error.clone();
        stale_error.message = redact_secret_text(&stale_error.message);
        snapshot.status = AccountUsageStatus::Stale;
        snapshot.observed_at = now_rfc3339();
        snapshot.stale = true;
        snapshot.error = Some(stale_error);
        Some(snapshot)
    }));
}

fn rate_limits_to_metrics(rate_limits: &[RateLimitSnapshot]) -> Vec<AccountUsageMetric> {
    let mut metrics = Vec::new();
    for snapshot in rate_limits {
        if let Some(primary) = &snapshot.primary {
            let label = window_label(snapshot, primary, "primary");
            metrics.push(window_metric(snapshot, primary, "primary", &label));
        }
        if let Some(secondary) = &snapshot.secondary {
            let label = window_label(snapshot, secondary, "secondary");
            metrics.push(window_metric(snapshot, secondary, "secondary", &label));
        }
        if let Some(credits) = &snapshot.credits {
            if credits.has_credits {
                metrics.push(AccountUsageMetric {
                    metric_key: format!("codex.{}.credits", snapshot.limit_id),
                    label: if credits.unlimited {
                        "Credits（无限）".to_string()
                    } else {
                        "Credits".to_string()
                    },
                    unit: "credit".to_string(),
                    scope: AccountUsageMetricScope::Workspace,
                    used: None,
                    limit: None,
                    remaining: credits
                        .balance
                        .as_ref()
                        .and_then(|value| value.parse().ok()),
                    percentage: None,
                    reset_at: None,
                });
            }
        }
    }
    metrics
}

fn append_reset_credit_metric(
    metrics: &mut Vec<AccountUsageMetric>,
    reset_credits: Option<ResetCreditsSummary>,
) {
    let Some(reset_credits) = reset_credits else {
        return;
    };
    if reset_credits.available_count == 0 {
        return;
    }

    metrics.push(AccountUsageMetric {
        metric_key: "codex.reset_credits.available".to_string(),
        label: "Reset credits".to_string(),
        unit: "reset_credit".to_string(),
        scope: AccountUsageMetricScope::Workspace,
        used: None,
        limit: None,
        remaining: Some(reset_credits.available_count as f64),
        percentage: None,
        reset_at: reset_credits.next_expires_at,
    });
}

fn parse_reset_credits_response(body: &str) -> Result<ResetCreditsSummary, AccountUsageError> {
    let response: ResetCreditsResponse = serde_json::from_str(body).map_err(|error| {
        AccountUsageError::new(AccountUsageStatus::SchemaChanged, error.to_string())
    })?;
    Ok(reset_credits_summary(response))
}

fn reset_credits_summary(response: ResetCreditsResponse) -> ResetCreditsSummary {
    let available = response
        .credits
        .into_iter()
        .filter(|credit| {
            credit
                .status
                .as_deref()
                .map(|status| status.eq_ignore_ascii_case("available"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let available_count = response.available_count.unwrap_or(available.len() as u64);
    let next_expires_at = available
        .iter()
        .filter_map(reset_credit_expiry)
        .min_by_key(|(_, timestamp)| *timestamp)
        .map(|(value, _)| value)
        .or_else(|| available.iter().find_map(reset_credit_expiry_value));

    ResetCreditsSummary {
        available_count,
        next_expires_at,
    }
}

fn reset_credit_expiry(credit: &ResetCredit) -> Option<(String, i64)> {
    let value = reset_credit_expiry_value(credit)?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(&value)
        .ok()?
        .timestamp();
    Some((value, timestamp))
}

fn reset_credit_expiry_value(credit: &ResetCredit) -> Option<String> {
    credit
        .expires_at
        .as_ref()
        .or(credit.expires_at_camel.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn window_label(snapshot: &RateLimitSnapshot, window: &RateLimitWindow, key: &str) -> String {
    if snapshot.limit_id == "codex" {
        // Codex 固定按短窗/周窗展示；即使接口缺少窗口时长，也不要回退为通用文案。
        if key == "primary" {
            return "5h window".to_string();
        }
        if key == "secondary" {
            return "7d window".to_string();
        }
        return match window.window_minutes {
            Some(300) => "5h window".to_string(),
            Some(10080) => "7d window".to_string(),
            _ => "Codex window".to_string(),
        };
    }
    if snapshot.limit_id == "codex_code_review" {
        return if key == "primary" {
            "Code Review".to_string()
        } else {
            "Code Review 次级窗口".to_string()
        };
    }
    let fallback = if key == "primary" {
        "Primary window"
    } else {
        "Secondary window"
    };
    snapshot
        .limit_name
        .as_ref()
        .map(|name| format!("{name} {fallback}"))
        .unwrap_or_else(|| fallback.to_string())
}

fn window_metric(
    snapshot: &RateLimitSnapshot,
    window: &RateLimitWindow,
    key: &str,
    label: &str,
) -> AccountUsageMetric {
    AccountUsageMetric {
        metric_key: format!("codex.{}.{}", snapshot.limit_id, key),
        label: label.to_string(),
        unit: "percent".to_string(),
        scope: AccountUsageMetricScope::Workspace,
        used: Some(window.used_percent),
        limit: Some(100.0),
        remaining: Some((100.0 - window.used_percent).max(0.0)),
        percentage: Some(window.used_percent),
        reset_at: window.resets_at.and_then(timestamp_to_rfc3339),
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let header = HeaderName::from_bytes(name.as_bytes()).ok()?;
    headers
        .get(header)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn header_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
    header_str(headers, name)?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    header_str(headers, name)?.parse().ok()
}

fn header_bool(headers: &HeaderMap, name: &str) -> Option<bool> {
    let value = header_str(headers, name)?;
    if value.eq_ignore_ascii_case("true") || value == "1" {
        Some(true)
    } else if value.eq_ignore_ascii_case("false") || value == "0" {
        Some(false)
    } else {
        None
    }
}

fn value_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn value_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn value_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_i64().and_then(|value| match value {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        }),
        Value::String(value) if value.eq_ignore_ascii_case("true") || value == "1" => Some(true),
        Value::String(value) if value.eq_ignore_ascii_case("false") || value == "0" => Some(false),
        _ => None,
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    header_str(headers, "retry-after")?.parse().ok()
}

fn header_name_to_limit_id(header_name: &str) -> Option<String> {
    let prefix = header_name
        .strip_suffix("-primary-used-percent")?
        .strip_prefix("x-")?;
    Some(normalize_limit_id(prefix))
}

fn normalize_limit_id(name: impl AsRef<str>) -> String {
    name.as_ref().trim().to_ascii_lowercase().replace('-', "_")
}

fn has_rate_limit_data(snapshot: &RateLimitSnapshot) -> bool {
    snapshot.primary.is_some() || snapshot.secondary.is_some() || snapshot.credits.is_some()
}

fn plan_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.to_ascii_lowercase()),
        Value::Object(map) => map
            .get("type")
            .or_else(|| map.get("plan"))
            .or_else(|| map.get("raw"))
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_lowercase()),
        _ => None,
    }
}

fn timestamp_to_rfc3339(timestamp: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0).map(|value| value.to_rfc3339())
}

fn codex_account_key(identity: &CodexAuthIdentity) -> String {
    let source_hash = stable_hash(identity.auth_file.to_string_lossy().as_ref());
    format!(
        "{}:{}:{source_hash}",
        identity.account_id,
        identity.user_id.as_deref().unwrap_or("user")
    )
}

fn stable_hash(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn fake_jwt(email: &str, account_id: &str, plan_type: &str) -> String {
        let encode = |value: &Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(value).unwrap())
        };
        let header = encode(&serde_json::json!({ "alg": "none", "typ": "JWT" }));
        let payload = encode(&serde_json::json!({
            "email": email,
            "exp": 4102444800_i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_plan_type": plan_type,
                "chatgpt_user_id": "user-1"
            }
        }));
        format!("{header}.{payload}.sig")
    }

    fn auth_file(path: &Path, account_id: &str) -> CodexAuthFile {
        let jwt = fake_jwt("user@example.com", account_id, "business");
        let auth = CodexAuthFile {
            auth_mode: Some("chatgpt".to_string()),
            openai_api_key: None,
            tokens: Some(CodexTokenData {
                id_token: jwt.clone(),
                access_token: jwt,
                refresh_token: "refresh-token".to_string(),
                account_id: Some(account_id.to_string()),
            }),
            last_refresh: None,
        };
        std::fs::write(path, serde_json::to_string(&auth).unwrap()).unwrap();
        auth
    }

    fn sample_snapshot(account_key: &str) -> AccountUsageSnapshot {
        AccountUsageSnapshot {
            provider_id: "codex".to_string(),
            account_key: account_key.to_string(),
            account_label: None,
            plan: Some("pro".to_string()),
            status: AccountUsageStatus::Ok,
            source: AccountUsageSource::AuthFile,
            confidence: AccountUsageConfidence::High,
            observed_at: now_rfc3339(),
            period_start: None,
            period_end: None,
            reset_at: None,
            stale: false,
            error: None,
            metrics: vec![AccountUsageMetric {
                metric_key: "codex.codex.primary".to_string(),
                label: "5h window".to_string(),
                unit: "percent".to_string(),
                scope: AccountUsageMetricScope::Workspace,
                used: Some(20.0),
                limit: Some(100.0),
                remaining: Some(80.0),
                percentage: Some(20.0),
                reset_at: None,
            }],
        }
    }

    #[test]
    fn test_parse_codex_auth_extracts_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("auth.json");
        let auth = auth_file(&path, "workspace-1");
        let identity = parse_codex_identity(&path, &auth).unwrap();
        assert_eq!(identity.account_id, "workspace-1");
        assert_eq!(identity.account_label.as_deref(), Some("user@example.com"));
        assert_eq!(identity.plan.as_deref(), Some("business"));
        assert!(identity.expires_at.is_some());
    }

    #[test]
    fn test_api_key_auth_is_unsupported() {
        let auth = CodexAuthFile {
            auth_mode: Some("api_key".to_string()),
            openai_api_key: Some("sk-secret".to_string()),
            tokens: None,
            last_refresh: None,
        };
        let error = parse_codex_identity(Path::new("auth.json"), &auth).unwrap_err();
        assert_eq!(error.code, AccountUsageStatus::Unsupported);
    }

    #[test]
    fn test_missing_auth_tokens_is_auth_required() {
        let auth = CodexAuthFile {
            auth_mode: Some("chatgpt".to_string()),
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
        };
        let error = parse_codex_identity(Path::new("auth.json"), &auth).unwrap_err();

        assert_eq!(error.code, AccountUsageStatus::AuthRequired);
    }

    #[test]
    fn test_apply_token_refresh_updates_auth_without_losing_existing_refresh_token() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("auth.json");
        let mut auth = auth_file(&path, "workspace-1");
        let new_jwt = fake_jwt("new@example.com", "workspace-1", "pro");

        apply_refreshed_tokens(
            &mut auth,
            RefreshResponse {
                id_token: Some(new_jwt.clone()),
                access_token: Some("new-access-token".to_string()),
                refresh_token: None,
            },
        )
        .unwrap();
        let tokens = auth.tokens.unwrap();

        assert_eq!(tokens.id_token, new_jwt);
        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "refresh-token");
        assert!(auth.last_refresh.is_some());
    }

    #[test]
    fn test_unauthorized_usage_status_maps_to_auth_required_and_redacts() {
        let error = error_for_usage_status(
            reqwest::StatusCode::UNAUTHORIZED,
            &HeaderMap::new(),
            "authorization: bearer secret-token",
        )
        .unwrap();

        assert_eq!(error.code, AccountUsageStatus::AuthRequired);
        assert!(!error.message.contains("secret-token"));
    }

    #[test]
    fn test_parse_rate_limit_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-primary-used-percent",
            HeaderValue::from_static("12.5"),
        );
        headers.insert(
            "x-codex-primary-window-minutes",
            HeaderValue::from_static("300"),
        );
        headers.insert(
            "x-codex-primary-reset-at",
            HeaderValue::from_static("1704069000"),
        );
        headers.insert(
            "x-codex-secondary-primary-used-percent",
            HeaderValue::from_static("80"),
        );
        headers.insert(
            "x-codex-credits-has-credits",
            HeaderValue::from_static("true"),
        );
        headers.insert(
            "x-codex-credits-unlimited",
            HeaderValue::from_static("false"),
        );
        headers.insert("x-codex-credits-balance", HeaderValue::from_static("13.4"));

        let snapshots = parse_rate_limit_response(&headers, "{}").unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].limit_id, "codex");
        assert_eq!(snapshots[1].limit_id, "codex_secondary");
        assert_eq!(
            snapshots[0].credits.as_ref().unwrap().balance.as_deref(),
            Some("13.4")
        );
    }

    #[test]
    fn test_parse_wham_usage_response() {
        let body = serde_json::json!({
            "plan_type": "pro",
            "email": "user@example.com",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 25,
                    "limit_window_seconds": 18000,
                    "reset_at": 1704069000
                },
                "secondary_window": {
                    "used_percent": 60,
                    "limit_window_seconds": 604800,
                    "reset_at": 1704672000
                }
            },
            "code_review_rate_limit": {
                "primary_window": {
                    "used_percent": 10,
                    "limit_window_seconds": 604800,
                    "reset_at": 1704672000
                }
            },
            "credits": {
                "has_credits": true,
                "unlimited": false,
                "balance": 13.4
            }
        })
        .to_string();

        let snapshots = parse_rate_limit_response(&HeaderMap::new(), &body).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].limit_id, "codex");
        assert_eq!(snapshots[0].plan_type.as_deref(), Some("pro"));
        assert_eq!(snapshots[0].primary.as_ref().unwrap().used_percent, 25.0);
        assert_eq!(
            snapshots[0].primary.as_ref().unwrap().window_minutes,
            Some(300)
        );
        assert_eq!(
            snapshots[0].secondary.as_ref().unwrap().window_minutes,
            Some(10080)
        );
        assert_eq!(
            snapshots[0].credits.as_ref().unwrap().balance.as_deref(),
            Some("13.4")
        );
        assert_eq!(snapshots[1].limit_id, "codex_code_review");
        assert!(snapshots[1].credits.is_none());

        let metrics = rate_limits_to_metrics(&snapshots);
        assert!(metrics.iter().any(|metric| metric.label == "5h window"));
        assert!(metrics.iter().any(|metric| metric.label == "7d window"));
        assert!(metrics.iter().any(|metric| metric.label == "Code Review"));
    }

    #[test]
    fn test_parse_reset_credits_response_uses_available_count_and_earliest_expiry() {
        let body = serde_json::json!({
            "available_count": 3,
            "credits": [
                { "status": "available", "expires_at": "2026-07-18T00:00:00Z" },
                { "status": "redeemed", "expires_at": "2026-07-01T00:00:00Z" },
                { "status": "available", "expires_at": "2026-07-12T00:00:00Z" }
            ]
        })
        .to_string();

        let summary = parse_reset_credits_response(&body).unwrap();

        assert_eq!(summary.available_count, 3);
        assert_eq!(
            summary.next_expires_at.as_deref(),
            Some("2026-07-12T00:00:00Z")
        );
    }

    #[test]
    fn test_append_reset_credit_metric() {
        let mut metrics = Vec::new();
        append_reset_credit_metric(
            &mut metrics,
            Some(ResetCreditsSummary {
                available_count: 2,
                next_expires_at: Some("2026-07-12T00:00:00Z".to_string()),
            }),
        );

        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].metric_key, "codex.reset_credits.available");
        assert_eq!(metrics[0].remaining, Some(2.0));
        assert_eq!(metrics[0].reset_at.as_deref(), Some("2026-07-12T00:00:00Z"));
    }

    #[test]
    fn test_codex_labels_do_not_depend_on_window_minutes() {
        let metrics = rate_limits_to_metrics(&[RateLimitSnapshot {
            limit_id: "codex".to_string(),
            limit_name: None,
            primary: Some(RateLimitWindow {
                used_percent: 2.0,
                window_minutes: None,
                resets_at: None,
            }),
            secondary: Some(RateLimitWindow {
                used_percent: 8.0,
                window_minutes: None,
                resets_at: None,
            }),
            credits: None,
            plan_type: None,
        }]);

        assert!(metrics.iter().any(|metric| metric.label == "5h window"));
        assert!(metrics.iter().any(|metric| metric.label == "7d window"));
        assert!(!metrics
            .iter()
            .any(|metric| metric.label.contains("额度窗口")));
    }

    #[test]
    fn test_parse_rate_limit_event() {
        let payload = serde_json::json!({
            "type": "codex.rate_limits",
            "plan_type": "pro",
            "metered_limit_name": "codex_other",
            "rate_limits": {
                "primary": { "used_percent": 50.0, "window_minutes": 300, "reset_at": 1704069000 }
            }
        })
        .to_string();
        let snapshot = parse_rate_limit_event(&payload).unwrap();
        assert_eq!(snapshot.limit_id, "codex_other");
        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        assert_eq!(snapshot.primary.unwrap().used_percent, 50.0);
    }

    #[test]
    fn test_workspace_identity_includes_source_path() {
        let temp = tempfile::tempdir().unwrap();
        let path_a = temp.path().join("a").join("auth.json");
        let path_b = temp.path().join("b").join("auth.json");
        std::fs::create_dir_all(path_a.parent().unwrap()).unwrap();
        std::fs::create_dir_all(path_b.parent().unwrap()).unwrap();
        let auth_a = auth_file(&path_a, "workspace-1");
        let auth_b = auth_file(&path_b, "workspace-1");
        let key_a = codex_account_key(&parse_codex_identity(&path_a, &auth_a).unwrap());
        let key_b = codex_account_key(&parse_codex_identity(&path_b, &auth_b).unwrap());
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn test_partial_failure_appends_stale_previous_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("account_usage.sqlite");
        crate::db::init_db(&db_path).unwrap();
        let refreshed = sample_snapshot("refreshed-account");
        let failed = sample_snapshot("failed-account");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::account_usage::store::upsert_snapshot(&conn, &refreshed).unwrap();
        crate::account_usage::store::upsert_snapshot(&conn, &failed).unwrap();
        drop(conn);

        let mut snapshots = vec![refreshed];
        let error = AccountUsageError::new(AccountUsageStatus::SchemaChanged, "schema changed");
        append_stale_codex_snapshots_for_partial_failure(&db_path, &mut snapshots, &error);

        let stale = snapshots
            .iter()
            .find(|snapshot| snapshot.account_key == "failed-account")
            .unwrap();
        assert!(stale.stale);
        assert_eq!(stale.status, AccountUsageStatus::Stale);
        assert_eq!(
            stale.error.as_ref().unwrap().code,
            AccountUsageStatus::SchemaChanged
        );
    }

    #[test]
    fn test_schema_failure_when_no_rate_limit_data() {
        let headers = HeaderMap::new();
        let error = parse_rate_limit_response(&headers, "{}").unwrap_err();
        assert_eq!(error.code, AccountUsageStatus::SchemaChanged);
    }
}
