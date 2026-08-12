use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, COOKIE, RETRY_AFTER};

use crate::account_usage::{
    redact_secret_text, AccountUsageCapability, AccountUsageConfidence, AccountUsageError,
    AccountUsageMetric, AccountUsageMetricScope, AccountUsageProvider, AccountUsageProviderInfo,
    AccountUsageProviderState, AccountUsageRefreshContext, AccountUsageResult,
    AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus, CredentialRequirement,
};

/// OpenCode Go 控制台地址
const OPENCODE_ORIGIN: &str = "https://opencode.ai";

/// 请求网页控制台时使用的浏览器标识
const OPENCODE_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

/// OpenCode Go 网页用量 Provider
pub struct OpenCodeGoUsageProvider;

impl AccountUsageProvider for OpenCodeGoUsageProvider {
    /// 返回稳定的 Provider ID
    fn id(&self) -> &'static str {
        "opencode-go"
    }

    /// 返回默认刷新间隔秒数
    fn default_refresh_interval_secs(&self) -> u64 {
        10 * 60
    }

    /// 根据凭据状态构建 Provider 信息
    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "OpenCode Go".to_string(),
            enabled: state.enabled,
            show_in_menu_bar: state.show_in_menu_bar,
            available: state.credential_ref.is_some() && state.credential_label.is_some(),
            source: AccountUsageSource::InternalApi,
            confidence: AccountUsageConfidence::Low,
            capabilities: vec![
                AccountUsageCapability::AccountUsage,
                AccountUsageCapability::AccountQuota,
                AccountUsageCapability::InternalApi,
                AccountUsageCapability::CookieRequired,
            ],
            credential_requirements: vec![
                CredentialRequirement {
                    key: "workspace_id".to_string(),
                    label: "Workspace ID".to_string(),
                    secret: false,
                    required: true,
                    description: "Go 页面 URL 中 /workspace/ 后的 ID".to_string(),
                },
                CredentialRequirement {
                    key: "auth_cookie".to_string(),
                    label: "OpenCode auth Cookie".to_string(),
                    secret: true,
                    required: true,
                    description: "opencode.ai 的 auth Cookie 值".to_string(),
                },
            ],
            experimental: true,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    /// OpenCode Go 不自动读取浏览器 Cookie
    fn detect(&self) -> bool {
        false
    }

    /// 使用已保存的 Workspace ID 与 Cookie 拉取三档用量
    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let conn = rusqlite::Connection::open(&context.db_path).map_err(|error| {
            AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
        })?;
        let state = crate::account_usage::store::get_provider_state(&conn, self.id())?.ok_or_else(
            || AccountUsageError::new(AccountUsageStatus::AuthRequired, "OpenCode Go 凭据未配置"),
        )?;
        let credential_ref = state.credential_ref.ok_or_else(|| {
            AccountUsageError::new(
                AccountUsageStatus::AuthRequired,
                "OpenCode Go Cookie 未配置",
            )
        })?;
        let workspace_id = state
            .credential_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AccountUsageError::new(
                    AccountUsageStatus::AuthRequired,
                    "OpenCode Go Workspace ID 未配置",
                )
            })?;
        let auth_cookie = context.credentials.load_secret(&credential_ref)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(OPENCODE_USER_AGENT)
            .build()
            .map_err(|error| {
                AccountUsageError::new(AccountUsageStatus::Error, error.to_string())
            })?;

        fetch_opencode_go_usage(&client, workspace_id, &auth_cookie)
    }
}

/// 已解析的单个额度窗口
#[derive(Debug, Clone, PartialEq)]
struct GoUsageWindow {
    /// 已使用百分比
    usage_percent: f64,
    /// 距离重置的秒数
    reset_in_secs: f64,
}

/// 请求 OpenCode Go 控制台并转换为账号用量快照
fn fetch_opencode_go_usage(
    client: &Client,
    workspace_id: &str,
    auth_cookie: &str,
) -> AccountUsageResult {
    let url = dashboard_url(workspace_id)?;
    let cookie = normalize_auth_cookie(auth_cookie)?;
    let response = client
        .get(url)
        .header(ACCEPT, "text/html")
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .header(COOKIE, cookie)
        .send()
        .map_err(|error| {
            AccountUsageError::new(
                AccountUsageStatus::Network,
                redact_secret_text(&error.to_string()),
            )
        })?;
    let status = response.status();
    let final_path = response.url().path().to_string();
    let retry_after_secs = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    if final_path.starts_with("/auth/") || status.as_u16() == 401 {
        return Err(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "OpenCode auth Cookie 已失效",
        ));
    }
    if status.as_u16() == 403 {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Forbidden,
            "当前账号无权访问该 OpenCode Workspace",
        ));
    }
    if status.as_u16() == 429 {
        return Err(AccountUsageError {
            code: AccountUsageStatus::RateLimited,
            message: "OpenCode Go 请求过于频繁".to_string(),
            retry_after_secs,
        });
    }
    if !status.is_success() {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Error,
            format!("OpenCode Go 请求失败: HTTP {status}"),
        ));
    }

    let html = response.text().map_err(|error| {
        AccountUsageError::new(
            AccountUsageStatus::Network,
            redact_secret_text(&error.to_string()),
        )
    })?;
    let observed_at = Utc::now();
    parse_opencode_go_snapshot(&html, workspace_id, observed_at).map(|snapshot| vec![snapshot])
}

