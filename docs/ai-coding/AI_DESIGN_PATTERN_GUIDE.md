# AI 设计模式规范

## 1. 总原则

- 先用本项目已有的模式，不为假想的需求加抽象。
- 小文件、单一职责；能写成纯函数的就写成纯函数。

## 2. 本项目既有模式

| 模式 | 在哪里 | 怎么用 |
|---|---|---|
| 接口 + 假实现 | `LinearApi`、`JobRunner`、`ReadinessProbe`、`CommandRunner` | 生产代码依赖 trait；测试用 `FakeLinear`、`FakeLinearServer`、`ScriptedRunner`、`FakeRunner` |
| 纯函数决策 | `trigger::evaluate`、`trigger::should_enqueue`、`scheduler::pick`、`engine::decide`、`prompt::render`、`executor::build` | 不做 IO（`trigger::evaluate` 只额外检查绑定目录是否存在），单元测试覆盖每个分支 |
| 单一编排者 | `Engine` | 运行状态只在 `Engine` 里；命令行、MCP、面板都是它的客户端 |
| 记录先行 | `store::mark_started` 在启动子进程之前 | 先写 SQLite，再做外部动作；重启后能恢复 |
| 薄 IPC | `ipc::server` 的分发函数 | 只做参数解析和分发，业务规则留在 `Engine` |
| 进程树守卫 | `runner::run` | 进程组 / Job Object 加 `KillOnDrop`，结束时总是清理整棵树 |

## 3. 反模式（禁止）

- 只有一个实现、测试里也不需要替换时，不要造 trait。
- 全局可变单例、`static mut`。
- 在纯逻辑层读文件、看系统时间、发网络请求。
- 持有 `std::sync::Mutex` 守卫时 `.await`；在 `match` / `for` 的表达式里加锁，又在分支里再加锁（会死锁）。先把查询结果绑定到局部变量。
- 把外部文本（需求内容、路径、用户输入）拼进子进程参数或 shell 命令。
- 吞掉错误：`let _ =` 只用于清理路径（结束进程、删除临时文件、写日志）。

## 4. 扩展指引

- **新增执行者**：`ExecutorKind` 加变体 → `executor::build` 加参数 → `outcome::observe` 识别最终消息 → `doctor::check_executor` 加检查 → 各自的测试 → 实机验收。
- **新增 IPC 方法**：`METHODS` → `server` 的分发 → 调用方（命令行 / MCP）→ 测试。
- **新增 Linear 工具**：先用 M0 探针取真实样本 → `REQUIRED_TOOLS` → `parse.rs` → `McpLinear` → 测试用假服务端。
