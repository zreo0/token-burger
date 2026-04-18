## Context

TokenBurger 项目脚手架已完成：Tauri v2 + React 18 + Vite + Tailwind + Framer Motion + ESLint。当前 Rust 端 `db/`、`watcher/`、`adapters/` 均为 TODO 占位，前端 Popup/Settings 为空壳，tray title 写死 "0"。

现有代码基础：
- `adapters/mod.rs`：已定义 `DataSource` 枚举、`AgentAdapter` Trait、`TokenLog` / `TokenType` 结构体（含 serde 序列化和单元测试）
- `lib.rs`：已实现 tray 创建、positioner 定位、popup 失焦隐藏
- `src/types/index.ts`：已定义前端 `TokenLog` 接口
- `tauri.conf.json`：popup（360×480 无边框透明）和 settings（600×400 有边框）两个窗口

约束：
- Rust 端禁用 ORM，使用 `rusqlite` 原生 SQL
- 不开启 macOS App Sandbox
- 前端无 Prettier，仅 ESLint
- 4 空格缩进、单引号、必须分号

## Goals / Non-Goals

**Goals:**

- 实现完整的数据流闭环：日志监听 → Adapter 解析 → SQLite 入库 → 事件广播 → 前端渲染
- 6 个 Agent Adapter 全部可用（Claude Code、Codex、Gemini CLI、Copilot、OpenCode 新/旧）
- Popup 页面展示四层 Burger 动画 + 实时 token 数 + 预估花费
- Settings 页面支持 Agent 开关、监控模式切换、数据清理
- Tray title 实时显示格式化后的 token 总量
- 冷启动不阻塞 UI，支持进度展示
- 每个模块有对应的单元测试

**Non-Goals:**

- 不实现 CI/CD、代码签名、DMG 打包、自动更新
- 不实现多平台支持（仅 macOS）
- 不实现用户账号体系或云端同步
- 不实现历史趋势图表（留给后续迭代）

## Decisions

### D1: Rust 并发模型——专用写线程 + 独立读连接

rusqlite 的 `Connection` 是 `Send` 但不是 `Sync`。在 WAL 模式下，SQLite 支持一个 writer + 多个 reader（需要独立连接）。

**方案**：
- **写线程**：一个专用 `std::thread`，持有唯一的写连接，通过 `mpsc::channel` 接收写入请求
- **读连接**：Tauri commands 中按需创建只读连接（`OpenFlags::SQLITE_OPEN_READ_ONLY`），或用 `Arc<Mutex<Connection>>` 共享一个读连接
- **状态管理**：`app.manage()` 注册 `DbManager` 结构体，内含写通道 sender 和数据库路径

```rust
pub struct DbManager {
    /// 发送写入请求到专用写线程
    write_tx: mpsc::Sender<WriteRequest>,
    /// 数据库文件路径，用于创建只读连接
    db_path: PathBuf,
}
```

**替代方案**：`Arc<Mutex<Connection>>` 单连接——简单但所有操作串行化，冷启动批量写入时会阻塞前端查询。

**选择理由**：TokenBurger 的核心场景是 watcher 高频写入 + 前端实时查询并发，读写分离是必要的。

### D2: Watcher 架构——统一调度器 + 三种策略

Watcher 引擎作为单独的后台线程运行，内部根据 Adapter 的 `data_source()` 分发到不同策略：

```
WatcherEngine (std::thread)
├── NotifyStrategy    → JSONL 文件（Claude Code, Codex）
│   └── notify-debouncer-full, 500ms debounce
│   └── RecursiveMode::Recursive + 扩展名过滤
├── PollingStrategy   → JSON 文件（Gemini CLI, Copilot, OpenCode 旧）
│   └── 10s 轮询间隔 + mtime 比对
│   └── 全量解析变更文件
└── SqliteStrategy    → 外部 SQLite（OpenCode 新）
    └── 10s 轮询间隔 + timestamp 增量查询
```

**关键依赖**：`notify` v7 本身不含 debounce，需要 `notify-debouncer-full` crate。notify 不支持 glob，需监听父目录 + 手动过滤扩展名。

**事件流**：
1. 策略检测到变更 → 读取内容 → 调用 Adapter 的 `parse_content()` 或 `query_db()`
2. 解析结果 `Vec<TokenLog>` 通过 `write_tx` 发送到写线程
3. 写线程批量 `INSERT OR IGNORE` 入库
4. 写线程入库后查询当日汇总 → `app.emit("token-updated", summary)`
5. 写线程更新 tray title → `tray.set_title(Some(formatted))`

