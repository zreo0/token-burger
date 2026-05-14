# Purpose
定义账号用量/额度监控的独立管线、Provider 能力、刷新策略、持久化、安全凭据处理和 UI 状态分离要求。

## Requirements

### Requirement: 独立账号用量管线
系统 SHALL 提供独立于本地 token 日志采集的账号用量监控管线。账号用量数据 MUST NOT 写入 `token_logs`，账号用量 Provider 的刷新执行 MUST NOT 依赖现有 `AgentAdapter` trait。

#### Scenario: 账号用量不修改 token 日志
- **WHEN** 账号用量 Provider 刷新额度或账号状态
- **THEN** 系统将结果写入账号用量存储，并保持 `token_logs` 不变

#### Scenario: 现有 token 监控继续运行
- **WHEN** 账号用量监控被禁用或某个 Provider 刷新失败
- **THEN** 现有 token 日志监听和 token summary commands 继续正常运行

### Requirement: Provider 能力注册表
系统 SHALL 定义账号用量 Provider 注册表，每个 Provider 声明 id、显示名称、数据来源类型、支持能力、刷新间隔策略、凭据要求和是否实验性。注册表 MUST 包含第一阶段 Provider：`codex`、`claude-code`、`cursor`、`github-copilot`。

#### Scenario: 前端获取 Provider 能力
- **WHEN** 前端请求账号用量 Provider 元数据
- **THEN** 后端返回所有已注册 Provider，并包含足以支撑 UI 标签展示的能力与凭据元数据

#### Scenario: 后续新增 Provider
- **WHEN** 新增 Provider 模块并注册到账号用量 Provider registry
- **THEN** 该 Provider 出现在账号用量 Provider 列表中，且不需要修改已有 Provider 实现

### Requirement: 快照与指标持久化
系统 SHALL 使用 Provider 账号快照加零个或多个指标行来持久化账号用量。快照 MUST 包含 provider id、账号身份 hash 或 key、可用时的脱敏账号标签、套餐、状态、来源、可信度、观测时间、可选周期/重置时间、stale 标记和结构化错误码。每个指标 MUST 包含 metric key、label、unit、scope，以及可选的 used、limit、remaining、percentage 字段。

#### Scenario: 多窗口额度持久化
- **WHEN** Provider 返回短窗口和长窗口额度数据
- **THEN** 系统存储一个账号快照，并为每个额度窗口分别存储指标行

#### Scenario: 不支持账号额度时的持久化
- **WHEN** Provider 能报告本地用量但不能可靠报告账号额度
- **THEN** 系统存储本地用量指标，并将账号额度标记为 unsupported 或不返回额度指标，不得编造剩余额度

### Requirement: 安全凭据处理
系统 MUST 仅将用户管理的 cookie、access token、refresh token、API key、bearer token 等密钥存储在 OS 凭据存储或其他批准的安全存储中。SQLite MUST NOT 包含密钥值。日志、事件、错误和前端 payload MUST 对密钥脱敏。

#### Scenario: 保存 Provider 凭据
- **WHEN** 用户为账号用量监控保存 cookie、token、API key 或可刷新的凭据
- **THEN** 密钥存入 OS 凭据存储，SQLite 只保存非密钥凭据引用和元数据

#### Scenario: 刷新失败错误脱敏
- **WHEN** Provider 请求失败，错误内容包含请求 header 或凭据
- **THEN** 前端收到结构化且已脱敏的错误，日志和事件中不出现任何密钥值

### Requirement: 缓存优先刷新行为
系统 SHALL 在执行网络刷新前先从 SQLite 加载最近一次成功的账号用量快照。刷新 MUST 异步执行，MUST 按 provider/account key 单飞，MUST 使用 timeout，且 MUST 在刷新失败时保留最近一次成功快照并标记 stale。

#### Scenario: 启动时使用缓存数据
- **WHEN** 应用启动且存在缓存的账号用量快照
- **THEN** 前端可以在远程 Provider 刷新完成前展示缓存快照

#### Scenario: 刷新失败保留 stale 快照
- **WHEN** Provider 在已有成功快照后因网络、schema、认证或限流错误刷新失败
- **THEN** 系统保留上一次快照，将其标记为 stale，并记录结构化状态/错误码

#### Scenario: 并发刷新去重
- **WHEN** 多个 UI 操作同时请求刷新同一个 provider/account
- **THEN** 后端只执行一次 Provider 刷新，并复用同一结果更新状态

### Requirement: 账号用量 Tauri commands 与事件
系统 SHALL 暴露 Tauri commands，用于列出账号用量 Provider、读取账号用量快照、刷新全部已启用账号用量 Provider、刷新单个 Provider、保存或清除 Provider 凭据、更新 Provider 启用状态。系统 SHALL 在快照变化后发送 `account-usage-updated` 事件。

#### Scenario: 读取缓存账号用量
- **WHEN** 前端调用账号用量读取 command
- **THEN** 后端返回按 provider/account 分组的最新快照和指标

#### Scenario: 手动刷新单个 Provider
- **WHEN** 前端请求刷新 `codex`
- **THEN** 后端只刷新 Codex 账号用量 Provider，并在状态变化时发送 `account-usage-updated`