/// 构建经过路径转义的 Workspace Go 页面地址
fn dashboard_url(workspace_id: &str) -> Result<reqwest::Url, AccountUsageError> {
    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        return Err(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "OpenCode Go Workspace ID 未配置",
        ));
    }
    let mut url = reqwest::Url::parse(OPENCODE_ORIGIN)
        .map_err(|error| AccountUsageError::new(AccountUsageStatus::Error, error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| AccountUsageError::new(AccountUsageStatus::Error, "OpenCode Go 页面地址无效"))?
        .extend(["workspace", workspace_id, "go"]);
    Ok(url)
}

/// 将用户输入规范化为只包含 auth 的 Cookie 请求头
fn normalize_auth_cookie(input: &str) -> Result<String, AccountUsageError> {
    let input = input.trim();
    let value = input
        .split(';')
        .map(str::trim)
        .find_map(|item| item.strip_prefix("auth="))
        .unwrap_or(input)
        .trim();
    if value.is_empty() || value.contains(';') || value.chars().any(char::is_control) {
        return Err(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "OpenCode auth Cookie 格式无效",
        ));
    }
    Ok(format!("auth={value}"))
}

/// 从页面 hydration 数据解析三档用量并构建快照
fn parse_opencode_go_snapshot(
    html: &str,
    workspace_id: &str,
    observed_at: DateTime<Utc>,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    if html.contains("data-slot=\"promo-description\"") {
        return Err(AccountUsageError::new(
            AccountUsageStatus::Unsupported,
            "当前 Workspace 未订阅 OpenCode Go",
        ));
    }

    let hydration_windows = (
        parse_window_usage(html, "rollingUsage"),
        parse_window_usage(html, "weeklyUsage"),
        parse_window_usage(html, "monthlyUsage"),
    );
    let windows = match hydration_windows {
        (Some(rolling), Some(weekly), Some(monthly)) => Some((rolling, weekly, monthly)),
        _ => parse_data_slot_windows(html),
    };
    let Some((rolling, weekly, monthly)) = windows else {
        return Err(AccountUsageError::new(
            AccountUsageStatus::SchemaChanged,
            "OpenCode Go 页面缺少 rollingUsage、weeklyUsage 或 monthlyUsage",
        ));
    };

    let metrics = vec![
        usage_metric("rolling", "5h window", &rolling, &observed_at),
        usage_metric("weekly", "Weekly", &weekly, &observed_at),
        usage_metric("monthly", "Monthly", &monthly, &observed_at),
    ];
    let reset_at = metrics.first().and_then(|metric| metric.reset_at.clone());

    Ok(AccountUsageSnapshot {
        provider_id: "opencode-go".to_string(),
        account_key: format!("workspace:{workspace_id}"),
        account_label: Some(workspace_id.to_string()),
        plan: Some("Go".to_string()),
        status: AccountUsageStatus::Ok,
        source: AccountUsageSource::InternalApi,
        confidence: AccountUsageConfidence::Low,
        observed_at: observed_at.to_rfc3339(),
        period_start: None,
        period_end: None,
        reset_at,
        stale: false,
        error: None,
        metrics,
    })
}

/// 从指定 hydration 对象读取百分比与重置秒数
fn parse_window_usage(html: &str, field: &str) -> Option<GoUsageWindow> {
    for marker in [format!("{field}:"), format!("\"{field}\":")] {
        let Some(index) = html.find(&marker) else {
            continue;
        };
        let remainder = &html[index + marker.len()..];
        let object_start = remainder.find('{')? + 1;
        let object = &remainder[object_start..];
        let object_end = object.find('}')?;
        let object = &object[..object_end];
        let usage_percent = parse_number_field(object, "usagePercent")?;
        let reset_in_secs = parse_number_field(object, "resetInSec")?;

        return Some(GoUsageWindow {
            usage_percent,
            reset_in_secs,
        });
    }
    None
}