### D3: Tauri IPC 协议设计

**Commands（前端 → Rust）**：

| Command | 参数 | 返回值 | 用途 |
|---------|------|--------|------|
| `get_token_summary` | `{ range: "today" \| "7d" \| "30d" }` | `TokenSummary` | 查询指定时间范围的 token 汇总 |
| `get_agent_list` | — | `Vec<AgentInfo>` | 获取所有支持的 Agent 及其状态 |
| `toggle_agent` | `{ agent: string, enabled: bool }` | `()` | 启用/禁用某个 Agent 的监控 |
| `get_settings` | — | `AppSettings` | 获取当前设置 |
| `update_settings` | `AppSettings` | `()` | 更新设置 |
| `clear_data` | `{ keep_days: Option<u32> }` | `()` | 清理历史数据 |
| `get_pricing` | — | `PricingTable` | 获取模型价格表 |

**Events（Rust → 前端）**：

| Event | Payload | 触发时机 |
|-------|---------|---------|
| `token-updated` | `TokenSummary` | 每次入库完成后 |
| `cold-start-progress` | `{ agent: string, done: bool, total: u32, completed: u32 }` | 冷启动解析进度 |

**`TokenSummary` 结构**：

```typescript
interface TokenSummary {
    input: number;
    cache_create: number;
    cache_read: number;
    output: number;
    total: number;
    by_agent: Record<string, { input: number; cache_create: number; cache_read: number; output: number }>;
    by_model: Record<string, { input: number; cache_create: number; cache_read: number; output: number }>;
}
```

### D4: 前端状态管理——useTokenStream hook + React Context

不引入 Redux/Zustand 等状态库。使用 `useTokenStream` 自定义 hook 封装 Tauri 事件监听：

```
App
├── TokenProvider (Context)
│   ├── useTokenStream() → 监听 token-updated 事件
│   ├── 初始化时调用 get_token_summary command
│   └── 提供 summary + loading + error 状态
├── Popup
│   ├── Burger 组件（消费 Context）
│   ├── 时间范围选择器
│   └── 费用展示
└── Settings
    ├── Agent 列表（调用 get_agent_list）
    ├── 监控模式切换
    └── 数据清理
```

**理由**：TokenBurger 状态简单（一个 summary 对象 + settings），Context 足够，不需要外部状态库。

### D5: Burger 动画方案——Framer Motion spring + useMotionValue

四层汉堡自下而上：底部面包 → Input → Cache Create → Cache Read → Output → 顶部面包。

每层的视觉属性：
- **厚度**：`height` 由 `motion.div` 的 `animate={{ height }}` 控制，使用 `spring` transition（stiffness: 300, damping: 20）
- **数字**：使用 Framer Motion 的 `useMotionValue` + `useTransform` 实现平滑数字滚动
- **颜色**：每层固定颜色（Input: 绿色, Cache Create: 橙色, Cache Read: 蓝色, Output: 红色）

厚度计算：`minHeight + (tokenCount / maxTokenCount) * maxExtraHeight`，设置合理的 min/max 防止层级过薄或过厚。

### D6: 费率引擎——Rust 拉取 + 前端计算

**Rust 端**：
- 启动时检查 `~/.token-burger/pricing/model_pricing_YYYY-MM-DD.json` 是否存在
- 不存在则从 `https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json` 拉取
- 拉取失败则使用项目内置的 `src-tauri/resources/default_pricing.json`
- 通过 `get_pricing` command 返回给前端

**前端金额计算**：
- `cost = (input_tokens * input_price + output_tokens * output_price + cache_create_tokens * cache_create_price + cache_read_tokens * cache_read_price) / 1_000_000`
- 价格单位：美元/百万 token（LiteLLM 格式）
- 使用 JavaScript 原生 Number（精度足够，token 级别的金额计算不需要 BigDecimal）

**模型名匹配**（按优先级）：
1. 精确匹配：`model_id === pricing_key`
2. 归一化匹配：去除日期后缀（`-20250219`）和版本号（`-v2`）
3. Provider 前缀匹配：`anthropic/claude-3-7-sonnet` → `claude-3-7-sonnet`
4. 未匹配：显示 $0.00

**新增 Rust 依赖**：`reqwest`（HTTP 客户端，features: `["rustls-tls", "json"]`）

### D7: 设置持久化——SQLite settings 表

新增 `app_settings` 表存储用户配置，避免引入额外的配置文件格式：

