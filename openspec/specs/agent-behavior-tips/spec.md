# Purpose
定义 Agent 运行提醒的事件解析、队列生命周期和轻量提示窗口行为。

## Requirements

### Requirement: Agent 数据读取复用与行为解析边界
系统 SHALL 在 token watcher 读取链路上复用新增数据给行为解析器。第一版 MUST NOT 为运行提醒启动第二套文件扫描、notify 注册或 SQLite 轮询。

#### Scenario: Token 与行为共享新增 batch
- **WHEN** 已启用 Agent 产生新的文件内容或 SQLite row
- **THEN** watcher 只读取一次新增数据，并将同一批数据用于 token 统计和行为解析

#### Scenario: 行为解析器不读取数据源
- **WHEN** 行为解析器运行
- **THEN** 它不得自行执行 glob 扫描、notify 注册、SQLite 打开查询或数据源发现

#### Scenario: offset 仍由读取链路维护
- **WHEN** 新增数据被成功处理
- **THEN** offset 或 watermark 由 watcher 读取链路更新，不由行为解析器单独维护

### Requirement: Agent 行为事件模型
系统 SHALL 定义统一的 Agent 行为事件模型，用于表达新轮次开始、权限请求、权限已处理、任务完成、任务中断和工具错误等短生命周期信号。事件 MUST 包含 `agent_name`、`session_id`、`kind`、`timestamp` 和稳定去重 key 所需字段；Codex 权限事件 MUST 包含 `turn_id` 与 `call_id`。`turn_started` MUST 作为内部清理信号，不展示 UI。

#### Scenario: 事件包含精确命中字段
- **WHEN** Codex 行为解析器识别到需要权限的工具调用
- **THEN** 系统生成包含 `agent_name = "codex"`、`session_id`、`turn_id`、`call_id`、`kind = "permission_requested"` 和可解析 `timestamp` 的行为事件

#### Scenario: 新轮次开始不展示
- **WHEN** 系统解析到某 Agent 会话的新轮次开始信号
- **THEN** 系统生成内部 `turn_started` 事件用于清理同会话旧提醒，但不展示弹窗

#### Scenario: 不包含敏感内容
- **WHEN** 系统向前端发送行为事件
- **THEN** payload 不包含 prompt、完整工具输出、完整命令参数或凭据值

### Requirement: Codex 行为解析
系统 SHALL 从 Codex JSONL 增量内容中解析行为信号。`task_started` MUST 解析为内部新轮次开始；`exec_command` 的 `sandbox_permissions = "require_escalated"` MUST 解析为权限请求；同一 `call_id` 的 `function_call_output` MUST 解析为权限已处理；`task_complete` MUST 解析为任务完成；`turn_aborted` MUST 解析为任务中断。时间 MUST 优先使用 JSONL 顶层 `timestamp`。

#### Scenario: 解析新轮次开始
- **WHEN** Codex JSONL 新增 `event_msg` 且 payload type 为 `task_started`
- **THEN** 系统生成内部 `turn_started` 行为事件，并使用同一 `session_id` 清理旧提醒

#### Scenario: 解析权限请求
- **WHEN** Codex JSONL 新增一行 `response_item`，其 payload 为 `function_call`，名称为 `exec_command`，且参数中 `sandbox_permissions` 为 `"require_escalated"`
- **THEN** 系统生成 `permission_requested` 行为事件，并使用该行的 `call_id` 作为权限提示精确匹配字段

#### Scenario: 权限处理后自动解析 resolved
- **WHEN** Codex JSONL 后续出现同一 `call_id` 的 `function_call_output`
- **THEN** 系统生成 `permission_resolved` 行为事件，用于清除同一权限提示

#### Scenario: 解析任务完成
- **WHEN** Codex JSONL 新增 `event_msg` 且 payload type 为 `task_complete`
- **THEN** 系统生成 `run_completed` 行为事件，并保留同一 `turn_id` 用于清理该轮仍存在的权限提醒

#### Scenario: 解析任务中断
- **WHEN** Codex JSONL 新增 `event_msg` 且 payload type 为 `turn_aborted`
- **THEN** 系统生成 `run_aborted` 行为事件，并保留中断 reason（如可用）

