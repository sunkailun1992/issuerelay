# 运行改进：实测与验收（2026-10-27）— 自动测试与 macOS 沙箱实机验收通过，Windows 待实机

## 结论

- 实测（设计第 4 节）：2026-10-27 在沙箱里做完，两个执行者都能接续会话，并与精简运行的参数兼容；只出方案能做到「能读、能在 Linear 留言、不能改文件」。样本在 [samples/2026-10-27](samples/2026-10-27/README.md)。
- 自动测试：AGENTS.md 的六条验证命令全部通过（见「自动测试」）。
- 实机验收（设计第 14 节第 1–7 项）：2026-10-28 00:13–00:39（UTC+8）在 macOS 沙箱里全部通过，所有者在场回答了钥匙串的询问。Windows 待实机。
- 合并与发布：所有者 2026-10-28 让合并后，squash 合并进 `main` 为 `752256a`；之后所有者让发版，随 v0.3.0 发布（见 [2026-10-28-v0.3.0-release.md](2026-10-28-v0.3.0-release.md)）。

## 环境

- 实测：macOS 27.0，Apple 芯片；Claude Code 2.1.285；codex-cli 0.159.2；Linear 官方 MCP。临时 git 仓库 `~/issuerelay-sandbox/probe-1027`（实测后删掉了探测写的文件）；Linear 写操作只落在沙箱项目的一条需求上（以 `🤖 IssueRelay probe` 开头的评论，按规则不删，留在那里）。
- 自动测试：分支 `feat/run-improvements`，rustc 1.99.0，Node.js 与 pnpm 按 `app/package.json`。
- 实机验收：
  - 应用：`8a4190f` 构建的调试版应用（`pnpm --dir app tauri build --debug --bundles app`，ad hoc 签名），用 `open` 启动（与托盘程序一样由 launchd 带起，环境是登录 shell 的）。已装的 v0.2.4 没有覆盖，它从 10-26 起就没在运行。第一次读钥匙串时停在「连接中」约 10 分钟，所有者按了「永远允许」后，00:13 连上。
  - 数据：用本机真实的数据目录，先备份 `config.toml` 与运行记录（`state.db` 连同 WAL）。验收期间把那个不是沙箱的绑定从设置里拿掉，免得它的项目被派活；结束后从备份逐字节还原 `config.toml`。运行记录保留这次的作业 #42–#51。
  - 沙箱：原有的沙箱绑定（`~/issuerelay-sandbox/demo`，Claude，自动审查，精简）。分流用的第二个目录 `~/issuerelay-sandbox/demo-b` 另配了一个 bare remote，两次并行的推送不会抢同一个分支。
  - Linear：分流标签用工作区已有的 `Feature`；新建了工作区标签 `ai-plan`（之后留着用）；测试需求 DEMO-49 至 DEMO-55。
  - 环境检查：Claude、Codex 和两者的精简运行都就绪，沙箱绑定 `claude (lean): ok`。
- 面板画面没有逐页目视（所有者 10-22 不让截 IssueRelay 的图）。数据用命令行与 `--json` 核对，面板怎么显示这些字段由组件测试覆盖。

## 实测（设计第 4 节）

| 项 | 做法 | 结果 |
|---|---|---|
| Linear `statusType`、`relations` | `get_issue` 的 `fields` 带上两者；给沙箱需求加一条「被挡住」关系，读完再删掉 | `statusType` 是状态类型（`unstarted` 等）；`relations.blockedBy` 每一项只有编号与标题（样本 `linear/`） |
| Claude 接续 | 新会话记一个词，再用 `--resume` 加精简参数接续 | 照常有 `system/init`，只有 `linear-server`，会话编号不变，记得那个词；`total_cost_usd` 与 `modelUsage` 是会话累计的（0.0551374 → 0.0943946），顶层 `usage` 只是这次 |
| Claude 接续不存在的会话 | `--resume` 一个不存在的编号 | 退出码 1，没有 `system/init`，只有一条 `result`（`error_during_execution`） |
| Claude 只出方案 | `--tools Read,Grep,Glob` | 工具只剩 `Glob`、`Grep`、`Read` 与 Linear；读了文件，留了评论，目录里没有多出文件；不带 `--tools` 接续后写了文件 |
| Codex 接续 | `codex exec resume … <线程编号> -` | 同一线程，记得那个词；没有 `--approve-for-me` 与 `-s` |
| Codex 接续不存在的会话 | 同上，编号不存在 | 退出码 1，标准输出为空 |
| Codex 只出方案 | `-s read-only`；再加 `-c mcp_servers.linear.default_tools_approval_mode=approve`；接续时用 `-c sandbox_mode=read-only` | 只读沙箱拒绝写文件；不加放行设置时 Linear 评论因要审批而失败，加了就成功；接续时同样；用完全放开接续后写了文件 |

## 自动测试

