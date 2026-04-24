## ADDED Requirements

### Requirement: 检查更新
应用 SHALL 通过 `tauri-plugin-updater` 检查 GitHub Releases 上是否有新版本。

#### Scenario: 无新版本
- **WHEN** 用户点击"检查更新"按钮且当前已是最新版本
- **THEN** 显示"已是最新版本"提示

#### Scenario: 有新版本
- **WHEN** 用户点击"检查更新"按钮且存在新版本
- **THEN** 显示新版本号，并提供"下载更新"和"稍后"两个操作

#### Scenario: 检查失败
- **WHEN** 检查更新时网络不可用或请求失败
- **THEN** 显示错误信息，用户可重试

### Requirement: 下载并安装更新
应用 SHALL 支持在应用内下载更新包并安装。

#### Scenario: 下载进度
- **WHEN** 用户点击"下载更新"
- **THEN** 显示下载进度百分比

#### Scenario: 下载完成
- **WHEN** 更新包下载并安装就绪
- **THEN** 提示用户"立即重启"以应用更新

#### Scenario: 下载失败
- **WHEN** 下载过程中网络中断或发生错误
- **THEN** 显示错误信息，用户可重试

#### Scenario: 重启安装
- **WHEN** 用户点击"立即重启"
- **THEN** 应用重启并完成更新安装

### Requirement: Updater 插件配置
应用 SHALL 在 Tauri 配置中声明 updater 插件，包含公钥和 GitHub Releases endpoint。

#### Scenario: Updater 配置
- **WHEN** 应用构建
- **THEN** `tauri.conf.json` 包含 `plugins.updater` 配置段，含 `pubkey` 和 `endpoints`

### Requirement: CI 签名构建
Release workflow SHALL 使用签名密钥对构建产物进行签名，生成 `latest.json` manifest。

#### Scenario: 签名构建
- **WHEN** CI 执行 release 构建
- **THEN** 使用 `TAURI_SIGNING_PRIVATE_KEY` 对产物签名
- **AND** 生成 `latest.json` 上传到 GitHub Release