/// 按官方页面顺序解析新版 data-slot 用量条目
fn parse_data_slot_windows(html: &str) -> Option<(GoUsageWindow, GoUsageWindow, GoUsageWindow)> {
    let usage = &html[html.find("data-slot=\"usage\"")?..];
    let mut items = usage.split("data-slot=\"usage-item\"").skip(1);
    let rolling = parse_data_slot_item(items.next()?)?;
    let weekly = parse_data_slot_item(items.next()?)?;
    let monthly = parse_data_slot_item(items.next()?)?;
    Some((rolling, weekly, monthly))
}

/// 从单个 data-slot 条目解析百分比与重置时间
fn parse_data_slot_item(item: &str) -> Option<GoUsageWindow> {
    let usage_percent = slot_text(item, "usage-value").and_then(|text| first_number(&text))?;
    let reset_in_secs = if item.contains("data-slot=\"reset-now\"") {
        0.0
    } else {
        slot_text(item, "reset-time").and_then(|text| parse_duration_secs(&text))?
    };
    Some(GoUsageWindow {
        usage_percent,
        reset_in_secs,
    })
}

/// 提取指定 data-slot 元素中的可见文本
fn slot_text(fragment: &str, slot: &str) -> Option<String> {
    let marker = format!("data-slot=\"{slot}\"");
    let element = &fragment[fragment.find(&marker)? + marker.len()..];
    let content = &element[element.find('>')? + 1..];
    let content = &content[..content.find("</span>")?];
    let mut text = String::with_capacity(content.len());
    let mut inside_tag = false;
    for ch in content.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => text.push(ch),
            _ => {}
        }
    }
    Some(text)
}

/// 读取文本中出现的第一个有限数字
fn first_number(text: &str) -> Option<f64> {
    text.split(|ch: char| !ch.is_ascii_digit() && !matches!(ch, '+' | '-' | '.' | 'e' | 'E'))
        .filter(|value| !value.is_empty())
        .find_map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
        })
}

/// 将英文天、小时、分钟和秒组合转换为秒数
fn parse_duration_secs(text: &str) -> Option<f64> {
    let normalized = text.to_ascii_lowercase();
    if normalized.contains("reset-now") || normalized.contains("reset now") {
        return Some(0.0);
    }

    let mut total = 0.0;
    let mut pending_number = None;
    let mut matched = false;
    for token in normalized.split_whitespace() {
        let token = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.');
        if let Ok(value) = token.parse::<f64>() {
            pending_number = value.is_finite().then_some(value);
            continue;
        }
        let Some(value) = pending_number.take() else {
            continue;
        };
        let multiplier = if token.starts_with("day") {
            86_400.0
        } else if token.starts_with("hour") {
            3_600.0
        } else if token.starts_with("minute") {
            60.0
        } else if token.starts_with("second") {
            1.0
        } else {
            continue;
        };
        total += value * multiplier;
        matched = true;
    }
    matched.then_some(total)
}

/// 从扁平 JavaScript 对象中读取有限数字字段
fn parse_number_field(object: &str, field: &str) -> Option<f64> {
    for marker in [format!("{field}:"), format!("\"{field}\":")] {
        let Some(index) = object.find(&marker) else {
            continue;
        };
        let value = object[index + marker.len()..].trim_start();
        let end = value
            .find(|ch: char| !ch.is_ascii_digit() && !matches!(ch, '+' | '-' | '.' | 'e' | 'E'))
            .unwrap_or(value.len());
        return value[..end]
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite());
    }
    None
}

