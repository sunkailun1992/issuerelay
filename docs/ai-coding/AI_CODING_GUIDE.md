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
| 前端构建 | `pnpm --dir app build`（产物 `app/dist`；`tauri build`（打开 `custom-protocol` 特性）构建的应用内嵌它，调试构建、`cargo test` 与 `cargo clippy` 不内嵌；与 CI 一样排在 cargo 命令之前） |
| 前端检查与测试 | `pnpm --dir app lint`（含 `tsc` 类型检查）；`pnpm --dir app test` |
| 面板截图与版面检查 | `pnpm --dir app test:screens`：把面板的关键屏拍下来，与 `docs/gates/screenshots/m2/` 的基准图比对（容差见 `TESTING_SPEC.md`），再量最小窗口下的版面；界面有意改动之后，用 `pnpm --dir app screenshots` 重写不再相符的基准图，换了机器或 Chrome 的大版本用 `pnpm --dir app exec playwright test --update-snapshots=all` 整套重拍（它也重写 README 用的四张图，单独重拍那四张用 `pnpm --dir app readme-screenshots`）。只在本机跑，不进 CI（字体随系统不同，换一台机器、CI 的机器上比对不稳定）；用本机装的 Google Chrome，不下载浏览器 |
| 桌面应用开发版 | `pnpm --dir app tauri dev`；只在浏览器里看面板（假数据）：`pnpm --dir app dev`，地址加 `?mock=fresh` 是首次运行，加 `?mock=connecting` 是首次连接 Linear 还没结束，加 `?lang=zh-CN`（或别的语言代码）让设置里的界面语言是它 |
| 本机安装包（macOS） | `bash scripts/stage-cli.sh && pnpm --dir app build && node scripts/release-checks.mjs panel && CI=true pnpm --dir app tauri build --bundles app,dmg --config src-tauri/tauri.bundle.conf.json`：产物在 `target/release/bundle/`（git 忽略）；`CI=true` 让打包器跳过给 dmg 窗口排版的访达脚本，免得弹出「控制访达」的授权框；打包配置去掉了 `beforeBuildCommand`，`tauri build` 不会再自己构建面板，所以面板要像命令里那样先构建并过 `panel` 门禁 |

工具链由 `rust-toolchain.toml` 固定为 1.99.0；前端要 Node.js 22.22.2+、24.15+ 或 26 及以上（`app/package.json` 的 `engines`），pnpm 版本由 `app/package.json` 的 `packageManager` 固定。

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
| 把命令行加入 PATH、应用安装位置 | `crates/core/src/cli_path.rs` |
| 状态流转与编排 | `crates/core/src/engine.rs` |
| 宿主组装（IPC、后台循环、Linear 连接切换；启动不等 Linear，首次连接在后台） | `crates/core/src/host.rs` |
| IPC 方法表 | `crates/core/src/ipc/protocol.rs` |
| 命令行与 MCP 控制服务 | `crates/cli/src/main.rs`、`mcp.rs`、`format.rs` |
| 外来文本清洗（Linear 标题、AI 摘要、错误信息）| `crates/core/src/text.rs`、`crates/cli/src/format.rs` |
| 日志逐行清洗（devhost 与桌面应用共用） | `crates/core/src/logscrub.rs` |
| 托盘、通知、退出状态机、启动状态 | `app/src-tauri/src/tray.rs`、`notify.rs`、`quit.rs`、`state.rs`、`startup.rs` |
| 自动更新（检查、验签、经退出流程安装） | `app/src-tauri/src/update.rs` |
| 面板命令（含仅面板可用的标记完成、重试、停止） | `app/src-tauri/src/commands.rs` |
| 面板能调什么（应用清单、能力文件） | `app/src-tauri/build.rs`、`app/src-tauri/capabilities/default.json` |
| 前端数据层、假后端与虚构数据 | `app/src/lib/` |
| 面板唯一接触 Tauri 的地方、后端接口 | `app/src/lib/api.ts`、`app/src/lib/backend.ts` |
| 面板的查询、操作与事件刷新 | `app/src/hooks/queries.ts`、`useEngineEvents.ts`、`app/src/lib/query-client.ts` |
| 面板显示文本清洗、失败的显示 | `app/src/lib/clean.ts`（用例表 `clean-cases.json`）、`app/src/lib/errors.ts`、`app/src/components/states.tsx` |
| 翻译文件与语言解析（面板与托盘共用） | `app/src/i18n/`、`app/src-tauri/src/i18n.rs` |
| 设计变量、主题与状态标识 | `app/src/styles.css`（`styles.test.ts` 核对）、`app/src/lib/theme.ts`、`app/src/components/StatusBadge.tsx` |

