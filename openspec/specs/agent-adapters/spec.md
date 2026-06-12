# Purpose
定义各 Agent 数据源、Token 解析器与行为解析器的统一行为。

## Requirements

### Requirement: Agent Source Pipeline 实现
系统 SHALL 将 Agent 数据源描述与解析器分离。每个已支持 Agent MUST 提供稳定的 Agent source 描述，包括 `agent_name`、数据源类型、监听路径或 SQLite 位置，以及用于 offset/watermark 的 source key。Token 解析 MUST 由 Token extractor 完成；行为解析 MUST 由独立 Behavior extractor 完成。解析器 MUST NOT 自行执行 glob 扫描、notify 注册、文件读取、SQLite 打开查询或数据源发现。

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
- **THEN** 该 Agent 返回空事件集合，不得影响 token 解析

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
系统 SHALL 将 Token 解析逻辑放在 Token extractor 中，同时保持 `TokenLog` 结构和下游入库语义不变。Token extractor MUST 返回 token logs，并 MAY 返回解析状态元数据（如 Codex 最终 model）供 watcher 更新 parser cache；Token extractor 不得更新 offset/watermark，不得广播前端事件。

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

### Requirement: Claude Code Adapter
系统 SHALL 解析 `~/.claude/projects/**/*.jsonl` 中的 JSONL 日志。每行 JSON 中 `type == "assistant"` 的事件包含 `message.usage` 字段，从中提取 `input_tokens`、`cache_creation_input_tokens`、`cache_read_input_tokens`、`output_tokens`。`model` 字段提取模型 ID，`conversationId` 或 `sessionId` 作为 `session_id`，每行的 `uuid` 作为 `request_id`。provider 固定为 `"Anthropic"`。

#### Scenario: 解析 assistant 事件
- **WHEN** 读取到一行 `type: "assistant"` 的 JSONL
- **THEN** 提取 usage 中的四种 token 数，生成 input、cache_create、cache_read、output 对应的 TokenLog

#### Scenario: 跳过非 assistant 事件
- **WHEN** 读取到 `type: "human"` 或 `type: "tool_result"` 的行
- **THEN** 跳过该行，不生成 TokenLog

#### Scenario: 递归扫描项目子目录
- **WHEN** `~/.claude/projects/` 下有多层嵌套的项目目录
- **THEN** 系统递归扫描所有 `.jsonl` 文件

### Requirement: Codex Adapter
系统 SHALL 解析 `~/.codex/sessions/**/*.jsonl` 中的 JSONL 日志。`event_msg` 类型的 `token_count` 事件包含 token 计数信息，`turn_context` 事件 MAY 更新后续 token logs 使用的模型 ID。provider 固定为 `"OpenAI"`。

#### Scenario: 解析 token_count 事件
- **WHEN** 读取到包含 token 计数的 `event_msg` 行
- **THEN** 提取 input、cache_read、output token 数，生成对应的 TokenLog

#### Scenario: 维护模型上下文
- **WHEN** 读取到 `turn_context` 中的模型信息
- **THEN** 后续 token logs 使用该模型 ID，直到新的模型上下文出现

#### Scenario: 格式异常容错
- **WHEN** 某行 JSON 格式损坏
- **THEN** 跳过该行并记录 warn 日志，继续解析后续行

### Requirement: Gemini CLI Adapter
系统 SHALL 解析 `~/.gemini/tmp/*/chats/*.json` 中的 JSON 文件。每个文件是一个完整的聊天记录，`messages` 数组中的 assistant/model 消息包含 token 或 usage metadata。provider 固定为 `"Google"`。

#### Scenario: 全量解析聊天文件
- **WHEN** 检测到某个 JSON 文件的 mtime 发生变化
- **THEN** 全量解析该文件，提取 assistant/model 消息的 token 数据

#### Scenario: 处理 hash 目录
- **WHEN** `~/.gemini/tmp/` 下有多个 hash 命名的子目录
- **THEN** 系统扫描所有 hash 目录下的 `chats/*.json`

### Requirement: OpenCode Adapter（新版 SQLite）
系统 SHALL 查询 `~/.local/share/opencode/opencode.db` 中的 `message` 表。系统 MUST 将查询结果封装为 SQLite row batch，Token extractor 和 Behavior extractor MUST 从同一批 message row 中分别产出 token logs 和完成事件。系统 MUST NOT 长期保留独立于 batch pipeline 的 OpenCode `query_db_batch` 特例。

#### Scenario: 同 row 产出 token 与行为
- **WHEN** OpenCode SQLite 查询返回 assistant message row 且 `finish = "stop"`
- **THEN** Token extractor 从该 row 产出 token logs，Behavior extractor 从同一 row 产出 `run_completed`

#### Scenario: 无 token 但有行为
- **WHEN** OpenCode row 不产生 token logs 但满足完成事件条件
- **THEN** watcher 仍推进统一 watermark，并允许 Behavior extractor 产出行为事件

#### Scenario: watermark 字段选择
- **WHEN** OpenCode message 表存在 `time_updated`
- **THEN** source 查询使用 `time_updated` 作为增量 watermark，否则回退到 `time_created`

