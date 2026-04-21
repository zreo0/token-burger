## Why

当前 Settings 页面虽然功能完整，但整体观感仍偏普通 Web 表单：分组层级较弱、控件风格不统一、暗色模式材质感不足，且窗口层与内容层没有形成 macOS 偏好设置窗口应有的一致体验。随着设置项逐步稳定，现阶段适合把视觉与交互基线提升到更接近 macOS 原生偏好设置的水平。

## What Changes

- 重构 Settings 窗口的视觉层级，使其更接近 macOS Preferences Window：顶部导航更克制，内容区采用分组卡片与清晰的区块层次。
- 优化 General、Agents、Data 三个分区的布局、留白、分隔线和对齐方式，减少当前单页堆叠感。
- 调整 Toggle、Segmented Control、Select、按钮和危险操作区的样式语义，使控件更符合 macOS 风格。
- 提升亮色/暗色模式的一致性，建立更贴近 macOS 的背景、材质、文本层级和状态色。
- 统一 Settings 窗口本身与页面内容的视觉表达，避免窗口 chrome 与页面内容风格割裂。

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `settings-ui`: 调整 Settings 窗口的视觉与交互要求，使其更明确地约束 macOS 风格、窗口层级、分组布局、控件语义和危险操作呈现。

## Impact

- Affected code: `src/pages/Settings/index.tsx`, `src/pages/Settings/index.css`, `src-tauri/src/lib.rs`, 以及相关 i18n 文案文件（如需补充说明文本）。
- Affected systems: Settings 前端 UI、Tauri settings 窗口创建配置、亮暗色主题表现。
- No API contract changes expected.