## 6. 运行数据与配置

| 数据 | 位置 | 说明 |
|---|---|---|
| 设置与绑定 | `<配置目录>/com.sunkailun.issuerelay/config.toml` | TOML；先写临时文件再改名；每次保存前清扫同目录下超过 10 分钟的残留临时文件（`config.toml.<pid>.<n>.tmp`，保存中途崩溃留下的）；加载时对本版本不认识的配置键在日志里警告一次（只记键名，不记值），下次保存会丢掉它们 |
| 运行记录 | `<数据目录>/com.sunkailun.issuerelay/state.db` | SQLite；表结构版本记在 `user_version`（当前 2），比当前程序新的拒绝打开，没记版本的旧库（版本 0）按实际缺的列补齐；macOS 上每次运行记下执行者的进程号和启动时间，运行正常结束时清除，宿主崩溃后留到下次启动清扫处理完才清除 |
| 运行日志 | `<数据目录>/com.sunkailun.issuerelay/logs/<任务 ID>.jsonl` | 执行者的原始输出，一行一条：标准输出原样，标准错误记为 `{"stream":"stderr","text":…}`，启动失败记为 `{"stream":"runner","text":…}`；运行器每次打开日志都把 Unix 上的权限设为 0600（重试沿用的旧文件也一样）；单行最多保留 4 MiB，更长的被截断并记一条说明；`logs.rs` 按保留天数（`log_retention_days`，默认 30）和总大小（`log_max_total_mb`，默认 1024）清理，宿主启动时和之后每 6 小时一次，正在运行的任务的日志不删 |
| 桌面应用的程序日志 | `<数据目录>/com.sunkailun.issuerelay/logs/app.<日期>.log` | tracing 日志：每天一个（按 UTC 日期命名），保留 7 个（只数 `app.*.log`，运行日志不受影响）；每行先经 `logscrub::CleanWriter` 去掉控制字符与不可见字符；`RUST_LOG` 可改过滤，但 rmcp 不低于 info（它在 debug 级写出 OAuth 授权码）；调试构建同时写 stderr |
| Linear 凭据 | 系统钥匙串：服务名 `com.sunkailun.issuerelay`，账户 `linear-oauth` / `linear-api-key` | 不写盘；改用 API key 时清掉 OAuth 凭据，OAuth 登录时清掉 API key（`auth::clear_oauth_credentials`、`clear_api_key`） |
| IPC 端点 | macOS：数据目录下的 `issuerelay.sock`，路径超过 103 字节时改用 `<临时目录>/issuerelay-<哈希>/issuerelay.sock`；Windows：`\\.\pipe\issuerelay-<哈希>`（哈希取自数据目录） | 只限当前用户 |
| 单实例锁 | macOS：套接字所在目录下的 `issuerelay.lock`（0600，运行中的宿主持有排他锁，进程退出时由系统释放，文件本身不删）；Windows：靠管道的 `first_pipe_instance` | 第二个宿主启动时报「已在运行」；`Host::stop` 把本机通信放在最后停，锁要等运行都结束、结果记录完才释放 |
| 桌面应用单实例 | macOS：`/tmp/com_sunkailun_issuerelay_si.sock`；Windows：命名互斥量 `com.sunkailun.issuerelay-sim`（tauri-plugin-single-instance 2.5.3） | 第二次打开应用只把已运行的面板带到前面；与上一行宿主的单实例锁是两回事：开发宿主不经过它，两个宿主之间靠宿主的锁报「已在运行」 |
| 命令行的 PATH 入口 | macOS：`~/.local/bin/issuerelay`（指向应用包内命令行程序的符号链接）；Windows：用户级 `Path`（`HKCU\Environment`）里的安装目录 | 只在用户打开设置里的开关时建，关掉时删，且只删 IssueRelay 自己建的；macOS 不覆盖别人的文件，指向别份应用的旧链接换成指向这一份；Windows 保持原来的注册表类型、不分大小写去重；都不需要管理员（`cli_path.rs`） |
| 装不上的更新 | `<数据目录>/com.sunkailun.issuerelay/update-install-failure.txt` | 「安装并重启」在宿主停下后没装上时写下清洗过的一行原因，程序按原版本重启后读出、删除，面板顶部的横幅显示一次 |