| 命令 | 结果 |
|---|---|
| `pnpm --dir app build` | 通过 |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace` | 通过，1055 个测试（审查后的修正与新测试之后） |
| `pnpm --dir app lint` | 通过 |
| `pnpm --dir app test` | 通过，1219 个测试 |
| `bash scripts/check-secrets.sh --strict` | 没有输出 |
| `pnpm --dir app test:screens`（只在本机） | 通过，144 项；需求、运行记录、项目绑定三页的基准截图与 README 的四张图按新界面重拍 |

覆盖的重点（设计第 13 节）：运行中途需求被取消、完成、改派、删除后停止并写对原因，用户停止仍是 `stopped by the user`；读失败不停；被挡住的需求等待、放开后入队，挡住与否每轮按需重查；两条标签分到两个目录并行；只出方案的运行记成方案已出、只用精简的参数（「按个人设置」沿用用户设置）；重做时新任务带上 `resume_job_id`、参数里有 `--resume`；接续不成时同一任务从头再跑；接续只在 7 天内、同执行者同目录、同模式或方案转动手时发生（「自动审查」的 Codex 平常运行不接续）；用量与会话编号的解析（含第 4 节实测的事件形状）；新列在旧库上补齐、旧版本形状的库照常读；面板各页的新显示、五种语言的键；分流标签的规则与 core 共用用例表 `app/src/lib/routing-cases.json`。

## 全分支审查

自动测试全过之后，派了一个没看过实现过程的审查子智能体对照设计与计划审全分支（`origin/main..98b4eab`），提了 9 条。处理如下（设计里对应处都写明了「全分支审查后」）：

| # | 发现 | 处理 |
|---|---|---|
| 1 | 方案运行仍能用 Linear 的 `merge_diff`、`submit_diff_review` 等工具，作用在需求关联的拉取请求上 | 改：只留读取、`save_comment`、`save_issue`，其余 29 个写的工具关掉（`executor::PLAN_DENIED_LINEAR_TOOLS`，测试按 2026-10-27 的 69 个工具核对）。Claude 用 `--disallowedTools`，实测在 `bypassPermissions` 下生效（样本 `claude/plan-disallowed-tools.jsonl`）。Codex 用 `disabled_tools`，只在精简运行时加；2026-10-28 00:0x 额度恢复后实测，Codex 列出的 Linear 工具从 69 个变成 65 个，关掉的 4 个都不在（样本 `codex/plan-disabled-tools.jsonl`） |
| 2 | 工作区没有 `ai-plan` 标签时，`list_issues` 若报错会让每轮检查失败 | 不改：实测 `list_issues` 给不存在的标签回空列表（`{"issues":[],"hasNextPage":false}`），这个工作区也确实还没有 `ai-plan` |
| 3 | AI 照团队说明把需求转给别人（或团队把待测试状态归为完成类）会被当成改派停掉 | 改：需求在「待测试」状态时不因 Linear 上的改动停运行 |
| 4 | 「按个人设置」的 Codex 方案运行接续时，用户配置的 profile 可能盖过 `-c sandbox_mode=read-only` | 改：这种方案运行不接续，从头开始（新运行用 `-s read-only`，不会被盖过） |
| 5 | 运行已退出、正在核对结果时，一次检查仍会记下停止原因并发出停止，原因留在表里，被同一任务以后手动停止的那次误用，还打断「再读」 | 改：运行登记表多一个「收尾中」标记，在同一把锁里判断；新测试在去掉修复时失败 |
| 6 | 方案运行结束后读需求失败，会被记成「无法核对」的失败 | 改：方案运行的结果只看它自己的报告，读失败只记日志 |
| 7a | 开始前重读需求时用户停止又重新运行，这次开始仍会接续旧会话 | 改：登记运行前再读一次行上的 `resume_job_id`，变了就交给下一轮；新测试在去掉修复时失败 |
| 7b | 旧版本重新运行过的行，升级回来仍按旧的 `mode` 与 `resume_job_id` 走 | 改：与花费、会话同一条规则，`details_started_at` 对不上 `started_at` 时都按没有读 |
| 8 | 一条读不到的挡住需求会让整轮检查失败，所有项目都停 | 改：这条需求沿用上次的结论，下一轮再读；新测试在去掉修复时失败 |
| 9 | Linear 上改了名的分流标签，需求会默默去默认绑定 | 改：绑定页的卡片标出 Linear 已经没有的分流标签。命令行 `bind add --label` 按 `id` 替换的行为照设计保留（`--id` 可以另起） |

## 实机验收（设计第 14 节）

在沙箱 Linear 项目与 `~/issuerelay-sandbox` 上，用本分支构建的应用逐项跑。

| 项 | 做法 | 结果 |
|---|---|---|
| 1 Linear 上取消 | DEMO-49：AI 把需求改成 In Progress 后（00:14:30），在 Linear 上改成 Canceled（00:14:43） | 通过：00:15:01 的检查停掉运行，主机日志 `stopping the run: its issue changed in Linear`，作业 #42 记成「已取消」，原因 `the issue was canceled in Linear`；没有残留的执行者进程。AI 已在 feature 分支提交了 5 次，还没合并、推送，说明是在运行中途停下的。停下后目录留在那个 feature 分支上（与用户自己停止时一样），验收时手动切回 `test` |
| 2 用量 | 作业 #43、#44 的记录对照各自运行日志的最后一行 `result` | 通过：花费（`total_cost_usd` 0.3940442 → 394044 微美元；0.41221 → 412210）、输入与输出 token、缓存读写、轮数（17）与会话编号逐项相同。`issuerelay jobs` 的花费栏显示 `$0.39`、`$0.41` |
| 3 被挡住 | DEMO-51 被 DEMO-50（没有触发标签）挡住；之后把 DEMO-50 设为 Done（00:18:30） | 通过：DEMO-51 先停在「等待」，原因 `blocked by DEMO-50, which are not done yet`。00:20:27 入队并开始，00:22:17 待验收，AI 的摘要里写了挡住它的需求已完成。放行比设为 Done 晚了约 2 分钟：「被挡住」的结论按设计第 8 节缓存 2 分钟 |
| 4 分流 | 给沙箱项目加分流绑定 `#Feature`（`demo-b`）；DEMO-52 带 `Feature`，DEMO-53 不带 | 通过：DEMO-52 用 `…#Feature` 绑定在 `demo-b` 跑，DEMO-53 用默认绑定在 `demo` 跑，两次同时进行（00:16–00:18）。`route-b.txt` 只在 `demo-b` 的 remote 上，`route-a.txt` 只在 `demo` 的 remote 上 |
| 5 只出方案 | DEMO-54 只带 `ai-plan`（默认绑定，Claude）；DEMO-55 带 `ai-plan` 与 `Feature`（分流绑定改成 Codex、完全放开） | 通过：两条都在 Linear 下留了以「🤖 IssueRelay」开头的方案评论，去掉了 `ai-plan`，状态留在 Todo；两个目录的 HEAD、分支与文件都没变；作业 #46、#47 记成「待补充信息」，`mode = plan`、`report = planned`（面板显示「方案已出」）。Claude 的 `system/init` 只有 `Glob`、`Grep`、`Read` 与 40 个 Linear 工具，`merge_diff` 不在；两次运行用到的 Linear 工具只有 `get_issue`、`list_comments`、`save_comment`、`save_issue`。Codex 在只读沙箱里跑了 `rg`、`python3` 读文件，没有写 |
| 6 接续 | DEMO-54、DEMO-55 加上 `ai-ready`；DEMO-53 退回 Todo 并补一句要求；之后 DEMO-53 改成只带 `ai-plan` | 通过：作业 #49（Claude）与 #48（Codex）的运行日志第一行分别是 `this run continues the session of run 46` 与 `… run 47`，会话（线程）编号与方案那次相同，Claude 的工具恢复完整；#49 自己的花费 $1.30 是会话累计 $2.90 减去方案那次的 $1.60。退回重做的作业 #50 接续 #43 的会话。平常运行之后的方案运行 #51 从头开始，日志写着 `this run starts afresh: a planning run does not continue the session of a usual run` |
| 7 退回 v0.2.4 | 在新版里用 `bind remove` 删掉分流绑定，结束调试版应用，打开已装的 v0.2.4 | 通过：v0.2.4 的 `issuerelay jobs` 列出新版写的 #42–#51，方案已出的显示成 `needs_info`（设计第 5 节预料的样子）。设置里新版写入的 `settings.resume_sessions`、`settings.trigger.plan_label` 只得到一条「不认识的键，忽略，下次保存时丢掉」的提醒。约 50 秒连上 Linear，没有再问钥匙串。验收完结束 v0.2.4，还原设置 |
| — Windows | 同上各项 | 待实机（只有 CI） |

