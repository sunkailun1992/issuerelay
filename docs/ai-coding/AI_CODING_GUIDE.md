# IssueRelay AI 编程指南

## 1. 项目定位

IssueRelay 是每个开发者装一份的托盘程序：监听 Linear 上指派给自己、带 `ai-ready`、状态为 Todo 的需求，每条新起一次 Claude Code 或 Codex 无人值守运行。程序只负责发现需求、派活、核对结果、留记录；读需求、改代码、git、推送、改 Linear 状态都由 AI 完成，唯一例外是面板上由用户点击的「标记完成」。

不做的事见设计 2.2：推送进已打开的 AI 会话、webhook、多个 Linear 工作区、AI 专用账号、Linux 安装包。

## 2. 构建与运行

| 目的 | 命令 |
|---|---|
| 构建 | `cargo build --workspace` |
| 测试 | `cargo test --workspace`（会自动编译测试用的 `fake-agent`） |
| 格式与静态检查 | `cargo fmt --all --check`；`cargo clippy --workspace --all-targets -- -D warnings` |
| 开发宿主 | `cargo run -p issuerelay-devhost -- login`（或 `api-key`、`logout`、`projects`、`run`） |
| 命令行 | `cargo run -p issuerelay-cli -- status`（宿主运行时才有响应） |
| 前端依赖 | `pnpm --dir app install` |
| 前端构建 | `pnpm --dir app build`（产物 `app/dist`；应用 crate 的发布构建内嵌它，所以排在 cargo 命令之前） |
| 前端检查与测试 | `pnpm --dir app lint`；`pnpm --dir app test` |
| 桌面应用开发版 | `pnpm --dir app tauri dev`；只在浏览器里看面板（假数据）：`pnpm --dir app dev` |

工具链由 `rust-toolchain.toml` 固定为 1.99.0；前端要 Node.js 22 以上，pnpm 版本由 `app/package.json` 的 `packageManager` 固定。

## 3. 分层

| 层 | 模块 | 职责 | 可以依赖 |
|---|---|---|---|
| 基础 | `core::paths`、`core::config`、`core::text` | 位置、设置、绑定；清洗要展示的外来文本 | 无 |
| 纯逻辑 | `core::model`、`trigger`、`scheduler`、`prompt`、`executor`（拼命令）、`executor::outcome` | 判断与生成，不做 IO | 基础层 |
| 外部接入 | `core::store`（SQLite）、`runner`（子进程）、`linear`（MCP）、`doctor`（检查命令） | 与外部世界交互 | 基础层、纯逻辑层 |
| 编排 | `core::engine` | 唯一持有运行状态的地方 | 以上全部 |
| 通信 | `core::ipc` | 本机 JSON-RPC，薄层 | `engine` |
| 入口 | `core::host`（宿主组装）、`crates/cli`、`crates/devhost`、（M2）`app/` | 用户与 AI 的入口 | `core`；`core::host` 在 `core` 内，依赖以上全部 |

规则：纯逻辑层不读文件、不看系统时间、不发请求（例外：`trigger::evaluate` 检查绑定目录是否存在）；运行状态只在 `engine`；命令行只经 IPC 访问引擎。

## 4. 关键数据流

1. `engine::poll_once` 调 `LinearApi::candidate_issues`，取「指派给我 + 标签 + Todo」的需求。
2. `trigger::evaluate` 判断能否派活（项目已绑定、目录存在、执行者就绪），`trigger::should_enqueue` 防止重复。
3. 新需求写入 `store`，状态为「排队中」或「等待绑定」。
4. `engine::dispatch` 用 `scheduler::pick` 选出可以开始的任务（同目录一次一条，总数不超上限）。
5. 开跑前重新读取需求，不再满足条件就取消。
6. `prompt::render` 与 `executor::build` 生成命令，`runner::run` 启动并记录日志。
7. 结束后再次读取需求，`engine::decide` 结合退出码、Linear 状态和 AI 的报告决定结果，写回 `store`。
8. 任务每次变化、每次检查结束、配置每次保存，引擎都经 `Engine::subscribe` 发出 `EngineEvent`；面板（M2）在进程内订阅它刷新界面，落后太多（`Lagged`）时重新读取全部状态。

