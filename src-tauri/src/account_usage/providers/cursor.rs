use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{json, Value};

use super::cursor_auth::{self, CursorCredential};
use crate::account_usage::credentials::CredentialMetadata;
use crate::account_usage::{
    now_rfc3339, AccountUsageCapability, AccountUsageConfidence, AccountUsageError,
    AccountUsageMetric, AccountUsageMetricScope, AccountUsageProvider, AccountUsageProviderInfo,
    AccountUsageProviderState, AccountUsageRefreshContext, AccountUsageResult,
    AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus,
};

/** Cursor 桌面端的只读额度接口 */
const DASHBOARD_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService";
/** 网页端提供原始 membershipType，区分旧 Ultra 与 unified_ultra */
const SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";

/**
 * 复用 Cursor 本机登录读取账号额度，后台不触发系统授权
 */
pub struct CursorUsageProvider;

impl AccountUsageProvider for CursorUsageProvider {
    /**
     * 返回稳定的 Provider 标识
     */
    fn id(&self) -> &'static str {
        "cursor"
    }

    /**
     * 返回自动刷新间隔，单位为秒
     */
    fn default_refresh_interval_secs(&self) -> u64 {
        300
    }

    /**
     * 根据保存的授权状态返回能力，枚举设置时不读取钥匙串
     */
    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "Cursor".to_string(),
            enabled: state.enabled,
            show_in_menu_bar: state.show_in_menu_bar,
            available: state.credential_ref.as_deref() == Some(cursor_auth::CREDENTIAL_REF),
            source: AccountUsageSource::InternalApi,
            confidence: AccountUsageConfidence::Medium,
            capabilities: vec![
                AccountUsageCapability::AccountUsage,
                AccountUsageCapability::AccountQuota,
                AccountUsageCapability::InternalApi,
                AccountUsageCapability::LocalCredentialImport,
            ],
            credential_requirements: vec![],
            experimental: true,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    /**
     * 仅检查 Cursor 数据库是否存在，不触发登录或授权
     */
    fn detect(&self) -> bool {
        cursor_auth::database_path().is_some_and(|path| path.is_file())
    }

    /**
     * 用户主动授权时导入本机凭据并加密保存，返回凭据引用
     */
    fn authorize(
        &self,
        context: AccountUsageRefreshContext,
    ) -> Result<CredentialMetadata, AccountUsageError> {
        let credential = cursor_auth::read_local(true)?;
        cursor_auth::save(&context.credentials, &credential, true)
    }

    /**
     * 查询保存账号的额度，仅在授权失效时尝试无交互读取同一账号的新凭据
     */
    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let conn = rusqlite::Connection::open(&context.db_path)?;
        let state = crate::account_usage::store::get_provider_state(&conn, self.id())?;
        if state
            .as_ref()
            .and_then(|state| state.credential_ref.as_deref())
            != Some(cursor_auth::CREDENTIAL_REF)
        {
            return Err(auth_expired("请点击授权读取 Cursor 登录，无需填写 Cookie"));
        }
        let credential = cursor_auth::load(&context.credentials)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("TokenBurger Cursor account-usage")
            .build()
            .map_err(|_| usage_error(AccountUsageStatus::Network, "无法初始化 Cursor 请求"))?;
        refresh_with_recovery(
            &credential,
            |credential| fetch_cursor_usage(&client, credential),
            || cursor_auth::read_local(false),
            |credential| cursor_auth::save(&context.credentials, credential, false).map(|_| ()),
        )
    }
}

/**
 * 只对鉴权失败重读本机凭据；同一账号的新 token 验证成功后才保存，不轮换 refresh token
 */
