# 安全政策

## English summary

- Report security problems privately: open the Security tab and click "Report a vulnerability". Never open a public issue for them.
- Include the affected version or commit, steps to reproduce and the impact. Do not paste real tokens, and revoke any token that has leaked.
- Supported versions: `main` until the first release; after that the latest release and `main`.
- In scope: credential leaks, the "mark done" boundary, local IPC access by other users, command injection from Linear text, leftover child processes, the fixed prompt rules and the secret scanner; an update that is installed although it fails the signature check against the committed public key (or whose signed version differs from the manifest), or that is older than the running version; the release workflow leaking the update-signing key or a write token; and "add the CLI to PATH" changing or deleting a file or `Path` entry it did not create.
- Out of scope: what the AI does in a repository the user bound with the full-access tier (a documented risk), and bugs in Claude Code, Codex or Linear themselves.
- This is a volunteer project. We aim to acknowledge a report within 7 days, publish a security advisory after the fix and credit you if you wish.

## 支持的版本

- 发布第一版之前：只支持 `main`。
- 发布之后：支持最新发布版本和 `main`。

## 如何报告

请私下报告，不要开公开 issue。本仓库已开启私密漏洞报告。

1. 打开本仓库的 Security 页面，点「Report a vulnerability」（[直达链接](https://github.com/sunkailun1992/issuerelay/security/advisories/new)）。
2. 写清影响的版本或提交、复现步骤、影响范围。
3. 报告里不要放真实令牌。已经泄露的令牌，先去对应服务吊销。

## 关注范围

下面这类问题在范围内，我们最关心。它们对应 [AGENTS.md](AGENTS.md) 的硬规则和 [SECURITY_CODING_SPEC.md](docs/ai-coding/SECURITY_CODING_SPEC.md)：

- 令牌或凭据被写进日志、文件或进程参数
- 「标记完成」可以经 IPC、命令行或 MCP 触发
- 本机 IPC 能被其他用户访问
- Linear 文本被拼进子进程参数，或经 shell 执行
- 结束运行后留下子进程
- 提示词的三条固定规则被程序本身删改或绕过
- `scripts/check-secrets.sh` 可被绕过，导致泄露
- 没通过提交公钥验签的更新被安装（含签名里记的版本与清单不符的），或比正在运行的版本更旧的更新被安装
- 发布工作流让更新签名私钥或可写令牌落到不该去的地方
- 「把命令行加入 PATH」改动或删除了不是它建的文件或 `Path` 条目

## 不在范围内

- 用户自己选择「完全放开」档位后，AI 在用户绑定的仓库里按需求内容执行命令。这是文档写明的风险模型，见 [README.md](README.md) 的「安全须知」。
- Claude Code、Codex、Linear 自身的漏洞：请报给对应厂商。

## 响应

这是志愿维护的项目。我们尽力在 7 天内确认收到你的报告。修复后发布安全公告，你愿意的话，公告里会致谢。
