# AGENTS.md

本文件是 IssueRelay 仓库的 AI 编码入口。AI 修改本仓库前，先读本文件，再按任务风险读 `docs/ai-coding/` 里对应的规范、当前的设计与计划，以及要改的真实代码。

## 项目定位

- 项目名称：IssueRelay（仓库与命令行 `issuerelay`，应用标识 `com.sunkailun.issuerelay`）
- 项目类型：每个开发者装一份的桌面托盘程序（macOS、Windows）。监听 Linear 上指派给自己、带 `ai-ready` 标签、状态为 Todo 的需求，每条新起一次 Claude Code 或 Codex 无人值守运行去完成。
- 技术栈：Rust 1.99（edition 2024）、tokio、rmcp 3.5（MCP 客户端与服务端）、rusqlite、process-wrap、keyring、clap；桌面应用（M2）是 Tauri 2 外壳（`app/src-tauri`）加 React / TypeScript 面板（`app/src`）。
- 外部系统：Linear 官方 MCP（`https://mcp.linear.app/mcp`，OAuth）；本机的 Claude Code 与 Codex 命令行（各自登录，各自配置 Linear MCP）。
- 核心风险：AI 在「完全放开」权限下执行任意命令；Linear 需求内容是不可信输入，可能夹带指令；同一需求被重复派活；AI 把需求设成 Done 或推主分支；令牌、公司信息或本机路径进入这个公开仓库；macOS 与 Windows 行为不一致。

## 修改前阅读顺序

1. `README.md`
2. `docs/ai-coding/README.md`：规范目录与维护规则
3. `docs/ai-coding/AI_CODING_GUIDE.md`：分层、数据流、运行数据、外部接口变更、版本与提交、桌面面板（第 12 节）
4. 按任务再读：目录 `AI_DIRECTORY_STRUCTURE_GUIDE.md`、注释 `AI_COMMENT_STYLE_GUIDE.md`、设计模式 `AI_DESIGN_PATTERN_GUIDE.md`、门禁 `AI_ENGINEERING_GUARDRAILS.md`、安全 `SECURITY_CODING_SPEC.md`、测试 `TESTING_SPEC.md`、流程 `AI_AUTOMATION_WORKFLOW.md`
5. 设计 `docs/superpowers/specs/`、计划 `docs/superpowers/plans/`、验收报告 `docs/gates/`
6. 要改的源码及其上下游：`linear` → `trigger` → `store` / `scheduler` → `engine` → `executor` / `runner` → `ipc` → `cli`；面板是 `app/src-tauri/src/commands.rs` → `app/src/lib/api.ts`（`backend.ts` 的接口）→ `app/src/hooks/queries.ts` → 页面与组件

改执行者参数、提示词固定规则、触发与防重复规则、IPC 方法表、凭据存储、面板命令与能力文件时，不能只读摘要，必须读真实代码、对应测试和验收报告。

## 项目边界（硬规则）

- **提示词的三条固定规则不可删**：不把状态设为 Done、不推主分支、不发正式环境。
- **「标记完成」只在面板里由用户触发**：IPC 方法表、命令行、MCP 控制服务都不得提供；`ipc::protocol` 的测试会拦截。
- **子进程只用固定参数**：提示词走标准输入，工作目录用 current_dir；不经 shell 启动；参数里不放提示词、路径或任何来自 Linear 的文本。仅有三处例外：① 一键配置命令（`setup.rs`）唯一的可变参数是 IssueRelay 自身命令行程序的绝对路径，放在 `--` 之后；② 读取执行者环境（`env.rs`）时运行用户的登录 shell（`$SHELL -l -i -c` 加一段固定脚本，不含任何外部文本），它在自己的会话里运行，读到结束标记或满 10 秒即被结束；③ 精简运行（Claude）的 `--settings` 里，唯一的可变部分是用户级 `CLAUDE.md` 的绝对路径：由 IssueRelay 从用户主目录（或执行者环境里的 `CLAUDE_CONFIG_DIR`）得出，经 JSON 编码；不是绝对路径或含不安全字符时不加这一项，并在运行日志里写明。
- **结束运行必须结束整棵进程树**（Unix：进程组以及所有后代进程，包括已离开该组的；Windows：Job Object），正常退出后也要清理进程组里的残留进程（执行者已经退出时，离开进程组的后代无从查找，由实机验收测量）。
- **是否重新派活只看 Linear 的 `updatedAt`**，不用本机时钟。
- **程序对 Linear 的写操作只有「标记完成」一处**，其余状态和评论都由 AI 写。
- **两个平台同等对待**：只在一个平台编译的代码（`#[cfg(unix)]`、`#[cfg(target_os = "macos")]` 等）都要有另一个平台的实现，或在注释里写明另一个平台为什么不需要（例如 Windows 没有应用菜单、隐藏的窗口没有任务栏按钮）；CI 两个平台都跑。
- **面板只能调本程序登记的命令**：只有 `app/src/lib/api.ts` 接触 `@tauri-apps/api`；新命令同时登记进 `lib.rs` 的命令表、`build.rs` 的应用清单和能力文件（三者不一致时绊线测试失败）；面板不拿任何插件权限，运行时也不加能力；浏览器只打开解析、规范化之后的 `linear.app` 链接；`Host::env()`（含没打码的代理）从不发给网页视图。细则见 `SECURITY_CODING_SPEC.md` 第 5 节。
- **外来文本在面板里只当数据**：引擎、执行者、AI 和 Linear 的文本只作为 React 文本节点、清洗后显示，从不单独充当标题（先显示由结构化数据得出的已翻译标题，原文作为细节行放在下面）。细则见 `SECURITY_CODING_SPEC.md` 第 3 节。
- **测试通过不等于实机验收**：真实 Linear、真实 Claude / Codex 的结果写进 `docs/gates/`，没做就写「待实机」。

