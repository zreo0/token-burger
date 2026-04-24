## Context

Settings 页面当前有 General / Agents / Data 三个 Tab。需要新增 About Tab 展示应用信息和更新功能。项目使用 Tauri v2 + React 18，已有 `reqwest` 依赖但无 updater 插件。Release workflow 使用 `tauri-action`，已支持多平台构建但未启用签名。

## Goals / Non-Goals

**Goals:**
- 在 Settings 页面新增 About Tab，展示版本号、GitHub 链接、检查更新入口
- 集成 Tauri Updater 插件实现完整的应用内更新链路（检查 → 下载 → 安装重启）
- 使用 `tauri-plugin-opener` 打开外部链接

**Non-Goals:**
- 不实现自动检查更新（仅用户手动触发）
- 不实现自建更新服务器（直接使用 GitHub Releases）
- 不实现下载断点续传
- 不修改现有 Tab 的功能

## Decisions

### D1: 更新插件选择 — `tauri-plugin-updater`
使用 Tauri 官方 updater 插件而非手动调用 GitHub API。
- 内置签名验证、下载、安装、重启全链路
- 前端 JS API 直接调用，无需自写 Rust command
- 替代方案：手动 reqwest + GitHub API → 只能跳转浏览器下载，无法应用内安装

### D2: 外部链接打开 — `tauri-plugin-opener`
使用 `tauri-plugin-opener` 而非 `tauri-plugin-shell`。
- opener 只暴露"打开 URL/文件"能力，权限面最小
- shell 插件可执行任意命令，对本需求过重
- 替代方案：自写 Rust command 调用 `open` crate → 多一个依赖，且 opener 已经跨平台

### D3: 更新 endpoint — GitHub Releases
使用 `https://github.com/zreo0/token-burger/releases/latest/download/latest.json` 作为 updater endpoint。
- `tauri-action` 构建时自动生成 `latest.json` manifest 文件并上传到 Release
- 公开仓库，无需 token
- 替代方案：自建服务器 → 增加运维成本，无必要

### D4: 版本号获取 — `@tauri-apps/api/app`
前端通过 `getVersion()` 获取版本号，来源是 `tauri.conf.json` 中的 `version` 字段。
- 无需额外 Rust command
- 单一版本源（tauri.conf.json）

### D5: 更新状态机设计
前端使用状态机管理更新流程 UI：

```
idle ──▶ checking ──▶ no-update (3秒后回到 idle)
                  ──▶ update-available ──▶ downloading(progress%) ──▶ ready-to-restart
                  ──▶ error (显示错误信息，可重试)
```

- `downloading` 状态携带进度百分比，通过 `update.downloadAndInstall(callback)` 的回调获取
- 错误统一在 About Tab 内以内联方式展示，不使用全局弹窗

### D6: Rust 端插件注册
在 `lib.rs`（或 `main.rs`）的 Tauri Builder 链中注册两个插件：

```rust
.plugin(tauri_plugin_updater::Builder::new().build())
.plugin(tauri_plugin_opener::init())
```

### D7: Capabilities 权限配置
在 `src-tauri/capabilities/` 下添加 updater 和 opener 的权限声明，确保前端 JS API 有权调用。

## Risks / Trade-offs

- [签名密钥管理] 私钥泄露会导致恶意更新包通过验证 → 私钥仅存于 GitHub Secrets，不入库
- [GitHub API 限流] 频繁检查更新可能触发 GitHub 限流 → 仅手动触发，非自动轮询，风险极低
- [下载中断] 网络不稳定导致下载失败 → updater 插件会抛错，前端捕获后显示错误信息并允许重试
- [首次配置成本] 需要手动生成密钥对并配置 GitHub Secrets → 一次性操作，文档化步骤
