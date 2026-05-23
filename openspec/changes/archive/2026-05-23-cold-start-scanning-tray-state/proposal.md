## Why

冷启动扫描大量历史数据时，当前菜单栏可能显示 `0` 或部分统计，用户容易误以为数据已完成加载。需要在扫描结束前明确进入“扫描中”状态，并避免用户打开内容尚不可信的 Popup。

## What Changes

- 主托盘图标创建时，token title 初始显示本地化的扫描中状态（英文 `Scanning...`，中文 `扫描中...`），而不是 `0`。
- 冷启动扫描完成前，左键点击主托盘图标不展示 Popup；右键菜单与设置入口保持可用。
- 冷启动完成后，主托盘恢复展示真实 token 汇总，并允许左键打开 Popup。
- Provider usage 的菜单栏显示逻辑不纳入本次改动范围。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `watcher-engine`: 冷启动生命周期需要提供可靠的完成状态，用于驱动主托盘从扫描态切换到正常 token 展示态。
- `tauri-bridge`: 主托盘 title 初始扫描态、完成后 token title 更新、冷启动期间左键点击门控的行为契约发生变化。
- `burger-ui`: 冷启动期间不再通过用户点击主托盘展示 Popup 内容，避免展示未完成数据。

## Impact

- 影响 Rust 侧启动编排、Watcher 冷启动完成通知、主托盘 title/tooltip/点击处理逻辑。
- 影响 i18n 文案来源，需复用启动时读取的语言设置或同等后端本地化逻辑。
- 影响现有 Popup 冷启动提示相关前端逻辑的可达性，但不要求处理账号用量 Provider usage。
- 不引入新的第三方依赖，不改变 token 数据库结构。
