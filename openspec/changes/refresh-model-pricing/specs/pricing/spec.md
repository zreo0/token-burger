## MODIFIED Requirements

### Requirement: 远程价格表拉取
系统 SHALL 从 `https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json` 拉取模型价格数据。启动加载、自动刷新和手动刷新均使用 10 秒超时；运行时请求 MUST 在 blocking 工作线程执行，不得阻塞 Tauri UI。

#### Scenario: 拉取成功
- **WHEN** 网络可用且 URL 可达
- **THEN** 系统下载价格表 JSON，缓存到本地并允许调用方应用新价格表

#### Scenario: 启动时拉取失败
- **WHEN** 启动加载期间网络不可用或超时
- **THEN** 系统记录 warn 日志，尝试使用本地历史缓存或内置 fallback

#### Scenario: 运行时拉取失败
- **WHEN** 自动或手动刷新期间网络不可用、超时或价格表无法解析
- **THEN** 系统返回或记录刷新失败并继续保留当前内存价格表

### Requirement: 按天本地缓存
系统 SHALL 将拉取成功的价格表缓存到 `dirs::data_local_dir()/token-burger/pricing/model_pricing_YYYY-MM-DD.json`（生产环境）或 `dirs::data_local_dir()/token-burger/dev/pricing/model_pricing_YYYY-MM-DD.json`（开发环境）。自动加载路径在同一天内 SHALL 复用有效缓存，不重复拉取。

#### Scenario: 当天缓存存在
- **WHEN** 启动或运行时自动检查发现当天有效缓存文件已存在
- **THEN** 系统使用或继续保留当天价格，不发起网络请求

#### Scenario: 应用运行期间跨天
- **WHEN** 应用未退出且本地日期进入新的一天，当前日期没有有效缓存
- **THEN** 系统在后台检查或下一次 Popup 展示时发起网络请求并生成当天缓存

#### Scenario: 自动刷新失败后重试
- **WHEN** 新一天的远程拉取失败且当天缓存仍不存在
- **THEN** 系统保留旧价格，并在后续定期检查或 Popup 展示时再次尝试

## ADDED Requirements

### Requirement: 运行时自动更新价格表
系统 SHALL 在应用运行期间每小时检查一次模型价格新鲜度，并在每次 Popup 展示时补充检查。刷新成功后 MUST 原子替换共享内存中的完整 `PricingTable`，使后续查询返回新价格。

#### Scenario: 后台跨天刷新成功
- **WHEN** 后台检查发现当天没有有效缓存且远程拉取成功
- **THEN** 系统替换内存价格表并通知前端价格已更新

#### Scenario: 休眠跨天后打开 Popup
- **WHEN** 系统休眠期间跨天且用户唤醒后打开 Popup
- **THEN** Popup 触发价格新鲜度检查，并在刷新成功后重新计算预估费用

### Requirement: 手动强制刷新价格表
设置窗口 SHALL 在“数据”标签提供模型价格手动刷新入口。手动刷新 MUST 绕过当天缓存直接请求远程源；只有远程拉取和解析成功时才替换当前内存价格表。

#### Scenario: 手动刷新成功
- **WHEN** 用户点击“立即刷新”且远程价格可用
- **THEN** 系统覆盖当天缓存、更新内存价格表，并在设置窗口显示成功反馈和模型数量

#### Scenario: 手动刷新失败
- **WHEN** 用户点击“立即刷新”但远程请求或解析失败
- **THEN** 设置窗口显示失败反馈，当前内存价格和已有缓存保持可用
