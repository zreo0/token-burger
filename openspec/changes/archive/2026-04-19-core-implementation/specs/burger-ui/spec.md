## ADDED Requirements

### Requirement: 四层 Burger 动画组件
Popup 页面 SHALL 展示一个四层汉堡图案，自下而上依次为：底部面包 → Input Token → Cache Create Token → Cache Read Token → Output Token → 顶部面包。每层使用 Framer Motion 的 `motion.div` 实现动画。

#### Scenario: 初始渲染
- **WHEN** Popup 页面首次加载并获取到 TokenSummary
- **THEN** 四层汉堡按各 token 类型的数量渲染对应厚度，使用 spring 动画过渡

#### Scenario: 零数据状态
- **WHEN** 所有 token 类型数量为 0
- **THEN** 四层均显示最小厚度，数字显示 0

### Requirement: 层厚度动态变化
每层的高度 SHALL 根据 token 数量动态计算：`height = minHeight + (tokenCount / maxTokenCount) * maxExtraHeight`。使用 Framer Motion `spring` transition（stiffness: 300, damping: 20）实现物理弹簧感的厚度变化。

#### Scenario: Token 数增加
- **WHEN** 收到 token-updated 事件，input token 从 1000 增加到 5000
- **THEN** Input 层高度以弹簧动画平滑增加

#### Scenario: 厚度上下限
- **WHEN** 某层 token 数为 0
- **THEN** 该层显示 minHeight（不消失）
- **WHEN** 某层 token 数极大
- **THEN** 该层高度不超过 maxExtraHeight + minHeight

### Requirement: 数字滚动效果
每层上的 token 数字 SHALL 使用 Framer Motion 的 `useMotionValue` + `useTransform` 实现平滑滚动增加效果，而非直接跳变。

#### Scenario: 数字平滑过渡
- **WHEN** input token 从 10000 变为 15000
- **THEN** 数字从 10000 平滑滚动到 15000（约 500ms 过渡）

### Requirement: 层颜色区分
四层 MUST 使用不同的高饱和度渐变色作为视觉区分：Input（绿色系）、Cache Create（橙色系）、Cache Read（蓝色系）、Output（红色系）。颜色方案 SHALL 同时支持暗色和亮色模式。

#### Scenario: 暗色模式
- **WHEN** 系统为暗色模式
- **THEN** 四层使用暗色模式下的渐变色方案

#### Scenario: 亮色模式
- **WHEN** 系统为亮色模式
- **THEN** 四层使用亮色模式下的渐变色方案

### Requirement: 时间范围选择
Popup 页面顶部 SHALL 展示时间范围选择器（Segmented Control 风格），选项为 Today / 7D / 30D。切换时调用 `get_token_summary` command 刷新数据。

#### Scenario: 切换时间范围
- **WHEN** 用户点击 "7D" 选项
- **THEN** 调用 `get_token_summary({ range: '7d' })`，Burger 动画过渡到新数据

#### Scenario: 默认选中 Today
- **WHEN** Popup 页面首次打开
- **THEN** 默认选中 "Today"

### Requirement: 预估花费展示
Popup 页面底部 SHALL 展示当前时间范围的预估花费金额（美元），使用价格表实时计算。

#### Scenario: 显示花费
- **WHEN** 今日消耗 input=1000000, output=100000（claude-3-7-sonnet）
- **THEN** 底部显示计算后的美元金额（如 "$3.15"）

#### Scenario: 无价格数据
- **WHEN** 价格表中无匹配模型
- **THEN** 显示 "$0.00"

### Requirement: useTokenStream Hook
系统 SHALL 提供 `useTokenStream` 自定义 hook，封装 Tauri `token-updated` 事件监听和 `get_token_summary` command 调用。Hook 返回 `{ summary, loading, error, refresh }` 状态。

#### Scenario: 初始化加载
- **WHEN** hook 首次挂载
- **THEN** 调用 `get_token_summary` 获取初始数据，`loading` 为 true 直到数据返回

#### Scenario: 实时更新
- **WHEN** 收到 `token-updated` 事件
- **THEN** 自动更新 `summary` 状态，触发 UI 重渲染

#### Scenario: 组件卸载清理
- **WHEN** 组件卸载
- **THEN** 取消事件监听（调用 unlisten）

### Requirement: Popup 窗口视觉风格
Popup 窗口 SHALL 采用 macOS 原生 popover 风格：无边框 + 透明背景 + 圆角容器（12px）+ 毛玻璃效果（`backdrop-filter: blur()`）+ 柔和阴影。暗色模式优先，同时支持亮色模式（跟随系统 `prefers-color-scheme`）。

#### Scenario: 暗色模式渲染
- **WHEN** 系统为暗色模式
- **THEN** Popup 使用深色半透明背景 + 毛玻璃效果

#### Scenario: 亮色模式渲染
- **WHEN** 系统为亮色模式
- **THEN** Popup 使用浅色半透明背景 + 毛玻璃效果

### Requirement: 冷启动加载状态
Popup 页面 SHALL 在冷启动期间显示加载状态（"Burger 制作中..."），监听 `cold-start-progress` 事件更新进度。

#### Scenario: 冷启动进行中
- **WHEN** 冷启动尚未完成
- **THEN** 显示加载动画和进度文字（如 "Loading... 2/5 agents"）

#### Scenario: 冷启动完成
- **WHEN** 收到最后一个 Agent 的 `cold-start-progress`（completed == total）
- **THEN** 隐藏加载状态，显示完整的 Burger 视图

### Requirement: i18n 支持
所有用户可见的 UI 文本 SHALL 通过 `react-i18next` 的 `useTranslation` hook 获取，支持英文（默认）和简体中文。Agent 名称和 model ID 不翻译。

#### Scenario: 英文界面
- **WHEN** 语言设置为 "en"
- **THEN** 所有 UI 文本显示英文（如 "Today", "Estimated Cost"）

#### Scenario: 中文界面
- **WHEN** 语言设置为 "zh-CN"
- **THEN** 所有 UI 文本显示中文（如 "今日", "预估花费"）

### Requirement: Error Boundary
Popup 页面 SHALL 被 React Error Boundary 包裹。渲染异常时显示 fallback UI，不影响 tray 和 settings 功能。

#### Scenario: 渲染异常
- **WHEN** Burger 组件抛出运行时错误
- **THEN** 显示 fallback UI（如 "Something went wrong"），tray 和 settings 正常工作
