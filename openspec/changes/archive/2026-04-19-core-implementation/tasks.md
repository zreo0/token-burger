## 1. 依赖与配置

- [x] 1.1 Cargo.toml 新增依赖：`notify-debouncer-full`、`reqwest`（features: rustls-tls, json, blocking）、`glob`、`log`、`dirs`
- [x] 1.2 package.json 新增依赖：`react-i18next`、`i18next`
- [x] 1.3 更新 `src-tauri/capabilities/default.json`：声明 Tauri 核心权限和事件权限（Rust 端通过 std::fs / reqwest 直接调用，无需额外前端权限）
- [x] 1.4 创建 `src-tauri/resources/default_pricing.json`：内置 fallback 价格表（从 LiteLLM 提取主流模型子集）

## 2. SQLite 数据库层

- [x] 2.1 实现 `db/mod.rs`：`get_db_path()` 函数（dev/prod 条件编译隔离）、`init_db()` 函数（WAL 模式 + 创建 token_logs / file_offsets / app_settings 三张表及索引）
- [x] 2.2 实现 `db/mod.rs`：`DbManager` 结构体（write_tx + db_path）、专用写线程启动逻辑（mpsc channel 接收 WriteRequest、事务批量 INSERT OR IGNORE、入库后查询汇总 + emit + 更新 tray title）
- [x] 2.3 实现 `db/queries.rs`：token_logs 批量插入（1000 条分批事务）、按时间范围汇总查询（today/7d/30d，按 token_type/agent/model 分组）、数据清理（按天数删除 / 清空全部）
- [x] 2.4 实现 `db/queries.rs`：file_offsets CRUD（get_offset / update_offset）、app_settings CRUD（get_setting / set_setting / get_all_settings）
- [x] 2.5 编写 `db/` 模块单元测试：使用 `Connection::open_in_memory()` 测试 Schema 初始化、批量插入去重、汇总查询、设置读写

## 3. Agent Adapters

- [x] 3.1 重构 `adapters/mod.rs`：为 AgentAdapter Trait 添加 `log_paths()` 方法（返回需要监听的路径列表，支持 glob 展开）；确保 TokenLog 结构体与前端 types 对齐（含 id 字段）
- [x] 3.2 实现 `adapters/claude_code.rs`：解析 `~/.claude/projects/**/*.jsonl`，提取 assistant 事件的 `message.usage`（input/cache_creation/cache_read/output tokens），conversationId → session_id，uuid → request_id，provider 固定 "Anthropic"
- [x] 3.3 实现 `adapters/codex.rs`：解析 `~/.codex/sessions/*.jsonl`，提取 event_msg 中的 token 计数，provider 固定 "OpenAI"
- [x] 3.4 实现 `adapters/gemini_cli.rs`：解析 `~/.gemini/tmp/<hash>/chats/*.json`，全量解析 messages 数组中的 tokens 字段，provider 固定 "Google"
- [x] 3.5 实现 `adapters/copilot.rs`：解析 `~/.copilot/history-session-state/*.json`，从 timeline 事件推断 token 消耗，provider 固定 "GitHub"
- [x] 3.6 实现 `adapters/opencode.rs`：新版 SQLite 模式（查询 `~/.local/share/opencode/opencode.db` 的 message 表，role=assistant，从 data JSON 提取 tokens）+ 旧版 JSON fallback（解析 `storage/message/**/*.json`）
- [x] 3.7 编写 Adapter 单元测试：每个 Adapter 用内联 fixture 数据测试 `parse_content()` 的正确解析、格式异常跳过、空内容处理

## 4. Watcher 引擎