### Requirement: OpenCode 完成事件解析
系统 SHALL 从 OpenCode SQLite 数据库中解析任务完成信号。第一版 MUST 仅将 `message.data.role = "assistant"` 且 `message.data.finish = "stop"` 的新增或更新记录解析为 `run_completed` 行为事件；系统 MUST NOT 从 OpenCode 解析权限请求。

#### Scenario: 解析 OpenCode 完成
- **WHEN** 现有 OpenCode SQLite token 轮询发现新的 assistant message，且 data JSON 中 `finish` 为 `"stop"`
- **THEN** 系统生成 `agent_name = "opencode"` 且 `kind = "run_completed"` 的行为事件

#### Scenario: 不解析 OpenCode 权限
- **WHEN** OpenCode SQLite 中存在 `permission`、`session.permission` 或 tool pending/running 相关字段
- **THEN** 第一版系统不生成 OpenCode `permission_requested` 行为事件

### Requirement: MiMoCode 完成事件解析
系统 SHALL 从 MiMoCode SQLite 数据库中解析任务完成信号。第一版 MUST 仅将 `message.data.role = "assistant"` 且 `message.data.finish = "stop"` 的新增或更新记录解析为 `run_completed` 行为事件。系统 MUST NOT 从 MiMoCode 解析权限请求。

#### Scenario: 解析 MiMoCode 完成
- **WHEN** 现有 MiMoCode SQLite token 轮询发现新的 assistant message，且 data JSON 中 `finish` 为 `"stop"`
- **THEN** 系统生成 `agent_name = "mimocode"` 且 `kind = "run_completed"` 的行为事件

#### Scenario: 不解析 MiMoCode 权限
- **WHEN** MiMoCode 运行期间出现权限请求或工具等待状态
- **THEN** 第一版系统不生成 MiMoCode `permission_requested` 行为事件

#### Scenario: 冷启动不回放完成提醒
- **WHEN** 应用启动并扫描 MiMoCode SQLite 历史记录
- **THEN** 系统可以补录 MiMoCode token 数据，但不得把历史完成记录转换为行为提示

#### Scenario: 行为提示开关关闭
- **WHEN** 行为提示总开关关闭且 MiMoCode 产生新的 SQLite row
- **THEN** 系统仍可解析 MiMoCode token logs，但不运行 MiMoCode 行为解析、不入队、不展示 behavior-tip

### Requirement: MiMoCode 行为提示展示
系统 SHALL 在 behavior-tip 前端中为 MiMoCode 提供稳定展示名称、图标和本地化完成文案。MiMoCode 完成提醒 MUST 使用已有轻量提示窗口、队列去重、自动隐藏和不抢焦点规则。

#### Scenario: 展示 MiMoCode 完成提醒
- **WHEN** dispatcher 选中 MiMoCode `run_completed` 事件作为当前提示
- **THEN** behavior-tip 窗口显示 MiMoCode 名称、MiMoCode 图标和完成提示文案，并按完成提醒规则自动隐藏

#### Scenario: 前端图标存在
- **WHEN** behavior-tip 收到 `agent_name = "mimocode"` 的提示
- **THEN** 前端使用 MiMoCode provider icon，而不是文本 fallback

### Requirement: 行为监听依赖 Agent 启用状态
系统 SHALL 复用现有 token watcher 的 Agent 启用状态和新增数据读取入口。未启用 Agent 的记录 MUST NOT 被行为解析器读取、解析、入队或展示。Agent 重新启用后的历史补扫 MUST NOT 产生行为提示。

#### Scenario: 关闭 Agent 后不提示
- **WHEN** 用户在 Settings 中关闭 Codex 或 OpenCode Agent
- **THEN** 系统不再为该 Agent 启动行为监听、行为解析或行为提示，即使行为提示总开关仍为开启

#### Scenario: 同源数据只读取一次
- **WHEN** 已启用 Agent 产生新的日志内容或 SQLite message row
- **THEN** 系统从 token watcher 的同一批新增数据中产出 token logs 和行为事件，而不是启动第二套独立文件扫描或 SQLite 轮询

