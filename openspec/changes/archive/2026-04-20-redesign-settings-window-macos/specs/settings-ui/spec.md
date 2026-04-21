# Purpose
收紧 Settings 窗口在 macOS 风格下的视觉与交互要求，使导航、分组、控件和危险操作表达具备一致的偏好设置窗口体验。

## MODIFIED Requirements

### Requirement: Settings 窗口视觉风格
Settings 窗口 SHALL 呈现接近 macOS 偏好设置窗口的视觉层级：保留原生标准窗口装饰，使用顶部导航在 `General`、`Agents`、`Data` 之间切换；内容区 MUST 采用分组式卡片（grouped / inset grouped）组织设置项，并在亮色与暗色模式下保持一致的层级、间距、文本权重和状态色。页面中的 Toggle、Segmented Control、Select、数字输入和按钮 SHALL 采用统一的 macOS 风格语义；危险操作 MUST 通过破坏性文案、确认步骤和克制的视觉强调表达，而不是以大面积警告样式主导页面。

#### Scenario: 导航结构
- **WHEN** 用户打开 Settings
- **THEN** 窗口顶部显示 `General`、`Agents`、`Data` 三个分区导航
- **AND** 导航在视觉上表现为居中且非全宽的 segmented control，而不是铺满整个内容宽度
- **AND** 当前分区有明确的激活态

#### Scenario: 分组内容层级
- **WHEN** 用户查看任一分区内容
- **THEN** 设置项以 grouped card 或 inset grouped list 形式组织，而不是直接裸露在页面背景上
- **AND** 同一卡片中的多行设置项通过分隔线、统一行高和一致留白形成层级

#### Scenario: 控件语义一致
- **WHEN** 用户查看语言选择、监控模式、Agent 开关、保留天数和操作按钮
- **THEN** 这些控件在尺寸、圆角、状态色、文本层级和交互反馈上保持统一的 macOS 风格语义
- **AND** 已启用的 Toggle 使用 macOS 语义的开启状态色，而不是通用 Web 强调蓝色

#### Scenario: 危险操作收敛表达
- **WHEN** 用户进入 Data 分区并查看清理操作
- **THEN** `清理旧数据` 和 `清空全部数据` 以普通操作与破坏性操作的层级区分呈现
- **AND** 破坏性操作不会以大面积高饱和警告块作为默认常驻主视觉
- **AND** 用户在执行 `清空全部数据` 前仍需经过明确确认

#### Scenario: 亮暗色模式一致性
- **WHEN** 系统在亮色与暗色模式之间切换
- **THEN** Settings 窗口的背景层、卡片层、文本主次层级、边框/分隔线和控件状态同步切换
- **AND** 暗色模式保持接近 macOS 的材质与对比关系，而不是简单反色
