## ADDED Requirements

### Requirement: 修复 idx_request_dedup 索引冲突
DESIGN.md 第 4.1 节的 `CREATE UNIQUE INDEX idx_request_dedup ON token_logs(request_id)` SHALL 修改为普通索引 `CREATE INDEX`，消除与表级 `UNIQUE(request_id, token_type)` 约束的冲突。

#### Scenario: 索引不再冲突
- **WHEN** 查看修改后的 DESIGN.md 第 4.1 节 Schema 定义
- **THEN** `idx_request_dedup` SHALL 为普通索引（非 UNIQUE），且 `UNIQUE(request_id, token_type)` 复合约束保持不变

### Requirement: 明确 Tauri v2
DESIGN.md 第 2 节 SHALL 明确指定使用 Tauri v2，并注明 v2 的关键 API 差异。

#### Scenario: 版本声明
- **WHEN** 查看修改后的 DESIGN.md 第 2 节
- **THEN** 应用框架描述 SHALL 明确为 "Tauri v2 (Rust + Web Frontend)"

### Requirement: Agent 日志格式定义
DESIGN.md SHALL 新增章节，详细定义每个支持的 Agent 的日志路径、文件格式和 Token 数据提取方式。

#### Scenario: Claude Code 日志格式
- **WHEN** 查看 Agent 日志格式章节
- **THEN** SHALL 定义：路径 `~/.claude/projects/**/*.jsonl`、格式 JSONL、Token 来源 `message.usage`（assistant 事件）、需递归扫描项目子目录

#### Scenario: Codex 日志格式
- **WHEN** 查看 Agent 日志格式章节
- **THEN** SHALL 定义：路径 `~/.codex/sessions/*.jsonl`、格式 JSONL、Token 来源 `event_msg`（token_count 类型）

#### Scenario: Gemini CLI 日志格式
- **WHEN** 查看 Agent 日志格式章节
- **THEN** SHALL 定义：路径 `~/.gemini/tmp/<hash>/chats/*.json`、格式 JSON、Token 来源 `messages[].tokens`

#### Scenario: Copilot 日志格式
- **WHEN** 查看 Agent 日志格式章节
- **THEN** SHALL 定义：路径 `~/.copilot/history-session-state/*.json`、格式 JSON、Token 来源 timeline 事件推断

#### Scenario: OpenCode 日志格式（新版 SQLite）
- **WHEN** 查看 Agent 日志格式章节
- **THEN** SHALL 定义：路径 `~/.local/share/opencode/opencode.db`、格式 SQLite、Token 来源 `message.data` JSON 中的 `tokens` 字段（仅 role=assistant）

#### Scenario: OpenCode 日志格式（旧版 JSON）
- **WHEN** 查看 Agent 日志格式章节
- **THEN** SHALL 定义：路径 `~/.local/share/opencode/storage/message/**/*.json`、格式 JSON、Token 来源 `tokens.input/output/cache.read/cache.write`、作为新版 SQLite 的 fallback

### Requirement: 多数据源 Adapter Trait 重设计
DESIGN.md 第 4.2 节的 AgentAdapter Trait SHALL 重新设计，支持 JSONL 增量读取、JSON 全量解析和 SQLite 直读三种数据源模式。

#### Scenario: DataSource 枚举
- **WHEN** 查看修改后的 Adapter Trait 定义
- **THEN** SHALL 包含 `DataSource` 枚举，含 `Jsonl { paths: Vec<PathBuf> }`、`Json { paths: Vec<PathBuf> }`、`Sqlite { db_path: PathBuf }` 三个变体

#### Scenario: Trait 方法签名
- **WHEN** 查看修改后的 AgentAdapter Trait
- **THEN** SHALL 包含 `agent_name()`、`data_source()`、`parse_content()`（文本解析）、`query_db()`（SQLite 查询）方法

