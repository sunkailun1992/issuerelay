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
bash scripts/check-secrets.sh --strict
```

## 禁止事项

- 禁止把 Linear 令牌或 API key、Claude / Codex / GitHub 凭据、任何公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径写进代码、测试、夹具、文档、提交信息。
- 禁止删减提示词固定规则；禁止在 IPC、命令行、MCP 暴露「标记完成」「重试」「取消」。
- 禁止经 shell 启动子进程，禁止把外部文本拼进参数。
- 禁止强推，禁止改写已推送的历史。
- 禁止用夹具测试、设计文档或计划冒充实机验收。
