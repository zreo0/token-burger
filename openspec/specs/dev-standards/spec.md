# Purpose
定义项目级开发文档、协作约定和工程规范。

## Requirements

### Requirement: AGENTS.md 文档
项目根目录 SHALL 包含 `AGENTS.md`，定义项目的技术栈、代码规范和开发约定，格式与现有项目保持一致。

#### Scenario: Tech Stack 章节
- **WHEN** 查看 AGENTS.md 的 Tech Stack 部分
- **THEN** SHALL 列出：Tauri v2（Rust + Web Frontend）、React 18 + TypeScript、Vite、Tailwind CSS、Framer Motion、rusqlite、notify、serde

#### Scenario: Code Style 章节
- **WHEN** 查看 AGENTS.md 的 Code Style 部分
- **THEN** SHALL 定义：4 空格缩进、单引号、必须分号、ESLint flat config（无 Prettier）、中文注释

#### Scenario: 命名规范
- **WHEN** 查看 AGENTS.md 的命名规范部分
- **THEN** SHALL 分别定义 TypeScript 和 Rust 的命名约定：
  - TypeScript：组件 PascalCase 目录、变量/函数 camelCase、常量 UPPER_SNAKE_CASE、文件 kebab-case（组件除外）
  - Rust：文件/变量/函数 snake_case、类型/Trait PascalCase、常量 UPPER_SNAKE_CASE

#### Scenario: Architecture Patterns 章节
- **WHEN** 查看 AGENTS.md 的 Architecture Patterns 部分
- **THEN** SHALL 描述：前后端分层（Rust 系统级操作 + React UI 渲染）、适配器模式（AgentAdapter Trait）、事件驱动（Tauri emit/listen）

#### Scenario: Git Workflow 章节
- **WHEN** 查看 AGENTS.md 的 Git Workflow 部分
- **THEN** SHALL 定义：主分支 `main`、功能分支 `feat/{feature-name}`、commit 风格使用 `feat:`、`fix:`、`wip:`、`chore:`、`docs:`、`style:`、`refactor:`、`perf:`、`test:` 前缀

#### Scenario: 测试约定章节
- **WHEN** 查看 AGENTS.md 的测试约定部分
- **THEN** SHALL 定义：前端使用 vitest + `__test__/` 目录、Rust 使用 `#[cfg(test)]` 内置测试、测试文件命名 `*.test.ts(x)` / `*.spec.rs`

#### Scenario: Project Conventions 章节
- **WHEN** 查看 AGENTS.md 的 Project Conventions 部分
- **THEN** SHALL 包含"如无必要，勿增实体。保持改动最小化，不要改动和需求无关的代码。"

### Requirement: CLAUDE.md 文档
项目根目录 SHALL 包含 `CLAUDE.md`，内容为对 AGENTS.md 的引用。

#### Scenario: CLAUDE.md 内容
- **WHEN** 查看 `CLAUDE.md`
- **THEN** 内容 SHALL 为 `@AGENTS.md`

### Requirement: README.md 文档
项目根目录 SHALL 包含 `README.md`，提供项目简介和开发指南。

#### Scenario: 项目简介
- **WHEN** 查看 README.md 的开头
- **THEN** SHALL 包含项目名称（TokenBurger）和一句话描述其功能（macOS 菜单栏 AI Token 消耗监控工具）

#### Scenario: 前置依赖说明
- **WHEN** 查看 README.md 的前置依赖部分
- **THEN** SHALL 列出：Rust（含最低版本）、Node.js（含最低版本）、Tauri CLI v2 的安装方式

#### Scenario: 安装与运行
- **WHEN** 查看 README.md 的运行部分
- **THEN** SHALL 包含 `npm install`、`npm run tauri dev`（开发模式）、`npm run tauri build`（生产构建）的命令说明

#### Scenario: 项目结构
- **WHEN** 查看 README.md 的项目结构部分
- **THEN** SHALL 包含简要的目录树，标注 `src/`（前端）和 `src-tauri/`（Rust 后端）的职责

#### Scenario: 开发命令速查
- **WHEN** 查看 README.md 的开发命令部分
- **THEN** SHALL 以表格或列表形式列出常用命令：`npm run dev`、`npm run build`、`npm run lint`、`npm run test`、`cargo test`（在 src-tauri 下）