fn refresh_with_recovery(
    credential: &CursorCredential,
    fetch: impl Fn(&CursorCredential) -> AccountUsageResult,
    discover: impl FnOnce() -> Result<CursorCredential, AccountUsageError>,
    save: impl FnOnce(&CursorCredential) -> Result<(), AccountUsageError>,
) -> AccountUsageResult {
    match fetch(credential) {
        Err(error) if error.code == AccountUsageStatus::AuthRequired => {
            let fresh = discover().map_err(|_| {
                auth_expired("Cursor 授权已过期，请先在 Cursor 登录，再点击重新授权")
            })?;
            if fresh.user_id != credential.user_id {
                return Err(auth_expired("Cursor 账号已变更，请点击重新授权"));
            }
            if fresh.access_token == credential.access_token {
                return Err(auth_expired(
                    "Cursor 授权已过期，请先在 Cursor 登录，再点击重新授权",
                ));
            }
            let snapshots = fetch(&fresh)?;
            save(&fresh)?;
            Ok(snapshots)
        }
        result => result,
    }
}

/**
 * 并行读取套餐元信息及周期额度，再为新版套餐补充共享池的产品明细
 */
fn fetch_cursor_usage(client: &Client, credential: &CursorCredential) -> AccountUsageResult {
    let cookie = session_cookie(credential);
    let (summary, plan, sand, period) = std::thread::scope(|scope| {
        let summary =
            scope.spawn(|| request_json(client.get(SUMMARY_URL).header("cookie", &cookie)));
        let plan = scope.spawn(|| rpc(client, credential, "GetPlanInfo"));
        let sand = scope.spawn(|| rpc(client, credential, "GetSandUsageStatus"));
        let period = scope.spawn(|| rpc(client, credential, "GetCurrentPeriodUsage"));
        (summary.join(), plan.join(), sand.join(), period.join())
    });
    let thread_error = || usage_error(AccountUsageStatus::Error, "Cursor 查询未完成，请重试");
    let summary = summary.map_err(|_| thread_error())?;
    let plan = plan.map_err(|_| thread_error())?;
    let sand = sand.map_err(|_| thread_error())?;
    let period = period.map_err(|_| thread_error())?;
    let period = match period {
        Err(error) if error.code == AccountUsageStatus::AuthRequired => request_json(
            client
                .post("https://cursor.com/api/dashboard/get-current-period-usage")
                .header("cookie", &cookie)
                .json(&json!({})),
        ),
        result => result,
    };
    // 仅全部关键查询均拒绝时才判定整个登录过期，单个可选接口无权限不能拖垮已有数据
    if [&summary, &plan, &sand, &period].iter().all(
        |result| matches!(result, Err(error) if error.code == AccountUsageStatus::AuthRequired),
    ) {
        return Err(auth_expired("Cursor 授权已过期，请点击重新授权"));
    }
    let kind = resolve_plan_kind(&summary, &plan, &sand)?;
    let shared = if kind == PlanKind::Unified {
        Some(rpc(client, credential, "GetXaiUserUsageSnapshot"))
    } else {
        None
    };
    if let Some(error) = failed_sources_error(&[&period, shared.as_ref().unwrap_or(&summary)]) {
        return Err(error);
    }
    let snapshot = parse_snapshot(
        credential,
        kind,
        summary.as_ref().ok(),
        plan.as_ref().ok(),
        period.as_ref().ok(),
        shared.as_ref().and_then(|result| result.as_ref().ok()),
    )?;
    Ok(vec![snapshot])
}

/**
 * 请求指定只读 RPC，返回 JSON，凭据始终只发往固定 Cursor 主机
 */
fn rpc(
    client: &Client,
    credential: &CursorCredential,
    method: &str,
) -> Result<Value, AccountUsageError> {
    request_json(
        client
            .post(format!("{DASHBOARD_URL}/{method}"))
            .bearer_auth(&credential.access_token)
            .header("Connect-Protocol-Version", "1")
            .json(&json!({})),
    )
}

/**
 * 执行 JSON 请求并分类错误，不记录服务端响应正文或凭据，不跟随重定向
 */
