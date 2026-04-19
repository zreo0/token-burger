## Why

当前 Popup 数据弹窗虽然已具备实时 token 展示能力，但视觉层级偏弱、数据表达不够直观，且现有 Burger 结构与 TokenBurger 的主题隐喻没有形成稳定、易读的设计语言。随着后续继续在弹窗中加入更多统计信息，需要先重构 Popup 的信息架构、动画节奏和 Burger 视觉语义，避免小窗继续堆叠而变得拥挤和失衡。

## What Changes

- 重新设计 Popup 小窗的信息层级，将总 token、预估花费、Burger 主可视化、Top Models 和冷启动状态组织为更适合菜单栏小窗的结构。
- 调整 Burger 的语义映射：底部面包固定表示 Input，顶部面包固定表示 Output，中间两层依次表示 Cache Create 和 Cache Read，并通过中间层厚度表达缓存参与度与“汉堡丰满程度”。
- 将各层数值直接展示在对应层内，采用带上下限的非线性厚度映射，保证薄层和厚层都保持可读性与稳定形态。
- 更新 Popup 动画策略：Today 实时更新使用细致、平滑、不打扰操作的连续动画；7D/30D 切换使用干脆、快速、不拖沓的重组动画。
- 在 Popup 中补充 Top Models 轻量榜单展示，为后续扩展更多统计信息预留空间，同时保持菜单栏应用应有的紧凑与克制。

## Capabilities

### New Capabilities
<!-- None -->

### Modified Capabilities
- `burger-ui`: 调整 Popup 布局、Burger 层级语义、层内数值展示、厚度映射、Top Models 展示和动效节奏要求。

## Impact

- Affected specs: `openspec/specs/burger-ui/spec.md`
- Affected frontend: `src/pages/Popup/index.tsx`, `src/pages/Popup/index.css`, `src/components/Burger/index.tsx`, `src/components/Burger/index.css`, `src/components/Burger/BurgerLayer.tsx`
- Affected configuration: `src-tauri/tauri.conf.json`（Popup 窗口尺寸可能需要调整）
- Existing data contracts can be reused: `TokenSummary`, `summary.by_model`, pricing calculation and `cold-start-progress` event
