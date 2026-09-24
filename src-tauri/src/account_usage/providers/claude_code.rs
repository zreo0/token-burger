use std::path::PathBuf;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::account_usage::{
    now_rfc3339, AccountUsageCapability, AccountUsageConfidence, AccountUsageError,
    AccountUsageMetric, AccountUsageMetricScope, AccountUsageProvider, AccountUsageProviderInfo,
    AccountUsageProviderState, AccountUsageRefreshContext, AccountUsageResult,
    AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus, CredentialRequirement,
};

/**
 * Claude 账号用量接口，与本地 token 日志独立
 */
const OAUTH_BASE: &str = "https://api.anthropic.com/api/oauth";
/**
 * Claude 网页账号接口，用于只有桌面端登录的用户手动提供 sessionKey
 */
const WEB_BASE: &str = "https://claude.ai/api";

/** 委托续期的进程级冷却，同时防止超时后的刷新线程重复启动 CLI */
#[cfg(unix)]
static CLI_REFRESH_ATTEMPT: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/**
 * 读取 CLI 凭据，仅在有效 OAuth 登录明确过期时调用一次续期，手动凭据不经过这里
 */
fn load_cli_credential(
    load: impl FnOnce() -> Result<ClaudeCredential, AccountUsageError>,
    renew: impl FnOnce(&str) -> Result<ClaudeCredential, AccountUsageError>,
) -> Result<ClaudeCredential, AccountUsageError> {
    match load()? {
        ClaudeCredential::ExpiredOAuth(token) => renew(&token),
        credential => Ok(credential),
    }
}

/**
 * 返回不包含凭据或 CLI 输出的续期失败提示
 */
fn cli_refresh_error() -> AccountUsageError {
    usage_error(
        AccountUsageStatus::AuthRequired,
        "Claude 自动续期未完成，请打开 Claude CLI 确认登录后重试",
    )
}

/**
 * 预留五分钟冷却，返回本次是否允许启动 CLI
 */
#[cfg(any(unix, test))]
fn reserve_cli_refresh(last: &mut Option<std::time::Instant>, now: std::time::Instant) -> bool {
    if last.is_some_and(|last| now.duration_since(last) < Duration::from_secs(300)) {
        return false;
    }
    *last = Some(now);
    true
}

/**
 * 在 Unix 终端中委托 CLI 续期，继承当前 profile，不自行轮换 refresh token
 */
#[cfg(unix)]
fn refresh_cli_credential(previous: &str) -> Result<ClaudeCredential, AccountUsageError> {
    {
        let mut last = CLI_REFRESH_ATTEMPT
            .lock()
            .map_err(|_| cli_refresh_error())?;
        if !reserve_cli_refresh(&mut last, std::time::Instant::now()) {
            return Err(usage_error(
                AccountUsageStatus::AuthRequired,
                "Claude 自动续期冷却中，请稍后重试或打开 CLI 确认登录",
            ));
        }
    }

    // Finder 启动的应用通常没有用户安装目录，将常见 CLI 目录补到 PATH 尾部
    let mut paths: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|path| path.is_absolute())
        .collect();
    if let Some(home) = dirs::home_dir() {
        paths.extend([home.join(".local/bin"), home.join(".claude/local")]);
    }
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    let binary = paths
        .iter()
        .map(|path| path.join("claude"))
        .find(|path| {
            use std::os::unix::fs::PermissionsExt;
            path.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
        .ok_or_else(|| {
            usage_error(
                AccountUsageStatus::AuthRequired,
                "未找到 Claude CLI，请安装并登录后重试",
            )
        })?;
    let mut command = std::process::Command::new(binary);
    command.arg("/status").current_dir(std::env::temp_dir());
    command.env(
        "PATH",
        std::env::join_paths(paths).map_err(|_| cli_refresh_error())?,
    );
    // 明确使用磁盘中的登录账号，避免环境变量把 CLI 路由到其他账号或 API key
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
    ] {
        command.env_remove(key);
    }
    run_cli_refresh(
        &mut command,
        previous,
        Duration::from_secs(8),
        discover_credential,
    )
}

/**
 * Windows 暂不创建 ConPTY，保持明确的重新登录提示
 */
#[cfg(not(unix))]
fn refresh_cli_credential(_: &str) -> Result<ClaudeCredential, AccountUsageError> {
    Err(cli_refresh_error())
}

/**
 * 为命令创建独立终端，限时等待凭据变更，并在所有退出路径结束进程组和回收子进程
 */
