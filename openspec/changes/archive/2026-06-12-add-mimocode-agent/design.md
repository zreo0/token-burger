## Context

MiMoCode 是基于 OpenCode 改造的 Code Agent，但它已经形成独立运行面：

- CLI 名称是 `mimo`
- 全局配置使用 `mimocode.json(c)`，而不是 `opencode.json`
- 默认数据目录是 `~/.local/share/mimocode`
- `MIMOCODE_HOME` 会整体迁移 `config/data/state/cache`
- `MIMOCODE_DB` 可直接覆盖数据库文件路径
- 本地实测 `mimocode.db` 的 `message` 表包含 `id`、`session_id`、`agent_id`、`time_created`、`time_updated`、`data`

TokenBurger 当前已经有统一的 Agent pipeline：Agent source 只描述数据源，Watcher 负责读取、SQLite polling 和 watermark 推进，Token extractor 与 Behavior extractor 从同一批 batch 消费数据。OpenCode 已经接入该 pipeline，但 MiMoCode 不应复用 OpenCode adapter 内部实现，因为两者后续 schema、配置来源和事件语义可能分叉。

## Goals / Non-Goals

**Goals:**

- 以独立 `mimocode` Agent 接入 MiMoCode
- 复用现有 Watcher、SQLite polling、watermark、TokenLog 入库、行为提示 dispatcher
- 支持未登录状态下解析本地 `mimocode.db` 已产生的 token 数据
- 支持 `MIMOCODE_DB` 与 `MIMOCODE_HOME` 定位非默认数据库
- 在现有四类 TokenType 下正确统计 input、cache read、cache create、output，并处理 MiMoCode reasoning token
- 增加 MiMoCode 完成提醒与前端展示资源

**Non-Goals:**

- 不把 MiMoCode 作为 OpenCode adapter 的配置分支
- 不新增账号额度监控 provider
- 不接入 MiMoCode 权限请求提醒
- 不改 TokenLog 表结构或新增 reasoning TokenType
- 不依赖 `mimo stats` 作为统计入口

## Decisions

### 1. 独立实现 `MiMoCodeAdapter`

MiMoCode SHALL 使用独立 `src-tauri/src/adapters/mimocode.rs`，并注册为 `agent_name = "mimocode"`。

原因：MiMoCode 与 OpenCode 的表结构目前相似，但文档和本地证据都说明 MiMoCode 已经独立管理配置、数据目录和环境变量。直接复用 OpenCode adapter 会把 OpenCode 的 fallback、provider 默认值、agent name、日志文案和路径假设带入 MiMoCode。

替代方案：抽出一个 shared SQLite message parser。暂不采用，因为当前可复用代码量主要是 SQL 查询和字段读取，抽象收益不高，反而会让两边后续差异更难拆开。

### 2. 只复用通用 pipeline 和 SQLite strategy

MiMoCode SHALL 复用现有 `DataSource::Sqlite`、`AgentDataBatch::SqliteRows`、SQLite polling、offset/watermark 入库和行为 fan-out。

原因：这些是 TokenBurger 的 Agent 数据读取基础设施，不包含 OpenCode 专属语义。复用它们可以保持冷启动、增量轮询、行为提示开关和错误隔离的一致性。

替代方案：为 MiMoCode 新建独立 DB 轮询。暂不采用，因为会违反 Watcher 单一读取入口，也会重复处理 offset、冷启动和行为开关。

### 3. 数据库定位顺序

MiMoCode 数据库路径 SHALL 按以下顺序解析：

1. `MIMOCODE_DB`
2. `MIMOCODE_HOME/data/mimocode.db`
3. `~/.local/share/mimocode/mimocode.db`

原因：MiMoCode 文档明确 `MIMOCODE_DB` 可覆盖数据库路径，`MIMOCODE_HOME` 会迁移 data 目录。默认路径覆盖普通用户已运行过 `mimo` 的场景。

替代方案：调用 `mimo db path`。暂不采用，因为 CLI 可能触发额外权限、依赖当前工作目录，且本地已观察到某些 `mimo stats` 命令会受 git 或 SQLite checkpoint 影响。

### 4. Token 映射策略