`<配置目录>` 取自 `dirs::config_dir()`，`<数据目录>` 取自 `dirs::data_local_dir()`：macOS 都是 `~/Library/Application Support`；Windows 分别是 `%APPDATA%`（随账户漫游）和 `%LOCALAPPDATA%`（只在本机，运行记录和日志不随账户同步）。这些都只在用户本机，不进仓库。

## 7. 外部接口变更规则

- **Linear MCP**：工具名、参数、返回格式以 M0 样本（`docs/gates/samples/m0/linear/`）为准。Linear 改了格式时，先更新脱敏样本和 `parse.rs` 的测试，再改代码；增删工具或参数要同步 `REQUIRED_TOOLS` / `REQUIRED_ARGUMENTS`（`fields` 的取值同步 `REQUIRED_FIELDS`）、`crates/core/tests/fixtures/linear-tools.json`（M0 `tools.json` 的副本，假服务端按它校验每次调用）和测试。
- **Claude Code / Codex 参数**：以验收报告记录的版本为准。参数变化时同时改 `executor::build`、它的测试和设计 6.1 的表；输出事件格式变化时改 `outcome::observe` 和样本测试；面板的时间线读同样的输出和日志里 IssueRelay 自己的 `{"stream":…,"text":…}` 行（`app/src/lib/timeline.ts`，测试直接读 `crates/core/tests/fixtures/{claude-stream,codex-exec}.jsonl`，不复制），格式变了一并改。
- **IPC 方法表**：改 `METHODS` 必须同步 `server` 的分发、命令行、MCP 工具和测试；「标记完成」「重试」「取消」不得加入。
- **面板的类型**：Rust 侧给面板的类型变了（含 `StatusSnapshot`、`IssueStatus`、`StartupState`，以及 `quit-requested` 的载荷 `quit::QuitPrompt { running, waiting, purpose }`、`update-status` 与 `update_status`、`check_update` 给出的 `update::UpdateStatus`），同步改 `app/src/lib/types.ts`、`mock-fixtures.ts` 和假后端（`mock.ts`），以及读它们的组件（`QuitDialog`、`UpdateBanner`、设置页的版本一行）。
- **面板命令**：增删命令要同时改 `commands.rs`、`lib.rs` 的 `generate_handler!`、`build.rs` 的 `PANEL_COMMANDS`、`capabilities/default.json`（`allow-<命令>`，下划线写成连字符）和前端的 `backend.ts`、`api.ts`、`mock.ts`；`lib.rs` 与 `api.test.ts` 的测试拦住不一致。面板要用 Tauri 自带的其他接口（窗口、路径等）时，只在能力文件里加用到的那一条权限，并在提交说明里写明理由；插件权限不给，插件只在 Rust 侧用。
- **开发者工具**：调试构建里 Cmd+Opt+I（macOS）/ Ctrl+Shift+I（Windows）什么也不做，因为能力文件没有给 `core:webview:allow-internal-toggle-devtools`；需要时只在本机临时加上（`lib.rs` 与 `api.test.ts` 的边界测试会因此失败），不要提交；发布构建没有开发者工具。

## 8. 外部事实与夹具

- 外部事实（命令行参数、输出格式、接口行为）写清日期和来源版本，例如「codex-cli 0.159.2（2026-10-10）的 `--approve-for-me`」，写在代码注释或验收报告里。
- 来自真实运行的夹具必须脱敏后才能提交，规则见 `TESTING_SPEC.md`。

## 9. 里程碑与文档流

设计（`docs/superpowers/specs/`）→ 实施计划（`docs/superpowers/plans/`）→ 按任务实现 → 实机验收报告（`docs/gates/`）→ 推送。里程碑：M0 实测验证、M1 引擎与命令行、M2 桌面应用、M3 安装与发布。

## 10. 版本