/// 将单个窗口转换为统一百分比指标
fn usage_metric(
    key: &str,
    label: &str,
    window: &GoUsageWindow,
    observed_at: &DateTime<Utc>,
) -> AccountUsageMetric {
    let percentage = window.usage_percent.clamp(0.0, 100.0);
    let reset_in_secs = window.reset_in_secs.max(0.0).ceil() as i64;
    let reset_at = *observed_at + chrono::Duration::seconds(reset_in_secs);

    AccountUsageMetric {
        metric_key: format!("opencode-go.{key}"),
        label: label.to_string(),
        unit: "percent".to_string(),
        scope: AccountUsageMetricScope::Workspace,
        used: Some(percentage),
        limit: Some(100.0),
        remaining: Some(100.0 - percentage),
        percentage: Some(percentage),
        reset_at: Some(reset_at.to_rfc3339()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构建测试用 Provider 状态
    fn provider_state(configured: bool) -> AccountUsageProviderState {
        AccountUsageProviderState {
            provider_id: "opencode-go".to_string(),
            enabled: true,
            show_in_menu_bar: false,
            refresh_interval_secs: 600,
            last_refresh_at: None,
            retry_after_until: None,
            credential_ref: configured.then(|| "opencode-go:workspace-1:auth_cookie".to_string()),
            credential_label: configured.then(|| "workspace-1".to_string()),
            auto_discovery_enabled: false,
        }
    }

    /// 验证 Provider 明确标记内部接口和实验状态
    #[test]
    fn test_info_marks_internal_api_as_experimental() {
        let info = OpenCodeGoUsageProvider.info(&provider_state(true));

        assert!(info.available);
        assert!(info.experimental);
        assert_eq!(info.source, AccountUsageSource::InternalApi);
        assert_eq!(info.credential_requirements.len(), 2);
    }

    /// 验证 hydration 字段顺序变化不会影响三档额度解析
    #[test]
    fn test_parse_hydration_usage_windows() {
        let html = r#"
            rollingUsage:$R[1]={status:"under",resetInSec:3600,usagePercent:12.5}
            weeklyUsage:$R[2]={usagePercent:45,resetInSec:7200,status:"under"}
            monthlyUsage:$R[3]={resetInSec:10800,status:"under",usagePercent:80}
        "#;
        let observed_at = DateTime::parse_from_rfc3339("2026-08-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let snapshot = parse_opencode_go_snapshot(html, "workspace-1", observed_at).unwrap();

        assert_eq!(snapshot.metrics.len(), 3);
        assert_eq!(snapshot.metrics[0].percentage, Some(12.5));
        assert_eq!(snapshot.metrics[0].remaining, Some(87.5));
        assert_eq!(
            snapshot.metrics[0].reset_at.as_deref(),
            Some("2026-08-07T01:00:00+00:00")
        );
        assert_eq!(snapshot.metrics[1].percentage, Some(45.0));
        assert_eq!(snapshot.metrics[2].percentage, Some(80.0));
    }

    /// 验证缺少任一额度窗口时报告页面结构变化
    #[test]
    fn test_missing_usage_window_is_schema_error() {
        let html = r#"
            rollingUsage:$R[1]={usagePercent:10,resetInSec:3600}
            weeklyUsage:$R[2]={usagePercent:20,resetInSec:7200}
        "#;

        let error = parse_opencode_go_snapshot(html, "workspace-1", Utc::now()).unwrap_err();

        assert_eq!(error.code, AccountUsageStatus::SchemaChanged);
    }

    /// 验证新版 data-slot 页面可以回退解析三档额度
    #[test]
    fn test_parse_data_slot_usage_windows() {
        let html = r#"
            <div data-slot="usage">
                <div data-slot="usage-item">
                    <span data-slot="usage-value"><!--$-->1.5<!--/-->%</span>
                    <span data-slot="reset-time"><!--$-->Resets in<!--/--> 1 hour 30 minutes</span>
                </div>
                <div data-slot="usage-item">
                    <span data-slot="usage-value">20%</span>
                    <span data-slot="reset-time">Resets in 6 days 2 hours</span>
                </div>
                <div data-slot="usage-item">
                    <span data-slot="usage-value">40%</span>
                    <span data-slot="reset-now">reset-now</span>
                </div>
            </div>
        "#;
        let observed_at = DateTime::parse_from_rfc3339("2026-08-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let snapshot = parse_opencode_go_snapshot(html, "workspace-1", observed_at).unwrap();

        assert_eq!(snapshot.metrics[0].percentage, Some(1.5));
        assert_eq!(
            snapshot.metrics[0].reset_at.as_deref(),
            Some("2026-08-07T01:30:00+00:00")
        );
        assert_eq!(snapshot.metrics[1].percentage, Some(20.0));
        assert_eq!(
            snapshot.metrics[2].reset_at.as_deref(),
            Some("2026-08-07T00:00:00+00:00")
        );
    }

    /// 验证只向服务端发送 auth Cookie
    #[test]
    fn test_normalize_auth_cookie() {
        assert_eq!(normalize_auth_cookie("secret").unwrap(), "auth=secret");
        assert_eq!(
            normalize_auth_cookie("theme=dark; auth=secret; locale=en").unwrap(),
            "auth=secret"
        );
        assert!(normalize_auth_cookie(" ").is_err());
    }

    /// 验证 Workspace ID 被编码为单个 URL 路径段
    #[test]
    fn test_dashboard_url_encodes_workspace_id() {
        let url = dashboard_url("workspace/test").unwrap();

        assert_eq!(
            url.as_str(),
            "https://opencode.ai/workspace/workspace%2Ftest/go"
        );
    }
}
