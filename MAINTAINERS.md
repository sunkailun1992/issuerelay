# 维护者与治理

## English summary

- Current maintainer: @sunkailun1992 (admin and repository owner, project founder), responsible for everything.
- Roles: contributor (anyone who opened an issue or PR), maintainer (a collaborator invited with write access: reviews and merges PRs, triages issues, approves workflow runs for first-time contributors) and admin (the repository owner: settings, rulesets, secrets, releases).
- The repository belongs to a personal account, so GitHub offers only two levels: the owner (admin rights) and collaborators with write access. Maintain and admin-style roles for others become available only if the project moves to a GitHub organization, and this file is updated then.
- A maintainer other than the author approves a PR before it is squash-merged. Changes in the Critical (极高风险) areas need two maintainers once there are two; until then the admin reviews and writes the reason in the PR. The admin has the final say.
- `main` is protected by a ruleset: PRs only, one approval, resolved conversations, squash merge only, and the checks `test (macos-latest)`, `test (windows-latest)` and `secrets` must pass.
- The admin can bypass the ruleset in two cases only: the founder's direct commits during M0–M1, and merging the admin's own PRs while there is a single maintainer (see "分支保护"). Milestones: M0 verification, M1 engine and CLI, M2 desktop app, M3 installers and releases (design section 17).
- To become a maintainer: contribute steadily, get nominated by a maintainer and accepted by the admin, then be invited as a collaborator with write access. Maintainers inactive for 6 months become emeritus.
- Releases start at M3. Maintainers bump the version when they release; contributors never do. The update-signing key lives only in GitHub Actions secrets.

## 当前维护者

| GitHub 账号 | 角色 | 负责范围 |
|---|---|---|
| @sunkailun1992 | 管理员（项目发起人） | 全部 |

## 角色

- **贡献者**：提过 issue 或 PR 的任何人。
- **维护者**：被邀请的协作者，有 write 权限。评审与合并 PR，分诊 issue。
- **管理员**：仓库所有者，有 admin 权限。管理仓库设置、规则集、密钥与发布。

本仓库属于个人账号，GitHub 只提供「所有者」和「有 write 权限的协作者」两种角色。所以只有所有者能绕过分支保护、管理私密安全公告。maintain 这样的中间角色，以及多人共有的管理员，要等项目迁到 GitHub 组织才有，届时更新本节。

私密报告（漏洞报告和行为准则举报）默认只有管理员能看到。需要时，管理员可以邀请维护者加入某一份报告。

## 维护者职责

- 评审 PR。
- 批准首次贡献者的工作流运行：仓库设置要求，首次贡献者的 PR 要先经维护者批准，CI 才会开始跑。
- 分诊 issue：加标签，关闭重复的。
- 守住 [AGENTS.md](AGENTS.md) 的硬规则和公开仓库红线。
- 保持 CI 可用。
- M3 起负责发布。里程碑 M0–M3 的含义见 [README.md](README.md) 的状态说明和[设计文档](docs/superpowers/specs/2026-10-10-issuerelay-design.md)第 17 节。

## 如何成为维护者

1. 持续贡献。例如合并过 5 个以上有分量的 PR，或长期参与评审与分诊。
2. 现有维护者提名，管理员同意。
3. 在本文件的「当前维护者」表里加一行，并由管理员邀请对方成为协作者，授予 write 权限。个人账号的仓库没有 maintain 角色，项目迁到 GitHub 组织后才可以改授 maintain，届时更新本节。

## 决策与合并

- **日常改动**：由作者以外的一名维护者批准即可合并。
- **只有一名维护者期间**：管理员自己的 PR 在 CI 全绿后经绕过合并，并在 PR 里写明理由。
- **极高风险范围**（见 [CONTRIBUTING.md](CONTRIBUTING.md) 的「开始之前」）：维护者达到两名后，需要两名维护者批准。只有一名维护者时，由管理员评审，并在 PR 里写明理由。GitHub 规则集只强制 1 人批准，第二个批准由维护者自己把关。
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

M3 起，由维护者推送 `vX.Y.Z` 标签，触发 GitHub Actions 打包发布。发布时由维护者升版本号，外部贡献者的 PR 不改版本号。更新包的签名私钥只放在 GitHub Actions secrets，只有管理员能改。

## 离任

连续 6 个月不活跃的维护者转为荣誉维护者，收回写权限。随时可以回来。
