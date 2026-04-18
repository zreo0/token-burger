# AGENTS.md

## Tech Stack

- **应用框架**: Tauri v2 (Rust + Web Frontend)
- **后端**: Rust (rusqlite, notify, serde, tokio, chrono)
- **前端**: React 18 + TypeScript
- **构建工具**: Vite
- **样式**: Tailwind CSS
- **动画**: Framer Motion
- **测试**: vitest (前端) / cargo test (Rust)

## Project Conventions

- 如无必要，勿增实体。保持改动最小化，不要改动和需求无关的代码。
- 中文注释。
- 禁止引入 Prettier。

## Code Style

- 缩进：4 空格
- 引号：单引号 (TypeScript)
- 分号：必须 (TypeScript)
- ESLint：flat config 格式，无 Prettier
- Rust：`cargo fmt` 默认风格

## 命名规范

### TypeScript
- 组件目录：PascalCase（`Burger/`、`Popup/`）
- 变量/函数：camelCase
- 常量：UPPER_SNAKE_CASE
- 文件：kebab-case（组件目录除外）

### Rust
- 文件/变量/函数：snake_case
- 类型/Trait：PascalCase
- 常量：UPPER_SNAKE_CASE

## Architecture Patterns

- **前后端分层**: Rust 负责系统级操作（文件监听、SQLite、Tray），React 负责 UI 渲染
- **适配器模式**: AgentAdapter Trait 统一不同 Agent 的日志解析
- **事件驱动**: Tauri emit/listen 实现前后端实时通信

## Git Workflow

**分支策略: Trunk Based Development**
- 主要在 `main` 分支开发
- 功能分支：`feat/{feature-name}`

**提交规范:**
- 格式: `type: description`
- 常用类型:
  - `feat:` - 新功能
  - `fix:` - Bug 修复
  - `wip:` - 进行中的工作
  - `chore:` - 构建/工具变更
  - `docs:` - 文档更新
  - `style:` - 代码格式(不影响功能)
  - `refactor:` - 重构
  - `perf:` - 性能优化
  - `test:` - 测试相关
- 示例: `feat: add pricing engine with caching`

## 测试约定

### 前端
- 框架：vitest
- 目录：`__test__/` 放在对应模块目录下
- 命名：`*.test.ts` / `*.test.tsx`

### Rust
- 框架：cargo test（内置）
- 约定：`#[cfg(test)] mod tests` 写在同文件底部
- 集成测试：`src-tauri/tests/`