fn request_json(request: RequestBuilder) -> Result<Value, AccountUsageError> {
    let response = request
        .header("accept", "application/json")
        .header("origin", "https://cursor.com")
        .header("referer", "https://cursor.com/dashboard/spending")
        .send()
        .map_err(|_| usage_error(AccountUsageStatus::Network, "无法连接 Cursor，请稍后重试"))?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 || status.is_redirection() {
        return Err(auth_expired("Cursor 授权已过期，请点击重新授权"));
    }
    if status.as_u16() == 429 {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(1, 3600);
        let mut error = usage_error(
            AccountUsageStatus::RateLimited,
            "Cursor 请求过于频繁，稍后将自动重试",
        );
        error.retry_after_secs = Some(retry);
        return Err(error);
    }
    if !status.is_success() {
        return Err(usage_error(
            AccountUsageStatus::Network,
            "Cursor 用量暂不可用，请稍后重试",
        ));
    }
    response.json().map_err(|_| {
        usage_error(
            AccountUsageStatus::SchemaChanged,
            "Cursor 返回了无法识别的用量数据",
        )
    })
}

/**
 * 使用本机账号与 token 生成网页会话，兼容已含用户标识的 token
 */
fn session_cookie(credential: &CursorCredential) -> String {
    let pair = if credential.access_token.contains("::") {
        credential.access_token.clone()
    } else {
        format!("{}::{}", credential.user_id, credential.access_token)
    };
    let encoded: String = pair
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("WorkosCursorSessionToken={encoded}")
}

/**
 * 接口明确区分的套餐模式，未知值不按名称或账期长度猜测
 */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlanKind {
    Unified,
    Legacy,
    Unknown,
}

/**
 * 所有候选来源失败时保留真实错误，优先传递限流及退避时间，其次是网络错误
 */
fn failed_sources_error(
    results: &[&Result<Value, AccountUsageError>],
) -> Option<AccountUsageError> {
    if results.iter().any(|result| result.is_ok()) {
        return None;
    }
    results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .max_by_key(|error| match error.code {
            AccountUsageStatus::RateLimited => 3,
            AccountUsageStatus::Network => 2,
            _ => 1,
        })
        .cloned()
}

/**
 * 从套餐接口结果识别模式，只有正常返回未知字段时才报告协议变化
 */
fn resolve_plan_kind(
    summary: &Result<Value, AccountUsageError>,
    plan: &Result<Value, AccountUsageError>,
    sand: &Result<Value, AccountUsageError>,
) -> Result<PlanKind, AccountUsageError> {
    let kind = plan_kind(
        summary.as_ref().ok(),
        plan.as_ref().ok(),
        sand.as_ref().ok(),
    );
    if kind != PlanKind::Unknown {
        return Ok(kind);
    }
    Err(
        failed_sources_error(&[summary, plan, sand]).unwrap_or_else(|| {
            usage_error(
                AccountUsageStatus::SchemaChanged,
                "暂时无法识别 Cursor 套餐，请稍后刷新",
            )
        }),
    )
}

/**
 * 优先使用服务端统一套餐标记，再依据原始 membershipType 与用量周期枚举判断
 */
fn plan_kind(summary: Option<&Value>, plan: Option<&Value>, sand: Option<&Value>) -> PlanKind {
    if let Some(unified) = sand
        .and_then(|value| value.get("isUnified"))
        .and_then(Value::as_bool)
    {
        return if unified {
            PlanKind::Unified
        } else {
            PlanKind::Legacy
        };
    }
    if let Some(membership) = summary
        .and_then(|value| value.get("membershipType"))
        .and_then(Value::as_str)
    {
        if membership.starts_with("unified_") || matches!(membership, "plus" | "premium" | "super")
        {
            return PlanKind::Unified;
        }
        if matches!(
            membership,
            "free"
                | "hobby"
                | "start"
                | "pro"
                | "pro_plus"
                | "ultra"
                | "enterprise"
                | "free_trial"
                | "pro_student"
                | "express"
                | "pro_monthly"
                | "pro_yearly"
        ) {
            return PlanKind::Legacy;
        }
    }
    match plan.and_then(|value| value.pointer("/planInfo/includedUsagePeriod")) {
        Some(Value::String(value)) if value == "INCLUDED_USAGE_PERIOD_WEEKLY" => PlanKind::Unified,
        Some(Value::String(value)) if value == "INCLUDED_USAGE_PERIOD_MONTHLY" => PlanKind::Legacy,
        Some(Value::Number(value)) if value.as_u64() == Some(2) => PlanKind::Unified,
        Some(Value::Number(value)) if value.as_u64() == Some(1) => PlanKind::Legacy,
        _ => PlanKind::Unknown,
    }
}

