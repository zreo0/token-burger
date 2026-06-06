## Context

当前数据流以 token 统计为中心：Watcher 读取 Agent 日志，Adapter 提取 `TokenLog`，写线程入库后广播 `token-updated` 并更新托盘标题。Codex 与 OpenCode 的真实本地记录还包含运行行为信号，但现有 Adapter 会丢弃非 token 行。

本 change 的实现范围是第一版行为提示接入：在现有 watcher/adapter 读取链路上复用新增数据给行为解析器，避免新增第二套文件扫描或 SQLite 轮询。它不完成 `AgentAdapter` 到 `AgentSource` / `TokenExtractor` 的完整 trait 重构；该重构应作为后续独立 change 处理。

已验证的第一版信号边界：
- Codex JSONL 的顶层 `timestamp` 可可靠解析，`task_started`、`task_complete`、`turn_aborted` 具备结构化 `turn_id`
- Codex 权限请求可从 `exec_command` 的 `sandbox_permissions: "require_escalated"` 和 `call_id` 识别，同一 `call_id` 的 `function_call_output` 可作为已处理信号
- OpenCode SQLite 的 `time_created` / `time_updated` 是毫秒 Unix epoch；`message.data.role = "assistant"` 且 `finish = "stop"` 可作为任务完成信号
- OpenCode 当前未验证到稳定的权限等待记录，第一版不做 OpenCode 权限提示

提示层应贴合现有 Popup 的深色、圆润、轻拟物气质，不使用系统通知，不引入活动历史视图，也不作为打开 Agent 或处理审批的入口。

## Goals / Non-Goals

**Goals:**
- 为 Codex 权限请求、Codex 任务终止、OpenCode 任务完成提供轻量 UI 提醒
- 在多个会话同时触发事件时保持单窗口展示和最多 10 条内存队列
- 同一会话进入新轮次时自动清理旧提醒；权限处理、任务完成或任务中断也会清理仍存在的相关提醒
- macOS 将提示定位在菜单栏图标附近，Windows 将提示定位在任务栏托盘区域附近
- 保持现有 token 统计、Popup、Settings 和账号用量菜单栏展示独立
- 在现有 watcher/adapter 读取链路上实现同源 fan-out，避免重复扫描或重复轮询
- 行为提示复用现有 Agent 启用状态；未启用的 Agent 不监听、不解析、不提示
- 提供行为提示总开关，关闭期间只处理新 token 数据，不运行行为解析 fan-out 或独立行为轮询

**Non-Goals:**
- 不使用系统通知插件
- 不新增活动历史、事件中心或完整时间线视图
- 不持久化行为事件历史
- 不展示 prompt、工具输出全文、完整命令参数或敏感内容
- 不实现 OpenCode 权限审批提示
- 不接入 Claude Code、Gemini CLI 或其他 Agent 的行为解析
- 不提供 `Open`、跳转、审批处理或其他干预 Agent 运行的操作
- 不新增独立的行为 Agent 开关；行为提示只依赖现有 Agent 开关和行为提示总开关
- 不在本 change 中完成 `AgentAdapter` 到 `AgentSource` / `TokenExtractor` 的完整重构、trait 拆分或大面积改名

## Decisions

### Decision: 新增独立行为解析模块，而不是扩展 `TokenLog`

新增 `behavior/` 模块，定义统一的 `AgentBehaviorEvent` 与各 Agent 独立解析器。该模块只拥有事件模型、解析函数和 dispatcher，不拥有文件监听、SQLite 轮询或数据源发现能力。Watcher 在读取增量内容或轮询外部 SQLite 时，将同一批新增数据分发给 token parser 与行为 parser，同时产出 token logs 与行为事件。

原因：
- `TokenLog` 是统计数据，行为事件是短生命周期 UI 信号，混在同一结构会污染数据库与汇总逻辑
- 每个 Agent 的行为记录形态差异大，独立模块便于后续补 OpenCode 权限或新增其他 Agent
- 数据源读取权仍归现有 watcher；行为模块只消费 watcher 已经拿到的增量数据，避免重复扫描或重复轮询

备选方案：在 `AgentAdapter` 上增加更多方法。该方案容易让 token 解析 trait 变宽，并把 OpenCode 的多表行为轮询挤进 token 查询接口，暂不采用。

### Decision: 第一版采用兼容式 fan-out，完整 AgentSource 重构延期

现有 `AgentAdapter` 仍承担 Agent 标识、数据源发现和 token 解析职责。第一版不大面积拆分 trait，而是在 watcher 已经读取到新增文件内容或 SQLite row 后，将同一批数据额外交给行为解析器。Codex 复用 JSONL 增量内容切片；OpenCode 通过同一次 SQLite 增量查询同时产出 token logs 与行为事件。

