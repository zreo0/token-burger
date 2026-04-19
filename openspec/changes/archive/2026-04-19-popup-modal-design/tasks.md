## 1. Popup 布局与窗口结构调整

- [x] 1.1 调整 `src-tauri/tauri.conf.json` 中 popup 窗口尺寸到新的菜单栏小窗范围
- [x] 1.2 重构 `src/pages/Popup/index.tsx` 的信息层级，整理为时间范围、顶部摘要、Burger、Top Models、状态区
- [x] 1.3 更新 `src/pages/Popup/index.css`，实现更符合菜单栏小窗的分区布局、留白和 macOS popover 视觉风格

## 2. Burger 语义与视觉重构

- [x] 2.1 重构 `src/components/Burger/index.tsx`，将四层顺序调整为 Input / Cache Create / Cache Read / Output
- [x] 2.2 重构 `src/components/Burger/BurgerLayer.tsx`，支持层内展示名称与数值，并区分面包层与缓存层的视觉角色
- [x] 2.3 更新 `src/components/Burger/index.css`，移除旧的高饱和渐变风格，改为更克制的主题化层级样式
- [x] 2.4 实现缓存层 1 到 3 行文本高度的非线性厚度映射，并固定 Input / Output 为约 1.5 行文本高度

## 3. 动画与数据展示细化

- [x] 3.1 为 Today 实时更新实现细致、连续且不扰动布局的数字与层高动画
- [x] 3.2 为 7D/30D 切换实现快速、干脆的整体重组动画
- [x] 3.3 在 Popup 中新增 Top Models 展示逻辑，基于 `summary.by_model` 计算并渲染前 2 个模型
- [x] 3.4 调整冷启动状态展示为轻量状态区，避免遮挡主可视化结构

## 4. 验证与测试

- [x] 4.1 补充或更新前端测试，覆盖时间范围切换、Top Models 展示和零数据层级渲染
- [x] 4.2 验证 Today 实时更新、7D/30D 切换、深浅色模式和窗口尺寸调整后的视觉稳定性
