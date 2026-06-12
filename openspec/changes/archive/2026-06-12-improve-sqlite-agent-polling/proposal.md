## Why

OpenCode 与 MiMoCode 的外部 SQLite `message` 表当前都没有可用于全局 `time_updated > ? ORDER BY time_updated` 的索引。现有实现虽然只返回增量 row，但 SQLite 执行层会扫描 `message` 全表；OpenCode 数据库变大后，这不是长久方案。

需要在不修改外部 Agent 数据库、不重新全量补扫既有用户数据、不长期保留两套轮询逻辑的前提下，优化 SQLite Agent 的增量读取策略。

## What Changes

- 新增外部 SQLite source 的 per-session cursor 读取策略，优先利用现有 `message(session_id, time_created, id)` 索引查询新建 message
- 新增本地 SQLite cursor 存储，用于记录每个外部 SQLite source/session 的 `created_cursor` 与 `updated_cursor`
- 新增从旧版全局 watermark 到 per-session cursor 的一次性 bootstrap，升级后不从头重扫历史数据
- 将旧版全局 `time_updated` watermark 保留为迁移输入和回滚参考，不再作为高频轮询主路径
- 对旧 message 后续更新使用按 session 分批 reconciliation，允许小范围 overlap，并依赖 TokenLog upsert 保证重复读取安全
- 明确不修改 OpenCode/MiMoCode 外部数据库，不给外部 DB 创建索引，也不做运行中首次生成 DB 的自动接入

## Capabilities

### New Capabilities

- 无。此次变更优化现有 Watcher 与本地数据库 cursor 存储，不新增独立顶层能力。

### Modified Capabilities

- `watcher-engine`：SQLite 策略从高频全局 `time_updated` 查询改为 per-session cursor 增量 + 分批更新校准
- `sqlite-database`：新增外部 SQLite source cursor 持久化与重复 TokenLog 更新语义

## Impact

- 影响代码区域：`src-tauri/src/watcher/sqlite_strategy.rs`、`src-tauri/src/watcher/mod.rs`、`src-tauri/src/db/mod.rs`、`src-tauri/src/db/queries.rs`、OpenCode/MiMoCode SQLite adapter 查询接口
- 影响本地 DB schema：需要新增或扩展 cursor 持久化结构，保留已有 `file_offsets` 数据
- 兼容性：已运行用户使用旧 `sqlite:<db_path>` watermark bootstrap 新 cursor，不重新全量跑历史
- 性能：高频路径避免 `message` 全表扫描；低频/分批 reconciliation 允许受控扫描以保证最终不漏更新
