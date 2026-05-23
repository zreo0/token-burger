## MODIFIED Requirements

### Requirement: 冷启动加载状态
冷启动期间，系统 SHALL 避免通过主托盘左键展示 Popup 内容。主托盘扫描态 SHALL 作为用户可见的 token 数据加载状态；Popup 即使被预热或后台创建，也不得在冷启动完成前因用户点击主托盘而展示未完成的 token summary。冷启动完成后，Popup SHALL 恢复展示完整内容。

#### Scenario: 冷启动进行中
- **WHEN** 冷启动尚未完成且用户左键点击主托盘
- **THEN** Popup 不展示，用户仅通过主托盘扫描态获知 token 数据仍在加载

#### Scenario: 冷启动完成
- **WHEN** 冷启动完成后用户左键点击主托盘
- **THEN** Popup 展示完整的摘要、Burger、Top Models 和相关状态内容

#### Scenario: 后台预热不展示未完成数据
- **WHEN** Popup 窗口在冷启动期间被后台预热或已存在但不可见
- **THEN** 系统不得因为主托盘左键点击而显示该窗口
