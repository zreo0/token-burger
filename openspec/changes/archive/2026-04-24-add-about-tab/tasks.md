## 1. 依赖安装

- [x] 1.1 安装前端依赖：`npm install @tauri-apps/plugin-updater @tauri-apps/plugin-opener`
- [x] 1.2 安装 Rust 依赖：在 `src-tauri/` 下 `cargo add tauri-plugin-updater tauri-plugin-opener`

## 2. Tauri 配置

- [x] 2.1 `src-tauri/tauri.conf.json` 添加 `plugins.updater` 配置段（pubkey 占位符 + GitHub Releases endpoint）
- [x] 2.2 `src-tauri/capabilities/default.json` 添加 updater 和 opener 权限（`updater:default`、`opener:default`）
- [x] 2.3 `src-tauri/src/lib.rs` 注册 `tauri_plugin_updater` 和 `tauri_plugin_opener` 插件

## 3. 前端 - About Tab UI

- [x] 3.1 `src/pages/Settings/index.tsx` 扩展 `Tab` 类型为 `'general' | 'agents' | 'data' | 'about'`，tabs 数组新增 `about`
- [x] 3.2 实现 About Tab 内容：版本号展示（`getVersion()`）、GitHub 链接（`openUrl()`）、检查更新按钮
- [x] 3.3 实现更新状态机：idle → checking → no-update / update-available → downloading(progress) → ready-to-restart / error
- [x] 3.4 `src/pages/Settings/index.css` 添加 About Tab 相关样式（进度条、状态文字、链接样式），含暗色模式适配

## 4. i18n

- [x] 4.1 `src/i18n/locales/en.json` 添加 about 相关翻译 key（about、version、github、checkUpdate、checking、upToDate、newVersion、download、later、downloading、restart、error、retry）
- [x] 4.2 `src/i18n/locales/zh-CN.json` 添加对应中文翻译

## 5. CI 配置

- [x] 5.1 `.github/workflows/release.yml` 取消注释 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 环境变量
