## ADDED Requirements

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
