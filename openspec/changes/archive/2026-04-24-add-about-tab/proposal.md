## Why

Settings 页面缺少应用信息展示和版本更新能力。用户无法在应用内查看当前版本、访问项目主页，也无法检查和安装新版本。需要新增"关于"Tab，集成 Tauri Updater 实现完整的应用内更新链路。

## What Changes

- Settings 页面新增 `About` Tab，展示版本号、GitHub 链接、检查更新按钮
- 集成 `tauri-plugin-updater` 实现应用内检查更新、下载、安装完整链路
- 集成 `tauri-plugin-opener` 实现点击 GitHub 链接在系统浏览器中打开
- `tauri.conf.json` 添加 updater 插件配置（endpoint + 公钥）
- CI release workflow 启用签名环境变量
- 更新状态机：idle → checking → no-update / update-available → downloading(progress) → ready-to-restart / error

## Capabilities

### New Capabilities
- `app-updater`: 应用内检查更新、下载更新包、安装更新并重启的完整更新链路
- `about-tab`: Settings 页面"关于"Tab 的 UI 展示（版本号、GitHub 链接、更新操作）

### Modified Capabilities
- `settings-ui`: 导航 Tab 从 3 个扩展为 4 个（新增 About），底部 footer 保持不变

## Impact

- **依赖新增**: `@tauri-apps/plugin-updater`（npm + cargo）、`@tauri-apps/plugin-opener`（npm + cargo）
- **配置变更**: `tauri.conf.json` 添加 `plugins.updater` 配置段；capabilities 添加 updater 和 opener 权限
- **CI 变更**: `release.yml` 启用 `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 环境变量
- **前端变更**: `Settings/index.tsx` 扩展 Tab 类型和 about 内容；i18n 新增翻译 key
- **用户操作**: 需手动生成签名密钥对（`tauri signer generate`），公钥写入配置，私钥配置到 GitHub Secrets
