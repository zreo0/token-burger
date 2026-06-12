## 1. 后端 Agent 接入

- [x] 1.1 新增 `src-tauri/src/adapters/mimocode.rs`，实现独立 `MiMoCodeAdapter`
- [x] 1.2 实现 MiMoCode 数据库路径解析，顺序为 `MIMOCODE_DB`、`MIMOCODE_HOME/data/mimocode.db`、默认 XDG data 路径
- [x] 1.3 实现 MiMoCode SQLite `message` 查询，优先使用 `time_updated` watermark，缺失时回退 `time_created`
- [x] 1.4 将 `mimocode` 注册到内置 Agent pipeline，并加入默认 enabled agents

## 2. Token 与行为解析

- [x] 2.1 实现 MiMoCode token extractor，只解析 assistant row
- [x] 2.2 将 MiMoCode `tokens.input`、`tokens.cache.read`、`tokens.cache.write`、`tokens.output` 映射到现有 TokenLog
- [x] 2.3 将 MiMoCode `tokens.reasoning` 合并到 output，并在 metadata 保留原始 reasoning 数值
- [x] 2.4 确保 MiMoCode 正数 cost 只写入同一 message 的 input TokenLog
- [x] 2.5 新增 MiMoCode behavior parser，只从 assistant `finish = "stop"` 生成 `run_completed`
- [x] 2.6 确保 MiMoCode 第一版不生成 `permission_requested` 行为事件

## 3. 前端与图标资源

- [x] 3.1 在 behavior-tip 前端接入 MiMoCode label、provider icon 和完成文案 i18n
- [x] 3.2 使用已有 `src/assets/provider-icons/xiaomimimo.svg` 生成 `src-tauri/icons/provider-menubar/xiaomimimo.pdf`
- [x] 3.3 在 tray provider menubar icon 映射中接入 MiMoCode 图标
- [x] 3.4 补充 BehaviorTip 前端单元测试，覆盖 MiMoCode label、icon 和完成文案映射

## 4. 验证

- [x] 4.1 为 MiMoCode token parser 添加 Rust 单元测试，覆盖 assistant、非 assistant、reasoning 合并、cost 单次计入和异常 JSON
- [x] 4.2 为 MiMoCode SQLite 查询添加 Rust 单元测试，覆盖 watermark 字段选择和增量 row 查询
- [x] 4.3 为 MiMoCode behavior parser 添加 Rust 单元测试，覆盖完成事件与权限事件不解析
- [x] 4.4 运行相关 Rust 测试与前端测试，确认现有 OpenCode、Codex 行为不回归