- 语义化版本，唯一来源是工作区 `Cargo.toml` 的 `[workspace.package] version`，各 crate 都用 `version.workspace = true`。应用的版本也只在这里：`app/src-tauri/tauri.conf.json` 不写 `version`，安装包的版本和应用对外报告的版本（`CARGO_PKG_VERSION`）都跟随 Cargo；`app/package.json` 的 `version` 只是私有前端包的版本，发布时不动。
- 另有三处写着对 `issuerelay-core` 的版本要求：`crates/cli`、`crates/devhost`、`app/src-tauri` 的 `Cargo.toml`（现在是 `"0.1.0"`，按 Cargo 的规则即 `^0.1.0`）。新版本不再满足它时（0.x 阶段升次版本号，或升主版本号），这三处一起改，否则 Cargo 解析不了；`Cargo.lock` 随之更新、一起提交。
- 版本号只由维护者在发布时升。外部贡献者的 PR 不改任何版本号（`Cargo.toml`、`Cargo.lock` 里本仓库 crate 的版本、`app/package.json`）。
- 发布时，上次发布以来有运行行为变化才升版本，一次发布只升一次；只改了文档、测试、规范不升。
- 升版本的提交同样经 PR 进 `main`。推送前按 `origin/main` 的最新值顺延，开工时算的号不作数；PR 里的号在合入前都是暂定，合入时已被占用就再顺延，并同步改 PR 标题、提交信息和文档里写到的版本号。PR 里写明旧版本、新版本和这次发布包含的行为变化。
- 版本号只用 `X.Y.Z` 三段数字，不带 `-rc.1` 这类预发布后缀（M3 起打的 Windows MSI 安装包不接受）。
- M3 起推送 `vX.Y.Z` 标签触发发布（`.github/workflows/release.yml`）：标签必须等于工作区版本、标签提交必须在 `origin/main` 上（`scripts/release-checks.mjs`，试打包同样检查），工作流只建草稿，步骤见 `MAINTAINERS.md`「发布」。

## 11. 分支与提交

- 分支与 PR 的做法见 `CONTRIBUTING.md`「分支、提交与 PR」，合并规则见 `MAINTAINERS.md`「决策与合并」，本地与云端的同步见 `AI_ENGINEERING_GUARDRAILS.md` 第 7.3 节；禁止强推。
- 一个任务一个提交；提交前跑第 2 节的检查和 `bash scripts/check-secrets.sh --strict`，再用 `git status --short`、`git diff --stat` 确认只提交了这次相关的文件。
- 提交信息用英文 Conventional Commits：`feat|fix|docs|test|chore|refactor|ci(<crate 或模块>): <做了什么>`，正文写做了什么、为什么、怎样验证的。
- AI 参与时，结尾加工具给出的 `Co-Authored-By` 行；其中的模型名写实际参与这次改动的模型（子智能体写的提交写子智能体的模型），不照抄别的提交。
- 规则改动单独提交：在 `AGENTS.md`、`docs/ai-coding/`、`CONTRIBUTING.md`、`MAINTAINERS.md`、`.github/pull_request_template.md` 里新增、收紧或放宽规则，用一个只含这些文件的 `docs` 提交（例如 `docs(ai-coding): …`），不与代码改动混在一起，规则的变化在历史里才查得到、也能单独回滚。代码改动顺带要同步的事实登记（目录表里加新文件、关键文件索引、外部事实的日期与版本）跟代码在同一个提交里。

## 12. 桌面面板（`app/`）

托盘程序的 Rust 外壳在 `app/src-tauri`，面板是 `app/src` 里的 React 页面。面板只是引擎的又一个客户端：不持有运行状态，任务的状态和原因都取自引擎。本节是写面板时的规则；网页视图的安全边界见 `SECURITY_CODING_SPEC.md` 第 3、5 节，测试见 `TESTING_SPEC.md`「前端测试规则」。

### 12.1 与 Tauri 的边界

- 只有 `app/src/lib/api.ts` 引入 `@tauri-apps/api`，而且只用 `invoke` 和 `listen`；别处都经 `backend.ts` 的 `Backend` 接口、用 `useBackend()` 拿后端。别的文件碰 Tauri，或调了能力文件没给的命令，`api.test.ts` 就失败。
- 增删命令按第 7 节「面板命令」一条同时改所有位置；Rust 侧给面板的类型变了，按「面板的类型」一条同步。
- 插件（打开链接、选目录、通知、开机自启）只在 Rust 侧的命令里用。命令出错时返回清洗过的一行文字，或 `backend.ts` 的 `ERROR_CODES` 里的一个代码，面板按 `errors.<代码>` 翻译。

### 12.2 数据

- 读数据用 `app/src/hooks/queries.ts` 里的查询。键在 `queryKeys` 里，名字在前、参数在后，事件和操作按前缀刷新。
- 查询缓存只有 `createQueryClient()` 一个出处，应用和测试共用：调用都在本机，所以离线照常读写（`networkMode: "always"`）、失败不自动重试、不因窗口焦点或网络恢复重读。
- 引擎的变化经 `useEngineEvents` 刷新对应的查询，事件丢了（`engine-resync`）就全部重读；Linear 连接状态变了，`useLinearConnectionRefresh` 重读依赖 Linear 的数据。只有启动状态和运行中任务的日志按间隔重读，其余不靠轮询。

