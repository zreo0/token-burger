## Context

当前启动流程会先启动 Watcher 冷启动线程，再创建主托盘图标。冷启动线程在后台逐 Agent 扫描历史日志，写线程会边入库边更新 tray title，因此用户可能在扫描未完成时看到 `0`、旧数据或部分数据。Popup 也可以在此期间被左键打开，导致用户把未完成数据理解成最终统计。

Tauri v2 的 `TrayIconBuilder` 支持同时设置 menu 与 `on_tray_icon_event`，并可通过 `show_menu_on_left_click(false)` 保持左键自定义、右键菜单可用；tray 创建后也可通过已有 handle 更新 title/tooltip。因此本次改动可以集中在现有 Rust 启动编排与主托盘处理，不需要引入新依赖。

## Goals / Non-Goals

**Goals:**

- 主托盘创建时先显示本地化扫描状态，而不是 `0`。
- 冷启动完成前，左键点击主托盘不展示 Popup。
- 右键菜单与 Settings 入口在扫描期间保持可用。
- 冷启动完成后，立即恢复真实 token title，并允许正常打开 Popup。

**Non-Goals:**

- 不处理 Provider usage 的菜单栏图标、百分比或刷新策略。
- 不改变账号用量 UI 与存储。
- 不改变 token 日志解析规则、数据库 schema 或定价计算。
- 不新增独立启动窗口或系统通知。

## Decisions

1. 后端维护冷启动完成状态

   在 Rust 侧增加一个轻量的冷启动状态，随 `WatcherEngine::start` 创建并在所有启用 Adapter 冷启动完成后标记为完成。主托盘左键处理直接读取这个状态，避免依赖前端事件是否已订阅。

   备选方案是前端监听 `cold-start-progress` 后决定是否展示 Popup，但这会受到 Popup 尚未创建、事件错过、隐藏预热窗口等因素影响，不适合作为主托盘点击门控。

2. 扫描中 title 使用后端本地化

   托盘 title 在创建时设置为当前语言对应的扫描中文案：`Scanning...` 或 `扫描中...`。语言来源沿用启动时已读取的 `language` 设置。这样无需等待前端 i18n 初始化，也能在菜单栏第一时间给出状态。

   备选方案是始终显示英文，但会破坏现有 i18n 体验；另一方案是等前端初始化后回写 title，但启动阶段反馈会延迟。

3. 冷启动完成后主动刷新一次真实汇总

   冷启动期间写线程可能多次收到插入批次并尝试更新 title。为了避免扫描态被部分数据覆盖，扫描完成前主 token title 更新应被抑制或延后；完成后由后端查询当日汇总并更新主托盘 title。

   备选方案是允许部分数据逐步更新，但这正是本次要避免的误导体验。

4. 左键门控只影响主 Popup

   `on_tray_icon_event` 中左键点击如果冷启动未完成则直接返回，不调用 `toggle_popup_window`。右键菜单由 tray menu 处理，保持设置入口可用。

## Risks / Trade-offs

- [Risk] 冷启动没有启用 Adapter 时可能一直停在扫描态。→ Mitigation: total 为 0 时应立即标记完成并刷新真实汇总。
- [Risk] 写线程在扫描期间继续更新 tray title，覆盖扫描文案。→ Mitigation: tray title 更新函数需要检查冷启动状态，未完成时保持扫描文案。
- [Risk] 冷启动完成信号与最后一批写入存在竞态。→ Mitigation: 完成后主动执行一次汇总查询；如写入队列仍有批次，后续正常更新会继续刷新为真实数据。
- [Risk] 用户以为应用无响应，因为左键没有弹窗。→ Mitigation: title/tooltip 明确显示扫描中，右键设置仍可打开。
