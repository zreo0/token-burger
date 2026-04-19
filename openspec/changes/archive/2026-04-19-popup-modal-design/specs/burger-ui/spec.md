## ADDED Requirements

### Requirement: Popup 顶部摘要与 Top Models 展示
Popup 页面 SHALL 在时间范围选择器下方展示当前范围的总 Token 与预估花费摘要，并在 Burger 主可视化下方展示 Top Models 轻量榜单。

#### Scenario: 展示摘要信息
- **WHEN** Popup 页面已获取到当前时间范围的 `TokenSummary` 与价格表
- **THEN** 页面在 Burger 之前显示总 Token 数与预估花费

#### Scenario: 展示 Top Models
- **WHEN** `summary.by_model` 中存在模型统计数据
- **THEN** 页面在 Burger 之后展示按 token 消耗排序的前 2 个模型及其 token 数量

#### Scenario: 无模型数据
- **WHEN** `summary.by_model` 为空
- **THEN** 页面不显示 Top Models 列表或显示空态占位，但不得影响 Burger 主体布局

## MODIFIED Requirements

### Requirement: 四层 Burger 动画组件
Popup 页面 SHALL 展示一个四层 Burger 视图，自下而上依次为：Input 底部面包、Cache Create、Cache Read、Output 顶部面包。每层使用 Framer Motion 驱动动画，并直接在对应层内展示名称与 token 数值。

#### Scenario: 初始渲染
- **WHEN** Popup 页面首次加载并获取到 TokenSummary
- **THEN** 页面按 Input → Cache Create → Cache Read → Output 的顺序渲染四层 Burger，且每层在层内显示对应名称和数值

#### Scenario: 零数据状态
- **WHEN** 所有 token 类型数量为 0
- **THEN** Burger 仍保留四层结构，Input 与 Output 维持固定面包高度，两个缓存层至少显示最薄层高，并在层内显示 0

### Requirement: 层厚度动态变化
系统 SHALL 使用带上下限的非线性厚度映射来计算 Burger 各层高度。Input 与 Output SHALL 维持约 1.5 行文本高度的固定面包层；Cache Create 与 Cache Read SHALL 在约 1 行到 3 行文本高度之间变化，并在高值区间逐步逼近上限而不是线性无限增长。

#### Scenario: 缓存层低值变化
- **WHEN** Cache Read token 从很小的值增长到中等值
- **THEN** Cache Read 层厚度应有明显增长，以便用户感知缓存参与度变化

#### Scenario: 缓存层高值封顶
- **WHEN** Cache Create token 数极大
- **THEN** Cache Create 层高度不得超过约 3 行文本高度的视觉上限

#### Scenario: 面包层固定高度
- **WHEN** Input 或 Output token 数发生变化
- **THEN** 对应面包层仍保持固定的约 1.5 行文本高度，仅更新层内数值与轻量动效

### Requirement: 数字滚动效果
每层上的 token 数字 SHALL 在对应层内平滑更新，Today 范围的实时增长 MUST 采用细致、连续且不干扰操作的数字动画，而不是直接跳变或夸张翻滚。

#### Scenario: Today 实时更新
- **WHEN** Today 范围收到 `token-updated` 事件，某层 token 数增加
- **THEN** 该层内数字以平滑、短时且连续的方式过渡到新值，同时不造成整体布局抖动

#### Scenario: 数值保持可读
- **WHEN** 多次实时更新在短时间内连续到达
- **THEN** 数字动画仍应保持可读，不得出现明显卡顿或持续拖尾

### Requirement: 层颜色区分
四层 MUST 使用克制的主题色与明暗层级来区分语义，其中 Input 与 Output 需保有“面包”锚点感，Cache Create 与 Cache Read 需保有中间夹层的区分度。颜色方案 SHALL 同时支持暗色和亮色模式，且文字对比度必须满足层内可读性要求。

#### Scenario: 暗色模式
- **WHEN** 系统为暗色模式
- **THEN** Burger 各层使用暗色模式下的主题色与高对比文字，保持层内信息可读

#### Scenario: 亮色模式
- **WHEN** 系统为亮色模式
- **THEN** Burger 各层使用亮色模式下的主题色与高对比文字，保持层内信息可读

### Requirement: 时间范围选择
Popup 页面顶部 SHALL 展示时间范围选择器（Segmented Control 风格），选项为 Today / 7D / 30D。切换时调用 `get_token_summary` command 刷新数据，并以干脆、快速、不拖沓的重组动画更新 Burger、摘要和 Top Models。

#### Scenario: 切换时间范围
- **WHEN** 用户点击 `7D` 选项
- **THEN** 调用 `get_token_summary({ range: '7d' })`，并以快速重组动画更新 Popup 内容

#### Scenario: 默认选中 Today
- **WHEN** Popup 页面首次打开
- **THEN** 默认选中 `Today`

### Requirement: 预估花费展示
Popup 页面 SHALL 在顶部摘要区展示当前时间范围的预估花费金额（美元），并与总 Token 一起作为第一层级信息呈现。

#### Scenario: 显示花费
- **WHEN** 当前时间范围存在可计算价格的 token 消耗
- **THEN** 顶部摘要区显示计算后的美元金额

#### Scenario: 无价格数据
- **WHEN** 价格表中无匹配模型
- **THEN** 顶部摘要区显示 `$0.00`

### Requirement: Popup 窗口视觉风格
Popup 窗口 SHALL 保持 macOS 原生 popover 风格：无边框、透明背景、圆角容器、毛玻璃效果和柔和阴影，同时采用更适合菜单栏小窗的重新分区布局，并允许窗口尺寸适度扩大以容纳摘要、Burger 和 Top Models。

#### Scenario: 重新布局后仍保持菜单栏气质
- **WHEN** Popup 加入摘要区、Burger 主视图、Top Models 和状态区
- **THEN** 页面仍保持单屏阅读完成、无完整仪表盘式拥挤感

#### Scenario: 调整窗口尺寸
- **WHEN** 重新设计后的 Popup 需要更多垂直空间
- **THEN** 窗口尺寸可适度扩大，但仍应维持菜单栏小窗尺度

### Requirement: 冷启动加载状态
Popup 页面 SHALL 在冷启动期间显示轻量状态信息，并在不遮挡 Burger 主体结构的前提下更新进度。

#### Scenario: 冷启动进行中
- **WHEN** 冷启动尚未完成
- **THEN** 页面显示轻量状态信息和进度文字（如 `Loading... 2/5 agents`），且不打断摘要与主布局

#### Scenario: 冷启动完成
- **WHEN** 收到最后一个 Agent 的 `cold-start-progress`（completed == total）
- **THEN** 隐藏冷启动状态信息，显示完整的 Popup 内容