### 12.3 文案与语言

- 用户看得到的每段文字都走翻译键（`t()`），包括 `aria-label`、`title` 和提示；组件里不写死任何语言的文字。
- 五种语言（`zh-CN`、`zh-TW`、`en`、`ja`、`ko`）的文件在 `app/src/i18n/locales/`：键和 `{{占位符}}` 必须一一对应、不能留空（`i18n.test.ts`）；组件用到而 `en.json` 没有的键、或指向一组文案而不是一句文案的键（如 `t("nav")`），都让测试失败（`test-setup.ts` 的守卫）。新增文案时五个文件一起加；带数量的句子用 `_one` / `_other` 两种形式。
- 托盘菜单、macOS 的应用菜单（`menu.*`，`quit::APP_MENU_KEYS`）和通知在 Rust 侧读同一批文件（`app/src-tauri/src/i18n.rs` 编译时嵌入），随设置里的语言更新（`tray::refresh`）；Rust 用到的键登记在 `USED_KEYS`，测试检查每种语言都有。
- 系统语言一律取 Rust 侧 `sys_locale` 报告的值（面板经 `AppInfo.system_locale` 拿到），面板、托盘和通知因此总是同一种语言。语言解析两边各有一份实现，共读 `language-cases.json`；新增语言的步骤见设计 9.4。
- 插值不转义（`escapeValue: false`），靠 React 转义文本节点，所以翻译出来的文字也只放进文本节点。

### 12.4 失败与外来文本

- 失败一律显示为「已翻译的标题＋清洗过的细节行」：`describeError` 由错误代码或调用处给的键得出标题，引擎原文清洗后作为 `DetailLine` 放在下面；读取失败的页面用 `ErrorState`，操作失败用 `reportFailure` 弹提示。
- 引擎原文（任务原因、`last_error`、环境检查的问题、命令返回的错误）从不单独出现；系统通知只有需求编号、标题和一句翻译过的话，从不带引擎原文（`notify.rs`）。理由与清洗规则见 `SECURITY_CODING_SPEC.md` 第 3 节。

### 12.5 外观

- 颜色、圆角、字体和布局常量只用 `app/src/styles.css` 的设计变量（`--sidebar-width`、`--content-max-width`、`--page-padding`、`--state-*` 等），经 Tailwind 的类名使用；组件里不写颜色值，也不写像素值（间距、字号用 Tailwind 的刻度）。
- 浅色、深色都跟随系统（`useSystemTheme` 切换 `<html>` 的 `dark` 类），两套颜色都写在 `styles.css`。状态色在两种主题下都要让标签文字在自己的淡底上不低于 4.5:1，`styles.test.ts` 核对。
- 任务状态一律用 `StatusBadge`（每种状态固定的颜色和图标），不另配颜色。

### 12.6 页面状态

- 每个读数据的页面都有加载（`LoadingState`）、空（`EmptyState`，写明下一步做什么）和错误（`ErrorState`，能重试）三种状态，并至少有一个用失败的假后端驱动的错误状态测试。概览、需求、运行记录、运行详情、项目绑定、环境检查、设置和首次引导都是这样（使用说明只是文字，没有数据要读）。
- 渲染出错时换成翻译过的错误页（`ErrorBoundary`），不留空白窗口：一页出错只换掉这一页，错误显示在外框里，侧栏还在，点别的页面就恢复（`AppShell` 里包住 `<Outlet/>` 的 `PageBoundary`，按地址复位）；外框或首次引导本身出错换掉所有页面（`App.tsx` 里包住 `<Routes>` 的 `PageBoundary`，到别的地址就恢复）；页面外的部分（事件桥、语言与主题、启动闸门）出错换掉整个面板。退出询问（`QuitDialog`）和提示（toast）不在这两层之内，各在一个出错时什么也不显示的边界里，所以任何一页出错时退出照样会问。错误页的「重新加载」先把地址改回 `#/` 再重载，不会重新打开出错的那一页。
- 首次连接 Linear 还没结束（`status.linear.kind === "connecting"`）时，要 Linear 的数据被拒绝不算失败：需求页和绑定对话框的项目列表显示「正在连接 Linear…」（`LinearConnecting`），连接状态一变就重读。

