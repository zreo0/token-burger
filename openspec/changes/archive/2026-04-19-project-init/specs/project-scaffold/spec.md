## ADDED Requirements

### Requirement: Tauri v2 项目初始化
系统 SHALL 使用 Tauri v2 创建完整的项目脚手架，包含 Rust 后端和 React/TypeScript/Vite 前端，执行 `npm install && cd src-tauri && cargo build` 后 SHALL 编译通过且无错误。

#### Scenario: 前端依赖安装与构建
- **WHEN** 在项目根目录执行 `npm install && npm run build`
- **THEN** 构建成功，产出 `dist/` 目录包含有效的 HTML/JS/CSS 文件

#### Scenario: Rust 后端编译
- **WHEN** 在 `src-tauri/` 目录执行 `cargo build`
- **THEN** 编译成功且无 error（允许 warning）

#### Scenario: Tauri 开发模式启动
- **WHEN** 在项目根目录执行 `npm run tauri dev`
- **THEN** 应用窗口正常打开，前端页面可渲染

### Requirement: package.json 配置
项目根目录 SHALL 包含 `package.json`，定义项目名称、版本、脚本命令和所有前端依赖。

#### Scenario: 必要的 npm scripts
- **WHEN** 查看 `package.json` 的 `scripts` 字段
- **THEN** SHALL 包含以下命令：`dev`（Vite 开发服务器）、`build`（Vite 生产构建）、`preview`（Vite 预览）、`tauri`（Tauri CLI）、`lint`（ESLint 检查）、`lint:fix`（ESLint 自动修复）、`test`（vitest 运行）

#### Scenario: 前端核心依赖
- **WHEN** 查看 `package.json` 的 `dependencies`
- **THEN** SHALL 包含：`react`（^18）、`react-dom`（^18）、`react-router-dom`、`framer-motion`、`@tauri-apps/api`（v2）、`@tauri-apps/plugin-shell`

#### Scenario: 开发依赖
- **WHEN** 查看 `package.json` 的 `devDependencies`
- **THEN** SHALL 包含：`typescript`、`vite`、`@vitejs/plugin-react`、`tailwindcss`、`@tailwindcss/vite`、`eslint`、`@typescript-eslint/eslint-plugin`、`@typescript-eslint/parser`、`eslint-plugin-react-hooks`、`vitest`、`@tauri-apps/cli`（v2）、`@types/react`、`@types/react-dom`

### Requirement: Cargo.toml 配置
`src-tauri/Cargo.toml` SHALL 定义 Rust 后端的包信息和依赖。

#### Scenario: Rust 核心依赖
- **WHEN** 查看 `Cargo.toml` 的 `[dependencies]`
- **THEN** SHALL 包含：`tauri`（v2，features 含 tray-icon）、`rusqlite`（features 含 bundled）、`notify`、`serde`（features 含 derive）、`serde_json`、`tokio`（features 含 full）、`chrono`

### Requirement: Vite 配置
项目根目录 SHALL 包含 `vite.config.ts`，配置 React 插件和 Tauri 开发服务器集成。

#### Scenario: Vite 配置内容
- **WHEN** 查看 `vite.config.ts`
- **THEN** SHALL 配置 `@vitejs/plugin-react` 插件、`@tailwindcss/vite` 插件、`server.strictPort = true`、以及 Tauri 开发环境所需的端口和 host 设置

### Requirement: TypeScript 配置
项目根目录 SHALL 包含 `tsconfig.json` 和 `tsconfig.node.json`。

#### Scenario: tsconfig.json 核心配置
- **WHEN** 查看 `tsconfig.json`
- **THEN** SHALL 设置 `strict: true`、`jsx: "react-jsx"`、`target: "ES2021"`、`module: "ESNext"`、`moduleResolution: "bundler"`、`baseUrl: "./"` 并配置 `@/*` 路径别名指向 `src/*`

### Requirement: Tauri 配置
`src-tauri/tauri.conf.json` SHALL 定义应用窗口、安全权限和构建配置。

#### Scenario: 安全配置
- **WHEN** 查看 `tauri.conf.json`
- **THEN** SHALL 禁用 macOS App Sandbox（`"sandbox": false`），配置 capabilities 允许 tray-icon 和 shell 插件

#### Scenario: 窗口配置
- **WHEN** 查看 `tauri.conf.json` 的窗口定义
- **THEN** SHALL 定义 popup 窗口（无边框、不可调整大小、初始隐藏）和 settings 窗口（有标题栏、可调整大小、初始隐藏）

### Requirement: ESLint Flat Config
项目根目录 SHALL 包含 `eslint.config.js`，使用 ESLint 扁平化配置格式。

#### Scenario: 核心规则
- **WHEN** 查看 `eslint.config.js` 的规则定义
- **THEN** SHALL 强制：4 空格缩进、必须分号、单引号、React Hooks 规则（rules-of-hooks error、exhaustive-deps warn）

#### Scenario: 无 Prettier
- **WHEN** 检查项目依赖和配置文件
- **THEN** SHALL 不存在任何 prettier 相关的依赖、配置文件或 ESLint 插件

#### Scenario: Lint 通过
- **WHEN** 对项目骨架代码执行 `npm run lint`
- **THEN** SHALL 无 error 输出

### Requirement: 前端目录骨架
`src/` 目录 SHALL 按照 DESIGN.md 第 5 节的规范创建目录结构。

#### Scenario: 目录结构
- **WHEN** 查看 `src/` 目录
- **THEN** SHALL 包含：`components/`、`hooks/`、`pages/`（含 `Popup/` 和 `Setting/` 子目录）、`types/`、`App.tsx`、`main.tsx`

#### Scenario: 组件目录约定
- **WHEN** 查看任意组件或页面目录
- **THEN** 每个组件/页面 SHALL 是一个独立目录，包含 `index.tsx` 作为入口

### Requirement: Rust 后端目录骨架
`src-tauri/src/` 目录 SHALL 按照 DESIGN.md 第 5 节的规范创建目录结构。

#### Scenario: 目录结构
- **WHEN** 查看 `src-tauri/src/` 目录
- **THEN** SHALL 包含：`main.rs`（或 `lib.rs` + `main.rs`）、`db/`（含 `mod.rs`）、`watcher/`（含 `mod.rs`）、`adapters/`（含 `mod.rs`）

### Requirement: 测试基础设施
项目 SHALL 配置前端和 Rust 端的测试框架，使测试命令可立即执行。

#### Scenario: 前端测试框架
- **WHEN** 执行 `npm run test`
- **THEN** vitest 正常启动并执行（即使当前无测试文件，也不应报配置错误）

#### Scenario: Rust 测试框架
- **WHEN** 在 `src-tauri/` 执行 `cargo test`
- **THEN** cargo test 正常执行（即使当前无测试用例）

#### Scenario: 前端测试目录约定
- **WHEN** 需要为某个模块编写测试
- **THEN** SHALL 在该模块目录下创建 `__test__/` 子目录，测试文件命名为 `*.test.ts` 或 `*.test.tsx`

### Requirement: .gitignore 配置
项目根目录 SHALL 包含 `.gitignore`，排除构建产物、依赖目录和系统文件。

#### Scenario: 忽略规则
- **WHEN** 查看 `.gitignore`
- **THEN** SHALL 包含：`node_modules/`、`dist/`、`src-tauri/target/`、`.DS_Store`、`*.log`