## 5. 关键文件索引

| 要改的事 | 文件 |
|---|---|
| 默认设置、绑定字段 | `crates/core/src/config.rs` |
| 触发条件、防重复 | `crates/core/src/trigger.rs` |
| 排队规则 | `crates/core/src/scheduler.rs` |
| 提示词与固定规则 | `crates/core/src/prompt.rs` |
| 执行者参数、权限档位 | `crates/core/src/executor/mod.rs` |
| 最终结果识别 | `crates/core/src/executor/outcome.rs` |
| 子进程、超时、结束进程树 | `crates/core/src/runner.rs` |
| 日志清理 | `crates/core/src/logs.rs` |
| Linear 工具调用与解析 | `crates/core/src/linear/mcp.rs`、`parse.rs` |
| Linear 授权与钥匙串 | `crates/core/src/linear/auth.rs` |
| 环境检查 | `crates/core/src/doctor.rs` |
| 一键配置命令（Linear MCP、控制服务注册） | `crates/core/src/setup.rs` |
| 执行者环境（PATH、代理） | `crates/core/src/env.rs` |
| 状态流转与编排 | `crates/core/src/engine.rs` |
| 宿主组装（IPC、后台循环、Linear 连接切换；启动不等 Linear，首次连接在后台） | `crates/core/src/host.rs` |
| IPC 方法表 | `crates/core/src/ipc/protocol.rs` |
| 命令行与 MCP 控制服务 | `crates/cli/src/main.rs`、`mcp.rs`、`format.rs` |
| 外来文本清洗（Linear 标题、AI 摘要、错误信息）| `crates/core/src/text.rs`、`crates/cli/src/format.rs` |

## 6. 运行数据与配置

| 数据 | 位置 | 说明 |
|---|---|---|
| 设置与绑定 | `<配置目录>/com.sunkailun.issuerelay/config.toml` | TOML；先写临时文件再改名；每次保存前清扫同目录下超过 10 分钟的残留临时文件（`config.toml.<pid>.<n>.tmp`，保存中途崩溃留下的）；加载时对本版本不认识的配置键在日志里警告一次（只记键名，不记值），下次保存会丢掉它们 |
| 运行记录 | `<数据目录>/com.sunkailun.issuerelay/state.db` | SQLite；表结构版本记在 `user_version`（当前 2），比当前程序新的拒绝打开，没记版本的旧库（版本 0）按实际缺的列补齐；macOS 上每次运行记下执行者的进程号和启动时间，运行正常结束时清除，宿主崩溃后留到下次启动清扫处理完才清除 |
| 运行日志 | `<数据目录>/com.sunkailun.issuerelay/logs/<任务 ID>.jsonl` | 执行者的原始输出，一行一条：标准输出原样，标准错误记为 `{"stream":"stderr","text":…}`，启动失败记为 `{"stream":"runner","text":…}`；运行器每次打开日志都把 Unix 上的权限设为 0600（重试沿用的旧文件也一样）；单行最多保留 4 MiB，更长的被截断并记一条说明；`logs.rs` 按保留天数（`log_retention_days`，默认 30）和总大小（`log_max_total_mb`，默认 1024）清理，宿主启动时和之后每 6 小时一次，正在运行的任务的日志不删 |
| Linear 凭据 | 系统钥匙串：服务名 `com.sunkailun.issuerelay`，账户 `linear-oauth` / `linear-api-key` | 不写盘；改用 API key 时清掉 OAuth 凭据，OAuth 登录时清掉 API key（`auth::clear_oauth_credentials`、`clear_api_key`） |
| IPC 端点 | macOS：数据目录下的 `issuerelay.sock`，路径超过 103 字节时改用 `<临时目录>/issuerelay-<哈希>/issuerelay.sock`；Windows：`\\.\pipe\issuerelay-<哈希>`（哈希取自数据目录） | 只限当前用户 |
| 单实例锁 | macOS：套接字所在目录下的 `issuerelay.lock`（0600，运行中的宿主持有排他锁，进程退出时由系统释放，文件本身不删）；Windows：靠管道的 `first_pipe_instance` | 第二个宿主启动时报「已在运行」；`Host::stop` 把本机通信放在最后停，锁要等运行都结束、结果记录完才释放 |