/**
 * 将真实额度映射为通用指标，新版共享额度与产品消耗分别表达，旧版保持独立额度池
 */
fn parse_snapshot(
    credential: &CursorCredential,
    kind: PlanKind,
    summary: Option<&Value>,
    plan: Option<&Value>,
    period: Option<&Value>,
    shared: Option<&Value>,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let plan_name = field(plan, "/planInfo/planName")
        .and_then(Value::as_str)
        .or_else(|| field(summary, "/membershipType").and_then(Value::as_str))
        .map(format_membership);
    let mut start = field(period, "/billingCycleStart")
        .and_then(timestamp)
        .or_else(|| field(summary, "/billingCycleStart").and_then(timestamp));
    let mut end = field(period, "/billingCycleEnd")
        .and_then(timestamp)
        .or_else(|| field(summary, "/billingCycleEnd").and_then(timestamp));
    let mut metrics = Vec::new();
    if kind == PlanKind::Unified {
        // usage-summary 在新版可能仍是旧月账期的零值，不能用它填补共享周额度
        // 当前协议的 usagePercent 为 optional，缺失表示未知，只有产品行中的非 optional 值可补零
        let shared_percent = field(shared, "/usagePercent").and_then(number);
        let (percent, products) = if let Some(percent) = shared_percent {
            start = field(shared, "/currentPeriodStartMs").and_then(timestamp);
            end = field(shared, "/currentPeriodEndMs").and_then(timestamp);
            (
                Some(percent),
                field(shared, "/productUsage").and_then(Value::as_array),
            )
        } else {
            start = field(period, "/billingCycleStart").and_then(timestamp);
            end = field(period, "/billingCycleEnd").and_then(timestamp);
            (
                field(period, "/planUsage/totalPercentUsed").and_then(number),
                None,
            )
        };
        let percent = percent.ok_or_else(|| {
            usage_error(
                AccountUsageStatus::SchemaChanged,
                "共享额度暂不可用，请稍后刷新",
            )
        })?;
        metrics.push(percent_metric(
            "cursor.shared",
            "Shared usage",
            percent,
            end.clone(),
        ));
        let bot = products.and_then(|rows| {
            rows.iter().find(|row| {
                row.get("product")
                    .and_then(Value::as_str)
                    .is_some_and(|product| {
                        product
                            .chars()
                            .filter(char::is_ascii_alphanumeric)
                            .collect::<String>()
                            .eq_ignore_ascii_case("GrokBot")
                    })
            })
        });
        // 产品百分比是整个共享池的占用，不能作为另一个额度池参与汇总
        metrics.push(AccountUsageMetric {
            metric_key: "cursor.product.grok_bot".to_string(),
            label: "Grok Bot".to_string(),
            unit: "percent_share".to_string(),
            scope: AccountUsageMetricScope::Account,
            used: bot.and_then(|row| match row.get("usagePercent") {
                Some(value) => number(value),
                // 该字段是非 optional protobuf double，存在产品行但省略数值时表示零
                None => Some(0.0),
            }),
            limit: None,
            remaining: None,
            percentage: None,
            reset_at: None,
        });
    } else if kind == PlanKind::Legacy {
        for (key, label, field_name) in [
            ("cursor.auto", "Cursor Models", "autoPercentUsed"),
            ("cursor.api", "Other Models", "apiPercentUsed"),
        ] {
            let value = field(period, &format!("/planUsage/{field_name}"))
                .or_else(|| field(summary, &format!("/individualUsage/plan/{field_name}")))
                .or_else(|| field(summary, &format!("/teamUsage/plan/{field_name}")));
            if let Some(percent) = value.and_then(number) {
                metrics.push(percent_metric(key, label, percent, end.clone()));
            }
        }
        if metrics.is_empty() {
            return Err(usage_error(
                AccountUsageStatus::SchemaChanged,
                "Cursor 响应缺少额度信息，请稍后刷新",
            ));
        }
    } else {
        return Err(usage_error(
            AccountUsageStatus::SchemaChanged,
            "暂时无法识别 Cursor 套餐",
        ));
    }
    Ok(AccountUsageSnapshot {
        provider_id: "cursor".to_string(),
        account_key: format!("cursor:{}", credential.user_id),
        account_label: credential.email.clone(),
        plan: plan_name,
        status: AccountUsageStatus::Ok,
        source: AccountUsageSource::InternalApi,
        confidence: AccountUsageConfidence::Medium,
        observed_at: now_rfc3339(),
        period_start: start,
        period_end: end.clone(),
        reset_at: end,
        stale: false,
        error: None,
        metrics,
    })
}

