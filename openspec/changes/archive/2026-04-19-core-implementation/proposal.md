## Why

项目脚手架已完成（Tauri v2 + React + Vite + ESLint + 测试基础设施），但所有业务模块仍为空占位。当前 Rust 端的 `db/mod.rs`、`watcher/mod.rs` 只有 TODO 注释，Adapter 仅有 Trait 定义无实现，前端 Popup/Settings 页面为空壳，tray title 写死 "0"。需要实现 DESIGN.md 定义的完整数据流闭环：日志监听 → 解析 → 入库 → 事件广播 → UI 渲染。

## What Changes

- 实现 SQLite 数据库层：Schema 初始化（WAL + token_logs + file_offsets）、连接管理、CRUD 封装、dev/prod 环境隔离
- 实现 6 个 Agent Adapter：Claude Code、Codex、Gemini CLI、Copilot、OpenCode（新/旧）的日志解析
- 实现 Watcher 引擎：notify + debounce（JSONL）、定时轮询 + mtime（JSON）、定时查询（SQLite）、offset 断点续传、文件轮转检测
- 实现冷启动流程：后台线程逐 Agent 解析、30 天 mtime 过滤、批量事务、进度广播
- 定义 Tauri IPC 层：commands + `token-updated` 事件，连接 Rust 后端与 React 前端
- 实现费率引擎：LiteLLM 远程拉取 + 按天本地缓存 + 内置 fallback + 模型名匹配
- 实现 Burger 四层动画组件：Framer Motion 弹簧动画 + 数字滚动 + 厚度动态变化
- 实现 Popup 页面：Burger 组件 + 时间范围 + 预估花费
- 实现 useTokenStream hook：Tauri 事件监听 + React 状态管理
- 实现 Settings 页面：Agent 开关、监控模式切换、数据清理配置
- 实现 tray 动态更新：token 总量格式化（K/M/B）+ `set_title()` 实时刷新

## Capabilities

### New Capabilities

- `sqlite-database`: SQLite 数据库层——Schema 初始化、WAL 模式、连接管理、token_logs/file_offsets CRUD、dev/prod 文件隔离
- `agent-adapters`: 6 个 Agent 的日志解析实现——Claude Code (JSONL)、Codex (JSONL)、Gemini CLI (JSON)、Copilot (JSON)、OpenCode 新版 (SQLite)、OpenCode 旧版 (JSON fallback)
- `watcher-engine`: 数据流引擎——三种监听策略（notify+debounce / 轮询+mtime / 定时查询）、offset 断点续传、文件轮转检测、冷启动编排、事件广播
- `tauri-bridge`: Tauri IPC 层——commands 定义（查询 token 汇总、获取 agent 列表、读写设置、获取价格表等）、`token-updated` / `cold-start-progress` 事件协议、tray title 动态更新
- `pricing`: 费率引擎——LiteLLM 远程拉取、按天本地缓存、内置 fallback 价格表、模型名匹配策略（精确→归一化→子串→$0）、前端金额计算
- `burger-ui`: Popup 页面与 Burger 组件——四层汉堡动画（Framer Motion spring）、数字滚动效果、层厚度动态变化、时间范围选择、预估花费展示、useTokenStream hook
- `settings-ui`: Settings 页面——Agent 检测与 Toggle 开关、监控模式切换（实时/轮询）、数据保留天数配置、清空历史数据

### Modified Capabilities

（无已有 spec 需要修改）

## Impact

- **Rust 端**：`src-tauri/src/` 下 `db/`、`watcher/`、`adapters/` 三个模块从 TODO 占位变为完整实现；`lib.rs` 需注册 Tauri commands 和启动 watcher；新增 `pricing/` 模块
- **前端**：`src/` 下新增 `components/Burger/`、`hooks/useTokenStream.ts`、`utils/`（格式化、费用计算）；Popup 和 Settings 页面从空壳变为完整 UI
- **依赖**：Rust 端可能新增 `reqwest`（HTTP 拉取价格表）、`glob`（路径匹配）；前端无新依赖（Framer Motion 已在 package.json）
- **Tauri 配置**：`src-tauri/capabilities/` 可能需要声明 fs/http 权限
- **测试**：每个 Rust 模块需内联 `#[cfg(test)]` 单元测试；前端 hooks/utils 需 vitest 测试