#[cfg(unix)]
fn run_cli_refresh(
    command: &mut std::process::Command,
    previous: &str,
    timeout: Duration,
    load: impl Fn() -> Result<ClaudeCredential, AccountUsageError>,
) -> Result<ClaudeCredential, AccountUsageError> {
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let (mut master_fd, mut slave_fd) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty 写入两个有效的 fd 指针，成功后立即交给 File 管理生命周期
    if unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    } != 0
    {
        return Err(cli_refresh_error());
    }
    // SAFETY: 两个描述符由本次 openpty 创建，尚未被其他对象拥有
    let (mut master, slave) =
        unsafe { (File::from_raw_fd(master_fd), File::from_raw_fd(slave_fd)) };
    // SAFETY: 防止原始终端描述符泄漏给子进程，Command 会独立设置标准输入输出
    if unsafe { libc::fcntl(master_fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1
        || unsafe { libc::fcntl(slave_fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1
    {
        return Err(cli_refresh_error());
    }
    // SAFETY: master 在函数期间保持有效，非阻塞读取避免 CLI 没有输出时卡住超时检查
    if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == -1 {
        return Err(cli_refresh_error());
    }
    command
        .stdin(Stdio::from(
            slave.try_clone().map_err(|_| cli_refresh_error())?,
        ))
        .stdout(Stdio::from(
            slave.try_clone().map_err(|_| cli_refresh_error())?,
        ))
        .stderr(Stdio::from(slave))
        .env("TERM", "xterm-256color");
    // SAFETY: fork 后仅调用异步信号安全的系统调用；标准输入已由 Command 接到 slave
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| cli_refresh_error())?;
    let started = std::time::Instant::now();
    let result = (|| {
        loop {
            // 丢弃终端输出，既避免缓冲区阻塞，也不把账号信息写入日志
            let mut buffer = [0; 4096];
            for _ in 0..16 {
                if !matches!(master.read(&mut buffer), Ok(count) if count > 0) {
                    break;
                }
            }
            if let Ok(ClaudeCredential::OAuth(token)) = load() {
                if token != previous {
                    return Ok(ClaudeCredential::OAuth(token));
                }
            }
            if started.elapsed() >= timeout {
                return Err(cli_refresh_error());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    // SAFETY: 子进程通过 setsid 成为进程组首领；回收前 PID 不会被重用
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub struct ClaudeCodeUsageProvider;

impl AccountUsageProvider for ClaudeCodeUsageProvider {
    /**
     * 返回与 CLI、桌面 Code 共用的 Provider 标识
     */
    fn id(&self) -> &'static str {
        "claude-code"
    }

    /**
     * 返回账号额度的默认刷新间隔，单位为秒
     */
    fn default_refresh_interval_secs(&self) -> u64 {
        300
    }

    /**
     * 根据保存的状态返回账号额度能力及凭据说明
     */
    fn info(&self, state: &AccountUsageProviderState) -> AccountUsageProviderInfo {
        AccountUsageProviderInfo {
            id: self.id().to_string(),
            display_name: "Claude Code".to_string(),
            enabled: state.enabled,
            show_in_menu_bar: state.show_in_menu_bar,
            available: state.credential_ref.is_some() || self.detect(),
            source: AccountUsageSource::InternalApi,
            confidence: AccountUsageConfidence::Medium,
            capabilities: vec![
                AccountUsageCapability::AccountUsage,
                AccountUsageCapability::AccountQuota,
                AccountUsageCapability::InternalApi,
            ],
            credential_requirements: vec![CredentialRequirement {
                key: "session".to_string(),
                label: "Claude OAuth token / sessionKey".to_string(),
                secret: true,
                required: true,
                description:
                    "可选覆盖 CLI 登录凭据；支持 sk-ant-oat… 或网页 sessionKey，清除后恢复自动读取"
                        .to_string(),
            }],
            experimental: true,
            default_refresh_interval_secs: self.default_refresh_interval_secs(),
            refresh_interval_secs: state.refresh_interval_secs,
        }
    }

    /**
     * 只检查配置文件或安装目录，不在枚举设置时读取钥匙串
     */
    fn detect(&self) -> bool {
        credentials_path().exists() || crate::adapters::claude_code::config_root().exists()
    }

    /**
     * 读取指定凭据并查询同一账号的额度，失败交给 manager 保留过期快照
     */
    fn refresh(&self, context: AccountUsageRefreshContext) -> AccountUsageResult {
        let conn = rusqlite::Connection::open(&context.db_path)
            .map_err(|_| usage_error(AccountUsageStatus::Error, "无法读取账号设置"))?;
        let state = crate::account_usage::store::get_provider_state(&conn, self.id())?;
        let credential = match state.and_then(|state| state.credential_ref) {
            Some(reference) => {
                parse_manual_credential(&context.credentials.load_secret(&reference)?)?
            }
            None => load_cli_credential(discover_credential, refresh_cli_credential)?,
        };
        // 禁止重定向，避免内部接口重定向时把登录凭据带到其他站点
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("TokenBurger Claude account-usage")
            .build()
            .map_err(|_| usage_error(AccountUsageStatus::Network, "无法初始化 Claude 请求"))?;
        let snapshot = match credential {
            ClaudeCredential::OAuth(token) => fetch_oauth_usage(&client, &token, OAUTH_BASE)?,
            ClaudeCredential::Web(session) => fetch_web_usage(&client, &session, WEB_BASE)?,
            ClaudeCredential::ExpiredOAuth(_) => return Err(cli_refresh_error()),
        };
        Ok(vec![snapshot])
    }
}

/**
 * 凭据只在刷新期间保留内存，不写入日志或账号快照
 */
enum ClaudeCredential {
    /**
     * Claude Code 登录 access token
     */
    OAuth(String),
    /** CLI 已过期的凭据，仅供委托续期，不发送额度请求 */
    ExpiredOAuth(String),
    /**
     * Claude 网页 sessionKey
     */
    Web(String),
}

/**
 * 根据安全存储环境覆盖及 CLI 配置返回凭据文件路径
 */
fn credentials_path() -> PathBuf {
    let root = match std::env::var_os("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
        Some(value) if value.is_empty() => dirs::home_dir().unwrap_or_default().join(".claude"),
        Some(value) => PathBuf::from(value),
        None => crate::adapters::claude_code::config_root(),
    };
    root.join(".credentials.json")
}

/**
 * 解析用户明确保存的 OAuth token 或 Cookie，仅提取需要的 sessionKey
 */
fn parse_manual_credential(secret: &str) -> Result<ClaudeCredential, AccountUsageError> {
    let secret = secret.trim();
    if secret.starts_with("sk-ant-oat") && !secret.chars().any(char::is_whitespace) {
        return Ok(ClaudeCredential::OAuth(secret.to_string()));
    }
    let cookie = secret
        .strip_prefix("Cookie:")
        .or_else(|| secret.strip_prefix("cookie:"))
        .unwrap_or(secret)
        .trim();
    let session = if cookie.contains('=') {
        cookie.split(';').find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == "sessionKey").then_some(value.trim())
        })
    } else {
        Some(cookie)
    };
    match session
        .filter(|value| value.starts_with("sk-ant-sid") && !value.chars().any(char::is_whitespace))
    {
        Some(session) => Ok(ClaudeCredential::Web(session.to_string())),
        None => Err(usage_error(
            AccountUsageStatus::AuthRequired,
            "请提供 Claude 登录 OAuth token 或 sessionKey；API key 不支持订阅额度查询",
        )),
    }
}

/**
 * 每次刷新重新读取 CLI 凭据，CLI 自身轮换 token 后无需重新配置
 */
fn discover_credential() -> Result<ClaudeCredential, AccountUsageError> {
    let mut expired = None;
    let mut last_error = usage_error(
        AccountUsageStatus::AuthRequired,
        "未找到 Claude 登录凭据，请在 CLI 登录或填写 OAuth token / sessionKey",
    );
    if let Ok(contents) = std::fs::read_to_string(credentials_path()) {
        match parse_oauth_credentials(&contents, chrono::Utc::now().timestamp_millis()) {
            // 文件可能落后于钥匙串，优先寻找已经由 CLI 更新的有效凭据
            Ok(credential @ ClaudeCredential::ExpiredOAuth(_)) => expired = Some(credential),
            Ok(credential) => return Ok(credential),
            Err(error) => last_error = error,
        }
    }
    // 自定义 profile 不能回退到全局钥匙串，避免查询另一个账号
    let custom_profile = ["CLAUDE_CONFIG_DIR", "CLAUDE_SECURESTORAGE_CONFIG_DIR"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()));
    if !custom_profile {
        match read_cli_keychain() {
            Ok(Some(contents)) => {
                match parse_oauth_credentials(&contents, chrono::Utc::now().timestamp_millis()) {
                    Ok(credential) => return Ok(credential),
                    Err(error) => last_error = error,
                }
            }
            Err(error) => last_error = error,
            Ok(None) => {}
        }
    }
    expired.ok_or(last_error)
}

/**
 * 校验 CLI OAuth JSON 的 scope 和过期时间，返回可用于额度接口的 token
 */
fn parse_oauth_credentials(
    contents: &str,
    now_ms: i64,
) -> Result<ClaudeCredential, AccountUsageError> {
    let value: Value = serde_json::from_str(contents).map_err(|_| {
        usage_error(
            AccountUsageStatus::AuthRequired,
            "Claude 凭据格式无效，请重新登录",
        )
    })?;
    let oauth = value.get("claudeAiOauth").ok_or_else(|| {
        usage_error(
            AccountUsageStatus::AuthRequired,
            "Claude 凭据缺少账号 OAuth 登录信息",
        )
    })?;
    if let Some(scopes) = oauth.get("scopes").and_then(Value::as_array) {
        if !scopes
            .iter()
            .any(|scope| scope.as_str() == Some("user:profile"))
        {
            return Err(usage_error(
                AccountUsageStatus::Forbidden,
                "Claude token 缺少 user:profile 权限，请使用 CLI 登录凭据",
            ));
        }
    }
    let token = text_at(oauth, "accessToken").ok_or_else(|| {
        usage_error(
            AccountUsageStatus::AuthRequired,
            "Claude 登录缺少 access token",
        )
    })?;
    if oauth
        .get("expiresAt")
        .and_then(Value::as_i64)
        .is_some_and(|expires| expires <= now_ms)
    {
        return Ok(ClaudeCredential::ExpiredOAuth(token));
    }
    Ok(ClaudeCredential::OAuth(token))
}

/**
 * 无交互读取 macOS CLI 钥匙串，受保护条目返回错误而不弹出后台授权框
 */
#[cfg(target_os = "macos")]
fn read_cli_keychain() -> Result<Option<String>, AccountUsageError> {
    use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
    let result = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service("Claude Code-credentials")
        .load_data(true)
        .skip_authenticated_items(true)
        .search();
    match result {
        Ok(items) => Ok(items.into_iter().find_map(|item| match item {
            SearchResult::Data(bytes) => String::from_utf8(bytes).ok(),
            _ => None,
        })),
        Err(_) => Err(usage_error(
            AccountUsageStatus::CredentialUnavailable,
            "无法无交互读取 Claude 钥匙串，请手动填写 OAuth token 或 sessionKey",
        )),
    }
}

/**
 * 非 macOS 平台由 CLI 凭据文件提供登录信息
 */
#[cfg(not(target_os = "macos"))]
fn read_cli_keychain() -> Result<Option<String>, AccountUsageError> {
    Ok(None)
}

/**
 * 请求并验证 JSON，错误信息不包含响应正文或凭据
 */
fn request_json(request: RequestBuilder) -> Result<Value, AccountUsageError> {
    let response = request
        .send()
        .map_err(|_| usage_error(AccountUsageStatus::Network, "Claude 额度网络请求失败"))?;
    if let Some(error) = http_error(response.status().as_u16(), response.headers()) {
        return Err(error);
    }
    response.json().map_err(|_| {
        usage_error(
            AccountUsageStatus::SchemaChanged,
            "Claude 额度响应不是有效 JSON",
        )
    })
}

/**
 * 将 HTTP 状态和 Retry-After 转为稳定的账号错误，不回显服务端内容
 */
fn http_error(status: u16, headers: &HeaderMap) -> Option<AccountUsageError> {
    let (code, message) = match status {
        200..=299 => return None,
        401 => (
            AccountUsageStatus::AuthRequired,
            "Claude 登录已失效，请更新凭据",
        ),
        403 => (
            AccountUsageStatus::Forbidden,
            "Claude 额度访问被拒绝，请检查账号权限或网络限制",
        ),
        429 => (
            AccountUsageStatus::RateLimited,
            "Claude 额度接口被限流，稍后重试",
        ),
        500..=599 => (AccountUsageStatus::Network, "Claude 额度服务暂时不可用"),
        _ => (AccountUsageStatus::Error, "Claude 额度请求失败"),
    };
    let mut error = usage_error(code, message);
    if status == 429 {
        let retry_after = headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok());
        error.retry_after_secs = Some(
            retry_after
                .and_then(|value| value.parse().ok())
                .or_else(|| {
                    let date = chrono::DateTime::parse_from_rfc2822(retry_after?).ok()?;
                    Some((date.timestamp() - chrono::Utc::now().timestamp()).max(1) as u64)
                })
                .unwrap_or(300)
                .max(1),
        );
    }
    Some(error)
}

/**
 * 用 OAuth 查询 profile 和额度，身份只取同一登录 token 的服务端响应
 */
fn fetch_oauth_usage(
    client: &Client,
    token: &str,
    base: &str,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let request = |path: &str| {
        client
            .get(format!("{base}/{path}"))
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("Accept", "application/json")
            .header("User-Agent", "claude-code/2.1.0")
    };
    let profile = request_json(request("profile"))?;
    let account = profile.get("account").unwrap_or(&profile);
    let organization = profile.get("organization").unwrap_or(&Value::Null);
    let account_id = text_at(account, "uuid")
        .or_else(|| text_at(&profile, "account_uuid"))
        .or_else(|| text_at(&profile, "accountUuid"));
    let org_id = text_at(organization, "uuid")
        .or_else(|| text_at(&profile, "organization_uuid"))
        .or_else(|| text_at(&profile, "organizationUuid"));
    let key = account_identity(account_id.as_deref(), org_id.as_deref())?;
    let usage = request_json(request("usage"))?;
    let label = text_at(account, "email_address").or_else(|| text_at(account, "email"));
    let plan = text_at(organization, "organization_type")
        .or_else(|| text_at(organization, "rate_limit_tier"));
    build_snapshot(key, label, plan, &usage)
}

/**
 * 用网页 sessionKey 查询明确的 Claude 组织，多个候选时拒绝猜测
 */
fn fetch_web_usage(
    client: &Client,
    session: &str,
    base: &str,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let request = |path: &str| {
        client
            .get(format!("{base}/{path}"))
            .header("Cookie", format!("sessionKey={session}"))
            .header("Accept", "application/json")
    };
    let organizations = request_json(request("organizations"))?;
    let org = select_web_organization(&organizations)?;
    let org_id = text_at(org, "uuid")
        .ok_or_else(|| usage_error(AccountUsageStatus::SchemaChanged, "Claude 组织缺少标识"))?;
    // URL 段仅允许 UUID 字符，避免异常服务端字段改变查询目标
    if !org_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err(usage_error(
            AccountUsageStatus::SchemaChanged,
            "Claude 组织标识无效",
        ));
    }
    let account = request_json(request("account"))?;
    let account_id = text_at(&account, "uuid");
    let key = account_identity(account_id.as_deref(), Some(&org_id))?;
    let usage = request_json(request(&format!("organizations/{org_id}/usage")))?;
    build_snapshot(
        key,
        text_at(&account, "email_address").or_else(|| text_at(&account, "email")),
        text_at(org, "organization_type"),
        &usage,
    )
}