/**
 * 从可选 JSON 中读取指定路径，返回借用的字段值
 */
fn field<'a>(value: Option<&'a Value>, path: &str) -> Option<&'a Value> {
    value.and_then(|value| value.pointer(path))
}

/**
 * 构建额度指标，参数为标识、名称、已用比例及重置时间，返回百分比指标
 */
fn percent_metric(
    key: &str,
    label: &str,
    percent: f64,
    reset_at: Option<String>,
) -> AccountUsageMetric {
    AccountUsageMetric {
        metric_key: key.to_string(),
        label: label.to_string(),
        unit: "percent".to_string(),
        scope: AccountUsageMetricScope::Account,
        used: Some(percent),
        limit: Some(100.0),
        remaining: Some((100.0 - percent).max(0.0)),
        percentage: Some(percent),
        reset_at,
    }
}

/**
 * 读取有限非负数字，兼容 protobuf JSON 中的数字字符串
 */
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/**
 * 解析 ISO 日期或秒、毫秒时间戳，统一返回 RFC3339 时间
 */
fn timestamp(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        if let Ok(date) = chrono::DateTime::parse_from_rfc3339(text) {
            return Some(date.to_rfc3339());
        }
    }
    let value = number(value).filter(|value| *value > 0.0)?;
    let millis = if value >= 10_000_000_000.0 {
        value
    } else {
        value * 1000.0
    };
    chrono::DateTime::from_timestamp_millis(millis as i64).map(|date| date.to_rfc3339())
}

/**
 * 将套餐标识转换为简短名称，新旧模式由指标区分而非展示名称
 */
fn format_membership(raw: &str) -> String {
    let raw = raw.strip_prefix("unified_").unwrap_or(raw);
    if raw == "pro_plus" {
        return "Pro+".to_string();
    }
    raw.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| format!("{}{}", first.to_uppercase(), chars.as_str()))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/**
 * 构建固定文案的请求错误，避免泄露响应数据
 */
fn usage_error(code: AccountUsageStatus, message: &str) -> AccountUsageError {
    AccountUsageError::new(code, message)
}

/**
 * 构建可通过重新授权恢复的状态
 */
fn auth_expired(message: &str) -> AccountUsageError {
    usage_error(AccountUsageStatus::AuthRequired, message)
}

