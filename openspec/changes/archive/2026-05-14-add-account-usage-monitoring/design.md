## Context

Token Burger 已经具备稳定的本地 token 采集链路：`AgentAdapter` 负责解析本地 JSONL/JSON/SQLite 数据源，`WatcherEngine` 负责监听变化，SQLite 中的 `token_logs` 负责保存 token 明细并支持成本汇总。账号级用量监控属于另一类数据域：它需要凭据、远程请求、订阅周期、额度重置、缓存 stale 语义和 Provider 专属错误处理。

第一阶段需要支持 Codex、Claude Code、Cursor、GitHub Copilot，同时保持架构可扩展，以便后续快速接入 OAuth 文件、本地应用缓存、组织级指标等不同来源的 Provider。

## Goals / Non-Goals

**Goals:**

- 新增独立的账号用量监控基础架构，不污染 `AgentAdapter`、`TokenLog` 或 `token_logs`。
- 使用“账号快照 + 多指标”的模型表示 Provider 账号数据，使多窗口额度、本地用量、组织指标、无限套餐和 unsupported 状态都能被统一表达。
- 从第一版开始建立安全凭据存储和脱敏规则。
- 完成第一阶段 Provider 行为：
  - Codex：高可信度账号额度窗口、套餐、credits 和重置时间。
  - Claude Code：高可信度本地 token/cost 用量，账号剩余额度明确标记不可用。
  - Cursor：基于显式 cookie/session 的实验性用量/额度监控。
  - GitHub Copilot：基于 token 的用量/额度监控，并明确指标作用域。
- 启动时优先读取缓存快照，随后异步刷新；刷新具备 timeout、单飞、退避和 stale 兜底。
- 前端账号用量状态与现有 token summary 状态分离。

**Non-Goals:**

- 不替换现有 token 日志 watcher 和 pricing 链路。
- 当 Provider 没有可靠账号额度来源时，不从本地 token 日志推导真实订阅剩余额度。
- 不将 token、cookie、refresh token、API key 存入 SQLite 或前端本地状态。
- 不在默认情况下静默解密浏览器 cookie 或提取用户密钥。
- 第一阶段不构建动态插件系统。
- 对只提供组织级指标或不稳定数据的 Provider，不承诺稳定个人剩余额度。

## Decisions

### Decision 1: 新增 `account_usage`，不扩展 `AgentAdapter`

新增 Rust 后端模块：

```text
src-tauri/src/account_usage/
    mod.rs
    manager.rs
    store.rs
    credentials.rs
    providers/
        codex.rs
        claude_code.rs
        cursor.rs
        github_copilot.rs
```

现有 `AgentAdapter` 继续只负责本地 token 日志采集。账号用量 Provider 使用新的抽象，概念如下：

```text
AccountUsageProvider
    id()
    display_name()
    capabilities()
    detect()
    refresh(context) -> AccountUsageSnapshot
```

理由：token 日志是本地追加型事实；账号用量是周期性刷新的账号快照。两者混用会让存储、刷新、安全和 UI 语义变得模糊。

备选方案：

- 扩展 `AgentAdapter` 增加额度方法：拒绝，因为会把文件监听与带凭据的远程请求耦合在一起。
- 为每类 Provider 建多个 trait：第一阶段拒绝，因为 capability metadata 足以表达数据源差异，复杂度更低。

### Decision 2: 使用新的快照表和指标表

新增独立表：

```text
account_usage_snapshots
account_usage_metrics
account_usage_provider_states
```

快照保存账号/Provider 身份、状态、来源、可信度、观测时间、周期/重置字段、stale 状态和结构化错误。指标保存灵活的命名指标，例如 `codex.primary`、`codex.secondary`、`copilot.premium_interactions`、`cursor.plan`、`claude.local_tokens`、`cost.usd`。

理由：固定字段如 `remaining_percent` 无法覆盖多窗口、本地-only 用量、组织级指标、无限额度和 unsupported quota。指标列表可以让 UI 统一渲染，同时保留 Provider 差异。

备选方案：

- 只保存 Provider 原始 JSON：拒绝，因为会把解析责任推给 UI，并增加敏感响应落盘风险。
- 在 `token_logs` 中增加字段：拒绝，因为额度快照不是 token 事件。

### Decision 3: 用 capability 与 confidence 表达数据可靠性

每个 Provider 和快照都声明能力和可信度：

```text
capabilities: local_tokens, account_usage, account_quota, cost_estimate, multi_account, official_api, internal_api, cookie_required, auth_file_required
confidence: high | medium | low
status: ok | stale | auth_required | unsupported | rate_limited | error
```

理由：Codex 账号额度、Claude 本地日志、Cursor cookie 用量、Copilot token 额度的可靠性和安全属性不同。UI 必须把这些差异展示清楚，避免用户误解。

### Decision 4: 密钥只存在于 OS 凭据存储或外部 auth 文件

SQLite 只保存快照、Provider 状态、凭据引用、hash 和脱敏标签。用户管理的密钥存入 OS 凭据存储；Provider 自有的 auth 文件只保存路径引用，不复制 token 值。

规则：

