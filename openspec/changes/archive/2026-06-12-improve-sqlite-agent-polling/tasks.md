## 1. 本地数据库与写入语义

- [x] 1.1 新增 `external_sqlite_cursors` 本地表初始化，包含 `source_key`、`session_id`、`created_time`、`created_id`、`updated_time`
- [x] 1.2 新增 cursor 查询、批量 upsert、按 source 读取和按 source 判断是否已 bootstrap 的 DB helper
- [x] 1.3 将外部 SQLite cursor 写入路径改为基于 `(request_id, token_type)` 的幂等 upsert，允许重复记录更新 token_count、metadata、cost 和 timestamp，同时保留普通写入去重语义
- [x] 1.4 补充 DB 单元测试，覆盖 schema 幂等初始化、cursor CRUD、TokenLog 重复写入更新

## 2. SQLite source 查询接口

- [x] 2.1 为外部 SQLite adapter 增加列出 session id 的只读查询能力
- [x] 2.2 为 OpenCode/MiMoCode 增加按 session created cursor 查询 message 的能力，使用 `session_id + time_created + id`
- [x] 2.3 为 OpenCode/MiMoCode 增加按 session updated cursor 查询 message 的能力，用于更新校准
- [x] 2.4 保留旧全局 watermark 查询能力仅用于兼容或测试，不作为新版高频主路径
- [x] 2.5 补充 OpenCode/MiMoCode adapter 测试，验证 per-session created 查询使用正确排序并不漏同毫秒 id

## 3. Cursor bootstrap 与迁移

- [x] 3.1 实现从旧 `sqlite:<db_path>` global watermark bootstrap per-session cursor 的逻辑
- [x] 3.2 bootstrap 时只写 cursor，不解析历史 row，不写 TokenLog，不推进旧 global watermark
- [x] 3.3 无旧 global watermark 时执行 per-session 全量 catch-up，并在写入成功后建立 cursor
- [x] 3.4 补充迁移测试，覆盖已有旧 watermark 不重头补扫、无旧 watermark 首次接入会建立 cursor

## 4. Watcher SQLite 策略

- [x] 4.1 将正常轮询主路径替换为 per-session created cursor 增量读取
- [x] 4.2 增加 per-session updated reconciliation，支持 overlap 和每轮 session 数量限制
- [x] 4.3 确保 Token extractor 与 Behavior extractor 仍消费同一批 SQLite row batch
- [x] 4.4 确保 cursor 只在 TokenLog 写入和 cursor 状态更新成功后推进
- [x] 4.5 确保外部 SQLite DB 始终只读打开，不创建索引、不修改外部 schema

## 5. 验证与回归

- [x] 5.1 使用真实 OpenCode/MiMoCode schema 构造测试，验证 created 查询的 `EXPLAIN QUERY PLAN` 可使用 `message_session_time_created_id_idx`
- [x] 5.2 验证升级场景不会长期同时运行旧全局轮询和新版 per-session 轮询
- [x] 5.3 验证 overlap 重复读取不会产生重复统计，且旧 message 更新可修正既有 TokenLog
- [x] 5.4 运行 Rust 全量测试，确认现有 OpenCode/MiMoCode token 与行为解析不回归
