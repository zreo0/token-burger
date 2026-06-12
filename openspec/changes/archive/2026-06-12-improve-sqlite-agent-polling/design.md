## Context

OpenCode 与 MiMoCode 都使用外部 SQLite `message` 表作为 token 数据源。真实库结构显示：

- OpenCode `message` 约 36091 行，DB 约 851 MB
- MiMoCode `message` 当前约 54 行
- 两者都有 `message(session_id, time_created, id)` 索引
- 两者都没有全局 `time_updated` 索引
- 当前全局查询 `WHERE time_updated > ? ORDER BY time_updated` 的 query plan 是 `SCAN message` 与临时排序

当前实现逻辑上是增量查询，但执行层仍可能每轮扫描 `message` 全表。用户明确不希望修改外部 Agent 数据库，也接受运行中首次生成 DB 需要重启 App 的边界。

## Goals / Non-Goals

**Goals:**

- 高频轮询避免扫描外部 `message` 全表
- 利用现有 `message(session_id, time_created, id)` 索引读取新建 message
- 通过分批 reconciliation 覆盖旧 message 的后续更新
- 已有用户从旧全局 watermark 平滑迁移，不重新从头补扫
- 升级后只运行新版 per-session 策略，不长期保留旧全局轮询主路径
- 不修改 OpenCode/MiMoCode 外部数据库

**Non-Goals:**

- 不给外部数据库创建索引
- 不依赖 `session.time_updated` 作为正确性依据
- 不实现运行中首次生成 DB 的自动接入
- 不改变前端 Tauri command、事件名或 TokenSummary payload
- 不新增独立 Agent adapter

## Decisions

### 1. 使用 per-session created cursor 作为高频主路径

系统为每个外部 SQLite source 的每个 session 维护：

```text
created_cursor = (last_time_created, last_id)
```

高频查询使用：

```sql
SELECT id, session_id, data, time_created, time_updated
FROM message
WHERE session_id = ?
  AND (
    time_created > ?
    OR (time_created = ? AND id > ?)
  )
ORDER BY time_created ASC, id ASC
LIMIT ?
```

原因：该查询能使用现有 `message(session_id, time_created, id)` 索引。`id` 是 TEXT，不作为全局游标，只作为同一 `time_created` 下的稳定 tie-breaker。

替代方案：继续使用全局 `time_updated`。不采用，因为真实 query plan 会全表扫描。使用全局 `id > ?` 也不采用，因为真实数据按 id 排序与按时间排序不一致。

### 2. 使用 per-session updated cursor 做分批校准

旧 message 可能在创建后更新 tokens、finish 或 cost。仅靠 `time_created` cursor 不能覆盖这类更新，所以系统为每个 session 维护：

```text
updated_cursor = last_time_updated
```

校准查询使用：

```sql
SELECT id, session_id, data, time_created, time_updated
FROM message
WHERE session_id = ?
  AND time_updated >= ?
```

该查询不能完整利用 `time_updated` 索引，因为外部 DB 没有这个索引；但它能利用 `session_id` 前缀，将扫描范围限制在单个 session。校准 MUST 分批执行，每轮只处理有限数量 session，并使用 overlap 回退一小段时间。

替代方案：用 `session.time_updated` 先筛活跃 session。真实数据已经证明 `session.time_updated` 不可靠覆盖 message 更新时间，因此只能作为提示信号，不能作为正确性依据。

### 3. 旧 global watermark 只用于一次性 bootstrap

已有用户本地 `file_offsets` 中已经有旧 key：

```text
sqlite:<db_path> -> old_global_time_updated
```

新版启动时若某 source 没有 per-session cursor，但存在旧 global watermark，则执行 bootstrap：

1. 扫描外部 `session` id 列表
2. 对每个 session 查询旧 watermark 之前最后一条已处理 message：
   ```sql
   SELECT id, time_created
   FROM message
   WHERE session_id = ?
     AND time_updated <= ?
   ORDER BY time_created DESC, id DESC
   LIMIT 1
   ```
