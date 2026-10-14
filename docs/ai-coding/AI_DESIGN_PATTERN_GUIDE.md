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
| 进程树守卫 | `runner::run` 的 `TreeGuard` | 结束时总是清理整棵树：Unix 是进程组以及所有后代进程，包括已离开该组的（顺着父进程查找，核对启动时间后才发信号），Windows 是 Job Object，另加 `KillOnDrop`。超时、卡住、取消先向进程组发 SIGTERM、给执行者 5 秒收尾；future 被丢弃时立即强制结束；正常退出后执行者已不在，只清理进程组里的残留 |
| 面板的后端接口 + 假后端 | `Backend`（`app/src/lib/backend.ts`）← `tauriBackend`（`api.ts`，唯一接触 Tauri）/ `createMockBackend`（`mock.ts`，数据在 `mock-fixtures.ts`） | 组件经 `useBackend()` 拿后端；`BackendProvider` 没有默认值，忘了包就直接报错。新能力先加进 `Backend` 和假后端，假后端照引擎的规则与报错文本 |
| 应用与测试共用的装配 | `createQueryClient()`、`renderWithProviders` | 测试跑的是应用自己的缓存规则，不另配一套 |
| 查询集中、事件刷新 | `app/src/hooks/queries.ts`（`queryKeys`）、`useEngineEvents`、`useLinearConnectionRefresh` | 键名在前、参数在后，按前缀刷新；引擎事件让对应查询失效，丢了事件就全部重读，不靠轮询 |
| 结构化数据得出标题 | `describeError`、`ErrorState`、`DetailLine`、需求页按 `WaitReasonView` 的代码翻译 | 标题来自代码或状态并翻译；外来原文清洗后只做细节行 |
| 共用用例表 | `*-cases.json`、`trigger-defaults.json`（清单见 `TESTING_SPEC.md`） | 面板重复 core 的规则时，一张表两边的测试共读 |
| 边界绊线测试 | `app/src-tauri/src/lib.rs`（命令表、应用清单、能力文件、解析出的权限、运行时加能力）、`app/src/lib/api.test.ts` | 改命令时它们一起失败，提醒同步改；不为让它们通过而放宽 |
| 记日志而不是吞错 | 应用 crate 的 `logging::warn_failed` / `debug_failed`；面板操作失败走 `reportFailure` | 每个被忽略的 `Result` 都记日志：托盘、窗口、存储 `warn`，发给可能已关闭的网页视图的事件 `debug` |

## 3. 反模式（禁止）

- 只有一个实现、测试里也不需要替换时，不要造 trait。
- 全局可变单例、`static mut`。
- 在纯逻辑层读文件、看系统时间、发网络请求。
- 持有 `std::sync::Mutex` 守卫时 `.await`；在 `match` / `for` 的表达式里加锁，又在分支里再加锁（会死锁）。先把查询结果绑定到局部变量。
- 把外部文本（需求内容、路径、用户输入）拼进子进程参数或 shell 命令。仅有两处例外：① 一键配置命令（`setup.rs`）唯一的可变参数是 IssueRelay 自身命令行程序的绝对路径，放在 `--` 之后；② 读取执行者环境（`env.rs`）时运行用户的登录 shell（`$SHELL -l -i -c` 加一段固定脚本，不含任何外部文本），它在自己的会话里运行，读到结束标记或满 10 秒即被结束。
- 吞掉错误：`let _ =` 只用于清理路径（结束进程、删除临时文件、写日志）；应用 crate 里改用 `logging::warn_failed` / `debug_failed`。
- 面板里：在 `api.ts` 以外引入 `@tauri-apps/api`；组件自己建 `QueryClient` 或绕过 `useBackend()` 直接调命令；把外来原文当标题，或用 `dangerouslySetInnerHTML`；复制一份 core 的规则表或样本；写死颜色值、像素值或任何语言的文案；为面板另引入一套全局状态库（引擎给的数据都放查询缓存）。

## 4. 扩展指引

- **新增执行者**：`ExecutorKind` 加变体 → `executor::build` 加参数 → `outcome::observe` 识别最终消息 → `doctor::check_executor` 加检查 → 各自的测试 → 实机验收。
- **新增 IPC 方法**：`METHODS` → `server` 的分发 → 调用方（命令行 / MCP）→ 测试。
- **新增 Linear 工具**：先用 M0 探针取真实样本（包括 `tools/list` 的参数表）→ `REQUIRED_TOOLS` 与 `REQUIRED_ARGUMENTS`（用到 `fields` 的再加 `REQUIRED_FIELDS`）→ `parse.rs`（读不懂就报错，不跳过）→ `McpLinear` → 更新 `crates/core/tests/fixtures/linear-tools.json`，假服务端按它校验每次调用 → 测试。
- **新增面板命令**：按 `AI_CODING_GUIDE.md` 第 7 节「面板命令」：`commands.rs` → `lib.rs` 的 `generate_handler!` → `build.rs` 的 `PANEL_COMMANDS` → `capabilities/default.json` 的 `allow-<命令>` → `backend.ts`、`api.ts`、`mock.ts`（类型变了再改 `types.ts`、`mock-fixtures.ts`）→ 查询或操作钩子 → 测试。需要插件的事只在命令里做。
- **新增页面**：`app/src/pages/` 加页面 → `App.tsx` 加路由 → `AppShell` 的 `PAGES` 加入口，`nav.<名字>` 五种语言都加 → 加载、空、错误三种状态与错误状态测试 → 在 620 与 605 px 两个内容宽度下量一遍。
- **面板要重复 core 的规则**：先写 JSON 用例表 → TS 实现与读表的测试 → app crate 里读同一张表、对照 core 函数的 Rust 测试（`TESTING_SPEC.md`「面板与 core 共用的用例表」）。