```sql
CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

预定义 key：
- `enabled_agents`：JSON 数组，如 `["claude-code", "codex"]`
- `watch_mode`：`"realtime"` | `"polling"`
- `keep_days`：整数，默认 30
- `polling_interval_secs`：整数，默认 10

**替代方案**：TOML/JSON 配置文件——但已有 SQLite，不需要引入额外的序列化格式和文件 I/O。

### D8: 冷启动编排

启动流程：
1. `lib.rs` setup → 初始化 DB → 启动 watcher 线程
2. Watcher 线程首先进入冷启动模式：
   - 遍历所有启用的 Adapter
   - 每个 Adapter 扫描日志文件，按 mtime 过滤最近 N 天
   - 逐文件解析 → 批量写入（1000 条/事务）
   - 每完成一个 Adapter → `emit("cold-start-progress", ...)`
3. 全部完成后切换到正常监听模式
4. 前端收到 `cold-start-progress` 事件后显示/隐藏加载状态

### D9: 错误处理与日志

**Rust 端**：
- 使用 `log` crate + `env_logger`（开发时）或 Tauri 内置日志
- Adapter 解析错误：`warn!` 记录 + 跳过该行/文件，不中断
- SQLite 写入失败：重试 3 次（`busy_timeout(5000)`），仍失败则 `error!` 记录 + 跳过本轮
- 网络请求失败（费率拉取）：`warn!` 记录 + 使用 fallback

**前端**：
- React Error Boundary 包裹 Popup 和 Settings 页面
- Tauri command 调用统一 try-catch，错误时显示 toast 或 fallback UI

### D10: 新增 Rust 依赖

| Crate | 用途 | Features |
|-------|------|----------|
| `notify-debouncer-full` | 文件监听防抖 | — |
| `reqwest` | HTTP 拉取费率表 | `rustls-tls`, `json`, `blocking` |
| `glob` | 日志路径模式匹配 | — |
| `log` | 日志框架 | — |
| `dirs` | 获取 `$HOME` 等标准路径 | — |

**不新增前端依赖**：Framer Motion、React Router、Tailwind 已在 package.json 中。

### D11: 文件结构扩展

```
src-tauri/src/
├── lib.rs              # setup: 初始化 DB + 注册 commands + 启动 watcher
├── main.rs
├── commands.rs         # 所有 #[tauri::command] 定义
├── db/
│   ├── mod.rs          # DbManager + Schema 初始化 + WAL + dev/prod 隔离
│   └── queries.rs      # 具体 SQL 查询封装
├── watcher/
│   ├── mod.rs          # WatcherEngine 调度器
│   ├── notify_strategy.rs   # JSONL 文件监听策略
│   ├── polling_strategy.rs  # JSON 文件轮询策略
│   └── sqlite_strategy.rs   # 外部 SQLite 查询策略
├── adapters/
│   ├── mod.rs          # Trait + TokenLog（已有）
│   ├── claude_code.rs
│   ├── codex.rs
│   ├── gemini_cli.rs
│   ├── copilot.rs
│   └── opencode.rs     # 新版 SQLite + 旧版 JSON fallback
└── pricing/
    └── mod.rs          # 费率拉取 + 缓存 + 匹配

src/
├── components/
│   └── Burger/
│       ├── index.tsx   # 四层汉堡主组件
│       ├── BurgerLayer.tsx  # 单层组件（动画 + 数字）
│       └── index.css
├── hooks/
│   └── useTokenStream.ts   # Tauri 事件监听 + 状态
├── utils/
│   ├── format.ts       # token 数格式化（K/M/B）、金额格式化
│   └── pricing.ts      # 前端金额计算逻辑
├── i18n/
│   ├── index.ts        # i18next 初始化
│   └── locales/
│       ├── en.json     # 英文（默认）
│       └── zh-CN.json  # 简体中文
├── context/
│   └── TokenContext.tsx # React Context Provider
├── pages/
│   ├── Popup/
│   │   ├── index.tsx   # Burger + 时间范围 + 费用
│   │   └── index.css
│   └── Settings/
│       ├── index.tsx   # Agent 列表 + 监控偏好 + 数据清理
│       └── index.css
├── types/
│   └── index.ts        # TokenLog + TokenSummary + AppSettings 等
└── App.tsx

