## MODIFIED Requirements

### Requirement: Settings 窗口视觉风格
Settings 窗口 SHALL 呈现接近 macOS 偏好设置窗口的视觉层级：保留原生标准窗口装饰，使用顶部导航在 `General`、`Agents`、`Data`、`About` 之间切换；内容区 MUST 采用分组式卡片（grouped / inset grouped）组织设置项，并在亮色与暗色模式下保持一致的层级、间距、文本权重和状态色。页面中的 Toggle、Segmented Control、Select、数字输入和按钮 SHALL 采用统一的 macOS 风格语义；危险操作 MUST 通过破坏性文案、确认步骤和克制的视觉强调表达，而不是以大面积警告样式主导页面。

#### Scenario: 导航结构
- **WHEN** 用户打开 Settings
- **THEN** 窗口顶部显示 `General`、`Agents`、`Data`、`About` 四个分区导航
- **AND** 导航在视觉上表现为居中且非全宽的 segmented control，而不是铺满整个内容宽度
- **AND** 当前分区有明确的激活态