#### Scenario: 凭据更新触发刷新
- **WHEN** 用户保存或清除 Provider 凭据
- **THEN** 后端更新凭据元数据，并调度或执行 Provider 刷新以更新账号状态

### Requirement: Codex 账号用量 Provider
Codex 账号用量 Provider SHALL 解析支持的 Codex auth 文件，在需要时刷新 OAuth 凭据，请求认证后的账号用量，并解析额度窗口、重置时间戳、套餐、credits、账号身份和 workspace 上下文。Codex 身份在 source file path 或 workspace metadata 可用时 MUST NOT 只依赖 email 或原始 account id。

#### Scenario: Codex 额度刷新成功
- **WHEN** 存在有效 Codex auth 文件，且账号用量接口返回额度窗口
- **THEN** 系统存储 Codex 快照，包含账号、套餐、来源元数据，并为每个返回的额度窗口存储指标

#### Scenario: Codex token 刷新重试
- **WHEN** Codex access token 已过期，或第一次用量请求返回 unauthorized，且 refresh token 可用
- **THEN** Provider 刷新 token 一次，重试用量请求，并仅持久化非密钥快照数据

#### Scenario: Codex workspace 分离
- **WHEN** 两个 Codex workspace 使用相同 email 或用户级 account id，但使用不同 auth file path 或 workspace metadata
- **THEN** 系统将它们表示为不同的账号用量条目

### Requirement: Claude Code 本地用量 Provider
Claude Code 账号用量 Provider SHALL 基于已有 Claude Code token 日志暴露本地用量和成本估算快照。除非后续实现可靠账号额度来源，否则它 MUST 将账号额度标记为 unsupported 或 unavailable。

#### Scenario: Claude 本地用量汇总
- **WHEN** 本地数据库中存在 Claude Code token 日志
- **THEN** Provider 返回相关周期的本地用量指标，并将来源可信度标记为本地日志数据

#### Scenario: Claude 账号额度不可用
- **WHEN** UI 展示 Claude Code 账号用量
- **THEN** UI 明确展示订阅剩余额度不可用，而不是从本地 token 日志估算额度

### Requirement: Cursor 账号用量 Provider
Cursor 账号用量 Provider SHALL 支持用户显式提供的 cookie 或 web session 凭据。如果实现自动浏览器 cookie 发现，则该能力 MUST 在用户显式启用前保持禁用。Cursor 用量数据 MUST 标记为实验性/内部来源，并在解析或认证失败时使用 stale 兜底。

#### Scenario: Cursor 手动 session 刷新成功
- **WHEN** 用户提供有效 Cursor session 凭据并请求刷新
- **THEN** Provider 获取 Cursor 用量/账号数据，存储套餐、重置时间和用量指标，并将来源标记为实验性或内部来源

#### Scenario: Cursor 自动发现必须 opt-in
- **WHEN** 自动浏览器 session 发现未启用
- **THEN** Provider 不解密或读取浏览器 cookie

#### Scenario: Cursor schema 失败
- **WHEN** Cursor 返回的数据在已有成功快照后无法解析
- **THEN** 系统保留上一份快照并标记 stale，同时记录 schema 相关错误码

### Requirement: GitHub Copilot 账号用量 Provider
GitHub Copilot 账号用量 Provider SHALL 支持显式 GitHub token 凭据，也可从允许的开发者环境来源发现 token。Provider SHALL 在可用时获取用量/额度数据，并 MUST 标记指标是 personal、organization、team 还是 enterprise 作用域。组织级指标 MUST NOT 展示为个人剩余额度。

#### Scenario: Copilot 个人额度刷新
- **WHEN** 有效 GitHub token 具备 Copilot 访问权限并返回个人额度数据
- **THEN** 系统存储个人 Copilot 指标，例如 premium interactions、chat、completions、reset date 和可用套餐信息

#### Scenario: Copilot 组织指标标记作用域
- **WHEN** GitHub token 返回 organization、team 或 enterprise 指标
- **THEN** 系统使用对应 scope 存储这些指标，且不把它们展示为个人账号额度

#### Scenario: Copilot token 缺失
- **WHEN** 不存在可用 GitHub token
- **THEN** Provider 返回 auth-required 状态，不发起未认证额度请求

### Requirement: 账号用量 UI 状态分离
前端 SHALL 将账号用量状态与 token summary 状态分开管理。账号用量 UI MUST 展示 Provider、账号标签、套餐、状态、来源/可信度标签、指标、可用时的重置时间、stale/error/auth 状态和手动刷新控制。

#### Scenario: 账号用量卡片展示 stale 状态
- **WHEN** Provider 因刷新失败只剩 stale 缓存快照
- **THEN** UI 展示最近一次已知指标，并显示 stale/error 提示，不完全隐藏卡片

#### Scenario: 账号用量不影响 token summary UI
- **WHEN** 账号用量快照更新
- **THEN** 现有 token summary 视图继续使用 `TokenContext`，除非 token 数据变化，否则不强制重新计算 token summary