MiMoCode token extractor SHALL 只处理 `message.data.role = "assistant"` 的 row。映射规则：

- `tokens.input` -> `TokenType::Input`
- `tokens.cache.read` -> `TokenType::CacheRead`
- `tokens.cache.write` -> `TokenType::CacheCreate`
- `tokens.output + tokens.reasoning` -> `TokenType::Output`
- `providerID` -> provider
- `modelID` 或 `model` -> model_id
- `message.session_id` -> session_id
- `message.id + token suffix` -> request_id
- 正数 `cost` 只挂到 input log，避免下游重复求和

原因：当前 TokenLog 只有四类 token，没有独立 reasoning 类型。MiMoCode 的 reasoning 是真实消耗，不计入会低估总 token；合并到 output 可以在不迁移 schema 的前提下保持总量更接近 MiMoCode 本地统计。原始 reasoning 值可写入 metadata，为后续 schema 扩展保留依据。

替代方案：忽略 reasoning。暂不采用，因为会漏算。新增 `TokenType::Reasoning` 暂不采用，因为会牵动 DB、汇总、前端图表和历史数据语义。

### 5. 行为解析只支持完成事件

MiMoCode behavior extractor SHALL 只从 SQLite row 中解析 `role = "assistant"` 且 `finish = "stop"` 的 `run_completed`。第一版 MUST NOT 生成权限请求事件。

原因：本地源码与 DB 观察显示权限请求是运行时 BusEvent，不稳定落在 `mimocode.db`。在没有可靠持久事件源前，做权限提醒会产生漏报或误报。

替代方案：扫描 message data 或 event 表寻找权限状态。暂不采用，因为当前样本 event 表为空，且 message row 不能稳定表达“正在等待用户处理”的权限状态。

### 6. 图标资源选择

前端 agent icon SHALL 使用已有 `src/assets/provider-icons/xiaomimimo.svg`。菜单栏图标 SHALL 由同一 SVG 生成 `src-tauri/icons/provider-menubar/xiaomimimo.pdf`，并接入 provider menubar icon 映射。

原因：SVG 是 `currentColor` 单色矢量，更适合 macOS 菜单栏模板图标。PNG 可作为参考或备用，但不作为第一选择。

替代方案：直接使用 `xiaomimimo.png`。暂不采用，因为菜单栏需要在深浅色模式下表现稳定，单色矢量转 PDF 更合适。

## Risks / Trade-offs

- [Risk] MiMoCode 后续修改 SQLite schema → Mitigation：adapter 独立实现，并为 schema 字段缺失提供 warn 和空结果，不影响其他 Agent
- [Risk] `tokens.reasoning` 合并到 output 后与“可见输出 token”概念不完全一致 → Mitigation：metadata 保留原始 reasoning，后续可迁移为独立 token type
- [Risk] 未登录状态 cost 可能为 0 或缺失 → Mitigation：cost 缺失时保持 None 或 0 的现有语义，不影响 token 统计
- [Risk] `MIMOCODE_HOME` 与 XDG 自定义路径组合更复杂 → Mitigation：第一版只支持文档明确的 `MIMOCODE_DB`、`MIMOCODE_HOME` 和默认路径
- [Risk] 权限提醒缺失 → Mitigation：spec 明确第一版不支持，后续需要 MiMoCode 持久化权限事件或提供插件事件输出后再扩展

## Migration Plan

1. 新增 MiMoCode adapter、behavior parser、图标和默认 enabled agent
2. 对本地样本 `mimocode.db` 建立 parser 单元测试与 SQLite 查询测试
3. 生成菜单栏 PDF 资源并接入 tray icon 映射
4. 运行 Rust 与前端相关测试，确认现有 OpenCode/Codex 行为不变

回滚策略：从 agent 注册列表、默认 enabled agents、前端 label/icon 和 tray icon 映射中移除 `mimocode`，保留不被引用的测试资源不会影响其他 Agent。

## Open Questions

- MiMoCode 是否会在未来稳定持久化权限请求事件；当前不能证明，因此不纳入第一版
- 是否需要在下一次 schema 变更中新增 `reasoning` TokenType；当前变更先用 metadata 预留