第一版边界：
- 保留 `AgentAdapter::parse_content` 和 `AgentAdapter::query_db` 作为现有 token 解析入口
- 行为解析模块只消费 watcher 或 OpenCode 查询链路已经拿到的内容/row
- OpenCode 的 `query_db_batch` 是第一版兼容入口，用于避免独立行为轮询，不作为最终通用抽象
- 不新增行为专用 glob 扫描、notify 注册或 SQLite 轮询

后续更干净的目标架构应由独立 change 处理：

```
AgentSource
        │
        ▼
AgentDataBatch
        │
        ├── TokenExtractor → TokenLog → DB/summary
        └── BehaviorExtractor → AgentBehaviorEvent → dispatcher/tip
```

原因：
- 行为提示是用户可见功能，第一版应优先避免重复数据源和行为回放，而不是混入大规模 trait 重构
- 完整拆分会影响所有 Agent adapter，风险和验证成本明显高于本功能本身
- OpenCode 如果需要基于 `time_updated` 识别完成事件，应升级同一条 SQLite 查询链，而不是新增行为轮询

### Decision: 行为监听复用 token watcher 的 Agent 启用状态和读取入口

行为提示与 token 统计本质上读取同一批 Agent 记录，但产出的业务对象不同。第一版 SHALL 复用现有 `enabled_agents` 生成的 active adapters：未启用 Agent 不启动对应文件监听、SQLite 轮询或行为解析。

读取层应尽量单次读取后 fan-out：
- Codex JSONL：notify/reconcile 已经读取的新增内容切片同时交给 token parser 与行为 parser；行为提示总开关关闭时跳过行为 parser
- OpenCode SQLite：复用已启用 OpenCode adapter 的 SQLite 增量查询结果，从同一批 message row 产出 token logs 与 `run_completed` 行为事件；第一版不启动独立的 OpenCode 行为轮询
- Agent 重新启用时的 catch-up/cold-start 只补 token 数据，不产生历史行为提示；进入正常监听后仅处理重新启用后的新事件

门控顺序：
1. `enabled_agents` 决定哪些 Agent adapter 会参与 token watcher
2. 行为提示总开关决定是否从这些已启用 Agent 的新增记录中解析行为事件
3. dispatcher 只接收通过上述两层门控后的行为事件

原因：
- 用户关闭某个 Agent 时，预期是不再读取它的统计，也不再接收它的行为提醒
- 同源数据不应被 token 统计和行为监听重复读取，否则会增加文件扫描或 SQLite 查询成本
- 行为提示是附加 UI 信号，不应扩大现有 Agent 开关的监听范围

### Decision: 增加内部 `turn_started` 事件清理同会话旧提醒

行为解析器除展示型事件外，还产出内部 `turn_started` 事件。该事件不展示 UI，仅清理同一 `agent_name + session_id` 下仍在队列或当前窗口中的旧提醒。

事件 key 由 `agent_name + session_id + turn_id + call_id + kind` 派生。Codex JSONL 的 `session_id` 可使用 session 文件路径或 session metadata 中可用的稳定 id；权限事件必须包含 `call_id`。

原因：
- 提醒只代表“刚发生的状态”，不是长期状态管理；同会话进入下一轮后，上一轮提醒自然过期
- 多会话并发时不能靠 Agent 名称粗略清理，否则会误伤其他会话
- Codex 的 `function_call_output` 可精确命中 `call_id`，用于清理仍存在的权限提醒

### Decision: 队列使用简单 FIFO，最多保留 10 条

dispatcher 维护一个内存提醒队列，最多 10 条。超过 10 条时直接丢弃最旧提醒，不做优先级保留。当前提醒关闭或自动消失后展示下一条。

生命周期规则：
- `turn_started`：清理同一 `agent_name + session_id` 的旧提醒，不展示 UI
- `permission_requested`：加入队列或替换同 key 提醒，不按时间自动隐藏
- `permission_resolved`：如果同一 `agent_name + session_id + turn_id + call_id` 的提醒仍存在，则移除
- `run_completed` / `run_aborted`：清理同一 `agent_name + session_id + turn_id` 的旧提醒，再展示 5 秒终止提醒
- `tool_error`：展示 8 秒；第一版可只保留类型与 UI 能力，不强制接入解析
- 手动关闭：只移除当前提醒，不代表审批完成，不产生状态标记

原因：
- 用户明确希望第一版只是提醒，不具备干预处理能力
- FIFO 行为比复杂优先级更可预测