impl From<rusqlite::Error> for AccountUsageError {
    /**
     * 将数据库错误转换为不携带 SQL 内容的账号用量错误
     */
    fn from(_: rusqlite::Error) -> Self {
        usage_error(AccountUsageStatus::Error, "无法读取账号用量数据")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * 创建不含真实账号信息的测试凭据
     */
    fn credential() -> CursorCredential {
        CursorCredential {
            access_token: "old".into(),
            user_id: "test-user".into(),
            email: None,
        }
    }

    /**
     * 验证新旧 Ultra 以及未知套餐不混淆，服务端明确标记优先
     */
    #[test]
    fn distinguishes_plan_generations() {
        for (name, expected) in [
            ("ultra", PlanKind::Legacy),
            ("unified_ultra", PlanKind::Unified),
            ("premium", PlanKind::Unified),
            ("pro", PlanKind::Legacy),
            ("future", PlanKind::Unknown),
        ] {
            assert_eq!(
                plan_kind(Some(&json!({"membershipType": name})), None, None),
                expected
            );
        }
        assert_eq!(
            plan_kind(
                Some(&json!({"membershipType":"ultra"})),
                None,
                Some(&json!({"isUnified":true}))
            ),
            PlanKind::Unified
        );
        assert_eq!(
            plan_kind(
                None,
                Some(&json!({"planInfo":{"includedUsagePeriod":2}})),
                None
            ),
            PlanKind::Unified
        );
    }

    /**
     * 共享额度直接采用新接口，Grok Bot 使用产品明细，不重复计算总量
     */
    #[test]
    fn shared_pool_and_bot_share_are_distinct() {
        let shared = json!({"usagePercent":37.5, "currentPeriodStartMs":"1791882381427",
            "currentPeriodEndMs":"1792487181427", "productUsage":[
                {"product":"Cursor","usagePercent":20.0}, {"product":"GrokBot","usagePercent":12.5}]});
        let snapshot = parse_snapshot(
            &credential(),
            PlanKind::Unified,
            None,
            None,
            None,
            Some(&shared),
        )
        .unwrap();
        assert_eq!(snapshot.metrics.len(), 2);
        assert_eq!(snapshot.metrics[0].remaining, Some(62.5));
        assert_eq!(snapshot.metrics[1].used, Some(12.5));
        assert_eq!(snapshot.metrics[1].percentage, None);
        assert!(snapshot.reset_at.is_some());
    }

    /**
     * 明细缺失时保持未知，且不把网页摘要的旧月账期零值当作新版额度
     */
    #[test]
    fn missing_shared_data_is_not_zero() {
        let summary = json!({"individualUsage":{"plan":{"totalPercentUsed":0}}});
        assert!(parse_snapshot(
            &credential(),
            PlanKind::Unified,
            Some(&summary),
            None,
            None,
            None
        )
        .is_err());
        let period = json!({"planUsage":{"totalPercentUsed":0},"billingCycleEnd":"1792487181427"});
        let snapshot = parse_snapshot(
            &credential(),
            PlanKind::Unified,
            Some(&summary),
            None,
            Some(&period),
            None,
        )
        .unwrap();
        assert_eq!(snapshot.metrics[0].percentage, Some(0.0));
        assert_eq!(snapshot.metrics[1].used, None);
    }

    /**
     * optional 总量缺失不补零，显式零保留周期与明细，非 optional 产品零值可以省略
     */
    #[test]
    fn optional_percent_presence_is_preserved() {
        let mut shared = json!({"currentPeriodEndMs":"1792487181427",
            "productUsage":[{"product":"GrokBot"}]});
        assert!(parse_snapshot(
            &credential(),
            PlanKind::Unified,
            None,
            None,
            None,
            Some(&shared)
        )
        .is_err());
        shared["usagePercent"] = json!(0);
        let snapshot = parse_snapshot(
            &credential(),
            PlanKind::Unified,
            None,
            None,
            None,
            Some(&shared),
        )
        .unwrap();
        assert_eq!(snapshot.metrics[0].percentage, Some(0.0));
        assert_eq!(snapshot.metrics[1].used, Some(0.0));
        assert!(snapshot.reset_at.is_some());
        let mut period = json!({"planUsage":{}});
        assert!(parse_snapshot(
            &credential(),
            PlanKind::Legacy,
            None,
            None,
            Some(&period),
            None
        )
        .is_err());
        period["planUsage"] = json!({"autoPercentUsed":0, "apiPercentUsed":0});
        let snapshot = parse_snapshot(
            &credential(),
            PlanKind::Legacy,
            None,
            None,
            Some(&period),
            None,
        )
        .unwrap();
        assert!(snapshot
            .metrics
            .iter()
            .all(|metric| metric.percentage == Some(0.0)));
    }

    /**
     * 套餐元信息全部失败时透传限流和退避，仅正常的未知字段报告协议变化
     */
    #[test]
    fn plan_detection_preserves_transport_errors() {
        let network = Err(usage_error(AccountUsageStatus::Network, "离线"));
        let rate = Err(AccountUsageError {
            code: AccountUsageStatus::RateLimited,
            message: "限流".into(),
            retry_after_secs: Some(60),
        });
        let error = resolve_plan_kind(&network, &rate, &network).unwrap_err();
        assert_eq!(error.code, AccountUsageStatus::RateLimited);
        assert_eq!(error.retry_after_secs, Some(60));
        assert_eq!(
            resolve_plan_kind(&network, &network, &network)
                .unwrap_err()
                .code,
            AccountUsageStatus::Network
        );
        assert_eq!(
            resolve_plan_kind(&Ok(json!({"membershipType":"future"})), &network, &rate)
                .unwrap_err()
                .code,
            AccountUsageStatus::SchemaChanged
        );
        assert_eq!(
            resolve_plan_kind(&Ok(json!({"membershipType":"premium"})), &network, &rate).unwrap(),
            PlanKind::Unified
        );
    }

    /**
     * 缺省时间戳不能变成 1970 年的重置时间，实际时间戳保持可解析
     */
    #[test]
    fn zero_timestamp_is_unknown() {
        assert_eq!(timestamp(&json!(0)), None);
        assert_eq!(timestamp(&json!("0")), None);
        assert!(timestamp(&json!("1792487181427")).is_some());
    }

    /**
     * 旧套餐显示两个独立额度池，不生成平均值或 Grok Bot 指标
     */
    #[test]
    fn legacy_pools_stay_separate() {
        let period = json!({"planUsage":{"autoPercentUsed":20,"apiPercentUsed":80}});
        let snapshot = parse_snapshot(
            &credential(),
            PlanKind::Legacy,
            None,
            None,
            Some(&period),
            None,
        )
        .unwrap();
        assert_eq!(
            snapshot
                .metrics
                .iter()
                .map(|m| m.metric_key.as_str())
                .collect::<Vec<_>>(),
            ["cursor.auto", "cursor.api"]
        );
    }

    /**
     * 只有授权失败允许重读凭据，普通网络错误不能触发授权或保存
     */
    #[test]
    fn network_failure_never_discovers_credentials() {
        let error = refresh_with_recovery(
            &credential(),
            |_| Err(usage_error(AccountUsageStatus::Network, "离线")),
            || panic!("不应读取凭据"),
            |_| panic!("不应保存"),
        )
        .unwrap_err();
        assert_eq!(error.code, AccountUsageStatus::Network);
    }

    /**
     * 静默恢复只接受同一账号的新 token，成功后更新加密副本
     */
    #[test]
    fn recovers_same_account_only() {
        let saved = std::cell::Cell::new(false);
        let result = refresh_with_recovery(
            &credential(),
            |value| {
                if value.access_token == "old" {
                    Err(auth_expired("过期"))
                } else {
                    Ok(vec![])
                }
            },
            || {
                Ok(CursorCredential {
                    access_token: "fresh".into(),
                    ..credential()
                })
            },
            |_| {
                saved.set(true);
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert!(saved.get());
        let result = refresh_with_recovery(
            &credential(),
            |_| Err(auth_expired("过期")),
            || {
                Ok(CursorCredential {
                    user_id: "another-user".into(),
                    ..credential()
                })
            },
            |_| panic!("不得切换账号"),
        );
        assert_eq!(result.unwrap_err().code, AccountUsageStatus::AuthRequired);
    }
}