/**
 * 从组织列表选择唯一的 Claude 聊天组织，API Console 组织不参与订阅额度
 */
fn select_web_organization(value: &Value) -> Result<&Value, AccountUsageError> {
    let organizations = value
        .as_array()
        .ok_or_else(|| usage_error(AccountUsageStatus::SchemaChanged, "Claude 组织列表格式变化"))?;
    let candidates: Vec<_> = organizations
        .iter()
        .filter(|org| {
            org.get("capabilities")
                .and_then(Value::as_array)
                .is_some_and(|caps| caps.iter().any(|cap| cap.as_str() == Some("chat")))
        })
        .collect();
    match candidates.as_slice() {
        [org] => Ok(org),
        [] if organizations.len() == 1 && organizations[0].get("capabilities").is_none() => {
            Ok(&organizations[0])
        }
        _ => Err(usage_error(
            AccountUsageStatus::AuthRequired,
            "未找到唯一的 Claude 订阅组织，请使用目标组织的 CLI OAuth 登录凭据",
        )),
    }
}

/**
 * 根据服务端账号和组织构造稳定标识，避免把不同组织的额度混在一起
 */
fn account_identity(
    account: Option<&str>,
    organization: Option<&str>,
) -> Result<String, AccountUsageError> {
    match (account, organization) {
        (Some(account), Some(org)) => Ok(format!("claude:{account}:{org}")),
        _ => Err(usage_error(
            AccountUsageStatus::SchemaChanged,
            "Claude 响应缺少账号或组织标识",
        )),
    }
}

