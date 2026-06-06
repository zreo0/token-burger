## MODIFIED Requirements

### Requirement: Agent Source Pipeline 实现
系统 SHALL 将现有 token-centric adapter contract 重构为 Agent source 与解析器分离的内部 contract。每个已支持 Agent MUST 提供稳定的 Agent source 描述，包括 `agent_name`、数据源类型、监听路径或 SQLite 位置，以及用于 offset/watermark 的 source key。Token 解析 MUST 由 Token extractor 完成；行为解析 MUST 由独立 Behavior extractor 完成。解析器 MUST NOT 自行执行 glob 扫描、notify 注册、文件读取、SQLite 打开查询或数据源发现。

#### Scenario: Source 注册
- **WHEN** 系统初始化 watcher
- **THEN** 所有已启用 Agent 的 source 被注册到 WatcherEngine，并保留原有 `agent_name` 标识

#### Scenario: 解析器不拥有读取入口
- **WHEN** Token extractor 或 Behavior extractor 运行
- **THEN** 它只接收 watcher 提供的 batch，并不得自行打开文件、扫描路径或查询 SQLite

#### Scenario: 旧 token 输出兼容
- **WHEN** 同一批 Codex、OpenCode、Claude Code 或 Gemini CLI 记录在重构前后被解析
- **THEN** 生成的 `TokenLog` 在 agent、provider、model、token_type、token_count、session_id、request_id、cost 和 timestamp 语义上保持一致

#### Scenario: 不支持的解析器显式为空
- **WHEN** 某 Agent 暂不支持行为解析
- **THEN** 该 Agent 不注册 Behavior extractor 或返回空事件集合，不得影响 token 解析

## ADDED Requirements

### Requirement: AgentDataBatch 输入模型
系统 SHALL 定义统一的内部 batch 模型，用于表达 watcher 已读取的一批 Agent 数据。batch MUST 覆盖 JSONL 增量内容、JSON 全量内容和 SQLite row 三类输入，并携带 agent_name、source key、路径或数据库标识，以及本批次对应的 offset/watermark 上下文。

#### Scenario: JSONL 增量 batch
- **WHEN** JSONL 文件从 offset N 追加内容到 offset M
- **THEN** 系统生成包含路径、agent_name、新增内容、N 和 M 的 JSONL batch

#### Scenario: JSON 全量 batch
- **WHEN** JSON 文件 mtime 变化并被全量读取
- **THEN** 系统生成包含路径、agent_name、完整内容和 mtime 的 JSON batch

#### Scenario: SQLite row batch
- **WHEN** SQLite 轮询读取到 watermark 之后的新 row
- **THEN** 系统生成包含数据库路径、agent_name、row 集合、上一水位线和下一水位线的 SQLite batch

### Requirement: TokenExtractor 输出兼容
系统 SHALL 将现有 Token 解析逻辑迁移到 Token extractor，同时保持 `TokenLog` 结构和下游入库语义不变。Token extractor MUST 返回 token logs，并 MAY 返回解析状态元数据（如 Codex 最终 model）供 watcher 更新 parser cache；Token extractor 不得更新 offset/watermark，不得广播前端事件。

#### Scenario: Token extractor 不更新水位线
- **WHEN** Token extractor 成功解析 batch
- **THEN** 它返回 token logs 和可选解析状态，offset 或 watermark 仍由 watcher 读取层统一推进

#### Scenario: 解析异常隔离
- **WHEN** Token extractor 遇到部分损坏的 JSON 行或 row data
- **THEN** 它跳过异常记录并继续处理剩余记录，不得中断 watcher 或行为解析

### Requirement: BehaviorExtractor 与 TokenExtractor 同级
系统 SHALL 将行为解析作为 Token 解析的同级消费者。Behavior extractor MUST 从同一个 `AgentDataBatch` 产出 `AgentBehaviorEvent`，并且不得依赖 Token extractor 的输出。

#### Scenario: 同 batch fan-out
- **WHEN** watcher 获取到一批新增 Codex JSONL 内容
- **THEN** 同一个 batch 可被 Token extractor 和 Behavior extractor 分别消费

#### Scenario: 行为解析关闭时跳过
- **WHEN** 行为提示总开关关闭
- **THEN** 系统不调用 Behavior extractor，也不缓存本批次行为事件

### Requirement: OpenCode SQLite row 统一解析
系统 SHALL 将 OpenCode SQLite 查询统一为 row batch 读取链路。Token extractor 和 Behavior extractor MUST 从同一批 message row 中分别产出 token logs 和完成事件。系统 MUST NOT 长期保留独立于 batch pipeline 的 OpenCode `query_db_batch` 特例。

#### Scenario: 同 row 产出 token 与行为
- **WHEN** OpenCode SQLite 查询返回 assistant message row 且 `finish = "stop"`
- **THEN** Token extractor 从该 row 产出 token logs，Behavior extractor 从同一 row 产出 `run_completed`

#### Scenario: 无 token 但有行为
- **WHEN** OpenCode row 不产生 token logs 但满足完成事件条件
- **THEN** watcher 仍推进统一 watermark，并允许 Behavior extractor 产出行为事件

#### Scenario: watermark 字段选择
- **WHEN** OpenCode message 表存在 `time_updated`
- **THEN** source 查询使用 `time_updated` 作为增量 watermark，否则回退到 `time_created`
