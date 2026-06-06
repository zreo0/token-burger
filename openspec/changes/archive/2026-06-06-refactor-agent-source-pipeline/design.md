## Context

当前后端以 `AgentAdapter` 为中心：adapter 负责 Agent 标识、数据源发现、文件路径或 SQLite 位置、token 解析，以及部分 SQLite 查询。`WatcherEngine` 根据 `DataSource` 分派 notify/polling/sqlite 策略，并把 adapter 解析出的 `TokenLog` 写入数据库。

`add-agent-behavior-tips` 在此基础上新增了行为提示 fan-out，但为了避免重复数据源，它在 watcher 和 OpenCode 查询链路中加入了兼容式分支。这解决了第一版功能问题，但 `AgentAdapter` 仍然 token-centric，OpenCode 仍有 `query_db_batch` 特例。后续补更多 Agent 行为解析时，这种结构会继续扩大特例。

本 change 只做内部架构重构，目标是把数据源读取和解析投影拆开；用户可见功能必须保持不变。

## Goals / Non-Goals

**Goals:**
- 将 Agent 数据源/source、读取 batch、Token 解析、行为解析拆成清晰边界
- 让 watcher 成为唯一的数据源读取、offset/watermark 推进和冷启动编排入口
- 让 Token 解析和行为解析消费同一批 `AgentDataBatch`
- 移除 OpenCode `query_db_batch` 这类长期特例，统一 SQLite row fan-out
- 保持所有现有 Agent 的 `TokenLog` 输出、provider、model、session/request 字段语义不变
- 保持行为提示的开关、Agent 启用状态、冷启动不回放和多会话清理语义不变
- 保持数据库 schema、Tauri command/event payload、前端页面和设置项不变
- 提供足够测试覆盖，证明重构前后统计和行为提示等价

**Non-Goals:**
- 不新增新的 Agent
- 不新增新的行为事件类型
- 不改变 UI、价格计算、Settings 结构或数据库业务表
- 不持久化行为事件历史
- 不重写前端提示组件
- 不修复与本重构无关的旧 spec 偏差或死代码

## Decisions

### Decision: 引入 AgentSource 与解析器分离的内部 contract

将现有 adapter contract 拆为三类职责：

```text
AgentSource
    -> 描述 agent_name、data_source、监听路径、SQLite 位置和 source key

AgentDataBatch
    -> 承载 watcher 读取到的一批新增数据和水位线信息

TokenExtractor / BehaviorExtractor
    -> 从 AgentDataBatch 产出 TokenLog 或 AgentBehaviorEvent
```

`AgentSource` 不产出 `TokenLog`；解析器不执行 glob、notify 注册、文件读取或 SQLite 打开查询。解析器只消费 batch。Token extractor 可以返回不影响业务数据的解析状态元数据，例如 Codex 的最终 model，用于 watcher 更新 parser cache；但不得返回或更新 offset/watermark。

原因：
- 数据源读取是 I/O 生命周期问题，解析是投影问题，混在一个 trait 会让扩展行为解析时继续污染 token adapter
- watcher 可以统一维护 offset/watermark，避免 token 和 behavior 各自推进水位线
- 未来新增 Agent 行为解析时只需新增解析器，不扩大监听数量

备选方案是在 `AgentAdapter` 上继续增加 `parse_behavior` 或 `query_db_batch` 方法。该方案改动小，但会让 trait 继续变宽，并把所有 future fan-out 都塞进 token adapter，长期不可取。

### Decision: 使用 AgentDataBatch 表达三类输入

定义一个内部 batch 枚举或等价结构，至少覆盖：
- `JsonlIncrement`：文件路径、agent_name、新增内容切片、起止 offset
- `JsonDocument`：文件路径、agent_name、完整内容、mtime
- `SqliteRows`：db path、agent_name、row 列集合、previous watermark、next watermark

batch 必须携带足够上下文，让 Codex token 解析保留 model cache 逻辑，让 OpenCode token/behavior 解析共享同一批 message row。

