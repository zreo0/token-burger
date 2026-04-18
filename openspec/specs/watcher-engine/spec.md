# Purpose
定义 WatcherEngine 的调度、监听策略、冷启动与事件广播行为。

## Requirements

### Requirement: 统一调度器
WatcherEngine SHALL 作为单独的后台线程运行，接收所有已启用 Adapter 的注册，根据每个 Adapter 的 `data_source()` 返回值自动分发到对应的监听策略（Notify / Polling / SQLite）。

#### Scenario: 启动时注册 Adapter
- **WHEN** 系统启动并完成冷启动
- **THEN** WatcherEngine 遍历所有启用的 Adapter，按 DataSource 类型分组，启动对应的监听策略

#### Scenario: Agent 动态启用/禁用
- **WHEN** 用户在 Settings 中禁用某个 Agent
- **THEN** WatcherEngine 停止该 Agent 的监听，不影响其他 Agent

### Requirement: Notify 策略（JSONL 文件）
系统 SHALL 使用 `notify-debouncer-full` 监听 JSONL 类型的日志目录，debounce 间隔 MUST 为 500ms。监听模式为 `RecursiveMode::Recursive`，仅处理 `.jsonl` 扩展名的文件变更事件。

#### Scenario: 文件内容变更触发解析
- **WHEN** Claude Code 的 JSONL 文件被追加写入
- **THEN** 500ms debounce 后，系统读取增量内容并调用 Adapter 解析

#### Scenario: 高频写入合并
- **WHEN** Agent 在 200ms 内连续写入 10 次
- **THEN** 系统仅触发一次解析（500ms debounce 合并）

#### Scenario: 非目标文件过滤
- **WHEN** 监听目录中出现 `.json` 或 `.log` 文件变更
- **THEN** 系统忽略该事件

### Requirement: Polling 策略（JSON 文件）
系统 SHALL 以可配置的间隔（默认 10 秒）轮询 JSON 类型的日志目录，通过比对文件 mtime 判断是否有变更。变更的文件 SHALL 全量解析。

#### Scenario: mtime 变更触发解析
- **WHEN** 轮询检测到某 JSON 文件的 mtime 大于上次记录
- **THEN** 全量读取并解析该文件

#### Scenario: mtime 未变不解析
- **WHEN** 轮询检测到文件 mtime 未变
- **THEN** 跳过该文件

### Requirement: SQLite 策略（外部数据库）
系统 SHALL 以可配置的间隔（默认 10 秒）查询外部 SQLite 数据库，使用时间戳进行增量查询（`created_at > since`）。

#### Scenario: 增量查询新记录
- **WHEN** 定时查询触发且有新记录
- **THEN** 调用 Adapter 的 `query_db()` 获取新数据，发送到写线程

#### Scenario: 外部数据库不可访问
- **WHEN** 外部 SQLite 文件被锁定或不存在
- **THEN** 记录 warn 日志，跳过本轮查询，下次重试

### Requirement: Offset 断点续传
系统 SHALL 为 JSONL 文件维护读取偏移量（存储在 `file_offsets` 表）。每次读取前从表中获取 `last_offset`，读取完成后立即更新。

#### Scenario: 增量读取
- **WHEN** 文件从 offset 1024 处有新内容追加
- **THEN** 系统从 1024 开始读取，解析新增内容，更新 offset 为文件末尾位置

#### Scenario: 首次读取
- **WHEN** `file_offsets` 表中无该文件记录
- **THEN** 从文件开头（offset 0）开始读取

### Requirement: 文件轮转检测
系统 SHALL 在读取前检查文件大小。如果当前文件大小小于记录的 `last_offset`，MUST 将 offset 重置为 0（文件被截断或轮转）。

#### Scenario: 文件被截断
- **WHEN** 文件大小从 10240 变为 512（小于 last_offset 10240）
- **THEN** 系统重置 offset 为 0，从头读取

#### Scenario: 文件正常增长
- **WHEN** 文件大小从 10240 变为 12288（大于 last_offset 10240）
- **THEN** 系统从 10240 开始增量读取

### Requirement: 冷启动编排
系统首次启动时 SHALL 进入冷启动模式：逐 Agent 在后台线程解析历史日志，按 mtime 过滤最近 N 天（默认 30 天，可配置）的文件。每完成一个 Agent MUST 通过 `emit("cold-start-progress", ...)` 广播进度。全部完成后切换到正常监听模式。

#### Scenario: 冷启动进度广播
- **WHEN** 冷启动完成 Claude Code 的历史解析
- **THEN** 系统 emit `cold-start-progress` 事件，payload 包含 `{ agent: "claude-code", done: true, total: 5, completed: 1 }`

#### Scenario: 冷启动 mtime 过滤
- **WHEN** `~/.claude/projects/` 下有 90 天前的 JSONL 文件
- **THEN** 系统跳过该文件（超出 30 天保留期）

#### Scenario: 增量可用
- **WHEN** Claude Code 冷启动完成但 Codex 尚未开始
- **THEN** 前端已可展示 Claude Code 的数据

### Requirement: 事件广播
每次入库完成后，写线程 SHALL 查询当日 token 汇总，通过 `app.emit("token-updated", summary)` 广播给前端，并更新 tray title。

#### Scenario: 入库后广播
- **WHEN** 写线程完成一批 TokenLog 的插入
- **THEN** 查询今日汇总 → emit `token-updated` 事件 → 更新 tray title 为格式化后的总量

#### Scenario: Tray title 格式化
- **WHEN** 今日总 token 数为 1234567
- **THEN** tray title 显示 `1.2M`
