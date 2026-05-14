## 1. 后端基础

- [x] 1.1 创建 `src-tauri/src/account_usage/` 模块结构，包含 `mod.rs`、`manager.rs`、`store.rs`、`credentials.rs` 和 `providers/` 子模块。
- [x] 1.2 定义账号用量领域类型，包括 Provider 元数据、能力、来源类型、可信度、状态、快照、指标、结构化错误和刷新结果。
- [x] 1.3 定义 `AccountUsageProvider` trait，并为 `codex`、`claude-code`、`cursor`、`github-copilot` 建立静态 Provider 注册表。
- [x] 1.4 添加账号用量 manager state，支持 Provider 启用状态、缓存快照加载、单飞刷新保护、timeout 处理和 stale 兜底。
- [x] 1.5 添加脱敏工具，覆盖 Provider 错误、URL、headers、cookies、tokens 和响应片段。

## 2. SQLite 持久化

- [x] 2.1 使用 `CREATE TABLE IF NOT EXISTS` 添加 `account_usage_snapshots`、`account_usage_metrics`、`account_usage_provider_states` 表初始化。
- [x] 2.2 实现账号用量存储函数，支持原子化 upsert 快照并替换对应指标。
- [x] 2.3 实现按 provider/account 分组查询最新快照的能力，包含 stale 快照和结构化错误字段。
- [x] 2.4 实现 Provider 状态持久化，保存启用状态、最近刷新时间、retry-after/backoff 时间和非密钥凭据引用。
- [x] 2.5 添加 Rust 测试，覆盖 schema 创建、快照 upsert、指标替换、stale 状态保留以及不影响 `token_logs`。

## 3. 凭据处理

- [x] 3.1 选择并添加 OS 凭据存储依赖，或实现跨平台凭据存储抽象，用于保存用户管理的密钥。
- [x] 3.2 实现 Provider 密钥的保存、读取、删除 API，确保密钥值不写入 SQLite。
- [x] 3.3 实现非密钥凭据引用和元数据在 Provider state 中的持久化。
- [x] 3.4 确保基于 auth 文件的 Provider 只引用本地 auth 文件，不将 token 值复制进 SQLite。
- [x] 3.5 添加测试或受控集成检查，证明已保存凭据不会出现在日志、事件和 SQLite 行中。

## 4. Tauri Commands 与事件

- [x] 4.1 添加 command，用于列出账号用量 Provider 及其能力/凭据元数据。
- [x] 4.2 添加 command，用于读取缓存的账号用量快照和指标。
- [x] 4.3 添加 command，用于刷新全部已启用 Provider 和刷新单个 Provider。
- [x] 4.4 添加 command，用于保存、清除和检查 Provider 凭据的非密钥状态。
- [x] 4.5 添加 command，用于更新账号用量 Provider 的启用状态和刷新间隔设置。
- [x] 4.6 在 Tauri 启动阶段注册 commands 和 managed state，且不改变现有 token commands。
- [x] 4.7 在快照或状态变化后发送 `account-usage-updated` 事件，并在需要时更新能力元数据。

## 5. Codex Provider

- [x] 5.1 实现 Codex auth 文件发现，覆盖默认 auth 路径以及配置的 auth 文件/目录环境变量。
- [x] 5.2 实现 Codex auth 解析，提取 access token、refresh token、id token、account id、email、plan、user id、过期时间和 API-key 模式检测结果。
- [x] 5.3 实现 Codex OAuth refresh；当 unauthorized 且 refresh token 可用时，只重试一次用量请求。
- [x] 5.4 实现 Codex 用量 endpoint 解析和认证请求，并确保错误已脱敏。
- [x] 5.5 将 Codex quota windows、credits、plan、reset timestamps 和 workspace/account identity 解析为快照和指标。
- [x] 5.6 确保 Codex 身份在 source path 与 workspace metadata 可用时能区分共享 email 或用户级 account id 的不同 workspace。
- [x] 5.7 使用 fixture payload 添加单元测试，覆盖成功、token refresh、unauthorized、缺失 auth、schema 失败和 workspace 分离。

## 6. Claude Code Provider

