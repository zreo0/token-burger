## MODIFIED Requirements

### Requirement: 冷启动编排
系统首次启动时 SHALL 进入冷启动模式：逐 Agent 在后台线程解析历史日志，按 mtime 过滤最近 N 天（默认 30 天，可配置）的文件。每完成一个 Agent MUST 通过 `emit("cold-start-progress", ...)` 广播进度。系统 MUST 维护可靠的冷启动完成状态；全部完成后 MUST 标记冷启动完成，并切换到正常监听模式。

#### Scenario: 冷启动进度广播
- **WHEN** 冷启动完成 Claude Code 的历史解析
- **THEN** 系统 emit `cold-start-progress` 事件，payload 包含 `{ agent: "claude-code", done: true, total: 5, completed: 1 }`

#### Scenario: 冷启动 mtime 过滤
- **WHEN** `~/.claude/projects/` 下有 90 天前的 JSONL 文件
- **THEN** 系统跳过该文件（超出 30 天保留期）

#### Scenario: 增量可用
- **WHEN** Claude Code 冷启动完成但 Codex 尚未开始
- **THEN** 数据可以入库并参与后续汇总，但主托盘 token title 不得把该部分数据展示为最终完成状态

#### Scenario: 冷启动完成状态
- **WHEN** 所有启用 Agent 的冷启动解析都已完成
- **THEN** 系统标记冷启动完成，并允许主托盘恢复正常 token 汇总展示与 Popup 打开行为

#### Scenario: 无启用 Agent
- **WHEN** 冷启动开始时没有任何启用 Agent 需要扫描
- **THEN** 系统立即标记冷启动完成，并进入正常监听模式
