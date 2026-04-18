## Context

TokenBurger 是一个 macOS 菜单栏应用，实时监控 AI Coding Agent 的 Token 消耗。当前仓库只有 DESIGN.md 和工具配置，没有任何可运行代码。

## Goals / Non-Goals

**Goals:**

- 产出可直接 `npm install && cargo build` 的完整项目脚手架
- DESIGN.md 补充章节覆盖所有调研发现的缺陷和遗漏
- AGENTS.md 和 README.md 符合团队现有项目的格式规范
- ESLint flat config 仅包含核心规则，不引入第三方配置包
- 测试基础设施就绪（vitest + cargo test），可立即编写测试

**Non-Goals:**

- 不实现任何业务功能代码（Adapter 解析、UI 组件、数据库操作等留给后续 change）
- 不创建 CI/CD 流水线
- 不处理 macOS 代码签名和 DMG 打包
- 不实现自动更新机制

## Decisions

### D1: Tauri v2（而非 v1）

Tauri v2 是当前稳定版，相比 v1：
- System Tray API 重新设计，`tray.set_title()` 原生支持菜单栏文字
- 插件系统模块化（`tauri-plugin-shell` 等按需引入）
- 更好的多窗口管理 API
- 安全模型改进（capabilities 替代 allowlist）

v1 文档和社区示例更多，但 v2 已稳定一年以上，不存在兼容性风险。

### D2: ESLint Flat Config，最小规则集

暂时不使用类似 `eslint-config-*` 等第三方配置包。仅依赖：
- `@typescript-eslint/eslint-plugin` + `@typescript-eslint/parser`
- `eslint-plugin-react-hooks`

核心规则：
- `indent: ['error', 4]`
- `semi: ['error', 'always']`
- `quotes: ['error', 'single']`
- `@typescript-eslint/no-explicit-any: 'warn'`
- `react-hooks/rules-of-hooks: 'error'`
- `react-hooks/exhaustive-deps: 'warn'`

理由：新项目不需要继承历史包袱，最小规则集足够保证一致性，后续按需添加。

### D3: 多数据源 Adapter Trait

调研发现各 Agent 日志格式差异很大：

| Agent | 路径 | 格式 | 监听策略 |
|-------|------|------|---------|
| Claude Code | `~/.claude/projects/**/*.jsonl` | JSONL | notify + offset 增量 |
| Codex | `~/.codex/sessions/*.jsonl` | JSONL | notify + offset 增量 |
| Gemini CLI | `~/.gemini/tmp/<hash>/chats/*.json` | JSON | 轮询 + mtime 缓存 |
| Copilot | `~/.copilot/history-session-state/*.json` | JSON | 轮询 + mtime 缓存 |
| OpenCode (新) | `~/.local/share/opencode/opencode.db` | SQLite | 定时查询 |
| OpenCode (旧) | `~/.local/share/opencode/storage/**/*.json` | JSON | 轮询 + mtime 缓存 |

原 DESIGN.md 的 `AgentAdapter` Trait 假设所有日志都是可增量读取的文本流，不适用于 JSON 全量解析和 SQLite 直读场景。

重新设计：

```rust
pub enum DataSource {
    /// JSONL 文件，支持 offset 增量读取
    Jsonl { paths: Vec<PathBuf> },
    /// JSON 文件，全量解析 + mtime 缓存
    Json { paths: Vec<PathBuf> },
    /// 外部 SQLite 数据库，定时查询
    Sqlite { db_path: PathBuf },
}

pub trait AgentAdapter: Send + Sync {
    fn agent_name(&self) -> &str;
    fn data_source(&self) -> DataSource;
    fn parse_content(&self, content: &str) -> Vec<TokenLog>;
    fn query_db(&self, db_path: &Path, since: Option<i64>) -> Result<Vec<TokenLog>>;
}
```

Watcher 引擎根据 `data_source()` 自动选择：JSONL 用 notify + debounce，JSON 用定时轮询 + mtime 检查，SQLite 用定时查询。

### D4: 修复 idx_request_dedup 索引冲突

原 Schema 中 `UNIQUE(request_id, token_type)` 允许同一 request_id 有多条不同 token_type 的记录，但 `CREATE UNIQUE INDEX idx_request_dedup ON token_logs(request_id)` 要求 request_id 全局唯一，两者矛盾。

