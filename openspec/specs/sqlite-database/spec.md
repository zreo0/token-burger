# Purpose
定义本地 SQLite 数据库的 schema、并发模型和查询清理行为。

## Requirements

### Requirement: Schema 初始化
系统启动时 SHALL 自动创建 SQLite 数据库并初始化 Schema。启用 WAL 模式（`PRAGMA journal_mode=WAL`），创建 `token_logs` 表（含 `UNIQUE(request_id, token_type)` 复合唯一约束）、`file_offsets` 表和 `app_settings` 表。索引 `idx_request_dedup` MUST 为普通索引（非唯一索引），避免与复合唯一约束冲突。

#### Scenario: 首次启动创建数据库
- **WHEN** 应用首次启动且数据库文件不存在
- **THEN** 系统创建数据库文件，执行 `PRAGMA journal_mode=WAL`，创建 `token_logs`、`file_offsets`、`app_settings` 三张表及所有索引

#### Scenario: 重复启动不破坏已有数据
- **WHEN** 应用启动且数据库已存在
- **THEN** 系统使用 `CREATE TABLE IF NOT EXISTS` 和 `CREATE INDEX IF NOT EXISTS`，不影响已有数据

### Requirement: Dev/Prod 环境隔离
系统 SHALL 使用 Rust 条件编译宏 `#[cfg(debug_assertions)]` 区分开发和生产环境的数据库文件名。开发环境 MUST 使用 `tokenburger_dev.sqlite`，生产环境 MUST 使用 `tokenburger_prod.sqlite`。

#### Scenario: 开发模式使用 dev 数据库
- **WHEN** 应用以 `cargo tauri dev` 启动（`debug_assertions` 为 true）
- **THEN** 数据库路径为 `{app_data_dir}/tokenburger_dev.sqlite`

#### Scenario: 生产模式使用 prod 数据库
- **WHEN** 应用以 `cargo tauri build` 构建后运行（`debug_assertions` 为 false）
- **THEN** 数据库路径为 `{app_data_dir}/tokenburger_prod.sqlite`

### Requirement: 读写分离并发模型
系统 SHALL 使用专用写线程持有唯一写连接，通过 `mpsc::channel` 接收写入请求。读操作 SHALL 使用独立的只读连接。DbManager 结构体 MUST 包含写通道 sender 和数据库路径，通过 `app.manage()` 注册为 Tauri 托管状态。

#### Scenario: 写入请求通过通道发送
- **WHEN** Watcher 引擎解析出新的 TokenLog 数据
- **THEN** 数据通过 `write_tx` 发送到写线程，写线程在事务中批量 `INSERT OR IGNORE`

#### Scenario: 读操作不阻塞写入
- **WHEN** 前端通过 command 查询 token 汇总
- **THEN** 查询使用独立的只读连接，不与写线程竞争锁

### Requirement: Token 日志批量写入
系统 SHALL 支持批量写入 `Vec<TokenLog>`，使用 `rusqlite` 事务 + `INSERT OR IGNORE` 确保原子性和幂等性。冷启动时 MUST 按 1000 条一批分割事务。

#### Scenario: 批量插入去重
- **WHEN** 写线程收到包含重复 `(request_id, token_type)` 的数据
- **THEN** 重复记录被 `INSERT OR IGNORE` 静默跳过，不报错

#### Scenario: 冷启动分批写入
- **WHEN** 冷启动解析出 5000 条记录
- **THEN** 系统分为 5 个事务（每个 1000 条）依次提交

### Requirement: Token 汇总查询
系统 SHALL 提供按时间范围查询 token 汇总的能力，返回按 `token_type` 分组的总量，以及按 `agent_name` 和 `model_id` 的细分统计。

#### Scenario: 查询今日汇总
- **WHEN** 前端请求 `range: "today"` 的 token 汇总
- **THEN** 系统返回今日 0:00 至当前时刻的 input/cache_create/cache_read/output 各类型总量、按 agent 和 model 的细分数据

#### Scenario: 查询最近 7 天汇总
- **WHEN** 前端请求 `range: "7d"` 的 token 汇总
- **THEN** 系统返回最近 7 天的汇总数据

### Requirement: 数据清理
系统 SHALL 支持按保留天数清理历史数据，以及清空全部数据。

#### Scenario: 保留最近 N 天数据
- **WHEN** 用户在设置中配置保留 30 天并触发清理
- **THEN** 系统删除 `token_logs` 中 `timestamp` 早于 30 天前的记录

#### Scenario: 清空全部数据
- **WHEN** 用户触发清空全部数据
- **THEN** 系统删除 `token_logs` 和 `file_offsets` 表中的所有记录，`app_settings` 保留

### Requirement: 设置持久化
系统 SHALL 使用 `app_settings` 表（key-value 结构）存储用户配置。预定义 key 包括：`enabled_agents`（JSON 数组）、`watch_mode`（`realtime` | `polling`）、`keep_days`（整数）、`polling_interval_secs`（整数）、`language`（`en` | `zh-CN`）。

#### Scenario: 读取默认设置
- **WHEN** 首次启动且 `app_settings` 表为空
- **THEN** 系统返回硬编码的默认值（所有 agent 启用、realtime 模式、30 天保留、10 秒轮询、英文语言）

#### Scenario: 更新设置
- **WHEN** 用户修改设置并保存
- **THEN** 系统使用 `INSERT OR REPLACE` 更新 `app_settings` 表中对应的 key
