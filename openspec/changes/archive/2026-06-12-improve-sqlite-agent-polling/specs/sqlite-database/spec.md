## MODIFIED Requirements

### Requirement: Schema 初始化
系统启动时 SHALL 自动创建 SQLite 数据库并初始化 Schema。启用 WAL 模式（`PRAGMA journal_mode=WAL`），创建 `token_logs` 表（含 `UNIQUE(request_id, token_type)` 复合唯一约束）、`file_offsets` 表、`app_settings` 表和外部 SQLite source cursor 表。索引 `idx_request_dedup` MUST 为普通索引（非唯一索引），避免与复合唯一约束冲突。Schema 初始化 MUST 使用幂等 DDL，不得破坏已有用户数据。

#### Scenario: 首次启动创建数据库
- **WHEN** 应用首次启动且数据库文件不存在
- **THEN** 系统创建数据库文件，执行 `PRAGMA journal_mode=WAL`，创建 `token_logs`、`file_offsets`、`app_settings`、外部 SQLite source cursor 表及所有索引

#### Scenario: 重复启动不破坏已有数据
- **WHEN** 应用启动且数据库已存在
- **THEN** 系统使用 `CREATE TABLE IF NOT EXISTS` 和 `CREATE INDEX IF NOT EXISTS`，不影响已有数据

### Requirement: Token 日志批量写入
系统 SHALL 支持批量写入 `Vec<TokenLog>`，使用 `rusqlite` 事务和 `(request_id, token_type)` 复合唯一约束确保原子性与幂等性。普通批量写入 MUST 保持既有重复记录不覆盖语义，避免文件类 Agent 在冷启动重扫或文件遍历顺序变化时产生不确定更新；外部 SQLite cursor 写入路径 MUST 在同一事务内写入 TokenLog 与 cursor，并允许重复 `(request_id, token_type)` 更新 token_count、provider、model_id、session_id、latency_ms、is_error、metadata、cost 和 timestamp 等可变字段。冷启动时 MUST 按 1000 条一批分割事务。

#### Scenario: 批量插入去重
- **WHEN** 普通写入路径收到包含重复 `(request_id, token_type)` 的数据
- **THEN** 重复记录保持既有记录，不覆盖为后到记录，不产生重复统计

#### Scenario: SQLite 重复记录更新
- **WHEN** 外部 SQLite reconciliation 通过 cursor 写入路径收到已有 `(request_id, token_type)` 但 token_count、metadata、cost 或 timestamp 已变化的数据
- **THEN** 系统更新既有 TokenLog，使后续汇总使用最新值

#### Scenario: 冷启动分批写入
- **WHEN** 冷启动解析出 5000 条记录
- **THEN** 系统分为 5 个事务（每个 1000 条）依次提交

## ADDED Requirements

### Requirement: 外部 SQLite source cursor 持久化
系统 SHALL 在本地 SQLite 数据库中持久化外部 SQLite source 的 per-session cursor。每条 cursor MUST 至少包含 source key、session_id、created_time、created_id 和 updated_time。cursor 更新 MUST 与对应 TokenLog 写入保持顺序一致：只有当本批 TokenLog 写入和 offset/cursor 状态更新成功后，系统才允许推进该 session 的 cursor。

#### Scenario: 保存 session cursor
- **WHEN** SQLite source 成功处理某 session 的新增 message row
- **THEN** 系统保存该 session 最新的 `created_time`、`created_id` 和 `updated_time`

#### Scenario: 读取 session cursor
- **WHEN** SQLite source 启动正常轮询
- **THEN** 系统从本地数据库读取该 source 的所有 session cursor，并用它们构造 per-session 增量查询

#### Scenario: cursor 与写入一致
- **WHEN** TokenLog 批量写入失败
- **THEN** 系统不得推进对应 session cursor，下一轮允许重试同一批外部 row

#### Scenario: 保留旧 global watermark
- **WHEN** 新版 cursor 表建立完成
- **THEN** 系统保留 `file_offsets` 中旧 `sqlite:<db_path>` key，不删除、不覆盖为新 cursor 语义
