## ADDED Requirements

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