src-tauri/resources/
└── default_pricing.json  # 内置 fallback 价格表
```

### D12: i18n——react-i18next + 命名空间

支持英文（默认）和简体中文两种语言。代码注释保持中文不变。

**方案**：`react-i18next` + `i18next`，纯前端翻译，Rust 端不涉及。

```
src/
├── i18n/
│   ├── index.ts          # i18next 初始化配置
│   └── locales/
│       ├── en.json       # 英文翻译
│       └── zh-CN.json    # 简体中文翻译
```

- 语言检测：优先读取 `app_settings` 表中的 `language` key，fallback 到 `navigator.language`
- 切换：Settings 页面提供语言下拉选择，切换后写入 `app_settings` 并立即生效
- 翻译范围：所有用户可见的 UI 文本（按钮、标签、提示、状态文字）；不翻译 agent 名称和 model ID
- 命名空间：单一 `translation` 命名空间，按页面用 key 前缀区分（`popup.title`、`settings.agents`）

**新增前端依赖**：`react-i18next`、`i18next`

### D13: Dev/Prod 环境隔离

遵循 DESIGN.md 第 12 节的要求，使用 Rust 条件编译宏实现数据文件物理隔离：

```rust
pub fn get_db_path(app_handle: &AppHandle) -> PathBuf {
    let mut path = app_handle.path().app_data_dir().expect("无法获取应用数据目录");
    if !path.exists() {
        std::fs::create_dir_all(&path).expect("无法创建应用数据目录");
    }
    #[cfg(debug_assertions)]
    { path.push("tokenburger_dev.sqlite"); }
    #[cfg(not(debug_assertions))]
    { path.push("tokenburger_prod.sqlite"); }
    path
}
```

- 开发环境：`tokenburger_dev.sqlite`
- 生产环境：`tokenburger_prod.sqlite`
- 费率缓存目录也做隔离：`~/.token-burger/dev/pricing/` vs `~/.token-burger/pricing/`
- Settings 页面底部显示当前环境标识（仅 debug 模式可见）

### D14: UI 设计方向——macOS 原生审美

遵循 Apple Human Interface Guidelines 的设计语言，打造现代、优雅的 macOS 原生体验：

**视觉风格**：
- 毛玻璃背景（`backdrop-filter: blur()`）+ 半透明层叠，与 macOS Sonoma/Sequoia 系统风格一致
- 圆角卡片（`border-radius: 12px`）、柔和阴影、微妙的边框（`1px solid rgba(255,255,255,0.1)`）
- 色彩：以系统灰为基底，Burger 四层使用高饱和度的渐变色作为视觉焦点
- 字体：`-apple-system, BlinkMacSystemFont, 'SF Pro'` 系统字体栈
- 暗色模式优先（开发者工具的典型使用场景），同时支持亮色模式（跟随系统 `prefers-color-scheme`）

**Popup 窗口**：
- 无边框 + 透明背景 + 圆角容器，模拟 macOS 原生 popover 效果
- 顶部：时间范围切换（Segmented Control 风格）
- 中部：Burger 动画区域（视觉焦点）
- 底部：预估花费 + 简要统计

**Settings 窗口**：
- 标准 macOS 偏好设置面板风格
- 左侧 sidebar 导航（General / Agents / Data）或顶部 tab bar
- 表单控件使用 macOS 原生风格（Toggle、Select、Segmented Control）
- 过渡动画：页面切换使用 `framer-motion` 的 `AnimatePresence` + slide 效果

**交互细节**：
- 所有状态变化有 150-300ms 的过渡动画
- 按钮 hover/active 状态有微妙的缩放和透明度变化
- 列表项 hover 高亮使用系统选中色
- 加载状态使用骨架屏（skeleton）而非 spinner

## Risks / Trade-offs

**[notify-debouncer-full 新增依赖] → notify v7 不含内置 debounce，必须引入。crate 成熟度高（notify 官方维护），风险低。**

**[reqwest 体积较大] → 使用 `rustls-tls` 而非 `native-tls` 减少系统依赖。仅用于启动时一次性拉取，blocking 模式即可，不需要 async runtime。**

**[单写线程瓶颈] → 冷启动时大量写入可能排队。缓解：批量事务（1000 条/批）+ `busy_timeout(5000)` + 写入队列有界（背压）。正常运行时写入频率很低，不构成瓶颈。**

**[前端 Context 性能] → TokenSummary 更新频率受 debounce 限制（≥500ms），不会导致过度渲染。如果未来需要更细粒度的订阅，可迁移到 Zustand。**

**[Agent 日志格式变更] → 每个 Adapter 独立实现，单个失败不影响其他。Adapter 的 `parse_content` 逐行解析 + 跳过异常行，容错性高。**

**[LiteLLM 价格表格式变更] → 内置 fallback 兜底。价格表结构简单（key-value），格式变更概率低。**