### Decision: 只展示一个轻量 `behavior-tip` WebView 小窗

新增独立窗口 label，例如 `behavior-tip`。窗口使用透明、无装饰、不可调整尺寸、置顶的 WebView，渲染一个小型提示卡片。提示 UI 不复用现有 Popup 内容区，不提供 `Open` 按钮，只提供关闭入口。

原因：
- 现有 Popup 是用户主动打开的统计面板，主动提醒需要独立生命周期
- 不做活动历史可以保持第一版范围小，避免变成事件中心

### Decision: 提示展示时不抢焦点

展示 `behavior-tip` 时只显示窗口，不调用 `set_focus()`，也不激活主应用。窗口创建时应尽量保持 `focused(false)`，点击关闭只关闭当前提醒。

原因：
- 轻提醒不能打断用户在 Codex、OpenCode 或其他应用中的输入
- 弹窗不承担打开或处理动作，抢焦点没有必要

### Decision: 定位优先使用 tray rect 缓存，并提供平台兜底

托盘事件中可拿到 rect 时缓存最近一次主托盘位置。展示提示时：
- macOS：优先贴近缓存的菜单栏 rect；无缓存时贴近主显示器右上角
- Windows：优先贴近缓存的任务栏 rect；无缓存时贴近主显示器右下角
如果现有 Popup 已打开，`behavior-tip` 仍然展示，但应尽量避开 Popup 矩形；无法准确避开时使用平台默认兜底位置。

原因：
- Tauri 主动弹窗时不一定能立即获得 tray rect
- 平台兜底可保证首次事件也能展示
- Popup 可见不代表用户已经看到行为提醒，二者不应互相替代

### Decision: 前端只监听当前提示，不维护完整历史

后端 dispatcher 负责队列、去重和生命周期，前端只接收当前展示状态，例如 `behavior-tip-updated`。前端负责渲染、关闭入口和自动隐藏计时。

原因：
- 第一版不做历史，前端状态越轻越容易保证提示准确
- 生命周期核心依赖后端解析事件，放在后端更容易测试多会话清理规则

### Decision: 设置只提供总开关

Settings 增加一个行为提示总开关，默认关闭。关闭期间后端在行为解析入口前短路，不运行行为解析 fan-out、不执行独立行为轮询、不入队、不弹窗、不缓存行为事件；重新开启后只处理新事件，不回放关闭期间的旧事件。现有 token 统计监听和 OpenCode token 查询仍按 Agent 启用状态运行。

原因：
- 第一版不做独立的行为 Agent/事件类型细粒度配置，避免设置复杂
- 用户关闭提醒后不应被旧事件补弹
- 行为提示关闭时继续解析行为没有用户价值，会造成不必要的 CPU 与 SQLite 读取损耗

### Decision: 视觉贴合现有深色 Popup

提示采用深色半透明卡片、圆角、轻内描边、柔和阴影和食材系语义色。权限状态使用琥珀/面包色呼吸条，完成状态使用现有费用绿色，错误状态使用 muted red。动效仅作用于状态点或状态条，不让整张卡片持续缩放。

原因：
- 与当前 TokenBurger Popup 的整体气质一致
- 小提示需要被看见但不能像系统警告一样打断用户

## Risks / Trade-offs

- Codex 日志 schema 变化 → 将解析器做成容错模式，未知行跳过并记录 debug/warn，行为提示不影响 token 入库
- OpenCode 完成信号可能存在旧数据 → 升级现有 OpenCode token SQLite 增量查询以支持需要的水位线，token 与行为共同消费结果，避免新增独立轮询
- 首次定位没有 tray rect → 使用平台兜底位置，后续 tray 交互刷新缓存
- 多事件高频到达导致闪烁 → dispatcher 使用 FIFO 队列、同 key 替换和固定自动隐藏时间
- 权限提示无法自动消失 → Codex 同时监听 `function_call_output`、`task_complete`、`turn_aborted` 与下一轮 `turn_started`
- 手动关闭被误解为审批处理 → 手动关闭只移除当前提醒，不生成 resolved，不改变 Agent 状态
- 设置关闭期间跳过行为解析 → 这是有意取舍；提醒不是历史记录，重新开启后只处理新事件，token 统计不受影响
- Agent 开关与行为提示范围不一致 → 行为提示必须复用 `enabled_agents` active adapters，未启用 Agent 不进入行为解析
- `AgentAdapter` 仍然 token-centric → 这是第一版有意保留的兼容边界，完整 `AgentSource` / `TokenExtractor` 拆分应单独开 change
