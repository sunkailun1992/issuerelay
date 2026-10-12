# IssueRelay M0–M1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 先实测验证无人值守执行和 Linear MCP 连接（M0），再做出有完整测试的监听引擎、命令行与 MCP 控制服务（M1），并在本机用开发宿主跑通「Linear 需求 → Claude Code / Codex → In Testing」。

**Architecture:** Rust 工作区。`issuerelay-core` 是库：Linear MCP 客户端（rmcp）、触发与防重复规则、SQLite 记录、调度、执行者命令拼装、子进程运行器、环境检查、引擎和本机 IPC；它还带一个只供测试用的 `fake-agent` 二进制。`issuerelay-cli` 是控制台程序（命令行 + stdio MCP 控制服务），只经 IPC 访问引擎。`issuerelay-devhost` 是只供开发调试的无界面宿主（不随安装包发布），M2 的 Tauri 托盘程序会用同样的方式内嵌引擎。

**Tech Stack:** Rust 1.99（edition 2024）、tokio、rmcp 3.5、rusqlite 0.40（bundled）、process-wrap 10、keyring 4、clap 4、serde / serde_json / toml、chrono、which、tokio-util。

**Spec:** `docs/superpowers/specs/2026-10-10-issuerelay-design.md`

## Global Constraints

- 工具链：rustc 1.99.0 stable，由 `rust-toolchain.toml` 固定；所有 crate 用 edition 2024。
- 依赖版本（2026-10-10 在本机探针工程里编译并实测过这些用法）：rmcp 3.5.1、tokio 1.53、process-wrap 10.0.1（特性 `tokio1`）、keyring 4.2（默认特性 `v1`，已含 macOS 钥匙串与 Windows 凭据管理器）、rusqlite 0.40（特性 `bundled`）、reqwest 0.13、dirs 7、chrono 0.4（特性 `serde`）、toml 1.1、which 8、tokio-util 0.7、clap 4.6（特性 `derive`）、sys-locale 0.3、url 2、open 5、thiserror 2、async-trait 0.1、schemars 1.2、tracing 0.1、tracing-subscriber 0.3（特性 `env-filter`）、tempfile 3（仅测试）。
- 应用标识 `com.sunkailun.issuerelay`。配置文件 `dirs::config_dir()/com.sunkailun.issuerelay/config.toml`；数据目录 `dirs::data_local_dir()/com.sunkailun.issuerelay/`（Windows 为本地、不随账户漫游的应用数据目录；macOS 与 `data_dir()` 相同），内含 `state.db` 与 `logs/`。这与 M2 中 Tauri 的 `app_config_dir()` / `app_local_data_dir()` 一致。
- Linear MCP 端点 `https://mcp.linear.app/mcp`。钥匙串服务名 `com.sunkailun.issuerelay`，账户名 `linear-oauth` 与 `linear-api-key`。
- Linear MCP 实测口径（M0 验收报告）：需求的 `projectId` 是项目的 `uuid`，项目的 `id` 是 `P-<团队>-N` 编号，绑定一律存项目 `uuid`；翻页看 `hasNextPage` 与 `cursor`；查不到需求时返回工具错误，文字为 `Could not find referenced Issue.`；授权服务器要求 RFC 9207 的 `iss` 参数，回调时把完整回调地址交给 rmcp 的 `OAuthState::handle_callback_url`。
- 触发默认值：标签 `ai-ready`；状态 `Todo` / `In Progress` / `In Testing` / `Done`；名称比较时忽略首尾空白和 ASCII 大小写。
- 默认设置：每 30 秒检查一次，同时运行总上限 2 条，单次运行上限 60 分钟，连续 15 分钟无输出算卡住，执行者 Claude，权限档位「完全放开」。
- AI 评论前缀 `🤖 IssueRelay`；AI 最后一行输出 `{"result":"in_testing|needs_info|failed","summary":"…"}`。
- 执行者参数（设计 6.1）：Claude `-p --output-format stream-json --verbose --permission-prompts none --permission-mode <bypassPermissions|auto|dontAsk>`；Codex `exec --json [--dangerously-bypass-approvals-and-sandbox|--approve-for-me] [--skip-git-repo-check] -`。提示词一律经标准输入传入，工作目录用子进程的 current_dir 设置，命令行参数里不放任何提示词或路径。「完全放开」档已在 M0 实测通过（macOS，2026-10-10，Claude Code 2.1.285、codex-cli 0.159.2，见 `docs/gates/2026-10-11-m0-verification.md`）；「自动审查」「按个人设置」两档的参数只据帮助信息，由 Task 23 实测；Windows 待实机。Task 12 照此实现。
- 「标记完成」只由 M2 面板在进程内调用 `Engine::mark_done`；IPC、命令行、MCP 控制服务都不得提供。
- 代码和测试必须在 macOS 与 Windows 上都能编译、通过；每个 `#[cfg(unix)]` 分支都要有对应的 `#[cfg(windows)]` 实现。
- 仓库里不得出现任何公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径。测试夹具一律用虚构数据（`DEMO-1`、`Demo Project`、`Jane Doe`）。
- 文档用中文；提交信息用英文 Conventional Commits，结尾加 `Co-Authored-By: Claude <实际写这个提交的模型> <noreply@anthropic.com>`。
- 计划里的代码按 rustc 1.99 与同版 clippy 写成，关键接口已在探针工程里编译实测。执行时如果 clippy 给出不改变行为的机械性建议（例如去掉多余的 `&`、合并嵌套的 `if`），照建议改即可；涉及行为的改动要回到计划和设计核对。
- 每次提交前跑：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`bash scripts/check-secrets.sh --strict`（必须无输出；新文件要先 `git add` 才会被扫描）。推送前 `git fetch origin`；远端已前进就 rebase 到最新远端并重跑验证，不强推。

## Review Focus

1. **Windows 上 npm 装的 `claude.cmd` / `codex.cmd`，配合带空格、中文、引号、换行的目录和提示词**：应原样送达 AI，不被 `cmd.exe` 改写。由 Task 12 的 `args_never_contain_prompt_or_paths` 和 Task 15 的 Windows 专属测试 `cmd_wrapper_passes_prompt_and_paths_intact` 钉住。
2. **需求在排队期间被改动**（去掉标签、改派给别人、状态变了）：开跑前必须重新读取并放弃，不得照旧启动。由 Task 19 的 `pre_dispatch_recheck_cancels_changed_issue` 钉住。
3. **带 `ai-ready` 的需求超过一页**：必须跟着游标翻页，不能只处理第一页。由 Task 16 的 `candidate_issues_follows_pagination` 钉住。
4. **本机时钟与 Linear 时钟不一致**：是否重新入队只比较 Linear 自己的 `updatedAt`（运行结束核对时记下的那个值），不用本机时间。由 Task 8 的 `should_enqueue_compares_linear_clock_only` 和 Task 19 的 `failed_issue_requeues_only_after_linear_update` 钉住。
5. **同时开了两个宿主**（例如开发宿主和以后的托盘程序）：第二个必须拒绝启动，避免同一条需求被派两次。由 Task 20 的 `second_server_refuses_to_start` 钉住。

## 计划总览

**怎么读这份计划**：先看本节和上面的 Global Constraints；执行时按任务编号顺序走。每个任务都写明了要改的文件、对外接口、测试代码、运行命令和提交信息。标着「用户操作」的步骤，需要你本人在浏览器里授权或确认。

| 阶段 | 任务 | 交付 | 需要你做的事 |
|---|---|---|---|
| 准备 | 1 | 仓库骨架、AI 编码规范（`AGENTS.md` 与 `docs/ai-coding/`）、密钥扫描、CI | 无 |
| M0 实测验证 | 2 | Linear 沙箱项目与本机 git 沙箱 | 同意建测试项目和测试需求 |
| | 3 | Linear MCP 探针与真实返回样本 | 在浏览器里授权 Linear |
| | 4 | macOS 上 Claude Code / Codex 无人值守实测与样本 | 登录 Claude 命令行；为 Claude Code、Codex 授权 Linear |
| | 5 | Windows 上同样实测 | 提供 Windows 机器（没有就跳过） |
| | 6 | M0 验收报告；按实测修正设计与计划 | 审阅报告 |
| | 6b | 贡献与共同维护：贡献指南、行为准则、安全政策、维护者与治理、issue / PR 模板、CODEOWNERS、Dependabot | 无 |
| M1 引擎与命令行 | 7–13 | 配置、数据模型、触发与防重复、SQLite、调度、提示词、执行者命令、结果解析 | 无 |
| | 14–15 | 测试用假执行者；子进程运行器（超时、卡住、结束进程树） | 无 |
| | 16–18 | Linear 接口与授权；环境检查 | 无 |
| | 19–20 | 引擎；本机 IPC | 无 |
| | 21–22 | `issuerelay` 命令行与 MCP 控制服务；开发宿主 | 无 |
| | 23 | M1 实机验收报告 | 在浏览器里授权；在沙箱里建测试需求 |

**依赖关系**：Task 1 → M0（2 → 3 → 4 → 5 → 6 → 6b）→ M1（7 → 8 → … → 23）。Task 13、16、18 会用到 M0 的真实样本；M0 结论与本计划不一致时，先按 Task 6 改计划，再继续往下做。

## 文件结构

| 路径 | 职责 |
|---|---|
| `Cargo.toml`、`rust-toolchain.toml` | 工作区（成员 `crates/*`，排除 `spikes/`）与工具链 |
| `AGENTS.md`、`CLAUDE.md`、`.github/copilot-instructions.md` | AI 入口与最高优先级边界（后两个只转发到 `AGENTS.md`） |
| `docs/ai-coding/` | AI 编码规范：指南、目录、注释、设计模式、门禁、安全、测试、流程 |
| `README.md`、`LICENSE` | 项目说明（中英）、Apache-2.0 |
| `scripts/check-secrets.sh` | 扫描密钥、内网地址、本机绝对路径 |
| `.github/workflows/ci.yml` | macOS + Windows 上 fmt / clippy / test；Ubuntu 上 check-secrets |
| `spikes/linear-mcp-probe/` | M0 探针（独立 cargo 工程，不进工作区，保留作参考） |
| `docs/gates/` | 验收报告与脱敏样本 |
| `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`MAINTAINERS.md`、`.github/` 下的模板、`CODEOWNERS`、`dependabot.yml` | 贡献与共同维护（Task 6b） |
| `crates/core/src/paths.rs` | 配置、数据、日志、IPC 端点位置 |
| `crates/core/src/config.rs` | 设置与绑定，TOML 读写 |
| `crates/core/src/model.rs` | `Issue`、`Job`、`JobState` |
| `crates/core/src/trigger.rs` | 触发条件、可派活判断、防重复规则 |
| `crates/core/src/store.rs` | SQLite 运行记录 |
| `crates/core/src/scheduler.rs` | 同目录排队与并发上限 |
| `crates/core/src/prompt.rs` | 固定规则 + 团队模板生成提示词 |
| `crates/core/src/executor/mod.rs` | 按执行者与权限档位拼命令 |
| `crates/core/src/executor/outcome.rs` | 从输出流里取最终消息，解析结果 JSON |
| `crates/core/src/bin/fake_agent.rs` | 测试用假执行者 |
| `crates/core/src/runner.rs` | 启动子进程、写日志、超时、卡住、结束整个进程树 |
| `crates/core/src/linear/mod.rs` | `LinearApi` 接口与常量 |
| `crates/core/src/linear/parse.rs` | 解析 Linear MCP 返回的 JSON 文本 |
| `crates/core/src/linear/mcp.rs` | 基于 rmcp 的 `LinearApi` 实现 |
| `crates/core/src/linear/auth.rs` | OAuth 回环授权、API key、钥匙串存储 |
| `crates/core/src/doctor.rs` | 执行者、工作区、绑定检查 |
| `crates/core/src/engine.rs` | 轮询、入队、派活、核对结果、暂停、标记完成 |
| `crates/core/src/ipc/{mod,protocol,server,client}.rs` | 本机 JSON-RPC（Unix 套接字 / 命名管道） |
| `crates/core/tests/common/` | 假 Linear（内存版与 MCP 服务端版）、假运行器、测试夹具工具 |
| `crates/cli/` | `issuerelay` 命令行与 MCP 控制服务 |
| `crates/devhost/` | `issuerelay-devhost` 开发宿主 |

---

### Task 1: 仓库骨架、AI 编码规范与 CI

> 执行记录（2026-10-11）：已完成。评审中修订了 `ci.yml`、`scripts/check-secrets.sh`、`.gitignore`，新增 `.gitattributes`，文档里的密钥扫描命令统一为 `--strict`（见 git 历史）。下文是原计划文本，以仓库现状为准。

**Files:**
- Create: `Cargo.toml`、`rust-toolchain.toml`、`.gitignore`、`LICENSE`、`README.md`、`AGENTS.md`、`CLAUDE.md`、`.github/copilot-instructions.md`、`.github/workflows/ci.yml`、`scripts/check-secrets.sh`、`docs/gates/README.md`、`crates/core/Cargo.toml`、`crates/core/src/lib.rs`
- Create（AI 编码规范，结构与作者其他个人仓库的 `docs/ai-coding/` 一致）：`docs/ai-coding/README.md`、`AI_CODING_GUIDE.md`、`AI_DIRECTORY_STRUCTURE_GUIDE.md`、`AI_COMMENT_STYLE_GUIDE.md`、`AI_DESIGN_PATTERN_GUIDE.md`、`AI_ENGINEERING_GUARDRAILS.md`、`SECURITY_CODING_SPEC.md`、`TESTING_SPEC.md`、`AI_AUTOMATION_WORKFLOW.md`

**Interfaces:**
- Consumes: 无
- Produces: 工作区成员通配 `crates/*`；crate 名 `issuerelay-core`（Rust 里写作 `issuerelay_core`）；`scripts/check-secrets.sh [--strict]`；后续每个任务都按 `AGENTS.md` 与 `docs/ai-coding/` 执行

- [ ] **Step 1: 写工作区与工具链文件**

`Cargo.toml`：

```toml
[workspace]
resolver = "3"
members = ["crates/*"]
exclude = ["spikes"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/sunkailun1992/issuerelay"
```

`rust-toolchain.toml`：

```toml
[toolchain]
channel = "1.99.0"
components = ["rustfmt", "clippy"]
```

`.gitignore`：

```gitignore
/target
**/target
.DS_Store
*.swp
/spikes/**/samples-raw/
```

`crates/core/Cargo.toml`：

```toml
[package]
name = "issuerelay-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
```

`crates/core/src/lib.rs`：

```rust
//! IssueRelay engine: watches Linear issues assigned to the current user and hands each one
//! to a fresh headless Claude Code or Codex run.
```

- [ ] **Step 2: 下载许可证原文**

Run: `curl -sSfL https://www.apache.org/licenses/LICENSE-2.0.txt -o LICENSE && head -3 LICENSE`
Expected: 第 2 行含 `Apache License`

- [ ] **Step 3: 写 AI 入口文件**

`AGENTS.md`：

````markdown
# AGENTS.md

本文件是 IssueRelay 仓库的 AI 编码入口。AI 修改本仓库前，先读本文件，再按任务风险读 `docs/ai-coding/` 里对应的规范、当前的设计与计划，以及要改的真实代码。

## 项目定位

- 项目名称：IssueRelay（仓库与命令行 `issuerelay`，应用标识 `com.sunkailun.issuerelay`）
- 项目类型：每个开发者装一份的桌面托盘程序（macOS、Windows）。监听 Linear 上指派给自己、带 `ai-ready` 标签、状态为 Todo 的需求，每条新起一次 Claude Code 或 Codex 无人值守运行去完成。
- 技术栈：Rust 1.99（edition 2024）、tokio、rmcp 3.5（MCP 客户端与服务端）、rusqlite、process-wrap、keyring、clap；M2 起加 Tauri 2 与 React / TypeScript 面板。
- 外部系统：Linear 官方 MCP（`https://mcp.linear.app/mcp`，OAuth）；本机的 Claude Code 与 Codex 命令行（各自登录，各自配置 Linear MCP）。
- 核心风险：AI 在「完全放开」权限下执行任意命令；Linear 需求内容是不可信输入，可能夹带指令；同一需求被重复派活；AI 把需求设成 Done 或推主分支；令牌、公司信息或本机路径进入这个公开仓库；macOS 与 Windows 行为不一致。

## 修改前阅读顺序

1. `README.md`
2. `docs/ai-coding/README.md`：规范目录与维护规则
3. `docs/ai-coding/AI_CODING_GUIDE.md`：分层、数据流、运行数据、外部接口变更、版本与提交
4. 按任务再读：目录 `AI_DIRECTORY_STRUCTURE_GUIDE.md`、注释 `AI_COMMENT_STYLE_GUIDE.md`、设计模式 `AI_DESIGN_PATTERN_GUIDE.md`、门禁 `AI_ENGINEERING_GUARDRAILS.md`、安全 `SECURITY_CODING_SPEC.md`、测试 `TESTING_SPEC.md`、流程 `AI_AUTOMATION_WORKFLOW.md`
5. 设计 `docs/superpowers/specs/`、计划 `docs/superpowers/plans/`、验收报告 `docs/gates/`
6. 要改的源码及其上下游：`linear` → `trigger` → `store` / `scheduler` → `engine` → `executor` / `runner` → `ipc` → `cli`

改执行者参数、提示词固定规则、触发与防重复规则、IPC 方法表、凭据存储时，不能只读摘要，必须读真实代码、对应测试和验收报告。

## 项目边界（硬规则）

- **提示词的三条固定规则不可删**：不把状态设为 Done、不推主分支、不发正式环境。
- **「标记完成」只在面板里由用户触发**：IPC 方法表、命令行、MCP 控制服务都不得提供；`ipc::protocol` 的测试会拦截。
- **子进程只用固定参数**：提示词走标准输入，工作目录用 current_dir；参数里不放提示词、路径或任何来自 Linear 的文本。
- **结束运行必须结束整棵进程树**（Unix 进程组、Windows Job Object），正常退出后也要清理残留进程。
- **是否重新派活只看 Linear 的 `updatedAt`**，不用本机时钟。
- **程序对 Linear 的写操作只有「标记完成」一处**，其余状态和评论都由 AI 写。
- **两个平台同等对待**：每个 `#[cfg(unix)]` 都要有 `#[cfg(windows)]` 实现，CI 两个平台都跑。
- **测试通过不等于实机验收**：真实 Linear、真实 Claude / Codex 的结果写进 `docs/gates/`，没做就写「待实机」。

## 本地与云端执行

本仓库在本地会话和云端会话（claude.ai 云端容器等）各有一份副本。环境变量 `CLAUDE_CODE_REMOTE=true`，或会话里没有本机配置，都按云端会话处理。细则见 `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 7 节。

- 本地会话：可以做实机验收、打版本标签、发布安装包。
- 云端会话：只读代码、改代码、编译、测试、看 CI，只推会话指定的分支；不做实机验收、不打标签、不发布，需要实机的结论写「待本地实机」。
- 两边都要做：开工、提交前、推送前各 `git fetch` 一次；远端已前进就在最新远端上 rebase 并重跑验证；版本号推送前按 `origin/main` 最新顺延；禁止强推（含 `--force-with-lease`）。

## AI 工程门禁

改动前按 `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 判风险。执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权、IPC 方法表、标记完成属于极高风险。

## 多智能体协作

- 子智能体可以并行读代码、查文档、补测试、做 Review。
- `executor`、`prompt`、`runner`、`linear::auth`、`ipc::protocol` 的改动由主智能体串行处理。
- 最终改动范围、测试结论、实机结果和提交说明由主智能体统一收口。

## 验证命令

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash scripts/check-secrets.sh
```

## 禁止事项

- 禁止把 Linear 令牌或 API key、Claude / Codex / GitHub 凭据、任何公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径写进代码、测试、夹具、文档、提交信息。
- 禁止删减提示词固定规则；禁止在 IPC、命令行、MCP 暴露「标记完成」「重试」「取消」。
- 禁止经 shell 启动子进程，禁止把外部文本拼进参数。
- 禁止强推，禁止改写已推送的历史。
- 禁止用夹具测试、设计文档或计划冒充实机验收。
````

`CLAUDE.md`：

```markdown
# CLAUDE.md

@AGENTS.md

本仓库的 AI 编码规则只维护一份：`AGENTS.md`。Codex 直接读取它，Claude Code 通过上面的导入行读取同一份内容；改规则请改 `AGENTS.md`，不要在本文件重复。
```

`.github/copilot-instructions.md`：

```markdown
# Copilot instructions

Read `AGENTS.md` at the repository root first. It is the single source of AI coding rules for this repository; details live in `docs/ai-coding/`.
```

- [ ] **Step 4: 写 README**

`README.md`：

````markdown
# IssueRelay

把 Linear 上指派给你的需求，自动交给你电脑上的 Claude Code 或 Codex 去完成。

*Hand the Linear issues assigned to you to Claude Code or Codex on your own machine.*

> **状态：开发中，尚未发布。** 当前在做 M0（实测验证）和 M1（引擎与命令行）；桌面面板与安装包在 M2、M3。
> *Status: under development, not released yet.*

## 它怎么工作

```text
Linear（指派给你 · 带 ai-ready · 状态 Todo）
   │  每 30 秒查一次（Linear 官方 MCP，OAuth 授权）
   ▼
IssueRelay（你电脑上的托盘程序）
   │  按「Linear 项目 → 本地目录」的绑定找到工作目录；同一目录一次只跑一条
   ▼
Claude Code / Codex（无人值守运行，用你自己的登录）
   │  读需求 → 改代码 → 推测试分支 → 把状态改成 In Testing，并留言（以「🤖 IssueRelay」开头）
   ▼
你验收后，在面板上点「标记完成」→ Linear 状态变为 Done
```

## 一条需求要被自动处理，必须同时满足

1. 指派给你；
2. 带标签 `ai-ready`；
3. 状态是 `Todo`；
4. 所属 Linear 项目已在 IssueRelay 里绑定了本地目录。

## 安全须知

- **默认以「完全放开」权限无人值守运行 AI**：它能执行任意命令、使用你电脑上的全部凭据。只在你信任的仓库和需求上使用；可以按项目调低档位。
- AI 被要求遵守三条固定规则：不把状态设为 Done、不推主分支、不发正式环境。这些规则写在提示词里，约束力取决于 AI 本身，请同时为主分支开启分支保护。
- Linear 令牌只存在系统钥匙串（macOS 钥匙串、Windows 凭据管理器）。IssueRelay 不上传任何数据，也没有遥测。

## 开发

需要 Rust 1.99（`rust-toolchain.toml` 会自动选用）。

```bash
cargo test --workspace
cargo run -p issuerelay-devhost -- --help   # 开发用无界面宿主（不发布）
cargo run -p issuerelay-cli -- --help       # 命令行
```

提交前的检查、代码规范与开发流程见 [AGENTS.md](AGENTS.md) 和 [docs/ai-coding](docs/ai-coding/README.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [设计](docs/superpowers/specs/2026-10-10-issuerelay-design.md) | 目标、关键决定、整体结构、需求的生命周期 |
| [实施计划](docs/superpowers/plans/2026-10-10-issuerelay-m0-m1.md) | M0–M1 的分步任务 |
| [验收报告](docs/gates/) | 每次实机验收的结果 |
| [AI 编码规范](docs/ai-coding/README.md) | 分层、目录、注释、设计模式、门禁、安全、测试、流程 |

## 许可证

[Apache-2.0](LICENSE)
````

- [ ] **Step 5: 写 AI 编码规范（`docs/ai-coding/`）**

`docs/ai-coding/README.md`：

````markdown
# AI 编程规范目录

本目录集中维护 IssueRelay 的 AI 编程规范。根目录 `AGENTS.md` 只放入口和最高优先级的边界；细分规范都在这里。结构与作者其他个人仓库（imctl 等）的 `docs/ai-coding/` 一致，内容按 Rust 工作区、桌面托盘程序、Linear MCP 和「无人值守调用 Claude Code / Codex」的场景收敛。

## 目录职责

```text
AGENTS.md
  -> docs/ai-coding/README.md            本文件：目录、阅读顺序、维护规则
    -> AI_CODING_GUIDE.md                分层、数据流、运行数据、外部接口变更、版本与提交
    -> AI_DIRECTORY_STRUCTURE_GUIDE.md   crate、模块、测试、文档、脚本放在哪里
    -> AI_COMMENT_STYLE_GUIDE.md         Rust / Shell / Markdown / TOML / JSON 夹具的注释写法
    -> AI_DESIGN_PATTERN_GUIDE.md        既有模式（接口 + 假实现、纯函数决策……）与禁止过度抽象
    -> AI_ENGINEERING_GUARDRAILS.md      风险分级、完成标准、必查项、本地与云端、交付说明模板
    -> SECURITY_CODING_SPEC.md           凭据、执行者权限、不可信输入、子进程、本机通信、开源卫生
    -> TESTING_SPEC.md                   测试分层、假实现、夹具脱敏、验收报告模板
    -> AI_AUTOMATION_WORKFLOW.md         设计 → 计划 → 实现 → 验收 → 推送的标准流程
```

`.github/copilot-instructions.md` 只做转发，避免与 `AGENTS.md` 冲突。

## 阅读顺序

1. 根目录 `AGENTS.md`
2. 本文件
3. `AI_CODING_GUIDE.md`
4. 与任务相关的其余规范
5. 当前的设计（`docs/superpowers/specs/`）、计划（`docs/superpowers/plans/`）、验收报告（`docs/gates/`）
6. 要改的源码及其上下游

## 维护规则

- 新规范放在本目录；同一条规则只保留一个主版本，其他地方链接过去。
- `README.md` 与本目录冲突时以本目录为准，并同步修正 `README.md`。
- 示例只用占位符和虚构数据（`<LINEAR_API_KEY>`、`DEMO-1`、`Demo Project`、`Jane Doe`），路径写 `~/…` 或 `<REPO>`，不写本机绝对路径。
- 只改文档或规范时不升版本号。
- 发现疑似密钥只报告文件和行号，不自动改写 git 历史；是否改写由用户决定。
````

`docs/ai-coding/AI_CODING_GUIDE.md`：

````markdown
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

工具链由 `rust-toolchain.toml` 固定为 1.99.0。

## 3. 分层

| 层 | 模块 | 职责 | 可以依赖 |
|---|---|---|---|
| 基础 | `core::paths`、`core::config` | 位置、设置、绑定 | 无 |
| 纯逻辑 | `core::model`、`trigger`、`scheduler`、`prompt`、`executor`（拼命令）、`executor::outcome` | 判断与生成，不做 IO | 基础层 |
| 外部接入 | `core::store`（SQLite）、`runner`（子进程）、`linear`（MCP）、`doctor`（检查命令） | 与外部世界交互 | 基础层、纯逻辑层 |
| 编排 | `core::engine` | 唯一持有运行状态的地方 | 以上全部 |
| 通信 | `core::ipc` | 本机 JSON-RPC，薄层 | `engine` |
| 入口 | `crates/cli`、`crates/devhost`、（M2）`app/` | 用户与 AI 的入口 | `core` |

规则：纯逻辑层不读文件、不看系统时间、不发请求；运行状态只在 `engine`；命令行只经 IPC 访问引擎。

## 4. 关键数据流

1. `engine::poll_once` 调 `LinearApi::candidate_issues`，取「指派给我 + 标签 + Todo」的需求。
2. `trigger::evaluate` 判断能否派活（项目已绑定、目录存在、执行者就绪），`trigger::should_enqueue` 防止重复。
3. 新需求写入 `store`，状态为「排队中」或「等待绑定」。
4. `engine::dispatch` 用 `scheduler::pick` 选出可以开始的任务（同目录一次一条，总数不超上限）。
5. 开跑前重新读取需求，不再满足条件就取消。
6. `prompt::render` 与 `executor::build` 生成命令，`runner::run` 启动并记录日志。
7. 结束后再次读取需求，`engine::decide` 结合退出码、Linear 状态和 AI 的报告决定结果，写回 `store`。

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
| Linear 工具调用与解析 | `crates/core/src/linear/mcp.rs`、`parse.rs` |
| Linear 授权与钥匙串 | `crates/core/src/linear/auth.rs` |
| 环境检查 | `crates/core/src/doctor.rs` |
| 状态流转与编排 | `crates/core/src/engine.rs` |
| IPC 方法表 | `crates/core/src/ipc/protocol.rs` |
| 命令行与 MCP 控制服务 | `crates/cli/src/main.rs`、`mcp.rs`、`format.rs` |

## 6. 运行数据与配置

| 数据 | 位置 | 说明 |
|---|---|---|
| 设置与绑定 | `<配置目录>/com.sunkailun.issuerelay/config.toml` | TOML；先写临时文件再改名 |
| 运行记录 | `<数据目录>/com.sunkailun.issuerelay/state.db` | SQLite |
| 运行日志 | `<数据目录>/com.sunkailun.issuerelay/logs/<任务 ID>.jsonl` | 执行者的原始输出 |
| Linear 凭据 | 系统钥匙串：服务名 `com.sunkailun.issuerelay`，账户 `linear-oauth` / `linear-api-key` | 不写盘 |
| IPC 端点 | macOS：数据目录下的 `issuerelay.sock`；Windows：`\\.\pipe\issuerelay-<用户>-<哈希>` | 只限当前用户 |

`<配置目录>`、`<数据目录>` 取自 `dirs::config_dir()` / `dirs::data_dir()`：macOS 都是 `~/Library/Application Support`，Windows 是 `%APPDATA%`。这些都只在用户本机，不进仓库。

## 7. 外部接口变更规则

- **Linear MCP**：工具名、参数、返回格式以 M0 样本（`docs/gates/samples/m0/linear/`）为准。Linear 改了格式时，先更新脱敏样本和 `parse.rs` 的测试，再改代码；增删工具要同步 `REQUIRED_TOOLS` 和测试用的假服务端。
- **Claude Code / Codex 参数**：以验收报告记录的版本为准。参数变化时同时改 `executor::build`、它的测试和设计 6.1 的表；输出事件格式变化时改 `outcome::observe` 和样本测试。
- **IPC 方法表**：改 `METHODS` 必须同步 `server` 的分发、命令行、MCP 工具和测试；「标记完成」「重试」「取消」不得加入。

## 8. 外部事实与夹具

- 外部事实（命令行参数、输出格式、接口行为）写清日期和来源版本，例如「codex-cli 0.159.2（2026-10-10）的 `--approve-for-me`」，写在代码注释或验收报告里。
- 来自真实运行的夹具必须脱敏后才能提交，规则见 `TESTING_SPEC.md`。

## 9. 里程碑与文档流

设计（`docs/superpowers/specs/`）→ 实施计划（`docs/superpowers/plans/`）→ 按任务实现 → 实机验收报告（`docs/gates/`）→ 推送。里程碑：M0 实测验证、M1 引擎与命令行、M2 桌面应用、M3 安装与发布。

## 10. 版本

- 语义化版本，由工作区 `[workspace.package] version` 统一管理。
- 只有运行行为变化才升版本；只改文档、测试、规范不升。
- 版本号在推送前按 `origin/main` 的最新值顺延，开工时算的号不作数。
- M3 起推送 `vX.Y.Z` 标签触发发布。

## 11. 分支与提交

- 一个任务一个提交；提交前跑第 2 节的检查和 `bash scripts/check-secrets.sh`。
- 提交信息用英文 Conventional Commits：`feat|fix|docs|test|chore|refactor(<crate 或模块>): <做了什么>`；AI 参与时结尾加 `Co-Authored-By` 行。
- 开工、提交前、推送前各 `git fetch`；远端已前进就 rebase 后重跑检查；禁止强推。
````

`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`：

````markdown
# AI 目录管理规范

## 1. 总原则

- 按职责放文件，一个文件一个职责；文件明显变大时先拆分再加功能。
- 新增顶层目录或 crate 前，先在本文件登记用途。
- 生成物（`target/`）、本机数据、真实运行的原始输出不进仓库。

## 2. 仓库根目录

| 路径 | 用途 |
|---|---|
| `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | 工作区、锁定的依赖、工具链 |
| `AGENTS.md`、`CLAUDE.md` | AI 入口（`CLAUDE.md` 只导入 `AGENTS.md`） |
| `README.md`、`LICENSE` | 说明与许可证 |
| `crates/` | Rust crate |
| `app/` | （M2）Tauri 外壳与面板前端 |
| `docs/` | 规范、设计、计划、验收报告 |
| `scripts/` | 仓库脚本 |
| `spikes/` | 一次性验证工程（独立 cargo 工程，不进工作区） |
| `.github/` | CI 与工具入口 |

## 3. crates/

| crate | 内容 |
|---|---|
| `core`（`issuerelay-core`） | 引擎库；`src/` 每个模块一个文件或目录；`src/bin/fake_agent.rs` 只供测试 |
| `cli`（`issuerelay-cli`） | 库 `src/lib.rs`（`format`、`mcp`）与二进制 `src/main.rs`（`issuerelay`） |
| `devhost`（`issuerelay-devhost`） | 开发用无界面宿主，`publish = false`，不进安装包 |

测试放置：
- 单元测试写在源文件末尾的 `#[cfg(test)] mod tests`，位于所有实现代码之后。
- 集成测试放在 `crates/<crate>/tests/*.rs`；共用的假实现放 `tests/common/`（文件头写 `#![allow(dead_code)]`）。
- 夹具放 `crates/<crate>/tests/fixtures/`，只放脱敏后的内容。

## 4. app/（M2 起）

- `app/src-tauri/`：Tauri 外壳（托盘、窗口、通知、开机自启、更新、IPC 服务端），内嵌 `issuerelay-core`。
- `app/src/`：React + TypeScript 面板；翻译文件放 `app/src/locales/<语言>.json`。

## 5. docs/

| 路径 | 用途 |
|---|---|
| `docs/ai-coding/` | AI 编码规范（本目录） |
| `docs/superpowers/specs/` | 设计文档，命名 `<日期>-<主题>-design.md` |
| `docs/superpowers/plans/` | 实施计划，命名 `<日期>-<主题>.md` |
| `docs/gates/` | 实机验收报告 `<日期>-<主题>.md`；脱敏样本放 `docs/gates/samples/<里程碑>/` |

## 6. scripts/ 与 .github/

- `scripts/check-secrets.sh`：扫描密钥和本机信息。新脚本的文件头写清用途、用法、退出码。
- `.github/workflows/ci.yml`：在 macOS 与 Windows 上跑格式、静态检查、测试，并扫描密钥。
- `.github/copilot-instructions.md`：只转发到 `AGENTS.md`。

## 7. 目录变更检查

1. 新文件放在本规范规定的位置了吗？
2. 新增的目录或 crate 在本文件登记了吗？
3. 有没有把生成物、本机数据、原始输出加进仓库？
4. 测试与夹具和被测代码在同一个 crate 吗？
````

`docs/ai-coding/AI_COMMENT_STYLE_GUIDE.md`：

````markdown
# AI 注释规范

## 1. 总原则

- 注释写「为什么」和代码看不出来的约束；代码本身能说清的不写。
- 源码注释用英文；文档（`docs/`、`README.md`、`AGENTS.md`）用中文，短句，先结论后细节。
- 注释和代码在同一次修改里更新；过时的注释比没有注释更糟。

## 2. 模块注释 `//!`

每个源文件第一行用 `//!` 一句话说明这个模块负责什么，必要时第二句说明关键约束。例如：

```rust
//! Runs one executor process: feeds the prompt on stdin, streams output into a JSONL log and
//! always kills the whole process tree when the run ends.
```

## 3. 文档注释 `///`

- 写在公开的类型、字段、函数上：做什么、关键约束、什么时候返回错误或 `None`。
- 不复述函数签名，不写「This function…」。

## 4. 实现注释 `//`

- 解释不显然的原因，例如「wait on the native child: the Job Object wrapper also waits for leftover processes」。
- 外部事实写明来源与日期，例如「confirmed with codex-cli 0.159.2 on 2026-10-10」。

## 5. unsafe

每个 `unsafe` 块前写 `// SAFETY:`，说明为什么在这里成立。

## 6. 测试

- 测试函数名用完整的 snake_case 句子说清期望的行为，例如 `failed_issue_requeues_only_after_linear_update`。
- 场景不直观时，在测试开头用一行注释说明在验证什么。

## 7. Shell 脚本

文件头写用途、用法和退出码，并使用 `set -euo pipefail`。

## 8. Markdown

- 中文，短句，一段一个意思；先给结论再给细节。
- 示例只用占位符与虚构数据（`<LINEAR_API_KEY>`、`DEMO-1`、`Demo Project`、`Jane Doe`、`~/…`）。

## 9. TOML 与 JSON 夹具

- TOML 只在取值不显然时加注释。
- JSON 不能写注释：夹具的来源和日期写在使用它的测试里，或写在同目录的 README 里。

## 10. 检查清单

1. 每个新文件都有 `//!` 吗？
2. 公开接口都有 `///` 吗？
3. 每个 `unsafe` 都有 `// SAFETY:` 吗？
4. 外部事实写了日期和版本吗？
5. 注释里有没有真实姓名、需求内容、本机路径、令牌？
````

`docs/ai-coding/AI_DESIGN_PATTERN_GUIDE.md`：

````markdown
# AI 设计模式规范

## 1. 总原则

- 先用本项目已有的模式，不为假想的需求加抽象。
- 小文件、单一职责；能写成纯函数的就写成纯函数。

## 2. 本项目既有模式

| 模式 | 在哪里 | 怎么用 |
|---|---|---|
| 接口 + 假实现 | `LinearApi`、`JobRunner`、`ReadinessProbe`、`CommandRunner` | 生产代码依赖 trait；测试用 `FakeLinear`、`FakeLinearServer`、`ScriptedRunner`、`FakeRunner` |
| 纯函数决策 | `trigger::evaluate`、`trigger::should_enqueue`、`scheduler::pick`、`engine::decide`、`prompt::render`、`executor::build` | 不做 IO，单元测试覆盖每个分支 |
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
````

`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md`：

````markdown
# AI 工程门禁规范

AI 可以加快开发，但不能替代实机验收、用户对权限和发布的决定、安全检查和回滚手段。

## 1. 核心原则

1. AI 生成的内容默认不可信，必须经过 diff 检查、四条验证命令和 Review 才能合入。
2. 提示词固定规则（不设 Done、不推主分支、不发正式环境）和「标记完成只在面板」是硬边界，任何改动都不得削弱。
3. 子进程只用固定参数；提示词走标准输入；结束运行要结束整棵进程树。
4. 是否重新派活只看 Linear 的 `updatedAt`；同一需求不得并行或重复派活。
5. 凭据只在系统钥匙串；仓库是公开的，任何密钥、公司信息、本机路径都不得进入。
6. macOS 与 Windows 同等对待；只在一个平台验证过的结论要写明。
7. 测试、设计和计划不能冒充实机验收。

## 2. 风险分级

| 级别 | 变更类型 | 门禁要求 |
|---|---|---|
| 低 | README、规范、注释、验收报告文字 | `git diff` 检查无密钥、公司信息、本机路径 |
| 中 | 纯逻辑（触发、调度、解析、格式化）、测试与夹具、命令行输出 | 四条验证命令通过；新增分支有测试 |
| 高 | 引擎状态流转、存储结构、IPC、Linear 调用、环境检查、开发宿主 | 中风险的要求，加集成测试，加沙箱上的实机验收与报告 |
| 极高 | 执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权流程、IPC 方法表、标记完成 | 先写设计或计划并取得用户同意；两个平台测试；实机验收；单独提交 |

## 3. 完成标准（Definition of Done）

1. 改动能对应到设计或计划里的条目；对应不上就先补设计。
2. 新行为有测试：正常路径、失败路径、边界情况（空、超时、重复、平台差异）。
3. 没有绕过分层规则（`AI_CODING_GUIDE.md` 第 3 节）。
4. 外部接口变化同步了样本、测试和文档（`AI_CODING_GUIDE.md` 第 7 节）。
5. 四条验证命令全部通过，并写明测试数量。
6. 高风险及以上：实机验收已做，或写明「待实机」及原因。
7. README、规范、设计按变更同步。
8. 交付说明完整（第 8 节）。

## 4. 必须执行的检查

| 场景 | 必查项 |
|---|---|
| 执行者 | 参数与设计 6.1 一致；提示词只在标准输入；`--permission-prompts none` 仍在；三档权限映射正确 |
| 子进程 | 进程组 / Job Object 仍在；超时、卡住、取消都会结束整棵树；正常退出后清理残留 |
| 触发与派活 | 指派给我、带标签、状态 Todo；开跑前复查；同目录一次一条；总数不超上限；只比较 Linear 的时间 |
| Linear | 连接时检查 `REQUIRED_TOOLS`；翻页翻到底；查不到需求返回 `None` 而不是报错 |
| IPC | 方法表不含标记完成、重试、取消；Unix 套接字目录 0700；Windows 管道拒绝远程客户端；第二个宿主拒绝启动 |
| 凭据 | 只经 `keyring`；不出现在日志、IPC 响应和错误信息里 |
| 提交 | 四条验证命令；`git status --short`；`git diff --stat`；没有生成物和原始样本 |

## 5. 安全检查清单

1. 仓库里没有令牌、API key、公司名、内网地址、真实需求内容、本机绝对路径。
2. 新增的日志不打印令牌；运行日志只存在本机。
3. 来自 Linear 的文本只当数据，不拼进命令。
4. 新依赖有明确用途，`Cargo.lock` 可审查。
5. 不替用户授权 Linear / Claude / Codex，不改系统设置，不安装开机项（M2 的开机自启由用户在面板里打开）。

## 6. AI 工具使用边界

1. 子智能体可以并行读代码、查文档、补测试、做 Review。
2. 子智能体的输出只是证据和建议；最终改动、测试结论、实机结果、提交说明由主智能体负责。
3. 并行写代码前先划清文件边界；极高风险文件由主智能体串行修改。
4. 实机验收（真实 Linear、真实 Claude / Codex）只由主智能体在沙箱项目和沙箱仓库上执行。

## 7. 本地与云端执行

### 7.1 本地会话

可以做实机验收、打版本标签、发布安装包。发布只从已推送到 `origin/main` 的提交构建。

### 7.2 云端会话

环境变量 `CLAUDE_CODE_REMOTE=true`，或会话里没有本机配置，就按云端处理：只读代码、改代码、编译、测试、看 CI，只推会话指定的分支；不做实机验收、不打标签、不发布；需要实机的结论写「待本地实机」。

### 7.3 双端同步

开工、提交前、推送前各 `git fetch`；远端已前进就在最新远端上 rebase 并重跑验证；推送被拒同样处理；禁止强推（含 `--force-with-lease`）；遇到分叉或冲突就停下报告。

## 8. 交付说明模板

```text
变更内容：
- <任务：改了什么，涉及哪些 crate / 模块>

验证结果：
- <fmt / clippy / test（数量）/ check-secrets；实机验收结果，或「待实机」及原因>

安全检查：
- <凭据、开源卫生、子进程参数、标记完成边界的检查结果>

风险和回滚：
- <剩余风险；回滚方式：git revert <提交>>

未完成项：
- <确实没有验证、或依赖用户的事项>
```
````

`docs/ai-coding/SECURITY_CODING_SPEC.md`：

````markdown
# 安全编码规范

## 1. 凭据

- Linear 的 OAuth 令牌和 API key 只存在系统钥匙串（`keyring`，服务名 `com.sunkailun.issuerelay`），不写文件，不进日志，不出现在 IPC 响应和错误信息里。
- Claude Code、Codex、git、CI、测试环境的凭据，程序一概不碰，由各自的工具管理。
- 测试不得读写真实钥匙串；必须碰的测试标 `#[ignore]`，并写明手动运行的方式。

## 2. 执行者权限

- 默认档位「完全放开」意味着 AI 能执行任意命令、使用用户的全部凭据。用户首次选择时必须明确提示（M2 面板）。
- 三档权限的参数映射（设计 6.1）只能在设计更新后修改；`--permission-prompts none` 必须保留，避免无人值守时卡住。
- 固定规则替代不了仓库自身的保护：README 提醒用户为主分支开启分支保护。

## 3. 不可信输入

- Linear 的标题、描述、评论可能夹带指令（提示注入）。程序不把这些文本放进命令行；提示词里只放需求编号和链接，由 AI 用自己的 Linear 连接去读；固定规则写在提示词最前面，并声明优先于其他任何内容。
- Linear MCP 的返回按不可信数据解析：解析失败就返回错误，不猜测。
- 执行者的输出只用来记录和识别最终结果；成功与否以 Linear 上的真实状态为准，不以 AI 的自述为准。

## 4. 子进程

- 只用固定参数；提示词经标准输入传入；工作目录用 current_dir 设置；不经 shell。
- 结束运行一律结束整棵进程树：Unix 用进程组，Windows 用 Job Object；正常退出后也清理残留进程。
- 设有总时长上限和无输出上限。

## 5. 本机通信

- macOS：套接字所在目录权限 0700；启动前检测是否已有宿主在运行，只有残留的套接字文件才删除。
- Windows：命名管道设 `reject_remote_clients(true)`；用 `first_pipe_instance(true)` 保证只有一个宿主。
- 只提供 `METHODS` 列出的方法；「标记完成」「重试」「取消」只在面板（进程内）提供。

## 6. 日志与数据

- 运行日志可能包含代码和 AI 打印出来的敏感内容：只存在本机数据目录，按保留天数和总大小清理，不上传。
- 没有遥测，不向任何第三方发送数据（Linear 除外，且只经用户授权的 MCP 连接）。

## 7. 开源卫生

- 仓库是公开的。代码、测试、夹具、文档、提交信息里不得出现令牌、API key、公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径。
- 每次提交前跑 `bash scripts/check-secrets.sh`，CI 里用 `--strict`；GitHub 已开启密钥扫描与推送拦截。
- 来自真实运行的样本先脱敏再提交（见 `TESTING_SPEC.md`）。
- 发现疑似泄露只报告文件和行号；是否改写历史由用户决定。

## 8. 供应链

- 新依赖用 `cargo add` 引入，并说明用途；`Cargo.lock` 入库。
- GitHub Actions 只用官方或广泛使用的 action，并固定主版本。
- 不在构建脚本或 CI 里下载并执行来历不明的脚本。

## 9. AI 边界

AI 不替用户完成任何授权（Linear、Claude、Codex、GitHub），不改系统设置，不安装或卸载开机项，不改写已推送的历史。
````

`docs/ai-coding/TESTING_SPEC.md`：

`````markdown
# 测试分层规范

## 定位

IssueRelay 驱动的是真实的 Linear 和真实的 AI。纯逻辑在单元测试里完整覆盖；与外部的交互用假实现做集成测试；真实环境只在沙箱上验收，并写成报告。「测试通过」和「实机验收通过」永远分开记录。

## 分层

| 层 | 工具 | 对象 | 何时跑 |
|---|---|---|---|
| 单元测试 | `#[test]` / `#[tokio::test]`，写在源文件末尾的 `mod tests` | 配置、触发、调度、提示词、命令拼装、结果识别、解析、格式化 | 每次提交 |
| 集成测试 | `crates/*/tests/`，加上 `tests/common/` 的假实现 | 运行器 + `fake-agent`、Linear MCP + 假服务端、引擎 + 假 Linear + 脚本化运行器、IPC | 每次提交 |
| 平台测试 | `#[cfg(windows)]` / `#[cfg(unix)]` 的测试，CI 两个平台都跑 | `.cmd` 包装、命名管道、残留的套接字文件 | 推 main 与开 PR 时 |
| 实机验收 | 开发宿主（M2 起是托盘程序）+ 真实 Linear + 真实 Claude / Codex | 完整流程、耗时、失败与超时 | 每个里程碑收尾，写 `docs/gates/` |

## 单元与集成测试规则

- 不访问网络，不碰真实钥匙串，不依赖本机装的 `claude` / `codex`。
- 与时间有关的上限用毫秒级（例如卡住判定 400 毫秒），不写长时间的 `sleep`。
- 临时文件一律用 `tempfile`，不写本机固定路径。
- 每个状态、每个错误分支至少一个测试；修 bug 先写一个能复现它的测试。
- 测试数据只用虚构值：`DEMO-1`、`Demo Project`、`Jane Doe`、`user-1`、`proj-1`。

## 假实现清单

| 假实现 | 位置 | 替代什么 |
|---|---|---|
| `fake-agent` | `crates/core/src/bin/fake_agent.rs` | Claude Code / Codex 进程（按脚本输出、退出码、启动孙进程） |
| `FakeLinearServer`、`PartialServer` | `crates/core/tests/common/fake_server.rs` | Linear MCP 服务端（经内存管道连接） |
| `FakeLinear` | `crates/core/tests/common/fake_linear.rs` | `LinearApi` 的内存版 |
| `ScriptedRunner`、`FakeReadiness` | `crates/core/tests/common/engine_kit.rs` | 子进程运行与环境检查 |
| `FakeRunner` | `crates/core/src/doctor.rs` 的测试模块 | 检查命令的输出 |

## 夹具与脱敏

- 真实样本放在 `docs/gates/samples/<里程碑>/`，测试需要时复制到 `crates/<crate>/tests/fixtures/`。
- 提交前替换：姓名改为 `Jane Doe`，邮箱改为 `jane@example.com`，需求标识改为 `DEMO-N`，UUID 改为 `00000000-0000-0000-0000-00000000000N`，项目改为 `Demo Project`，团队改为 `Demo`，URL 改为 `https://linear.app/demo/issue/DEMO-N`，路径改为 `~/…`，会话 ID 改为全零。字段名与结构保持原样。
- 不为了让测试通过去手改真实样本；需要别的形态就另写一份虚构夹具。

## 实机验收规则

- 只在 Linear 沙箱项目和本机沙箱仓库（用本地裸仓库充当远端）上进行，不碰真实业务仓库。
- 记录环境与版本、每一步的命令和结果、从建需求到被接走的耗时、Linear 状态与评论、推送结果、异常与未验证项。
- 没做的项写「待实机」，不能用测试结果代替。

## 验收报告模板

````markdown
# <里程碑>：<主题>（<日期>）— <通过 | 部分通过 | 待实机>

## 环境
- 系统与版本；Claude Code / Codex 版本；IssueRelay 提交

## 步骤与结果
| 步骤 | 命令 | 结果 |
|---|---|---|

## 与设计不一致之处

## 未闭环
````
`````

`docs/ai-coding/AI_AUTOMATION_WORKFLOW.md`：

````markdown
# AI 自动化开发流程

## 一、标准流程

1. **澄清需求**：弄清目的、使用者、成功标准；有歧义就问用户，一次问一个问题。
2. **设计**：写进 `docs/superpowers/specs/<日期>-<主题>-design.md`，用户审阅同意后才进入下一步。
3. **实施计划**：写进 `docs/superpowers/plans/<日期>-<主题>.md`，每个任务列出文件、接口、测试、步骤和提交；用户审阅并选择执行方式。
4. **按任务实现**：先写失败的测试，再写最少的实现，测试通过后提交；一个任务一个提交。
5. **全量验证**：四条验证命令。
6. **实机验收**：高风险及以上的改动在沙箱上验收，写 `docs/gates/` 报告。
7. **推送**：推送前 `git fetch`，必要时 rebase 并重跑验证。
8. **归档**：用户要求时，把结论归档进作者的知识库。

## 二、需求说明模板

```markdown
# 需求说明 - <主题>
## 背景
## 目标
## 范围（做什么 / 不做什么）
## 需要用户做的事（授权、提供环境）
```

## 三、验收标准模板

```markdown
# 验收标准 - <主题>
## 测试（新增哪些、覆盖哪些分支）
## 安全（凭据、开源卫生、子进程、标记完成边界）
## 实机（沙箱上的步骤与期望结果）
## 文档（需要同步的文件）
```

## 四、多智能体流程

1. 主智能体读规范、设计和计划，划分任务与文件边界。
2. 子智能体按任务实现或审查；每个任务完成后由新的审查者检查，通过后才进入下一个任务。
3. 极高风险文件（`executor`、`prompt`、`runner`、`linear::auth`、`ipc::protocol`）由主智能体串行修改。
4. 主智能体汇总结果、跑全量验证、做实机验收、写交付说明。

## 五、子智能体输出要求

- 改了哪些文件、为什么改；跑了哪些命令、结果和测试数量。
- 没做的、拿不准的、需要主智能体决定的事项。
- 不贴密钥、真实需求内容、本机路径。

## 六、交付说明

使用 `AI_ENGINEERING_GUARDRAILS.md` 第 8 节的模板。
````

- [ ] **Step 6: 写验收报告目录说明**

`docs/gates/README.md`：

```markdown
# 验收报告

每次实机验收写一份 `<YYYY-MM-DD>-<主题>.md`，结构与脱敏规则见 `docs/ai-coding/TESTING_SPEC.md`。真实运行的样本放 `samples/<里程碑>/`，提交前必须脱敏。
```

- [ ] **Step 7: 写密钥扫描脚本**

`scripts/check-secrets.sh`：

```bash
#!/usr/bin/env bash
# Scan tracked files for values that must never be committed: private keys,
# Linear / Anthropic / OpenAI / GitHub / AWS tokens, private network addresses
# and machine-specific absolute paths. Prints file:line for each hit. Exits 0
# unless --strict is given and something matched. New files are scanned only
# after `git add`.
set -euo pipefail
cd "$(dirname "$0")/.."
STRICT=0
[[ "${1:-}" == "--strict" ]] && STRICT=1

PATTERNS=(
  'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY'
  'lin_(api|oauth)_[A-Za-z0-9]{16,}'
  'sk-ant-[A-Za-z0-9_-]{16,}'
  'sk-[A-Za-z0-9]{32,}'
  'gh[pousr]_[A-Za-z0-9]{30,}'
  'AKIA[0-9A-Z]{16}'
  '(^|[^0-9.])10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}'
  '(^|[^0-9.])192\.168\.[0-9]{1,3}\.[0-9]{1,3}'
  '(^|[^0-9.])172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}'
  '/Users/[A-Za-z0-9._-]+/'
  '/Volumes/[^ /]+/'
  '[A-Za-z]:\\Users\\[A-Za-z0-9._-]+'
)
EXCLUDES=(':!scripts/check-secrets.sh' ':!Cargo.lock' ':!LICENSE')

hits=0
for p in "${PATTERNS[@]}"; do
  while IFS= read -r loc; do
    [[ -z "$loc" ]] && continue
    echo "check-secrets: $loc"
    hits=$((hits + 1))
  done < <(git grep -n -I -E -e "$p" -- . "${EXCLUDES[@]}" | cut -d: -f1,2 || true)
done

if [[ $hits -gt 0 ]]; then
  echo "check-secrets: $hits hit(s)"
  [[ $STRICT -eq 1 ]] && exit 1
fi
exit 0
```

Run: `chmod +x scripts/check-secrets.sh`

- [ ] **Step 8: 写 CI**

`.github/workflows/ci.yml`：

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v5
      - run: rustup show
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace

  secrets:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - run: bash scripts/check-secrets.sh --strict
```

- [ ] **Step 9: 本地验证**

Run: `cargo build --workspace && cargo test --workspace && git add -A && bash scripts/check-secrets.sh --strict`
Expected: 构建成功；`test result: ok. 0 passed`；check-secrets 无输出、退出码 0

- [ ] **Step 10: 提交并推送**

```bash
git add -A
git commit -m "chore: scaffold workspace, AI coding rules, secret scan and CI

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git fetch origin && git push origin main
```

推送后用 `gh run list --limit 1` 看一次 CI 结果（不轮询）；失败就按日志修复后重推。

---

### Task 2: M0 沙箱（Linear 测试项目与本地 git 沙箱）

**Files:**
- 仓库内不新增文件；沙箱在仓库外的 `~/issuerelay-sandbox/`，路径不得写进仓库。

**Interfaces:**
- Consumes: 无
- Produces: Linear 项目「IssueRelay Sandbox」及两条测试需求（下称 SANDBOX-A、SANDBOX-B，实际编号以 Linear 为准）；本机 `~/issuerelay-sandbox/remote.git`（裸仓库，充当远端）与 `~/issuerelay-sandbox/demo`（工作副本，含 `main`、`test` 分支和 `AGENTS.md`）

- [ ] **Step 1: 在 Linear 建测试项目和两条需求（先征得用户同意）**

在用户自己的 Linear 工作区建项目「IssueRelay Sandbox」，再建两条需求，都要：指派给用户本人、加标签 `ai-ready`、状态 Todo、属于该项目。可以在征得同意后用会话里的 Linear 工具创建，也可以请用户在网页上建。

- SANDBOX-A 标题：`Sandbox: add hello.txt`；描述：

  ```text
  仓库：本机沙箱 demo（工作目录下的 demo 文件夹），测试分支 test。
  需求：在仓库根目录新增 hello.txt，内容一行：hello from IssueRelay
  验收标准：test 分支上有 hello.txt 且内容正确；不改其他文件。
  ```

- SANDBOX-B 标题：`Sandbox: add usage line to README`；描述：

  ```text
  仓库：本机沙箱 demo（工作目录下的 demo 文件夹），测试分支 test。
  需求：在 README.md 末尾加一行：Usage: see hello.txt
  验收标准：test 分支上的 README.md 末行是这句话；不改其他文件。
  ```

同时确认工作区里已有标签 `ai-ready` 和状态 In Progress、In Testing、Done；缺了就请用户在 Linear 的团队设置里补上。

- [ ] **Step 2: 建本机 git 沙箱**

```bash
SANDBOX="$HOME/issuerelay-sandbox"
mkdir -p "$SANDBOX" && cd "$SANDBOX"
git init --bare -b main remote.git
git clone remote.git demo && cd demo
printf '# demo\n' > README.md
cat > AGENTS.md <<'EOF'
# Sandbox rules
- Create a branch from `test` named `feature/<issue-identifier-lowercase>`.
- When done, merge it into `test`, run `git push origin test`, then delete the feature branch.
- Never push `main`.
- Validation: the files named in the issue exist with the requested content.
EOF
git add . && git commit -m "chore: sandbox init" && git push origin main
git checkout -b test && git push -u origin test
git -C "$SANDBOX/remote.git" branch
```

Expected: 最后一条输出 `main` 与 `test` 两个分支。

---

### Task 3: M0 Linear MCP 探针

> 执行记录（2026-10-11）：已完成。实测发现 Linear 要求回调带 RFC 9207 的 `iss`（M0 发现 F5），下方探针代码已改为把完整回调地址交给 `handle_callback_url`；Step 1 补了 `cargo add` 之前的占位 `src/main.rs`；原始输出改为固定写在探针目录里。

**Files:**
- Create: `spikes/linear-mcp-probe/Cargo.toml`、`spikes/linear-mcp-probe/src/main.rs`、`spikes/README.md`
- Create（运行后，脱敏）：`docs/gates/samples/m0/linear/*.json`

**Interfaces:**
- Consumes: Task 2 的 Linear 测试项目与需求
- Produces: 已验证的 OAuth 回环授权写法（Task 17 照抄）；Linear MCP 各工具的真实返回样本（Task 16 的解析与夹具以此为准）

- [ ] **Step 1: 建探针工程**

```bash
mkdir -p spikes/linear-mcp-probe/src
cd spikes/linear-mcp-probe
cat > Cargo.toml <<'EOF'
[package]
name = "linear-mcp-probe"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]
EOF
printf 'fn main() {}\n' > src/main.rs   # cargo add 需要已有构建目标；Step 2 会整体替换
cargo add -q anyhow async-trait keyring open serde_json url
cargo add -q reqwest@0.13
cargo add -q tokio --features full
cargo add -q rmcp@3.5 --features client,transport-streamable-http-client-reqwest,auth,reqwest
cd ../..
```

`spikes/README.md`：

```markdown
# spikes

一次性验证用的独立 cargo 工程，不属于工作区，不随产品发布。`linear-mcp-probe` 是 M0 用来验证 Linear MCP OAuth 授权与工具返回格式的探针，结论见 `docs/gates/` 的 M0 验收报告。运行时生成的原始输出固定写到探针目录下的 `samples-raw/`（不论从哪个目录运行；已被 .gitignore 排除），脱敏后才放进 `docs/gates/samples/`。
```

- [ ] **Step 2: 写探针代码**

`spikes/linear-mcp-probe/src/main.rs`：

```rust
//! M0 probe: authorize against Linear's MCP server with OAuth (loopback redirect),
//! store the token in the system keychain, and dump raw tool output for the parser.
use std::path::Path;

use anyhow::{Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, ClientConfig},
    service::RunningService,
    transport::{
        StreamableHttpClientTransport,
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore,
            OAuthState, StoredCredentials,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const MCP_URL: &str = "https://mcp.linear.app/mcp";
const SERVICE: &str = "com.sunkailun.issuerelay.probe";
const ACCOUNT: &str = "linear-oauth";
const FIELDS: [&str; 13] = [
    "id", "uuid", "title", "url", "status", "labels", "project", "projectId", "assigneeId",
    "parentId", "priority", "createdAt", "updatedAt",
];

type Client = RunningService<RoleClient, ClientConfig>;

struct KeyringStore;

fn entry() -> Result<keyring::Entry, AuthError> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| AuthError::CredentialStoreError(e.to_string()))
}

#[async_trait::async_trait]
impl CredentialStore for KeyringStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match entry()?.get_password() {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| AuthError::CredentialStoreError(e.to_string())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        entry()?
            .set_password(&text)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))
    }

    async fn clear(&self) -> Result<(), AuthError> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("login") => login().await,
        Some("dump") => dump().await,
        _ => bail!("usage: linear-mcp-probe login|dump"),
    }
}

async fn login() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());
    let mut state = OAuthState::new(MCP_URL, None).await?;
    if let OAuthState::Unauthorized(manager) = &mut state {
        manager.set_credential_store(KeyringStore);
    }
    state
        .start_authorization(AuthorizationRequest::new(redirect.as_str()).with_client_name("IssueRelay probe"))
        .await?;
    let url = state.get_authorization_url().await?;
    println!("Open this URL to authorize:\n{url}");
    let _ = open::that_detached(&url);
    let callback = wait_for_callback(&listener).await?;
    state.handle_callback_url(&callback).await?;
    let scopes = KeyringStore.load().await?.map(|c| c.granted_scopes).unwrap_or_default();
    println!("authorized; granted scopes: {scopes:?}");
    Ok(())
}

/// Waits for the browser redirect and returns the full callback URL, so rmcp can
/// validate `code`, `state` and the RFC 9207 `iss` parameter itself.
async fn wait_for_callback(listener: &TcpListener) -> Result<String> {
    let port = listener.local_addr()?.port();
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);
        if let Some(url) = callback_url(request.lines().next().unwrap_or_default(), port) {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<h1>Authorized. You can close this tab.</h1>")
                .await?;
            return Ok(url);
        }
        socket.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n").await?;
    }
}

/// Any request to /callback is the redirect, including an OAuth error response; rmcp then
/// fails on the missing `code` (it does not read `error`) instead of the probe waiting forever.
fn callback_url(request_line: &str, port: u16) -> Option<String> {
    let target = request_line.split_whitespace().nth(1)?;
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).ok()?;
    (url.path() == "/callback").then(|| url.to_string())
}

async fn connect() -> Result<Client> {
    let mut manager = AuthorizationManager::new(MCP_URL).await?;
    manager.set_credential_store(KeyringStore);
    if !manager.initialize_from_store().await? {
        bail!("not logged in; run `login` first");
    }
    let transport = StreamableHttpClientTransport::with_client(
        AuthClient::new(reqwest::Client::default(), manager),
        StreamableHttpClientTransportConfig::with_uri(MCP_URL),
    );
    Ok(ClientConfig::default().serve(transport).await?)
}

async fn call(client: &Client, tool: &'static str, args: Value) -> Result<String> {
    let Value::Object(map) = args else { bail!("arguments must be an object") };
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(map))
        .await?;
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    Ok(if result.is_error.unwrap_or(false) { format!("ERROR: {text}") } else { text })
}

fn save(dir: &Path, name: &str, text: &str) -> Result<()> {
    std::fs::write(dir.join(format!("{name}.json")), text)?;
    println!("saved {name}");
    Ok(())
}

async fn dump() -> Result<()> {
    // Raw output stays inside the probe directory (git-ignored) wherever cargo runs from.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples-raw");
    std::fs::create_dir_all(&dir)?;
    let client = connect().await?;
    let tools = client.list_all_tools().await?;
    std::fs::write(dir.join("tools.json"), serde_json::to_string_pretty(&tools)?)?;
    save(&dir, "get_user_me", &call(&client, "get_user", json!({"query": "me"})).await?)?;
    let page1 = call(&client, "list_issues", json!({"assignee": "me", "label": "ai-ready", "limit": 1, "fields": FIELDS})).await?;
    save(&dir, "list_issues_page1", &page1)?;
    let v: Value = serde_json::from_str(&page1).unwrap_or(Value::Null);
    if let Some(cursor) = ["cursor", "nextCursor", "endCursor"].iter().find_map(|k| v.get(*k).and_then(Value::as_str)) {
        let page2 = call(&client, "list_issues", json!({"assignee": "me", "label": "ai-ready", "limit": 1, "cursor": cursor, "fields": FIELDS})).await?;
        save(&dir, "list_issues_page2", &page2)?;
    }
    if let Some(id) = v["issues"][0]["id"].as_str() {
        save(&dir, "get_issue", &call(&client, "get_issue", json!({"id": id, "fields": FIELDS})).await?)?;
    }
    save(&dir, "get_issue_missing", &call(&client, "get_issue", json!({"id": "ZZZ-999999"})).await?)?;
    save(&dir, "list_projects", &call(&client, "list_projects", json!({"limit": 50})).await?)?;
    save(&dir, "list_issue_labels", &call(&client, "list_issue_labels", json!({"limit": 100})).await?)?;
    let teams = call(&client, "list_teams", json!({})).await?;
    save(&dir, "list_teams", &teams)?;
    let tv: Value = serde_json::from_str(&teams).unwrap_or(Value::Null);
    if let Some(team) = tv["teams"][0]["id"].as_str() {
        save(&dir, "list_issue_statuses", &call(&client, "list_issue_statuses", json!({"team": team})).await?)?;
    }
    client.cancel().await?;
    Ok(())
}
```

Run: `cargo build --manifest-path spikes/linear-mcp-probe/Cargo.toml`
Expected: 编译成功

- [ ] **Step 3: 授权（用户操作）**

Run: `cd spikes/linear-mcp-probe && cargo run -q -- login`
用户在弹出的浏览器里完成 Linear 授权。
Expected: 终端打印 `authorized; granted scopes: [...]`。记下授予的 scope，确认包含写权限（例如 `write`）；如果只有只读，在验收报告里记为待解决问题。

- [ ] **Step 4: 抓取原始样本**

Run: `cd spikes/linear-mcp-probe && cargo run -q -- dump && ls samples-raw`
Expected: 生成 `tools.json`、`get_user_me.json`、`list_issues_page1.json`、`list_issues_page2.json`（沙箱里有两条 `ai-ready` 需求时才有）、`get_issue.json`、`get_issue_missing.json`、`list_projects.json`、`list_issue_labels.json`、`list_teams.json`、`list_issue_statuses.json`。

逐个核对并记进 Task 6 的验收报告：
1. `tools.json` 里有没有这 8 个工具：`list_issues`、`get_issue`、`save_issue`、`get_user`、`list_projects`、`list_issue_labels`、`list_teams`、`list_issue_statuses`，以及 `list_issues` 是否接受 `assignee`、`label`、`state`、`limit`、`cursor`、`fields` 这些参数。
2. 翻页：`hasNextPage` 为真时，游标字段叫什么（`cursor`、`nextCursor` 还是 `endCursor`）。
3. 需求对象里各字段的实际形态：`id` 是标识符还是 UUID、有没有 `uuid`、`status` 是字符串还是对象、`labels` 是名称数组还是对象数组、`priority` 是数字还是对象、`projectId` 是否和 `list_projects` 返回的项目 `id` 是同一种 ID。
4. `get_issue_missing.json`：查不到需求时是工具错误（`ERROR:` 开头）还是正常返回，错误文字里有没有 `not found`。
5. `get_user_me.json` 里当前用户 ID 字段的名字。

- [ ] **Step 5: 脱敏后存样本并提交**

把 `samples-raw/` 里的每个文件复制到 `docs/gates/samples/m0/linear/`，按 `docs/gates/README.md` 的规则脱敏：真实姓名改为 `Jane Doe`，邮箱改为 `jane@example.com`，需求标识改为 `DEMO-1`、`DEMO-2`，UUID 改为 `00000000-0000-0000-0000-00000000000N`，项目名改为 `Demo Project`，团队名改为 `Demo`，URL 改为 `https://linear.app/demo/issue/DEMO-1`；字段名与结构保持原样。`tools.json` 只保留上面 8 个工具。

```bash
git add spikes docs/gates/samples/m0/linear
bash scripts/check-secrets.sh --strict
git commit -m "test(m0): add Linear MCP probe and redacted samples

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: M0 macOS 无人值守执行实测

**Files:**
- Create（脱敏）：`docs/gates/samples/m0/executors/claude-stream.jsonl`、`codex-exec.jsonl`、`claude-mcp-list.txt`、`codex-mcp-list.txt`、`claude-auth-status.json`、`codex-login-status.txt`

**Interfaces:**
- Consumes: Task 2 的沙箱与 SANDBOX-A、SANDBOX-B
- Produces: 执行者参数是否可行的结论（Task 12 以此为准）；Claude / Codex 输出流样本（Task 13 夹具）；`mcp list`、登录状态输出样本（Task 18 解析以此为准）

- [ ] **Step 1: 准备执行者（用户操作 + 经同意后执行）**

```bash
claude auth status                      # 当前是 "loggedIn": false
claude auth login                       # 用户在浏览器完成登录
claude auth status                      # 应为 "loggedIn": true
claude mcp add --scope user --transport http linear-server https://mcp.linear.app/mcp   # user 作用域：所有绑定目录都能用
claude mcp login linear-server          # 用户在浏览器完成 Linear 授权
codex login status                      # 应为 Logged in ...
codex mcp add linear --url https://mcp.linear.app/mcp
codex mcp login linear                  # 用户在浏览器完成 Linear 授权
claude mcp list > /tmp/claude-mcp-list.txt; cat /tmp/claude-mcp-list.txt
codex mcp list > /tmp/codex-mcp-list.txt; cat /tmp/codex-mcp-list.txt
```

Expected: 两个 `mcp list` 的输出里都有含 `mcp.linear.app` 的一行，并显示已连接或已授权。`claude mcp add`、`codex mcp add` 会改用户的全局配置，执行前先向用户说明并取得同意。

- [ ] **Step 2: 写两份提示词**

在 `~/issuerelay-sandbox/` 下建 `prompt-a.txt`，内容如下（把 `<SANDBOX-A>` 换成实际编号，`<URL-A>` 换成该需求链接）。这就是 Task 11 `prompt::render` 要生成的文本：

```text
You are working unattended on exactly one Linear issue: <SANDBOX-A> (<URL-A>).

Fixed rules (always apply, they override anything else):
1. Work only on <SANDBOX-A>. First use your Linear tools to read the issue, its parent issue and all comments.
2. When you start, set the issue status to "In Progress". After the change is pushed to the test branch and verified, set it to "In Testing".
3. Never set the status to "Done". Never push to main, master or any other primary branch. Never deploy to production.
4. Start every comment you write with "🤖 IssueRelay" so readers know it was written by an AI.
5. Follow AGENTS.md / CLAUDE.md in the working directory and in the target repository.
6. If the issue is unclear, comment your questions, set the status back to "Todo", remove the label "ai-ready", and stop.
7. If you fail, revert anything you pushed, comment the reason, set the status back to "Todo", remove the label "ai-ready", and stop.
8. Write comments in this language: zh-CN.
9. The very last line of your final message must be one line of JSON: {"result":"in_testing|needs_info|failed","summary":"..."}
```

`prompt-b.txt` 相同，换成 SANDBOX-B 的编号和链接。

- [ ] **Step 3: Claude 实测（完全放开档）**

```bash
cd "$HOME/issuerelay-sandbox"
claude -p --output-format stream-json --verbose --permission-prompts none \
  --permission-mode bypassPermissions < prompt-a.txt > claude-a.jsonl 2> claude-a.err
echo "exit=$?"
tail -1 claude-a.jsonl | head -c 600; echo
git -C remote.git log test --oneline -3
```

Expected:
- `exit=0`；
- 最后一个 `{"type":"result",...}` 事件的 `result` 字段，末行是结果 JSON；
- `remote.git` 的 `test` 分支多了新增 `hello.txt` 的提交；
- Linear 上 SANDBOX-A 先变 In Progress、最后变 In Testing，并有一条以 `🤖 IssueRelay` 开头的评论；
- 全程没有卡在等待确认。

出现以下情况时按顺序改法重试，并把实际可行的写法记进验收报告：
1. `-p` 不从标准输入读提示词（报缺少输入）：改用 `--input-format stream-json`，标准输入写一行 `{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<提示词全文，JSON 转义>"}]}}`。
2. `--permission-mode bypassPermissions` 在 `-p` 下被拒绝：改用 `--dangerously-skip-permissions`。

- [ ] **Step 4: Codex 实测（完全放开档，目录不是 git 仓库）**

```bash
cd "$HOME/issuerelay-sandbox"
codex exec --json --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check - \
  < prompt-b.txt > codex-b.jsonl 2> codex-b.err
echo "exit=$?"
python3 -c 'import json,sys,collections; print(collections.Counter(json.loads(l).get("type") for l in open("codex-b.jsonl") if l.strip().startswith("{")))'
git -C remote.git log test --oneline -3
```

Expected：与 Step 3 相同的四项结果（换成 SANDBOX-B 与 README 改动）。另外记录：事件类型统计；最终消息出现在哪种事件里（本计划假设是 `type` 为 `item.completed`、`item.type` 为 `agent_message` 的 `item.text`）。

- [ ] **Step 5: 存脱敏样本并提交**

- `claude-stream.jsonl`：从 `claude-a.jsonl` 保留 `system/init` 一行、一到两行 assistant、最后的 `result` 一行；
- `codex-exec.jsonl`：从 `codex-b.jsonl` 保留 `thread.started`、一两条 `item.*`、最后一条 agent 消息和 `turn.completed`；
- 两个 `mcp list` 文件原样保存；
- `claude auth status` 只保留 `{"loggedIn": true}`；`codex login status` 原样保存。

全部按 `docs/gates/README.md` 脱敏（路径改成 `~/issuerelay-sandbox/...`，会话 ID 改成 `00000000-...`）。

```bash
git add docs/gates/samples/m0/executors
bash scripts/check-secrets.sh --strict
git commit -m "test(m0): add redacted Claude and Codex headless run samples

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: M0 Windows 无人值守执行实测

**Files:**
- Create（脱敏，可选）：`docs/gates/samples/m0/executors/windows-*.jsonl`

**Interfaces:**
- Consumes: Task 4 的提示词与做法
- Produces: Windows 上 `.cmd` 包装、标准输入传提示词、Linear MCP 是否可行的结论

- [ ] **Step 1: 确认有无 Windows 环境**

问用户是否有可用的 Windows 实机或虚拟机（设计第 18 节待定问题 4）。没有就跳过本任务，在 Task 6 的报告里写「Windows 未验证，风险留到 M2」，然后继续。

- [ ] **Step 2: 在 Windows 上重复 Task 2 Step 2 与 Task 4**

在 Windows 上装好 Git、Node、Claude Code、Codex，按 Task 4 Step 1 配置登录和 Linear MCP。用 `cmd.exe` 的重定向执行，避免 PowerShell 管道改写中文编码：

```bat
cd /d %USERPROFILE%\issuerelay-sandbox
where claude
where codex
cmd /c "claude -p --output-format stream-json --verbose --permission-prompts none --permission-mode bypassPermissions < prompt-a.txt > claude-a.jsonl 2> claude-a.err"
echo exit=%ERRORLEVEL%
cmd /c "codex exec --json --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check - < prompt-b.txt > codex-b.jsonl 2> codex-b.err"
echo exit=%ERRORLEVEL%
```

需求和沙箱用 Task 2 新建的同类需求（不要复用已到 In Testing 的那两条）。

Expected：与 Task 4 相同的验收项。另外记录 `where claude`、`where codex` 输出的是 `.exe` 还是 `.cmd`（Task 15 的 Windows 测试要覆盖 `.cmd`）。

---

### Task 6: M0 验收报告与设计表更新

**Files:**
- Create: `docs/gates/2026-10-11-m0-verification.md`（日期按实际完成日）
- Modify（仅当实测与设计不符）：`docs/superpowers/specs/2026-10-10-issuerelay-design.md` 第 6.1、7 节；本计划的 Global Constraints 与 Task 12、13、16、18

**Interfaces:**
- Consumes: Task 3–5 的结论与样本
- Produces: M1 任务依据的「实测口径」

- [ ] **Step 1: 写验收报告**

按以下结构写：

```markdown
# M0 实测验证（<日期>）

## 环境
- macOS <版本>；Claude Code <版本>；codex-cli <版本>；rustc <版本>
- Windows：<版本，或「未验证」>

## Linear MCP
| 检查项 | 结果 |
|---|---|
| OAuth 回环授权（127.0.0.1 随机端口） | 通过 / 失败（原因） |
| 授予的 scope | … |
| 8 个必需工具都在 | 是 / 缺 … |
| 翻页游标字段 | cursor / nextCursor / endCursor |
| 需求字段形态 | id=…；uuid=…；status=…；labels=…；priority=…；projectId 与项目 id 是否同一种 |
| 查不到需求时的返回 | … |

## 执行者
| 组合 | 命令 | 退出码 | Linear 状态变化 | 推送结果 | 卡在确认 | 备注 |
|---|---|---|---|---|---|---|
| macOS + Claude + 完全放开 | … | … | … | … | 否 | 标准输入是否生效 |
| macOS + Codex + 完全放开 | … | … | … | … | 否 | 最终消息所在事件 |
| Windows + Claude | … | … | … | … | … | where 结果 |
| Windows + Codex | … | … | … | … | … | … |

## 与设计不一致之处及处理

## 未闭环
```

- [ ] **Step 2: 按实测改设计与计划**

只在实测与假设不符时修改，逐条对应：
- 权限参数或标准输入写法不同 → 改设计 6.1 的表、Global Constraints 的执行者参数、Task 12 的 `build` 及其测试；
- Codex 最终消息不在 `item.completed` / `agent_message` → 改 Task 13 的 `observe` 及测试；
- 游标字段、需求字段形态不同 → 改 Task 16 的 `parse.rs`、夹具与测试；
- `mcp list` 或登录状态的输出格式不同 → 改 Task 18 的解析函数与测试。

- [ ] **Step 3: 提交**

```bash
git add docs
bash scripts/check-secrets.sh --strict
git commit -m "docs(m0): record verification results and align spec

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git fetch origin && git push origin main
```

---

### Task 6b: 贡献与共同维护

用户 2026-10-10 要求：仓库要能接受别人贡献代码，开发者可以一起维护。依据设计第 16.1 节。GitHub 仓库设置已由维护者在用户同意后配置好，本任务只写仓库里的文件，文件内容必须与这些设置一致：

- 规则集 `main`（作用于默认分支）：禁止删除、禁止强推；合并必须走 PR，至少 1 人批准，批准后再有推送需重新批准，评审对话必须全部解决，只允许 squash；必需检查 `test (macos-latest)`、`test (windows-latest)`、`secrets`（GitHub Actions）；仓库管理员可以绕过。
- 仓库只允许 squash 合并，提交标题取 PR 标题、正文取 PR 描述，合并后自动删除分支。
- 已开启：私密漏洞报告、Dependabot 安全更新、密钥扫描与推送拦截。未开启 Discussions。

**Files:**
- Create: `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`MAINTAINERS.md`
- Create: `.github/CODEOWNERS`、`.github/pull_request_template.md`、`.github/ISSUE_TEMPLATE/bug_report.yml`、`.github/ISSUE_TEMPLATE/feature_request.yml`、`.github/ISSUE_TEMPLATE/config.yml`、`.github/dependabot.yml`
- Modify: `README.md`（加「参与贡献」一节）、`AGENTS.md`（加「贡献流程」一节）、`docs/ai-coding/AI_AUTOMATION_WORKFLOW.md`（加「七、外部贡献」）、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 2、6 节登记新文件）

**Interfaces:**
- Consumes: Task 1 的 `AGENTS.md`、`docs/ai-coding/`、`ci.yml` 的三个任务名、`scripts/check-secrets.sh --strict`
- Produces: 外部贡献者与共同维护者的入口文档；后续任务新增高风险文件时同步更新 `.github/CODEOWNERS`

**写作要求（四份说明文档）**：
- 正文用中文，开头放一段英文摘要（3–8 行，标题 `## English summary`），方便不读中文的贡献者。
- 硬规则只在 `AGENTS.md` 写一份，其他文档链接过去，不逐条复述；需要点名时只列名称。
- 语句短、先写结论；每节回答一个问题；命令放进代码块；不写空话。
- 不写任何公司名、个人邮箱、手机号或本机路径。联系渠道只用 GitHub（私密漏洞报告、issue、@维护者）。

- [ ] **Step 1: 写 `CONTRIBUTING.md`**

按顺序包含以下各节，每节内容至少覆盖所列要点：

1. `# 参与贡献` 与 `## English summary`：欢迎 issue 和 PR；先读 `AGENTS.md`（AI 规则对人同样适用）；提交前跑四条检查；提交信息用英文 Conventional Commits；不贴令牌、公司信息、真实 Linear 内容；安全问题走私密漏洞报告；贡献按 Apache-2.0 授权。
2. `## 可以贡献什么`：问题报告、文档、测试、修 bug、新功能（先开 issue 讨论）；M2 起的界面翻译（每种语言一个翻译文件，见设计 9.4）。标签 `good first issue`、`help wanted` 适合新人。
3. `## 开始之前`：大改动和新功能先开 issue 说明要解决的问题和方案，维护者确认后再写代码；改动涉及高风险范围时必须先在 issue 里写方案，高风险范围按 `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 的「极高风险」列表，点名为：执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权、IPC 方法表、标记完成。
4. `## 开发环境`：macOS 或 Windows；装好 rustup 后在仓库根目录运行 `rustup toolchain install`，按 `rust-toolchain.toml` 装好 1.99.0 及 rustfmt、clippy；克隆后 `cargo build --workspace`；单元与集成测试用假执行者和假 Linear MCP，不需要 Linear 账号，也不需要安装 Claude Code 或 Codex；实机验收需要自己的 Linear 测试项目、Claude Code 或 Codex，步骤见 `docs/gates/` 和 `docs/ai-coding/TESTING_SPEC.md`。
5. `## 提交前检查`：代码块列出四条命令，与 `AGENTS.md` 的「验证命令」完全一致：
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   bash scripts/check-secrets.sh --strict
   ```
   写明：密钥扫描必须无输出、退出码 0；CI 会在 macOS、Windows 上跑前三条，在 Ubuntu 上跑密钥扫描，三项都通过才能合并。
6. `## 分支、提交与 PR`，分条写：
   - fork 后从最新 `main` 切分支，建议命名 `feat/<主题>`、`fix/<主题>`；一个 PR 只做一件事。
   - 提交信息和 PR 标题用英文 Conventional Commits（`feat`、`fix`、`docs`、`test`、`refactor`、`chore`、`ci`，可带范围，如 `fix(runner): ...`）。squash 合并时 PR 标题就是 `main` 上的提交标题，所以 PR 标题也要按格式写。
   - PR 描述按模板填；草稿 PR 可以提前开。
   - 合并条件：CI 三项全绿、至少一名维护者批准、评审对话全部解决；由维护者 squash 合并，合并时把提交正文精简成英文摘要。
   - 已推送的分支不要 rebase 或强推（`AGENTS.md` 禁止强推）。需要更新时追加提交；要同步 `main` 就把 `main` 合并进你的分支，最终会 squash 成一个提交，分支历史不重要。
7. `## 代码与文档规范`：先读 `AGENTS.md` 和 `docs/ai-coding/README.md`；两个平台同等对待；新行为要有测试，修 bug 先写能复现的测试（`docs/ai-coding/TESTING_SPEC.md`）；测试通过不等于实机验收，涉及真实 Linear、Claude Code 或 Codex 行为的改动要在 PR 里写实机结果或「待实机」；设计、计划、验收报告用中文，`README.md` 中英双语；改了行为同步改文档。
8. `## 使用 AI 辅助`：欢迎；Codex 直接读 `AGENTS.md`，Claude Code 经 `CLAUDE.md` 导入，GitHub Copilot 读 `.github/copilot-instructions.md`；提交者对 AI 生成的每一行负责，提交前自己审一遍；PR 模板里勾选是否含 AI 生成内容。
9. `## 公开仓库红线`：本仓库公开，issue、PR 描述、评论、截图、日志、测试夹具里都不能出现令牌或凭据、公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径；贴日志前先脱敏（规则见 `docs/ai-coding/TESTING_SPEC.md` 的「夹具与脱敏」）；`scripts/check-secrets.sh` 只能兜底；万一泄露了令牌，先去对应服务吊销，再联系维护者清理。
10. `## 许可证`：贡献按 Apache-2.0 授权（许可证第 5 条：提交即按同一许可证授权），不需要签 CLA，也不要求 DCO 签名。
11. `## 行为准则与安全问题`：链接 `CODE_OF_CONDUCT.md`、`SECURITY.md`、`MAINTAINERS.md`。

- [ ] **Step 2: 写 `CODE_OF_CONDUCT.md`**

用自己的话写，不照抄任何现成文本；末尾注明「参考 Contributor Covenant 2.1（https://www.contributor-covenant.org/version/2/1/code_of_conduct/）」。各节：

1. `# 行为准则` 与 `## English summary`。
2. `## 我们的承诺`：不论经验、性别、年龄、国籍、民族、宗教、残障、外貌等，所有参与者都受到尊重。
3. `## 期望的行为`：就事论事、给出可执行的意见、接受批评、承认错误、照顾新人。
4. `## 不可接受的行为`：骚扰、人身攻击、歧视性言论、公开他人的私人信息、未经允许公开任何公司或客户的信息、持续打断讨论。
5. `## 适用范围`：本仓库的 issue、PR、评审、提交信息，以及代表本项目发言的场合。
6. `## 举报`：在仓库的 Security 页面点「Report a vulnerability」私下提交，标题写「CoC report」。这是本仓库唯一的私下渠道，只有维护者能看到；不要在公开 issue 里写细节。维护者对举报人身份保密。
7. `## 处理方式`：按情节依次为提醒、书面警告、临时禁止参与、永久禁止参与；由维护者决定，管理员有最终决定权。

- [ ] **Step 3: 写 `SECURITY.md`**

1. `# 安全政策` 与 `## English summary`。
2. `## 支持的版本`：发布第一版前只支持 `main`；发布后支持最新版本和 `main`。
3. `## 如何报告`：在仓库的 Security 页面点「Report a vulnerability」私下提交（已开启私密漏洞报告）；不要开公开 issue；写清影响的版本或提交、复现步骤、影响范围；报告里不要放真实令牌，已经泄露的令牌先去对应服务吊销。
4. `## 关注范围`：令牌或凭据被写进日志、文件或进程参数；「标记完成」可以经 IPC、命令行或 MCP 触发；本机 IPC 能被其他用户访问；Linear 文本被拼进子进程参数或经 shell 执行；结束运行后留下子进程；提示词三条固定规则被程序本身删改或绕过；`scripts/check-secrets.sh` 可被绕过导致泄露。
5. `## 不在范围内`：用户自己选择「完全放开」档位后，AI 在用户绑定的仓库里按需求内容执行命令（这是文档写明的风险模型，见 `README.md` 的安全须知）；Claude Code、Codex、Linear 自身的漏洞请报给对应厂商。
6. `## 响应`：这是志愿维护的项目，尽力在 7 天内确认收到；修复后发布安全公告，报告人愿意的话在公告里致谢。

- [ ] **Step 4: 写 `MAINTAINERS.md`**

1. `# 维护者与治理` 与 `## English summary`。
2. `## 当前维护者`：表格，列「GitHub 账号 / 角色 / 负责范围」，一行：`@sunkailun1992` / 管理员（项目发起人） / 全部。
3. `## 角色`：贡献者（提过 issue 或 PR 的任何人）；维护者（GitHub write 或 maintain 权限：评审与合并 PR、分诊 issue）；管理员（admin 权限：仓库设置、规则集、密钥与发布）。
4. `## 维护者职责`：评审 PR、分诊 issue（加标签、关闭重复）、守住 `AGENTS.md` 的硬规则与公开仓库红线、保持 CI 可用、M3 起负责发布。
5. `## 如何成为维护者`：持续贡献（例如合并过 5 个以上有分量的 PR，或长期参与评审与分诊）；现有维护者提名，管理员同意；在本文件加一行，并在 GitHub 授予 write 权限（之后可升为 maintain）。
6. `## 决策与合并`：日常改动由作者以外的一名维护者批准即可合并；只有一名维护者期间，管理员自己的 PR 在 CI 全绿后经绕过合并，并在 PR 里写明理由；squash 合并时把提交正文精简成英文摘要（删掉模板里的中文标题、注释和未勾选项）；高风险范围（同 `CONTRIBUTING.md` 第 3 节）在维护者达到两名后需要两名维护者批准，只有一名维护者时由管理员评审并在 PR 里写明理由；设计变更先改 `docs/superpowers/specs/` 并在 PR 里讨论；意见分歧时由管理员最终决定。
7. `## 分支保护`：如实写出本任务开头列出的规则集配置；说明「仓库管理员可以绕过」只用于两种情况：M0–M1 期间项目发起人直接提交开发进度（M1 收尾时停止，届时更新本节）；只有一名维护者期间合并管理员自己的 PR。
8. `## 发布`：M3 起由维护者推送 `vX.Y.Z` 标签触发 GitHub Actions 打包发布；更新包签名私钥只放在 GitHub Actions secrets，只有管理员能改。
9. `## 离任`：连续 6 个月不活跃的维护者转为荣誉维护者，收回写权限，随时可以回来。

- [ ] **Step 5: 写 `.github/` 下的文件（内容逐字照抄）**

`.github/CODEOWNERS`：

```text
# Code owners are requested for review automatically. High-risk paths are
# listed explicitly so they stay covered when more maintainers join; add new
# high-risk files here when they are created.
*                                  @sunkailun1992
/AGENTS.md                         @sunkailun1992
/scripts/check-secrets.sh          @sunkailun1992
/.github/                          @sunkailun1992
/crates/core/src/prompt.rs         @sunkailun1992
/crates/core/src/engine.rs         @sunkailun1992
/crates/core/src/executor/         @sunkailun1992
/crates/core/src/runner.rs         @sunkailun1992
/crates/core/src/linear/auth.rs    @sunkailun1992
/crates/core/src/ipc/protocol.rs   @sunkailun1992
/crates/cli/src/mcp.rs             @sunkailun1992
/crates/cli/tests/mcp_tools.rs     @sunkailun1992
```

`.github/pull_request_template.md`：

```markdown
## 改动说明 / Summary

<!-- 做了什么、为什么。PR 标题请用英文 Conventional Commits，例如 `fix(runner): kill the whole process tree on timeout`。 -->

## 关联 issue / Related issue

<!-- 例如 Closes #12；没有就写「无」 -->

## 风险等级 / Risk level

<!-- 见 docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md；勾一个 -->
- [ ] 低 / Low
- [ ] 中 / Medium
- [ ] 高 / High
- [ ] 极高 / Critical（执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权、IPC 方法表、标记完成）

## 验证 / Verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `bash scripts/check-secrets.sh --strict`（无输出）
- 本机测试过的平台 / Platforms tested locally: macOS / Windows / 仅 CI

## 实机验收 / Real-machine check

<!-- 涉及真实 Linear、Claude Code 或 Codex 行为时：写结果（并更新 docs/gates/），或写「待实机」及原因；不涉及就写「不涉及」 -->

## 自查 / Checklist

- [ ] 描述、截图、日志、夹具里没有令牌或凭据、公司名、内网地址、真实 Linear 内容、本机绝对路径
- [ ] 行为变化已同步文档
- [ ] 含 AI 生成的内容，我已逐行审过 / Contains AI-generated changes, reviewed line by line（不含就不勾）
```

`.github/ISSUE_TEMPLATE/bug_report.yml`：

```yaml
name: 问题报告 / Bug report
description: IssueRelay 的行为与预期不符 / Something does not work as expected
labels: ["bug"]
body:
  - type: markdown
    attributes:
      value: |
        本仓库公开。提交前请删除令牌、凭据、公司信息、内网地址、真实 Linear 需求内容和本机路径。安全漏洞请走 Security 页面的私密报告，不要在这里提交。
        This repository is public. Remove tokens, credentials, company information, internal addresses, real Linear content and local paths. Report security issues privately from the Security tab instead.
  - type: textarea
    id: what-happened
    attributes:
      label: 发生了什么 / What happened
    validations:
      required: true
  - type: textarea
    id: expected
    attributes:
      label: 期望的行为 / Expected behavior
    validations:
      required: true
  - type: textarea
    id: steps
    attributes:
      label: 复现步骤 / Steps to reproduce
      placeholder: "1. ...\n2. ...\n3. ..."
    validations:
      required: true
  - type: input
    id: version
    attributes:
      label: IssueRelay 版本或提交 / Version or commit
    validations:
      required: true
  - type: dropdown
    id: os
    attributes:
      label: 操作系统 / Operating system
      options:
        - macOS
        - Windows
        - 其他 / Other
    validations:
      required: true
  - type: dropdown
    id: executor
    attributes:
      label: 执行者 / Executor
      options:
        - Claude Code
        - Codex
        - 两者 / Both
        - 不涉及 / Not related
    validations:
      required: true
  - type: textarea
    id: logs
    attributes:
      label: 日志（已脱敏）/ Logs (redacted)
      render: text
  - type: checkboxes
    id: redaction
    attributes:
      label: 确认 / Confirmation
      options:
        - label: 我已删除令牌、凭据、公司信息、内网地址、真实 Linear 需求内容和本机路径 / I removed tokens, credentials, company information, internal addresses, real Linear content and local paths
          required: true
```

`.github/ISSUE_TEMPLATE/feature_request.yml`：

```yaml
name: 功能建议 / Feature request
description: 提出新功能或改进 / Suggest a new feature or an improvement
labels: ["enhancement"]
body:
  - type: markdown
    attributes:
      value: |
        大改动请先在这里讨论，维护者确认后再写代码。本仓库公开，请不要贴公司信息或真实 Linear 需求内容。
        Please discuss larger changes here before writing code. This repository is public: do not paste company information or real Linear content.
  - type: textarea
    id: problem
    attributes:
      label: 要解决的问题 / Problem
    validations:
      required: true
  - type: textarea
    id: proposal
    attributes:
      label: 建议的方案 / Proposal
    validations:
      required: true
  - type: textarea
    id: alternatives
    attributes:
      label: 考虑过的其他方案 / Alternatives considered
  - type: dropdown
    id: area
    attributes:
      label: 涉及范围 / Area
      options:
        - 引擎 / Engine
        - 命令行与 MCP / CLI and MCP
        - 面板与托盘 / Panel and tray
        - 安装与发布 / Packaging and release
        - 文档 / Docs
        - 其他 / Other
    validations:
      required: true
  - type: checkboxes
    id: redaction
    attributes:
      label: 确认 / Confirmation
      options:
        - label: 内容里没有公司信息、令牌或真实 Linear 需求内容 / No company information, tokens or real Linear content
          required: true
```

`.github/ISSUE_TEMPLATE/config.yml`：

```yaml
blank_issues_enabled: false
contact_links:
  - name: 安全漏洞 / Security vulnerability
    url: https://github.com/sunkailun1992/issuerelay/security/advisories/new
    about: 请私下报告，不要开公开 issue / Report privately, never in a public issue
  - name: 行为准则举报 / Code of conduct report
    url: https://github.com/sunkailun1992/issuerelay/security/advisories/new
    about: 私下提交，标题写 CoC report / Report privately with the title "CoC report"
```

`.github/dependabot.yml`：

```yaml
version: 2
updates:
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: weekly
    open-pull-requests-limit: 5
    commit-message:
      prefix: chore
      include: scope
    groups:
      cargo-minor-and-patch:
        update-types: [minor, patch]
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
    commit-message:
      prefix: ci
    groups:
      github-actions:
        patterns: ["*"]
```

- [ ] **Step 6: 改四份已有文档**

`README.md`：在「## 文档」一节之前插入：

```markdown
## 参与贡献 / Contributing

欢迎 issue 和 PR。动手前请先读 [CONTRIBUTING.md](CONTRIBUTING.md)。
Issues and pull requests are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

- [CONTRIBUTING.md](CONTRIBUTING.md)：开发环境、提交前检查、PR 流程 / setup, checks, pull requests
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)：行为准则 / code of conduct
- [SECURITY.md](SECURITY.md)：私下报告安全问题 / report vulnerabilities privately
- [MAINTAINERS.md](MAINTAINERS.md)：维护者、评审与合并规则 / maintainers and merge rules
```

`AGENTS.md`：在「## 多智能体协作」一节之前插入：

```markdown
## 贡献流程

- 外部贡献者走 fork + PR：替外部贡献者工作的 AI 只推送贡献者自己 fork 里的分支，不直接推本仓库的 `main`；PR 按 `.github/pull_request_template.md` 填写。维护者在 M1 收尾前直接提交的做法见 `MAINTAINERS.md`。
- 合并条件、评审人数与分支保护见 `MAINTAINERS.md`；开发环境与提交规范见 `CONTRIBUTING.md`。
- 新增或移动高风险文件（执行者参数、提示词、进程运行、凭据、IPC 协议）时，同步更新 `.github/CODEOWNERS`。
```

`docs/ai-coding/AI_AUTOMATION_WORKFLOW.md`：在文件末尾追加：

```markdown
## 七、外部贡献

1. 先确认 issue 已获维护者认可（大改动、高风险范围必须如此）。
2. 在贡献者自己的 fork 里从最新 `main` 切分支，按第一节的标准流程开发。
3. PR 标题用英文 Conventional Commits；描述按 PR 模板填写风险等级、验证结果和实机结论。
4. CI 三项（macOS、Windows、密钥扫描）全绿，评审对话全部解决，维护者批准后由维护者 squash 合并。
5. 合并后如果改了高风险文件的位置，检查 `.github/CODEOWNERS` 是否仍然覆盖。
```

`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`：
- 第 2 节表格中，在 `README.md`、`LICENSE` 一行后面加一行：`` | `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`MAINTAINERS.md` | 贡献指南、行为准则、安全政策、维护者与治理 | ``；同一表格里 `.github/` 一行的用途改为「CI、贡献模板与工具入口」。
- 第 6 节列表末尾加四行：
  - `` `.github/CODEOWNERS`：自动请求评审的负责人；高风险路径逐条列出。 ``
  - `` `.github/pull_request_template.md`、`.github/ISSUE_TEMPLATE/`：PR 与 issue 模板，带「已脱敏」确认项。 ``
  - `` `.github/dependabot.yml`：每周检查 cargo 与 GitHub Actions 依赖更新。 ``
  - `` 仓库设置（规则集、合并方式、安全功能）不在仓库文件里，现状记在 `MAINTAINERS.md` 的「分支保护」一节；改设置时同步改那一节。 ``

- [ ] **Step 7: 检查**

```bash
bash scripts/check-secrets.sh --strict; echo "exit=$?"
ruby -e 'require "yaml"; %w[.github/dependabot.yml .github/ISSUE_TEMPLATE/bug_report.yml .github/ISSUE_TEMPLATE/feature_request.yml .github/ISSUE_TEMPLATE/config.yml .github/workflows/ci.yml].each { |f| YAML.load_file(f) }; puts "yaml ok"'
python3 -I - <<'PY'
import re, pathlib
bad = []
for md in ["README.md", "AGENTS.md", "CONTRIBUTING.md", "CODE_OF_CONDUCT.md", "SECURITY.md", "MAINTAINERS.md",
           "docs/ai-coding/AI_AUTOMATION_WORKFLOW.md", "docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md"]:
    base = pathlib.Path(md).parent
    for target in re.findall(r"\]\(([^)#\s]+)", pathlib.Path(md).read_text(encoding="utf-8")):
        if not target.startswith("http") and not (base / target).exists():
            bad.append(f"{md} -> {target}")
print("links ok" if not bad else "\n".join(bad))
PY
```

Expected：密钥扫描无输出、`exit=0`；`yaml ok`；`links ok`。另外人工核对：四份说明文档都有 `## English summary`；`CONTRIBUTING.md` 的四条检查命令与 `AGENTS.md` 完全一致；`MAINTAINERS.md` 的分支保护描述与本任务开头的设置逐项一致；全文没有公司名、个人邮箱或本机路径。

- [ ] **Step 8: 提交**

```bash
git add CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md MAINTAINERS.md README.md AGENTS.md \
  .github/CODEOWNERS .github/pull_request_template.md .github/ISSUE_TEMPLATE .github/dependabot.yml \
  docs/ai-coding/AI_AUTOMATION_WORKFLOW.md docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md
bash scripts/check-secrets.sh --strict
git commit -m "docs: add contribution, conduct, security and maintainer guides

Co-Authored-By: Claude <实际模型名> <noreply@anthropic.com>"
```

评审通过后再推送；推送后用 `gh api repos/sunkailun1992/issuerelay/codeowners/errors` 确认 CODEOWNERS 没有语法错误。

---

### Task 7: core 路径与配置

> 执行说明（2026-10-12）：下面 `ipc_endpoint` 的代码是派发时的版本；Task 20 已改为只由数据目录决定端点（Windows 管道 `issuerelay-<哈希>`，Unix 套接字超过 103 字节时改放 `<临时目录>/issuerelay-<哈希>/`），并删掉了 `sanitize`。第 1 节表格里的 IPC 端点一行同理，以 `docs/ai-coding/AI_CODING_GUIDE.md` 为准。

**Files:**
- Create: `crates/core/src/paths.rs`、`crates/core/src/config.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces:
  - `paths::APP_ID: &str`；`paths::AppPaths { config_dir, data_dir }`，方法 `system() -> Result<AppPaths>`、`under(&Path) -> AppPaths`、`config_file()`、`state_db()`、`logs_dir()`、`ipc_endpoint() -> IpcEndpoint`
  - `paths::IpcEndpoint { Unix(PathBuf), Pipe(String) }`
  - `config::ExecutorKind { Claude, Codex }`（`as_str`、`program_name`、`parse`）；`config::PermissionProfile { Full, Auto, Personal }`（`parse`）
  - `config::TriggerNames { label, todo, in_progress, in_testing, done }`；`config::Settings`（字段见代码，`resolved_language()`、`resolved_comment_language()`）
  - `config::Binding { id, project_id, project_name, directory, executor, permission, model, prompt_override }`
  - `config::Config { settings, bindings }`：`load(&Path)`、`save(&Path)`、`binding_for_project(&str)`、`executor_for(&Binding)`、`permission_for(&Binding)`

- [ ] **Step 1: 加依赖**

```bash
cd crates/core
cargo add anyhow dirs sys-locale toml
cargo add serde --features derive
cargo add --dev tempfile
cd ../..
```

- [ ] **Step 2: 写失败的测试**

`crates/core/src/lib.rs` 改为：

```rust
//! IssueRelay engine: watches Linear issues assigned to the current user and hands each one
//! to a fresh headless Claude Code or Codex run.
pub mod config;
pub mod paths;
```

`crates/core/src/paths.rs` 先只放测试：

```rust
//! Where IssueRelay keeps its configuration, state, logs and IPC endpoint.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn under_builds_expected_files() {
        let root = Path::new("/tmp/issuerelay-test");
        let p = AppPaths::under(root);
        assert_eq!(p.config_file(), root.join("config").join("config.toml"));
        assert_eq!(p.state_db(), root.join("data").join("state.db"));
        assert_eq!(p.logs_dir(), root.join("data").join("logs"));
    }

    #[test]
    fn endpoint_is_stable_and_unique_per_data_dir() {
        let a = AppPaths::under(Path::new("/tmp/a"));
        let b = AppPaths::under(Path::new("/tmp/b"));
        assert_eq!(a.ipc_endpoint(), a.ipc_endpoint());
        assert_ne!(a.ipc_endpoint(), b.ipc_endpoint());
    }

    #[cfg(unix)]
    #[test]
    fn long_socket_path_falls_back_to_temp_dir() {
        let long = format!("/tmp/{}", "d".repeat(120));
        match AppPaths::under(Path::new(&long)).ipc_endpoint() {
            IpcEndpoint::Unix(path) => {
                assert!(path.starts_with(std::env::temp_dir()));
                assert!(path.as_os_str().len() < 104 + 40);
            }
            other => panic!("expected a unix socket, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_a_named_pipe() {
        match AppPaths::under(Path::new(r"C:\issuerelay-test")).ipc_endpoint() {
            IpcEndpoint::Pipe(name) => assert!(name.starts_with(r"\\.\pipe\issuerelay-")),
            other => panic!("expected a named pipe, got {other:?}"),
        }
    }
}
```

`crates/core/src/config.rs` 先只放测试：

```rust
//! User settings and Linear-project bindings, stored as TOML.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn binding(project: &str, executor: Option<ExecutorKind>) -> Binding {
        Binding {
            id: project.into(),
            project_id: project.into(),
            project_name: "Demo Project".into(),
            directory: PathBuf::from("demo"),
            executor,
            permission: None,
            model: None,
            prompt_override: None,
        }
    }

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(s.language, "system");
        assert_eq!(s.poll_interval_secs, 30);
        assert_eq!(s.max_concurrent, 2);
        assert_eq!(s.run_timeout_mins, 60);
        assert_eq!(s.stall_timeout_mins, 15);
        assert_eq!(s.default_executor, ExecutorKind::Claude);
        assert_eq!(s.default_permission, PermissionProfile::Full);
        assert_eq!(s.comment_language, "follow-ui");
        assert!(!s.paused);
        assert_eq!(s.trigger.label, "ai-ready");
        assert_eq!(s.trigger.todo, "Todo");
        assert_eq!(s.trigger.in_progress, "In Progress");
        assert_eq!(s.trigger.in_testing, "In Testing");
        assert_eq!(s.trigger.done, "Done");
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(&dir.path().join("none.toml")).unwrap(), Config::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let mut cfg = Config::default();
        cfg.settings.max_concurrent = 3;
        cfg.bindings.push(binding("proj-1", Some(ExecutorKind::Codex)));
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);
        cfg.settings.paused = true;
        cfg.save(&path).unwrap();
        assert!(Config::load(&path).unwrap().settings.paused);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[settings]\npoll_interval_secs = 10\n").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.settings.poll_interval_secs, 10);
        assert_eq!(cfg.settings.max_concurrent, 2);
        assert!(cfg.bindings.is_empty());
    }

    #[test]
    fn invalid_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "settings = 5\n").unwrap();
        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn binding_lookup_and_effective_choices() {
        let mut cfg = Config::default();
        cfg.bindings.push(binding("proj-1", None));
        cfg.bindings.push(Binding {
            permission: Some(PermissionProfile::Auto),
            ..binding("proj-2", Some(ExecutorKind::Codex))
        });
        let b1 = cfg.binding_for_project("proj-1").unwrap();
        assert_eq!(cfg.executor_for(b1), ExecutorKind::Claude);
        assert_eq!(cfg.permission_for(b1), PermissionProfile::Full);
        let b2 = cfg.binding_for_project("proj-2").unwrap();
        assert_eq!(cfg.executor_for(b2), ExecutorKind::Codex);
        assert_eq!(cfg.permission_for(b2), PermissionProfile::Auto);
        assert!(cfg.binding_for_project("proj-3").is_none());
    }

    #[test]
    fn comment_language_resolution() {
        let mut s = Settings { language: "ja".into(), ..Settings::default() };
        assert_eq!(s.resolved_comment_language(), "ja");
        s.comment_language = "en".into();
        assert_eq!(s.resolved_comment_language(), "en");
    }

    #[test]
    fn names_parse_case_insensitively() {
        assert_eq!(ExecutorKind::parse(" Codex "), Some(ExecutorKind::Codex));
        assert_eq!(ExecutorKind::parse("claude"), Some(ExecutorKind::Claude));
        assert_eq!(ExecutorKind::parse("gpt"), None);
        assert_eq!(PermissionProfile::parse("AUTO"), Some(PermissionProfile::Auto));
        assert_eq!(PermissionProfile::parse("personal"), Some(PermissionProfile::Personal));
    }
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core`
Expected: 编译失败，报 `AppPaths`、`Settings`、`Config` 等未定义

- [ ] **Step 4: 实现 paths.rs**

在 `crates/core/src/paths.rs` 的测试模块之前加入：

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const APP_ID: &str = "com.sunkailun.issuerelay";
/// macOS limits Unix socket paths to 104 bytes; stay well below it.
const UNIX_SOCKET_MAX: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    Unix(PathBuf),
    Pipe(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl AppPaths {
    pub fn system() -> Result<Self> {
        let config = dirs::config_dir().context("this system has no config directory")?;
        let data = dirs::data_local_dir().context("this system has no local data directory")?;
        Ok(Self { config_dir: config.join(APP_ID), data_dir: data.join(APP_ID) })
    }

    pub fn under(root: &Path) -> Self {
        Self { config_dir: root.join("config"), data_dir: root.join("data") }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn state_db(&self) -> PathBuf {
        self.data_dir.join("state.db")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn ipc_endpoint(&self) -> IpcEndpoint {
        let user = sanitize(
            &std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_default(),
        );
        let tag = short_hash(&self.data_dir.to_string_lossy());
        if cfg!(windows) {
            return IpcEndpoint::Pipe(format!(r"\\.\pipe\issuerelay-{user}-{tag}"));
        }
        let preferred = self.data_dir.join("issuerelay.sock");
        if preferred.as_os_str().len() <= UNIX_SOCKET_MAX {
            IpcEndpoint::Unix(preferred)
        } else {
            IpcEndpoint::Unix(
                std::env::temp_dir()
                    .join(format!("issuerelay-{user}-{tag}"))
                    .join("issuerelay.sock"),
            )
        }
    }
}

fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    if cleaned.is_empty() { "user".into() } else { cleaned }
}

/// FNV-1a; only needs to be stable and spread out, not cryptographic.
fn short_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}
```

- [ ] **Step 5: 实现 config.rs**

在 `crates/core/src/config.rs` 的测试模块之前加入：

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorKind {
    Claude,
    Codex,
}

impl ExecutorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutorKind::Claude => "claude",
            ExecutorKind::Codex => "codex",
        }
    }

    pub fn program_name(self) -> &'static str {
        self.as_str()
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(ExecutorKind::Claude),
            "codex" => Some(ExecutorKind::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionProfile {
    /// Skip every permission check (spec default).
    Full,
    /// Let the executor's own automatic reviewer decide.
    Auto,
    /// Use only what the user already allowed in their own settings.
    Personal,
}

impl PermissionProfile {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "full" => Some(PermissionProfile::Full),
            "auto" => Some(PermissionProfile::Auto),
            "personal" => Some(PermissionProfile::Personal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggerNames {
    pub label: String,
    pub todo: String,
    pub in_progress: String,
    pub in_testing: String,
    pub done: String,
}

impl Default for TriggerNames {
    fn default() -> Self {
        Self {
            label: "ai-ready".into(),
            todo: "Todo".into(),
            in_progress: "In Progress".into(),
            in_testing: "In Testing".into(),
            done: "Done".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system" or a BCP 47 tag such as "zh-CN".
    pub language: String,
    pub poll_interval_secs: u64,
    pub max_concurrent: usize,
    pub run_timeout_mins: u64,
    pub stall_timeout_mins: u64,
    pub default_executor: ExecutorKind,
    pub default_permission: PermissionProfile,
    pub prompt_template: String,
    /// "follow-ui" or a language tag for comments the AI writes in Linear.
    pub comment_language: String,
    pub paused: bool,
    pub trigger: TriggerNames,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            poll_interval_secs: 30,
            max_concurrent: 2,
            run_timeout_mins: 60,
            stall_timeout_mins: 15,
            default_executor: ExecutorKind::Claude,
            default_permission: PermissionProfile::Full,
            prompt_template: String::new(),
            comment_language: "follow-ui".into(),
            paused: false,
            trigger: TriggerNames::default(),
        }
    }
}

impl Settings {
    pub fn resolved_language(&self) -> String {
        if self.language == "system" {
            sys_locale::get_locale().unwrap_or_else(|| "en-US".into())
        } else {
            self.language.clone()
        }
    }

    pub fn resolved_comment_language(&self) -> String {
        if self.comment_language == "follow-ui" {
            self.resolved_language()
        } else {
            self.comment_language.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub directory: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<ExecutorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_override: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub settings: Settings,
    pub bindings: Vec<Binding>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("invalid config file {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// Writes to a temporary file first, then renames it over the old one.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    pub fn binding_for_project(&self, project_id: &str) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.project_id == project_id)
    }

    pub fn executor_for(&self, binding: &Binding) -> ExecutorKind {
        binding.executor.unwrap_or(self.settings.default_executor)
    }

    pub fn permission_for(&self, binding: &Binding) -> PermissionProfile {
        binding.permission.unwrap_or(self.settings.default_permission)
    }
}
```

- [ ] **Step 6: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core`
Expected: PASS（paths 3 个、config 8 个；Windows 上 paths 换成命名管道那个测试）

- [ ] **Step 7: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add app paths and TOML config with bindings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: core 数据模型、触发与防重复规则

**Files:**
- Create: `crates/core/src/model.rs`、`crates/core/src/trigger.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 7 的 `Config`、`Binding`、`ExecutorKind`、`TriggerNames`
- Produces:
  - `model::Issue { uuid, identifier, title, url, state, labels, project_id, project_name, assignee_id, parent_identifier, priority: u8, created_at, updated_at }`
  - `model::JobState { WaitingBinding, Queued, Running, AwaitingAcceptance, NeedsInfo, Done, Failed, Canceled }`（`as_str`、`parse`、`is_active`、`ALL`）
  - `model::Job { id, issue_uuid, issue_identifier, title, binding_id, directory, executor, state, reason, priority, issue_created_at, discovered_updated_at, verified_updated_at, created_at, started_at, finished_at, log_path, summary }`
  - `trigger::same_name(&str, &str) -> bool`；`trigger::is_candidate(&Issue, &TriggerNames, me: &str) -> bool`
  - `trigger::WaitReason { ProjectNotBound, DirectoryMissing(PathBuf), ExecutorNotReady(ExecutorKind) }`（`describe()`）；`trigger::Eligibility { NotCandidate, Waiting(WaitReason), Eligible(Binding) }`
  - `trigger::evaluate(&Issue, &Config, me: &str, ready: &HashSet<ExecutorKind>) -> Eligibility`
  - `trigger::should_enqueue(latest: Option<&Job>, &Issue) -> bool`

- [ ] **Step 1: 加依赖**

```bash
cd crates/core && cargo add chrono --features serde && cd ../..
```

- [ ] **Step 2: 写数据模型**

`crates/core/src/model.rs`：

```rust
//! Data types shared by the engine, the store and the IPC layer.
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::ExecutorKind;

/// The subset of a Linear issue IssueRelay needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub uuid: String,
    /// Human identifier such as `DEMO-1`.
    pub identifier: String,
    pub title: String,
    pub url: String,
    /// Status name, e.g. `Todo`.
    pub state: String,
    pub labels: Vec<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub assignee_id: Option<String>,
    pub parent_identifier: Option<String>,
    /// Linear priority: 0 none, 1 urgent … 4 low.
    pub priority: u8,
    pub created_at: DateTime<Utc>,
    /// Linear's own clock; the only timestamp used to decide re-queuing.
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    WaitingBinding,
    Queued,
    Running,
    AwaitingAcceptance,
    NeedsInfo,
    Done,
    Failed,
    Canceled,
}

impl JobState {
    pub const ALL: [JobState; 8] = [
        JobState::WaitingBinding,
        JobState::Queued,
        JobState::Running,
        JobState::AwaitingAcceptance,
        JobState::NeedsInfo,
        JobState::Done,
        JobState::Failed,
        JobState::Canceled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            JobState::WaitingBinding => "waiting_binding",
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::AwaitingAcceptance => "awaiting_acceptance",
            JobState::NeedsInfo => "needs_info",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Canceled => "canceled",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    /// Waiting, queued or running: the issue is still being tracked.
    pub fn is_active(self) -> bool {
        matches!(self, JobState::WaitingBinding | JobState::Queued | JobState::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub issue_uuid: String,
    pub issue_identifier: String,
    pub title: String,
    pub binding_id: Option<String>,
    pub directory: Option<PathBuf>,
    pub executor: Option<ExecutorKind>,
    pub state: JobState,
    pub reason: Option<String>,
    pub priority: u8,
    pub issue_created_at: DateTime<Utc>,
    pub discovered_updated_at: DateTime<Utc>,
    pub verified_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub log_path: Option<PathBuf>,
    pub summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_state_names_round_trip() {
        for state in JobState::ALL {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
        }
        assert_eq!(JobState::parse("nope"), None);
    }

    #[test]
    fn only_waiting_queued_running_are_active() {
        let active: Vec<_> = JobState::ALL.into_iter().filter(|s| s.is_active()).collect();
        assert_eq!(active, vec![JobState::WaitingBinding, JobState::Queued, JobState::Running]);
    }
}
```

- [ ] **Step 3: 写触发规则的失败测试**

`crates/core/src/lib.rs` 加上 `pub mod model;` 和 `pub mod trigger;`。`crates/core/src/trigger.rs` 先写测试：

```rust
//! Which issues IssueRelay should pick up, and when an issue it already handled comes back.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Binding, Config, ExecutorKind};
    use crate::model::{Issue, Job, JobState};
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use std::collections::HashSet;
    use std::path::Path;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap()
    }

    fn issue(project: Option<&str>) -> Issue {
        Issue {
            uuid: "u-1".into(),
            identifier: "DEMO-1".into(),
            title: "Demo".into(),
            url: "https://linear.app/demo/issue/DEMO-1".into(),
            state: "Todo".into(),
            labels: vec!["ai-ready".into()],
            project_id: project.map(str::to_string),
            project_name: Some("Demo Project".into()),
            assignee_id: Some("me".into()),
            parent_identifier: None,
            priority: 0,
            created_at: t0(),
            updated_at: t0(),
        }
    }

    fn config_with(project: &str, dir: &Path, executor: Option<ExecutorKind>) -> Config {
        let mut cfg = Config::default();
        cfg.bindings.push(Binding {
            id: project.into(),
            project_id: project.into(),
            project_name: "Demo Project".into(),
            directory: dir.to_path_buf(),
            executor,
            permission: None,
            model: None,
            prompt_override: None,
        });
        cfg
    }

    fn job(state: JobState, verified: Option<DateTime<Utc>>) -> Job {
        Job {
            id: 1,
            issue_uuid: "u-1".into(),
            issue_identifier: "DEMO-1".into(),
            title: "Demo".into(),
            binding_id: None,
            directory: None,
            executor: None,
            state,
            reason: None,
            priority: 0,
            issue_created_at: t0(),
            discovered_updated_at: t0(),
            verified_updated_at: verified,
            created_at: t0(),
            started_at: None,
            finished_at: None,
            log_path: None,
            summary: None,
        }
    }

    fn all_ready() -> HashSet<ExecutorKind> {
        [ExecutorKind::Claude, ExecutorKind::Codex].into_iter().collect()
    }

    #[test]
    fn candidate_needs_assignee_label_and_todo() {
        let names = Config::default().settings.trigger;
        assert!(is_candidate(&issue(None), &names, "me"));
        assert!(!is_candidate(&issue(None), &names, "someone-else"));
        let mut other_label = issue(None);
        other_label.labels = vec!["bug".into()];
        assert!(!is_candidate(&other_label, &names, "me"));
        let mut in_progress = issue(None);
        in_progress.state = "In Progress".into();
        assert!(!is_candidate(&in_progress, &names, "me"));
    }

    #[test]
    fn names_compare_case_insensitively() {
        let names = Config::default().settings.trigger;
        let mut i = issue(None);
        i.labels = vec![" AI-Ready ".into()];
        i.state = "todo".into();
        assert!(is_candidate(&i, &names, "me"));
    }

    #[test]
    fn unbound_project_waits() {
        let cfg = Config::default();
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready()),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
        assert_eq!(
            evaluate(&issue(None), &cfg, "me", &all_ready()),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
    }

    #[test]
    fn missing_directory_waits() {
        let cfg = config_with("proj-1", Path::new("/definitely/not/here"), None);
        assert!(matches!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready()),
            Eligibility::Waiting(WaitReason::DirectoryMissing(_))
        ));
    }

    #[test]
    fn executor_not_ready_waits() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), Some(ExecutorKind::Codex));
        let only_claude: HashSet<_> = [ExecutorKind::Claude].into_iter().collect();
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &only_claude),
            Eligibility::Waiting(WaitReason::ExecutorNotReady(ExecutorKind::Codex))
        );
    }

    #[test]
    fn bound_ready_issue_is_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        match evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready()) {
            Eligibility::Eligible(binding) => assert_eq!(binding.project_id, "proj-1"),
            other => panic!("expected eligible, got {other:?}"),
        }
    }

    #[test]
    fn non_candidate_is_not_evaluated_further() {
        let cfg = Config::default();
        assert_eq!(evaluate(&issue(None), &cfg, "other", &all_ready()), Eligibility::NotCandidate);
    }

    #[test]
    fn enqueue_new_issue_but_not_tracked_one() {
        assert!(should_enqueue(None, &issue(None)));
        for state in [JobState::WaitingBinding, JobState::Queued, JobState::Running] {
            assert!(!should_enqueue(Some(&job(state, None)), &issue(None)));
        }
    }

    #[test]
    fn finished_issue_requeues_only_after_linear_update() {
        let finished = job(JobState::Failed, Some(t0()));
        assert!(!should_enqueue(Some(&finished), &issue(None)));
        let mut touched = issue(None);
        touched.updated_at = t0() + Duration::seconds(1);
        assert!(should_enqueue(Some(&finished), &touched));
    }

    #[test]
    fn should_enqueue_compares_linear_clock_only() {
        // Local timestamps far in the future must not matter; only Linear's updatedAt does.
        let far_future = Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap();
        let mut finished = job(JobState::NeedsInfo, Some(t0()));
        finished.created_at = far_future;
        finished.finished_at = Some(far_future);
        let mut touched = issue(None);
        touched.updated_at = t0() + Duration::seconds(1);
        assert!(should_enqueue(Some(&finished), &touched));
        assert!(!should_enqueue(Some(&finished), &issue(None)));
    }

    #[test]
    fn falls_back_to_discovered_time_when_never_verified() {
        let canceled = job(JobState::Canceled, None);
        assert!(!should_enqueue(Some(&canceled), &issue(None)));
        let mut touched = issue(None);
        touched.updated_at = t0() + Duration::seconds(5);
        assert!(should_enqueue(Some(&canceled), &touched));
    }
}
```

- [ ] **Step 4: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core trigger`
Expected: 编译失败，报 `is_candidate`、`evaluate`、`should_enqueue` 等未定义

- [ ] **Step 5: 实现触发规则**

在 `crates/core/src/trigger.rs` 的测试模块之前加入：

```rust
use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::{Binding, Config, ExecutorKind, TriggerNames};
use crate::model::{Issue, Job};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    ProjectNotBound,
    DirectoryMissing(PathBuf),
    ExecutorNotReady(ExecutorKind),
}

impl WaitReason {
    pub fn describe(&self) -> String {
        match self {
            WaitReason::ProjectNotBound => "project is not bound to a local directory".into(),
            WaitReason::DirectoryMissing(dir) => {
                format!("bound directory does not exist: {}", dir.display())
            }
            WaitReason::ExecutorNotReady(kind) => format!("executor {} is not ready", kind.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    NotCandidate,
    Waiting(WaitReason),
    Eligible(Binding),
}

/// Linear names are compared ignoring surrounding whitespace and ASCII case.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

pub fn is_candidate(issue: &Issue, names: &TriggerNames, me: &str) -> bool {
    issue.assignee_id.as_deref() == Some(me)
        && issue.labels.iter().any(|label| same_name(label, &names.label))
        && same_name(&issue.state, &names.todo)
}

pub fn evaluate(issue: &Issue, cfg: &Config, me: &str, ready: &HashSet<ExecutorKind>) -> Eligibility {
    if !is_candidate(issue, &cfg.settings.trigger, me) {
        return Eligibility::NotCandidate;
    }
    let Some(binding) = issue.project_id.as_deref().and_then(|p| cfg.binding_for_project(p)) else {
        return Eligibility::Waiting(WaitReason::ProjectNotBound);
    };
    if !binding.directory.is_dir() {
        return Eligibility::Waiting(WaitReason::DirectoryMissing(binding.directory.clone()));
    }
    let kind = cfg.executor_for(binding);
    if !ready.contains(&kind) {
        return Eligibility::Waiting(WaitReason::ExecutorNotReady(kind));
    }
    Eligibility::Eligible(binding.clone())
}

/// A tracked issue (waiting, queued, running) is never enqueued again. A finished one comes
/// back only when Linear's own `updatedAt` moved past the value recorded when the last run
/// was verified, so clock skew on this machine cannot cause duplicates or misses.
pub fn should_enqueue(latest: Option<&Job>, issue: &Issue) -> bool {
    match latest {
        None => true,
        Some(job) if job.state.is_active() => false,
        Some(job) => {
            let seen = job.verified_updated_at.unwrap_or(job.discovered_updated_at);
            issue.updated_at > seen
        }
    }
}
```

- [ ] **Step 6: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core`
Expected: PASS（model 2 个、trigger 11 个，加上之前的）

- [ ] **Step 7: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add issue/job model with trigger and requeue rules

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: core SQLite 运行记录

> 执行记录（2026-10-11）：已完成。评审中修订了 `mark_started`（只动排队中的记录，返回是否改了）、`mark_finished`（`None` 保留原核对值）、时间精度（AutoSi）、数据库版本号，新增 `set_state_if`，并补齐测试；下文实现块是原计划文本，以仓库为准。

**Files:**
- Create: `crates/core/src/store.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 7 的 `Binding`、`ExecutorKind`；Task 8 的 `Issue`、`Job`、`JobState`
- Produces:
  - `store::NewJob<'a> { issue: &'a Issue, binding: Option<&'a Binding>, executor: Option<ExecutorKind>, state: JobState, reason: Option<String> }`
  - `store::Store`：`open(&Path)`、`open_in_memory()`、`insert(NewJob) -> Result<Job>`、`get(i64) -> Result<Option<Job>>`、`latest_for_issue(&str) -> Result<Option<Job>>`、`list_by_state(&[JobState]) -> Result<Vec<Job>>`、`list_recent(usize) -> Result<Vec<Job>>`、`assign(id, state, Option<&Binding>, Option<ExecutorKind>, Option<&str>)`、`set_state(id, state, Option<&str>)`、`mark_started(id, &Path) -> Result<bool>`（只把排队中的记录改为运行中，返回是否改了）、`mark_finished(id, state, reason: Option<&str>, summary: Option<&str>, verified_updated_at: Option<DateTime<Utc>>)`（`None` 保留原有核对值）、`set_state_if(id, expected: &[JobState], state, reason) -> Result<bool>`（只在当前状态属于 `expected` 时改，返回是否改了）、`requeue(id)`、`recover_interrupted() -> Result<usize>`

- [ ] **Step 1: 加依赖**

```bash
cd crates/core && cargo add rusqlite --features bundled && cd ../..
```

- [ ] **Step 2: 写失败的测试**

`crates/core/src/lib.rs` 加 `pub mod store;`。`crates/core/src/store.rs` 先写测试：

```rust
//! SQLite-backed run records.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Binding, ExecutorKind};
    use crate::model::{Issue, JobState};
    use chrono::{Duration, TimeZone, Utc};
    use std::path::{Path, PathBuf};

    fn issue(uuid: &str) -> Issue {
        let t = Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap();
        Issue {
            uuid: uuid.into(),
            identifier: format!("DEMO-{uuid}"),
            title: "Demo 需求".into(),
            url: "https://linear.app/demo/issue/DEMO-1".into(),
            state: "Todo".into(),
            labels: vec!["ai-ready".into()],
            project_id: Some("proj-1".into()),
            project_name: Some("Demo Project".into()),
            assignee_id: Some("me".into()),
            parent_identifier: None,
            priority: 2,
            created_at: t,
            updated_at: t + Duration::milliseconds(321),
        }
    }

    fn binding() -> Binding {
        Binding {
            id: "proj-1".into(),
            project_id: "proj-1".into(),
            project_name: "Demo Project".into(),
            directory: PathBuf::from("demo dir"),
            executor: None,
            permission: None,
            model: None,
            prompt_override: None,
        }
    }

    fn queued(store: &Store, uuid: &str) -> Job {
        let i = issue(uuid);
        let b = binding();
        store
            .insert(NewJob {
                issue: &i,
                binding: Some(&b),
                executor: Some(ExecutorKind::Claude),
                state: JobState::Queued,
                reason: None,
            })
            .unwrap()
    }

    #[test]
    fn insert_and_get_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        let loaded = store.get(job.id).unwrap().unwrap();
        assert_eq!(loaded, job);
        assert_eq!(loaded.issue_identifier, "DEMO-1");
        assert_eq!(loaded.directory, Some(PathBuf::from("demo dir")));
        assert_eq!(loaded.executor, Some(ExecutorKind::Claude));
        assert_eq!(loaded.priority, 2);
        assert_eq!(loaded.discovered_updated_at, issue("1").updated_at);
        assert!(store.get(999).unwrap().is_none());
    }

    #[test]
    fn latest_for_issue_returns_newest() {
        let store = Store::open_in_memory().unwrap();
        let first = queued(&store, "1");
        store.set_state(first.id, JobState::Failed, Some("boom")).unwrap();
        let second = queued(&store, "1");
        queued(&store, "2");
        assert_eq!(store.latest_for_issue("1").unwrap().unwrap().id, second.id);
        assert!(store.latest_for_issue("3").unwrap().is_none());
    }

    #[test]
    fn list_by_state_filters() {
        let store = Store::open_in_memory().unwrap();
        let a = queued(&store, "1");
        let b = queued(&store, "2");
        store.set_state(b.id, JobState::WaitingBinding, Some("unbound")).unwrap();
        let only_queued = store.list_by_state(&[JobState::Queued]).unwrap();
        assert_eq!(only_queued.iter().map(|j| j.id).collect::<Vec<_>>(), vec![a.id]);
        assert_eq!(store.list_by_state(&[JobState::Queued, JobState::WaitingBinding]).unwrap().len(), 2);
        assert!(store.list_by_state(&[]).unwrap().is_empty());
    }

    #[test]
    fn lifecycle_updates() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        store.mark_started(job.id, Path::new("logs/1.jsonl")).unwrap();
        let running = store.get(job.id).unwrap().unwrap();
        assert_eq!(running.state, JobState::Running);
        assert!(running.started_at.is_some());
        assert_eq!(running.log_path, Some(PathBuf::from("logs/1.jsonl")));
        let verified = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        store
            .mark_finished(job.id, JobState::AwaitingAcceptance, None, Some("pushed to test"), Some(verified))
            .unwrap();
        let done = store.get(job.id).unwrap().unwrap();
        assert_eq!(done.state, JobState::AwaitingAcceptance);
        assert_eq!(done.summary.as_deref(), Some("pushed to test"));
        assert_eq!(done.verified_updated_at, Some(verified));
        assert!(done.finished_at.is_some());
        store.requeue(job.id).unwrap();
        let again = store.get(job.id).unwrap().unwrap();
        assert_eq!(again.state, JobState::Queued);
        assert!(again.started_at.is_none() && again.finished_at.is_none() && again.summary.is_none());
    }

    #[test]
    fn assign_changes_binding_and_state() {
        let store = Store::open_in_memory().unwrap();
        let i = issue("1");
        let waiting = store
            .insert(NewJob { issue: &i, binding: None, executor: None, state: JobState::WaitingBinding, reason: Some("unbound".into()) })
            .unwrap();
        assert_eq!(waiting.directory, None);
        let b = binding();
        store.assign(waiting.id, JobState::Queued, Some(&b), Some(ExecutorKind::Codex), None).unwrap();
        let now = store.get(waiting.id).unwrap().unwrap();
        assert_eq!(now.state, JobState::Queued);
        assert_eq!(now.binding_id.as_deref(), Some("proj-1"));
        assert_eq!(now.executor, Some(ExecutorKind::Codex));
        assert_eq!(now.reason, None);
    }

    #[test]
    fn recover_marks_running_as_failed() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        store.mark_started(job.id, Path::new("x.jsonl")).unwrap();
        queued(&store, "2");
        assert_eq!(store.recover_interrupted().unwrap(), 1);
        let recovered = store.get(job.id).unwrap().unwrap();
        assert_eq!(recovered.state, JobState::Failed);
        assert!(recovered.reason.unwrap().contains("interrupted"));
        assert_eq!(store.recover_interrupted().unwrap(), 0);
    }

    #[test]
    fn file_store_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.db");
        let id = {
            let store = Store::open(&path).unwrap();
            queued(&store, "1").id
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().state, JobState::Queued);
        assert_eq!(store.list_recent(10).unwrap().len(), 1);
    }
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core store`
Expected: 编译失败，报 `Store`、`NewJob` 未定义

- [ ] **Step 4: 实现**

在 `crates/core/src/store.rs` 的测试模块之前加入：

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Type};

use crate::config::{Binding, ExecutorKind};
use crate::model::{Issue, Job, JobState};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_uuid TEXT NOT NULL,
    issue_identifier TEXT NOT NULL,
    title TEXT NOT NULL,
    binding_id TEXT,
    directory TEXT,
    executor TEXT,
    state TEXT NOT NULL,
    reason TEXT,
    priority INTEGER NOT NULL,
    issue_created_at TEXT NOT NULL,
    discovered_updated_at TEXT NOT NULL,
    verified_updated_at TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    log_path TEXT,
    summary TEXT
);
CREATE INDEX IF NOT EXISTS jobs_by_issue ON jobs (issue_uuid, id);
CREATE INDEX IF NOT EXISTS jobs_by_state ON jobs (state);
";

const COLUMNS: &str = "id, issue_uuid, issue_identifier, title, binding_id, directory, executor, \
    state, reason, priority, issue_created_at, discovered_updated_at, verified_updated_at, \
    created_at, started_at, finished_at, log_path, summary";

pub struct NewJob<'a> {
    pub issue: &'a Issue,
    pub binding: Option<&'a Binding>,
    pub executor: Option<ExecutorKind>,
    pub state: JobState,
    pub reason: Option<String>,
}

pub struct Store {
    conn: Connection,
}

fn fmt(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn conversion(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, message.into())
}

fn ts(column: usize, text: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| conversion(column, e.to_string()))
}

fn opt_ts(column: usize, text: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    text.map(|t| ts(column, t)).transpose()
}

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    let state: String = row.get(7)?;
    let executor: Option<String> = row.get(6)?;
    let priority: i64 = row.get(9)?;
    Ok(Job {
        id: row.get(0)?,
        issue_uuid: row.get(1)?,
        issue_identifier: row.get(2)?,
        title: row.get(3)?,
        binding_id: row.get(4)?,
        directory: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
        executor: executor.as_deref().and_then(ExecutorKind::parse),
        state: JobState::parse(&state).ok_or_else(|| conversion(7, format!("unknown state {state}")))?,
        reason: row.get(8)?,
        priority: priority.clamp(0, 4) as u8,
        issue_created_at: ts(10, row.get(10)?)?,
        discovered_updated_at: ts(11, row.get(11)?)?,
        verified_updated_at: opt_ts(12, row.get(12)?)?,
        created_at: ts(13, row.get(13)?)?,
        started_at: opt_ts(14, row.get(14)?)?,
        finished_at: opt_ts(15, row.get(15)?)?,
        log_path: row.get::<_, Option<String>>(16)?.map(PathBuf::from),
        summary: row.get(17)?,
    })
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert(&self, new: NewJob<'_>) -> Result<Job> {
        let issue = new.issue;
        self.conn.execute(
            "INSERT INTO jobs (issue_uuid, issue_identifier, title, binding_id, directory, executor, \
             state, reason, priority, issue_created_at, discovered_updated_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                issue.uuid,
                issue.identifier,
                issue.title,
                new.binding.map(|b| b.id.clone()),
                new.binding.map(|b| b.directory.to_string_lossy().into_owned()),
                new.executor.map(|e| e.as_str()),
                new.state.as_str(),
                new.reason,
                i64::from(issue.priority),
                fmt(issue.created_at),
                fmt(issue.updated_at),
                fmt(Utc::now()),
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get(id)?.context("inserted job disappeared")
    }

    pub fn get(&self, id: i64) -> Result<Option<Job>> {
        let sql = format!("SELECT {COLUMNS} FROM jobs WHERE id = ?1");
        Ok(self.conn.query_row(&sql, params![id], row_to_job).optional()?)
    }

    pub fn latest_for_issue(&self, uuid: &str) -> Result<Option<Job>> {
        let sql = format!("SELECT {COLUMNS} FROM jobs WHERE issue_uuid = ?1 ORDER BY id DESC LIMIT 1");
        Ok(self.conn.query_row(&sql, params![uuid], row_to_job).optional()?)
    }

    pub fn list_by_state(&self, states: &[JobState]) -> Result<Vec<Job>> {
        if states.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=states.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let sql = format!("SELECT {COLUMNS} FROM jobs WHERE state IN ({placeholders}) ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(states.iter().map(|s| s.as_str())), row_to_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<Job>> {
        let sql = format!("SELECT {COLUMNS} FROM jobs ORDER BY id DESC LIMIT ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], row_to_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn assign(
        &self,
        id: i64,
        state: JobState,
        binding: Option<&Binding>,
        executor: Option<ExecutorKind>,
        reason: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, binding_id = ?3, directory = ?4, executor = ?5, reason = ?6 WHERE id = ?1",
            params![
                id,
                state.as_str(),
                binding.map(|b| b.id.clone()),
                binding.map(|b| b.directory.to_string_lossy().into_owned()),
                executor.map(|e| e.as_str()),
                reason,
            ],
        )?;
        Ok(())
    }

    pub fn set_state(&self, id: i64, state: JobState, reason: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = ?3 WHERE id = ?1",
            params![id, state.as_str(), reason],
        )?;
        Ok(())
    }

    pub fn mark_started(&self, id: i64, log_path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = 'running', reason = NULL, started_at = ?2, finished_at = NULL, log_path = ?3 WHERE id = ?1",
            params![id, fmt(Utc::now()), log_path.to_string_lossy().into_owned()],
        )?;
        Ok(())
    }

    pub fn mark_finished(
        &self,
        id: i64,
        state: JobState,
        reason: Option<&str>,
        summary: Option<&str>,
        verified_updated_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = ?3, summary = ?4, verified_updated_at = ?5, finished_at = ?6 WHERE id = ?1",
            params![id, state.as_str(), reason, summary, verified_updated_at.map(fmt), fmt(Utc::now())],
        )?;
        Ok(())
    }

    pub fn requeue(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = 'queued', reason = NULL, summary = NULL, started_at = NULL, finished_at = NULL WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Runs left in `running` belong to a previous process that stopped; they are not retried.
    pub fn recover_interrupted(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE jobs SET state = 'failed', reason = 'interrupted: IssueRelay stopped while the run was in progress', finished_at = ?1 WHERE state = 'running'",
            params![fmt(Utc::now())],
        )?)
    }
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core store`
Expected: PASS（7 个）

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add SQLite job store

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: core 调度

**Files:**
- Create: `crates/core/src/scheduler.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: 无（只用 `chrono`）
- Produces: `scheduler::Candidate { job_id: i64, directory: PathBuf, priority: u8, issue_created_at: DateTime<Utc> }`；`scheduler::pick(queued: &[Candidate], running_dirs: &HashSet<PathBuf>, running: usize, max: usize) -> Vec<i64>`

- [ ] **Step 1: 写失败的测试**

`crates/core/src/lib.rs` 加 `pub mod scheduler;`。`crates/core/src/scheduler.rs`：

```rust
//! Picks which queued jobs start next: one run per directory, at most `max` overall,
//! urgent first, then oldest.

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn c(id: i64, dir: &str, priority: u8, minutes: i64) -> Candidate {
        Candidate {
            job_id: id,
            directory: PathBuf::from(dir),
            priority,
            issue_created_at: Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap() + Duration::minutes(minutes),
        }
    }

    #[test]
    fn respects_global_cap() {
        let queued = [c(1, "a", 0, 0), c(2, "b", 0, 1), c(3, "c", 0, 2)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 2), vec![1, 2]);
        assert_eq!(pick(&queued, &HashSet::new(), 1, 2), vec![1]);
        assert!(pick(&queued, &HashSet::new(), 2, 2).is_empty());
    }

    #[test]
    fn one_run_per_directory() {
        let queued = [c(1, "a", 0, 0), c(2, "a", 0, 1), c(3, "b", 0, 2)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 5), vec![1, 3]);
        let busy: HashSet<PathBuf> = [PathBuf::from("a")].into_iter().collect();
        assert_eq!(pick(&queued, &busy, 1, 5), vec![3]);
    }

    #[test]
    fn urgent_first_then_oldest_and_none_last() {
        let queued = [c(1, "a", 0, 0), c(2, "b", 3, 5), c(3, "c", 1, 9), c(4, "d", 3, 1)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 4), vec![3, 4, 2, 1]);
    }

    #[test]
    fn zero_cap_starts_nothing() {
        assert!(pick(&[c(1, "a", 0, 0)], &HashSet::new(), 0, 0).is_empty());
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core scheduler`
Expected: 编译失败，报 `Candidate`、`pick` 未定义

- [ ] **Step 3: 实现**

在测试模块之前加入：

```rust
use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub job_id: i64,
    pub directory: PathBuf,
    pub priority: u8,
    pub issue_created_at: DateTime<Utc>,
}

/// Linear priority 1 (urgent) … 4 (low) first; 0 means "no priority" and goes last.
fn rank(priority: u8) -> u8 {
    if priority == 0 { 5 } else { priority }
}

pub fn pick(queued: &[Candidate], running_dirs: &HashSet<PathBuf>, running: usize, max: usize) -> Vec<i64> {
    let mut ordered: Vec<&Candidate> = queued.iter().collect();
    ordered.sort_by_key(|c| (rank(c.priority), c.issue_created_at, c.job_id));
    let mut busy = running_dirs.clone();
    let mut picked = Vec::new();
    for candidate in ordered {
        if running + picked.len() >= max {
            break;
        }
        if busy.insert(candidate.directory.clone()) {
            picked.push(candidate.job_id);
        }
    }
    picked
}
```

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core scheduler`
Expected: PASS（4 个）

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add per-directory scheduler with global cap

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: core 提示词

**Files:**
- Create: `crates/core/src/prompt.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: Task 7 的 `TriggerNames`
- Produces: `prompt::COMMENT_PREFIX: &str = "🤖 IssueRelay"`；`prompt::PromptInput<'a> { identifier, url, names: &TriggerNames, comment_language, template }`；`prompt::render(&PromptInput) -> String`

- [ ] **Step 1: 写失败的测试**

`crates/core/src/lib.rs` 加 `pub mod prompt;`。`crates/core/src/prompt.rs`：

```rust
//! Builds the instructions sent to Claude Code / Codex for one issue.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TriggerNames;

    fn input<'a>(names: &'a TriggerNames, template: &'a str) -> PromptInput<'a> {
        PromptInput {
            identifier: "DEMO-7",
            url: "https://linear.app/demo/issue/DEMO-7",
            names,
            comment_language: "zh-CN",
            template,
        }
    }

    #[test]
    fn contains_issue_and_fixed_rules() {
        let names = TriggerNames::default();
        let text = render(&input(&names, ""));
        assert!(text.contains("exactly one Linear issue: DEMO-7 (https://linear.app/demo/issue/DEMO-7)"));
        assert!(text.contains("set the issue status to \"In Progress\""));
        assert!(text.contains("set it to \"In Testing\""));
        assert!(text.contains("Never set the status to \"Done\""));
        assert!(text.contains("Never push to main, master"));
        assert!(text.contains("Never deploy to production"));
        assert!(text.contains("\"🤖 IssueRelay\""));
        assert!(text.contains("remove the label \"ai-ready\""));
        assert!(text.contains("this language: zh-CN"));
        assert!(text.contains(r#"{"result":"in_testing|needs_info|failed","summary":"..."}"#));
        assert!(!text.contains("Team instructions"));
    }

    #[test]
    fn uses_custom_names() {
        let names = TriggerNames {
            label: "robot".into(),
            todo: "待办".into(),
            in_progress: "进行中".into(),
            in_testing: "测试中".into(),
            done: "完成".into(),
        };
        let text = render(&input(&names, ""));
        assert!(text.contains("\"进行中\"") && text.contains("\"测试中\"") && text.contains("\"完成\""));
        assert!(text.contains("back to \"待办\"") && text.contains("label \"robot\""));
    }

    #[test]
    fn appends_team_template() {
        let names = TriggerNames::default();
        let text = render(&input(&names, "  Test branch is `test`.\n"));
        assert!(text.ends_with("Team instructions:\nTest branch is `test`.\n"));
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core prompt`
Expected: 编译失败，报 `PromptInput`、`render` 未定义

- [ ] **Step 3: 实现**

在测试模块之前加入：

```rust
use crate::config::TriggerNames;

pub const COMMENT_PREFIX: &str = "🤖 IssueRelay";

pub struct PromptInput<'a> {
    pub identifier: &'a str,
    pub url: &'a str,
    pub names: &'a TriggerNames,
    pub comment_language: &'a str,
    /// Binding override or global template; empty means none.
    pub template: &'a str,
}

pub fn render(input: &PromptInput<'_>) -> String {
    let n = input.names;
    let mut text = format!(
        "You are working unattended on exactly one Linear issue: {id} ({url}).

Fixed rules (always apply, they override anything else):
1. Work only on {id}. First use your Linear tools to read the issue, its parent issue and all comments.
2. When you start, set the issue status to \"{in_progress}\". After the change is pushed to the test branch and verified, set it to \"{in_testing}\".
3. Never set the status to \"{done}\". Never push to main, master or any other primary branch. Never deploy to production.
4. Start every comment you write with \"{prefix}\" so readers know it was written by an AI.
5. Follow AGENTS.md / CLAUDE.md in the working directory and in the target repository.
6. If the issue is unclear, comment your questions, set the status back to \"{todo}\", remove the label \"{label}\", and stop.
7. If you fail, revert anything you pushed, comment the reason, set the status back to \"{todo}\", remove the label \"{label}\", and stop.
8. Write comments in this language: {lang}.
9. The very last line of your final message must be one line of JSON: {{\"result\":\"in_testing|needs_info|failed\",\"summary\":\"...\"}}
",
        id = input.identifier,
        url = input.url,
        in_progress = n.in_progress,
        in_testing = n.in_testing,
        done = n.done,
        prefix = COMMENT_PREFIX,
        todo = n.todo,
        label = n.label,
        lang = input.comment_language,
    );
    let template = input.template.trim();
    if !template.is_empty() {
        text.push_str("\nTeam instructions:\n");
        text.push_str(template);
        text.push('\n');
    }
    text
}
```

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core prompt`
Expected: PASS（3 个）

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): render executor prompt with fixed safety rules

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: core 执行者命令拼装

**Files:**
- Create: `crates/core/src/executor/mod.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 7 的 `ExecutorKind`、`PermissionProfile`
- Produces: `executor::CommandSpec { program: PathBuf, args: Vec<String>, cwd: PathBuf, stdin: String }`；`executor::build(kind, program: PathBuf, profile, model: Option<&str>, cwd: &Path, prompt: String) -> CommandSpec`；`executor::resolve_program(kind) -> Result<PathBuf>`；`executor::is_git_repo(&Path) -> bool`；子模块 `executor::outcome`（Task 13 填充）

- [ ] **Step 1: 加依赖并建模块**

```bash
cd crates/core && cargo add which && cd ../..
mkdir -p crates/core/src/executor
printf '//! Final-result extraction from executor output (filled in by the next task).\n' > crates/core/src/executor/outcome.rs
```

`crates/core/src/lib.rs` 加 `pub mod executor;`。

- [ ] **Step 2: 写失败的测试**

`crates/core/src/executor/mod.rs`：

```rust
//! Builds the command line for one executor run. The prompt always travels on stdin and the
//! working directory is set on the process, so no user-controlled text ever becomes an
//! argument (this keeps `claude.cmd` / `codex.cmd` safe on Windows).
pub mod outcome;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExecutorKind, PermissionProfile};
    use std::path::PathBuf;

    fn claude(profile: PermissionProfile, model: Option<&str>) -> CommandSpec {
        build(ExecutorKind::Claude, PathBuf::from("claude"), profile, model, Path::new("."), "p".into())
    }

    #[test]
    fn claude_full_profile_args() {
        assert_eq!(
            claude(PermissionProfile::Full, None).args,
            [
                "-p", "--output-format", "stream-json", "--verbose", "--permission-prompts", "none",
                "--permission-mode", "bypassPermissions",
            ]
        );
    }

    #[test]
    fn claude_other_profiles_and_model() {
        assert_eq!(claude(PermissionProfile::Auto, None).args.last().unwrap(), "auto");
        assert_eq!(claude(PermissionProfile::Personal, None).args.last().unwrap(), "dontAsk");
        let with_model = claude(PermissionProfile::Full, Some("claude-sonnet-5-5")).args;
        assert_eq!(&with_model[with_model.len() - 2..], ["--model", "claude-sonnet-5-5"]);
    }

    #[test]
    fn codex_profiles_and_git_check() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let plain = tempfile::tempdir().unwrap();
        let codex = |profile, dir: &Path| {
            build(ExecutorKind::Codex, PathBuf::from("codex"), profile, None, dir, "p".into()).args
        };
        assert_eq!(
            codex(PermissionProfile::Full, repo.path()),
            ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-"]
        );
        assert_eq!(codex(PermissionProfile::Auto, repo.path()), ["exec", "--json", "--approve-for-me", "-"]);
        assert_eq!(codex(PermissionProfile::Personal, repo.path()), ["exec", "--json", "-"]);
        assert_eq!(
            codex(PermissionProfile::Full, plain.path()),
            ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "--skip-git-repo-check", "-"]
        );
    }

    #[test]
    fn args_never_contain_prompt_or_paths() {
        let dir = tempfile::tempdir().unwrap();
        let tricky_dir = dir.path().join("目录 with \"quotes\"");
        std::fs::create_dir(&tricky_dir).unwrap();
        let prompt = "line1\n\"quoted\" & | ^ % 中文 'x'".to_string();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let spec = build(kind, PathBuf::from("x"), PermissionProfile::Full, None, &tricky_dir, prompt.clone());
            assert_eq!(spec.stdin, prompt);
            assert_eq!(spec.cwd, tricky_dir);
            assert!(spec.args.iter().all(|a| !a.contains("中文") && !a.contains("目录") && !a.contains('"')));
        }
    }

    #[test]
    fn unsafe_model_names_are_dropped() {
        let args = claude(PermissionProfile::Full, Some("x\" & calc")).args;
        assert!(!args.contains(&"--model".to_string()));
    }

    #[test]
    fn git_repo_detection() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
        std::fs::write(dir.path().join(".git"), "gitdir: elsewhere").unwrap();
        assert!(is_git_repo(dir.path()));
    }
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core executor`
Expected: 编译失败，报 `build`、`CommandSpec` 等未定义

- [ ] **Step 4: 实现**

在 `pub mod outcome;` 之后、测试模块之前加入：

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{ExecutorKind, PermissionProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Written to the child's stdin, then stdin is closed.
    pub stdin: String,
}

/// Resolves `claude` / `codex` on PATH, including `.cmd` shims on Windows.
pub fn resolve_program(kind: ExecutorKind) -> Result<PathBuf> {
    which::which(kind.program_name())
        .with_context(|| format!("`{}` was not found on PATH", kind.program_name()))
}

pub fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn is_safe_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 100
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

pub fn build(
    kind: ExecutorKind,
    program: PathBuf,
    profile: PermissionProfile,
    model: Option<&str>,
    cwd: &Path,
    prompt: String,
) -> CommandSpec {
    let model = model.filter(|m| is_safe_token(m));
    let mut args: Vec<String> = Vec::new();
    match kind {
        ExecutorKind::Claude => {
            args.extend(
                ["-p", "--output-format", "stream-json", "--verbose", "--permission-prompts", "none", "--permission-mode"]
                    .map(String::from),
            );
            args.push(
                match profile {
                    PermissionProfile::Full => "bypassPermissions",
                    PermissionProfile::Auto => "auto",
                    PermissionProfile::Personal => "dontAsk",
                }
                .into(),
            );
            if let Some(m) = model {
                args.extend(["--model".to_string(), m.to_string()]);
            }
        }
        ExecutorKind::Codex => {
            args.extend(["exec", "--json"].map(String::from));
            match profile {
                PermissionProfile::Full => args.push("--dangerously-bypass-approvals-and-sandbox".into()),
                PermissionProfile::Auto => args.push("--approve-for-me".into()),
                PermissionProfile::Personal => {}
            }
            if !is_git_repo(cwd) {
                args.push("--skip-git-repo-check".into());
            }
            if let Some(m) = model {
                args.extend(["-m".to_string(), m.to_string()]);
            }
            args.push("-".into());
        }
    }
    CommandSpec { program, args, cwd: cwd.to_path_buf(), stdin: prompt }
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core executor`
Expected: PASS（6 个）

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): build claude/codex command specs per permission profile

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 13: core 最终结果解析

**Files:**
- Modify: `crates/core/src/executor/outcome.rs`、`crates/core/Cargo.toml`
- Create: `crates/core/tests/fixtures/claude-stream.jsonl`、`crates/core/tests/fixtures/codex-exec.jsonl`（从 Task 4 的样本复制）

**Interfaces:**
- Consumes: Task 7 的 `ExecutorKind`
- Produces: `outcome::Report { InTesting(String), NeedsInfo(String), Failed(String) }`（`summary()`）；`outcome::FinalTextTracker`：`new(kind)`、`observe(&mut self, line: &str)`、`final_text()`、`into_final_text()`；`outcome::parse_report(&str) -> Option<Report>`

- [ ] **Step 1: 加依赖并复制样本**

```bash
cd crates/core && cargo add serde_json && cd ../..
mkdir -p crates/core/tests/fixtures
cp docs/gates/samples/m0/executors/claude-stream.jsonl crates/core/tests/fixtures/claude-stream.jsonl
cp docs/gates/samples/m0/executors/codex-exec.jsonl crates/core/tests/fixtures/codex-exec.jsonl
```

- [ ] **Step 2: 写失败的测试**

`crates/core/src/executor/outcome.rs` 整体替换为：

```rust
//! Pulls the executor's final message out of its JSON event stream and parses the one-line
//! result JSON the prompt asks for. Event shapes were confirmed in M0 (docs/gates).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExecutorKind;

    #[test]
    fn claude_final_text_comes_from_result_event() {
        let mut t = FinalTextTracker::new(ExecutorKind::Claude);
        t.observe(r#"{"type":"system","subtype":"init"}"#);
        t.observe(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}"#);
        t.observe("not json at all");
        t.observe(r#"{"type":"result","subtype":"success","result":"Done.\n{\"result\":\"in_testing\",\"summary\":\"pushed\"}"}"#);
        assert_eq!(parse_report(t.final_text().unwrap()), Some(Report::InTesting("pushed".into())));
    }

    #[test]
    fn codex_final_text_is_last_agent_message() {
        let mut t = FinalTextTracker::new(ExecutorKind::Codex);
        t.observe(r#"{"type":"thread.started","thread_id":"0"}"#);
        t.observe(r#"{"type":"item.completed","item":{"type":"agent_message","text":"first"}}"#);
        t.observe(r#"{"type":"item.completed","item":{"type":"command_execution","command":"ls"}}"#);
        t.observe(r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"result\":\"needs_info\",\"summary\":\"which repo?\"}"}}"#);
        t.observe(r#"{"type":"turn.completed"}"#);
        assert_eq!(t.into_final_text().as_deref(), Some(r#"{"result":"needs_info","summary":"which repo?"}"#));
    }

    #[test]
    fn report_parsing_variants() {
        assert_eq!(parse_report("{\"result\":\"failed\",\"summary\":\"build broke\"}"), Some(Report::Failed("build broke".into())));
        assert_eq!(parse_report("text\n```\n{\"result\":\"in_testing\",\"summary\":\"ok\"}\n```"), Some(Report::InTesting("ok".into())));
        assert_eq!(parse_report("{\"result\":\"in_testing\"}"), Some(Report::InTesting(String::new())));
        assert_eq!(parse_report("{\"result\":\"maybe\"}\nno json"), None);
        assert_eq!(parse_report(""), None);
        assert_eq!(Report::NeedsInfo("q".into()).summary(), "q");
    }

    #[test]
    fn last_matching_json_line_wins() {
        let text = "{\"result\":\"failed\",\"summary\":\"old\"}\nmore\n{\"result\":\"in_testing\",\"summary\":\"new\"}";
        assert_eq!(parse_report(text), Some(Report::InTesting("new".into())));
    }

    #[test]
    fn real_m0_samples_yield_reports() {
        for (kind, sample) in [
            (ExecutorKind::Claude, include_str!("../../tests/fixtures/claude-stream.jsonl")),
            (ExecutorKind::Codex, include_str!("../../tests/fixtures/codex-exec.jsonl")),
        ] {
            let mut t = FinalTextTracker::new(kind);
            sample.lines().for_each(|line| t.observe(line));
            let report = t.final_text().and_then(parse_report);
            assert!(report.is_some(), "{kind:?} sample produced no report");
        }
    }
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core outcome`
Expected: 编译失败，报 `FinalTextTracker`、`parse_report`、`Report` 未定义

- [ ] **Step 4: 实现**

在测试模块之前加入：

```rust
use serde_json::Value;

use crate::config::ExecutorKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    InTesting(String),
    NeedsInfo(String),
    Failed(String),
}

impl Report {
    pub fn summary(&self) -> &str {
        match self {
            Report::InTesting(s) | Report::NeedsInfo(s) | Report::Failed(s) => s,
        }
    }
}

#[derive(Debug)]
pub struct FinalTextTracker {
    kind: ExecutorKind,
    last: Option<String>,
}

impl FinalTextTracker {
    pub fn new(kind: ExecutorKind) -> Self {
        Self { kind, last: None }
    }

    pub fn observe(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<Value>(line) else { return };
        let text = match self.kind {
            ExecutorKind::Claude if event["type"] == "result" => event["result"].as_str(),
            ExecutorKind::Codex
                if event["type"] == "item.completed" && event["item"]["type"] == "agent_message" =>
            {
                event["item"]["text"].as_str()
            }
            _ => None,
        };
        if let Some(text) = text {
            self.last = Some(text.to_string());
        }
    }

    pub fn final_text(&self) -> Option<&str> {
        self.last.as_deref()
    }

    pub fn into_final_text(self) -> Option<String> {
        self.last
    }
}

/// Scans from the end for the first line that is a JSON object with a known `result`.
pub fn parse_report(text: &str) -> Option<Report> {
    for line in text.lines().rev() {
        let line = line.trim().trim_matches('`').trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        let summary = value["summary"].as_str().unwrap_or_default().to_string();
        let report = match value["result"].as_str() {
            Some("in_testing") => Report::InTesting(summary),
            Some("needs_info") => Report::NeedsInfo(summary),
            Some("failed") => Report::Failed(summary),
            _ => continue,
        };
        return Some(report);
    }
    None
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core outcome`
Expected: PASS（5 个）。`real_m0_samples_yield_reports` 失败说明 M0 样本的事件形态与 `observe` 的假设不同：按样本改 `observe` 与对应测试，并在 M0 验收报告里补记。

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): extract final executor message and parse result report

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 14: core 测试用假执行者

**Files:**
- Create: `crates/core/src/bin/fake_agent.rs`、`crates/core/tests/fake_agent.rs`
- Modify: `crates/core/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces: 二进制 `fake-agent`，集成测试里用 `env!("CARGO_BIN_EXE_fake-agent")` 定位。用法：`fake-agent <script.json>`，脚本是步骤数组，每步可含：`{"out": 文本}` 写一行到标准输出；`{"err": 文本}` 写一行到标准错误；`{"sleep_ms": n}`；`{"stdin_to": 路径}` 把标准输入全部读完写入文件；`{"heartbeat": 路径, "every_ms": n}` 启动一个每 n 毫秒往文件追加一个字节的后台子进程；`{"repeat_out": 文本, "every_ms": n}` 无限循环输出；`{"exit": 码}` 立即退出。没有 `exit` 步骤时退出码为 0。

- [ ] **Step 1: 声明二进制目标**

在 `crates/core/Cargo.toml` 末尾加：

```toml
[[bin]]
name = "fake-agent"
path = "src/bin/fake_agent.rs"
test = false
doc = false
```

- [ ] **Step 2: 写失败的测试**

`crates/core/tests/fake_agent.rs`：

```rust
use std::process::Command;

#[test]
fn runs_script_and_exits_with_code() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    std::fs::write(&script, r#"[{"out":"one"},{"err":"warn"},{"out":"two"},{"exit":7}]"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fake-agent")).arg(&script).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().collect::<Vec<_>>(), ["one", "two"]);
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "warn");
}

#[test]
fn copies_stdin_to_file() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("stdin.txt");
    let script = dir.path().join("script.json");
    std::fs::write(&script, serde_json::json!([{"stdin_to": copy}]).to_string()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg(&script)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all("多行\n\"提示词\"".as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "多行\n\"提示词\"");
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core --test fake_agent`
Expected: 失败，`src/bin/fake_agent.rs` 不存在

- [ ] **Step 4: 实现**

`crates/core/src/bin/fake_agent.rs`：

```rust
//! Test double for `claude` / `codex`, driven by a JSON script. Only used by tests.
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--heartbeat") {
        let file = args.get(2).expect("usage: fake-agent --heartbeat <file> <ms>");
        let every = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
        heartbeat(Path::new(file), every);
    }
    let script = args.get(1).expect("usage: fake-agent <script.json>");
    let text = std::fs::read_to_string(script).expect("read script");
    let steps: Vec<Value> = serde_json::from_str(&text).expect("parse script");
    let mut code = 0;
    for step in steps {
        if let Some(line) = step.get("out").and_then(Value::as_str) {
            println!("{line}");
            std::io::stdout().flush().ok();
        }
        if let Some(line) = step.get("err").and_then(Value::as_str) {
            eprintln!("{line}");
        }
        if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            std::thread::sleep(Duration::from_millis(ms));
        }
        if let Some(path) = step.get("stdin_to").and_then(Value::as_str) {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).expect("read stdin");
            std::fs::write(path, input).expect("write stdin copy");
        }
        if let Some(path) = step.get("heartbeat").and_then(Value::as_str) {
            let every = step.get("every_ms").and_then(Value::as_u64).unwrap_or(100);
            Command::new(std::env::current_exe().expect("current exe"))
                .args(["--heartbeat", path, &every.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn heartbeat");
        }
        if let Some(line) = step.get("repeat_out").and_then(Value::as_str) {
            let every = step.get("every_ms").and_then(Value::as_u64).unwrap_or(100);
            loop {
                println!("{line}");
                std::io::stdout().flush().ok();
                std::thread::sleep(Duration::from_millis(every));
            }
        }
        if let Some(exit) = step.get("exit").and_then(Value::as_i64) {
            code = exit as i32;
            break;
        }
    }
    std::process::exit(code);
}

fn heartbeat(file: &Path, every_ms: u64) -> ! {
    loop {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
            let _ = f.write_all(b".");
        }
        std::thread::sleep(Duration::from_millis(every_ms));
    }
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core --test fake_agent`
Expected: PASS（2 个）

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "test(core): add scripted fake-agent binary for runner tests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 15: core 子进程运行器

**Files:**
- Create: `crates/core/src/runner.rs`、`crates/core/tests/runner.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 12 的 `CommandSpec`；Task 13 的 `FinalTextTracker`；Task 14 的 `fake-agent`
- Produces: `runner::RunLimits { total: Duration, stall: Duration }`；`runner::ExitKind { Exited(Option<i32>), TimedOut, Stalled, Canceled, SpawnFailed(String) }`；`runner::RunOutput { exit: ExitKind, final_text: Option<String> }`；`runner::run(spec: CommandSpec, kind: ExecutorKind, log_path: &Path, limits: RunLimits, cancel: CancellationToken) -> RunOutput`。日志文件每行一条：标准输出原样写入，标准错误写成 `{"stream":"stderr","text":…}`，启动失败写一行 `{"stream":"runner","text":…}`；Unix 上日志权限 0600。`run` 的 future 被丢弃时也会结束整棵进程树（评审补充）。

- [ ] **Step 1: 加依赖**

```bash
cd crates/core
cargo add tokio --features full
cargo add tokio-util
cargo add process-wrap --features tokio1
cd ../..
```

- [ ] **Step 2: 写失败的测试**

`crates/core/src/lib.rs` 加 `pub mod runner;`，并新建空的 `crates/core/src/runner.rs`（只含一行文档注释 `//! Runs one executor process.`）。`crates/core/tests/runner.rs`：

```rust
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use issuerelay_core::{
    config::ExecutorKind,
    executor::CommandSpec,
    runner::{self, ExitKind, RunLimits},
};
use tokio_util::sync::CancellationToken;

fn agent() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-agent"))
}

fn script(dir: &Path, steps: serde_json::Value) -> PathBuf {
    let path = dir.join("script.json");
    std::fs::write(&path, steps.to_string()).unwrap();
    path
}

fn spec(program: PathBuf, cwd: &Path, script: &Path, stdin: &str) -> CommandSpec {
    CommandSpec {
        program,
        args: vec![script.to_string_lossy().into_owned()],
        cwd: cwd.to_path_buf(),
        stdin: stdin.into(),
    }
}

fn limits(total_ms: u64, stall_ms: u64) -> RunLimits {
    RunLimits { total: Duration::from_millis(total_ms), stall: Duration::from_millis(stall_ms) }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

const RESULT_LINE: &str = r#"{"type":"result","result":"done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}"}"#;

#[tokio::test]
async fn exits_zero_captures_final_text_and_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("stdin.txt");
    let s = script(dir.path(), serde_json::json!([
        {"stdin_to": copy}, {"out": "{\"type\":\"system\"}"}, {"err": "note"}, {"out": RESULT_LINE}, {"exit": 0}
    ]));
    let log = dir.path().join("logs").join("1.jsonl");
    let prompt = "多语言 \"quoted\" prompt\nline 2";
    let out = runner::run(spec(agent(), dir.path(), &s, prompt), ExecutorKind::Claude, &log, limits(30_000, 30_000), CancellationToken::new()).await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert!(out.final_text.unwrap().contains("in_testing"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), prompt);
    let lines: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().map(String::from).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().any(|l| l.contains(r#""stream":"stderr""#) && l.contains("note")));
}

#[tokio::test]
async fn nonzero_exit_code_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"exit": 3}]));
    let out = runner::run(spec(agent(), dir.path(), &s, ""), ExecutorKind::Codex, &dir.path().join("l.jsonl"), limits(30_000, 30_000), CancellationToken::new()).await;
    assert_eq!(out.exit, ExitKind::Exited(Some(3)));
    assert_eq!(out.final_text, None);
}

#[tokio::test]
async fn stalled_run_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"out": "x"}, {"sleep_ms": 20_000}]));
    let started = Instant::now();
    let out = runner::run(spec(agent(), dir.path(), &s, ""), ExecutorKind::Claude, &dir.path().join("l.jsonl"), limits(30_000, 400), CancellationToken::new()).await;
    assert_eq!(out.exit, ExitKind::Stalled);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn chatty_run_hits_total_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"repeat_out": "tick", "every_ms": 50}]));
    let out = runner::run(spec(agent(), dir.path(), &s, ""), ExecutorKind::Claude, &dir.path().join("l.jsonl"), limits(600, 5_000), CancellationToken::new()).await;
    assert_eq!(out.exit, ExitKind::TimedOut);
}

#[tokio::test]
async fn cancel_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"sleep_ms": 20_000}]));
    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        canceller.cancel();
    });
    let out = runner::run(spec(agent(), dir.path(), &s, ""), ExecutorKind::Claude, &dir.path().join("l.jsonl"), limits(30_000, 30_000), token).await;
    assert_eq!(out.exit, ExitKind::Canceled);
}

#[tokio::test]
async fn kill_reaches_grandchildren() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(dir.path(), serde_json::json!([{"heartbeat": hb, "every_ms": 50}, {"sleep_ms": 20_000}]));
    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        canceller.cancel();
    });
    let out = runner::run(spec(agent(), dir.path(), &s, ""), ExecutorKind::Claude, &dir.path().join("l.jsonl"), limits(30_000, 30_000), token).await;
    assert_eq!(out.exit, ExitKind::Canceled);
    assert!(file_len(&hb) > 0, "heartbeat child never ran");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let settled = file_len(&hb);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(file_len(&hb), settled, "grandchild survived the kill");
}

#[tokio::test]
async fn normal_exit_returns_promptly_even_with_background_child() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(dir.path(), serde_json::json!([{"heartbeat": hb, "every_ms": 50}, {"out": RESULT_LINE}, {"exit": 0}]));
    let started = Instant::now();
    let out = runner::run(spec(agent(), dir.path(), &s, ""), ExecutorKind::Claude, &dir.path().join("l.jsonl"), limits(60_000, 60_000), CancellationToken::new()).await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert!(started.elapsed() < Duration::from_secs(10), "waited for the background child");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let settled = file_len(&hb);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(file_len(&hb), settled, "background child was left running");
}

#[tokio::test]
async fn missing_program_reports_spawn_failure() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([]));
    let out = runner::run(spec(dir.path().join("no-such-program"), dir.path(), &s, ""), ExecutorKind::Claude, &dir.path().join("l.jsonl"), limits(5_000, 5_000), CancellationToken::new()).await;
    assert!(matches!(out.exit, ExitKind::SpawnFailed(_)));
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_wrapper_passes_prompt_and_paths_intact() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("目录 with space");
    std::fs::create_dir(&work).unwrap();
    let copy = work.join("stdin.txt");
    let s = script(&work, serde_json::json!([{"stdin_to": copy}, {"exit": 0}]));
    let wrapper = work.join("agent.cmd");
    std::fs::write(&wrapper, format!("@echo off\r\n\"{}\" %*\r\n", agent().display())).unwrap();
    let prompt = "line1\n\"quoted\" & | ^ % 中文 'x'";
    let out = runner::run(spec(wrapper, &work, &s, prompt), ExecutorKind::Codex, &work.join("l.jsonl"), limits(30_000, 30_000), CancellationToken::new()).await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), prompt);
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core --test runner`
Expected: 编译失败，报 `runner::run`、`RunLimits`、`ExitKind` 未定义

- [ ] **Step 4: 实现**

`crates/core/src/runner.rs` 整体替换为：

```rust
//! Runs one executor process: feeds the prompt on stdin, streams stdout/stderr into a JSONL
//! log, enforces the total and no-output limits, and always kills the whole process tree
//! (process group on Unix, Job Object on Windows) when the run ends.
use std::{path::Path, process::Stdio, time::Duration};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::config::ExecutorKind;
use crate::executor::{CommandSpec, outcome::FinalTextTracker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    pub total: Duration,
    pub stall: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitKind {
    Exited(Option<i32>),
    TimedOut,
    Stalled,
    Canceled,
    SpawnFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub exit: ExitKind,
    pub final_text: Option<String>,
}

enum Line {
    Out(String),
    Err(String),
}

fn failed(message: String) -> RunOutput {
    RunOutput { exit: ExitKind::SpawnFailed(message), final_text: None }
}

pub async fn run(
    spec: CommandSpec,
    kind: ExecutorKind,
    log_path: &Path,
    limits: RunLimits,
    cancel: CancellationToken,
) -> RunOutput {
    if let Some(dir) = log_path.parent()
        && let Err(e) = tokio::fs::create_dir_all(dir).await
    {
        return failed(format!("cannot create log directory: {e}"));
    }
    let mut log = match File::create(log_path).await {
        Ok(file) => file,
        Err(e) => return failed(format!("cannot create log file: {e}")),
    };

    let mut command = CommandWrap::with_new(&spec.program, |c| {
        c.args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    command.wrap(KillOnDrop);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return failed(e.to_string()),
    };

    if let Some(mut stdin) = child.stdin().take() {
        let prompt = spec.stdin.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(stdout) = child.stdout().take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(Line::Out(line)).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(stderr) = child.stderr().take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(Line::Err(line)).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut tracker = FinalTextTracker::new(kind);
    let deadline = Instant::now() + limits.total;
    let mut last_output = Instant::now();
    let mut pipes_open = true;
    let exit = {
        // Wait on the executor process itself. process-wrap's own wait() also waits for every
        // process left in a Windows job, which would turn a normal exit into a stall whenever
        // the AI leaves a background process running.
        // SAFETY: we only call wait() on the native child; the wrappers' kill and reap logic
        // still runs below through `child`.
        let Some(leader) = (unsafe { child.try_inner_child_mut() }) else {
            return failed("process wrapper hid the native child".into());
        };
        loop {
            tokio::select! {
                status = leader.wait() => break ExitKind::Exited(status.ok().and_then(|s| s.code())),
                line = rx.recv(), if pipes_open => match line {
                    Some(line) => {
                        last_output = Instant::now();
                        record(&mut log, &mut tracker, &line).await;
                    }
                    None => pipes_open = false,
                },
                _ = tokio::time::sleep_until(deadline) => break ExitKind::TimedOut,
                _ = tokio::time::sleep_until(last_output + limits.stall) => break ExitKind::Stalled,
                _ = cancel.cancelled() => break ExitKind::Canceled,
            }
        }
    };

    // Kill whatever is left of the tree (also after a normal exit), then reap.
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    // Keep lines already buffered; the final result often arrives right before exit.
    let drain_until = Instant::now() + Duration::from_secs(2);
    while let Ok(Some(line)) = tokio::time::timeout_at(drain_until, rx.recv()).await {
        record(&mut log, &mut tracker, &line).await;
    }
    let _ = log.flush().await;
    RunOutput { exit, final_text: tracker.into_final_text() }
}

async fn record(log: &mut File, tracker: &mut FinalTextTracker, line: &Line) {
    let text = match line {
        Line::Out(text) => {
            tracker.observe(text);
            text.clone()
        }
        Line::Err(text) => serde_json::json!({"stream": "stderr", "text": text}).to_string(),
    };
    let _ = log.write_all(text.as_bytes()).await;
    let _ = log.write_all(b"\n").await;
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core --test runner`
Expected: PASS（macOS 8 个；Windows 9 个，多 `cmd_wrapper_passes_prompt_and_paths_intact`）

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): run executors with log capture, limits and tree kill

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 16: core Linear 接口与 MCP 实现

> 执行说明（2026-10-11）：下面的代码块是派发时的版本，已落地代码以修复提交 `fix(core): treat unreadable Linear replies as errors and bound every call` 及其后的测试提交为准：读不懂的回复一律报错（`uuid` 与状态必填，列表里任何一条读不懂都让整次调用失败），只有「Could not find referenced Issue」算需求不存在，每次调用 30 秒超时，连接时检查 `REQUIRED_ARGUMENTS`（设计第 7 节），`list_projects` 用 `limit: 50` 并要 `uuid`，假服务端按 `crates/core/tests/fixtures/linear-tools.json` 校验每次调用。

**Files:**
- Create: `crates/core/src/linear/mod.rs`、`crates/core/src/linear/parse.rs`、`crates/core/src/linear/mcp.rs`、`crates/core/tests/common/mod.rs`、`crates/core/tests/common/fake_server.rs`、`crates/core/tests/common/fake_linear.rs`、`crates/core/tests/linear_mcp.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 8 的 `Issue`；Task 3 的脱敏样本
- Produces:
  - `linear::Viewer { id, name }`、`linear::Project { id, name }`、`linear::Team { id, name }`
  - `linear::REQUIRED_TOOLS: [&str; 8]`、`linear::ISSUE_FIELDS: [&str; 13]`
  - `trait linear::LinearApi: Send + Sync`：`viewer()`、`candidate_issues(label, state)`、`issue(id) -> Option<Issue>`、`set_state(id, state)`、`projects()`、`labels()`、`teams()`、`statuses(team_id)`，全部 `async` 返回 `anyhow::Result`
  - `linear::parse::{Page<T>, parse_issue, parse_issue_page, parse_single_issue, parse_viewer, parse_projects, parse_labels, parse_teams, parse_statuses}`
  - `linear::mcp::{LinearClient, LinearError, McpLinear}`：`McpLinear::new(LinearClient) -> Result<McpLinear>`（缺工具时返回 `LinearError::MissingTools`）、`close()`
  - 测试共用：`common::fake_server::{FakeLinearServer, PartialServer, connect_full, connect_partial}`、`common::fake_linear::FakeLinear`（内存版 `LinearApi`）、`common::issue(uuid, project)`

- [ ] **Step 1: 加依赖**

```bash
cd crates/core
cargo add async-trait thiserror
cargo add reqwest@0.13
cargo add rmcp@3.5 --features client,transport-streamable-http-client-reqwest,auth,reqwest
cargo add --dev rmcp@3.5 --features server,macros,transport-async-rw,schemars
cargo add --dev schemars
cd ../..
```

- [ ] **Step 2: 写解析模块的失败测试**

`crates/core/src/lib.rs` 加 `pub mod linear;`。`crates/core/src/linear/mod.rs` 先写：

```rust
//! Linear access through Linear's official MCP server.
pub mod mcp;
pub mod parse;
```

`crates/core/src/linear/mcp.rs` 先只写一行 `//! rmcp-backed LinearApi.`。`crates/core/src/linear/parse.rs`：

```rust
//! Parses the JSON text Linear's MCP tools return. Field shapes follow the M0 samples in
//! docs/gates/samples/m0/linear; alternatives seen in other Linear payloads are accepted too.

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUE: &str = r#"{"id":"DEMO-1","uuid":"00000000-0000-0000-0000-000000000001","title":"Demo task","url":"https://linear.app/demo/issue/DEMO-1","status":"Todo","labels":["ai-ready","Bug"],"project":"Demo Project","projectId":"proj-1","assigneeId":"user-1","parentId":"DEMO-0","priority":2,"createdAt":"2026-10-01T08:00:00.000Z","updatedAt":"2026-10-01T09:00:00.500Z"}"#;

    #[test]
    fn parses_flat_issue() {
        let v: serde_json::Value = serde_json::from_str(ISSUE).unwrap();
        let issue = parse_issue(&v).unwrap();
        assert_eq!(issue.identifier, "DEMO-1");
        assert_eq!(issue.uuid, "00000000-0000-0000-0000-000000000001");
        assert_eq!(issue.state, "Todo");
        assert_eq!(issue.labels, ["ai-ready", "Bug"]);
        assert_eq!(issue.project_id.as_deref(), Some("proj-1"));
        assert_eq!(issue.project_name.as_deref(), Some("Demo Project"));
        assert_eq!(issue.assignee_id.as_deref(), Some("user-1"));
        assert_eq!(issue.parent_identifier.as_deref(), Some("DEMO-0"));
        assert_eq!(issue.priority, 2);
        assert_eq!(issue.updated_at.timestamp_subsec_millis(), 500);
    }

    #[test]
    fn parses_nested_shapes() {
        let v = serde_json::json!({
            "identifier": "DEMO-2", "title": "t", "url": "u",
            "state": {"name": "In Testing"},
            "labels": [{"name": "ai-ready"}],
            "project": {"id": "proj-2", "name": "Other"},
            "priority": {"value": 9},
            "createdAt": "2026-10-01T08:00:00Z", "updatedAt": "2026-10-01T08:00:00Z"
        });
        let issue = parse_issue(&v).unwrap();
        assert_eq!(issue.uuid, "DEMO-2");
        assert_eq!(issue.state, "In Testing");
        assert_eq!(issue.labels, ["ai-ready"]);
        assert_eq!(issue.project_name.as_deref(), Some("Other"));
        assert_eq!(issue.project_id.as_deref(), Some("proj-2"));
        assert_eq!(issue.priority, 4);
    }

    #[test]
    fn issue_without_timestamps_is_skipped() {
        let v = serde_json::json!({"id": "DEMO-3", "title": "t"});
        assert!(parse_issue(&v).is_none());
    }

    #[test]
    fn page_cursor_rules() {
        let page = parse_issue_page(&format!(r#"{{"issues":[{ISSUE}],"hasNextPage":true,"cursor":"c2"}}"#)).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("c2"));
        let last = parse_issue_page(&format!(r#"{{"issues":[{ISSUE}],"hasNextPage":false,"cursor":"stale"}}"#)).unwrap();
        assert_eq!(last.next_cursor, None);
        let alt = parse_issue_page(r#"{"issues":[],"nextCursor":"n"}"#).unwrap();
        assert_eq!(alt.next_cursor.as_deref(), Some("n"));
    }

    #[test]
    fn metadata_parsers() {
        assert_eq!(parse_viewer(r#"{"id":"user-1","name":"Jane Doe"}"#).unwrap().id, "user-1");
        assert_eq!(parse_viewer(r#"{"user":{"id":"user-2","displayName":"jd"}}"#).unwrap().name, "jd");
        assert_eq!(parse_projects(r#"{"projects":[{"id":"proj-1","name":"Demo Project"}]}"#).unwrap().items[0].name, "Demo Project");
        assert_eq!(parse_projects(r#"{"projects":[{"id":"P-DEMO-1","uuid":"proj-uuid","name":"Demo Project"}]}"#).unwrap().items[0].id, "proj-uuid");
        assert_eq!(parse_labels(r#"{"labels":[{"name":"ai-ready"},{"name":"Bug"}]}"#).unwrap().items, ["ai-ready", "Bug"]);
        assert_eq!(parse_teams(r#"{"teams":[{"id":"team-1","name":"Demo"}]}"#).unwrap().items[0].id, "team-1");
        assert_eq!(parse_statuses(r#"[{"name":"Todo"},{"name":"In Testing"}]"#).unwrap(), ["Todo", "In Testing"]);
        assert_eq!(parse_statuses(r#"{"statuses":[{"name":"Done"}]}"#).unwrap(), ["Done"]);
    }

    #[test]
    fn non_json_text_is_a_clear_error() {
        let err = parse_issue_page("Internal error").unwrap_err().to_string();
        assert!(err.contains("non-JSON") && err.contains("Internal error"));
        assert!(parse_issue_page(r#"{"nodes":[]}"#).is_err());
    }

    #[test]
    fn m0_samples_parse() {
        let page = parse_issue_page(include_str!("../../../../docs/gates/samples/m0/linear/list_issues_page1.json")).unwrap();
        assert!(!page.items.is_empty(), "sample page has no parsable issues");
        let projects = parse_projects(include_str!("../../../../docs/gates/samples/m0/linear/list_projects.json")).unwrap();
        assert_eq!(
            page.items[0].project_id.as_deref(),
            Some(projects.items[0].id.as_str()),
            "an issue's projectId must match the project uuid (M0 finding F1)"
        );
        assert!(!parse_viewer(include_str!("../../../../docs/gates/samples/m0/linear/get_user_me.json")).unwrap().id.is_empty());
        assert!(!parse_teams(include_str!("../../../../docs/gates/samples/m0/linear/list_teams.json")).unwrap().items.is_empty());
        assert!(!parse_statuses(include_str!("../../../../docs/gates/samples/m0/linear/list_issue_statuses.json")).unwrap().is_empty());
    }
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p issuerelay-core linear::parse`
Expected: 编译失败，报 `parse_issue` 等未定义

- [ ] **Step 4: 实现接口与解析**

`crates/core/src/linear/mod.rs` 整体替换为：

```rust
//! Linear access through Linear's official MCP server.
pub mod mcp;
pub mod parse;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::model::Issue;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewer {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
}

/// Tools IssueRelay calls; checked with `tools/list` on every connection.
pub const REQUIRED_TOOLS: [&str; 8] = [
    "list_issues",
    "get_issue",
    "save_issue",
    "get_user",
    "list_projects",
    "list_issue_labels",
    "list_teams",
    "list_issue_statuses",
];

pub const ISSUE_FIELDS: [&str; 13] = [
    "id", "uuid", "title", "url", "status", "labels", "project", "projectId", "assigneeId",
    "parentId", "priority", "createdAt", "updatedAt",
];

#[async_trait::async_trait]
pub trait LinearApi: Send + Sync {
    async fn viewer(&self) -> Result<Viewer>;
    /// Issues assigned to the authorized user with `label` in status `state`, all pages.
    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>>;
    /// `None` when Linear reports the issue does not exist.
    async fn issue(&self, id: &str) -> Result<Option<Issue>>;
    async fn set_state(&self, id: &str, state: &str) -> Result<()>;
    async fn projects(&self) -> Result<Vec<Project>>;
    async fn labels(&self) -> Result<Vec<String>>;
    async fn teams(&self) -> Result<Vec<Team>>;
    async fn statuses(&self, team_id: &str) -> Result<Vec<String>>;
}
```

在 `crates/core/src/linear/parse.rs` 的测试模块之前加入：

```rust
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{Project, Team, Viewer};
use crate::model::Issue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn time(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    let raw = v.get(key).and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(raw).ok().map(|t| t.with_timezone(&Utc))
}

fn priority(v: &Value) -> u8 {
    let raw = match v.get("priority") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::Object(o)) => o.get("value").and_then(Value::as_u64),
        _ => None,
    };
    raw.unwrap_or(0).min(4) as u8
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string).or_else(|| text(item, "name")))
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_issue(v: &Value) -> Option<Issue> {
    let identifier = text(v, "identifier").or_else(|| text(v, "id"))?;
    let project = v.get("project");
    Some(Issue {
        uuid: text(v, "uuid").unwrap_or_else(|| identifier.clone()),
        title: text(v, "title").unwrap_or_default(),
        url: text(v, "url").unwrap_or_default(),
        state: text(v, "status")
            .or_else(|| v.get("state").and_then(|s| text(s, "name")))
            .unwrap_or_default(),
        labels: names(v, "labels"),
        project_id: text(v, "projectId").or_else(|| project.and_then(|p| text(p, "id"))),
        project_name: text(v, "project").or_else(|| project.and_then(|p| text(p, "name"))),
        assignee_id: text(v, "assigneeId"),
        parent_identifier: text(v, "parentId"),
        priority: priority(v),
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        identifier,
    })
}

fn cursor(v: &Value) -> Option<String> {
    if v.get("hasNextPage").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    ["cursor", "nextCursor", "endCursor"].iter().find_map(|key| text(v, key))
}

fn json(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).with_context(|| {
        let preview: String = raw.chars().take(200).collect();
        format!("Linear returned non-JSON text: {preview}")
    })
}

fn list<T>(raw: &str, key: &str, item: impl Fn(&Value) -> Option<T>) -> Result<Page<T>> {
    let v = json(raw)?;
    let array = v
        .get(key)
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .ok_or_else(|| anyhow!("Linear response has no `{key}` array"))?;
    Ok(Page { items: array.iter().filter_map(item).collect(), next_cursor: cursor(&v) })
}

pub fn parse_issue_page(raw: &str) -> Result<Page<Issue>> {
    list(raw, "issues", parse_issue)
}

pub fn parse_single_issue(raw: &str) -> Result<Option<Issue>> {
    let v = json(raw)?;
    Ok(parse_issue(v.get("issue").unwrap_or(&v)))
}

pub fn parse_viewer(raw: &str) -> Result<Viewer> {
    let v = json(raw)?;
    let user = v.get("user").unwrap_or(&v);
    Ok(Viewer {
        id: text(user, "id").ok_or_else(|| anyhow!("Linear user has no id"))?,
        name: text(user, "name").or_else(|| text(user, "displayName")).unwrap_or_default(),
    })
}

pub fn parse_projects(raw: &str) -> Result<Page<Project>> {
    // Issues reference a project by its uuid; the project's `id` is its P-<KEY>-N identifier (M0 finding F1).
    list(raw, "projects", |p| {
        Some(Project { id: text(p, "uuid").or_else(|| text(p, "id"))?, name: text(p, "name").unwrap_or_default() })
    })
}

pub fn parse_labels(raw: &str) -> Result<Page<String>> {
    list(raw, "labels", |l| text(l, "name"))
}

pub fn parse_teams(raw: &str) -> Result<Page<Team>> {
    list(raw, "teams", |t| Some(Team { id: text(t, "id")?, name: text(t, "name").unwrap_or_default() }))
}

pub fn parse_statuses(raw: &str) -> Result<Vec<String>> {
    Ok(list(raw, "statuses", |s| text(s, "name"))?.items)
}
```

Run: `cargo test -p issuerelay-core linear::parse`
Expected: PASS（7 个）。`m0_samples_parse` 失败说明 M0 样本的字段形态与这里不同：按样本改解析函数和上面的测试，再重跑。

- [ ] **Step 5: 写测试用的假 Linear（MCP 服务端版与内存版）**

`crates/core/tests/common/mod.rs`：

```rust
#![allow(dead_code)]
pub mod fake_linear;
pub mod fake_server;

use chrono::{TimeZone, Utc};
use issuerelay_core::model::Issue;

pub fn issue(uuid: &str, project: &str) -> Issue {
    let t = Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap();
    Issue {
        uuid: uuid.into(),
        identifier: format!("DEMO-{uuid}"),
        title: format!("Demo task {uuid}"),
        url: format!("https://linear.app/demo/issue/DEMO-{uuid}"),
        state: "Todo".into(),
        labels: vec!["ai-ready".into()],
        project_id: Some(project.into()),
        project_name: Some("Demo Project".into()),
        assignee_id: Some("user-1".into()),
        parent_identifier: None,
        priority: 0,
        created_at: t,
        updated_at: t,
    }
}
```

`crates/core/tests/common/fake_server.rs`：

```rust
//! In-process MCP server that mimics Linear's tools, connected over an in-memory duplex.
use std::sync::{Arc, Mutex};

use issuerelay_core::linear::mcp::LinearClient;
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, model::ClientConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct FakeLinearServer {
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListIssuesArgs {
    pub assignee: Option<String>,
    pub label: Option<String>,
    pub state: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetIssueArgs {
    pub id: String,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SaveIssueArgs {
    pub id: String,
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetUserArgs {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PageArgs {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TeamArgs {
    pub team: String,
}

pub fn issue_json(n: u32) -> Value {
    json!({
        "id": format!("DEMO-{n}"),
        "uuid": format!("00000000-0000-0000-0000-00000000000{n}"),
        "title": format!("Demo task {n}"),
        "url": format!("https://linear.app/demo/issue/DEMO-{n}"),
        "status": "Todo",
        "labels": ["ai-ready"],
        "project": "Demo Project",
        "projectId": "proj-1",
        "assigneeId": "user-1",
        "parentId": null,
        "priority": {"value": 2, "name": "High"},
        "createdAt": "2026-10-01T08:00:00.000Z",
        "updatedAt": "2026-10-01T09:00:00.000Z"
    })
}

impl FakeLinearServer {
    fn record(&self, tool: &str, args: &impl Serialize) {
        self.calls.lock().unwrap().push((tool.into(), serde_json::to_value(args).unwrap()));
    }

    pub fn calls_to(&self, tool: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(t, _)| t == tool).map(|(_, a)| a.clone()).collect()
    }
}

#[rmcp::tool_router(server_handler)]
impl FakeLinearServer {
    #[rmcp::tool(description = "List issues")]
    async fn list_issues(&self, Parameters(args): Parameters<ListIssuesArgs>) -> String {
        self.record("list_issues", &args);
        if args.cursor.as_deref() == Some("c2") {
            json!({"issues": [issue_json(2)], "hasNextPage": false}).to_string()
        } else {
            json!({"issues": [issue_json(1)], "hasNextPage": true, "cursor": "c2"}).to_string()
        }
    }

    #[rmcp::tool(description = "Get one issue")]
    async fn get_issue(&self, Parameters(args): Parameters<GetIssueArgs>) -> Result<String, String> {
        self.record("get_issue", &args);
        if args.id == "DEMO-1" || args.id.ends_with("0001") {
            Ok(issue_json(1).to_string())
        } else {
            // Linear's real reply for an unknown issue (M0 sample get_issue_missing.json).
            Err(r#"{"error":"invalid_request","message":"Could not find referenced Issue.","status":400}"#.to_string())
        }
    }

    #[rmcp::tool(description = "Create or update an issue")]
    async fn save_issue(&self, Parameters(args): Parameters<SaveIssueArgs>) -> String {
        self.record("save_issue", &args);
        json!({"id": args.id, "status": args.state}).to_string()
    }

    #[rmcp::tool(description = "Get a user")]
    async fn get_user(&self, Parameters(args): Parameters<GetUserArgs>) -> String {
        self.record("get_user", &args);
        json!({"id": "user-1", "name": "Jane Doe", "email": "jane@example.com"}).to_string()
    }

    #[rmcp::tool(description = "List projects")]
    async fn list_projects(&self, Parameters(args): Parameters<PageArgs>) -> String {
        self.record("list_projects", &args);
        json!({"projects": [{"id": "P-DEMO-1", "uuid": "proj-1", "name": "Demo Project"}], "hasNextPage": false}).to_string()
    }

    #[rmcp::tool(description = "List labels")]
    async fn list_issue_labels(&self, Parameters(args): Parameters<PageArgs>) -> String {
        self.record("list_issue_labels", &args);
        json!({"labels": [{"id": "l1", "name": "ai-ready"}, {"id": "l2", "name": "Bug"}], "hasNextPage": false}).to_string()
    }

    #[rmcp::tool(description = "List teams")]
    async fn list_teams(&self, Parameters(args): Parameters<PageArgs>) -> String {
        self.record("list_teams", &args);
        json!({"teams": [{"id": "team-1", "name": "Demo"}], "hasNextPage": false}).to_string()
    }

    #[rmcp::tool(description = "List statuses of a team")]
    async fn list_issue_statuses(&self, Parameters(args): Parameters<TeamArgs>) -> String {
        self.record("list_issue_statuses", &args);
        json!([
            {"id": "s1", "type": "unstarted", "name": "Todo"},
            {"id": "s2", "type": "started", "name": "In Progress"},
            {"id": "s3", "type": "started", "name": "In Testing"},
            {"id": "s4", "type": "completed", "name": "Done"}
        ])
        .to_string()
    }
}

/// A server that only offers `list_issues`, to exercise the missing-tools check.
#[derive(Clone, Default)]
pub struct PartialServer;

#[rmcp::tool_router(server_handler)]
impl PartialServer {
    #[rmcp::tool(description = "List issues")]
    async fn list_issues(&self, Parameters(_args): Parameters<ListIssuesArgs>) -> String {
        json!({"issues": [], "hasNextPage": false}).to_string()
    }
}

pub async fn connect_full(server: FakeLinearServer) -> LinearClient {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ClientConfig::default().serve(client_io).await.expect("client handshake")
}

pub async fn connect_partial() -> LinearClient {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = PartialServer.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ClientConfig::default().serve(client_io).await.expect("client handshake")
}
```

`crates/core/tests/common/fake_linear.rs`：

```rust
//! In-memory LinearApi used by doctor and engine tests.
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use chrono::Duration;
use issuerelay_core::{
    linear::{LinearApi, Project, Team, Viewer},
    model::Issue,
    trigger::same_name,
};

#[derive(Default)]
struct State {
    me: String,
    issues: Vec<Issue>,
    fail_next: Option<String>,
    set_state_calls: Vec<(String, String)>,
    statuses: Vec<String>,
}

#[derive(Clone, Default)]
pub struct FakeLinear {
    state: Arc<Mutex<State>>,
}

impl FakeLinear {
    pub fn new(me: &str) -> Self {
        let fake = Self::default();
        {
            let mut s = fake.state.lock().unwrap();
            s.me = me.into();
            s.statuses = ["Todo", "In Progress", "In Testing", "Done"].map(String::from).to_vec();
        }
        fake
    }

    pub fn upsert(&self, issue: Issue) {
        let mut s = self.state.lock().unwrap();
        s.issues.retain(|i| i.uuid != issue.uuid);
        s.issues.push(issue);
    }

    /// Applies `change` and moves `updated_at` forward by one second, like Linear does.
    pub fn update(&self, uuid: &str, change: impl FnOnce(&mut Issue)) {
        let mut s = self.state.lock().unwrap();
        let issue = s.issues.iter_mut().find(|i| i.uuid == uuid || i.identifier == uuid).expect("unknown issue");
        change(issue);
        issue.updated_at += Duration::seconds(1);
    }

    pub fn get(&self, uuid: &str) -> Issue {
        self.state.lock().unwrap().issues.iter().find(|i| i.uuid == uuid).cloned().expect("unknown issue")
    }

    pub fn fail_next(&self, message: &str) {
        self.state.lock().unwrap().fail_next = Some(message.into());
    }

    pub fn set_statuses(&self, statuses: &[&str]) {
        self.state.lock().unwrap().statuses = statuses.iter().map(|s| s.to_string()).collect();
    }

    pub fn set_state_calls(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().set_state_calls.clone()
    }

    fn check_failure(&self) -> Result<()> {
        match self.state.lock().unwrap().fail_next.take() {
            Some(message) => Err(anyhow!(message)),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl LinearApi for FakeLinear {
    async fn viewer(&self) -> Result<Viewer> {
        self.check_failure()?;
        Ok(Viewer { id: self.state.lock().unwrap().me.clone(), name: "Jane Doe".into() })
    }

    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        self.check_failure()?;
        let s = self.state.lock().unwrap();
        Ok(s.issues
            .iter()
            .filter(|i| i.assignee_id.as_deref() == Some(s.me.as_str()))
            .filter(|i| i.labels.iter().any(|l| same_name(l, label)) && same_name(&i.state, state))
            .cloned()
            .collect())
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        self.check_failure()?;
        Ok(self.state.lock().unwrap().issues.iter().find(|i| i.uuid == id || i.identifier == id).cloned())
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        self.check_failure()?;
        self.state.lock().unwrap().set_state_calls.push((id.into(), state.into()));
        self.update(id, |i| i.state = state.into());
        Ok(())
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        Ok(vec![Project { id: "proj-1".into(), name: "Demo Project".into() }])
    }

    async fn labels(&self) -> Result<Vec<String>> {
        Ok(vec!["ai-ready".into(), "Bug".into()])
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        Ok(vec![Team { id: "team-1".into(), name: "Demo".into() }])
    }

    async fn statuses(&self, _team_id: &str) -> Result<Vec<String>> {
        Ok(self.state.lock().unwrap().statuses.clone())
    }
}
```

注意 `set_state` 里先把 `push` 那句的守卫用完（语句结束即释放），再调用会重新加锁的 `update`；不要把两者写进同一个持锁作用域。

- [ ] **Step 6: 写 MCP 实现的失败测试**

`crates/core/tests/linear_mcp.rs`：

```rust
mod common;

use common::fake_server::{FakeLinearServer, connect_full, connect_partial};
use issuerelay_core::linear::{
    LinearApi,
    mcp::{LinearError, McpLinear},
};

#[tokio::test]
async fn new_rejects_missing_tools() {
    let err = McpLinear::new(connect_partial().await).await.err().expect("should fail");
    match err.downcast_ref::<LinearError>() {
        Some(LinearError::MissingTools(missing)) => {
            assert!(missing.contains(&"get_issue".to_string()));
            assert!(!missing.contains(&"list_issues".to_string()));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn viewer_and_workspace_metadata() {
    let linear = McpLinear::new(connect_full(FakeLinearServer::default()).await).await.unwrap();
    assert_eq!(linear.viewer().await.unwrap().id, "user-1");
    assert_eq!(linear.projects().await.unwrap()[0].id, "proj-1");
    assert_eq!(linear.labels().await.unwrap(), ["ai-ready", "Bug"]);
    let teams = linear.teams().await.unwrap();
    assert_eq!(teams[0].id, "team-1");
    assert_eq!(linear.statuses("team-1").await.unwrap(), ["Todo", "In Progress", "In Testing", "Done"]);
    linear.close().await.unwrap();
}

#[tokio::test]
async fn candidate_issues_follows_pagination() {
    let server = FakeLinearServer::default();
    let linear = McpLinear::new(connect_full(server.clone()).await).await.unwrap();
    let issues = linear.candidate_issues("ai-ready", "Todo").await.unwrap();
    assert_eq!(issues.iter().map(|i| i.identifier.as_str()).collect::<Vec<_>>(), ["DEMO-1", "DEMO-2"]);
    let calls = server.calls_to("list_issues");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["assignee"], "me");
    assert_eq!(calls[0]["label"], "ai-ready");
    assert_eq!(calls[0]["state"], "Todo");
    assert!(calls[0]["fields"].as_array().unwrap().iter().any(|f| f == "uuid"));
    assert!(calls[0]["cursor"].is_null());
    assert_eq!(calls[1]["cursor"], "c2");
}

#[tokio::test]
async fn issue_lookup_and_missing_issue() {
    let linear = McpLinear::new(connect_full(FakeLinearServer::default()).await).await.unwrap();
    let issue = linear.issue("DEMO-1").await.unwrap().unwrap();
    assert_eq!(issue.priority, 2);
    assert_eq!(issue.project_id.as_deref(), Some("proj-1"));
    assert_eq!(issue.labels, ["ai-ready"]);
    assert!(linear.issue("DEMO-404").await.unwrap().is_none());
}

#[tokio::test]
async fn set_state_calls_save_issue() {
    let server = FakeLinearServer::default();
    let linear = McpLinear::new(connect_full(server.clone()).await).await.unwrap();
    linear.set_state("uuid-9", "Done").await.unwrap();
    let calls = server.calls_to("save_issue");
    assert_eq!(calls, [serde_json::json!({"id": "uuid-9", "state": "Done"})]);
}
```

Run: `cargo test -p issuerelay-core --test linear_mcp`
Expected: 编译失败，报 `McpLinear`、`LinearError`、`LinearClient` 未定义

- [ ] **Step 7: 实现 McpLinear**

`crates/core/src/linear/mcp.rs` 整体替换为：

```rust
//! LinearApi backed by Linear's MCP server through rmcp.
use anyhow::{Result, anyhow};
use rmcp::{
    RoleClient,
    model::{CallToolRequestParams, ClientConfig},
    service::RunningService,
};
use serde_json::{Value, json};

use super::{ISSUE_FIELDS, LinearApi, Project, REQUIRED_TOOLS, Team, Viewer, parse};
use crate::model::Issue;

pub type LinearClient = RunningService<RoleClient, ClientConfig>;

const MAX_PAGES: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum LinearError {
    #[error("Linear MCP is missing required tools: {0:?}")]
    MissingTools(Vec<String>),
    #[error("Linear tool `{tool}` failed: {message}")]
    ToolFailed { tool: String, message: String },
}

pub struct McpLinear {
    client: LinearClient,
}

fn is_not_found(error: &anyhow::Error) -> bool {
    // Linear answers an unknown issue with "Could not find referenced Issue." (M0 sample).
    match error.downcast_ref::<LinearError>() {
        Some(LinearError::ToolFailed { message, .. }) => {
            let message = message.to_lowercase();
            message.contains("could not find") || message.contains("not found")
        }
        _ => false,
    }
}

impl McpLinear {
    /// Fails with `LinearError::MissingTools` when Linear no longer offers a tool we rely on.
    pub async fn new(client: LinearClient) -> Result<Self> {
        let offered: Vec<String> = client.list_all_tools().await?.into_iter().map(|t| t.name.to_string()).collect();
        let missing: Vec<String> = REQUIRED_TOOLS
            .iter()
            .filter(|tool| !offered.iter().any(|name| name == *tool))
            .map(|tool| tool.to_string())
            .collect();
        if !missing.is_empty() {
            return Err(LinearError::MissingTools(missing).into());
        }
        Ok(Self { client })
    }

    pub async fn close(self) -> Result<()> {
        self.client.cancel().await?;
        Ok(())
    }

    async fn call(&self, tool: &'static str, args: Value) -> Result<String> {
        let Value::Object(arguments) = args else {
            return Err(anyhow!("tool arguments must be a JSON object"));
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
            .await?;
        let text: String = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.as_str())).collect();
        if result.is_error.unwrap_or(false) {
            return Err(LinearError::ToolFailed { tool: tool.into(), message: text }.into());
        }
        Ok(text)
    }

    async fn all_pages<T: Send>(
        &self,
        tool: &'static str,
        mut args: Value,
        parse_page: fn(&str) -> Result<parse::Page<T>>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut last: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = parse_page(&self.call(tool, args.clone()).await?)?;
            let empty = page.items.is_empty();
            items.extend(page.items);
            match follow(empty, page.next_cursor, last.as_deref()) {
                Some(cursor) => {
                    args["cursor"] = json!(cursor);
                    last = Some(cursor);
                }
                None => return Ok(items),
            }
        }
        Err(anyhow!("{tool} returned more than {MAX_PAGES} pages"))
    }
}

/// The cursor to request next. M0 never observed the last page of `list_issues`, so besides a
/// missing cursor, an empty page or a cursor that did not advance also ends the walk.
fn follow(page_was_empty: bool, next: Option<String>, last: Option<&str>) -> Option<String> {
    next.filter(|cursor| !page_was_empty && Some(cursor.as_str()) != last)
}

#[async_trait::async_trait]
impl LinearApi for McpLinear {
    async fn viewer(&self) -> Result<Viewer> {
        parse::parse_viewer(&self.call("get_user", json!({"query": "me"})).await?)
    }

    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        let args = json!({"assignee": "me", "label": label, "state": state, "limit": 50, "fields": ISSUE_FIELDS});
        self.all_pages("list_issues", args, parse::parse_issue_page).await
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        match self.call("get_issue", json!({"id": id, "fields": ISSUE_FIELDS})).await {
            Ok(text) => parse::parse_single_issue(&text),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        self.call("save_issue", json!({"id": id, "state": state})).await.map(|_| ())
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        self.all_pages("list_projects", json!({"limit": 100}), parse::parse_projects).await
    }

    async fn labels(&self) -> Result<Vec<String>> {
        self.all_pages("list_issue_labels", json!({"limit": 100}), parse::parse_labels).await
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        self.all_pages("list_teams", json!({"limit": 100}), parse::parse_teams).await
    }

    async fn statuses(&self, team_id: &str) -> Result<Vec<String>> {
        parse::parse_statuses(&self.call("list_issue_statuses", json!({"team": team_id})).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::follow;

    #[test]
    fn paging_stops_on_empty_pages_and_repeated_cursors() {
        assert_eq!(follow(false, Some("c2".into()), None).as_deref(), Some("c2"));
        assert_eq!(follow(false, None, Some("c1")), None);
        assert_eq!(follow(true, Some("c3".into()), Some("c2")), None);
        assert_eq!(follow(false, Some("c2".into()), Some("c2")), None);
    }
}
```

- [ ] **Step 8: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core --test linear_mcp && cargo test -p issuerelay-core linear::parse && cargo test -p issuerelay-core linear::mcp`
Expected: PASS（MCP 5 个、解析 7 个、翻页规则 1 个）

- [ ] **Step 9: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add LinearApi with MCP implementation and parsers

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 17: core Linear 授权与连接

> 执行说明（2026-10-11）：已落地代码以修复提交 `fix(core): harden the Linear login listener and keychain handling` 为准：回环端口并发处理连接，只有带本次 `state` 的回调（或不带 `state` 的错误回调）结束等待；钥匙串测试改用内存存储，下面的 `keyring_round_trip` 已删除（真实钥匙串只在 Task 23 验收时用）；MCP 握手 30 秒超时；API key 只走 https；测试数以该提交为准。

**Files:**
- Create: `crates/core/src/linear/auth.rs`
- Modify: `crates/core/src/linear/mod.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 3 验证过的 OAuth 写法；Task 7 的 `paths::APP_ID`；Task 16 的 `LinearClient`
- Produces: `auth::LINEAR_MCP_URL`、`auth::OAUTH_ACCOUNT`、`auth::API_KEY_ACCOUNT`；`auth::KeyringStore`（实现 rmcp 的 `CredentialStore`）；`auth::connect_stored(url) -> Result<Option<LinearClient>>`；`auth::connect_api_key(url, key) -> Result<LinearClient>`；`auth::login(url, open_url: impl Fn(&str)) -> Result<LinearClient>`；`auth::save_api_key(&str)`、`auth::load_api_key() -> Result<Option<String>>`、`auth::clear_credentials()`；`auth::callback_url(request_line: &str, port: u16) -> Option<String>`；`auth::oauth_error(callback_url: &str) -> Option<String>`

- [ ] **Step 1: 加依赖**

```bash
cd crates/core && cargo add keyring url && cd ../..
```

`crates/core/src/linear/mod.rs` 顶部的模块声明改为：

```rust
pub mod auth;
pub mod mcp;
pub mod parse;
```

- [ ] **Step 2: 写失败的测试**

`crates/core/src/linear/auth.rs`：

```rust
//! Connecting to Linear's MCP server: OAuth with a loopback redirect (tokens kept in the
//! system keychain) or a personal API key. Mirrors the flow verified by the M0 probe.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_url_keeps_the_whole_redirect_query() {
        assert_eq!(
            callback_url("GET /callback?code=abc%20def&state=xyz&iss=https%3A%2F%2Fmcp.linear.app HTTP/1.1", 4321).as_deref(),
            Some("http://127.0.0.1:4321/callback?code=abc%20def&state=xyz&iss=https%3A%2F%2Fmcp.linear.app")
        );
        // An OAuth error response is still the redirect; `login` reports it via `oauth_error`.
        assert_eq!(
            callback_url("GET /callback?error=access_denied&state=xyz HTTP/1.1", 4321).as_deref(),
            Some("http://127.0.0.1:4321/callback?error=access_denied&state=xyz")
        );
    }

    #[test]
    fn ignores_other_paths_and_garbage() {
        assert_eq!(callback_url("GET /favicon.ico HTTP/1.1", 4321), None);
        assert_eq!(callback_url("garbage", 4321), None);
        assert_eq!(callback_url("", 4321), None);
    }

    #[test]
    fn oauth_error_reads_error_and_description() {
        assert_eq!(
            oauth_error("http://127.0.0.1:1/callback?error=access_denied&error_description=User%20denied").as_deref(),
            Some("access_denied: User denied")
        );
        assert_eq!(oauth_error("http://127.0.0.1:1/callback?error=access_denied").as_deref(), Some("access_denied"));
        assert_eq!(oauth_error("http://127.0.0.1:1/callback?code=c&state=s"), None);
    }

    #[test]
    fn only_https_authorization_urls_on_the_mcp_host_are_opened() {
        assert!(ensure_expected_authorization_url("https://mcp.linear.app/authorize?x=1", LINEAR_MCP_URL).is_ok());
        assert!(ensure_expected_authorization_url("http://mcp.linear.app/authorize", LINEAR_MCP_URL).is_err());
        assert!(ensure_expected_authorization_url("https://attacker.example/authorize", LINEAR_MCP_URL).is_err());
    }

    #[tokio::test]
    async fn listener_skips_other_paths_and_returns_the_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_callback(&listener).await });
        for target in ["/favicon.ico", "/callback?error=access_denied&state=s"] {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            stream.write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).await.unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
        }
        let url = waiter.await.unwrap().unwrap();
        assert_eq!(url, format!("http://127.0.0.1:{port}/callback?error=access_denied&state=s"));
        assert_eq!(oauth_error(&url).as_deref(), Some("access_denied"));
    }

    /// Touches the real system keychain, so it only runs on request:
    /// `cargo test -p issuerelay-core -- --ignored keyring_round_trip`
    #[test]
    #[ignore]
    fn keyring_round_trip() {
        save_api_key("test-key-not-real").unwrap();
        assert_eq!(load_api_key().unwrap().as_deref(), Some("test-key-not-real"));
        clear_credentials().unwrap();
        assert_eq!(load_api_key().unwrap(), None);
    }
}
```

Run: `cargo test -p issuerelay-core linear::auth`
Expected: 编译失败，报 `callback_url` 等未定义

- [ ] **Step 3: 实现**

在 `crates/core/src/linear/auth.rs` 的测试模块之前加入：

```rust
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rmcp::{
    ServiceExt,
    model::ClientConfig,
    transport::{
        StreamableHttpClientTransport,
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore,
            OAuthState, StoredCredentials,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::mcp::LinearClient;
use crate::paths::APP_ID;

pub const LINEAR_MCP_URL: &str = "https://mcp.linear.app/mcp";
pub const OAUTH_ACCOUNT: &str = "linear-oauth";
pub const API_KEY_ACCOUNT: &str = "linear-api-key";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// A browser may open idle connections to the loopback port; each gets at most this long, so
/// one cannot hold up the real redirect for long.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Keeps rmcp's OAuth credentials (token, refresh token, client id) in the system keychain.
/// Keychain calls can block (or wait on a system prompt), so they run on the blocking pool.
pub struct KeyringStore {
    account: &'static str,
}

impl KeyringStore {
    pub fn oauth() -> Self {
        Self { account: OAUTH_ACCOUNT }
    }
}

fn entry(account: &'static str) -> Result<keyring::Entry, AuthError> {
    keyring::Entry::new(APP_ID, account).map_err(|e| AuthError::CredentialStoreError(e.to_string()))
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, AuthError> + Send + 'static,
) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?
}

#[async_trait::async_trait]
impl CredentialStore for KeyringStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let account = self.account;
        blocking(move || match entry(account)?.get_password() {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| AuthError::CredentialStoreError(e.to_string())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        })
        .await
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials).map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        let account = self.account;
        // Windows Credential Manager caps one secret at 2560 bytes (UTF-16); the token's size there
        // is checked during the Windows acceptance run.
        blocking(move || {
            entry(account)?
                .set_password(&text)
                .map_err(|e| AuthError::CredentialStoreError(e.to_string()))
        })
        .await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let account = self.account;
        blocking(move || match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        })
        .await
    }
}

async fn connect_with(manager: AuthorizationManager, url: &str) -> Result<LinearClient> {
    let transport = StreamableHttpClientTransport::with_client(
        AuthClient::new(reqwest::Client::default(), manager),
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    Ok(ClientConfig::default().serve(transport).await?)
}

/// Uses OAuth credentials saved by an earlier `login`; `None` when there are none.
pub async fn connect_stored(url: &str) -> Result<Option<LinearClient>> {
    let mut manager = AuthorizationManager::new(url).await?;
    manager.set_credential_store(KeyringStore::oauth());
    if !manager.initialize_from_store().await? {
        return Ok(None);
    }
    Ok(Some(connect_with(manager, url).await?))
}

/// Personal API key fallback; rmcp sends it as `Authorization: Bearer <key>`.
pub async fn connect_api_key(url: &str, key: &str) -> Result<LinearClient> {
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(key.to_string()),
    );
    Ok(ClientConfig::default().serve(transport).await?)
}

/// Browser OAuth with a loopback redirect on a random port. `open_url` shows or opens the URL.
pub async fn login(url: &str, open_url: impl Fn(&str)) -> Result<LinearClient> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());
    let mut state = OAuthState::new(url, None).await?;
    if let OAuthState::Unauthorized(manager) = &mut state {
        manager.set_credential_store(KeyringStore::oauth());
    }
    state
        .start_authorization(AuthorizationRequest::new(redirect.as_str()).with_client_name("IssueRelay"))
        .await?;
    let authorization_url = state.get_authorization_url().await?;
    ensure_expected_authorization_url(&authorization_url, url)?;
    open_url(&authorization_url);
    let callback = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(&listener))
        .await
        .context("timed out waiting for the browser authorization")??;
    if let Some(error) = oauth_error(&callback) {
        bail!("Linear authorization failed: {error}");
    }
    // rmcp parses code, state and the RFC 9207 `iss` itself; Linear requires `iss` (M0 finding F5).
    state.handle_callback_url(&callback).await?;
    let manager = state.into_authorization_manager().context("authorization did not complete")?;
    connect_with(manager, url).await
}

pub fn save_api_key(key: &str) -> Result<()> {
    keyring::Entry::new(APP_ID, API_KEY_ACCOUNT)?.set_password(key)?;
    Ok(())
}

pub fn load_api_key() -> Result<Option<String>> {
    match keyring::Entry::new(APP_ID, API_KEY_ACCOUNT)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn clear_credentials() -> Result<()> {
    for account in [OAUTH_ACCOUNT, API_KEY_ACCOUNT] {
        match keyring::Entry::new(APP_ID, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

async fn wait_for_callback(listener: &TcpListener) -> Result<String> {
    let port = listener.local_addr()?.port();
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let Ok(Ok(n)) = tokio::time::timeout(READ_TIMEOUT, socket.read(&mut buf)).await else {
            continue;
        };
        let request = String::from_utf8_lossy(&buf[..n]);
        if let Some(url) = callback_url(request.lines().next().unwrap_or_default(), port) {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<h1>IssueRelay received the browser's reply. You can close this tab and return to IssueRelay.</h1>")
                .await?;
            return Ok(url);
        }
        socket.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n").await?;
    }
}

/// The full redirect URL (code, state, `iss`, or an OAuth error). Any request to /callback counts
/// as the redirect, so an error response ends the wait instead of hanging until the timeout.
pub fn callback_url(request_line: &str, port: u16) -> Option<String> {
    let target = request_line.split_whitespace().nth(1)?;
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).ok()?;
    (url.path() == "/callback").then(|| url.to_string())
}

/// `error` (with `error_description`) from an OAuth error redirect; rmcp does not read them.
pub fn oauth_error(callback_url: &str) -> Option<String> {
    let url = url::Url::parse(callback_url).ok()?;
    let error = url.query_pairs().find(|(k, _)| k == "error")?.1.into_owned();
    match url.query_pairs().find(|(k, _)| k == "error_description") {
        Some((_, description)) => Some(format!("{error}: {description}")),
        None => Some(error),
    }
}

/// The authorization URL comes from remote metadata: only open https URLs on the MCP server's host.
fn ensure_expected_authorization_url(authorization_url: &str, mcp_url: &str) -> Result<()> {
    let authorization = url::Url::parse(authorization_url).context("invalid authorization URL")?;
    let mcp = url::Url::parse(mcp_url).context("invalid MCP URL")?;
    anyhow::ensure!(
        authorization.scheme() == "https" && authorization.host_str() == mcp.host_str(),
        "unexpected authorization URL host: {}",
        authorization.host_str().unwrap_or_default()
    );
    Ok(())
}
```

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core linear::auth`
Expected: PASS（5 个，`keyring_round_trip` 默认忽略）。登录与令牌续期的真实验证在 Task 23。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): connect to Linear MCP via OAuth loopback or API key

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 18: core 环境检查

> 执行说明（2026-10-11）：已落地代码以修复提交 `fix(core): cache readiness per executor and pin the doctor's behavior` 为准：`mcp list` 限时 60 秒、其余 20 秒，三项检查和两个执行者同时进行；按执行者缓存，没跑成的检查 60 秒后重查；`mcp list` 按 Linear 的主机和 `Connected` / `enabled` 状态精确匹配；问题描述带出真实状态；测试数以该提交为准。

**Files:**
- Create: `crates/core/src/doctor.rs`、`crates/core/tests/doctor.rs`
- Create（从 M0 样本复制）：`crates/core/tests/fixtures/claude-mcp-list.txt`、`crates/core/tests/fixtures/codex-mcp-list.txt`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: Task 7 的 `Config`、`ExecutorKind`、`TriggerNames`；Task 8 的 `same_name`；Task 12 的 `resolve_program`；Task 16 的 `LinearApi`、`Viewer`、测试用 `FakeLinear`
- Produces:
  - `doctor::CmdOutput { code, stdout, stderr }`；`trait doctor::CommandRunner`（`run(&Path, &[&str]) -> Result<CmdOutput>`）；`doctor::SystemRunner`
  - `doctor::ExecutorReport { kind, program: Option<PathBuf>, version, logged_in, linear_mcp, problems }`（`ready()`）
  - `doctor::check_executor(kind, Option<PathBuf>, &dyn CommandRunner) -> ExecutorReport`
  - `doctor::parse_claude_auth`、`doctor::parse_codex_login`、`doctor::mcp_list_has_linear`
  - `doctor::WorkspaceReport { label_ok, missing_states }`（`ok()`）；`doctor::check_workspace(&dyn LinearApi, &TriggerNames) -> Result<WorkspaceReport>`
  - `doctor::BindingReport { binding_id, project_name, directory_exists, executor, executor_ready }`；`doctor::check_bindings(&Config, &HashSet<ExecutorKind>) -> Vec<BindingReport>`
  - `doctor::DoctorReport { viewer, linear_error, executors, workspace, bindings }`
  - `trait doctor::ReadinessProbe`（`executor_reports(fresh: bool) -> Vec<ExecutorReport>`）；`doctor::SystemReadiness::new(ttl)`；`doctor::ready_set(&[ExecutorReport]) -> HashSet<ExecutorKind>`

- [ ] **Step 1: 复制 M0 样本**

```bash
cp docs/gates/samples/m0/executors/claude-mcp-list.txt crates/core/tests/fixtures/
cp docs/gates/samples/m0/executors/codex-mcp-list.txt crates/core/tests/fixtures/
```

- [ ] **Step 2: 写失败的单元测试**

`crates/core/src/lib.rs` 加 `pub mod doctor;`。`crates/core/src/doctor.rs`：

```rust
//! Environment checks: are Claude Code / Codex installed, logged in and connected to Linear;
//! does the Linear workspace have the label and statuses IssueRelay relies on; are bindings usable.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    struct FakeRunner(HashMap<String, CmdOutput>);

    #[async_trait::async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, _program: &Path, args: &[&str]) -> Result<CmdOutput> {
            self.0.get(&args.join(" ")).cloned().ok_or_else(|| anyhow::anyhow!("unexpected command {args:?}"))
        }
    }

    fn out(code: i32, stdout: &str) -> CmdOutput {
        CmdOutput { code: Some(code), stdout: stdout.into(), stderr: String::new() }
    }

    fn runner(pairs: &[(&str, CmdOutput)]) -> FakeRunner {
        FakeRunner(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    const CLAUDE_OK: &str = "linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected";
    const CLAUDE_AUTH: &str = "linear-server: https://mcp.linear.app/mcp (HTTP) - ! Needs authentication";

    #[tokio::test]
    async fn claude_ready_when_logged_in_and_linear_connected() {
        let r = runner(&[
            ("--version", out(0, "2.1.285 (Claude Code)\n")),
            ("auth status", out(0, r#"{"loggedIn": true, "authMethod": "claude.ai"}"#)),
            ("mcp list", out(0, CLAUDE_OK)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(report.ready(), "{report:?}");
        assert_eq!(report.version.as_deref(), Some("2.1.285 (Claude Code)"));
        assert!(report.problems.is_empty());
    }

    #[tokio::test]
    async fn claude_needing_linear_auth_is_not_ready() {
        let r = runner(&[
            ("--version", out(0, "2.1.285")),
            ("auth status", out(0, r#"{"loggedIn": true}"#)),
            ("mcp list", out(0, CLAUDE_AUTH)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(!report.ready());
        assert!(report.problems.iter().any(|p| p.contains("Linear MCP")));
    }

    #[tokio::test]
    async fn logged_out_claude_is_not_ready() {
        let r = runner(&[
            ("--version", out(0, "2.1.285")),
            ("auth status", out(0, r#"{"loggedIn": false, "authMethod": "none"}"#)),
            ("mcp list", out(0, CLAUDE_OK)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(!report.logged_in && !report.ready());
    }

    #[tokio::test]
    async fn codex_ready_and_logged_out_cases() {
        let ok = runner(&[
            ("--version", out(0, "codex-cli 0.159.2")),
            ("login status", out(0, "Logged in using ChatGPT")),
            ("mcp list", out(0, "Name    Url                         Status\nlinear  https://mcp.linear.app/mcp  enabled")),
        ]);
        assert!(check_executor(ExecutorKind::Codex, Some(PathBuf::from("codex")), &ok).await.ready());
        // Real codex-cli 0.159.2 prints the login status on stderr (M0 finding F4).
        let stderr_only = runner(&[
            ("--version", out(0, "codex-cli 0.159.2")),
            ("login status", CmdOutput { code: Some(0), stdout: String::new(), stderr: "Logged in using ChatGPT\n".into() }),
            ("mcp list", out(0, "linear  https://mcp.linear.app/mcp  -  enabled  OAuth")),
        ]);
        assert!(check_executor(ExecutorKind::Codex, Some(PathBuf::from("codex")), &stderr_only).await.ready());
        let logged_out = runner(&[
            ("--version", out(0, "codex-cli 0.159.2")),
            ("login status", out(1, "Not logged in")),
            ("mcp list", out(0, "linear  https://mcp.linear.app/mcp  enabled")),
        ]);
        assert!(!check_executor(ExecutorKind::Codex, Some(PathBuf::from("codex")), &logged_out).await.ready());
    }

    #[tokio::test]
    async fn missing_program_is_reported_without_running_anything() {
        let report = check_executor(ExecutorKind::Codex, None, &runner(&[])).await;
        assert!(!report.ready());
        assert!(report.problems[0].contains("not installed"));
    }

    #[test]
    fn mcp_list_parsing() {
        assert!(mcp_list_has_linear(ExecutorKind::Claude, CLAUDE_OK));
        assert!(!mcp_list_has_linear(ExecutorKind::Claude, CLAUDE_AUTH));
        assert!(!mcp_list_has_linear(ExecutorKind::Claude, "other: https://example.com/mcp - ✓ Connected"));
        assert!(!mcp_list_has_linear(ExecutorKind::Codex, "linear  https://mcp.linear.app/mcp  disabled"));
    }

    #[test]
    fn real_m0_mcp_list_samples_show_linear() {
        assert!(mcp_list_has_linear(ExecutorKind::Claude, include_str!("../tests/fixtures/claude-mcp-list.txt")));
        assert!(mcp_list_has_linear(ExecutorKind::Codex, include_str!("../tests/fixtures/codex-mcp-list.txt")));
    }

    #[test]
    fn login_parsing() {
        assert!(parse_claude_auth(r#"{"loggedIn": true}"#));
        assert!(!parse_claude_auth("not json"));
        assert!(parse_codex_login(Some(0), "Logged in using ChatGPT", ""));
        // Real codex-cli 0.159.2 writes the status to stderr (M0 finding F4).
        assert!(parse_codex_login(Some(0), "", "Logged in using ChatGPT"));
        assert!(!parse_codex_login(Some(1), "Not logged in", ""));
        assert!(!parse_codex_login(Some(0), "Not logged in", ""));
    }

    #[test]
    fn binding_checks() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        for (project, path, executor) in [
            ("proj-1", dir.path().to_path_buf(), None),
            ("proj-2", PathBuf::from("/definitely/not/here"), Some(ExecutorKind::Codex)),
        ] {
            cfg.bindings.push(crate::config::Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: path,
                executor,
                permission: None,
                model: None,
                prompt_override: None,
            });
        }
        let ready: HashSet<_> = [ExecutorKind::Claude].into_iter().collect();
        let reports = check_bindings(&cfg, &ready);
        assert!(reports[0].directory_exists && reports[0].executor_ready);
        assert!(!reports[1].directory_exists && !reports[1].executor_ready);
    }
}
```

Run: `cargo test -p issuerelay-core doctor`
Expected: 编译失败，报 `check_executor` 等未定义

- [ ] **Step 3: 实现**

在 `crates/core/src/doctor.rs` 的测试模块之前加入：

```rust
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::{Config, ExecutorKind, TriggerNames};
use crate::executor;
use crate::linear::{LinearApi, Viewer};
use crate::trigger::same_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput>;
}

/// Runs short check commands (`--version`, `auth status`, `mcp list`) with a 20 s limit.
pub struct SystemRunner;

#[async_trait::async_trait]
impl CommandRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
        let child = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(Duration::from_secs(20), child).await??;
        Ok(CmdOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorReport {
    pub kind: ExecutorKind,
    pub program: Option<PathBuf>,
    pub version: Option<String>,
    pub logged_in: bool,
    pub linear_mcp: bool,
    pub problems: Vec<String>,
}

impl ExecutorReport {
    pub fn ready(&self) -> bool {
        self.program.is_some() && self.logged_in && self.linear_mcp
    }
}

pub async fn check_executor(kind: ExecutorKind, program: Option<PathBuf>, runner: &dyn CommandRunner) -> ExecutorReport {
    let name = kind.program_name();
    let mut report = ExecutorReport {
        kind,
        program: program.clone(),
        version: None,
        logged_in: false,
        linear_mcp: false,
        problems: Vec::new(),
    };
    let Some(program) = program else {
        report.problems.push(format!("`{name}` is not installed or not on PATH"));
        return report;
    };
    match runner.run(&program, &["--version"]).await {
        Ok(o) if o.code == Some(0) => report.version = o.stdout.lines().next().map(|l| l.trim().to_string()),
        Ok(o) => report.problems.push(format!("`{name} --version` failed: {}", o.stderr.trim())),
        Err(e) => report.problems.push(format!("`{name} --version` failed: {e}")),
    }
    let login_args: &[&str] = match kind {
        ExecutorKind::Claude => &["auth", "status"],
        ExecutorKind::Codex => &["login", "status"],
    };
    match runner.run(&program, login_args).await {
        Ok(o) => {
            report.logged_in = match kind {
                ExecutorKind::Claude => parse_claude_auth(&o.stdout),
                ExecutorKind::Codex => parse_codex_login(o.code, &o.stdout, &o.stderr),
            }
        }
        Err(e) => report.problems.push(format!("login check failed: {e}")),
    }
    if !report.logged_in {
        report.problems.push(format!("{name} is not logged in"));
    }
    match runner.run(&program, &["mcp", "list"]).await {
        Ok(o) => report.linear_mcp = mcp_list_has_linear(kind, &o.stdout),
        Err(e) => report.problems.push(format!("`{name} mcp list` failed: {e}")),
    }
    if !report.linear_mcp {
        report.problems.push(format!("Linear MCP is not configured or not authorized in {name}"));
    }
    report
}

pub fn parse_claude_auth(stdout: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .and_then(|v| v["loggedIn"].as_bool())
        .unwrap_or(false)
}

pub fn parse_codex_login(code: Option<i32>, stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_lowercase();
    code == Some(0) && text.contains("logged in") && !text.contains("not logged in")
}

/// Formats confirmed in M0 (docs/gates/samples/m0/executors/*-mcp-list.txt).
pub fn mcp_list_has_linear(kind: ExecutorKind, stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let line = line.to_lowercase();
        line.contains("mcp.linear.app")
            && match kind {
                ExecutorKind::Claude => {
                    line.contains("connected")
                        && !line.contains("disconnected")
                        && !line.contains("needs authentication")
                        && !line.contains("failed")
                }
                ExecutorKind::Codex => !line.contains("disabled") && !line.contains("not logged in"),
            }
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReport {
    pub label_ok: bool,
    /// Entries like `Demo: In Testing` (team name and missing status).
    pub missing_states: Vec<String>,
}

impl WorkspaceReport {
    pub fn ok(&self) -> bool {
        self.label_ok && self.missing_states.is_empty()
    }
}

pub async fn check_workspace(api: &dyn LinearApi, names: &TriggerNames) -> Result<WorkspaceReport> {
    let label_ok = api.labels().await?.iter().any(|l| same_name(l, &names.label));
    let mut missing_states = Vec::new();
    for team in api.teams().await? {
        let statuses = api.statuses(&team.id).await?;
        for wanted in [&names.todo, &names.in_progress, &names.in_testing, &names.done] {
            if !statuses.iter().any(|s| same_name(s, wanted)) {
                missing_states.push(format!("{}: {wanted}", team.name));
            }
        }
    }
    Ok(WorkspaceReport { label_ok, missing_states })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingReport {
    pub binding_id: String,
    pub project_name: String,
    pub directory_exists: bool,
    pub executor: ExecutorKind,
    pub executor_ready: bool,
}

pub fn check_bindings(cfg: &Config, ready: &HashSet<ExecutorKind>) -> Vec<BindingReport> {
    cfg.bindings
        .iter()
        .map(|b| {
            let executor = cfg.executor_for(b);
            BindingReport {
                binding_id: b.id.clone(),
                project_name: b.project_name.clone(),
                directory_exists: b.directory.is_dir(),
                executor,
                executor_ready: ready.contains(&executor),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub viewer: Option<Viewer>,
    pub linear_error: Option<String>,
    pub executors: Vec<ExecutorReport>,
    pub workspace: Option<WorkspaceReport>,
    pub bindings: Vec<BindingReport>,
}

#[async_trait::async_trait]
pub trait ReadinessProbe: Send + Sync {
    /// `fresh` bypasses any cache (the doctor command wants current results).
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport>;
}

/// Checks the real `claude` / `codex` on PATH and caches the result for `ttl`.
pub struct SystemReadiness {
    runner: SystemRunner,
    ttl: Duration,
    cache: tokio::sync::Mutex<Option<(Instant, Vec<ExecutorReport>)>>,
}

impl SystemReadiness {
    pub fn new(ttl: Duration) -> Self {
        Self { runner: SystemRunner, ttl, cache: tokio::sync::Mutex::new(None) }
    }
}

#[async_trait::async_trait]
impl ReadinessProbe for SystemReadiness {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        let mut cache = self.cache.lock().await;
        if !fresh
            && let Some((at, reports)) = cache.as_ref()
            && at.elapsed() < self.ttl
        {
            return reports.clone();
        }
        let mut reports = Vec::new();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            reports.push(check_executor(kind, executor::resolve_program(kind).ok(), &self.runner).await);
        }
        *cache = Some((Instant::now(), reports.clone()));
        reports
    }
}

pub fn ready_set(reports: &[ExecutorReport]) -> HashSet<ExecutorKind> {
    reports.iter().filter(|r| r.ready()).map(|r| r.kind).collect()
}
```

Run: `cargo test -p issuerelay-core doctor`
Expected: PASS（9 个）。`real_m0_mcp_list_samples_show_linear` 失败说明 M0 输出格式不同：按样本改 `mcp_list_has_linear` 与对应测试。

- [ ] **Step 4: 写工作区检查的集成测试**

`crates/core/tests/doctor.rs`：

```rust
mod common;

use common::fake_linear::FakeLinear;
use issuerelay_core::{config::TriggerNames, doctor::check_workspace};

#[tokio::test]
async fn workspace_with_all_names_is_ok() {
    let report = check_workspace(&FakeLinear::new("user-1"), &TriggerNames::default()).await.unwrap();
    assert!(report.ok(), "{report:?}");
}

#[tokio::test]
async fn missing_status_and_label_are_reported() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["Todo", "In Progress", "Done"]);
    let names = TriggerNames { label: "robot".into(), ..TriggerNames::default() };
    let report = check_workspace(&linear, &names).await.unwrap();
    assert!(!report.label_ok);
    assert_eq!(report.missing_states, ["Demo: In Testing"]);
}
```

Run: `cargo test -p issuerelay-core --test doctor`
Expected: PASS（2 个）

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add executor, workspace and binding checks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 19: core 引擎

> 执行说明（2026-10-12）：下面的代码块是派发时的版本，已落地代码以 `fix(core): keep one run per directory and never overwrite a user's stop` 及其后续提交为准：登记运行时在运行集合锁里检查目录和总数上限，需求换了项目就改派后下一轮再调度；轮询和启动前的写入都在读状态的同一把存储锁里完成，用户的停止和重试不会被覆盖，一条需求不会有两条活动记录；派活中途暂停、改绑定或关停都会拦下启动；一条任务出错不影响其他任务；运行槽位总会释放；摘要和原因都经 `clean_text` 清理，没有结果行时保存 AI 的最后输出。测试数以提交为准。

**Files:**
- Create: `crates/core/src/engine.rs`、`crates/core/tests/common/engine_kit.rs`、`crates/core/tests/engine.rs`
- Modify: `crates/core/src/lib.rs`、`crates/core/tests/common/mod.rs`、`crates/core/Cargo.toml`

**Interfaces:**
- Consumes: Task 7–18 的全部公开接口
- Produces:
  - `trait engine::JobRunner`（`run(spec, kind, log_path: PathBuf, limits, cancel) -> RunOutput`）；`engine::ProcessJobRunner`
  - `engine::EngineDeps { config_path, store, linear: Arc<dyn LinearApi>, readiness: Arc<dyn ReadinessProbe>, job_runner: Arc<dyn JobRunner>, logs_dir }`
  - `engine::StatusSnapshot { paused, linear_ok, viewer, last_poll_at, last_error, running, queued, waiting }`
  - `engine::decide(&ExitKind, state: Option<&str>, report: Option<&Report>, &TriggerNames) -> (JobState, Option<String>)`
  - `engine::Engine`（`Clone`）：`new(EngineDeps)`、`status()`、`pause()`、`resume()`、`poll_now()`、`tick()`、`poll_once()`、`dispatch() -> Result<Vec<i64>>`、`wait_idle()`、`list_jobs(usize)`、`log_tail(i64, usize)`、`bindings()`、`add_binding(Binding)`、`remove_binding(&str) -> Result<bool>`、`cancel_job(i64)`、`retry_job(i64)`、`mark_done(i64)`（只给 M2 面板用）、`doctor() -> DoctorReport`、`run_forever(CancellationToken)`
  - 测试共用：`common::engine_kit::{FakeReadiness, Script, ScriptedRunner, Harness}`

- [ ] **Step 1: 加依赖**

```bash
cd crates/core && cargo add tracing && cd ../..
```

- [ ] **Step 2: 写测试工具**

`crates/core/tests/common/mod.rs` 的模块声明改为：

```rust
#![allow(dead_code)]
pub mod engine_kit;
pub mod fake_linear;
pub mod fake_server;
```

`crates/core/tests/common/engine_kit.rs`：

```rust
//! An Engine wired to fakes: in-memory Linear, scripted runs, always-ready executors.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use issuerelay_core::{
    config::{Binding, Config, ExecutorKind},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{Engine, EngineDeps, JobRunner},
    executor::CommandSpec,
    runner::{ExitKind, RunLimits, RunOutput},
    store::Store,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::fake_linear::FakeLinear;

pub struct FakeReadiness;

#[async_trait::async_trait]
impl ReadinessProbe for FakeReadiness {
    async fn executor_reports(&self, _fresh: bool) -> Vec<ExecutorReport> {
        [ExecutorKind::Claude, ExecutorKind::Codex]
            .into_iter()
            .map(|kind| ExecutorReport {
                kind,
                program: Some(PathBuf::from(format!("/fake/{}", kind.as_str()))),
                version: Some("test".into()),
                logged_in: true,
                linear_mcp: true,
                problems: Vec::new(),
            })
            .collect()
    }
}

/// What a scripted run does: optionally move the issue's status (as the AI would), then exit.
#[derive(Clone)]
pub struct Script {
    pub set_state: Option<&'static str>,
    pub exit: ExitKind,
    pub final_text: Option<String>,
}

impl Script {
    pub fn success() -> Self {
        Self {
            set_state: Some("In Testing"),
            exit: ExitKind::Exited(Some(0)),
            final_text: Some(r#"{"result":"in_testing","summary":"pushed to test"}"#.into()),
        }
    }

    pub fn exit_code(code: i32) -> Self {
        Self { set_state: None, exit: ExitKind::Exited(Some(code)), final_text: None }
    }

    pub fn needs_info() -> Self {
        Self {
            set_state: Some("Todo"),
            exit: ExitKind::Exited(Some(0)),
            final_text: Some(r#"{"result":"needs_info","summary":"which page?"}"#.into()),
        }
    }
}

pub struct ScriptedRunner {
    pub linear: FakeLinear,
    pub scripts: Mutex<HashMap<String, Script>>,
    pub started: Mutex<Vec<String>>,
    pub specs: Mutex<Vec<CommandSpec>>,
    /// Runs wait for a permit before finishing; tests add permits to let them end.
    pub gate: Arc<Semaphore>,
}

impl ScriptedRunner {
    pub fn new(linear: FakeLinear, gated: bool) -> Self {
        Self {
            linear,
            scripts: Mutex::new(HashMap::new()),
            started: Mutex::new(Vec::new()),
            specs: Mutex::new(Vec::new()),
            gate: Arc::new(Semaphore::new(if gated { 0 } else { 1_000 })),
        }
    }

    pub fn script(&self, identifier: &str, script: Script) {
        self.scripts.lock().unwrap().insert(identifier.into(), script);
    }

    pub fn started(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }
}

fn identifier_in(prompt: &str) -> String {
    prompt
        .split("Linear issue: ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_default()
        .to_string()
}

#[async_trait::async_trait]
impl JobRunner for ScriptedRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        _kind: ExecutorKind,
        log_path: PathBuf,
        _limits: RunLimits,
        cancel: CancellationToken,
    ) -> RunOutput {
        let id = identifier_in(&spec.stdin);
        self.started.lock().unwrap().push(id.clone());
        self.specs.lock().unwrap().push(spec);
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        std::fs::write(&log_path, "{\"type\":\"result\"}\n").unwrap();
        let script = self.scripts.lock().unwrap().get(&id).cloned().unwrap_or_else(Script::success);
        if let Some(state) = script.set_state {
            self.linear.update(&id, |issue| issue.state = state.into());
        }
        tokio::select! {
            permit = self.gate.acquire() => permit.expect("gate closed").forget(),
            _ = cancel.cancelled() => return RunOutput { exit: ExitKind::Canceled, final_text: None },
        }
        RunOutput { exit: script.exit, final_text: script.final_text }
    }
}

pub struct Harness {
    pub engine: Engine,
    pub linear: FakeLinear,
    pub runner: Arc<ScriptedRunner>,
    pub root: tempfile::TempDir,
}

impl Harness {
    pub fn new(max_concurrent: usize, gated: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.settings.max_concurrent = max_concurrent;
        cfg.save(&config_path).unwrap();
        let linear = FakeLinear::new("user-1");
        let runner = Arc::new(ScriptedRunner::new(linear.clone(), gated));
        let engine = Engine::new(EngineDeps {
            config_path,
            store: Store::open_in_memory().unwrap(),
            linear: Arc::new(linear.clone()),
            readiness: Arc::new(FakeReadiness),
            job_runner: runner.clone(),
            logs_dir: root.path().join("logs"),
        })
        .unwrap();
        Self { engine, linear, runner, root }
    }

    /// Creates `<root>/<dir_name>` and binds `project` to it with the default executor.
    pub fn bind(&self, project: &str, dir_name: &str) -> PathBuf {
        let dir = self.root.path().join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        self.engine
            .add_binding(Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: dir.clone(),
                executor: None,
                permission: None,
                model: None,
                prompt_override: None,
            })
            .unwrap();
        dir
    }

    pub async fn settle(&self) {
        self.engine.wait_idle().await;
    }

    pub fn latest(&self, uuid: &str) -> issuerelay_core::model::Job {
        self.engine
            .list_jobs(100)
            .unwrap()
            .into_iter()
            .find(|job| job.issue_uuid == uuid)
            .expect("no job for issue")
    }
}
```

- [ ] **Step 3: 写失败的引擎测试**

`crates/core/src/lib.rs` 加 `pub mod engine;`，并新建只含一行文档注释的 `crates/core/src/engine.rs`。`crates/core/tests/engine.rs`：

```rust
mod common;

use std::sync::Arc;

use common::{
    engine_kit::{FakeReadiness, Harness, Script, ScriptedRunner},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    config::Config,
    engine::{Engine, EngineDeps},
    model::JobState,
    store::{NewJob, Store},
};

#[tokio::test]
async fn bound_issue_runs_to_awaiting_acceptance() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::AwaitingAcceptance);
    assert_eq!(job.summary.as_deref(), Some("pushed to test"));
    assert_eq!(job.verified_updated_at, Some(h.linear.get("1").updated_at));
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    let spec = h.runner.specs.lock().unwrap()[0].clone();
    assert_eq!(spec.program, std::path::PathBuf::from("/fake/claude"));
    assert!(spec.args.contains(&"bypassPermissions".to_string()));
    assert!(spec.stdin.contains("DEMO-1") && spec.stdin.contains("Never set the status to \"Done\""));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 1, "a finished issue must not start again");
}

#[tokio::test]
async fn unbound_issue_waits_then_runs_after_binding() {
    let h = Harness::new(2, false);
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let waiting = h.latest("1");
    assert_eq!(waiting.state, JobState::WaitingBinding);
    assert!(waiting.reason.unwrap().contains("not bound"));
    assert!(h.runner.started().is_empty());
    h.bind("proj-1", "repo-a");
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.engine.list_jobs(100).unwrap().len(), 1, "the waiting job is reused");
}

#[tokio::test]
async fn same_directory_runs_one_at_a_time() {
    let h = Harness::new(5, true);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!((h.engine.status().running, h.engine.status().queued), (1, 1));
    h.runner.gate.add_permits(1);
    h.settle().await;
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn global_cap_limits_parallel_runs() {
    let h = Harness::new(2, true);
    for (project, dir) in [("proj-1", "a"), ("proj-2", "b"), ("proj-3", "c")] {
        h.bind(project, dir);
    }
    for (uuid, project) in [("1", "proj-1"), ("2", "proj-2"), ("3", "proj-3")] {
        h.linear.upsert(issue(uuid, project));
    }
    h.engine.tick().await.unwrap();
    assert_eq!((h.engine.status().running, h.engine.status().queued), (2, 1));
    h.runner.gate.add_permits(3);
    h.settle().await;
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 3);
}

#[tokio::test]
async fn queued_issue_losing_label_is_canceled() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("2").state, JobState::Queued);
    h.linear.update("2", |i| i.labels.clear());
    h.engine.poll_once().await.unwrap();
    let job = h.latest("2");
    assert_eq!(job.state, JobState::Canceled);
    assert!(job.reason.unwrap().contains("no longer matches"));
    h.runner.gate.add_permits(1);
    h.settle().await;
}

#[tokio::test]
async fn pre_dispatch_recheck_cancels_changed_issue() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
    h.linear.update("1", |i| i.assignee_id = Some("someone-else".into()));
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Canceled);
    assert!(job.reason.unwrap().contains("changed before"));
    assert!(h.runner.started().is_empty());
}

#[tokio::test]
async fn failed_issue_requeues_only_after_linear_update() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(3));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let failed = h.latest("1");
    assert_eq!(failed.state, JobState::Failed);
    assert!(failed.reason.unwrap().contains("code 3"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 1, "unchanged issue must not run again");
    h.linear.update("1", |i| i.title = "Demo task 1 (clarified)".into());
    h.runner.script("DEMO-1", Script::success());
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 2);
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.engine.list_jobs(100).unwrap().len(), 2);
}

#[tokio::test]
async fn needs_info_is_recorded_and_not_rerun() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::needs_info());
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::NeedsInfo);
    assert_eq!(job.summary.as_deref(), Some("which page?"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 1);
}

#[tokio::test]
async fn mark_done_updates_linear_and_job() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    h.engine.mark_done(job.id).await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
    assert_eq!(h.linear.get("1").state, "Done");
    assert_eq!(h.linear.set_state_calls(), [("1".to_string(), "Done".to_string())]);
    assert!(h.engine.mark_done(job.id).await.is_err(), "only awaiting-acceptance jobs can be marked done");
}

#[tokio::test]
async fn cancel_running_job() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Running);
    h.engine.cancel_job(job.id).unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::Canceled);
}

#[tokio::test]
async fn retry_requeues_failed_job() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    h.runner.script("DEMO-1", Script::success());
    h.engine.retry_job(job.id).unwrap();
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.engine.list_jobs(100).unwrap().len(), 1);
    assert!(h.engine.retry_job(job.id).is_err());
}

#[tokio::test]
async fn retry_rejects_an_older_run() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let first = h.latest("1");
    // Someone edits the issue in Linear, so it comes back as a new run, which fails too.
    h.linear.update("1", |i| i.title.push_str(" (edited)"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let second = h.latest("1");
    assert_ne!(second.id, first.id);
    assert!(h.engine.retry_job(first.id).is_err());
    h.engine.retry_job(second.id).unwrap();
}

#[tokio::test]
async fn paused_engine_does_nothing_and_pause_is_saved() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.pause().unwrap();
    h.engine.tick().await.unwrap();
    assert!(h.engine.list_jobs(10).unwrap().is_empty());
    assert!(h.engine.status().paused);
    assert!(Config::load(&h.root.path().join("config.toml")).unwrap().settings.paused);
    h.engine.resume().unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 1);
}

#[tokio::test]
async fn poll_error_is_reported_without_state_changes() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.fail_next("Linear is down");
    assert!(h.engine.tick().await.is_err());
    let status = h.engine.status();
    assert!(!status.linear_ok);
    assert!(status.last_error.unwrap().contains("Linear is down"));
    assert!(h.engine.list_jobs(10).unwrap().is_empty());
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert!(h.engine.status().linear_ok);
}

#[tokio::test]
async fn interrupted_runs_fail_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("state.db");
    {
        let store = Store::open(&db).unwrap();
        let i = issue("1", "proj-1");
        let job = store
            .insert(NewJob { issue: &i, binding: None, executor: None, state: JobState::Queued, reason: None })
            .unwrap();
        store.mark_started(job.id, &root.path().join("1.jsonl")).unwrap();
    }
    let linear = FakeLinear::new("user-1");
    let engine = Engine::new(EngineDeps {
        config_path: root.path().join("config.toml"),
        store: Store::open(&db).unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.path().join("logs"),
    })
    .unwrap();
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::Failed);
    assert!(job.reason.unwrap().contains("interrupted"));
}

#[tokio::test]
async fn log_tail_returns_last_lines() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.engine.log_tail(h.latest("1").id, 10).unwrap(), ["{\"type\":\"result\"}"]);
}
```

Run: `cargo test -p issuerelay-core --test engine`
Expected: 编译失败，报 `Engine`、`EngineDeps`、`JobRunner` 未定义

- [ ] **Step 4: 实现引擎**

`crates/core/src/engine.rs` 整体替换为：

```rust
//! The engine: polls Linear, records and schedules jobs, starts executor runs, verifies the
//! result in Linear and keeps the status the panel, CLI and MCP read. Every lock is taken and
//! released within one statement; no lock is held across `.await`.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::config::{Binding, Config, ExecutorKind, TriggerNames};
use crate::doctor::{self, DoctorReport, ExecutorReport, ReadinessProbe};
use crate::executor::{self, CommandSpec, outcome::{self, Report}};
use crate::linear::{LinearApi, Viewer};
use crate::model::{Issue, Job, JobState};
use crate::prompt::{self, PromptInput};
use crate::runner::{self, ExitKind, RunLimits, RunOutput};
use crate::scheduler::{self, Candidate};
use crate::store::{NewJob, Store};
use crate::trigger::{self, Eligibility, WaitReason, same_name};

#[async_trait::async_trait]
pub trait JobRunner: Send + Sync {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
    ) -> RunOutput;
}

/// Runs real executor processes.
pub struct ProcessJobRunner;

#[async_trait::async_trait]
impl JobRunner for ProcessJobRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
    ) -> RunOutput {
        runner::run(spec, kind, &log_path, limits, cancel).await
    }
}

pub struct EngineDeps {
    pub config_path: PathBuf,
    pub store: Store,
    pub linear: Arc<dyn LinearApi>,
    pub readiness: Arc<dyn ReadinessProbe>,
    pub job_runner: Arc<dyn JobRunner>,
    pub logs_dir: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub paused: bool,
    pub linear_ok: bool,
    pub viewer: Option<Viewer>,
    pub last_poll_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub running: usize,
    pub queued: usize,
    pub waiting: usize,
}

#[derive(Debug, Clone, Default)]
struct PollState {
    at: Option<DateTime<Utc>>,
    error: Option<String>,
    linear_ok: bool,
}

struct RunningJob {
    directory: PathBuf,
    cancel: CancellationToken,
}

struct Inner {
    config_path: PathBuf,
    config: RwLock<Config>,
    store: Mutex<Store>,
    linear: Arc<dyn LinearApi>,
    readiness: Arc<dyn ReadinessProbe>,
    job_runner: Arc<dyn JobRunner>,
    logs_dir: PathBuf,
    running: Mutex<HashMap<i64, RunningJob>>,
    viewer: Mutex<Option<Viewer>>,
    poll: Mutex<PollState>,
    poll_now: Notify,
    idle: Notify,
}

/// Cheap to clone; every clone drives the same engine.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

/// One spelling per folder for the one-run-per-directory rule: resolves symlinks, `..` and,
/// on case-insensitive volumes, letter case. Falls back to the path as written.
fn canonical_dir(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// Maps a finished run to a job state. Only a clean exit whose issue reached the testing
/// status counts as success; the AI's own report alone never does.
pub fn decide(
    exit: &ExitKind,
    state: Option<&str>,
    report: Option<&Report>,
    names: &TriggerNames,
) -> (JobState, Option<String>) {
    let is = |wanted: &str| state.is_some_and(|s| same_name(s, wanted));
    match exit {
        ExitKind::Canceled => (JobState::Canceled, Some("stopped by the user".into())),
        ExitKind::TimedOut => (JobState::Failed, Some("run exceeded the time limit".into())),
        ExitKind::Stalled => (JobState::Failed, Some("no output for too long; the run was stopped".into())),
        ExitKind::SpawnFailed(e) => (JobState::Failed, Some(format!("could not start the executor: {e}"))),
        ExitKind::Exited(code) if *code != Some(0) => {
            let code = code.map_or_else(|| "unknown".to_string(), |c| c.to_string());
            (JobState::Failed, Some(format!("executor exited with code {code}")))
        }
        ExitKind::Exited(_) if is(&names.in_testing) => (JobState::AwaitingAcceptance, None),
        ExitKind::Exited(_) if is(&names.done) => (JobState::Done, None),
        ExitKind::Exited(_) if matches!(report, Some(Report::NeedsInfo(_))) && is(&names.todo) => {
            (JobState::NeedsInfo, None)
        }
        ExitKind::Exited(_) => (
            JobState::Failed,
            Some(format!(
                "run finished but the issue status is {:?}, not {:?}",
                state.unwrap_or("unknown"),
                names.in_testing
            )),
        ),
    }
}

impl Engine {
    pub fn new(deps: EngineDeps) -> Result<Self> {
        let config = Config::load(&deps.config_path)?;
        let recovered = deps.store.recover_interrupted()?;
        if recovered > 0 {
            tracing::warn!(recovered, "runs interrupted by the previous shutdown were marked failed");
        }
        Ok(Self {
            inner: Arc::new(Inner {
                config_path: deps.config_path,
                config: RwLock::new(config),
                store: Mutex::new(deps.store),
                linear: deps.linear,
                readiness: deps.readiness,
                job_runner: deps.job_runner,
                logs_dir: deps.logs_dir,
                running: Mutex::new(HashMap::new()),
                viewer: Mutex::new(None),
                poll: Mutex::new(PollState::default()),
                poll_now: Notify::new(),
                idle: Notify::new(),
            }),
        })
    }

    fn config(&self) -> Config {
        self.inner.config.read().expect("config lock").clone()
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.inner.store.lock().expect("store lock")
    }

    fn count(&self, state: JobState) -> usize {
        self.store().list_by_state(&[state]).map(|jobs| jobs.len()).unwrap_or(0)
    }

    pub fn status(&self) -> StatusSnapshot {
        let paused = self.config().settings.paused;
        let poll = self.inner.poll.lock().expect("poll lock").clone();
        let viewer = self.inner.viewer.lock().expect("viewer lock").clone();
        let running = self.inner.running.lock().expect("running lock").len();
        StatusSnapshot {
            paused,
            linear_ok: poll.linear_ok,
            viewer,
            last_poll_at: poll.at,
            last_error: poll.error,
            running,
            queued: self.count(JobState::Queued),
            waiting: self.count(JobState::WaitingBinding),
        }
    }

    pub fn pause(&self) -> Result<()> {
        self.update_config(|c| c.settings.paused = true)
    }

    pub fn resume(&self) -> Result<()> {
        self.update_config(|c| c.settings.paused = false)?;
        self.poll_now();
        Ok(())
    }

    /// Wakes the background loop for an immediate check.
    pub fn poll_now(&self) {
        self.inner.poll_now.notify_one();
    }

    fn update_config(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut current = self.inner.config.write().expect("config lock");
        let mut next = current.clone();
        change(&mut next);
        next.validate()?;
        next.save(&self.inner.config_path)?;
        *current = next;
        Ok(())
    }

    pub fn bindings(&self) -> Vec<Binding> {
        self.config().bindings
    }

    /// Adds the binding, replacing any existing one for the same project or id.
    pub fn add_binding(&self, binding: Binding) -> Result<()> {
        if binding.project_id.trim().is_empty() {
            bail!("project id is required");
        }
        self.update_config(|c| {
            c.bindings.retain(|b| b.project_id != binding.project_id && b.id != binding.id);
            c.bindings.push(binding);
        })
    }

    pub fn remove_binding(&self, id: &str) -> Result<bool> {
        let mut removed = false;
        self.update_config(|c| {
            let before = c.bindings.len();
            c.bindings.retain(|b| b.id != id);
            removed = c.bindings.len() != before;
        })?;
        Ok(removed)
    }

    pub fn list_jobs(&self, limit: usize) -> Result<Vec<Job>> {
        self.store().list_recent(limit)
    }

    pub fn log_tail(&self, job_id: i64, lines: usize) -> Result<Vec<String>> {
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        let Some(path) = job.log_path else { return Ok(Vec::new()) };
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let all: Vec<&str> = text.lines().collect();
        Ok(all[all.len().saturating_sub(lines)..].iter().map(|line| line.to_string()).collect())
    }

    async fn viewer(&self) -> Result<Viewer> {
        let cached = self.inner.viewer.lock().expect("viewer lock").clone();
        if let Some(viewer) = cached {
            return Ok(viewer);
        }
        let viewer = self.inner.linear.viewer().await?;
        *self.inner.viewer.lock().expect("viewer lock") = Some(viewer.clone());
        Ok(viewer)
    }

    async fn readiness(&self) -> (HashSet<ExecutorKind>, Vec<ExecutorReport>) {
        let reports = self.inner.readiness.executor_reports(false).await;
        (doctor::ready_set(&reports), reports)
    }

    /// One cycle of the background loop: poll Linear, then start whatever may start.
    pub async fn tick(&self) -> Result<()> {
        self.poll_once().await?;
        self.dispatch().await?;
        Ok(())
    }

    pub async fn poll_once(&self) -> Result<()> {
        let cfg = self.config();
        if cfg.settings.paused {
            return Ok(());
        }
        let result = self.poll_inner(&cfg).await;
        let state = PollState {
            at: Some(Utc::now()),
            error: result.as_ref().err().map(|e| format!("{e:#}")),
            linear_ok: result.is_ok(),
        };
        *self.inner.poll.lock().expect("poll lock") = state;
        result
    }

    async fn poll_inner(&self, cfg: &Config) -> Result<()> {
        let names = &cfg.settings.trigger;
        let me = self.viewer().await?;
        let issues = self.inner.linear.candidate_issues(&names.label, &names.todo).await?;
        let (ready, _) = self.readiness().await;
        let mut seen = HashSet::new();
        for issue in issues.iter().filter(|i| trigger::is_candidate(i, names, &me.id)) {
            seen.insert(issue.uuid.clone());
            let latest = self.store().latest_for_issue(&issue.uuid)?;
            let eligibility = trigger::evaluate(issue, cfg, &me.id, &ready);
            match latest {
                Some(job) if matches!(job.state, JobState::Queued | JobState::WaitingBinding) => {
                    self.reassess(&job, cfg, eligibility)?;
                }
                Some(job) if job.state == JobState::Running => {}
                latest => {
                    if trigger::should_enqueue(latest.as_ref(), issue) {
                        self.enqueue(issue, cfg, eligibility)?;
                    }
                }
            }
        }
        let pending = self.store().list_by_state(&[JobState::Queued, JobState::WaitingBinding])?;
        for job in pending.iter().filter(|j| !seen.contains(&j.issue_uuid)) {
            self.store().set_state(
                job.id,
                JobState::Canceled,
                Some("no longer matches the trigger (label, assignee or status changed)"),
            )?;
        }
        Ok(())
    }

    fn enqueue(&self, issue: &Issue, cfg: &Config, eligibility: Eligibility) -> Result<()> {
        let new = match &eligibility {
            Eligibility::Eligible(binding) => NewJob {
                issue,
                binding: Some(binding),
                executor: Some(cfg.executor_for(binding)),
                state: JobState::Queued,
                reason: None,
            },
            Eligibility::Waiting(reason) => NewJob {
                issue,
                binding: None,
                executor: None,
                state: JobState::WaitingBinding,
                reason: Some(reason.describe()),
            },
            Eligibility::NotCandidate => return Ok(()),
        };
        self.store().insert(new)?;
        Ok(())
    }

    fn reassess(&self, job: &Job, cfg: &Config, eligibility: Eligibility) -> Result<()> {
        match eligibility {
            Eligibility::Eligible(binding) => {
                let unchanged = job.state == JobState::Queued
                    && job.binding_id.as_deref() == Some(binding.id.as_str())
                    && job.directory.as_deref() == Some(binding.directory.as_path());
                if !unchanged {
                    self.store().assign(job.id, JobState::Queued, Some(&binding), Some(cfg.executor_for(&binding)), None)?;
                }
            }
            Eligibility::Waiting(reason) => {
                self.store().assign(job.id, JobState::WaitingBinding, None, None, Some(&reason.describe()))?;
            }
            Eligibility::NotCandidate => {}
        }
        Ok(())
    }

    pub async fn dispatch(&self) -> Result<Vec<i64>> {
        let cfg = self.config();
        if cfg.settings.paused {
            return Ok(Vec::new());
        }
        let queued = self.store().list_by_state(&[JobState::Queued])?;
        let candidates: Vec<Candidate> = queued
            .iter()
            .filter_map(|job| {
                // The binding's current directory in one canonical spelling, so two bindings that
                // name the same folder differently never run at the same time.
                let binding = cfg.bindings.iter().find(|b| job.binding_id.as_deref() == Some(b.id.as_str()))?;
                Some(Candidate {
                    job_id: job.id,
                    directory: canonical_dir(&binding.directory),
                    priority: job.priority,
                    issue_created_at: job.issue_created_at,
                })
            })
            .collect();
        let (running_dirs, running_count) = {
            let running = self.inner.running.lock().expect("running lock");
            (running.values().map(|r| r.directory.clone()).collect::<HashSet<_>>(), running.len())
        };
        let picks = scheduler::pick(&candidates, &running_dirs, running_count, cfg.settings.max_concurrent);
        if picks.is_empty() {
            return Ok(Vec::new());
        }
        let me = self.viewer().await?;
        let (ready, reports) = self.readiness().await;
        let mut started = Vec::new();
        for id in picks {
            let job = self.store().get(id)?;
            let Some(job) = job else { continue };
            if self.start_job(&cfg, &me, &ready, &reports, job).await? {
                started.push(id);
            }
        }
        Ok(started)
    }

    async fn start_job(
        &self,
        cfg: &Config,
        me: &Viewer,
        ready: &HashSet<ExecutorKind>,
        reports: &[ExecutorReport],
        job: Job,
    ) -> Result<bool> {
        // The issue may have changed while queued; re-read it right before starting.
        let current = self.inner.linear.issue(&job.issue_uuid).await?;
        let Some(issue) = current.filter(|i| trigger::is_candidate(i, &cfg.settings.trigger, &me.id)) else {
            self.store().set_state(job.id, JobState::Canceled, Some("issue changed before the run started"))?;
            return Ok(false);
        };
        let binding = match trigger::evaluate(&issue, cfg, &me.id, ready) {
            Eligibility::Eligible(binding) => binding,
            Eligibility::Waiting(reason) => {
                self.store().assign(job.id, JobState::WaitingBinding, None, None, Some(&reason.describe()))?;
                return Ok(false);
            }
            Eligibility::NotCandidate => return Ok(false),
        };
        let kind = cfg.executor_for(&binding);
        let program = reports.iter().find(|r| r.kind == kind).and_then(|r| r.program.clone());
        let Some(program) = program else {
            let reason = WaitReason::ExecutorNotReady(kind).describe();
            self.store().assign(job.id, JobState::WaitingBinding, None, None, Some(&reason))?;
            return Ok(false);
        };
        let language = cfg.settings.resolved_comment_language();
        // A blank override falls back to the global template instead of silently dropping it.
        let template = binding
            .prompt_override
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| cfg.settings.prompt_template.clone());
        let prompt_text = prompt::render(&PromptInput {
            identifier: &issue.identifier,
            url: &issue.url,
            names: &cfg.settings.trigger,
            comment_language: &language,
            template: &template,
        });
        let spec = executor::build(kind, program, cfg.permission_for(&binding), binding.model.as_deref(), &binding.directory, prompt_text);
        let log_path = self.inner.logs_dir.join(format!("{}.jsonl", job.id));
        // Register the run before marking it started, so a stop request in between finds the token.
        let cancel = CancellationToken::new();
        self.inner
            .running
            .lock()
            .expect("running lock")
            .insert(job.id, RunningJob { directory: canonical_dir(&binding.directory), cancel: cancel.clone() });
        let started = self.store().mark_started(job.id, &log_path);
        if !matches!(started, Ok(true)) {
            // The user stopped it while the issue was being re-read, or the store failed.
            self.inner.running.lock().expect("running lock").remove(&job.id);
            return started.map(|_| false);
        }
        let limits = RunLimits {
            total: Duration::from_secs(cfg.settings.run_timeout_mins.saturating_mul(60)),
            stall: Duration::from_secs(cfg.settings.stall_timeout_mins.saturating_mul(60)),
        };
        let engine = self.clone();
        let job_id = job.id;
        let issue_uuid = job.issue_uuid.clone();
        tokio::spawn(async move {
            let output = engine.inner.job_runner.run(spec, kind, log_path, limits, cancel).await;
            if let Err(e) = engine.finish_job(job_id, &issue_uuid, output).await {
                tracing::error!(job_id, error = %e, "could not record the run result");
            }
        });
        Ok(true)
    }

    async fn finish_job(&self, job_id: i64, issue_uuid: &str, output: RunOutput) -> Result<()> {
        let names = self.config().settings.trigger;
        let report = output.final_text.as_deref().and_then(outcome::parse_report);
        let verified = self.inner.linear.issue(issue_uuid).await;
        let (state, reason, verified_at) = match verified {
            Ok(issue) => {
                let (state, reason) = decide(&output.exit, issue.as_ref().map(|i| i.state.as_str()), report.as_ref(), &names);
                (state, reason, issue.map(|i| i.updated_at))
            }
            Err(e) => {
                let state = if output.exit == ExitKind::Canceled { JobState::Canceled } else { JobState::Failed };
                (state, Some(format!("could not verify the result in Linear: {e:#}")), None)
            }
        };
        let summary = report.as_ref().map(|r| r.summary().to_string());
        let recorded = self.store().mark_finished(job_id, state, reason.as_deref(), summary.as_deref(), verified_at);
        self.inner.running.lock().expect("running lock").remove(&job_id);
        self.inner.idle.notify_waiters();
        self.poll_now();
        recorded
    }

    /// Resolves once no run is in progress (used by tests and by shutdown).
    pub async fn wait_idle(&self) {
        loop {
            let notified = self.inner.idle.notified();
            if self.inner.running.lock().expect("running lock").is_empty() {
                return;
            }
            notified.await;
        }
    }

    pub fn cancel_job(&self, job_id: i64) -> Result<()> {
        let running = self.inner.running.lock().expect("running lock").get(&job_id).map(|r| r.cancel.clone());
        if let Some(token) = running {
            token.cancel();
            return Ok(());
        }
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        if !matches!(job.state, JobState::Queued | JobState::WaitingBinding) {
            bail!("job {job_id} is {} and cannot be stopped", job.state.as_str());
        }
        let waiting = [JobState::Queued, JobState::WaitingBinding];
        if self.store().set_state_if(job_id, &waiting, JobState::Canceled, Some("stopped by the user"))? {
            return Ok(());
        }
        // It started between the two reads: stop the run itself.
        let running = self.inner.running.lock().expect("running lock").get(&job_id).map(|r| r.cancel.clone());
        match running {
            Some(token) => {
                token.cancel();
                Ok(())
            }
            None => bail!("job {job_id} changed state while being stopped; try again"),
        }
    }

    pub fn retry_job(&self, job_id: i64) -> Result<()> {
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        if !matches!(job.state, JobState::Failed | JobState::Canceled | JobState::NeedsInfo) {
            bail!("job {job_id} is {}; only failed, stopped or needs-info runs can be retried", job.state.as_str());
        }
        // Re-running an older row while a newer run of the issue exists would make two active rows.
        let latest = self.store().latest_for_issue(&job.issue_uuid)?;
        if latest.map(|j| j.id) != Some(job_id) {
            bail!("job {job_id} is not the latest run of {}; retry that one instead", job.issue_identifier);
        }
        self.store().requeue(job_id)?;
        self.poll_now();
        Ok(())
    }

    /// Marks an accepted run Done in Linear. Only the desktop panel calls this (spec 5.6);
    /// IPC, CLI and MCP never expose it.
    pub async fn mark_done(&self, job_id: i64) -> Result<()> {
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        if job.state != JobState::AwaitingAcceptance {
            bail!("job {job_id} is not awaiting acceptance");
        }
        let done = self.config().settings.trigger.done;
        self.inner.linear.set_state(&job.issue_uuid, &done).await?;
        self.store().set_state(job_id, JobState::Done, None)
    }

    pub async fn doctor(&self) -> DoctorReport {
        let cfg = self.config();
        let executors = self.inner.readiness.executor_reports(true).await;
        let ready = doctor::ready_set(&executors);
        let (viewer, linear_error) = match self.inner.linear.viewer().await {
            Ok(viewer) => (Some(viewer), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let workspace = if viewer.is_some() {
            doctor::check_workspace(self.inner.linear.as_ref(), &cfg.settings.trigger).await.ok()
        } else {
            None
        };
        DoctorReport { viewer, linear_error, executors, workspace, bindings: doctor::check_bindings(&cfg, &ready) }
    }

    /// The background loop. Returns after `shutdown` is cancelled and running jobs were told to stop.
    pub async fn run_forever(&self, shutdown: CancellationToken) {
        loop {
            if let Err(e) = self.tick().await {
                tracing::warn!(error = %format!("{e:#}"), "check failed");
            }
            let interval = Duration::from_secs(self.config().settings.poll_interval_secs.max(5));
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(interval) => {}
                _ = self.inner.poll_now.notified() => {}
            }
        }
        let running = self.inner.running.lock().expect("running lock");
        for job in running.values() {
            job.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> TriggerNames {
        TriggerNames::default()
    }

    #[test]
    fn decide_maps_outcomes() {
        let ok = ExitKind::Exited(Some(0));
        assert_eq!(decide(&ok, Some("In Testing"), None, &names()).0, JobState::AwaitingAcceptance);
        assert_eq!(decide(&ok, Some("done"), None, &names()).0, JobState::Done);
        let info = Report::NeedsInfo("q".into());
        assert_eq!(decide(&ok, Some("Todo"), Some(&info), &names()).0, JobState::NeedsInfo);
        assert_eq!(decide(&ok, Some("In Progress"), Some(&info), &names()).0, JobState::Failed);
        let claimed = Report::InTesting("done".into());
        let (state, reason) = decide(&ok, Some("In Progress"), Some(&claimed), &names());
        assert_eq!(state, JobState::Failed, "the AI's report alone is not success");
        assert!(reason.unwrap().contains("In Progress"));
        assert_eq!(decide(&ExitKind::Exited(Some(2)), Some("In Testing"), None, &names()).0, JobState::Failed);
        assert_eq!(decide(&ExitKind::Exited(None), None, None, &names()).1.unwrap(), "executor exited with code unknown");
        assert_eq!(decide(&ExitKind::Canceled, Some("In Testing"), None, &names()).0, JobState::Canceled);
        assert_eq!(decide(&ExitKind::Stalled, None, None, &names()).0, JobState::Failed);
        assert_eq!(decide(&ExitKind::TimedOut, None, None, &names()).0, JobState::Failed);
        assert_eq!(decide(&ExitKind::SpawnFailed("x".into()), None, None, &names()).0, JobState::Failed);
    }
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core --test engine && cargo test -p issuerelay-core engine::tests`
Expected: PASS（集成 16 个、单元 1 个）

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add engine with polling, scheduling, verification and recovery

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 20: core 本机 IPC

> 执行说明（2026-10-12）：落地时另按控制方补充要求改了 `crates/core/src/paths.rs`（端点只由数据目录决定，套接字路径上限 103 字节），一次失败的 accept / connect 不会停掉服务，`doctor` 的客户端超时为 180 秒，其余 30 秒；测试数以提交为准。

**Files:**
- Create: `crates/core/src/ipc/mod.rs`、`crates/core/src/ipc/protocol.rs`、`crates/core/src/ipc/server.rs`、`crates/core/src/ipc/client.rs`、`crates/core/tests/ipc.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: Task 7 的 `IpcEndpoint`、`Binding`；Task 19 的 `Engine`、测试用 `Harness`
- Produces:
  - `ipc::protocol::{METHODS, Request, Response, RpcError, METHOD_NOT_FOUND, PARSE_ERROR, SERVER_ERROR}`；`METHODS` 为 `status`、`pause`、`resume`、`poll_now`、`list_jobs`、`doctor`、`bindings.list`、`bindings.add`、`bindings.remove`、`logs`
  - `ipc::server::{IpcServer, AlreadyRunning, handle}`：`IpcServer::bind(&IpcEndpoint) -> Result<IpcServer>`、`run(self, Engine, CancellationToken) -> Result<()>`
  - `ipc::client::{call, NotRunning}`：`call(&IpcEndpoint, method: &str, params: Value) -> Result<Value>`

- [ ] **Step 1: 写协议模块与单元测试**

`crates/core/src/lib.rs` 加 `pub mod ipc;`。`crates/core/src/ipc/mod.rs`：

```rust
//! Local JSON-RPC between the engine host and the CLI / MCP control server: one JSON object
//! per line over a Unix domain socket (macOS) or a named pipe (Windows).
pub mod client;
pub mod protocol;
pub mod server;
```

`crates/core/src/ipc/protocol.rs`：

```rust
//! Request / response types and the method table.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Methods the engine answers over IPC. Marking an issue Done, retrying and cancelling are
/// deliberately absent: only the desktop panel may do those (spec 5.6, 10.3).
pub const METHODS: [&str; 10] = [
    "status",
    "pause",
    "resume",
    "poll_now",
    "list_jobs",
    "doctor",
    "bindings.list",
    "bindings.add",
    "bindings.remove",
    "logs",
];

pub const PARSE_ERROR: i64 = -32700;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const SERVER_ERROR: i64 = -32000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, method: method.into(), params }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: None, error: Some(RpcError { code, message: message.into() }) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_table_never_exposes_panel_only_actions() {
        for forbidden in ["mark_done", "retry_job", "cancel_job"] {
            assert!(!METHODS.contains(&forbidden), "{forbidden} must stay panel-only");
        }
    }

    #[test]
    fn wire_format() {
        let req = Request::new(7, "status", serde_json::json!({}));
        let text = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), req);
        let ok = serde_json::to_value(Response::ok(7, serde_json::json!(1))).unwrap();
        assert!(ok.get("error").is_none());
        let err = serde_json::to_value(Response::err(7, METHOD_NOT_FOUND, "nope")).unwrap();
        assert!(err.get("result").is_none());
        assert_eq!(err["error"]["code"], -32601);
        let bare: Request = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"status"}"#).unwrap();
        assert!(bare.params.is_null());
    }
}
```

`crates/core/src/ipc/server.rs` 与 `crates/core/src/ipc/client.rs` 先各只写一行文档注释。

Run: `cargo test -p issuerelay-core ipc::protocol`
Expected: PASS（2 个）

- [ ] **Step 2: 写失败的集成测试**

`crates/core/tests/ipc.rs`：

```rust
mod common;

use common::{engine_kit::Harness, issue};
use issuerelay_core::{
    ipc::{
        client::{self, NotRunning},
        server::{AlreadyRunning, IpcServer},
    },
    paths::{AppPaths, IpcEndpoint},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

async fn start(h: &Harness) -> (IpcEndpoint, CancellationToken) {
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    (endpoint, token)
}

#[tokio::test]
async fn status_pause_and_bindings_round_trip() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    assert_eq!(client::call(&endpoint, "status", json!({})).await.unwrap()["paused"], false);
    client::call(&endpoint, "pause", json!({})).await.unwrap();
    assert_eq!(client::call(&endpoint, "status", json!({})).await.unwrap()["paused"], true);
    let dir = h.root.path().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    let binding = json!({"id": "proj-9", "project_id": "proj-9", "project_name": "Demo Project", "directory": dir});
    client::call(&endpoint, "bindings.add", json!({"binding": binding})).await.unwrap();
    let list = client::call(&endpoint, "bindings.list", json!({})).await.unwrap();
    assert_eq!(list[0]["project_id"], "proj-9");
    let removed = client::call(&endpoint, "bindings.remove", json!({"id": "proj-9"})).await.unwrap();
    assert_eq!(removed["removed"], true);
    token.cancel();
}

#[tokio::test]
async fn jobs_and_logs_over_ipc() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let jobs = client::call(&endpoint, "list_jobs", json!({"limit": 5})).await.unwrap();
    assert_eq!(jobs[0]["state"], "awaiting_acceptance");
    let logs = client::call(&endpoint, "logs", json!({"job_id": jobs[0]["id"], "lines": 10})).await.unwrap();
    assert_eq!(logs, json!(["{\"type\":\"result\"}"]));
    token.cancel();
}

#[tokio::test]
async fn unknown_and_panel_only_methods_are_rejected() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    for method in ["mark_done", "retry_job", "cancel_job", "nope"] {
        let err = client::call(&endpoint, method, json!({"job_id": 1})).await.unwrap_err();
        assert!(err.to_string().contains("unknown method"), "{method}: {err}");
    }
    token.cancel();
}

#[tokio::test]
async fn second_server_refuses_to_start() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let err = IpcServer::bind(&endpoint).await.err().expect("second bind must fail");
    assert!(err.downcast_ref::<AlreadyRunning>().is_some(), "{err}");
    token.cancel();
}

#[tokio::test]
async fn client_reports_not_running() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = AppPaths::under(root.path()).ipc_endpoint();
    let err = client::call(&endpoint, "status", json!({})).await.unwrap_err();
    assert!(err.downcast_ref::<NotRunning>().is_some(), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn stale_socket_file_is_replaced() {
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else { panic!("expected a unix socket") };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"stale").unwrap();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    assert_eq!(client::call(&endpoint, "status", json!({})).await.unwrap()["paused"], false);
    token.cancel();
}
```

Run: `cargo test -p issuerelay-core --test ipc`
Expected: 编译失败，报 `IpcServer`、`client::call` 等未定义

- [ ] **Step 3: 实现服务端**

`crates/core/src/ipc/server.rs` 整体替换为：

```rust
//! IPC server: binds the endpoint (refusing to start when another host owns it) and answers
//! requests by calling the engine.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::protocol::{METHOD_NOT_FOUND, PARSE_ERROR, Request, Response, SERVER_ERROR};
use crate::config::Binding;
use crate::engine::Engine;
use crate::paths::IpcEndpoint;

#[derive(Debug, thiserror::Error)]
#[error("another IssueRelay instance is already running")]
pub struct AlreadyRunning;

#[derive(Debug, thiserror::Error)]
#[error("unknown method `{0}`")]
struct UnknownMethod(String);

pub async fn handle(engine: &Engine, request: Request) -> Response {
    match dispatch(engine, &request.method, &request.params).await {
        Ok(value) => Response::ok(request.id, value),
        Err(e) if e.is::<UnknownMethod>() => Response::err(request.id, METHOD_NOT_FOUND, e.to_string()),
        Err(e) => Response::err(request.id, SERVER_ERROR, format!("{e:#}")),
    }
}

async fn dispatch(engine: &Engine, method: &str, params: &Value) -> Result<Value> {
    Ok(match method {
        "status" => serde_json::to_value(engine.status())?,
        "pause" => {
            engine.pause()?;
            json!({"paused": true})
        }
        "resume" => {
            engine.resume()?;
            json!({"paused": false})
        }
        "poll_now" => {
            engine.poll_now();
            json!({"ok": true})
        }
        "list_jobs" => {
            let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20).min(500) as usize;
            serde_json::to_value(engine.list_jobs(limit)?)?
        }
        "doctor" => serde_json::to_value(engine.doctor().await)?,
        "bindings.list" => serde_json::to_value(engine.bindings())?,
        "bindings.add" => {
            let raw = params.get("binding").cloned().unwrap_or(Value::Null);
            let binding: Binding = serde_json::from_value(raw).context("invalid `binding`")?;
            engine.add_binding(binding)?;
            json!({"ok": true})
        }
        "bindings.remove" => {
            let id = params.get("id").and_then(Value::as_str).context("missing `id`")?;
            json!({"removed": engine.remove_binding(id)?})
        }
        "logs" => {
            let job_id = params.get("job_id").and_then(Value::as_i64).context("missing `job_id`")?;
            let lines = params.get("lines").and_then(Value::as_u64).unwrap_or(50).min(5_000) as usize;
            serde_json::to_value(engine.log_tail(job_id, lines)?)?
        }
        other => return Err(UnknownMethod(other.to_string()).into()),
    })
}

pub struct IpcServer {
    listener: Listener,
}

enum Listener {
    #[cfg(unix)]
    Unix { listener: tokio::net::UnixListener, path: std::path::PathBuf },
    #[cfg(windows)]
    Pipe { first: tokio::net::windows::named_pipe::NamedPipeServer, name: String },
}

impl IpcServer {
    /// Binds the endpoint; fails with `AlreadyRunning` when another host owns it.
    pub async fn bind(endpoint: &IpcEndpoint) -> Result<Self> {
        match endpoint {
            #[cfg(unix)]
            IpcEndpoint::Unix(path) => Ok(Self {
                listener: Listener::Unix { listener: bind_unix(path).await?, path: path.clone() },
            }),
            #[cfg(windows)]
            IpcEndpoint::Pipe(name) => Ok(Self { listener: Listener::Pipe { first: bind_pipe(name)?, name: name.clone() } }),
            other => bail!("IPC endpoint {other:?} is not supported on this platform"),
        }
    }

    pub async fn run(self, engine: Engine, shutdown: CancellationToken) -> Result<()> {
        match self.listener {
            #[cfg(unix)]
            Listener::Unix { listener, path } => {
                loop {
                    let accepted = tokio::select! {
                        _ = shutdown.cancelled() => None,
                        accepted = listener.accept() => Some(accepted),
                    };
                    let Some(accepted) = accepted else { break };
                    let (stream, _) = accepted?;
                    let engine = engine.clone();
                    tokio::spawn(async move {
                        let _ = serve_connection(stream, engine).await;
                    });
                }
                let _ = std::fs::remove_file(&path);
                Ok(())
            }
            #[cfg(windows)]
            Listener::Pipe { first, name } => {
                use tokio::net::windows::named_pipe::ServerOptions;
                let mut server = first;
                loop {
                    let connected = tokio::select! {
                        _ = shutdown.cancelled() => None,
                        result = server.connect() => Some(result),
                    };
                    let Some(result) = connected else { break };
                    result?;
                    let next = ServerOptions::new().reject_remote_clients(true).create(&name)?;
                    let client = std::mem::replace(&mut server, next);
                    let engine = engine.clone();
                    tokio::spawn(async move {
                        let _ = serve_connection(client, engine).await;
                    });
                }
                Ok(())
            }
        }
    }
}

#[cfg(unix)]
async fn bind_unix(path: &std::path::Path) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().context("socket path has no parent directory")?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if path.exists() {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(AlreadyRunning.into());
        }
        std::fs::remove_file(path)?;
    }
    Ok(tokio::net::UnixListener::bind(path)?)
}

#[cfg(windows)]
fn bind_pipe(name: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    const ERROR_ACCESS_DENIED: i32 = 5;
    match ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(name) {
        Ok(server) => Ok(server),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Err(AlreadyRunning.into()),
        Err(e) => Err(e.into()),
    }
}

async fn serve_connection<S>(stream: S, engine: Engine) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(&engine, request).await,
            Err(e) => Response::err(0, PARSE_ERROR, format!("invalid request: {e}")),
        };
        let mut text = serde_json::to_string(&response)?;
        text.push('\n');
        write.write_all(text.as_bytes()).await?;
    }
    Ok(())
}
```

- [ ] **Step 4: 实现客户端**

`crates/core/src/ipc/client.rs` 整体替换为：

```rust
//! IPC client used by the CLI and the MCP control server.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::protocol::{Request, Response};
use crate::paths::IpcEndpoint;

#[derive(Debug, thiserror::Error)]
#[error("IssueRelay is not running")]
pub struct NotRunning;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Sends one request and returns its `result`, or the error message the engine reported.
pub async fn call(endpoint: &IpcEndpoint, method: &str, params: Value) -> Result<Value> {
    let mut line = serde_json::to_string(&Request::new(1, method, params))?;
    line.push('\n');
    let reply = tokio::time::timeout(TIMEOUT, exchange(endpoint, line))
        .await
        .context("IssueRelay did not answer in time")??;
    let response: Response = serde_json::from_str(&reply).context("invalid response from IssueRelay")?;
    if let Some(error) = response.error {
        bail!("{}", error.message);
    }
    Ok(response.result.unwrap_or(Value::Null))
}

async fn exchange(endpoint: &IpcEndpoint, line: String) -> Result<String> {
    match endpoint {
        #[cfg(unix)]
        IpcEndpoint::Unix(path) => {
            let stream = tokio::net::UnixStream::connect(path).await.map_err(|_| NotRunning)?;
            roundtrip(stream, line).await
        }
        #[cfg(windows)]
        IpcEndpoint::Pipe(name) => roundtrip(open_pipe(name).await?, line).await,
        other => bail!("IPC endpoint {other:?} is not supported on this platform"),
    }
}

#[cfg(windows)]
async fn open_pipe(name: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..50 {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(NotRunning.into()),
            Err(e) => return Err(e.into()),
        }
    }
    bail!("IssueRelay's pipe stayed busy")
}

async fn roundtrip<S>(stream: S, line: String) -> Result<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    write.write_all(line.as_bytes()).await?;
    let mut lines = BufReader::new(read).lines();
    lines.next_line().await?.context("IssueRelay closed the connection without answering")
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p issuerelay-core --test ipc && cargo test -p issuerelay-core ipc::protocol`
Expected: PASS（macOS 集成 6 个、Windows 5 个；协议 2 个）

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(core): add local JSON-RPC over unix socket / named pipe

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 21: 命令行与 MCP 控制服务

**Files:**
- Create: `crates/cli/Cargo.toml`、`crates/cli/src/lib.rs`、`crates/cli/src/main.rs`、`crates/cli/src/format.rs`、`crates/cli/src/mcp.rs`、`crates/cli/tests/mcp_tools.rs`

**Interfaces:**
- Consumes: Task 7 的 `AppPaths`、`IpcEndpoint`、`Binding`、`ExecutorKind::parse`、`PermissionProfile::parse`；Task 20 的 `ipc::client::call`
- Produces: 二进制 `issuerelay`（子命令 `status`、`pause`、`resume`、`poll`、`doctor`、`jobs [--limit N]`、`logs <JOB_ID> [--lines N]`、`bind list|add|remove`、`mcp`，全局 `--json`）；库 `issuerelay_cli::{format::human, mcp::{ControlServer, serve}}`；MCP 工具恰好为 `status`、`list_jobs`、`pause`、`resume`、`poll_now`

- [ ] **Step 1: 建 crate**

```bash
mkdir -p crates/cli/src crates/cli/tests
cat > crates/cli/Cargo.toml <<'EOF'
[package]
name = "issuerelay-cli"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
name = "issuerelay_cli"
path = "src/lib.rs"

[[bin]]
name = "issuerelay"
path = "src/main.rs"

[dependencies]
EOF
cd crates/cli
cargo add issuerelay-core --path ../core
cargo add anyhow serde_json schemars
cargo add serde --features derive
cargo add clap --features derive
cargo add tokio --features full
cargo add rmcp@3.5 --features server,macros,transport-io,schemars
cargo add --dev rmcp@3.5 --features client,transport-async-rw
cargo add --dev tempfile
cd ../..
```

`crates/cli/src/lib.rs`：

```rust
//! Command-line front end and MCP control server for IssueRelay. Both talk to the running
//! engine host over local IPC and never act on their own.
pub mod format;
pub mod mcp;
```

- [ ] **Step 2: 写输出格式的失败测试**

`crates/cli/src/format.rs`：

```rust
//! Human-readable output for CLI commands; `--json` prints the raw result instead.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_lines() {
        let v = json!({"paused": false, "linear_ok": true, "viewer": {"id": "u", "name": "Jane Doe"},
            "last_poll_at": "2026-10-01T08:00:00Z", "last_error": null, "running": 1, "queued": 2, "waiting": 0});
        let text = human("status", &v);
        assert!(text.contains("Monitoring: on"));
        assert!(text.contains("Linear: connected as Jane Doe"));
        assert!(text.contains("Running: 1  Queued: 2  Waiting for binding: 0"));
        let down = json!({"paused": true, "linear_ok": false, "last_error": "timeout", "running": 0, "queued": 0, "waiting": 0});
        let text = human("status", &down);
        assert!(text.contains("Monitoring: paused") && text.contains("not connected (timeout)"));
    }

    #[test]
    fn jobs_table() {
        assert_eq!(human("list_jobs", &json!([])), "No runs yet.");
        let text = human("list_jobs", &json!([{"id": 3, "issue_identifier": "DEMO-1", "state": "running", "title": "Demo task"}]));
        assert!(text.starts_with("#3") && text.contains("DEMO-1") && text.contains("running") && text.ends_with("Demo task"));
    }

    #[test]
    fn doctor_summary() {
        let v = json!({
            "viewer": {"id": "u", "name": "Jane Doe"},
            "linear_error": null,
            "executors": [
                {"kind": "claude", "program": "/x/claude", "logged_in": true, "linear_mcp": true, "problems": []},
                {"kind": "codex", "program": null, "logged_in": false, "linear_mcp": false, "problems": ["`codex` is not installed or not on PATH"]}
            ],
            "workspace": {"label_ok": true, "missing_states": ["Demo: In Testing"]},
            "bindings": [{"binding_id": "p", "project_name": "Demo Project", "directory_exists": true, "executor": "claude", "executor_ready": true}]
        });
        let text = human("doctor", &v);
        assert!(text.contains("Linear: connected as Jane Doe"));
        assert!(text.contains("claude: ready"));
        assert!(text.contains("codex: not ready (`codex` is not installed or not on PATH)"));
        assert!(text.contains("missing statuses: Demo: In Testing"));
        assert!(text.contains("Binding Demo Project -> claude: ok"));
    }

    #[test]
    fn bindings_logs_and_simple_messages() {
        assert!(human("bindings.list", &json!([])).contains("issuerelay bind add"));
        let text = human("bindings.list", &json!([{"id": "p1", "project_name": "Demo Project", "directory": "demo"}]));
        assert!(text.contains("p1") && text.contains("Demo Project") && text.contains("[default]"));
        assert_eq!(human("logs", &json!(["a", "b"])), "a\nb");
        assert_eq!(human("pause", &json!({"paused": true})), "Monitoring paused (running jobs continue).");
        assert_eq!(human("bindings.remove", &json!({"removed": false})), "No binding with that id.");
    }
}
```

Run: `cargo test -p issuerelay-cli format`
Expected: 编译失败，报 `human` 未定义

- [ ] **Step 3: 实现输出格式**

在 `crates/cli/src/format.rs` 的测试模块之前加入：

```rust
use serde_json::Value;

pub fn human(method: &str, value: &Value) -> String {
    match method {
        "status" => status(value),
        "list_jobs" => jobs(value),
        "doctor" => doctor(value),
        "bindings.list" => bindings(value),
        "logs" => strings(value).join("\n"),
        "pause" => "Monitoring paused (running jobs continue).".into(),
        "resume" => "Monitoring resumed.".into(),
        "poll_now" => "Checking Linear now.".into(),
        "bindings.add" => "Binding saved.".into(),
        "bindings.remove" if value["removed"] == true => "Binding removed.".into(),
        "bindings.remove" => "No binding with that id.".into(),
        _ => value.to_string(),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("-")
}

fn strings(value: &Value) -> Vec<&str> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str).collect()
}

fn status(v: &Value) -> String {
    let monitoring = if v["paused"] == true { "paused" } else { "on" };
    let linear = if v["linear_ok"] == true {
        format!("connected as {}", text(&v["viewer"], "name"))
    } else {
        format!("not connected ({})", text(v, "last_error"))
    };
    format!(
        "Monitoring: {monitoring}\nLinear: {linear}\nRunning: {}  Queued: {}  Waiting for binding: {}\nLast check: {}",
        v["running"],
        v["queued"],
        v["waiting"],
        text(v, "last_poll_at")
    )
}

fn jobs(v: &Value) -> String {
    let items = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return "No runs yet.".into();
    }
    items
        .iter()
        .map(|j| {
            format!(
                "#{:<5} {:<12} {:<20} {}",
                j["id"].as_i64().unwrap_or_default(),
                text(j, "issue_identifier"),
                text(j, "state"),
                text(j, "title")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn doctor(v: &Value) -> String {
    let mut out = Vec::new();
    match v.get("viewer").filter(|x| !x.is_null()) {
        Some(viewer) => out.push(format!("Linear: connected as {}", text(viewer, "name"))),
        None => out.push(format!("Linear: not connected ({})", text(v, "linear_error"))),
    }
    for e in v["executors"].as_array().into_iter().flatten() {
        let ready = e["program"].is_string() && e["logged_in"] == true && e["linear_mcp"] == true;
        let problems = strings(&e["problems"]);
        let detail = if ready {
            "ready".to_string()
        } else if problems.is_empty() {
            "not ready".to_string()
        } else {
            format!("not ready ({})", problems.join("; "))
        };
        out.push(format!("{}: {detail}", text(e, "kind")));
    }
    if let Some(ws) = v.get("workspace").filter(|x| !x.is_null()) {
        let label = if ws["label_ok"] == true { "label ok" } else { "label missing" };
        let missing = strings(&ws["missing_states"]);
        out.push(if missing.is_empty() {
            format!("Workspace: {label}, statuses ok")
        } else {
            format!("Workspace: {label}, missing statuses: {}", missing.join(", "))
        });
    }
    for b in v["bindings"].as_array().into_iter().flatten() {
        let ok = b["directory_exists"] == true && b["executor_ready"] == true;
        out.push(format!(
            "Binding {} -> {}: {}",
            text(b, "project_name"),
            text(b, "executor"),
            if ok { "ok" } else { "not ready" }
        ));
    }
    out.join("\n")
}

fn bindings(v: &Value) -> String {
    let items = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return "No bindings. Add one with `issuerelay bind add`.".into();
    }
    items
        .iter()
        .map(|b| {
            let executor = b.get("executor").and_then(Value::as_str).unwrap_or("default");
            format!("{}  {}  ->  {}  [{executor}]", text(b, "id"), text(b, "project_name"), text(b, "directory"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

Run: `cargo test -p issuerelay-cli format`
Expected: PASS（4 个）

- [ ] **Step 4: 写 MCP 控制服务的失败测试**

`crates/cli/src/mcp.rs` 先只写一行 `//! MCP control server.`。`crates/cli/tests/mcp_tools.rs`：

```rust
use issuerelay_cli::mcp::ControlServer;
use issuerelay_core::paths::AppPaths;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientConfig},
};

#[tokio::test]
async fn exposes_only_safe_control_tools_and_reports_engine_down() {
    let root = tempfile::tempdir().unwrap();
    let server = ControlServer::new(AppPaths::under(root.path()).ipc_endpoint());
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = ClientConfig::default().serve(client_io).await.unwrap();
    let mut names: Vec<String> = client.list_all_tools().await.unwrap().into_iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["list_jobs", "pause", "poll_now", "resume", "status"]);
    let result = client.call_tool(CallToolRequestParams::new("status")).await.unwrap();
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.starts_with("error:") && text.contains("not running"), "{text}");
    client.cancel().await.unwrap();
}
```

Run: `cargo test -p issuerelay-cli --test mcp_tools`
Expected: 编译失败，报 `ControlServer` 未定义

- [ ] **Step 5: 实现 MCP 控制服务**

`crates/cli/src/mcp.rs` 整体替换为：

```rust
//! MCP control server (`issuerelay mcp`, stdio) for Claude Code / Codex. It only reads status
//! and pauses / resumes monitoring; it can never mark an issue Done or retry a run.
use issuerelay_core::{ipc::client, paths::IpcEndpoint};
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, transport::stdio};
use serde_json::json;

#[derive(Clone)]
pub struct ControlServer {
    endpoint: IpcEndpoint,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct JobsArgs {
    /// How many recent runs to return (default 20).
    pub limit: Option<u64>,
}

impl ControlServer {
    pub fn new(endpoint: IpcEndpoint) -> Self {
        Self { endpoint }
    }

    async fn forward(&self, method: &str, params: serde_json::Value) -> String {
        match client::call(&self.endpoint, method, params).await {
            Ok(value) => value.to_string(),
            Err(e) => format!("error: {e}"),
        }
    }
}

#[rmcp::tool_router(server_handler)]
impl ControlServer {
    #[rmcp::tool(description = "Show IssueRelay monitoring status")]
    async fn status(&self) -> String {
        self.forward("status", json!({})).await
    }

    #[rmcp::tool(description = "List recent IssueRelay runs")]
    async fn list_jobs(&self, Parameters(args): Parameters<JobsArgs>) -> String {
        self.forward("list_jobs", json!({"limit": args.limit.unwrap_or(20)})).await
    }

    #[rmcp::tool(description = "Pause monitoring; runs already in progress continue")]
    async fn pause(&self) -> String {
        self.forward("pause", json!({})).await
    }

    #[rmcp::tool(description = "Resume monitoring")]
    async fn resume(&self) -> String {
        self.forward("resume", json!({})).await
    }

    #[rmcp::tool(description = "Check Linear for new issues now")]
    async fn poll_now(&self) -> String {
        self.forward("poll_now", json!({})).await
    }
}

pub async fn serve(endpoint: IpcEndpoint) -> anyhow::Result<()> {
    let service = ControlServer::new(endpoint).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

Run: `cargo test -p issuerelay-cli --test mcp_tools`
Expected: PASS（1 个）

- [ ] **Step 6: 写命令行入口**

`crates/cli/src/main.rs`：

```rust
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use issuerelay_cli::{format, mcp};
use issuerelay_core::{
    config::{Binding, ExecutorKind, PermissionProfile},
    ipc::client,
    paths::AppPaths,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "issuerelay", version, about = "Control the running IssueRelay engine")]
struct Cli {
    /// Print the raw JSON result.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show monitoring status.
    Status,
    /// Pause monitoring (runs in progress continue).
    Pause,
    /// Resume monitoring.
    Resume,
    /// Check Linear now.
    Poll,
    /// Check executors, the Linear workspace and bindings.
    Doctor,
    /// List recent runs.
    Jobs {
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// Show the last lines of a run's log.
    Logs {
        job_id: i64,
        #[arg(long, default_value_t = 50)]
        lines: u64,
    },
    /// Manage Linear project bindings.
    #[command(subcommand)]
    Bind(BindCommand),
    /// Run the MCP control server on stdio (for Claude Code / Codex).
    Mcp,
}

#[derive(Subcommand)]
enum BindCommand {
    List,
    Add {
        #[arg(long)]
        project_id: String,
        #[arg(long)]
        project_name: String,
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, value_parser = parse_executor)]
        executor: Option<ExecutorKind>,
        #[arg(long, value_parser = parse_permission)]
        permission: Option<PermissionProfile>,
    },
    Remove {
        id: String,
    },
}

fn parse_executor(text: &str) -> Result<ExecutorKind, String> {
    ExecutorKind::parse(text).ok_or_else(|| format!("expected `claude` or `codex`, got `{text}`"))
}

fn parse_permission(text: &str) -> Result<PermissionProfile, String> {
    PermissionProfile::parse(text).ok_or_else(|| format!("expected `full`, `auto` or `personal`, got `{text}`"))
}

fn request(command: &Command) -> Result<(&'static str, Value)> {
    Ok(match command {
        Command::Status => ("status", json!({})),
        Command::Pause => ("pause", json!({})),
        Command::Resume => ("resume", json!({})),
        Command::Poll => ("poll_now", json!({})),
        Command::Doctor => ("doctor", json!({})),
        Command::Jobs { limit } => ("list_jobs", json!({"limit": limit})),
        Command::Logs { job_id, lines } => ("logs", json!({"job_id": job_id, "lines": lines})),
        Command::Bind(BindCommand::List) => ("bindings.list", json!({})),
        Command::Bind(BindCommand::Remove { id }) => ("bindings.remove", json!({"id": id})),
        Command::Bind(BindCommand::Add { project_id, project_name, dir, executor, permission }) => {
            let binding = Binding {
                id: project_id.clone(),
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                directory: std::path::absolute(dir)?,
                executor: *executor,
                permission: *permission,
                model: None,
                prompt_override: None,
            };
            ("bindings.add", json!({"binding": binding}))
        }
        Command::Mcp => unreachable!("handled before sending a request"),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let endpoint = AppPaths::system()?.ipc_endpoint();
    if matches!(cli.command, Command::Mcp) {
        return mcp::serve(endpoint).await;
    }
    let (method, params) = request(&cli.command)?;
    let value = client::call(&endpoint, method, params).await?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{}", format::human(method, &value));
    }
    Ok(())
}
```

Run: `cargo run -q -p issuerelay-cli -- --help && cargo run -q -p issuerelay-cli -- status; echo "exit=$?"`
Expected: 帮助里列出全部子命令；没有宿主在运行时 `status` 打印 `Error: IssueRelay is not running`，退出码 1

- [ ] **Step 7: 全量测试并提交**

```bash
cargo test --workspace
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(cli): add issuerelay command and MCP control server

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 22: 开发宿主

**Files:**
- Create: `crates/devhost/Cargo.toml`、`crates/devhost/src/main.rs`

**Interfaces:**
- Consumes: Task 17 的 `auth`；Task 16 的 `McpLinear`；Task 18 的 `SystemReadiness`；Task 19 的 `Engine`、`ProcessJobRunner`；Task 20 的 `IpcServer`
- Produces: 二进制 `issuerelay-devhost`（只供开发，`publish = false`，不进安装包）：`login`、`api-key`（从标准输入读 key）、`logout`、`projects`（列出 Linear 项目的 id 与名称，供 `bind add` 使用）、`run`

- [ ] **Step 1: 建 crate**

```bash
mkdir -p crates/devhost/src
cat > crates/devhost/Cargo.toml <<'EOF'
[package]
name = "issuerelay-devhost"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[dependencies]
EOF
cd crates/devhost
cargo add issuerelay-core --path ../core
cargo add anyhow open tracing tokio-util
cargo add clap --features derive
cargo add tokio --features full
cargo add tracing-subscriber --features env-filter
cd ../..
```

- [ ] **Step 2: 写宿主**

`crates/devhost/src/main.rs`：

```rust
//! Development-only headless host for the IssueRelay engine. Not shipped; the M2 tray app
//! embeds the engine the same way.
use std::{io::Read, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use issuerelay_core::{
    doctor::SystemReadiness,
    engine::{Engine, EngineDeps, ProcessJobRunner},
    ipc::server::IpcServer,
    linear::{
        LinearApi, auth,
        mcp::{LinearClient, McpLinear},
    },
    paths::AppPaths,
    store::Store,
};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "issuerelay-devhost", about = "Development-only headless host for IssueRelay (not shipped)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authorize with Linear in the browser and keep the token in the system keychain.
    Login,
    /// Read a Linear personal API key from stdin and keep it in the system keychain.
    ApiKey,
    /// Forget stored Linear credentials.
    Logout,
    /// List Linear projects (id and name) for `issuerelay bind add`.
    Projects,
    /// Run the engine and the IPC server until Ctrl+C.
    Run,
}

async fn connect() -> Result<LinearClient> {
    if let Some(client) = auth::connect_stored(auth::LINEAR_MCP_URL).await? {
        return Ok(client);
    }
    if let Some(key) = auth::load_api_key()? {
        return auth::connect_api_key(auth::LINEAR_MCP_URL, &key).await;
    }
    bail!("no Linear credentials; run `issuerelay-devhost login` (or `api-key`) first")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,issuerelay_core=debug")),
        )
        .init();
    match Cli::parse().command {
        Command::Login => {
            let client = auth::login(auth::LINEAR_MCP_URL, |url| {
                println!("Open this URL to authorize IssueRelay:\n{url}");
                let _ = open::that_detached(url);
            })
            .await?;
            let linear = McpLinear::new(client).await?;
            println!("Logged in to Linear as {}", linear.viewer().await?.name);
            linear.close().await
        }
        Command::ApiKey => {
            let mut key = String::new();
            std::io::stdin().read_to_string(&mut key)?;
            let key = key.trim();
            if key.is_empty() {
                bail!("no API key on stdin");
            }
            let linear = McpLinear::new(auth::connect_api_key(auth::LINEAR_MCP_URL, key).await?).await?;
            println!("API key works for {}", linear.viewer().await?.name);
            auth::save_api_key(key)?;
            linear.close().await
        }
        Command::Logout => {
            auth::clear_credentials()?;
            println!("Stored Linear credentials removed.");
            Ok(())
        }
        Command::Projects => {
            let linear = McpLinear::new(connect().await?).await?;
            for project in linear.projects().await? {
                println!("{}\t{}", project.id, project.name);
            }
            linear.close().await
        }
        Command::Run => {
            let paths = AppPaths::system()?;
            let server = IpcServer::bind(&paths.ipc_endpoint()).await?;
            let linear = Arc::new(McpLinear::new(connect().await?).await?);
            let engine = Engine::new(EngineDeps {
                config_path: paths.config_file(),
                store: Store::open(&paths.state_db())?,
                linear,
                readiness: Arc::new(SystemReadiness::new(Duration::from_secs(300))),
                job_runner: Arc::new(ProcessJobRunner),
                logs_dir: paths.logs_dir(),
            })?;
            let shutdown = CancellationToken::new();
            let ipc = tokio::spawn(server.run(engine.clone(), shutdown.clone()));
            let worker = {
                let engine = engine.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move { engine.run_forever(shutdown).await })
            };
            tracing::info!("IssueRelay devhost is running; press Ctrl+C to stop");
            tokio::signal::ctrl_c().await?;
            shutdown.cancel();
            let _ = worker.await;
            let _ = ipc.await;
            engine.wait_idle().await;
            Ok(())
        }
    }
}
```

Run: `cargo build -p issuerelay-devhost && cargo run -q -p issuerelay-devhost -- --help`
Expected: 构建成功；帮助列出 `login`、`api-key`、`logout`、`projects`、`run`

- [ ] **Step 3: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && bash scripts/check-secrets.sh --strict
git commit -m "feat(devhost): add development-only headless host

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 23: M1 实机验收

> 执行说明（2026-10-12，最终评审后）：按下列调整执行，结果写进验收报告。
> - 只在宿主停止时改 `config.toml`，每次改完（包括改回 60）都重启宿主；运行期间手改的配置会被拒绝保存。
> - 在普通终端里启动 devhost，不在 Claude Code / Codex 会话里启动（宿主会把环境变量传给执行者）。
> - 登录如果浏览器回调到不了本机端口，改用应用内浏览器打开授权链接，或用 `api-key`；`login` 之后不要重新编译 devhost，否则钥匙串会再次询问，弹窗时选「始终允许」。
> - 用 `issuerelay jobs --json` 看原因、摘要和时间，判断「1 分钟内被接走」。
> - 5b.2 改为能看见后代的检查：让 AI 在前台执行 `sleep 3600`，运行记为超时失败后 `pgrep -fl 'sleep 3600'` 必须为空；另做一次「运行中按 Ctrl+C 停掉 devhost」；正常运行、停止、超时之后都用 `ps -axo pid,ppid,pgid,command` 查残留，并检查沙箱仓库有没有遗留 `.git/index.lock`。
> - 重点观察 M0 没实测过的两种写法：按 uuid 调 `get_issue`，以及给 `list_issues` 传 `state: "Todo"` 与 `orderBy: "createdAt"`。启动前重读时任务被取消、原因是「issue changed before the run started」，先怀疑按 uuid 查询。

**Files:**
- Create: `docs/gates/<YYYY-MM-DD>-m1-acceptance.md`

**Interfaces:**
- Consumes: Task 2 的沙箱（需要新建测试需求）、Task 4 配好的 Claude Code / Codex、Task 21–22 的程序
- Produces: M1 的实机验收结论

- [ ] **Step 1: 全量验证**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && bash scripts/check-secrets.sh --strict`
Expected: 全部通过；记下各 crate 的测试数

- [ ] **Step 2: 登录并绑定（用户操作 + 会话执行）**

```bash
cargo run -q -p issuerelay-devhost -- login          # 用户在浏览器授权
cargo run -q -p issuerelay-devhost -- projects       # 找到 IssueRelay Sandbox 的项目 id
```

另开一个终端运行宿主（保持运行）：

```bash
cargo run -q -p issuerelay-devhost -- run
```

回到原终端：

```bash
cargo run -q -p issuerelay-cli -- bind add --project-id <SANDBOX_PROJECT_ID> --project-name "IssueRelay Sandbox" --dir ~/issuerelay-sandbox --executor claude
cargo run -q -p issuerelay-cli -- doctor
```

Expected: `doctor` 显示 Linear 已连接、claude 与 codex 都是 ready、工作区标签和状态齐全、绑定 ok。

- [ ] **Step 3: Claude 完整流程**

在沙箱项目新建一条需求（指派给自己、加 `ai-ready`、Todo），内容仿照 Task 2 的 SANDBOX-A（例如新增 `hello2.txt`）。然后：

```bash
cargo run -q -p issuerelay-cli -- jobs        # 30 秒内出现 running
cargo run -q -p issuerelay-cli -- status
cargo run -q -p issuerelay-cli -- jobs        # 运行结束后为 awaiting_acceptance
cargo run -q -p issuerelay-cli -- logs <JOB_ID> --lines 20
git -C ~/issuerelay-sandbox/remote.git log test --oneline -3
```

Expected：从建需求到出现 `running` 不超过 1 分钟；结束后为 `awaiting_acceptance`；Linear 上该需求为 In Testing，并有以 `🤖 IssueRelay` 开头的评论；`remote.git` 的 `test` 分支有对应提交；`main` 没被推送。

- [ ] **Step 4: Codex 与「待补充信息」**

```bash
cargo run -q -p issuerelay-cli -- bind add --project-id <SANDBOX_PROJECT_ID> --project-name "IssueRelay Sandbox" --dir ~/issuerelay-sandbox --executor codex
```

再建两条需求：一条写清楚的（验证 Codex 走到 `awaiting_acceptance`），一条故意写得含糊，例如只写「改一下首页」（验证得到 `needs_info`：AI 在需求下提问、改回 Todo、去掉 `ai-ready`）。

- [ ] **Step 5: 暂停、恢复与 MCP 控制服务**

```bash
cargo run -q -p issuerelay-cli -- pause
cargo run -q -p issuerelay-cli -- status      # Monitoring: paused；此时新建的 ai-ready 需求不会被接
cargo run -q -p issuerelay-cli -- resume
```

征得用户同意后，把 MCP 控制服务注册进 Claude Code，在对话里调用 `status`、`list_jobs`，确认能读到状态：

```bash
cargo build -q -p issuerelay-cli
claude mcp add issuerelay -- "$(pwd)/target/debug/issuerelay" mcp
```

验收完如用户不想保留，执行 `claude mcp remove issuerelay`。

- [ ] **Step 5b: 失败、超时、其他档位与 M0 留下的观察项**

1. 失败：建一条做不到的需求（例如要求修改沙箱里不存在的 `ghost/` 目录下的文件，并写明找不到就按失败处理）。期望：AI 评论失败原因、改回 Todo、去掉 `ai-ready`；本地记为 `failed`；`test` 分支没有新提交。
2. 超时：把配置文件（`dirs::config_dir()/com.sunkailun.issuerelay/config.toml`）`[settings]` 里的 `run_timeout_mins` 临时改为 2，重启宿主，建一条明显超过 2 分钟的需求。期望：到时整个进程树被结束（`pgrep -fl claude` 与 `pgrep -fl codex` 没有本次运行留下的进程），本地记为 `failed` 且原因是超时；验完改回 60。
3. 其他权限档位：对同一项目再执行一次 `bind add ... --permission auto`（会替换原绑定），把它改成「自动审查」，Claude、Codex 各跑一条简单需求；再用 `--permission personal`（「按个人设置」）各跑一条。记录是否卡在确认、是否完成；跑不通的档位在报告和设计 6.1 里写明「不可用」及原因。
4. 子需求：在沙箱建一条父需求和一条带 `ai-ready` 的子需求，记录 `list_issues` 返回的 `parentId` 形态（编号还是 UUID），与 Task 16 的 `parse_issue` 核对。
5. 翻页末页：用任一 Linear MCP 客户端（例如 Claude Code 的 Linear 工具）以 `limit` 1 沿 `cursor` 翻 `list_issues` 到最后一页，记录 `hasNextPage` 为假的那一页的形态（只记形态，不存内容）。
6. 令牌续期：宿主连续运行超过访问令牌有效期后，确认仍能查询（续期成功）；时间不够就写「待观察」。

- [ ] **Step 6: 写验收报告并提交**

按 `docs/gates/README.md` 的结构写 `docs/gates/<日期>-m1-acceptance.md`：环境与版本、每一步的命令和结果、从建需求到被接走的耗时、Linear 状态与评论、推送结果、未验证项（Windows，包括凭据在 Windows 凭据管理器里的长度；标记完成——属于 M2 面板）、与设计不一致之处。报告里的需求编号、项目名、路径按规则脱敏。

```bash
git add docs/gates
bash scripts/check-secrets.sh --strict
git commit -m "docs(m1): record engine and CLI acceptance on the sandbox

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git fetch origin && git push origin main
```

M1 到此结束。M2（桌面应用）的计划在 M1 验收报告确认后另写。

---

## 本计划不覆盖（留给 M2、M3）

以下设计内容不在本计划内，会在 M1 验收后另写计划：

| 设计章节 | 内容 | 里程碑 |
|---|---|---|
| 8 | 首次引导界面、一键执行 Linear MCP 添加命令 | M2 |
| 9 | 托盘菜单、面板全部页面、外观、多语言、系统通知 | M2 |
| 5.6 | 面板上的「标记完成」按钮（引擎接口 `Engine::mark_done` 已在 Task 19 完成） | M2 |
| 6.2 | 退出托盘程序时询问「等待完成 / 立即停止」 | M2 |
| 11 | 日志按保留天数与总大小清理（默认 30 天、1 GB）（M2 第一部分 Task 3 已完成） | M2 |
| 10.3 | 在设置里一键把 MCP 控制服务注册到 Claude Code / Codex | M2 |
| 8 | 图形程序启动时经登录 shell 读取 PATH，用它查找执行者（M0 推断 F2，M2 实机验证） | M2 |
| 8 | 执行者的代理环境：托盘程序从系统代理设置或登录 shell 取 `HTTPS_PROXY`、`HTTP_PROXY`、`NO_PROXY` 传给执行者，环境检查的提示点明代理（M1 验收 H1） | M2 |
| 7 | 中文标题的 Linear 链接带非 ASCII slug，`safe_url` 会把它丢掉：保留到编号为止的链接，或做百分号编码（M1 验收 H2） | M2 |
| 18 | 按绑定的精简运行方式：只加载 Linear MCP（M0 发现 F3） | M2 |
| 9.6 | 应用图标与托盘图标（两到三套方案由用户选定） | M2 |
| 6.2 | Windows 上从托盘程序启动执行者时加 `CREATE_NO_WINDOW`（经 process-wrap 的 `CreationFlags`，不能直接设，`JobObject` 会覆盖），避免弹出控制台窗口、用户关窗即中断运行（Task 15 评审）；同时加 `CREATE_NEW_PROCESS_GROUP`，让开发宿主所在控制台的 Ctrl+C 不直接打到执行者，被停止的运行记为「已停止」而不是「失败」（Task 22 评审）（M2 第一部分 Task 2 已完成，待 Windows 实机） | M2 |
| 6.2 | 宿主崩溃后的残留进程：记录每次运行的进程组 / Job，启动时清扫（Task 15 评审）（M2 第一部分 Task 4 已完成） | M2 |
| 6.4 | 环境检查命令超时后结束整棵进程树（被杀的 `claude mcp list` 可能留下它启动的 stdio MCP 服务器），从托盘程序运行时不弹控制台窗口（Task 18 评审）（M2 第一部分 Task 1、2 已完成） | M2 |
| 10.2 | Windows 命名管道改用只允许当前用户的安全描述符：默认描述符让其他用户能读、不能写请求（Task 20 评审）（M2 第一部分 Task 2 已完成，待 Windows 实机） | M2 |
| 12 | 安装包、自动更新、发布流程、代码签名 | M3 |

### 评审遗留的小项（2026-10-13 整理）

M0–M1 的任务评审和最终评审里决定以后处理的小项。按最终评审（c3921f5）的分类整理，并对照 5c336e6 的代码逐条核对过，已解决的不再列出。写 M2、M3 计划时逐条认领，或写明不做及原因。

**M2：引擎与运行**

| 模块 | 内容 | 来源 |
|---|---|---|
| `engine` | 重新授权后要重建引擎：`viewer` 在引擎的整个生命周期里只取一次 | Task 19 评审 |
| `engine` | `count()` 读存储出错时返回 0，错误被吞掉 | Task 19 评审 |
| `engine` | 退出时的「等待完成」需要能持久的暂停（现在的停止标志只在进程内）；第一次停止信号要等当前这轮检查结束（最长约 60 秒）才通知运行停止 | Task 19 评审、最终评审 |
| `engine` | 队首任务启动前的重读一直失败时，`max_concurrent = 1` 或同一目录下的其他任务一直轮不到 | Task 19 评审 |
| `engine` | 严格解析的取舍（读不懂的回复一律报错）：一条读不懂的需求会让这一轮不派活、挡住所有排队任务，错误会指明是哪条；面板要把这个错误显示清楚 | 最终评审 |
| `engine` | 状态为 `waiting_binding` 的新任务，日志也写成 `job queued` | 最终修复复审 |
| `engine`、面板 | `MissingArguments` 要和 `MissingTools` 一样转成用户看得懂的提示（设计 §7）；`list()` 的错误用 `{:#}` 显示 | Task 16 评审 |
| `runner` | 离开进程组的后代只收到 SIGKILL，没有先收到 SIGTERM；停止最多多等 5 秒；执行者在 SIGTERM 处理里新起的独立进程，如果它在宽限期内退出，会成为找不到的孤儿（M1 验收 H6 看到会话结束钩子几秒内自行退出）；对已回收的进程组 id 调 `killpg`（影响可忽略） | 最终修复、复审 |
| `runner` | 失控输出：通道没有上限，单行也没有长度上限（M2 第一部分 Task 3 已完成） | Task 15 评审 |
| `runner` | 运行日志：写入失败要能上报（有日志系统后）；重试会覆盖上一次的 `{job.id}.jsonl`（目前有意为之并已注明），做日志页时决定是否保留；0600 只对新建的日志文件生效（写入失败告警和 0600 已在 M2 第一部分 Task 3 完成，重试覆盖日志的取舍仍待做日志页时决定） | Task 15 评审 |
| 设计 | 超时和卡住判定用的单调时钟，在系统睡眠期间是否计时因平台而异，写进设计 | Task 15 评审 |

**M2：Linear、环境检查、IPC、命令行与面板**

| 模块 | 内容 | 来源 |
|---|---|---|
| `linear::mcp` | 改用 rmcp 自带的请求超时与取消；空页却 `hasNextPage: true`、或游标重复时 `follow()` 的处理；`ToolFailed` 的原文在显示处要转义；连接时只查工具名和参数名，不查 `fields` 枚举和上限；可选字段缺失时静默取默认值 | Task 16 评审 |
| `linear::auth` | 用户把 `RUST_LOG` 设到 rmcp 的 debug 级别时，rmcp 会把 OAuth 授权码写进日志，配置日志时要压住；`login` 换令牌期间仍占着回环端口，拿到回调后就该释放 | Task 17、22 评审 |
| `text` | `clean_text` / `is_invisible` 和 `oauth_error` 放过了标签字符 U+E0000–E007F、U+061C、U+00AD 和韩文填充字符；另外去掉 ZWJ / ZWNJ 会影响 emoji 和印度系文字的显示 | Task 17、19、21 评审 |
| `doctor` | 就绪检查在宿主的当前目录跑 `claude mcp list`，结果随目录变（local / project 作用域、`.mcp.json`），而运行在绑定目录：改为按绑定目录检查，或在提示里写明必须用 `--scope user` 注册 | 最终评审 |
| `doctor` | 工作区检查覆盖所有团队，多团队工作区里不用 IssueRelay 的团队也会报缺状态：只查已绑定项目所属的团队 | 最终评审 |
| `ipc` | Windows：客户端连接时校验管道服务端的属主 SID，防止其他本地用户抢先建同名管道（和上表 10.2 一起做）（M2 第一部分 Task 2 已完成，待 Windows 实机） | 最终评审 |
| `config` | 保存时静默丢弃未知配置项，有日志后至少警告一次；保存中途崩溃会留下 `config.toml.<pid>.<n>.tmp`，没有清扫；`comment_language` / `language` 可加语言标签格式的校验（U+2028、U+200B 和任意单行文本目前都能通过） | Task 7、11 评审 |
| `executor` | `resolve_program` 没有测试，可加带 PATH 参数的 `resolve_program_in`（用 `which_in`）；`is_git_repo` 在仓库子目录和不可读目录里返回 false（后果只是多加 `--skip-git-repo-check`） | Task 12 评审 |
| `cli` | MCP 工具返回的文本没有清洗（使用者是 AI，说明里已写明是数据）；`pause`、`resume`、`poll_now` 缺工具注解（`destructiveHint: false`、`idempotentHint`）；`bind add` 不规范化路径（引擎派活时会规范化）；`doctor` 等待期间没有任何输出 | Task 21 评审、最终评审 |
| 面板 | 「默认模型」存 `None`，不存空字符串；带 `[ ]` 的模型名（例如 `sonnet[1m]`）按设计会被拒绝，面板说明要写明 | Task 12 评审 |

**M3**

| 模块 | 内容 | 来源 |
|---|---|---|
| `store` | 数据库的 `user_version` 比当前程序新时不拒绝，第一次改表结构时处理（M2 第一部分 Task 4 已完成） | Task 9 评审 |
| 打包 | 命令行依赖整个 `core`；`fake-agent` 是 `issuerelay-core` 的普通可执行目标，打包时会被一起构建 | Task 21 评审、最终评审 |
| 依赖 | chrono 的默认特性（clock、wasmbind）用不到，可改成 `default-features = false` 加 `["std", "now", "serde"]` | Task 8 评审 |
| CI | `ci.yml` 显式安装 1.99.0，和 `rust-toolchain.toml` 重复（rustup 1.28 起 `rustup show` 不再自动安装，所以要显式装）：升级工具链时两处一起改 | Task 1 评审 |

**Windows 实机验收时一起看**

- 凭据在 Windows 凭据管理器里能否持久保存，以及长度上限（2560 字节）。
- 每次请求都读一次钥匙串，在 Windows 上是否有问题（macOS 验收时没有弹窗）。
- Windows 专用代码目前只在 CI 里编译和跑测试。
- 卡住判定的测试要求启动后 1.2 秒内出第一行，Windows 首次运行时的杀毒扫描可能让它偶发失败，留意 CI。

**测试打磨（没有已知缺陷）**

- `store`：测试夹具的 id 与 project_id 相同（`proj-1`），两者互换测不出来；`assign_stores_a_reason` 没断言 executor 为 `None`。
- `config`：模型名检查的测试只覆盖 `bindings[0]`，闭包 `binding` 遮住了同名的测试辅助函数；校验测试补空白的 `comment_language` / `language` 用例。
- `executor::outcome`：补 CRLF、缩进的 JSON、行内代码里的 JSON、跳过解析不了的 `{` 行、Claude 取最后一个结果事件、两种执行者互不干扰的测试；`is_error` 结果的文本仍会成为 `final_text`，要写进文档；夹具来源说明；用显式的 match 分支代替 `_ => None`。
- `fake-agent`：脚本校验宽松（类型错误、拼错的键被忽略，`every_ms` 没有下限）；文件头没列出每一步的键和顺序。
- `runner`：`end()` 在 `wait()` 出错时也会置为结束（无害）；`run` 的文档说启动失败会写日志，但日志目录或文件建不了时无处可写。
- `linear::auth`：`serve_one` 只读一次（请求行被拆成几次读时会返回 404，并等到登录超时；浏览器会一次写完请求头）；内存钥匙串由第一个钥匙串测试懒加载（以后若有库测试调用 `APP_ID` 的包装函数，可能因测试顺序碰到真实钥匙串）；返回时取消任务、连接超时的调用点、accept 失败重试、404 写入失败都没有测试；可改用 keyring-core 的 `mock::Store`。
- `trigger`：`all_lists_every_variant_once` 的注释说过了头；`is_candidate` 内联和 `JobState::parse` 大小写的变异体存活。
- `cli`：`ControlServer::new` 到 `client::call` 的衔接只有「宿主未运行」这一个测试覆盖。

**扫描脚本**

- 可选的本机私有清单（不进 git，放所有者名字、工作区标识、团队前缀），让 `check-secrets.sh` 一起查；在那之前，提交验收报告或样本前要先用自己的私有清单手动 grep 一遍。
- `git grep` 会跳过被跟踪的符号链接，脚本头部没写明。

**文档与计划文字（不影响行为）**

- `config.rs` 里 `Settings::validate` 的文档注释仍只说「stall or spin the engine」。
- `AI_CODING_GUIDE.md` 第 3 节和 `AI_DESIGN_PATTERN_GUIDE.md` 的反模式一节说纯逻辑层不做 IO，没写 `trigger` 检查目录是否存在这个例外；`AI_CODING_GUIDE.md` 第 5 节的索引没提 `config.rs` 里的模型名规则。
- `MAINTAINERS.md` 的设置表缺「首次贡献者的工作流运行需要批准」一行，批准运行的职责可加一句「先看 diff」；`AI_AUTOMATION_WORKFLOW.md` 第七节第 2 条是一整行长句；CODEOWNERS 的适用范围在 `AGENTS.md` 和流程文档里写「极高风险」，在 CODEOWNERS 和目录指南里写「高风险」；squash 提交的正文直接取 PR 描述（含模板里的勾选项），要靠维护者合并时整理。
- `linear::mcp` 关于空数组 `[]` 的文档说法过头；新加的外部事实注释缺日期（`CONNECT_TIMEOUT`、Linux 钥匙串说明、Vault / mock 说明）。
- 本计划里的历史快照：Task 1 的 `.gitignore` 仍是 `/spikes/**/samples-raw/`（仓库实际是 `**/samples-raw/`），Task 1 内嵌的 AI 编程指南仍写 `data_dir()` / `%APPDATA%`；Task 11、12 的代码块是修复前的版本（旧的团队说明标题、`is_safe_token`）；Task 3、4、23 经 `docs/gates/README.md` 转到 `TESTING_SPEC.md`；Task 6 的报告模板标题早于 `TESTING_SPEC.md`；Task 18 的一个反例还留着旧的「✓」。

