# Purpose
定义 WatcherEngine 的调度、监听策略、冷启动与事件广播行为。

## Requirements

### Requirement: 统一调度器
WatcherEngine SHALL 作为单独的后台线程运行，接收所有已启用 Agent source 的注册，根据每个 source 的数据源类型自动分发到对应的读取策略（Notify / Polling / SQLite）。读取策略 SHALL 生成 `AgentDataBatch` 或等价内部 batch，并将同一批 batch fan-out 给 Token extractor 与可选 Behavior extractor。WatcherEngine MUST 继续尊重 Settings 中的 Agent 启用状态。

#### Scenario: 启动时注册 source
- **WHEN** 系统启动并完成冷启动
- **THEN** WatcherEngine 遍历所有启用的 Agent source，按数据源类型分组，启动对应的监听策略

#### Scenario: Agent 动态启用/禁用
- **WHEN** 用户在 Settings 中禁用某个 Agent
- **THEN** WatcherEngine 停止该 Agent 的监听、Token 解析和行为解析，不影响其他 Agent

#### Scenario: 同批 fan-out
- **WHEN** 某个已启用 Agent 产生新增数据
- **THEN** WatcherEngine 只读取一次该新增数据，并将同一 batch 分发给 token 和行为消费者

### Requirement: Notify 策略（JSONL 文件）
系统 SHALL 使用 `notify-debouncer-full` 监听 JSONL 类型的日志目录，debounce 间隔 MUST 为 500ms。监听模式为 `RecursiveMode::Recursive`，仅处理 `.jsonl` 扩展名的文件变更事件。文件变更后，策略 SHALL 从记录的 offset 读取新增内容，生成 JSONL 增量 batch，并由 watcher 分发给解析消费者。

#### Scenario: 文件内容变更触发 batch
- **WHEN** Claude Code 或 Codex 的 JSONL 文件被追加写入
- **THEN** 500ms debounce 后，系统读取增量内容，生成 batch，并调用对应 Token extractor

#### Scenario: 高频写入合并
- **WHEN** Agent 在 200ms 内连续写入 10 次
- **THEN** 系统仅触发一次解析（500ms debounce 合并）

#### Scenario: 非目标文件过滤
- **WHEN** 监听目录中出现 `.json` 或 `.log` 文件变更
- **THEN** 系统忽略该事件

#### Scenario: Codex 行为同源解析
- **WHEN** Codex JSONL 增量 batch 在正常监听阶段生成且行为提示开启
- **THEN** 系统从同一 batch 同时执行 token 解析和行为解析

### Requirement: Polling 策略（JSON 文件）
系统 SHALL 以可配置的间隔（默认 10 秒）轮询 JSON 类型的日志目录，通过比对文件 mtime 判断是否有变更。变更的文件 SHALL 全量读取为 JSON batch，并由 watcher 分发给解析消费者。

#### Scenario: mtime 变更触发 batch
- **WHEN** 轮询检测到某 JSON 文件的 mtime 大于上次记录
- **THEN** 全量读取该文件，生成 batch，并调用对应 Token extractor

#### Scenario: mtime 未变不解析
- **WHEN** 轮询检测到文件 mtime 未变
- **THEN** 跳过该文件

### Requirement: SQLite 策略（外部数据库）
系统 SHALL 以可配置的间隔（默认 10 秒）查询外部 SQLite 数据库。对于支持 `message(session_id, time_created, id)` 索引的 SQLite Agent，系统 MUST 使用 per-session cursor 作为高频增量主路径：按 session 查询 `time_created` 和 `id` 之后的新建 message row，并将结果封装为 SQLite row batch，由 watcher 分发给 Token extractor 与可选 Behavior extractor。系统 MUST NOT 在高频轮询主路径中继续使用全局 `time_updated > ? ORDER BY time_updated` 扫描 `message` 全表。系统 MUST NOT 修改外部 Agent 数据库或为外部数据库创建索引。

#### Scenario: 增量查询新 row
- **WHEN** 定时查询触发且某 session 存在 `created_cursor` 之后的新建 message row
- **THEN** 系统使用 `session_id + time_created + id` 查询该 session 的新增 row，生成 SQLite row batch，调用对应解析器，并在处理成功后推进该 session 的 created cursor

#### Scenario: 外部数据库不可访问
- **WHEN** 外部 SQLite 文件被锁定或不存在
- **THEN** 记录 warn 日志，跳过本轮查询，下次重试

#### Scenario: OpenCode 单查询 fan-out
- **WHEN** OpenCode SQLite 查询返回新的 message row
- **THEN** 系统使用同一批 row 执行 token 解析和行为解析，不启动独立行为轮询

#### Scenario: MiMoCode 单查询 fan-out
- **WHEN** MiMoCode SQLite 查询返回新的 message row
- **THEN** 系统使用同一批 row 执行 token 解析和行为解析，不启动独立行为轮询

#### Scenario: 不修改外部数据库
- **WHEN** SQLite 策略初始化或轮询 OpenCode/MiMoCode 外部数据库
- **THEN** 系统只以只读方式打开外部数据库，不创建索引、不修改 schema、不写入外部表

### Requirement: SQLite source cursor bootstrap
系统 SHALL 将旧版全局 SQLite watermark 迁移为 per-session cursor。若某外部 SQLite source 尚无 per-session cursor，且本地 `file_offsets` 中存在旧 key `sqlite:<db_path>`，系统 MUST 使用该旧 watermark 初始化每个 session 的 created cursor 与 updated cursor。bootstrap MUST NOT 解析历史 message row，MUST NOT 写入 TokenLog，MUST NOT 从头补扫既有历史数据。