- [x] 4.1 实现 `watcher/mod.rs`：WatcherEngine 结构体（持有 adapters + write_tx + app_handle）、`start()` 方法（启动后台线程，按 DataSource 分发策略）、`stop()` 方法
- [x] 4.2 实现 `watcher/notify_strategy.rs`：使用 `notify-debouncer-full` 监听 JSONL 目录（500ms debounce、RecursiveMode::Recursive、.jsonl 扩展名过滤）、读取增量内容 → 调用 adapter.parse_content() → 发送到 write_tx
- [x] 4.3 实现 `watcher/polling_strategy.rs`：10s 间隔轮询 JSON 文件目录、mtime 比对、变更文件全量解析 → 发送到 write_tx
- [x] 4.4 实现 `watcher/sqlite_strategy.rs`：10s 间隔查询外部 SQLite、timestamp 增量查询 → 发送到 write_tx、数据库不可读时 warn 日志 + 跳过
- [x] 4.5 实现 offset 断点续传逻辑：读取前从 file_offsets 获取 last_offset、文件大小 < last_offset 时重置为 0（轮转检测）、读取后更新 offset
- [x] 4.6 实现冷启动编排：启动时遍历所有启用 Adapter → 按 mtime 过滤最近 N 天文件 → 逐文件解析 → 批量写入 → 每完成一个 Adapter emit cold-start-progress → 全部完成后切换到正常监听模式
- [x] 4.7 编写 Watcher 单元测试：offset 断点续传逻辑、文件轮转检测、mtime 过滤

## 5. 费率引擎

- [x] 5.1 实现 `pricing/mod.rs`：启动时检查本地缓存（`~/.token-burger/[dev/]pricing/model_pricing_YYYY-MM-DD.json`）→ 不存在则 reqwest blocking 拉取 LiteLLM JSON（10s 超时）→ 失败则加载内置 fallback
- [x] 5.2 实现模型名匹配逻辑：精确匹配 → 归一化匹配（去日期后缀 `-YYYYMMDD`、版本号 `-v\d`）→ Provider 前缀匹配（去 `anthropic/` 等前缀）→ 未匹配返回零价格
- [x] 5.3 编写 pricing 单元测试：缓存命中/未命中、模型名匹配四级策略、fallback 加载

## 6. Tauri IPC 层

- [x] 6.1 实现 `commands.rs`：定义 `get_token_summary`、`get_agent_list`、`toggle_agent`、`get_settings`、`update_settings`、`clear_data`、`get_pricing` 共 7 个 `#[tauri::command]`
- [x] 6.2 更新 `lib.rs`：setup 中初始化 DbManager（app.manage）、启动 WatcherEngine、注册所有 commands（invoke_handler）、启动费率引擎
- [x] 6.3 实现 tray title 动态更新：写线程入库后格式化 token 总量（K/M/B 规则）→ `tray.set_title(Some(formatted))`
- [x] 6.4 定义 Rust 端共享类型：TokenSummary、AgentInfo、AppSettings、PricingTable、ColdStartProgress 等 serde 可序列化结构体

## 7. 前端类型与工具函数

- [x] 7.1 更新 `src/types/index.ts`：新增 TokenSummary、AgentInfo、AppSettings、PricingTable、ColdStartProgress 接口定义，与 Rust 端对齐
- [x] 7.2 实现 `src/utils/format.ts`：token 数格式化函数（formatTokenCount: K/M/B 规则，保留一位小数）、金额格式化函数（formatCost: $X.XX）
- [x] 7.3 实现 `src/utils/pricing.ts`：前端金额计算函数（calculateCost: 按模型匹配价格 → 计算四种 token 类型的花费 → 求和）、模型名匹配函数（matchModelPrice: 精确→归一化→前缀→$0）
- [x] 7.4 编写 utils 单元测试：formatTokenCount 边界值（0/999/1000/1500/1000000/1500000000）、calculateCost 多模型汇总、matchModelPrice 四级匹配

## 8. i18n 国际化