- [x] 6.1 实现 Claude Code 账号用量 Provider，从现有 token 数据库 summary 派生本地用量，而不是远程账号额度。
- [x] 6.2 添加今日、7 天、30 天和/或本月本地 token 用量指标，并在 pricing 数据可用时包含估算成本。
- [x] 6.3 将 Claude Code 账号额度标记为 unsupported/unavailable，并暴露来源可信度为本地日志数据。
- [x] 6.4 确保 Claude Code Provider 刷新不会绕过现有 watcher pipeline 重新扫描日志，除非明确必要。
- [x] 6.5 添加测试，覆盖本地用量 summary、空数据、unsupported quota 状态以及不修改 token logs。

## 7. Cursor Provider

- [x] 7.1 实现显式 Cursor cookie/web-session 凭据支持，并接入 OS 凭据存储。
- [x] 7.2 默认禁用自动浏览器 cookie 发现；仅在实际实现发现能力时添加 opt-in 状态。
- [x] 7.3 实现 Cursor 账号和用量请求，包含 cookie header、类浏览器 user agent、timeout 和脱敏失败信息。
- [x] 7.4 将 Cursor 账号 email/name/id、membership/plan、billing reset、plan usage、auto usage 和 API usage 解析为快照和指标。
- [x] 7.5 将 Cursor 来源标记为 experimental/internal-source，并在 auth、parse 或 schema 错误时使用 stale 兜底。
- [x] 7.6 使用 fixture payload 添加测试，覆盖成功、session 过期、schema 变化、缺失凭据和 stale 快照保留。

## 8. GitHub Copilot Provider

- [x] 8.1 实现显式 GitHub token 凭据支持，并可选从允许的开发者环境来源发现 token。
- [x] 8.2 实现 Copilot 用量/额度请求，包含必需 GitHub headers、timeout 和脱敏错误。
- [x] 8.3 在可用时获取账号身份，并将快照关联到稳定的 GitHub login/email/account key。
- [x] 8.4 解析个人 Copilot quota 指标，例如 premium interactions、chat、completions、plan、remaining values 和 reset time。
- [x] 8.5 用明确 scope 表示 organization/team/enterprise 指标，并防止它们被展示为个人剩余额度。
- [x] 8.6 添加测试，覆盖个人额度成功、组织作用域指标、缺失 token、无效 token 和备用 quota 格式。

## 9. 前端状态与 UI

- [x] 9.1 添加 TypeScript 类型，与账号用量 Provider 元数据、快照、指标、状态、可信度和来源标签保持一致。
- [x] 9.2 添加 `AccountUsageContext` 和 `useAccountUsage` hook，支持缓存加载、刷新全部、刷新 Provider、凭据操作和事件监听。
- [x] 9.3 添加账号用量 UI 卡片，展示 Provider、账号标签、套餐、来源/可信度、指标、重置时间、stale/error/auth 状态和手动刷新。
- [x] 9.4 添加 Provider 设置控件，支持启用状态、凭据录入/移除、刷新间隔和实验性来源警告。
- [x] 9.5 保持现有 `TokenContext` 和 token summary UI 独立，不受账号用量状态更新影响。
- [x] 9.6 添加前端测试，覆盖正常、stale、unsupported quota、auth-required 和实验性 Provider 状态渲染。

## 10. 刷新策略与错误处理

- [x] 10.1 为 API、本地和实验性来源实现默认刷新间隔与按 Provider 覆盖。
- [x] 10.2 为限流 Provider 响应实现 backoff 和 `Retry-After` 处理。
- [x] 10.3 将 Provider 失败映射为结构化状态，例如 auth-required、forbidden、rate-limited、network、schema-changed、unsupported 和 error。
- [x] 10.4 确保刷新失败时保留上一次成功快照并标记 stale，同时发送更新事件。
- [x] 10.5 添加测试，覆盖并发刷新去重、限流 backoff、stale 兜底和结构化错误映射。

## 11. 验证

- [x] 11.1 运行 Rust 格式化和后端账号用量模块测试。
- [x] 11.2 运行前端 lint、typecheck 和账号用量 UI/state 测试。
- [x] 11.3 手动验证现有 token 监控、托盘更新和 token summary commands 仍可正常工作。
- [x] 11.4 手动验证第一阶段每个 Provider 在缺失凭据和 fixture-backed 成功快照下展示正确状态。
- [x] 11.5 验证生成的日志、SQLite 行、Tauri 事件和前端状态均不包含原始密钥。
