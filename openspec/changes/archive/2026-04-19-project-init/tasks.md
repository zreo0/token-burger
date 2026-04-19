## 1. DESIGN.md 补充与修复

- [x] 1.1 修改第 2 节：将“Tauri (Rust + Web Frontend)”改为“Tauri v2 (Rust + Web Frontend)”，补充 v2 关键 API 说明（tray.set_title、capabilities 安全模型、插件系统）
- [x] 1.2 修复第 4.1 节：将 `CREATE UNIQUE INDEX idx_request_dedup` 改为 `CREATE INDEX idx_request_dedup`
- [x] 1.3 补充第 4.1 节：在 Schema 初始化中添加 `PRAGMA journal_mode=WAL;`
- [x] 1.4 重写第 4.2 节：替换 AgentAdapter Trait 为多数据源版本（DataSource 枚举 + parse_content/query_db 方法签名）
- [x] 1.5 新增第 7 节“Agent 日志格式定义”：以表格形式列出 Claude Code、Codex、Gemini CLI、Copilot、OpenCode（新/旧）的日志路径、文件格式、Token 数据提取方式
- [x] 1.6 新增第 8 节“费率方案”：LiteLLM 远程拉取 + 本地按天缓存 + 内置 fallback JSON + 模型名匹配策略
- [x] 1.7 新增第 9 节“冷启动策略”：后台线程解析、30 天 mtime  过滤、进度事件广播、1000 条批量事务、增量可用
- [x] 1.8 新增第 10 节“错误处理策略”：日志损坏跳过、SQLite 重试 3 次、外部数据源降级、前端 Error Boundary
- [x] 1.9 新增第 11 节“测试策略”：前端 vitest + `__test__/` 目录约定、Rust `#[cfg(test)]` 内置测试、覆盖范围定义

## 2. 项目脚手架——前端配置文件

- [x] 2.1 创建 `package.json`：项目名 token-burger、scripts（dev/build/preview/tauri/lint/lint:fix/test）、dependencies（react/react-dom/react-router-dom/framer-motion/@tauri-apps/api/@tauri-apps/plugin-shell）、devDependencies（typescript/vite/@vitejs/plugin-react/tailwindcss/@tailwindcss/vite/eslint/@typescript-eslint/eslint-plugin/@typescript-eslint/parser/eslint-plugin-react-hooks/vitest/@tauri-apps/cli/@types/react/@types/react-dom）
- [x] 2.2 创建 `vite.config.ts`：配置 @vitejs/plugin-react、@tailwindcss/vite、server.strictPort、Tauri 开发环境端口和 host
- [x] 2.3 创建 `tsconfig.json`：strict/jsx:react-jsx/target:ES2021/module:ESNext/moduleResolution:bundler/baseUrl + @/* 路径别名
- [x] 2.4 创建 `tsconfig.node.json`：Vite 配置文件的 TypeScript 编译选项
- [x] 2.5 创建 `eslint.config.js`：flat config 格式，规则含 indent:4/semi:always/quotes:single/@typescript-eslint/react-hooks，无 Prettier
- [x] 2.6 创建 `.gitignore`：node_modules/dist/src-tauri/target/.DS_Store/*.log 等
- [x] 2.7 更新 `.editorconfig`：确认已有配置与项目规范一致（4 空格、UTF-8、trim trailing whitespace）

## 3. 项目脚手架——Rust 后端配置

- [x] 3.1 创建 `src-tauri/Cargo.toml`：包名 token-burger、edition 2021、dependencies（tauri v2 含 tray-icon feature、rusqlite 含 bundled、notify、serde 含 derive、serde_json、tokio 含 full、chrono）、build-dependencies（tauri-build v2）
- [x] 3.2 创建 `src-tauri/tauri.conf.json`：app identifier、sandbox:false、窗口定义（popup 无边框/隐藏 + settings 有标题栏/隐藏）、capabilities 配置（tray-icon、shell）
- [x] 3.3 创建 `src-tauri/build.rs`：标准 Tauri v2 build script
- [x] 3.4 创建 `src-tauri/capabilities/default.json`：Tauri v2 权限声明文件

## 4. 前端目录骨架

- [x] 4.1 创建 `src/main.tsx`：React 入口，挂载 App 到 #root
- [x] 4.2 创建 `src/App.tsx`：React Router Hash 模式，路由到 Popup 和 Setting 页面
- [x] 4.3 创建 `src/App.css`：Tailwind 基础导入
- [x] 4.4 创建 `src/pages/Popup/index.tsx`：Popup 页面占位组件
- [x] 4.5 创建 `src/pages/Setting/index.tsx`：Setting 页面占位组件
- [x] 4.6 创建 `src/components/` 目录（空，后续 change 填充）
- [x] 4.7 创建 `src/hooks/` 目录（空，后续 change 填充）
- [x] 4.8 创建 `src/types/index.ts`：TokenLog 等与 Rust 后端一致的类型定义占位
- [x] 4.9 创建 `index.html`：Vite 入口 HTML，引用 src/main.tsx

## 5. Rust 后端目录骨架

- [x] 5.1 创建 `src-tauri/src/main.rs`：Tauri v2 应用入口，注册 tray、窗口失焦隐藏逻辑的占位注释
- [x] 5.2 创建 `src-tauri/src/lib.rs`：Tauri v2 的 run() 函数（如 v2 推荐 lib.rs + main.rs 分离模式）
- [x] 5.3 创建 `src-tauri/src/db/mod.rs`：模块声明占位，含 TODO 注释（Schema 初始化、WAL 启用）
- [x] 5.4 创建 `src-tauri/src/watcher/mod.rs`：模块声明占位，含 TODO 注释（notify + debounce、轮询、SQLite 查询）
- [x] 5.5 创建 `src-tauri/src/adapters/mod.rs`：AgentAdapter Trait 定义 + DataSource 枚举（仅类型定义，不含实现）

## 6. 开发规范文档

- [x] 6.1 创建 `AGENTS.md`：Tech Stack / Project Conventions / Code Style / 命名规范 / Architecture Patterns / Git Workflow / 测试约定
- [x] 6.2 创建 `CLAUDE.md`：内容为 `@AGENTS.md`
- [x] 6.3 创建 `README.md`：项目简介 / 前置依赖 / 安装与运行 / 项目结构 / 开发命令速查

## 7. 验证

- [x] 7.1 执行 `npm install` 确认前端依赖安装成功
- [x] 7.2 执行 `npm run build` 确认前端构建通过
- [x] 7.3 执行 `npm run lint` 确认 ESLint 无 error
- [x] 7.4 在 src-tauri/ 执行 `cargo build` 确认 Rust 编译通过
- [x] 7.5 在 src-tauri/ 执行 `cargo test` 确认测试框架可运行
- [x] 7.6 执行 `npm run test -- --run` 确认 vitest 可运行（允许 0 测试通过）