- 不在 SQLite 中保存 cookie、access token、refresh token 或 API key。
- 不在 Tauri 事件中发送密钥。
- 不记录请求 header、cookie、token 或完整响应 body。
- 错误返回前必须脱敏。
- auth 文件路径只有在账号身份或恢复逻辑需要时才作为 source metadata 保存；其中的 token 值不得复制进 SQLite。

理由：账号用量监控引入敏感凭据，必须在实现 Provider 前明确安全边界。

### Decision 5: 缓存优先的刷新 manager 与 Provider 策略

启动流程：

1. 从 SQLite 读取最近一次成功快照并立即展示。
2. 后台异步刷新已启用 Provider。
3. Provider 刷新成功或进入 stale 状态后发送 `account-usage-updated`。

刷新 manager 行为：

- 每个 provider/account key 单飞刷新。
- API Provider 默认 5 分钟刷新间隔。
- Cursor cookie/session 来源默认 10-15 分钟刷新间隔，手动刷新不受此限制。
- Claude Code 本地 summary 可以复用现有 token summary，或在不发起远程请求的前提下使用更短间隔。
- 每次 Provider 刷新必须有 timeout。
- 429 响应优先遵守 `Retry-After`。
- 网络或 schema 错误保留上一次成功快照并标记 stale。
- 401/403 标记 `auth_required` 或 `forbidden`，避免高频重试。

理由：Provider API 可能限流或变更。缓存优先可以保证菜单栏和页面在刷新失败时仍可用，同时避免过度轮询。

### Decision 6: 第一阶段 Provider 行为

#### Codex

使用本地 Codex auth 文件作为凭据来源。解析默认和配置的 auth 文件候选，必要时刷新 OAuth token，并请求对应账号用量接口。解析额度窗口、重置时间、套餐、credits、账号/工作区身份。多工作区身份不能只依赖 email 或原始 account id；必须在可用时结合 source file path 与 workspace/account metadata，避免不同工作区被合并。

#### Claude Code

使用已有本地 token 日志采集结果作为用量与成本来源。账号用量快照从数据库 summary 派生 daily、weekly、monthly、overall 等本地用量指标。除非后续加入可靠账号额度来源，否则 quota metric 必须标记为 `unsupported` 或不返回。

#### Cursor

支持用户显式提供 cookie/session credential，并可在用户明确启用后做浏览器会话发现。请求 usage summary 和 account identity 接口，解析 billing cycle、plan usage、auto/API usage percent、reset time 和账号身份。source 标记为 `experimental` / `internal_api`，解析或认证失败时保留旧快照并标记 stale。

#### GitHub Copilot

优先使用用户显式保存的 GitHub token；可选地从开发者环境来源解析 token，例如 `gh auth token`、`GITHUB_TOKEN`、`GH_TOKEN` 或 host config。获取可用的 Copilot usage/quota 与账号身份。必须区分个人、组织、团队、企业指标；组织级数据不得展示为个人剩余额度。

### Decision 7: 前端状态分离

新增账号用量状态：

```text
AccountUsageContext
useAccountUsage
AccountUsage 页面/卡片
```

现有 token summary 卡片保持不变。账号用量 UI 展示：

- Provider 图标和名称
- 账号 label/email
- 套餐
- 状态与数据来源标签
- 指标行/额度窗口
- 重置时间或倒计时
- stale/error/auth-required 状态
- 手动刷新操作

理由：账号用量和 token summary 相关但语义不同。状态分离可以避免耦合，同时保留未来汇总展示的空间。

## Risks / Trade-offs

- Provider 内部接口可能变更 → 标记 source type 和 confidence，Provider parser 隔离，schema 错误时保留 stale 快照。
- Cookie/session 凭据敏感 → 需要显式 opt-in，只存 OS 凭据存储，日志和事件全部脱敏。
- 用户可能把本地 usage 误解为剩余额度 → UI 和数据模型必须区分 `local_tokens` 与 `account_quota`，并清晰展示 unsupported quota。
- 多账号身份可能碰撞 → 使用 Provider 专属身份策略，工作区型账号优先结合 source auth file path。
- 网络刷新可能拖慢启动 → 启动时只读缓存，刷新放后台。
- 高频轮询可能触发限流 → 使用 Provider 间隔、退避、`Retry-After` 和手动刷新。
- OS 凭据存储在部分平台可能不可用 → 返回结构化 `credential_unavailable` 错误；对于外部 auth 文件 Provider，尽量保持安全可用。

## Migration Plan

1. 使用 `CREATE TABLE IF NOT EXISTS` 新增 SQLite 表，不修改现有 token 表。
2. 应用启动时注册 account usage manager 为 Tauri managed state。
3. 前端初始化时加载缓存快照。
4. 使用新的 command/event 名称，不影响现有 token commands。
5. 按 Provider 逐步实现，并补充解析、凭据脱敏、错误映射测试。
6. 回滚时可禁用 account usage manager 并忽略新增表；现有 token 监控继续运行。

## Open Questions

- 第一版 UI 入口应放在新 tab、Settings 扩展区域，还是现有 popup 中的紧凑账号卡片。
- Cursor 自动浏览器 cookie 发现应默认禁用发布，还是第一版只支持手动 cookie/session。
- GitHub Copilot 第一版应优先支持个人 quota、官方组织指标，还是两者都支持但分开 scope 展示。
- 第一版跨平台 OS 凭据存储应选择哪个 crate/API。