原因：
- JSONL、JSON 和 SQLite 的读取形态不同，但 watcher 可以用统一 batch 分发
- batch 可以明确水位线更新只属于读取层，不属于解析器
- OpenCode 的 `time_updated` / `time_created` 选择可以在 source 查询阶段完成，解析器只处理 row

### Decision: 分阶段迁移，直接替换旧入口

实施顺序应避免一次性破坏所有 Agent，但如果迁移能够在同一批改动中完成并由测试证明等价，则不引入旧 adapter 兼容包装层：
1. 新增 batch 类型和解析器 trait
2. 调整 watcher 策略，让正常监听路径先生成 batch，再调用 token/behavior fan-out
3. 逐个迁移 Codex/OpenCode token 解析到独立 extractor，并用测试证明输出等价
4. 迁移 Claude Code、Gemini CLI 和其他现有 adapter
5. 删除旧 trait 方法，确保没有长期双入口

原因：
- 可以在每一步跑测试并比较输出
- OpenCode SQLite 风险最高，应在 row batch 和 watermark 测试稳定后再移除特例
- 对外行为保持不变，比一次性大改更安全
- 避免为了迁移过程保留不会被长期使用的兼容抽象

### Decision: 行为提示继续作为可选消费者

行为解析器只在两个条件同时满足时运行：
1. Agent 已启用并进入 watcher 正常监听
2. 行为提示总开关开启

冷启动 batch 不分发给行为解析器，或分发时必须显式标记 `cold_start = true` 并被行为 fan-out 跳过。关闭行为提示期间不缓存事件，不在重新开启后回放。

原因：
- 行为提示是短生命周期提醒，不是历史记录
- 用户关闭 Agent 或关闭行为提示时，不应产生额外解析成本
- 这保持当前行为提示 change 的产品边界

### Decision: 不改变写入、广播和前端接口

Token 写入仍通过现有 `WriteRequest` 和数据库写线程完成；`TokenLog` 结构、`token-updated` 事件、tray title、Popup 数据查询和 Settings 命令保持不变。行为提示仍通过现有 dispatcher 和 `behavior-tip` window 通信。

原因：
- 本 change 目标是内部数据流清晰化，不应制造前端迁移
- 保持接口不变能降低回归面，也让测试更容易定义等价标准

## Risks / Trade-offs

- Token 输出发生细微差异 → 为 Codex/OpenCode/Claude/Gemini 建立解析等价测试，比较 token_type、count、model、session、request、timestamp 和 cost 字段
- OpenCode watermark 推进错误 → 为 `time_updated` 优先、`time_created` fallback、无 token 但有行为事件、空批次等场景增加测试
- Codex model cache 被 batch 改造破坏 → 保留并测试 cache miss 全量恢复、后续增量使用缓存的路径
- 冷启动误弹行为提示 → 在 batch 或 fan-out 层加入冷启动门控测试
- 两套入口长期并存 → tasks 中明确移除或收敛旧 `query_db_batch`/`parse_content` 特例，不把兼容层作为最终状态
- 重构范围过大 → 分阶段提交，每阶段保持 `cargo test`、前端测试、lint/build 可运行

## Migration Plan

1. 添加新内部类型和 trait，不改变现有运行路径
2. 直接迁移各 Agent 到新 trait，验证 token 输出不变
3. 迁移 watcher 正常监听路径为 batch fan-out
4. 迁移冷启动路径为 batch token-only 处理
5. 迁移 OpenCode SQLite 为统一 row batch，移除长期 `query_db_batch` 特例
6. 移除不再使用的旧入口和孤儿代码
7. 运行 Rust/前端全量测试、lint、build，并手动验证基础统计和行为提示

## Resolved Choices

- `AgentSource`、`TokenExtractor` 和 `BehaviorExtractor` 使用同一 Agent adapter 结构体实现多个 trait
- SQLite row 使用强类型 `SqliteMessageRow` / `SqliteRowBatch`，不使用松散 JSON row 过渡
- 旧 `AgentAdapter` 名称和 `query_db_batch` 长期入口直接删除，不保留兼容层
