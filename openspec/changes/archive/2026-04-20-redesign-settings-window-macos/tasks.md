## 1. Settings 页面结构整理

- [x] 1.1 调整 `src/pages/Settings/index.tsx` 的结构，使顶部导航收敛为居中 segmented control，并把 General / Agents / Data 内容整理为 grouped section / card 结构
- [x] 1.2 重排 Agents 与 Data 分区的行布局和操作层级，使状态信息、开关、保留天数与清理操作符合新的分组语义

## 2. macOS 风格样式重构

- [x] 2.1 重写 `src/pages/Settings/index.css` 中与导航、卡片、分隔线、间距、字体层级相关的样式，建立更接近 macOS Preferences Window 的视觉基线
- [x] 2.2 统一 Toggle、Segmented Control、Select、数字输入、普通按钮和危险操作按钮的样式语义，并同步完善亮色/暗色模式表现

## 3. 窗口层协调与验证

- [x] 3.1 评估并微调 `src-tauri/src/lib.rs` 中 settings 窗口的尺寸或相关配置，使窗口比例与页面内容更协调，同时保留原生窗口装饰
- [x] 3.2 完成 Settings 窗口的联调验证：检查三个分区切换、危险操作确认、亮暗色模式观感，并运行相关前端校验/测试