## 本地与云端执行

本仓库在本地会话和云端会话（claude.ai 云端容器等）各有一份副本。环境变量 `CLAUDE_CODE_REMOTE=true`，或会话里没有本机配置，都按云端会话处理。细则见 `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 7 节。

- 本地会话：可以做实机验收、合并进 `main`（直接合并或合并 PR，按「贡献流程」）、打版本标签、发布安装包。
- 云端会话：只读代码、改代码、编译、测试、看 CI，只推会话指定的 `claude/<主题>` 分支，由维护者或本机会话合并；不合并进 `main`、不做实机验收、不打标签、不发布，需要实机的结论写「待本地实机」。
- 两边都要做：开工、提交前、推送前各 `git fetch` 一次；本地落后就快进；远端已前进就把还没推送的本地提交 rebase 到最新远端并重跑验证（已推送的分支不 rebase，要跟上 `main` 就合并 `main`）；遇到分叉、别人的未提交改动或解决不了的冲突就停下报告；需要升版本号时（维护者发布），推送前按 `origin/main` 最新顺延，外部贡献者不改版本号；禁止强推（含 `--force-with-lease`）和删除 `main`。

## AI 工程门禁

改动前按 `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 判风险。执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权、IPC 方法表、标记完成属于极高风险。

## 贡献流程

- 维护者一侧（所有者 2026-10-26 定、10-27 重申，与 tradingview-capture-mcp 一致）：维护者本人，以及维护者本机用维护者账号运行的 Claude / Codex 会话，可以直接合并到 `main`：合并前本机验证命令全过，合并后看 CI，红了马上修。直接合并只省掉 PR 这一步，不省掉验证、实机验收报告和所有者的决定。云端会话只推 `claude/<主题>` 分支，由维护者或本机会话合并。
- 外部贡献者走 fork + PR：替外部贡献者工作的 AI 只推送贡献者自己 fork 里的分支，不直接推本仓库的 `main`；PR 按 `.github/pull_request_template.md` 填写。
- 其他人的 PR：要 CI 三项全绿、维护者（目前只有仓库所有者）审核确认是有益的改动并批准、评审对话全部解决，才由维护者 squash 合并；AI 不得批准其他人的 PR，也不得在维护者本人批准之前合并它。
- 只允许 squash；禁止强推和删除 `main`；任何代理都不得修改 ruleset 或仓库权限。细则与分支保护见 `MAINTAINERS.md`；开发环境与提交前检查见 `CONTRIBUTING.md`。
- 提交：英文 Conventional Commits；规则改动单独提交；AI 参与时 `Co-Authored-By` 写实际参与的模型。细则见 `docs/ai-coding/AI_CODING_GUIDE.md` 第 11 节。
- 新增或移动高风险文件（执行者参数、提示词、进程运行、凭据、IPC 协议、面板命令与能力文件、显示文本清洗）时，同步更新 `.github/CODEOWNERS`。

## 多智能体协作

- 子智能体可以并行读代码、查文档、补测试、做 Review。
- `executor`、`prompt`、`runner`、`linear::auth`、`ipc::protocol` 的改动由主智能体串行处理。
- 最终改动范围、测试结论、实机结果和提交说明由主智能体统一收口。

## 验证命令

前端需要 Node.js 22.22.2+、24.15+ 或 26 及以上（`app/package.json` 的 `engines`）与 pnpm（版本见 `app/package.json` 的 `packageManager`），先 `pnpm --dir app install`。`pnpm --dir app build` 排在 cargo 命令之前，CI 也是这个顺序：`tauri build`（打开 `custom-protocol` 特性）构建的应用内嵌 `app/dist`（调试构建、`cargo test` 与 `cargo clippy` 不内嵌，连 `devUrl`）。没有先构建时，`build.rs` 会在 `app/dist` 放一个占位页让任何构建都能编过，但那样内嵌的不是真实面板。

```bash
pnpm --dir app build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir app lint
pnpm --dir app test
bash scripts/check-secrets.sh --strict
```

其中 `pnpm --dir app build` 是前置步骤，其余六条是验证命令，全部通过才算完成。

## 禁止事项

- 禁止把 Linear 令牌或 API key、Claude / Codex / GitHub 凭据、任何公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径写进代码、测试、夹具、文档、提交信息。
- 禁止删减提示词固定规则；禁止在 IPC、命令行、MCP 暴露「标记完成」「重试」「取消」。
- 禁止经 shell 启动子进程，禁止把外部文本拼进参数。仅有三处例外：① 一键配置命令（`setup.rs`）唯一的可变参数是 IssueRelay 自身命令行程序的绝对路径，放在 `--` 之后；② 读取执行者环境（`env.rs`）时运行用户的登录 shell（`$SHELL -l -i -c` 加一段固定脚本，不含任何外部文本），它在自己的会话里运行，读到结束标记或满 10 秒即被结束；③ 精简运行（Claude）的 `--settings` 里，唯一的可变部分是用户级 `CLAUDE.md` 的绝对路径：由 IssueRelay 从用户主目录（或执行者环境里的 `CLAUDE_CONFIG_DIR`）得出，经 JSON 编码；不是绝对路径或含不安全字符时不加这一项，并在运行日志里写明。
- 禁止在 `app/src/lib/api.ts` 以外使用 `@tauri-apps/api`，禁止给面板加插件权限或在运行时加能力，禁止用 `dangerouslySetInnerHTML` 显示任何文本。
- 禁止强推和删除 `main`，禁止改写已推送的历史；禁止任何代理修改 ruleset 或仓库权限。
- 禁止用夹具测试、设计文档或计划冒充实机验收。
