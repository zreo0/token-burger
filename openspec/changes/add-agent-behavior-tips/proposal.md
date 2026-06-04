## Why

TokenBurger 目前能实时统计 token，但用户仍需要切回各 Agent 才能知道当前运行是否等待权限、是否已经完成。本变更为菜单栏/任务栏场景补充一个轻量、界面型的实时提示层，让关键行为信号在不使用系统通知的前提下被及时看到。

## What Changes

- 在现有 watcher/adapter 架构上接入行为解析 fan-out，使行为提示与 token 统计复用同一批新增数据；完整 `AgentSource` / `TokenExtractor` 重构不属于本 change。
- 新增 Agent 行为事件解析与分发能力，第一版覆盖 Codex 的权限请求、权限处理、任务完成和任务中断，以及 OpenCode 的任务完成。
- 新增轻量 `behavior-tip` 弹窗，macOS 靠近菜单栏图标显示，Windows 靠近任务栏托盘区域显示；弹窗不抢焦点，只提供关闭入口。
- 新增提示队列和生命周期规则：内存最多保留 10 条提醒，超过后丢弃最旧提醒；同会话新轮次开始时清理旧提醒；完成/中断/错误提醒按固定时长自动消失。
- 新增行为提示总开关，默认开启；行为监听复用现有 Agent 启用状态，关闭期间不运行行为解析 fan-out 或独立行为轮询、不入队、不弹窗、不回放旧事件。
- 保持现有 Popup token 统计视图不变，第一版不新增活动历史视图，不做系统通知。
- 第一版不实现 OpenCode 权限审批提示；后续在确认稳定权限信号后通过独立 Agent 行为解析器补充，但仍复用现有 watcher 数据入口。
- 第一版不新增独立的行为 Agent 开关；行为提示只依赖现有 Agent 开关和行为提示总开关。

## Capabilities

### New Capabilities
- `agent-behavior-tips`: 定义 Agent 行为事件解析、跨会话提示队列、轻量提示弹窗定位与展示生命周期。

### Modified Capabilities

## Impact

- Rust 后端：保留现有 `AgentAdapter` token 解析链路，在 watcher 正常监听阶段增加行为解析 fan-out；新增行为事件类型、Agent 行为解析模块、提示分发/去重逻辑和行为提示开关读取。
- Tauri 窗口：新增 `behavior-tip` WebView 小窗，复用或补充托盘位置缓存，避免影响现有 Popup，展示时不抢焦点。
- React 前端：新增轻量提示页面/组件，监听后端事件并渲染与现有深色 Popup 气质一致的提示 UI。
- 数据与安全：不写入新的历史数据表；行为 payload 不包含 prompt、工具输出全文或敏感命令详情，仅保留用于展示和去重的必要元数据。