`<配置目录>` 取自 `dirs::config_dir()`，`<数据目录>` 取自 `dirs::data_local_dir()`：macOS 都是 `~/Library/Application Support`；Windows 分别是 `%APPDATA%`（随账户漫游）和 `%LOCALAPPDATA%`（只在本机，运行记录和日志不随账户同步）。这些都只在用户本机，不进仓库。

## 7. 外部接口变更规则

- **Linear MCP**：工具名、参数、返回格式以 M0 样本（`docs/gates/samples/m0/linear/`）为准。Linear 改了格式时，先更新脱敏样本和 `parse.rs` 的测试，再改代码；增删工具或参数要同步 `REQUIRED_TOOLS` / `REQUIRED_ARGUMENTS`（`fields` 的取值同步 `REQUIRED_FIELDS`）、`crates/core/tests/fixtures/linear-tools.json`（M0 `tools.json` 的副本，假服务端按它校验每次调用）和测试。
- **Claude Code / Codex 参数**：以验收报告记录的版本为准。参数变化时同时改 `executor::build`、它的测试和设计 6.1 的表；输出事件格式变化时改 `outcome::observe` 和样本测试。
- **IPC 方法表**：改 `METHODS` 必须同步 `server` 的分发、命令行、MCP 工具和测试；「标记完成」「重试」「取消」不得加入。

## 8. 外部事实与夹具

- 外部事实（命令行参数、输出格式、接口行为）写清日期和来源版本，例如「codex-cli 0.159.2（2026-10-10）的 `--approve-for-me`」，写在代码注释或验收报告里。
- 来自真实运行的夹具必须脱敏后才能提交，规则见 `TESTING_SPEC.md`。

## 9. 里程碑与文档流

设计（`docs/superpowers/specs/`）→ 实施计划（`docs/superpowers/plans/`）→ 按任务实现 → 实机验收报告（`docs/gates/`）→ 推送。里程碑：M0 实测验证、M1 引擎与命令行、M2 桌面应用、M3 安装与发布。

## 10. 版本

- 语义化版本，由工作区 `[workspace.package] version` 统一管理。应用的版本也只在这里：`app/src-tauri/tauri.conf.json` 不写 `version`，安装包的版本和应用对外报告的版本（`CARGO_PKG_VERSION`）都跟随 Cargo；`app/package.json` 的 `version` 只是私有前端包的版本，发布时不动。
- 只有运行行为变化才升版本；只改文档、测试、规范不升。
- 版本号只由维护者在发布时升（外部贡献者不改）；升版本时在推送前按 `origin/main` 的最新值顺延，开工时算的号不作数。
- M3 起推送 `vX.Y.Z` 标签触发发布。

## 11. 分支与提交

- 一个任务一个提交；提交前跑第 2 节的检查和 `bash scripts/check-secrets.sh --strict`。
- 提交信息用英文 Conventional Commits：`feat|fix|docs|test|chore|refactor|ci(<crate 或模块>): <做了什么>`；AI 参与时结尾加 `Co-Authored-By` 行。
- 开工、提交前、推送前各 `git fetch`；远端已前进就 rebase 后重跑检查；禁止强推。
