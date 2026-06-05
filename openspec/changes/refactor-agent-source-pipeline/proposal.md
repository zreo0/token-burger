## Why

当前 `AgentAdapter` 同时承担 Agent 标识、数据源发现、读取入口和 token 解析职责；行为提示接入后，watcher 又需要把同一批新增数据交给行为解析器。继续在现有 trait 上堆特例会让 Token 统计、行为提示和后续 Agent 扩展互相牵连。

本 change 通过内部架构重构，将 Agent 数据源监听、Token 解析和行为解析拆成清晰的流水线，同时保持现有统计、设置、冷启动、Popup 和行为提示能力不回归。

## What Changes

- 将现有 `AgentAdapter` 的职责拆分为更清晰的 Agent 数据源/source 描述、增量读取 batch、Token 解析消费者和行为解析消费者。
- 定义统一的 `AgentDataBatch` 或等价内部结构，用于承载文件增量内容、JSON 全量内容或 SQLite row，并由 watcher 负责 offset/watermark 生命周期。
- 将现有 Codex/OpenCode token 解析从 adapter 读取职责中剥离为 Token 解析器，保持 `TokenLog` 输出、入库和汇总语义不变。
- 将行为解析作为与 Token 解析同级的可选消费者，继续复用 `enabled_agents` 和行为提示总开关，不新增独立行为轮询。
- 统一 OpenCode SQLite 查询链路，避免 `query_db_batch` 作为长期特例存在；同一批 row 同时供 Token 与行为解析消费。
- 保持冷启动行为不变：历史扫描只补 token，不产生行为提示；正常监听只处理新增事件。
- 保持前端、数据库 schema、价格计算、Settings、Popup 和 tray title 对外行为不变。
- 不引入新的运行时依赖，不引入新的用户配置项，不改变用户可见功能。

## Capabilities

### New Capabilities

### Modified Capabilities

- `agent-adapters`: 将 adapter contract 从 token 解析入口重构为 Agent source 与解析器分离的内部 contract，同时保持各 Agent 的 token 解析结果不变。
- `watcher-engine`: 将 watcher 调度调整为读取一次、生成 batch、fan-out 给 Token/Behavior 消费者，并保持冷启动、offset、水位线、事件广播和 Agent 开关行为不变。

## Impact

- Rust 后端：影响 `src-tauri/src/adapters/`、`src-tauri/src/watcher/`、`src-tauri/src/behavior/` 以及相关测试。
- 数据流：内部从 `adapter.parse_content/query_db -> TokenLog` 迁移为 `AgentSource -> AgentDataBatch -> TokenExtractor/BehaviorExtractor`。
- 兼容性：不迁移已有 SQLite 业务表，不改变 `TokenLog` 数据结构，不改变 Tauri command/event payload。
- 风险控制：需要覆盖现有 Agent token 输出快照、OpenCode SQLite 增量水位线、Codex model cache、冷启动无行为提示、行为开关关闭不解析、Agent 关闭不监听。