/**
 * 将服务端已用百分比和重置时间映射为账号指标，缺失字段不视为零
 */
fn build_snapshot(
    key: String,
    label: Option<String>,
    plan: Option<String>,
    usage: &Value,
) -> Result<AccountUsageSnapshot, AccountUsageError> {
    let mut metrics = Vec::new();
    for (field, label) in [
        ("five_hour", "5h window"),
        ("seven_day", "7d window"),
        ("seven_day_sonnet", "Sonnet 7d window"),
        ("seven_day_opus", "Opus 7d window"),
        ("seven_day_routines", "Routines 7d window"),
        ("seven_day_cowork", "Cowork 7d window"),
    ] {
        if let Some(window) = usage.get(field) {
            if let Some(percent) = number_at(window, "utilization") {
                metrics.push(percent_metric(
                    field,
                    label,
                    percent,
                    text_at(window, "resets_at"),
                ));
            }
        }
    }
    if let Some(limits) = usage.get("limits").and_then(Value::as_array) {
        for limit in limits {
            if limit.get("kind").and_then(Value::as_str) != Some("weekly_scoped")
                || limit.get("is_active").and_then(Value::as_bool) == Some(false)
            {
                continue;
            }
            let model = limit.pointer("/scope/model").unwrap_or(&Value::Null);
            if let (Some(id), Some(percent)) = (text_at(model, "id"), number_at(limit, "percent")) {
                let label = text_at(model, "display_name").unwrap_or_else(|| id.clone());
                let all_models = label.eq_ignore_ascii_case("all models")
                    || id == "all"
                    || id.replace('_', "-").ends_with("all-models");
                let field = if all_models {
                    "seven_day".to_string()
                } else {
                    format!("seven_day_{id}")
                };
                let label = if all_models {
                    "7d window".to_string()
                } else {
                    format!("{label} 7d window")
                };
                let metric = percent_metric(&field, &label, percent, text_at(limit, "resets_at"));
                if let Some(existing) = metrics
                    .iter_mut()
                    .find(|existing| existing.metric_key == metric.metric_key)
                {
                    *existing = metric;
                } else {
                    metrics.push(metric);
                }
            }
        }
    }
    // 额外消费以美分返回，不能把它当成剩余 token 或预付余额
    if let Some(extra) = usage
        .get("extra_usage")
        .filter(|extra| extra.get("is_enabled").and_then(Value::as_bool) == Some(true))
    {
        let used = number_at(extra, "used_credits").map(|value| value / 100.0);
        let limit = number_at(extra, "monthly_limit").map(|value| value / 100.0);
        if used.is_some() || limit.is_some() {
            metrics.push(AccountUsageMetric {
                metric_key: "claude.extra_usage".into(),
                label: "额外消费（月）".into(),
                unit: "usd".into(),
                scope: AccountUsageMetricScope::Account,
                used,
                limit,
                remaining: used.zip(limit).map(|(used, limit)| (limit - used).max(0.0)),
                percentage: number_at(extra, "utilization"),
                reset_at: None,
            });
        }
    }
    if !metrics.iter().any(|metric| metric.unit == "percent") {
        return Err(usage_error(
            AccountUsageStatus::SchemaChanged,
            "Claude 响应缺少有效额度窗口",
        ));
    }
    Ok(AccountUsageSnapshot {
        provider_id: "claude-code".into(),
        account_key: key,
        account_label: label,
        plan,
        status: AccountUsageStatus::Ok,
        source: AccountUsageSource::InternalApi,
        confidence: AccountUsageConfidence::Medium,
        observed_at: now_rfc3339(),
        period_start: None,
        period_end: None,
        reset_at: None,
        stale: false,
        error: None,
        metrics,
    })
}