#### Scenario: 从旧 watermark 初始化 cursor
- **WHEN** 已运行用户升级后首次启动 SQLite source，且存在旧 `sqlite:<db_path>` watermark
- **THEN** 系统为每个 session 查询旧 watermark 之前最后一条 message 的 `(time_created, id)`，写入该 session 的 created cursor，并将 updated cursor 初始化为旧 watermark 减去 overlap 窗口

#### Scenario: 无旧 watermark 首次接入
- **WHEN** SQLite source 没有 per-session cursor 且没有旧 `sqlite:<db_path>` watermark
- **THEN** 系统按 session 分批读取历史 message，入库成功后建立 per-session cursor

#### Scenario: bootstrap 后不双跑
- **WHEN** 某 SQLite source 已成功建立 per-session cursor
- **THEN** 正常轮询阶段只运行 per-session 读取策略，不再同时运行旧版全局 `time_updated` 轮询主路径

### Requirement: SQLite source update reconciliation
系统 SHALL 为外部 SQLite source 提供旧 message 更新校准机制。系统 MUST 对已知 session 分批执行 `time_updated` 校准查询，并使用 overlap 窗口允许重复读取。校准查询 MUST 将扫描范围限制在单个 session 内，不得高频执行全局 `message` 全表扫描。重复读取的 row MUST 依赖 TokenLog upsert 保持幂等。

#### Scenario: 按 session 校准更新
- **WHEN** 某 session 的 updated reconciliation 到期
- **THEN** 系统查询该 session 中 `time_updated` 大于等于 updated cursor 减 overlap 的 row，并将结果封装为 SQLite row batch

#### Scenario: 校准分批执行
- **WHEN** 已知 session 数量较多
- **THEN** 系统每轮只处理有限数量 session 的 updated reconciliation，未处理 session 保留 cursor 并在后续轮次继续

#### Scenario: 校准重复读取安全
- **WHEN** overlap 导致同一 message row 被重复读取
- **THEN** 下游 TokenLog 写入通过 `(request_id, token_type)` upsert 更新或保持已有记录，不产生重复统计

#### Scenario: session 更新时间不作为唯一依据
- **WHEN** `session.time_updated` 小于该 session 下 message 的最大 `time_updated`
- **THEN** 系统仍必须能通过 per-session updated reconciliation 最终发现旧 message 更新，不得只依赖 `session.time_updated` 过滤候选 session

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
系统首次启动时 SHALL 进入冷启动模式：逐 Agent 在后台线程解析历史日志，按 mtime 或 source watermark 过滤最近 N 天（默认 30 天，可配置）的数据。每完成一个 Agent MUST 通过 `emit("cold-start-progress", ...)` 广播进度。系统 MUST 维护可靠的冷启动完成状态；全部完成后 MUST 标记冷启动完成，并切换到正常监听模式。冷启动期间 SHALL 只执行 token 解析，不得产生行为提示。

#### Scenario: 冷启动进度广播
- **WHEN** 冷启动完成 Claude Code 的历史解析
- **THEN** 系统 emit `cold-start-progress` 事件，payload 包含 `{ agent: "claude-code", done: true, total: 5, completed: 1 }`

#### Scenario: 冷启动 mtime 过滤
- **WHEN** `~/.claude/projects/` 下有 90 天前的 JSONL 文件
- **THEN** 系统跳过该文件（超出 30 天保留期）

#### Scenario: 增量可用
- **WHEN** Claude Code 冷启动完成但 Codex 尚未开始
- **THEN** 数据可以入库并参与后续汇总，但主托盘 token title 不得把该部分数据展示为最终完成状态

#### Scenario: 冷启动完成状态
- **WHEN** 所有启用 Agent 的冷启动解析都已完成
- **THEN** 系统标记冷启动完成，并允许主托盘恢复正常 token 汇总展示与 Popup 打开行为

#### Scenario: 无启用 Agent
- **WHEN** 冷启动开始时没有任何启用 Agent 需要扫描
- **THEN** 系统立即标记冷启动完成，并进入正常监听模式

#### Scenario: 冷启动不分发行为
- **WHEN** 冷启动读取到 Codex 权限请求或 OpenCode 完成记录
- **THEN** 系统只补录 token 数据，不生成、不入队、不展示行为提示

### Requirement: 事件广播
每次入库完成后，写线程 SHALL 查询当日 token 汇总，通过 `app.emit("token-updated", summary)` 广播给前端，并更新 tray title。

#### Scenario: 入库后广播
- **WHEN** 写线程完成一批 TokenLog 的插入
- **THEN** 查询今日汇总、emit `token-updated` 事件并更新 tray title

#### Scenario: Tray title 格式化
- **WHEN** 今日总 token 数为 1234567
- **THEN** tray title 显示 `1.2M`

### Requirement: 重构兼容性保护
WatcherEngine 重构 SHALL 不改变用户可见统计行为。Token 入库、`token-updated` 广播、tray title、Popup 查询、Settings Agent 开关和行为提示总开关 MUST 保持与重构前等价。

#### Scenario: 入库广播保持不变
- **WHEN** 写线程完成一批 TokenLog 的插入
- **THEN** 系统仍查询今日汇总、emit `token-updated` 事件并更新 tray title

#### Scenario: 行为提示关闭不解析
- **WHEN** 行为提示总开关关闭且 Agent 产生新增数据
- **THEN** watcher 仍执行 token 解析，但不调用 Behavior extractor

#### Scenario: Agent 关闭不监听
- **WHEN** 用户关闭某 Agent
- **THEN** watcher 不为该 Agent 启动 source 读取、token 解析或行为解析

#### Scenario: 前端接口不变
- **WHEN** 前端调用现有 Tauri command 或监听现有事件
- **THEN** command 名称、事件名称和 payload 结构保持不变