#### Scenario: Watcher 策略自动选择
- **WHEN** Watcher 引擎初始化某个 Adapter
- **THEN** SHALL 根据 `data_source()` 返回值自动选择监听策略：JSONL 用 notify + debounce、JSON 用定时轮询 + mtime 检查、SQLite 用定时查询

### Requirement: 费率方案
DESIGN.md SHALL 新增章节定义费率数据的获取、缓存和计算方案。

#### Scenario: 远程价格表获取
- **WHEN** 应用启动时
- **THEN** Rust 端 SHALL 尝试从 LiteLLM GitHub 拉取 `model_prices_and_context_window.json`

#### Scenario: 本地缓存
- **WHEN** 远程价格表获取成功
- **THEN** SHALL 缓存到 `~/.token-burger/pricing/model_pricing_YYYY-MM-DD.json`，同一天内不重复拉取

#### Scenario: 离线 Fallback
- **WHEN** 远程价格表获取失败且无本地缓存
- **THEN** SHALL 使用项目内置的默认价格表

#### Scenario: 模型名匹配
- **WHEN** 需要查找某个 model_id 的价格
- **THEN** SHALL 按以下优先级匹配：精确匹配 → 归一化匹配（去版本号/日期后缀）→ 子串匹配 → 未知模型返回 $0

### Requirement: SQLite WAL 模式
DESIGN.md 第 4.1 节 SHALL 补充 WAL 模式的启用要求。

#### Scenario: WAL 初始化
- **WHEN** SQLite 数据库初始化时
- **THEN** SHALL 执行 `PRAGMA journal_mode=WAL;`，确保 Watcher 写入和前端查询可并发

### Requirement: 冷启动策略
DESIGN.md SHALL 新增章节定义首次运行时的历史数据处理方案。

#### Scenario: 后台解析
- **WHEN** 应用首次启动检测到历史日志
- **THEN** SHALL 在后台线程逐 Agent 解析，不阻塞 UI 渲染

#### Scenario: 时间范围限制
- **WHEN** 后台解析历史日志时
- **THEN** SHALL 默认只处理最近 30 天的文件（按 mtime 过滤）

#### Scenario: 进度展示
- **WHEN** 后台解析进行中
- **THEN** 前端 SHALL 显示"Burger 制作中"进度状态，每完成一个 Agent 更新进度

#### Scenario: 增量可用
- **WHEN** 某个 Agent 的历史数据解析完成
- **THEN** 该 Agent 的数据 SHALL 立即可在 UI 中展示，无需等待所有 Agent 完成

### Requirement: 错误处理策略
DESIGN.md SHALL 新增章节定义各类异常场景的处理方案。

#### Scenario: 日志文件损坏
- **WHEN** Adapter 解析遇到无法解析的行或格式异常
- **THEN** SHALL 跳过该行/文件，记录 warn 日志，继续处理后续内容

#### Scenario: SQLite 写入失败
- **WHEN** 事务入库失败（如数据库锁定）
- **THEN** SHALL 重试最多 3 次，仍失败则跳过本轮写入，下次触发时重新尝试

#### Scenario: 外部数据源不可用
- **WHEN** OpenCode 的 SQLite 数据库不可读或不存在
- **THEN** SHALL 降级为不监控该 Agent，设置页面标记为"不可用"

#### Scenario: 前端异常隔离
- **WHEN** 前端组件渲染出错
- **THEN** React Error Boundary SHALL 捕获异常并显示 fallback UI，不影响其他组件

### Requirement: 测试策略
DESIGN.md SHALL 新增章节定义前后端的测试框架、目录约定和覆盖范围。

#### Scenario: 测试策略内容
- **WHEN** 查看测试策略章节
- **THEN** SHALL 定义：前端 vitest + `__test__/` 目录、Rust `#[cfg(test)]` 内置测试、覆盖范围（Adapter 解析、数据库 CRUD、格式化函数、费用计算）
