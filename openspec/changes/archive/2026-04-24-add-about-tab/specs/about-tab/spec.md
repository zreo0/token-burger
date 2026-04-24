## ADDED Requirements

### Requirement: About Tab 展示
Settings 页面 SHALL 包含 About Tab，展示应用版本号和 GitHub 项目链接。

#### Scenario: 显示版本号
- **WHEN** 用户切换到 About Tab
- **THEN** 显示当前应用版本号（通过 `@tauri-apps/api/app` 的 `getVersion()` 获取）

#### Scenario: GitHub 链接
- **WHEN** 用户点击 GitHub 链接
- **THEN** 使用 `tauri-plugin-opener` 在系统默认浏览器中打开 `https://github.com/zreo0/token-burger`

### Requirement: 检查更新 UI
About Tab SHALL 提供检查更新按钮，并根据更新状态展示不同 UI。

#### Scenario: 空闲状态
- **WHEN** 用户进入 About Tab 且未触发更新检查
- **THEN** 显示"检查更新"按钮

#### Scenario: 检查中
- **WHEN** 用户点击"检查更新"
- **THEN** 按钮显示加载状态（如 "检查中..."），不可重复点击

#### Scenario: 已是最新
- **WHEN** 检查结果为无新版本
- **THEN** 显示"已是最新版本"提示，3 秒后恢复为"检查更新"按钮

#### Scenario: 发现新版本
- **WHEN** 检查结果为有新版本
- **THEN** 显示新版本号，提供"下载更新"和"稍后"按钮

#### Scenario: 下载中
- **WHEN** 用户点击"下载更新"
- **THEN** 显示下载进度条和百分比

#### Scenario: 准备重启
- **WHEN** 下载完成
- **THEN** 显示"立即重启"按钮

#### Scenario: 错误状态
- **WHEN** 检查或下载过程中发生错误
- **THEN** 内联显示错误信息，提供"重试"按钮

### Requirement: About Tab i18n
About Tab 的所有文本 SHALL 通过 `react-i18next` 国际化，支持英文和简体中文。

#### Scenario: 中文 About
- **WHEN** 语言为 zh-CN
- **THEN** 所有 UI 文本显示中文（如"关于"、"检查更新"、"已是最新版本"）
