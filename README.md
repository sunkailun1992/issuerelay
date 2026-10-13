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
- AI 用的是你本机 Claude Code / Codex 的全部配置：用户级指令文件、插件、hooks 和所有 MCP 服务器都会生效，它们也会影响无人值守运行。
- Linear 令牌只存在系统钥匙串（macOS 钥匙串、Windows 凭据管理器）。IssueRelay 不上传任何数据，也没有遥测。

## 开发

需要 Rust 1.99：装好 rustup 后在仓库根目录运行 `rustup toolchain install`（按 `rust-toolchain.toml` 安装 1.99.0 及 rustfmt、clippy）。

```bash
cargo test --workspace
cargo run -p issuerelay-devhost -- --help   # 开发用无界面宿主（不发布）；没有登录时也会启动，并提示先登录
cargo run -p issuerelay-cli -- --help       # 命令行
```

提交前的检查、代码规范与开发流程见 [AGENTS.md](AGENTS.md) 和 [docs/ai-coding](docs/ai-coding/README.md)。

## 参与贡献 / Contributing

欢迎 issue 和 PR。动手前请先读 [CONTRIBUTING.md](CONTRIBUTING.md)。
Issues and pull requests are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

- [CONTRIBUTING.md](CONTRIBUTING.md)：开发环境、提交前检查、PR 流程 / setup, checks, pull requests
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)：行为准则 / code of conduct
- [SECURITY.md](SECURITY.md)：私下报告安全问题 / report vulnerabilities privately
- [MAINTAINERS.md](MAINTAINERS.md)：维护者、评审与合并规则 / maintainers and merge rules

## 文档

| 文档 | 内容 |
|---|---|
| [设计](docs/superpowers/specs/2026-10-10-issuerelay-design.md) | 目标、关键决定、整体结构、需求的生命周期 |
| [实施计划](docs/superpowers/plans/2026-10-10-issuerelay-m0-m1.md) | M0–M1 的分步任务 |
| [验收报告](docs/gates/) | 每次实机验收的结果 |
| [AI 编码规范](docs/ai-coding/README.md) | 分层、目录、注释、设计模式、门禁、安全、测试、流程 |

## 许可证

[Apache-2.0](LICENSE)
