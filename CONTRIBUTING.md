# 参与贡献

## English summary

- Issues and pull requests are welcome, in English or Chinese. Ask questions with the Question issue form (Discussions are off and blank issues are disabled).
- Open an issue first for large changes and new features, and for anything that touches the six Critical areas: executor flags and permission tiers, the prompt's fixed rules, ending the process tree, credential storage and authorization, the IPC method table, and mark-done.
- Read [AGENTS.md](AGENTS.md) before you code. It is written for AI tools, and its rules apply to people too.
- Run the four checks in the section "提交前检查" before you push: `cargo fmt`, `cargo clippy`, `cargo test` and `bash scripts/check-secrets.sh --strict` (on Windows the scan needs Git Bash or WSL).
- Write commit messages and PR titles as English Conventional Commits, for example `fix(runner): kill the whole process tree on timeout`. Do not change version numbers; maintainers do that when they release.
- A maintainer squash-merges the PR after CI is green, one maintainer has approved and every review conversation is resolved. For a first contribution, a maintainer must approve the CI run before it starts.
- This repository is public. Never post tokens, company information or real Linear content in issues, PRs, logs or screenshots.
- Report security problems privately from the Security tab, never in a public issue. Contributions are licensed under Apache-2.0.

## 可以贡献什么

- 问题报告：用 issue 模板提交，中文或英文都可以。
- 文档、测试、修 bug。
- 新功能：先开 issue 讨论，见「开始之前」。
- 界面翻译：M2 起接受，每种语言一个翻译文件。语言清单见[设计文档](docs/superpowers/specs/2026-10-10-issuerelay-design.md)第 9.4 节，文件位置见[目录规范](docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md)。里程碑 M0–M3 的含义见 [README.md](README.md) 的状态说明和设计文档第 17 节。

新人可以从带 `good first issue`（适合新人）或 `help wanted`（欢迎帮忙）标签的 issue 开始。

## 开始之前

先开 issue，等维护者确认，再写代码：

- 大改动和新功能：在 issue 里写清要解决的问题和你的方案。
- 涉及极高风险（Critical）的任何改动：必须先在 issue 里写方案。极高风险是 [AI_ENGINEERING_GUARDRAILS.md](docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md) 风险分级里最高的一级，共六项：执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权、IPC 方法表、标记完成。

其余的小改动（错别字、文档小修、范围明确的 bug 修复）可以直接开 PR。

有疑问先问：用「提问 / Question」模板开 issue。本仓库没有开 Discussions，也不能开空白 issue，所有 issue 都要用模板。

## 开发环境

