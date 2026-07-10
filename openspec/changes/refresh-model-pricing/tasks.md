## 1. 后端定价刷新能力

- [x] 1.1 扩展定价模块，提供有效当天缓存检查和强制远程拉取并缓存接口
- [x] 1.2 将 AppState 价格表改为 RwLock，并增加刷新互斥与安全的读写更新逻辑
- [x] 1.3 实现 ensure_pricing_fresh、reload_pricing commands 与 pricing-updated 事件
- [x] 1.4 启动每小时后台新鲜度检查，并注册新增 commands

## 2. 前端运行时更新

- [x] 2.1 Popup 在展示时检查价格新鲜度，并监听 pricing-updated 重新读取价格表
- [x] 2.2 Settings 数据页增加模型价格强制刷新按钮及加载、成功、失败状态
- [x] 2.3 补充刷新结果类型和中英文国际化文案

## 3. 测试

- [x] 3.1 补充 Rust 定价缓存检查、强制刷新辅助逻辑和并发状态相关单元测试
- [x] 3.2 补充 Popup 价格更新事件与 Settings 手动刷新交互测试

## 4. 验证

- [x] 4.1 执行 cargo fmt、cargo test、前端测试、TypeScript/构建检查并修复问题
