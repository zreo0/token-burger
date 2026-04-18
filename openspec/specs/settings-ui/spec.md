# Purpose
定义 Settings 窗口中的配置项、交互和展示要求。

## Requirements

### Requirement: Agent 列表展示
Settings 页面 SHALL 展示所有支持的 Agent 列表，每个 Agent 显示名称、数据源类型、可用状态（日志路径是否存在）和 Toggle 开关。

#### Scenario: 展示 Agent 列表
- **WHEN** 用户打开 Settings 页面
- **THEN** 显示 5 个 Agent 的卡片/行，每个包含名称、状态指示器和 Toggle 开关

#### Scenario: Agent 不可用
- **WHEN** 某 Agent 的日志路径不存在
- **THEN** 该 Agent 显示为灰色/禁用状态，Toggle 不可操作，标注 `Not detected`

#### Scenario: 切换 Agent 开关
- **WHEN** 用户关闭 Claude Code 的 Toggle
- **THEN** 调用 `toggle_agent` command，WatcherEngine 停止监听 Claude Code

### Requirement: 监控模式切换
Settings 页面 SHALL 提供监控模式选择：Realtime（基于文件系统事件监听）和 Polling（定时轮询）。使用 Segmented Control 风格的切换控件。

#### Scenario: 切换到 Polling 模式
- **WHEN** 用户选择 Polling 模式
- **THEN** 调用 `update_settings` 更新 `watch_mode`，WatcherEngine 切换到轮询策略

#### Scenario: 显示当前模式
- **WHEN** Settings 页面加载
- **THEN** Segmented Control 高亮当前生效的模式

### Requirement: 数据保留天数配置
Settings 页面 SHALL 提供数据保留天数的输入控件（数字输入或预设选项），默认 30 天。

#### Scenario: 修改保留天数
- **WHEN** 用户将保留天数从 30 改为 7
- **THEN** 调用 `update_settings` 更新 `keep_days`

### Requirement: 数据清理操作
Settings 页面 SHALL 提供`清理旧数据`和`清空全部数据`两个操作按钮。清空全部数据前 MUST 显示确认对话框。

#### Scenario: 清理旧数据
- **WHEN** 用户点击`清理旧数据`
- **THEN** 调用 `clear_data({ keep_days })` 删除超出保留期的记录

#### Scenario: 清空全部数据需确认
- **WHEN** 用户点击`清空全部数据`
- **THEN** 弹出确认对话框，用户确认后调用 `clear_data({})`

### Requirement: 语言切换
Settings 页面 SHALL 提供语言选择下拉菜单，选项为 English 和 简体中文。切换后立即生效，无需重启。

#### Scenario: 切换到中文
- **WHEN** 用户选择`简体中文`
- **THEN** 调用 `update_settings` 更新 `language` 为 `"zh-CN"`，`i18next.changeLanguage('zh-CN')` 立即切换界面语言

#### Scenario: 默认语言检测
- **WHEN** 首次启动且无语言设置
- **THEN** 检测 `navigator.language`，匹配 `zh` 开头则使用中文，否则使用英文

### Requirement: Settings 窗口视觉风格
Settings 窗口 SHALL 采用 macOS 原生偏好设置面板风格：标准窗口边框 + 侧边栏或顶部 Tab 导航（General / Agents / Data）。表单控件使用 macOS 原生风格（Toggle、Select、Segmented Control）。支持暗色/亮色模式。

#### Scenario: 导航结构
- **WHEN** 用户打开 Settings
- **THEN** 显示分区导航（General: 语言、监控模式；Agents: Agent 列表和开关；Data: 保留天数、清理操作）

#### Scenario: 页面切换动画
- **WHEN** 用户切换导航 Tab
- **THEN** 内容区域使用 Framer Motion `AnimatePresence` + slide 过渡动画

### Requirement: 开发环境标识
Settings 页面底部 SHALL 在开发模式下显示环境标识（如 `DEV MODE`），生产模式下不显示。

#### Scenario: 开发模式
- **WHEN** 应用以 dev 模式运行
- **THEN** Settings 底部显示 `DEV MODE` 标识

#### Scenario: 生产模式
- **WHEN** 应用以生产模式运行
- **THEN** 不显示环境标识

### Requirement: Settings i18n
Settings 页面的所有标签、按钮文本、提示信息 SHALL 通过 `react-i18next` 国际化，支持英文和简体中文。

#### Scenario: 中文 Settings
- **WHEN** 语言为 zh-CN
- **THEN** 所有 UI 文本显示中文（如 `代理列表`、`监控模式`、`数据清理`）