/**
 * 将单个窗口映射为百分比指标，remaining 只表示百分比而非 token 数
 */
fn percent_metric(
    key: &str,
    label: &str,
    used: f64,
    reset_at: Option<String>,
) -> AccountUsageMetric {
    AccountUsageMetric {
        metric_key: format!("claude.{key}"),
        label: label.into(),
        unit: "percent".into(),
        scope: AccountUsageMetricScope::Account,
        used: Some(used),
        limit: Some(100.0),
        remaining: Some((100.0 - used).clamp(0.0, 100.0)),
        percentage: Some(used),
        reset_at,
    }
}

/**
 * 读取非空字符串字段，缺失或类型变化时返回 None
 */
fn text_at(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/**
 * 读取非负有限数值字段，缺失或无效时不虚构零值
 */
fn number_at(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)?
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/**
 * 构造不含凭据及服务端正文的稳定错误
 */
fn usage_error(code: AccountUsageStatus, message: &str) -> AccountUsageError {
    AccountUsageError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    /**
     * 验证只有明确过期的 CLI 凭据会触发续期，失败直接返回且不循环重试
     */
    #[test]
    fn cli_refresh_only_runs_for_expired_credentials() {
        let fresh = load_cli_credential(
            || Ok(ClaudeCredential::OAuth("valid".into())),
            |_| panic!("有效凭据不应启动 CLI"),
        )
        .unwrap();
        assert!(matches!(fresh, ClaudeCredential::OAuth(token) if token == "valid"));
        assert!(load_cli_credential(
            || Err(cli_refresh_error()),
            |_| panic!("未登录不应启动 CLI"),
        )
        .is_err());
        let refreshed = load_cli_credential(
            || Ok(ClaudeCredential::ExpiredOAuth("old".into())),
            |old| {
                assert_eq!(old, "old");
                Ok(ClaudeCredential::OAuth("new".into()))
            },
        )
        .unwrap();
        assert!(matches!(refreshed, ClaudeCredential::OAuth(token) if token == "new"));
        assert!(load_cli_credential(
            || Ok(ClaudeCredential::ExpiredOAuth("old".into())),
            |_| Err(cli_refresh_error()),
        )
        .is_err());
    }

    /**
     * 验证失败或并发请求也受五分钟冷却限制
     */
    #[test]
    fn cli_refresh_reservation_has_bounded_cooldown() {
        let mut last = None;
        let now = std::time::Instant::now();
        assert!(reserve_cli_refresh(&mut last, now));
        assert!(!reserve_cli_refresh(&mut last, now));
        assert!(!reserve_cli_refresh(
            &mut last,
            now + Duration::from_secs(299)
        ));
        assert!(reserve_cli_refresh(
            &mut last,
            now + Duration::from_secs(300)
        ));
    }

    /**
     * 用模拟 CLI 验证终端环境、磁盘凭据更新、超时及进程清理，不运行真实 Claude
     */
    #[cfg(unix)]
    #[test]
    fn cli_refresh_observes_new_credentials_and_reaps_process() {
        for (token, expires, succeeds) in [
            ("new", 4102444800000_i64, true),
            ("old", 4102444800000, false),
            ("new", 1, false),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let credentials = temp.path().join("credentials.json");
            let pid_file = temp.path().join("pid");
            let payload =
                serde_json::json!({"claudeAiOauth": {"accessToken": token, "expiresAt": expires}})
                    .to_string();
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-c", "test -t 0 && test -t 1 && test -t 2 || exit 1; printf '%s' \"$$\" > \"$1\"; printf '%s' \"$3\" > \"$2\"; exec sleep 20", "fixture"])
                .arg(&pid_file).arg(&credentials).arg(payload);
            let started = std::time::Instant::now();
            let result = run_cli_refresh(&mut command, "old", Duration::from_millis(500), || {
                let content =
                    std::fs::read_to_string(&credentials).map_err(|_| cli_refresh_error())?;
                parse_oauth_credentials(&content, chrono::Utc::now().timestamp_millis())
            });
            assert_eq!(result.is_ok(), succeeds);
            assert!(started.elapsed() < Duration::from_secs(3));
            let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
            // SAFETY: 信号 0 仅检查进程是否存在，不向进程发送信号
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }

    /**
     * 模拟不可启动的 CLI，验证错误不包含可执行文件路径或终端输出
     */
    #[cfg(unix)]
    #[test]
    fn cli_refresh_spawn_failure_is_redacted() {
        let mut command = std::process::Command::new("/nonexistent/claude-fixture");
        let result = run_cli_refresh(&mut command, "old", Duration::from_millis(100), || {
            panic!("启动失败不应进入凭据轮询")
        });
        assert!(
            matches!(result, Err(error) if error.code == AccountUsageStatus::AuthRequired && !error.message.contains("fixture"))
        );
    }
    use serde_json::json;

    /**
     * 生成具有有效身份的测试快照，避免读取真实账号或钥匙串
     */
    fn snapshot(usage: Value) -> Result<AccountUsageSnapshot, AccountUsageError> {
        build_snapshot("claude:account:org".into(), None, None, &usage)
    }

    /**
     * 验证零用量、缺失窗口和额外消费金额的区别
     */
    #[test]
    fn maps_windows_without_turning_null_into_zero() {
        let result = snapshot(json!({
            "five_hour": {"utilization": 0, "resets_at": "2026-09-24T12:00:00Z"},
            "seven_day": {"utilization": 64.5}, "seven_day_opus": null,
            "seven_day_sonnet": {"utilization": null},
            "extra_usage": {"is_enabled": true, "used_credits": 1250, "monthly_limit": 5000}
        }))
        .unwrap();
        assert_eq!(result.metrics.len(), 3);
        assert_eq!(result.metrics[0].remaining, Some(100.0));
        assert_eq!(
            result.metrics[0].reset_at.as_deref(),
            Some("2026-09-24T12:00:00Z")
        );
        assert_eq!(result.metrics[1].remaining, Some(35.5));
        assert_eq!(result.metrics[2].used, Some(12.5));
        assert_eq!(result.metrics[2].remaining, Some(37.5));
        assert_eq!(
            snapshot(json!({"five_hour": null})).unwrap_err().code,
            AccountUsageStatus::SchemaChanged
        );
    }

    /**
     * 验证模型窗口合并及全模型周窗口去重
     */
    #[test]
    fn scoped_windows_replace_legacy_model_windows() {
        let result = snapshot(json!({
            "seven_day": {"utilization": 15}, "seven_day_sonnet": {"utilization": 5},
            "limits": [
                {"kind":"weekly_scoped", "percent":20,"scope":{"model":{"id":"sonnet","display_name":"Sonnet"}}},
                {"kind":"weekly_scoped", "percent":15,"scope":{"model":{"id":"all","display_name":"All models"}}},
                {"kind":"weekly_scoped", "percent":80,"is_active":false,"scope":{"model":{"id":"opus"}}}
            ]
        })).unwrap();
        assert_eq!(result.metrics.len(), 2);
        assert_eq!(result.metrics[1].used, Some(20.0));
    }

    /**
     * 验证登录凭据、权限、过期时间和手动凭据类型
     */
    #[test]
    fn credentials_require_login_scope_and_distinguish_expiry() {
        let valid = json!({"claudeAiOauth":{"accessToken":"test-access", "expiresAt":2000,"scopes":["user:profile"]}});
        assert!(parse_oauth_credentials(&valid.to_string(), 1000).is_ok());
        assert!(matches!(
            parse_oauth_credentials(&valid.to_string(), 2000),
            Ok(ClaudeCredential::ExpiredOAuth(_))
        ));
        let missing_scope =
            json!({"claudeAiOauth":{"accessToken":"test-access","scopes":["user:inference"]}});
        assert!(matches!(
            parse_oauth_credentials(&missing_scope.to_string(), 0),
            Err(AccountUsageError {
                code: AccountUsageStatus::Forbidden,
                ..
            })
        ));
        assert!(parse_oauth_credentials(r#"{"mcpOAuth":{}}"#, 0).is_err());
        assert!(
            matches!(parse_manual_credential("Cookie: x=ignored; sessionKey=sk-ant-sid-test; y=ignored"), Ok(ClaudeCredential::Web(value)) if value == "sk-ant-sid-test")
        );
        assert!(matches!(
            parse_manual_credential("sk-ant-oat-test"),
            Ok(ClaudeCredential::OAuth(_))
        ));
        assert!(parse_manual_credential("sk-ant-api-test").is_err());
    }

    /**
     * 验证同账号不同组织隔离及网页组织歧义处理
     */
    #[test]
    fn quota_identity_keeps_organizations_separate() {
        assert_ne!(
            account_identity(Some("user"), Some("a")).unwrap(),
            account_identity(Some("user"), Some("b")).unwrap()
        );
        assert!(account_identity(None, Some("a")).is_err());
        assert!(select_web_organization(&json!([
            {"uuid":"a","capabilities":["chat"]},{"uuid":"b","capabilities":["chat"]}
        ]))
        .is_err());
        let orgs =
            json!([{"uuid":"api","capabilities":["api"]},{"uuid":"chat","capabilities":["chat"]}]);
        assert_eq!(select_web_organization(&orgs).unwrap()["uuid"], "chat");
    }

    /**
     * 验证额度错误状态及限流退避时间
     */
    #[test]
    fn quota_errors_preserve_retry_after_and_auth_status() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "120".parse().unwrap());
        assert_eq!(
            http_error(429, &headers).unwrap().retry_after_secs,
            Some(120)
        );
        assert_eq!(
            http_error(429, &HeaderMap::new()).unwrap().retry_after_secs,
            Some(300)
        );
        assert_eq!(
            http_error(401, &headers).unwrap().code,
            AccountUsageStatus::AuthRequired
        );
        assert_eq!(
            http_error(403, &headers).unwrap().code,
            AccountUsageStatus::Forbidden
        );
    }

    /**
     * 验证切换账号后不残留旧账号的额度指标
     */
    #[test]
    fn successful_account_switch_removes_previous_quota() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::account_usage::store::init_schema(&conn).unwrap();
        let mut value = snapshot(json!({"five_hour":{"utilization":10}})).unwrap();
        crate::account_usage::store::upsert_snapshot(&conn, &value).unwrap();
        value.account_key = "claude:other:org".into();
        crate::account_usage::store::upsert_snapshot(&conn, &value).unwrap();
        let rows = crate::account_usage::store::latest_snapshots_by_provider(&conn, "claude-code")
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].account_key, value.account_key);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_usage_metrics", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    /**
     * 通过本机 HTTP 服务验证两条查询路径的凭据和账号身份
     */
    #[test]
    fn quota_transports_use_verified_identity_and_credentials() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, body) in [
                (
                    "/profile",
                    json!({"account":{"uuid":"user","email_address":"test@example.com"},"organization":{"uuid":"org"}}),
                ),
                (
                    "/usage",
                    json!({"five_hour":{"utilization":25},"seven_day":{"utilization":50}}),
                ),
                (
                    "/organizations",
                    json!([{"uuid":"org","capabilities":["chat"]}]),
                ),
                (
                    "/account",
                    json!({"uuid":"user","email_address":"test@example.com"}),
                ),
                (
                    "/organizations/org/usage",
                    json!({"five_hour":{"utilization":25},"seven_day":{"utilization":50}}),
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                    assert!(bytes.len() < 8192);
                }
                let request = String::from_utf8_lossy(&bytes).to_lowercase();
                assert!(request.starts_with(&format!("get {path} ")));
                if path == "/profile" || path == "/usage" {
                    assert!(request.contains("authorization: bearer fixture-token"));
                    assert!(request.contains("anthropic-beta: oauth-2025-04-20"));
                    assert!(!request.contains("cookie:"));
                } else {
                    assert!(request.contains("cookie: sessionkey=fixture-session"));
                    assert!(!request.contains("authorization:"));
                }
                let body = body.to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let result = fetch_oauth_usage(&client, "fixture-token", &base).unwrap();
        let web = fetch_web_usage(&client, "fixture-session", &base).unwrap();
        server.join().unwrap();
        assert_eq!(web.account_key, result.account_key);
        assert_eq!(web.metrics[0].remaining, result.metrics[0].remaining);
        assert_eq!(result.account_key, "claude:user:org");
        assert_eq!(result.metrics[0].remaining, Some(75.0));
    }
}