### 12.7 窗口尺寸

- 窗口默认 1100×720，最小 900×600（`tauri.conf.json`）。最小窗口下内容区约 620 px（减去 232 px 的侧栏和两侧页边距），Windows 出现滚动条时约 605 px：每个操作在 605 px 下都要够得着，不出现横向溢出。需求与运行记录的表格、绑定对话框、首次引导、设置里「把命令行加入 PATH」一行（打开开关、显示出 shell 那一行时）、概览页顶部的更新横幅与设置里版本和「检查更新」一行（有新版本，以及装不上的更新带一段很长的原因）在 900×600 与 885×600 下由 `app/tests/screens.spec.ts` 检查（`pnpm --dir app test:screens`，只在本机跑）；其他页面和设置的其余部分没有自动测试，改布局时在 620 与 605 px 两个宽度下各量一遍，结果写进提交说明。
- 表格每行最多放一个操作按钮（需求编号本身就是「在 Linear 中打开」，`IssueLink`）；长文本允许在词中断行（`wrap-anywhere`），长路径中间省略（`MiddleEllipsis`）；放进提示框（`Alert`，文字那一列是 `1fr` 的网格）的长文本，标题与正文要加 `min-w-0`，否则那一列会撑到最长那个词的宽度（`UpdateBanner`）。
- 表单对话框（`BindingDialog`）不高过窗口，字段在框内滚动，按钮始终看得见。

### 12.8 无障碍

- 装饰性图标加 `aria-hidden`；每个控件都有可读的名字（可见文字，或 `aria-label`，例如 `IssueLink` 的名字说明点了会做什么）。
- 可点击的东西用 `<button>` 或 Radix 组件，不给 `div`、`span` 加点击事件，键盘才能用（`IssueLink` 的测试按 Tab 和回车打开）。
- 页面里的区块用 `<section aria-labelledby>`，标题就是区块名（测试按 `role="region"` 找）；加载中用 `role="status"` 加名字；提醒用 `role="status"`，问题用 `role="alert"`。
- 确认操作用 `AlertDialog` 加 `AlertDialogTrigger`，关闭后 Radix 把焦点还给触发按钮；触发按钮随确认的操作一起消失时（移除绑定后卡片不在了），页面把焦点放到页头的按钮或标题上。有风险的确认把初始焦点放在安全的选项上（`FullAccessWarning` 的「取消」），回车不会误确认。
- 由页面控制打开、没有 Radix 触发按钮的对话框（`BindingDialog`、`FullAccessWarning`、`QuitDialog`）用 `useFocusReturn`（`onCloseAutoFocus`）：关闭时焦点回到打开前拿着焦点的元素，那个元素不在了就回到调用方给的备选（页头的按钮、页面标题；退出询问用当前页面的 h1，错误页与启动失败页的 h1 也能用脚本聚焦，`tabIndex={-1}`）。WebKit 里鼠标点击不会让按钮获得焦点，所以由按钮打开的对话框把被点的按钮记作打开者（`Bindings.tsx`），不依赖焦点。

### 12.9 与 core 共用的规则

面板重复 core 的规则时（显示文本清洗、模型名、「需要处理」判定、默认触发名、语言解析），用一张 JSON 用例表钉住，Rust 与 TypeScript 的测试读同一个文件，不复制；前端测试要用 core 的样本时也直接读 `crates/core/tests/fixtures/`。清单见 `TESTING_SPEC.md`「面板与 core 共用的用例表」。

### 12.10 生成文件与依赖

- `app/src/components/ui/` 是 shadcn/ui 生成的组件：照原样提交，ESLint 不检查；要别的样子就在外面包一层或传 `className`。手改只有两处，重新生成时要带上：`dialog.tsx` 两处写死的英文「Close」换成了翻译键 `common.close`（`components/DialogLabels.test.tsx` 把关）；`button.tsx` 默认按钮的悬停底色是 `bg-primary/90`（生成的是 `/80`，悬停时按钮文字只有 3.9:1；`styles.test.ts` 从该文件读出这个值核对 4.5:1）。
- `app/dist` 没构建时由 `build.rs` 放一个占位页；`app/src-tauri/gen/`、`app/src-tauri/permissions/autogenerated/` 每次构建由 `build.rs` 重新生成；三者都不进仓库。
- 前端依赖用 `pnpm --dir app add` 引入并说明用途，`app/pnpm-lock.yaml` 随同提交（见 `SECURITY_CODING_SPEC.md` 第 8 节）。