#### Scenario: 重新启用 Agent 不回放历史行为
- **WHEN** 用户重新启用某 Agent 且系统执行历史 catch-up
- **THEN** 系统可以补录该 Agent 的 token 数据，但不得把 catch-up 期间的历史记录转换为行为提示

### Requirement: 多会话提示队列
系统 SHALL 维护最多 10 条的内存行为提示队列，并按事件 key 去重。多个会话同时触发事件时，系统 MUST 只显示一个 `behavior-tip` 窗口；队列超过 10 条时 MUST 直接丢弃最旧提醒。队列 MUST 不持久化。

#### Scenario: 多个会话同时提醒
- **WHEN** 两个不同会话先后生成可展示行为事件
- **THEN** 系统只展示一个提示窗口，并将未展示的提醒保留在内存队列中

#### Scenario: 队列超过上限
- **WHEN** 内存队列已有 10 条提醒且又收到新的可展示提醒
- **THEN** 系统丢弃最旧提醒，并保留最新收到的提醒

#### Scenario: 同 key 更新
- **WHEN** 系统收到与已有提示 key 相同的行为事件
- **THEN** 系统更新该提示而不是创建重复提示

### Requirement: 提示生命周期
系统 SHALL 将行为提示视为轻量提醒而不是 Agent 状态。手动关闭 MUST 只移除当前提醒，不代表审批已处理，也不得产生状态标记。同一会话出现 `turn_started` 时，系统 MUST 清理该 `agent_name + session_id` 下所有旧提醒。完成和中断提醒 MUST 在 5 秒后自动隐藏；错误提醒 MUST 在 8 秒后自动隐藏；权限提醒不按时间自动隐藏，但可被手动关闭、同一 `call_id` 的 `permission_resolved` 或同会话新轮次清理。

#### Scenario: call_id resolved 自动关闭
- **WHEN** 当前展示 Codex 权限提示，且系统收到同一 `agent_name + session_id + turn_id + call_id` 的 `permission_resolved`
- **THEN** 系统关闭该权限提示，并在队列中有其他待展示提示时切换到下一条

#### Scenario: 新轮次清理旧提醒
- **WHEN** 某会话下存在旧提醒，且系统收到同一 `agent_name + session_id` 的 `turn_started`
- **THEN** 系统移除该会话下所有旧提醒，不影响其他 session 的提示

#### Scenario: 手动关闭只关闭当前提醒
- **WHEN** 用户点击权限提示关闭按钮
- **THEN** 系统移除当前提醒，但不得生成 `permission_resolved`，也不得清除其他会话的提醒

#### Scenario: 完成提醒自动隐藏
- **WHEN** 系统展示 `run_completed` 或 `run_aborted` 提醒
- **THEN** 该提醒在 5 秒后自动隐藏，并展示队列中的下一条提醒

#### Scenario: 错误提醒自动隐藏
- **WHEN** 系统展示 `tool_error` 提醒
- **THEN** 该提醒在 8 秒后自动隐藏，并展示队列中的下一条提醒

### Requirement: 轻量行为提示窗口
系统 SHALL 创建或复用独立的 `behavior-tip` WebView 窗口展示当前提示。该窗口 MUST 不使用系统通知，MUST 不替代现有 Popup，MUST 不展示活动历史视图，MUST 不提供 `Open`、跳转、审批处理或其他干预 Agent 运行的操作。窗口展示时 MUST 不抢焦点。

#### Scenario: 展示权限提示
- **WHEN** dispatcher 选中 `permission_requested` 作为当前提示
- **THEN** `behavior-tip` 窗口显示符合现有深色 Popup 气质的权限提示卡片，并只提供关闭入口

#### Scenario: 展示完成提示
- **WHEN** dispatcher 选中 `run_completed` 作为当前提示
- **THEN** `behavior-tip` 窗口显示完成提示，并在短时间后自动隐藏

#### Scenario: 不显示活动历史
- **WHEN** 用户看到行为提示窗口
- **THEN** 窗口仅显示当前提示和必要操作，不显示事件列表、时间线或历史记录

