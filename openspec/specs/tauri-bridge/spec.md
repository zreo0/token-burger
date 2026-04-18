# Purpose
定义 Tauri commands、事件广播、tray 更新与 capabilities 要求。

## Requirements

### Requirement: Token 汇总查询 Command
系统 SHALL 提供 `get_token_summary` command，接受 `range` 参数（`"today"` | `"7d"` | `"30d"`），返回 `TokenSummary` 结构体（含按 token_type 的总量、按 agent 和 model 的细分统计）。

#### Scenario: 查询今日汇总
- **WHEN** 前端调用 `invoke('get_token_summary', { range: 'today' })`
- **THEN** 返回今日 0:00 至当前时刻的 TokenSummary

#### Scenario: 无数据时返回零值
- **WHEN** 查询时间范围内无任何记录
- **THEN** 返回所有字段为 0 的 TokenSummary

### Requirement: Agent 列表查询 Command
系统 SHALL 提供 `get_agent_list` command，返回所有支持的 Agent 信息列表，包含 agent 名称、是否启用、是否可用（日志路径存在）、数据源类型。

#### Scenario: 获取 Agent 列表
- **WHEN** 前端调用 `invoke('get_agent_list')`
- **THEN** 返回 5 个 Agent 的信息，每个包含 `name`、`enabled`、`available`、`source_type` 字段（OpenCode 合并新旧模式为单个条目）

#### Scenario: Agent 日志路径不存在
- **WHEN** 用户未安装 Codex（`~/.codex/` 不存在）
- **THEN** 该 Agent 的 `available` 为 false

### Requirement: Agent 开关 Command
系统 SHALL 提供 `toggle_agent` command，接受 `agent` 名称和 `enabled` 布尔值，更新 `app_settings` 中的 `enabled_agents` 列表，并通知 WatcherEngine 动态启停监听。

#### Scenario: 禁用 Agent
- **WHEN** 前端调用 `invoke('toggle_agent', { agent: 'copilot', enabled: false })`
- **THEN** `enabled_agents` 中移除 `"copilot"`，WatcherEngine 停止 Copilot 的监听

### Requirement: 设置读写 Commands
系统 SHALL 提供 `get_settings` 和 `update_settings` commands，读写 `app_settings` 表中的用户配置。

#### Scenario: 获取设置
- **WHEN** 前端调用 `invoke('get_settings')`
- **THEN** 返回 `AppSettings` 对象（含 enabled_agents、watch_mode、keep_days、polling_interval_secs、language）

#### Scenario: 更新设置
- **WHEN** 前端调用 `invoke('update_settings', settings)`
- **THEN** 系统更新 `app_settings` 表，受影响的模块（如 WatcherEngine）立即应用新配置

### Requirement: 数据清理 Command
系统 SHALL 提供 `clear_data` command，接受可选的 `keep_days` 参数。有值时删除超出天数的记录，无值时清空全部数据。

#### Scenario: 按天数清理
- **WHEN** 前端调用 `invoke('clear_data', { keep_days: 7 })`
- **THEN** 删除 7 天前的 token_logs 记录

#### Scenario: 清空全部
- **WHEN** 前端调用 `invoke('clear_data', {})`
- **THEN** 清空 token_logs 和 file_offsets 表

### Requirement: 价格表查询 Command
系统 SHALL 提供 `get_pricing` command，返回当前加载的模型价格表。

#### Scenario: 获取价格表
- **WHEN** 前端调用 `invoke('get_pricing')`
- **THEN** 返回 `PricingTable`（模型名 → 价格映射）

### Requirement: token-updated 事件
系统 SHALL 在每次 token 数据入库后，通过 `app.emit("token-updated", payload)` 向前端广播 `TokenSummary`。

#### Scenario: 前端接收实时更新
- **WHEN** Watcher 检测到新的 token 数据并入库
- **THEN** 前端收到 `token-updated` 事件，payload 为最新的今日 TokenSummary

### Requirement: cold-start-progress 事件
系统 SHALL 在冷启动过程中，每完成一个 Agent 的历史解析后广播 `cold-start-progress` 事件。

#### Scenario: 冷启动进度通知
- **WHEN** 冷启动完成第 3 个 Agent（共 5 个）
- **THEN** 前端收到 `{ agent: "gemini-cli", done: true, total: 5, completed: 3 }`

### Requirement: Tray Title 动态更新
系统 SHALL 在每次 `token-updated` 事件广播时同步更新 tray title。格式化规则：`< 1000` 显示原数，`≥ 1000` 显示 K，`≥ 1000000` 显示 M，`≥ 1000000000` 显示 B。保留一位小数（如 `1.2K`、`3.5M`）。

#### Scenario: 格式化 token 数
- **WHEN** 今日总 token 数为 45678
- **THEN** tray title 显示 `45.7K`

#### Scenario: 零 token
- **WHEN** 今日无任何 token 记录
- **THEN** tray title 显示 `0`

### Requirement: Tauri Capabilities 声明
系统 MUST 在 `src-tauri/capabilities/` 中声明前端所需权限。文件系统读取和 HTTP 请求均由 Rust 后端直接执行（不经前端 IPC），因此只需声明 core 和 event 权限。

#### Scenario: 权限声明完整
- **WHEN** 应用构建
- **THEN** capabilities 文件包含 `core:default`、`core:event:default`（允许前端 listen/emit 事件）等必要权限