3. 写入该 session 的 `created_cursor`
4. 将 `updated_cursor` 初始化为 `old_global_time_updated - overlap_ms`

bootstrap 不解析历史 row，不写 TokenLog，不推进旧 global watermark。完成后运行阶段只使用 per-session 策略。

没有旧 global watermark 的首次接入场景，使用 per-session 全量 catch-up：按 session 分批读取所有历史 message，成功写入后推进 per-session cursor。

### 4. Cursor 存储使用本地单表

新增本地表：

```sql
CREATE TABLE IF NOT EXISTS external_sqlite_cursors (
  source_key TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_time INTEGER NOT NULL DEFAULT 0,
  created_id TEXT NOT NULL DEFAULT '',
  updated_time INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(source_key, session_id)
);
```

原因：现有 `file_offsets` 只能存单个 `u64`，无法表达 `(time_created, id)` 复合 cursor。使用单表足够表达 per-session 状态；旧 `file_offsets` 保留不删，用于迁移和回滚。

替代方案：把 cursor 编码进 `app_settings` 或多个 `file_offsets` key。暂不采用，因为动态 session cursor 不属于用户设置，且字符串 id 不适合塞进 u64 offset。

### 5. TokenLog 写入改为幂等 upsert

更新校准和 overlap 会重复读到同一 message。系统 MUST 以 `(request_id, token_type)` 做 upsert，更新 token_count、metadata、cost、timestamp 等可变字段。

原因：旧 message 后续更新时，重复读到同一 request_id 必须能修正已入库 token。只 `INSERT OR IGNORE` 会让 reconciliation 失去意义。

替代方案：避免 overlap 或只读取未见 request_id。暂不采用，因为无法覆盖旧 message 更新。

### 6. 控制批量与延迟

per-session 查询 MUST 支持 `LIMIT`。每轮轮询可以处理所有 session 的 created cursor，也可以处理固定数量 session；未处理 session 的 cursor 不推进，后续轮次继续处理，因此不会漏，只会延迟。

updated reconciliation MUST 分批 sweep session，避免单轮处理过多旧消息。实现可先使用简单 round-robin in-memory 位置；持久化 sweep position 不是第一版必须项。

## Risks / Trade-offs

- [Risk] `updated_cursor` 分批校准导致旧 message 更新不是立即可见 → Mitigation：高频 created path 负责新消息，updated path 保证最终一致
- [Risk] overlap 重复读取导致重复写入 → Mitigation：TokenLog 使用 `(request_id, token_type)` upsert
- [Risk] bootstrap 查询大量 session 时有一次性开销 → Mitigation：不解析历史，不写 TokenLog，且只在旧 source 首次升级时执行
- [Risk] 不使用 `session.time_updated` 会多扫一些 session → Mitigation：优先正确性；session 表远小于 message 表，message 查询走 session 前缀索引
- [Risk] 旧 app 回滚后旧 global watermark 可能落后 → Mitigation：保留旧 key，旧 app 最多重复读取新数据，依赖 upsert/去重保持统计稳定

## Migration Plan

1. 初始化本地 `external_sqlite_cursors` 表
2. 启动 SQLite source 时检查该 source 是否已有 cursor
3. 若无 cursor 且有旧 global watermark，执行 bootstrap，不解析历史 row
4. 若无 cursor 且无旧 global watermark，执行 per-session 全量 catch-up
5. 正常运行只使用 per-session created 增量和 updated reconciliation
6. 保留旧 `file_offsets` 中的 `sqlite:<db_path>` key，不再作为高频主路径推进

回滚策略：旧 `file_offsets` key 保留，旧版本仍可按旧逻辑运行；新表不会影响旧版本。

## Open Questions

- 第一版 updated reconciliation 的 overlap 窗口取值需要通过测试数据确认，建议从 2 分钟开始
- 每轮 created path 是否处理所有 session，还是按 batch 分轮处理，需要结合 OpenCode 真实 DB 测试耗时决定
