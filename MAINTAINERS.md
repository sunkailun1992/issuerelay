# 维护者与治理

## English summary

- Current maintainer: @sunkailun1992 (admin, project founder), responsible for everything.
- Roles: contributor (anyone who opened an issue or PR), maintainer (write or maintain permission: reviews and merges PRs, triages issues) and admin (admin permission: settings, rulesets, secrets, releases).
- A maintainer other than the author approves a PR before it is squash-merged. High-risk changes need two maintainers once there are two; until then the admin reviews and writes the reason in the PR. The admin has the final say.
- `main` is protected by a ruleset: PRs only, one approval, resolved conversations, squash merge only, and the checks `test (macos-latest)`, `test (windows-latest)` and `secrets` must pass.
- Admins can bypass the ruleset in two cases only: the founder's direct commits during M0–M1, and merging the admin's own PRs while there is a single maintainer (see "分支保护").
- To become a maintainer: contribute steadily, get nominated by a maintainer and accepted by the admin, then receive write permission. Maintainers inactive for 6 months become emeritus.
- Releases start at M3. The update-signing key lives only in GitHub Actions secrets.

## 当前维护者

| GitHub 账号 | 角色 | 负责范围 |
|---|---|---|
| @sunkailun1992 | 管理员（项目发起人） | 全部 |

## 角色

- **贡献者**：提过 issue 或 PR 的任何人。
- **维护者**：有 GitHub write 或 maintain 权限。评审与合并 PR，分诊 issue。
- **管理员**：有 admin 权限。管理仓库设置、规则集、密钥与发布。

私密报告（漏洞报告和行为准则举报）默认只有管理员能看到。需要时，管理员可以邀请维护者加入某一份报告。

## 维护者职责

- 评审 PR。
- 分诊 issue：加标签，关闭重复的。
- 守住 [AGENTS.md](AGENTS.md) 的硬规则和公开仓库红线。
- 保持 CI 可用。
- M3 起负责发布。

## 如何成为维护者

1. 持续贡献。例如合并过 5 个以上有分量的 PR，或长期参与评审与分诊。
2. 现有维护者提名，管理员同意。
3. 在本文件的「当前维护者」表里加一行，并在 GitHub 授予 write 权限（之后可升为 maintain）。

## 决策与合并

- **日常改动**：由作者以外的一名维护者批准即可合并。
- **只有一名维护者期间**：管理员自己的 PR 在 CI 全绿后经绕过合并，并在 PR 里写明理由。
- **高风险范围**（见 [CONTRIBUTING.md](CONTRIBUTING.md) 的「开始之前」）：维护者达到两名后，需要两名维护者批准。只有一名维护者时，由管理员评审，并在 PR 里写明理由。GitHub 规则集只强制 1 人批准，第二个批准由维护者自己把关。
- **squash 合并**：合并前确认 PR 标题符合英文 Conventional Commits，它就是 `main` 上的提交标题。再把提交正文精简成英文摘要，删掉模板里的中文标题、注释和未勾选项。
- **设计变更**：先改 `docs/superpowers/specs/` 里的设计文档，并在 PR 里讨论。
- **意见分歧**：由管理员最终决定。

## 分支保护

`main` 受规则集保护。仓库设置不在仓库文件里，现状只记在本节；改设置时，同步改这一节。

| 项目 | 现状 |
|---|---|
| 规则集 | `main`，作用于默认分支 |
| 删除与强推 | 禁止 |
| 合并方式 | 必须经 PR，只允许 squash |
| 批准 | 至少 1 人批准；批准后再有新推送，需要重新批准 |
| 评审对话 | 必须全部解决 |
| 必需检查 | `test (macos-latest)`、`test (windows-latest)`、`secrets`（GitHub Actions，见 `.github/workflows/ci.yml`） |
| 代码所有者 | `.github/CODEOWNERS` 自动请求评审，不强制代码所有者批准 |
| 绕过 | 仓库管理员可以绕过 |
| 仓库合并设置 | 只允许 squash 合并；提交标题取 PR 标题，正文取 PR 描述；合并后自动删除分支 |
| 安全功能 | 已开启：私密漏洞报告、Dependabot 安全更新、密钥扫描与推送拦截 |
| Discussions | 未开启 |

「仓库管理员可以绕过」只用于两种情况：

1. M0–M1 期间，项目发起人直接提交开发进度。M1 收尾时停止，届时更新本节。
2. 只有一名维护者期间，合并管理员自己的 PR。

除此之外不使用绕过。

## 发布

M3 起，由维护者推送 `vX.Y.Z` 标签，触发 GitHub Actions 打包发布。更新包的签名私钥只放在 GitHub Actions secrets，只有管理员能改。

## 离任

连续 6 个月不活跃的维护者转为荣誉维护者，收回写权限。随时可以回来。
