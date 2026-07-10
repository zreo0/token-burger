## ADDED Requirements

### Requirement: 价格表刷新 Commands
系统 SHALL 提供异步 `ensure_pricing_fresh` 和 `reload_pricing` commands。`ensure_pricing_fresh` 仅在当天没有有效缓存时拉取远程价格；`reload_pricing` SHALL 强制绕过当天缓存，并在成功后返回更新的模型数量。

#### Scenario: Popup 检查价格新鲜度
- **WHEN** 前端调用 `invoke('ensure_pricing_fresh')` 且当天缓存有效
- **THEN** command 不发起网络请求并成功返回未更新状态

#### Scenario: 设置页强制刷新价格
- **WHEN** 前端调用 `invoke('reload_pricing')` 且远程拉取成功
- **THEN** command 返回包含模型数量的刷新结果

#### Scenario: 并发刷新请求
- **WHEN** 自动检查和手动刷新在同一时间触发
- **THEN** 系统串行执行刷新，避免并发写入同一个当天缓存

### Requirement: pricing-updated 事件
系统 SHALL 在运行时价格表成功替换后，通过 `app.emit("pricing-updated", ())` 广播轻量更新事件。Popup SHALL 在收到事件后重新调用 `get_pricing` 并使用返回值重新计算费用。

#### Scenario: Popup 接收价格更新
- **WHEN** 自动或手动刷新成功替换内存价格表
- **THEN** Popup 收到 `pricing-updated` 事件、重新读取价格表并更新预估费用

#### Scenario: 价格刷新失败
- **WHEN** 自动或手动刷新未取得有效远程价格表
- **THEN** 系统不广播 `pricing-updated` 事件