- [x] 8.1 实现 `src/i18n/index.ts`：i18next 初始化配置（默认 en、fallback en、插值转义关闭）
- [x] 8.2 创建 `src/i18n/locales/en.json`：所有 UI 文本的英文翻译（popup.*/settings.*/common.*）
- [x] 8.3 创建 `src/i18n/locales/zh-CN.json`：所有 UI 文本的简体中文翻译
- [x] 8.4 更新 `src/main.tsx`：导入 i18n 初始化模块

## 9. 前端状态管理

- [x] 9.1 实现 `src/hooks/useTokenStream.ts`：监听 `token-updated` 事件 + 初始化调用 `get_token_summary` + 返回 { summary, loading, error, refresh, setRange }
- [x] 9.2 实现 `src/context/TokenContext.tsx`：TokenProvider 包裹 useTokenStream，通过 Context 向子组件提供 token 数据
- [x] 9.3 更新 `src/App.tsx`：用 TokenProvider 包裹路由

## 10. Popup 页面与 Burger 组件

- [x] 10.1 实现 `src/components/Burger/BurgerLayer.tsx`：单层组件（motion.div + spring 动画控制厚度 + useMotionValue 数字滚动 + 渐变色背景 + 暗色/亮色模式适配）
- [x] 10.2 实现 `src/components/Burger/index.tsx`：四层汉堡主组件（底部面包 + Input/CacheCreate/CacheRead/Output 四层 + 顶部面包），消费 TokenContext 数据
- [x] 10.3 实现 `src/components/Burger/index.css`：Burger 组件样式（毛玻璃效果、圆角、阴影、暗色/亮色模式变量）
- [x] 10.4 实现 `src/pages/Popup/index.tsx`：顶部 Segmented Control 时间范围选择器 + 中部 Burger 组件 + 底部预估花费展示 + 冷启动加载状态 + Error Boundary
- [x] 10.5 实现 `src/pages/Popup/index.css`：Popup 页面样式（macOS popover 风格：无边框圆角容器、毛玻璃背景、柔和阴影、系统字体栈）

## 11. Settings 页面

- [x] 11.1 实现 `src/pages/Settings/index.tsx`：Tab 导航结构（General / Agents / Data）+ AnimatePresence 页面切换动画
- [x] 11.2 实现 Settings General Tab：语言选择下拉（English / 简体中文）+ 监控模式 Segmented Control（Realtime / Polling）+ 开发环境标识
- [x] 11.3 实现 Settings Agents Tab：Agent 列表卡片（名称 + 状态指示器 + Toggle 开关 + 不可用时灰色禁用）
- [x] 11.4 实现 Settings Data Tab：数据保留天数配置 + "清理旧数据"按钮 + "清空全部数据"按钮（含确认对话框）
- [x] 11.5 实现 `src/pages/Settings/index.css`：Settings 页面样式（macOS 偏好设置面板风格、Tab 导航、表单控件原生风格、暗色/亮色模式）

## 12. 全局样式与主题

- [x] 12.1 配置 Tailwind 暗色/亮色模式支持（`prefers-color-scheme` 媒体查询策略）、定义 CSS 变量（背景色、文字色、边框色、选中色等）
- [x] 12.2 实现全局基础样式：系统字体栈（-apple-system, SF Pro）、圆角/阴影/毛玻璃 utility classes、过渡动画默认值（150-300ms）

## 13. 集成与验收

- [x] 13.1 端到端冒烟测试：`cargo tauri dev` 启动 → tray 显示汉堡图标和 "0" → 点击弹出 Popup → Burger 四层渲染 → Settings 可打开
- [x] 13.2 验证数据流闭环：手动创建测试 JSONL 文件 → Watcher 检测 → Adapter 解析 → SQLite 入库 → token-updated 事件 → Popup 数字更新 → tray title 更新
- [x] 13.3 运行全部测试：`cargo test`（Rust 端 Adapter/DB/Watcher 测试）+ `npx vitest run`（前端 utils/hooks 测试）
- [x] 13.4 运行 ESLint 检查：`npm run lint` 无错误