#### Scenario: 数据库不可读降级
- **WHEN** `opencode.db` 文件不存在
- **THEN** 系统自动切换到旧版 JSON fallback

### Requirement: OpenCode Adapter（旧版 JSON Fallback）
系统 SHALL 在新版 SQLite 不可用时，回退到解析 `~/.local/share/opencode/storage/message/**/*.json`。每个 JSON 文件包含 `tokens.input`、`tokens.output`、`tokens.cache.read`、`tokens.cache.write` 字段。

#### Scenario: Fallback 触发
- **WHEN** 新版 `opencode.db` 不存在
- **THEN** 系统自动切换到旧版 JSON 解析模式

#### Scenario: 解析旧版 JSON
- **WHEN** 检测到 `storage/message/` 下的 JSON 文件变更
- **THEN** 全量解析该文件，提取四种 token 数

### Requirement: MiMoCode Adapter（SQLite）
系统 SHALL 将 MiMoCode 作为独立 Agent 接入，agent_name MUST 为 `"mimocode"`。系统 MUST 查询 MiMoCode SQLite 数据库中的 `message` 表，并将查询结果封装为 SQLite row batch。MiMoCode adapter MUST 复用通用 Watcher SQLite polling、watermark 推进和 batch fan-out，不得复用 OpenCode adapter 的内部解析实现。

#### Scenario: 注册独立 Agent
- **WHEN** 系统初始化内置 Agent pipeline
- **THEN** `mimocode` 作为独立 Agent source 被注册，并可被 settings 中的 enabled agents 开关控制

#### Scenario: 数据库路径定位
- **WHEN** MiMoCode adapter 获取数据源
- **THEN** 系统按 `MIMOCODE_DB`、`MIMOCODE_HOME/data/mimocode.db`、`~/.local/share/mimocode/mimocode.db` 的顺序定位数据库

#### Scenario: SQLite batch 查询
- **WHEN** MiMoCode 数据库存在且 `message` 表有 watermark 之后的新 row
- **THEN** 系统查询 `id`、`session_id`、`data`、`time_created` 和 watermark 字段，并生成 `agent_name = "mimocode"` 的 SQLite row batch

#### Scenario: watermark 字段选择
- **WHEN** MiMoCode `message` 表存在 `time_updated`
- **THEN** source 查询使用 `time_updated` 作为增量 watermark，否则回退到 `time_created`

#### Scenario: 数据库不存在
- **WHEN** MiMoCode 数据库路径不存在
- **THEN** 系统不生成 MiMoCode token logs，不影响其他 Agent 的 watcher 初始化和统计

### Requirement: MiMoCode Token 解析
系统 SHALL 从 MiMoCode SQLite message row 中解析 token 数据。Token extractor MUST 只处理 `message.data.role = "assistant"` 的记录。系统 MUST 将 `tokens.input` 解析为 input，将 `tokens.cache.read` 解析为 cache_read，将 `tokens.cache.write` 解析为 cache_create，将 `tokens.output + tokens.reasoning` 解析为 output。系统 MUST 使用 `providerID` 作为 provider，使用 `modelID` 或 `model` 作为 model_id，使用 `message.session_id` 作为 session_id，使用 `message.id` 生成稳定 request_id。正数 `cost` MUST 只写入同一 message 的 input token log，避免成本重复求和。

#### Scenario: 解析 assistant token
- **WHEN** MiMoCode SQLite row 的 data JSON 包含 `role = "assistant"` 和 tokens 字段
- **THEN** 系统按 MiMoCode token 映射生成对应的 TokenLog，并设置 `agent_name = "mimocode"`

#### Scenario: 合并 reasoning 到 output
- **WHEN** MiMoCode assistant row 同时包含 `tokens.output` 和 `tokens.reasoning`
- **THEN** output TokenLog 的 token_count 等于两者之和，并在 metadata 中保留原始 reasoning 数值

#### Scenario: 跳过非 assistant row
- **WHEN** MiMoCode SQLite row 的 data JSON 中 `role` 不是 `"assistant"`
- **THEN** 系统跳过该 row，不生成 TokenLog

#### Scenario: 成本只计一次
- **WHEN** 同一 MiMoCode assistant row 生成 input、output、cache_read 或 cache_create 多条 TokenLog
- **THEN** 正数 cost 只出现在 input TokenLog 上，其他 TokenLog 的 cost 为空

#### Scenario: 格式异常容错
- **WHEN** MiMoCode row 的 data JSON 损坏或 tokens 字段缺失
- **THEN** 系统跳过异常记录并记录 warn 日志，继续处理剩余 row

### Requirement: 解析容错
所有 Token extractor 与 Behavior extractor 逐行/逐条解析，遇到格式异常的数据 MUST 跳过并记录 warn 日志，不得中断整体解析流程。

#### Scenario: 单行损坏不影响整体
- **WHEN** JSONL 文件中第 50 行格式损坏
- **THEN** 跳过第 50 行，继续解析第 51 行及后续内容，warn 日志记录损坏行号或解析错误

#### Scenario: 空文件处理
- **WHEN** 日志文件存在但内容为空
- **THEN** 返回空的 `Vec<TokenLog>` 和空行为事件集合，不报错
