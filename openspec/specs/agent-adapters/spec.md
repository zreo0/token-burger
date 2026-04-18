# Purpose
定义各 Agent 日志适配器的统一行为与解析要求。

## Requirements

### Requirement: AgentAdapter Trait 实现
每个 Agent 的日志解析器 SHALL 实现已定义的 `AgentAdapter` Trait（`agent_name()`、`data_source()`、`parse_content()`、`query_db()`）。不适用的方法（如 JSONL Adapter 的 `query_db()`）MUST 返回空结果或 `Err`。

#### Scenario: Adapter 注册
- **WHEN** 系统初始化
- **THEN** 所有 5 个 Adapter 实例被创建并注册到 Watcher 引擎（OpenCode 合并新旧模式为单个 Adapter）

#### Scenario: 不适用方法的默认行为
- **WHEN** 对 JSONL 类型的 Adapter 调用 `query_db()`
- **THEN** 返回 `Err`，不 panic

### Requirement: Claude Code Adapter
系统 SHALL 解析 `~/.claude/projects/**/*.jsonl` 中的 JSONL 日志。每行 JSON 中 `type == "assistant"` 的事件包含 `message.usage` 字段，从中提取 `input_tokens`、`cache_creation_input_tokens`、`cache_read_input_tokens`、`output_tokens`。`model` 字段提取模型 ID，`conversationId` 作为 `session_id`，每行的 `uuid` 作为 `request_id`。provider 固定为 `"Anthropic"`。

#### Scenario: 解析 assistant 事件
- **WHEN** 读取到一行 `type: "assistant"` 的 JSONL
- **THEN** 提取 usage 中的四种 token 数，生成 4 条 TokenLog（input/cache_create/cache_read/output），共享同一个 request_id

#### Scenario: 跳过非 assistant 事件
- **WHEN** 读取到 `type: "human"` 或 `type: "tool_result"` 的行
- **THEN** 跳过该行，不生成 TokenLog

#### Scenario: 递归扫描项目子目录
- **WHEN** `~/.claude/projects/` 下有多层嵌套的项目目录
- **THEN** 系统递归扫描所有 `.jsonl` 文件

### Requirement: Codex Adapter
系统 SHALL 解析 `~/.codex/sessions/*.jsonl` 中的 JSONL 日志。`event_msg` 类型的事件中包含 token 计数信息。provider 固定为 `"OpenAI"`。

#### Scenario: 解析 token_count 事件
- **WHEN** 读取到包含 token 计数的 event_msg 行
- **THEN** 提取 input/output token 数，生成对应的 TokenLog

#### Scenario: 格式异常容错
- **WHEN** 某行 JSON 格式损坏
- **THEN** 跳过该行并记录 warn 日志，继续解析后续行

### Requirement: Gemini CLI Adapter
系统 SHALL 解析 `~/.gemini/tmp/<hash>/chats/*.json` 中的 JSON 文件。每个文件是一个完整的聊天记录，`messages` 数组中的每条消息包含 `tokens` 字段。provider 固定为 `"Google"`。

#### Scenario: 全量解析聊天文件
- **WHEN** 检测到某个 JSON 文件的 mtime 发生变化
- **THEN** 全量解析该文件，提取所有 assistant 消息的 token 数据

#### Scenario: 处理 hash 目录
- **WHEN** `~/.gemini/tmp/` 下有多个 hash 命名的子目录
- **THEN** 系统扫描所有 hash 目录下的 `chats/*.json`

### Requirement: Copilot Adapter
系统 SHALL 解析 `~/.copilot/history-session-state/*.json` 中的 JSON 文件。通过 timeline 事件推断 token 消耗。provider 固定为 `"GitHub"`。

#### Scenario: 从 timeline 推断 token
- **WHEN** 检测到 session state 文件变更
- **THEN** 解析 timeline 事件，推断 input/output token 数

### Requirement: OpenCode Adapter（新版 SQLite）
系统 SHALL 查询 `~/.local/share/opencode/opencode.db` 中的 `message` 表。仅处理 `role = 'assistant'` 的记录，从 `data` JSON 字段中提取 `tokens` 对象（含 `input`、`output`、`cache.read`、`cache.write`）。

#### Scenario: 增量查询新记录
- **WHEN** 定时查询触发
- **THEN** 系统查询 `created_at > since` 的 assistant 消息，提取 token 数据

#### Scenario: 数据库不可读降级
- **WHEN** `opencode.db` 文件不存在或被锁定
- **THEN** 系统标记该 Agent 为"不可用"，不中断其他 Agent 的监控

### Requirement: OpenCode Adapter（旧版 JSON Fallback）
系统 SHALL 在新版 SQLite 不可用时，回退到解析 `~/.local/share/opencode/storage/message/**/*.json`。每个 JSON 文件包含 `tokens.input`、`tokens.output`、`tokens.cache.read`、`tokens.cache.write` 字段。

#### Scenario: Fallback 触发
- **WHEN** 新版 `opencode.db` 不存在
- **THEN** 系统自动切换到旧版 JSON 解析模式

#### Scenario: 解析旧版 JSON
- **WHEN** 检测到 `storage/message/` 下的 JSON 文件变更
- **THEN** 全量解析该文件，提取四种 token 数

### Requirement: 解析容错
所有 Adapter 的 `parse_content()` SHALL 逐行/逐条解析，遇到格式异常的数据 MUST 跳过并记录 warn 日志，不得中断整体解析流程。

#### Scenario: 单行损坏不影响整体
- **WHEN** JSONL 文件中第 50 行格式损坏
- **THEN** 跳过第 50 行，继续解析第 51 行及后续内容，warn 日志记录损坏行号和文件路径

#### Scenario: 空文件处理
- **WHEN** 日志文件存在但内容为空
- **THEN** 返回空的 `Vec<TokenLog>`，不报错
