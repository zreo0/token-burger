## ADDED Requirements

### Requirement: 远程价格表拉取
系统 SHALL 在启动时从 `https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json` 拉取模型价格数据。使用 `reqwest` blocking 模式，超时 10 秒。

#### Scenario: 拉取成功
- **WHEN** 网络可用且 URL 可达
- **THEN** 系统下载价格表 JSON 并缓存到本地

#### Scenario: 拉取失败
- **WHEN** 网络不可用或超时
- **THEN** 系统记录 warn 日志，尝试使用本地缓存或内置 fallback

### Requirement: 按天本地缓存
系统 SHALL 将拉取成功的价格表缓存到 `~/.token-burger/pricing/model_pricing_YYYY-MM-DD.json`（生产环境）或 `~/.token-burger/dev/pricing/model_pricing_YYYY-MM-DD.json`（开发环境）。同一天内不重复拉取。

#### Scenario: 当天缓存存在
- **WHEN** 启动时发现当天的缓存文件已存在
- **THEN** 直接加载缓存，不发起网络请求

#### Scenario: 缓存过期
- **WHEN** 启动时发现缓存文件日期为昨天
- **THEN** 发起新的网络请求拉取最新价格表

### Requirement: 内置 Fallback 价格表
系统 SHALL 在 `src-tauri/resources/default_pricing.json` 内置一份默认价格表。当远程拉取失败且无本地缓存时 MUST 使用此 fallback。

#### Scenario: 完全离线首次启动
- **WHEN** 无网络且无本地缓存
- **THEN** 系统加载内置的 `default_pricing.json`

### Requirement: 模型名匹配策略
系统 SHALL 按以下优先级匹配模型价格：1) 精确匹配 2) 归一化匹配（去除日期后缀如 `-20250219` 和版本号如 `-v2`）3) Provider 前缀匹配（去除 `anthropic/`、`openai/` 等前缀）4) 未匹配返回零价格。

#### Scenario: 精确匹配
- **WHEN** model_id 为 `claude-3-7-sonnet-20250219` 且价格表中存在该 key
- **THEN** 返回精确匹配的价格

#### Scenario: 归一化匹配
- **WHEN** model_id 为 `claude-3-7-sonnet-20250219` 且价格表中无精确 key 但有 `claude-3-7-sonnet`
- **THEN** 去除日期后缀后匹配成功

#### Scenario: Provider 前缀匹配
- **WHEN** model_id 为 `claude-3-7-sonnet` 且价格表中 key 为 `anthropic/claude-3-7-sonnet`
- **THEN** 匹配成功

#### Scenario: 未知模型
- **WHEN** model_id 在价格表中无任何匹配
- **THEN** 返回零价格（input_price = 0, output_price = 0）

### Requirement: 前端金额计算
前端 SHALL 使用 token 数量和价格表即时计算预估花费。公式：`cost = (input × input_price + output × output_price + cache_create × cache_create_price + cache_read × cache_read_price) / 1_000_000`。价格单位为美元/百万 token（LiteLLM 格式）。

#### Scenario: 计算单模型花费
- **WHEN** claude-3-7-sonnet 消耗 input=500000, output=50000
- **THEN** 按价格表中的 input_cost_per_token 和 output_cost_per_token 计算总花费

#### Scenario: 多模型汇总花费
- **WHEN** 今日使用了 3 个不同模型
- **THEN** 分别计算每个模型的花费后求和

#### Scenario: 金额格式化
- **WHEN** 计算结果为 0.0234 美元
- **THEN** 前端显示 "$0.02"（保留两位小数）
