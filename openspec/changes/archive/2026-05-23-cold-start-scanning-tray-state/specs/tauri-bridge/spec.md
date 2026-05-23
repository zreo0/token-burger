## ADDED Requirements

### Requirement: 冷启动期间主托盘点击门控
系统 SHALL 在冷启动完成前拦截主托盘左键点击，不展示 Popup。该门控 MUST 仅影响主托盘左键打开 Popup 的行为，不得影响右键菜单、Settings 打开或 Quit 行为。

#### Scenario: 扫描期间左键点击主托盘
- **WHEN** 冷启动尚未完成且用户左键点击主托盘图标
- **THEN** 系统不创建、不显示、不聚焦 Popup 窗口

#### Scenario: 扫描期间右键打开菜单
- **WHEN** 冷启动尚未完成且用户右键打开主托盘菜单
- **THEN** 系统正常展示菜单，并允许用户打开 Settings 或 Quit

#### Scenario: 扫描完成后左键点击主托盘
- **WHEN** 冷启动已经完成且用户左键点击主托盘图标
- **THEN** 系统按既有逻辑定位并展示 Popup 窗口

## MODIFIED Requirements

### Requirement: Tray Title 动态更新
系统 SHALL 在冷启动完成前将主 tray title 保持为本地化扫描状态；英文显示 `Scanning...`，简体中文显示 `扫描中...`。冷启动完成后，系统 SHALL 查询并展示真实 token 汇总。冷启动完成后的每次 `token-updated` 事件广播 SHALL 同步更新 tray title。格式化规则：`< 1000` 显示原数，`≥ 1000` 显示 K，`≥ 1000000` 显示 M，`≥ 1000000000` 显示 B。保留一位小数（如 `1.2K`、`3.5M`）。Provider usage 的附加菜单栏显示不属于本要求范围。

#### Scenario: 创建托盘时显示扫描态
- **WHEN** 应用创建主托盘图标且冷启动尚未完成
- **THEN** tray title 显示当前语言对应的扫描中文案，而不是 `0` 或部分 token 汇总

#### Scenario: 扫描期间入库不覆盖扫描态
- **WHEN** 冷启动期间写线程完成一批 TokenLog 插入并产生新的今日汇总
- **THEN** 主 tray title 仍保持扫描中文案，不展示部分 token 汇总

#### Scenario: 扫描完成后展示真实汇总
- **WHEN** 冷启动完成
- **THEN** 系统查询今日汇总并将主 tray title 更新为格式化后的真实 token 数

#### Scenario: 格式化 token 数
- **WHEN** 冷启动已完成且今日总 token 数为 45678
- **THEN** tray title 显示 `45.7K`

#### Scenario: 零 token
- **WHEN** 冷启动已完成且今日无任何 token 记录
- **THEN** tray title 显示 `0`