## 与设计不一致之处

没有与设计相反的地方。实现时定下、已写回设计的几处：

- 只出方案的运行遇到「按个人设置」的绑定时，沿用用户自己的设置与 `dontAsk`，只收紧内置工具（实现后的自动安全审查指出原先改成「完全放开」越过了用户自己收紧的权限；设计 W8 与第 10.2 节已改）。
- 分流标签从 Linear 的标签里选：为此加了一条只读的面板命令 `labels`（命令表、应用清单与能力文件同步登记）；新绑定的 `id` 被占用时在后面加 `-2`、`-3`……（设计第 9.3 节）。
- 运行详情给 Linear 停掉运行的四种原因各配了翻译好的标题，引擎的原句在下面一行（设计第 6.3 节）。
- `issuerelay jobs` 的花费写法同面板：`$0.09`，不到一分写 `<$0.01`，没有花费的留空（设计第 12 节）。

## 实机验收中看到的

- DEMO-55 的 Codex 方案评论写了「环境未提供 llm-wiki MCP」：Codex 的精简运行照样读用户级的 `~/.codex/AGENTS.md`（精简运行验收的 L6，README 已写明），那里的说明要它查记忆。不影响结果。
- Claude 的方案运行 DEMO-54 读了很多文件，用了约 6 分钟、$1.60。只出方案的花费可能比动手那次还高。

## 未闭环

- Windows 只靠 CI。
- 面板画面没有逐页目视（见「环境」）。
- 合并与发布：已做（`752256a`，v0.3.0）；发布说明写了退回旧版本前先删掉分流绑定（安装说明「退回旧版本」）。
