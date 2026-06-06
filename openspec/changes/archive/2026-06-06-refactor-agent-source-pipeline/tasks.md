## 1. 回归基线与现状盘点

- [x] 1.1 盘点当前 `AgentAdapter`、watcher 策略、OpenCode `query_db_batch`、Codex model cache 和行为提示 fan-out 的调用链
- [x] 1.2 为 Codex、OpenCode、Claude Code、Gemini CLI 的现有 token 解析添加或补齐等价测试 fixture，覆盖 `TokenLog` 关键字段
- [x] 1.3 为 Codex 权限/完成/中断和 OpenCode 完成事件补齐行为解析基线测试
- [x] 1.4 添加 watcher 级回归测试，覆盖 Agent 关闭不监听、行为提示关闭不解析、冷启动不产生行为提示
- [x] 1.5 记录当前 Tauri command/event payload 和 `TokenLog` 结构，确认本 change 不需要前端或数据库 schema 迁移

## 2. 新增 Agent Source Pipeline 类型

- [x] 2.1 新增 Agent source 内部模块，定义 source 标识、数据源类型、监听路径/SQLite 位置和 source key
- [x] 2.2 新增 `AgentDataBatch` 或等价结构，覆盖 JSONL 增量、JSON 全量和 SQLite row 三类输入
- [x] 2.3 新增 `TokenExtractor` trait，使解析器只消费 batch 并返回 token logs 与可选解析状态元数据
- [x] 2.4 新增 `BehaviorExtractor` trait，使行为解析器只消费 batch 并返回 `Vec<AgentBehaviorEvent>`
- [x] 2.5 确认无需旧 adapter 兼容包装层，直接迁移并移除旧入口
- [x] 2.6 添加 source key、batch watermark 和解析器空实现的单元测试

## 3. Watcher 读取层迁移

- [x] 3.1 将 JSONL notify 策略调整为读取一次新增内容并生成 JSONL batch，再分发给 token/behavior 消费者
- [x] 3.2 保留并测试 JSONL offset 断点续传、文件轮转和 Codex model cache 恢复逻辑
- [x] 3.3 将 JSON polling 策略调整为 mtime 变化后生成 JSON 全量 batch，再分发给 token 消费者
- [x] 3.4 将 SQLite polling 策略调整为 source 查询 row batch，再分发给 token/behavior 消费者
- [x] 3.5 将冷启动路径调整为 batch token-only 处理，明确跳过 Behavior extractor
- [x] 3.6 确保 watcher 仍只接收已启用 Agent source，`enabled_agents` 变更后重启逻辑不变
- [x] 3.7 确保行为提示总开关关闭时不调用 Behavior extractor，且不缓存关闭期间事件

## 4. Agent 解析器迁移

- [x] 4.1 将 Codex token 解析迁移为 Token extractor，保持 token 输出和 model cache 语义不变
- [x] 4.2 将 Codex 行为解析接入 Behavior extractor，复用同一 JSONL batch
- [x] 4.3 将 OpenCode SQLite 查询迁移为统一 row batch，支持 `time_updated` 优先和 `time_created` fallback
- [x] 4.4 将 OpenCode token 解析迁移为 Token extractor，保持新版 SQLite 与旧版 JSON fallback 行为不变
- [x] 4.5 将 OpenCode 完成事件解析接入 Behavior extractor，复用同一 SQLite row batch
- [x] 4.6 将 Claude Code token 解析迁移为 Token extractor，保持 JSONL 行级容错不变
- [x] 4.7 将 Gemini CLI token 解析迁移为 Token extractor，保持 JSON 全量解析和 mtime 语义不变
- [x] 4.8 移除或收敛不再需要的旧 `parse_content`、`query_db`、`query_db_batch` 长期入口和孤儿代码

## 5. 下游兼容与隔离

- [x] 5.1 保持 `WriteRequest`、token 入库流程、`token-updated` 广播和 tray title 更新行为不变
- [x] 5.2 保持 Popup 查询、Settings Agent 开关、行为提示总开关和现有 Tauri command 名称不变
- [x] 5.3 确保行为 dispatcher、`behavior-tip` 窗口和前端提示组件无需改动或只做类型适配
- [x] 5.4 确认本 change 不新增数据库业务表、不修改 `TokenLog` TypeScript/Rust 序列化结构
- [x] 5.5 清理因重构产生的无用 import、兼容包装和未使用 helper，不改动无关模块

## 6. 验证与收尾

- [x] 6.1 运行 `cargo fmt` 并检查 Rust 格式
- [x] 6.2 运行 `cargo test`，覆盖 adapter、watcher、OpenCode SQLite、Codex model cache 和行为解析
- [x] 6.3 运行前端测试，确认行为提示和 Settings 不回归
- [x] 6.4 运行 lint 和 build，确认 Tauri/React 构建链路不回归
- [x] 6.5 运行 `openspec validate refactor-agent-source-pipeline --strict`
- [x] 6.6 手动烟测 token 统计、Popup 打开、Agent 开关重启 watcher、Codex 权限提示和 OpenCode 完成提示
