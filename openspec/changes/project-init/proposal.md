## Why

TokenBurger 项目目前只有 DESIGN.md 设计文档，没有任何可运行的代码、项目配置或开发规范。需要完成项目初始化（Tauri v2 + React + TypeScript 脚手架）、补充设计文档中发现的缺陷和遗漏、建立代码规范（AGENTS.md）和项目说明（README.md），为后续功能实现打下可靠基础。

## What Changes

- 初始化 Tauri v2 项目脚手架（Rust 后端 + React/TypeScript/Vite 前端）
- 创建 ESLint flat config（4 空格缩进、单引号、必须分号、无 Prettier）
- 创建 AGENTS.md 定义项目技术栈、代码风格、架构模式、Git 规范
- 创建 README.md 包含项目简介、前置依赖、安装运行、目录结构说明
- 补充 DESIGN.md：修复 Schema 索引冲突、明确 Tauri v2、补充 Agent 日志格式、费率方案、WAL 模式、冷启动策略、Adapter Trait 重设计、测试策略、错误处理
- 配置前端测试框架（vitest），约定前端 `__test__/` 目录、Rust 端 `#[cfg(test)]` 内置测试

## Capabilities

### New Capabilities

- `project-scaffold`: Tauri v2 项目脚手架初始化，包括 package.json、Cargo.toml、vite.config.ts、tsconfig.json、tauri.conf.json、ESLint flat config、.editorconfig、.gitignore 等基础配置文件
- `dev-standards`: 开发规范文档（AGENTS.md + README.md），定义技术栈、代码风格、架构模式、命名规范、Git 工作流、测试约定
- `design-supplement`: DESIGN.md 补充章节，涵盖 Agent 日志格式定义、多数据源 Adapter Trait 重设计、费率方案（LiteLLM 远程 + 本地缓存）、SQLite WAL 模式、冷启动策略、错误处理策略、测试策略，以及修复 idx_request_dedup 索引冲突

### Modified Capabilities

（无现有 spec）

## Impact

- 项目根目录：新增 package.json、eslint.config.js、vite.config.ts、tsconfig.json、AGENTS.md、README.md、.gitignore
- src/ 目录：创建前端目录骨架（components/、hooks/、pages/、types/）
- src-tauri/ 目录：创建 Rust 后端目录骨架（Cargo.toml、tauri.conf.json、src/main.rs、db/、watcher/、adapters/）
- DESIGN.md：补充第 7-12 节（Agent 日志格式、Adapter 重设计、费率方案、WAL、冷启动、错误处理、测试策略），修复第 4.1 节索引冲突
- 依赖引入：Tauri v2 CLI、React 18、TypeScript、Vite、Tailwind CSS、Framer Motion、React Router、vitest、@typescript-eslint、rusqlite、notify、serde、tokio
