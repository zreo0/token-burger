## Why

Token Burger 当前已经能统计本地 code Agent 的 token 消耗和成本估算，但用户还需要了解各账号在当前订阅周期内的使用量、剩余额度、重置时间、套餐状态和认证状态。新增独立的账号用量监控能力，可以在不干扰现有本地日志采集链路的前提下，为后续扩展更多 Provider 建立稳定基础。

## What Changes

- 新增账号用量监控能力，将 Provider 账号快照、额度窗口和用量指标独立存储，不写入 `token_logs`。
- 新增独立的账号用量 Provider 抽象，不复用现有面向本地日志解析的 `AgentAdapter`。
- 第一阶段支持 Codex、Claude Code、Cursor、GitHub Copilot，并为每个 Provider 明确数据来源、可信度和安全边界：
  - Codex：基于本地 auth 文件和认证后的账号用量接口获取额度窗口、套餐、重置时间和 credits。
  - Claude Code：基于本地日志提供 token/cost 用量汇总；在没有可靠账号额度来源时，明确展示“账号剩余额度不可用”。
  - Cursor：基于用户显式提供的 cookie/session 获取用量信息，标记为实验性/内部来源，并支持旧快照兜底。
  - GitHub Copilot：基于 GitHub token 获取可用的用量/额度信息，并明确区分个人、组织、团队、企业等指标作用域。
- 新增安全凭据处理：cookie、token、refresh token、API key 等密钥不得写入 SQLite，不得出现在日志、事件或前端状态中。
- 新增 Tauri commands/events 与前端状态，用于列出账号用量 Provider、读取快照、刷新 Provider、管理凭据和展示结果。
- 新增刷新调度、缓存优先展示、stale 状态、结构化错误、单飞刷新、退避和限流处理。

## Capabilities

### New Capabilities
- `account-usage-monitoring`：追踪账号级用量、额度、重置窗口、Provider 状态、凭据需求、刷新行为，并定义 Codex、Claude Code、Cursor、GitHub Copilot 的第一阶段行为。

### Modified Capabilities

None.

## Impact

- Rust 后端：新增 `account_usage` 模块、Provider registry、刷新 manager、SQLite 持久化、凭据抽象和 IPC commands。
- 数据库：新增账号用量快照、指标、Provider 状态和刷新元数据表；现有 `token_logs` 不变。
- 前端：新增账号用量 context/hooks，以及 Provider 卡片、stale 状态、错误状态、额度窗口和手动刷新 UI。
- 安全：引入 OS 级凭据存储或等价安全存储，并对 Provider 请求、响应、日志、事件做脱敏约束。
- 运行时：新增针对已启用 Provider 的网络请求，要求具备 timeout、backoff、rate-limit 处理和可配置刷新间隔。