支持 macOS 和 Windows。装好 [rustup](https://rustup.rs/) 后，克隆你的 fork，在仓库根目录运行：

```bash
rustup toolchain install   # 按 rust-toolchain.toml 装好 1.99.0 及 rustfmt、clippy
cargo build --workspace
```

单元测试和集成测试用假执行者和假 Linear MCP，所以不需要 Linear 账号，也不需要安装 Claude Code 或 Codex。

实机验收才需要真实环境：你自己的 Linear 测试项目（不是真实业务项目），加上已登录的 Claude Code 或 Codex。步骤见 [docs/gates/](docs/gates/) 和 [TESTING_SPEC.md](docs/ai-coding/TESTING_SPEC.md)。

## 提交前检查

提交前在仓库根目录跑这四条，全部通过再推送：

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash scripts/check-secrets.sh --strict
```

- 密钥扫描必须无输出、退出码为 0。新文件要先 `git add`，扫描才会覆盖它。
- Windows 上运行 `bash scripts/check-secrets.sh --strict` 需要 Git Bash（随 Git for Windows 安装）或 WSL。
- CI 在 macOS 和 Windows 上跑前三条（`test (macos-latest)`、`test (windows-latest)`），在 Ubuntu 上跑密钥扫描（`secrets`）。三项都通过才能合并。第一次给本仓库贡献时，GitHub 会先等维护者批准你的 PR 的工作流运行，CI 才会开始；这是仓库设置，耐心等一下即可。
- 这四条与 [AGENTS.md](AGENTS.md) 的「验证命令」一致。如有出入，以 AGENTS.md 为准。

## 分支、提交与 PR

- **分支**：fork 后从最新 `main` 切分支，建议命名 `feat/<主题>`、`fix/<主题>`。一个 PR 只做一件事，目标分支选 `main`。
- **提交信息和 PR 标题**：用英文 Conventional Commits。类型取 `feat`、`fix`、`docs`、`test`、`refactor`、`chore`、`ci`，可带范围，如 `fix(runner): ...`。squash 合并后，PR 标题就是 `main` 上的提交标题，所以 PR 标题也要按这个格式写。
- **PR 描述**：按[模板](.github/pull_request_template.md)填，中文或英文都可以。草稿 PR 可以提前开。
- **合并条件**：CI 三项全绿（首次贡献者的 CI 要先由维护者批准运行）、至少一名维护者批准、评审对话全部解决。批准之后你再推送新提交，需要重新批准。由维护者 squash 合并，合并时把提交正文精简成英文摘要。
- **已推送的分支不要 rebase 或强推**（[AGENTS.md](AGENTS.md) 禁止强推）；还没推送的本地提交可以 rebase。需要更新就追加提交；要同步 `main`，就把 `main` 合并进你的分支。最终会 squash 成一个提交，分支历史不重要。
- **版本号**：外部贡献者不改版本号。维护者在发布时升版本号（M3 起，随 `vX.Y.Z` 标签）。

常用命令：

```bash
# 只需一次：添加上游仓库
git remote add upstream https://github.com/sunkailun1992/issuerelay.git

# 开始新工作：从最新 main 切分支（--no-track：先不绑定上游，下面第一次推送时再绑定到你的 fork）
git fetch upstream
git switch -c fix/kill-process-tree --no-track upstream/main

# 第一次推送：推到你自己的 fork（origin）
git push -u origin fix/kill-process-tree

# 之后要同步 main：合并，不要 rebase
git fetch upstream
git merge upstream/main
```

## 代码与文档规范

- 先读 [AGENTS.md](AGENTS.md) 和 [docs/ai-coding/README.md](docs/ai-coding/README.md)。AGENTS.md 虽然写给 AI，对人同样适用。
- macOS 和 Windows 同等对待，细则见 AGENTS.md 的「项目边界」。
- 新行为要有测试。修 bug 先写一个能复现它的测试，分层和假实现见 [TESTING_SPEC.md](docs/ai-coding/TESTING_SPEC.md)。
- 测试通过不等于实机验收。改动涉及真实 Linear、Claude Code 或 Codex 的行为时，在 PR 里写实机结果；做不了就写「待实机」和原因。
- 设计、计划、验收报告用中文，`README.md` 中英双语。源码注释用英文，写法见 [AI_COMMENT_STYLE_GUIDE.md](docs/ai-coding/AI_COMMENT_STYLE_GUIDE.md)。
- 改了行为，就同步改相关文档。

## 使用 AI 辅助

欢迎用 AI 辅助。各工具这样读到本仓库的规则：

| 工具 | 读取方式 |
|---|---|
| Codex | 直接读 `AGENTS.md` |
| Claude Code | 经 `CLAUDE.md` 导入 `AGENTS.md` |
| GitHub Copilot | 读 `.github/copilot-instructions.md`，它转发到 `AGENTS.md` |

- 你对 AI 生成的每一行负责，提交前自己逐行审一遍。
- PR 模板里有一项，请勾选你的改动是否含 AI 生成的内容。
- AI 能做什么、不能做什么，以 [AGENTS.md](AGENTS.md) 为准，包括其中的「贡献流程」一节。

## 公开仓库红线

本仓库公开。issue、PR 描述、评论、截图、日志、测试夹具里，都不能出现：

- 令牌或凭据
- 公司名
- 内网地址
- 真实 Linear 工作区或需求内容
- 本机绝对路径

（以 `AGENTS.md`「禁止事项」为准）

贴日志和样本前先脱敏，规则见 [TESTING_SPEC.md](docs/ai-coding/TESTING_SPEC.md) 的「夹具与脱敏」：用 `DEMO-1`、`Demo Project`、`Jane Doe` 这样的虚构值，路径写成 `~/…`。

`scripts/check-secrets.sh` 只能兜底：它只扫描仓库里的文件，扫不到 issue、评论和截图。

万一泄露了令牌：先去对应服务吊销，再通过 Security 页面的私密报告联系维护者清理（见 [SECURITY.md](SECURITY.md)）。不要在公开 issue 里重复贴出泄露的内容。

## 许可证

你提交的贡献按 Apache-2.0 授权（[LICENSE](LICENSE) 第 5 条：提交即按同一许可证授权）。不需要签 CLA，也不要求 DCO 签名。

## 行为准则与安全问题

- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)：参与时的行为准则，以及怎么举报。
- [SECURITY.md](SECURITY.md)：发现安全问题时怎么私下报告，不要开公开 issue。
- [MAINTAINERS.md](MAINTAINERS.md)：谁是维护者，评审、合并和分支保护的规则。
