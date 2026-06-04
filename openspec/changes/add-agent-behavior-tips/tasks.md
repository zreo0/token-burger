## 1. Agent 读取复用与行为解析边界

- [x] 1.1 保留现有 Agent adapter token 解析接口，在 watcher 正常监听阶段接入行为解析 fan-out，并明确完整 `AgentSource` / `TokenExtractor` 重构后续单独处理
- [x] 1.2 保持现有 Codex/OpenCode token 解析输出、入库和汇总行为不变，避免为了行为提示迁移 token parser
- [x] 1.3 为 watcher 定义单次新增数据 batch/fan-out 入口，使 Token 解析和行为解析共享同一批新增文件内容或 SQLite row
- [x] 1.4 新增 `behavior` Rust 模块，定义 `AgentBehaviorKind`、`AgentBehaviorEvent`、提示 key 生成和序列化结构；该模块不得拥有文件扫描、notify 注册、SQLite 查询或数据源发现职责
- [x] 1.5 实现 Codex 纯行为解析器，覆盖内部 `turn_started`、`permission_requested`、`permission_resolved`、`run_completed` 和 `run_aborted`
- [x] 1.6 实现 OpenCode 纯行为解析器，仅覆盖 `assistant finish = stop` 的 `run_completed`
- [x] 1.7 添加解析单元测试，覆盖现有 token 输出不回归、Codex 新轮次/权限请求/处理/完成/中断、OpenCode 完成、异常 JSON 跳过和敏感字段不外发

## 2. Watcher 接入与水位线

- [x] 2.1 在行为提示开关开启且 Codex Agent 已启用时，于 JSONL notify/reconcile 增量读取路径中从同一内容切片 fan-out 解析 Codex 行为事件，且不影响 token 解析和 offset 更新
- [x] 2.2 在冷启动历史扫描中避免弹出历史行为提示，只允许冷启动完成后的新增事件触发提示
- [x] 2.3 在 OpenCode Agent 已启用时复用现有 SQLite token 轮询结果解析完成事件，不启动独立行为轮询；如需 `time_updated` 才能覆盖更新记录，应升级同一增量查询供 token 与行为共同使用
- [x] 2.4 在行为解析入口前检查行为提示总开关和 Agent 启用状态，关闭期间跳过行为解析 fan-out，不缓存回放
- [x] 2.5 添加 watcher/轮询相关测试，验证关闭行为开关时不运行行为解析但 token logs 继续入库，关闭 Agent 时不运行该 Agent 的 token 监听或行为解析，且冷启动不产生历史提醒

## 3. 提示队列与生命周期

- [x] 3.1 实现后端 dispatcher，按事件 key 去重和更新提示，内存队列最多保留 10 条
- [x] 3.2 实现 FIFO 溢出规则：超过 10 条直接丢弃最旧提醒
- [x] 3.3 实现内部 `turn_started` 清理规则：同一 `agent_name + session_id` 的旧提醒全部移除
- [x] 3.4 实现权限清理规则：同一 `call_id` resolved 时移除仍存在的权限提醒
- [x] 3.5 实现自动隐藏规则：完成/中断 5 秒，错误 8 秒；权限不按时间自动隐藏
- [x] 3.6 添加 dispatcher 单元测试，覆盖多会话并发、队列溢出、手动关闭、新轮次清理和 resolved 清理

## 4. Tauri 轻量提示窗口

- [x] 4.1 新增 `behavior-tip` WebView 窗口创建/复用逻辑，窗口应无装饰、透明、置顶、不可调整尺寸
- [x] 4.2 缓存主托盘最近一次 rect，并在展示提示时按 macOS 菜单栏和 Windows 任务栏规则定位
- [x] 4.3 实现无 rect 时的平台兜底定位：macOS 右上角附近，Windows 右下角附近
- [x] 4.4 后端向 `behavior-tip` 窗口广播当前提示状态，并在队列为空时隐藏窗口
- [x] 4.5 展示 `behavior-tip` 时不得调用 `set_focus()`，不得激活或打断当前输入焦点
- [x] 4.6 Popup 已打开时仍展示 `behavior-tip`，并尽量避开 Popup 矩形
- [x] 4.7 确认现有 Popup 左键打开、冷启动门控、Settings 和 Quit 行为不变

## 5. 前端提示 UI

- [x] 5.1 新增 BehaviorTip 页面/组件，监听当前提示状态并渲染单条轻量提示
- [x] 5.2 设计提示视觉与现有深色 Popup 一致：圆角、半透明深色卡片、轻内描边、食材系状态色
- [x] 5.3 实现极简内容结构：Agent 名称、事件标题、短摘要和关闭按钮，不提供 `Open` 或跳转动作
- [x] 5.4 实现克制动效：状态点或状态条呼吸，窗口进入/退出只使用 opacity 与 transform
- [x] 5.5 添加前端测试，覆盖不同 kind、关闭回调、自动隐藏行为和长文本截断

## 6. 设置开关

- [x] 6.1 在 AppSettings 或设置存储中新增行为提示总开关，默认开启
- [x] 6.2 在 Settings UI 中新增行为提示总开关，不新增独立的行为 Agent 或事件类型细粒度设置
- [x] 6.3 添加设置读写测试，验证关闭期间不运行行为解析 fan-out、不入队、不弹窗，重新开启后不回放旧事件

## 7. 验证与收尾

- [x] 7.1 运行 Rust 单元测试，验证行为解析、dispatcher、设置开关和 watcher 接入
- [x] 7.2 运行前端测试与 lint，验证 BehaviorTip 组件、Settings 开关和现有 Popup 不回归
- [ ] 7.3 手动验证 macOS 菜单栏附近定位、无 rect 兜底定位和不抢焦点
- [ ] 7.4 在 Windows 或可替代环境中验证任务栏附近定位、透明窗口表现和不抢焦点
- [x] 7.5 检查行为 payload 脱敏，确认不包含 prompt、完整工具输出、完整命令参数、长路径或凭据值
