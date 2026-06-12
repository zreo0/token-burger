## MODIFIED Requirements

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

## ADDED Requirements

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