#### Scenario: 不抢焦点
- **WHEN** 系统显示 `behavior-tip` 窗口
- **THEN** 当前用户输入焦点不被切换到 `behavior-tip` 或 TokenBurger 主窗口

### Requirement: 平台定位
系统 SHALL 将 `behavior-tip` 定位在与主托盘图标相关的位置。macOS SHOULD 优先靠近菜单栏图标；Windows SHOULD 优先靠近任务栏托盘区域。若当前没有可用托盘 rect，系统 MUST 使用主显示器平台默认角落作为兜底位置。若现有 Popup 已打开，系统仍 SHALL 展示 `behavior-tip`，并尽量避开 Popup 矩形。

#### Scenario: macOS 使用菜单栏位置
- **WHEN** macOS 上已有最近一次主托盘 rect 缓存且需要显示行为提示
- **THEN** 系统将 `behavior-tip` 定位到该菜单栏图标附近

#### Scenario: Windows 使用任务栏位置
- **WHEN** Windows 上已有最近一次主托盘 rect 缓存且需要显示行为提示
- **THEN** 系统将 `behavior-tip` 定位到该任务栏图标附近

#### Scenario: 无 rect 兜底
- **WHEN** 行为提示首次出现且系统尚无主托盘 rect 缓存
- **THEN** macOS 将窗口定位到主显示器右上角附近，Windows 将窗口定位到主显示器右下角附近

#### Scenario: Popup 打开时仍展示
- **WHEN** 现有 token summary Popup 已可见且新的行为提醒需要展示
- **THEN** 系统仍展示 `behavior-tip`，并尽量避免覆盖现有 Popup

### Requirement: 行为提示开关
系统 SHALL 提供行为提示总开关，默认关闭。关闭后，系统 MUST 在行为解析入口前短路，避免运行行为解析 fan-out 或独立行为轮询；系统 MUST 不将行为事件加入提示队列、不显示 `behavior-tip`、不缓存关闭期间的行为提醒。重新开启后 MUST 只处理新事件，不回放旧提醒。现有 token 统计监听 MUST 不受该开关影响。

#### Scenario: 默认关闭
- **WHEN** 用户未配置行为提示开关
- **THEN** 系统按关闭状态启动，不解析行为事件、不入队、不显示 `behavior-tip`

#### Scenario: 关闭后跳过行为处理
- **WHEN** 用户关闭行为提示开关且 Agent 产生新的日志或 SQLite 记录
- **THEN** 系统不运行行为解析 fan-out 或独立行为轮询，不入队、不弹窗，也不在重新开启后回放该事件

#### Scenario: token 统计继续运行
- **WHEN** 用户关闭行为提示开关且 Agent 产生新的 token 记录
- **THEN** 系统仍按现有 token 监听流程解析、入库并广播 token 汇总

### Requirement: 启动后只处理新事件
系统 SHALL 在冷启动历史扫描期间禁止产生行为提示。冷启动和 offset 初始化完成后，系统 MUST 只对后续新增或更新的行为事件生成提醒。

#### Scenario: 冷启动不弹历史提醒
- **WHEN** 应用启动并扫描历史 Codex JSONL 或 OpenCode SQLite 记录
- **THEN** 系统不展示任何历史行为提醒

#### Scenario: 启动后新增事件可提醒
- **WHEN** 冷启动完成后 Codex 或 OpenCode 产生新的可展示行为事件
- **THEN** 系统按行为提示规则展示轻量提醒

### Requirement: 现有统计功能隔离
行为提示功能 SHALL 与现有 token 统计、Popup、账号用量刷新和主托盘 title 更新保持隔离。行为解析失败 MUST NOT 阻止 token logs 入库或 `token-updated` 广播。

#### Scenario: 行为解析失败不影响 token
- **WHEN** 某条 Codex 或 OpenCode 行为记录格式异常但 token 记录可正常解析
- **THEN** 系统跳过该行为事件并继续入库 token logs

#### Scenario: Popup 不被行为提示替代
- **WHEN** 用户左键点击主托盘打开现有 Popup
- **THEN** 系统仍展示 token summary Popup，而不是行为提示窗口