修正：改为普通索引 `CREATE INDEX IF NOT EXISTS idx_request_dedup ON token_logs(request_id)`。复合唯一约束已覆盖去重需求。

### D5: SQLite WAL 模式

TokenBurger 存在 Watcher 线程写入 + 前端查询读取的并发场景。默认 DELETE journal 模式下写入会阻塞读取。

启用 WAL：`PRAGMA journal_mode=WAL;`

WAL 的自动 checkpoint（默认 1000 页）对 TokenBurger 的数据量完全够用，无需手动管理。无实质副作用。

### D6: 费率方案——LiteLLM 远程 + 本地缓存 + 内置 Fallback

从 LiteLLM GitHub 拉取 `model_prices_and_context_window.json`，本地按天缓存。

TokenBurger 采用相同方案：
- Rust 端启动时拉取远程价格表，缓存到 `~/.token-burger/pricing/model_pricing_YYYY-MM-DD.json`
- 项目内置一份默认价格表作为离线 fallback
- 前端拿到 token 数 + 价格表后即时计算金额
- 模型名匹配策略：精确匹配 → 归一化匹配（去版本号/日期后缀）→ 子串匹配 → 未知模型显示 $0

### D7: 冷启动策略

重度用户的 Claude Code 日志可能有几百 MB。首次启动策略：
- 后台线程逐 Agent 解析，按 mtime 只处理最近 30 天的文件
- 每解析完一个 Agent 即可展示部分数据
- 前端显示"🍔 Burger 制作中..."进度状态
- 批量 INSERT 使用事务（1000 条一批），避免长事务
- 完成后切换到正常监听模式
- 设置页面可配置保留天数

### D8: 测试策略

前端（TypeScript）：
- 框架：vitest（与 Vite 生态一致）
- 目录约定：`__test__/` 放在对应模块目录下
- 覆盖范围：hooks、utils（格式化函数、费用计算）、类型守卫
- 命名：`*.test.ts` / `*.test.tsx`

Rust 端：
- 框架：cargo test（内置）
- 约定：`#[cfg(test)] mod tests` 写在同文件底部
- 覆盖范围：Adapter 解析逻辑（用内联的 fixture 数据）、数据库 CRUD、防抖逻辑
- 集成测试：`src-tauri/tests/` 目录（如需跨模块测试）

### D9: AGENTS.md 结构

文件中应该包含：
- Tech Stack（Tauri v2 / Rust / React 18 / TypeScript / Vite / Tailwind / Framer Motion）
- Project Conventions（如无必要，勿增实体）
- Code Style（4 空格、单引号、分号、ESLint flat config）
- 命名规范（Rust: snake_case 文件/变量、PascalCase 类型；TS: camelCase 变量、PascalCase 组件）
- Architecture Patterns（前后端分层、适配器模式、事件驱动）
- Git Workflow（`feat:` / `fix:` / `chore:` 等 commit 风格）
- 测试约定

CLAUDE.md 内容为 `@AGENTS.md`。

### D10: README.md 结构

README.md 应包含以下部分：

- 项目简介（一句话说明）
- 前置依赖（Rust、Node.js、Tauri CLI 版本要求）
- 安装与运行命令
- 项目结构说明（简要目录树）
- 开发命令速查

### D11: 错误处理策略

- 日志文件损坏/格式异常：Adapter 的 `parse_content` 逐行解析，跳过无法解析的行，记录 warn 日志，不中断整体流程
- SQLite 锁定：WAL 模式下极少发生；若发生，重试 3 次后跳过本轮写入
- 外部 SQLite（OpenCode）不可读：降级为不监控该 Agent，设置页面标记为"不可用"
- 前端：React Error Boundary 包裹核心组件，异常时显示 fallback UI

## Risks / Trade-offs

**[LiteLLM 价格表不可用] → 内置 fallback JSON 兜底，金额显示可能不准确但不影响 token 统计**

**[Agent 日志格式变更] → Adapter 模式隔离变更影响范围，单个 Adapter 失败不影响其他 Agent 监控**

**[冷启动耗时过长] → 30 天限制 + 后台线程 + 进度展示；用户可在设置中调整保留天数**

**[notify 在某些文件系统上不可靠] → 提供轮询模式作为备选，用户可在设置中切换**

**[Tauri v2 的 tray.set_title() 兼容性] → 需要在实现阶段验证 macOS 上的实际表现，如不可用则退化为仅图标模式**
