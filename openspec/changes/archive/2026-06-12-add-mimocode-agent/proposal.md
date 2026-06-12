## Why

MiMoCode 虽然基于 OpenCode 改造，但它已经使用独立的 CLI、配置名、环境变量、数据目录和数据库路径。将 MiMoCode 塞进现有 OpenCode adapter 会让两个实现共享不稳定假设，后续任一方 schema 或行为变化都会互相牵连。

TokenBurger 需要把 MiMoCode 作为独立 Code Agent 接入：复用现有 Watcher、SQLite polling、watermark、TokenLog 入库和行为提示队列，但独立实现 MiMoCode 的 source 描述、token 解析和行为解析。

## What Changes

- 新增 `mimocode` Agent source，默认读取 MiMoCode SQLite 数据库，支持 `MIMOCODE_DB` 和 `MIMOCODE_HOME` 定位数据库
- 新增 MiMoCode token extractor，解析 `message.data` 中的 assistant usage、provider、model、session 和 request 信息
- 在现有 TokenType 限制下，将 MiMoCode `tokens.reasoning` 合并到 output 统计，并保留原始 reasoning 元数据用于后续扩展
- 新增 MiMoCode 完成事件解析：第一版只从 assistant `finish = "stop"` 生成 `run_completed`
- 新增 MiMoCode 前端展示名称、图标和菜单栏图标资源接入
- 第一版不做 MiMoCode 权限请求提醒，不新增 MiMoCode 账号额度监控，也不复用 OpenCode adapter 内部实现

## Capabilities

### New Capabilities

- 无。此次变更扩展现有 Agent adapter 与行为提示能力，不新增独立顶层能力。

### Modified Capabilities

- `agent-adapters`：新增独立 MiMoCode SQLite adapter
- `agent-behavior-tips`：新增 MiMoCode 完成提醒解析

## Impact

- 影响 specs：`agent-adapters`、`agent-behavior-tips`
- 影响代码区域：Rust Agent source/adapter、Watcher SQLite source 注册、Token extractor、Behavior extractor、前端 agent label/icon、Tauri 菜单栏图标资源
- 额外准备：用户本机需要安装并至少运行过 `mimo`，确保 `mimocode.db` 存在；若使用非默认 profile，需要提供 `MIMOCODE_DB` 或 `MIMOCODE_HOME`
