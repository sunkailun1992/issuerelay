# IssueRelay 运行改进设计（参考同类项目）

- 日期：2026-10-27
- 状态：所有者 2026-10-27 定下范围（「按推薦都實現吧」，第 3 节 W1–W3）；本文其余细节是设计者的取舍，标为「设计者定」，所有者可以改。
- 范围：六项改进，来自同类项目的比较（第 1 节）。动到触发与防重复规则、执行者参数与提示词，属于极高风险（`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 2 节）：实现分项提交，两个平台都跑测试，并在沙箱上实机验收（第 14 节）。观察期照旧：不发版，发版等所有者说。
- 实测环境：macOS 27.0、Claude Code 2.1.285、codex-cli 0.159.2、Linear 官方 MCP，2026-10-27 在沙箱目录和沙箱 Linear 项目里实测（第 4 节）。

## 一页概览

| # | 改进 | 一句话 |
|---|---|---|
| 1 | 需求在 Linear 上被取消、完成、改派或删除时，停掉正在跑的运行 | 每次检查顺带重读运行中的需求；被人取消或改派就结束整棵进程树，记为「已取消」并写明原因 |
| 2 | 每次运行记下花费、token 与轮数 | 从执行者自己的输出里读：Claude 有花费、token、轮数，Codex 只有 token；面板运行详情显示 |
| 3 | 被未完成的需求挡住时先不派 | 需求有「被 X 挡住」的关系且 X 还没完成或取消，就等着，原因写明是哪条 |
| 4 | 按标签分到不同目录 | 同一个 Linear 项目可以有多个绑定，各认一组标签；需求按标签进对应目录，各自排队 |
| 5 | 只出方案 | 带 `ai-plan` 的需求，AI 只读需求和代码、在 Linear 留方案，不改代码；技术上收掉写文件和执行命令的工具 |
| 6 | 重做时接续上一次的会话 | 需求退回重做（或出完方案再加 `ai-ready`）时，接着上一次运行的会话继续，不从头开始 |

不做：「别人建的需求先在面板确认」（所有者 2026-10-27 不要）；每条需求一个独立 worktree、由 IssueRelay 代理 Linear 工具（只记下，以后再议）。

## 1. 背景与目的

2026-10-27 所有者问有没有可参考的同类项目来完善 IssueRelay。比较过的项目与它们的做法：

- **openai/symphony**（Apache-2.0，规格加 Elixir 原型）：轮询 Linear、每条需求一个工作目录；每轮回查运行中的需求，变成终态就停；按 `priority`、创建时间排派活顺序；记 token 与运行时长；需求模型带 `blocked_by`；续跑时把后续指示发进同一个会话。
- **cyrusagents/cyrus**（Apache-2.0）：webhook 触发的 Linear 代理；`routingLabels` 按标签把同一团队或项目的需求分到不同仓库；`labelPrompts` 按标签切换模式（如只做需求分析的 scoper），并按模式收紧可用工具。
- **BloopAI/vibe-kanban**、**OrchestratorInc/agent-orchestrator**：本机多代理看板；评审意见与失败回到原来那个代理的会话里继续。
- Linear 自己的 coding sessions、Codex 的 Linear 整合都在云端跑，碰不到本机环境；IssueRelay「在自己电脑上跑」的定位不变。

IssueRelay 已有的：派活按 Linear 优先级与创建时间排序（`scheduler.rs`）；排队、等待中的任务在需求不再符合条件时取消（`engine.rs` `reconcile`）。缺的是本文的六项。

## 2. 目标、非目标与反目标

**目标**

1. 需求在 Linear 上被人取消、完成、改派或删除时，运行最迟在下一次检查（默认 30 秒）时停下，整棵进程树结束，运行记录写明原因。
2. 每次运行记下执行者自报的花费、token 与轮数，面板能看到；记录在程序重启、日志被清理后仍在。
3. 被挡住的需求不派活；挡住它的需求完成或取消后，下一次检查自动入队。
4. 一个 Linear 项目能按标签分到几个目录；同一条需求不会同时进两个目录。
5. 「只出方案」的运行不能改文件、不能执行命令（技术上收掉相应工具，不只靠提示词），但能在 Linear 留方案。
6. 重做时接续上一次的会话；接续不了（会话已不在、执行者或目录变了）就从头开始，不卡住、不失败。
7. 升级后能退回 v0.2.4 照常使用：运行记录库不升版本号，不加新的任务状态值（第 5 节）。

**非目标**

- 失败后自动重试（总设计第 1.2 节第 4 条：失败不自动重试）。
- webhook、Linear 的 Agent API（要公网入口，总设计第 2.2 节）。
- 每条需求一个独立 worktree、IssueRelay 代理 Linear 工具。
- 按绑定另设「接续会话」开关、另设被挡住检查的开关（设计者定：先只有全局开关，见第 11.5 节；被挡住检查总是开着）。
- 暂停监控时也回查运行中的需求（暂停就是不查 Linear，见第 6.4 节）。
- Windows 实机验收（暂无机器，照旧只靠 CI）。

**反目标**（做成这样就算失败）

- 用户在面板上点「停止」与 Linear 上取消分不清，或 Linear 上的一次正常改动（例如 AI 自己去掉 `ai-ready`）把运行停了。
- 花费被说成实际扣费：订阅登录时它是执行者按 API 价格算的估计。
- 只出方案的运行改了文件，或推了分支。
- 接续了别的目录、别的执行者的会话，或接续失败后运行直接记成失败。
- 退回旧版本后运行记录打不开，或旧版本把记录读错。
- 为此在执行者参数里放入来自 Linear 的文本。

## 3. 已定的决定

| # | 决定 | 来源 |
|---|---|---|
| W1 | 做第 1–6 项；「别人建的需求先确认」不做，理由是默认只认指派给自己的需求 | 所有者 2026-10-27 |
| W2 | 「按标签分流」两种都做：按标签分到不同目录，和「只出方案」 | 所有者 2026-10-27 |
| W3 | 观察期照旧：不发版；独立 worktree、代理 Linear 工具只记下 | 所有者 2026-10-27、10-22 |
| W4 | 运行记录库只加可空的列、不升版本号；不加新的任务状态值，「只出方案」的结果记为 `needs_info` 加运行模式 | 设计者定（第 5 节） |
| W5 | 只出方案：Claude 用 `--tools Read,Grep,Glob` 收掉写文件与执行命令的内置工具；Codex 用只读沙箱，并放行 Linear 的工具调用 | 设计者定，按第 4 节实测 |
| W6 | 接续会话默认开，设置里一个全局开关；手动「重新运行」总是从头开始 | 设计者定 |
| W7 | 被挡住：只认 Linear 的「被 X 挡住」（`blockedBy`）关系，X 的状态类型是 `completed` 或 `canceled` 才算放开 | 设计者定 |
| W8 | 只出方案按精简运行启动（只带 Linear 一个 MCP 服务，不读用户设置），不看绑定的精简开关；做不到精简时（绑定目录就是主目录）等待。「按个人设置」的绑定例外：沿用用户自己的设置与 `dontAsk`，只收紧内置工具（实现后的安全审查，2026-10-27） | 设计者定，按实现前审读（第 15 节）与安全审查 |
| W9 | 「被挡住」只挡平常的运行；方案运行照常派，先把方案想好正是它的用处 | 设计者定，同上 |
| W10 | 接续只在同一模式之间，以及「方案之后的平常运行」；平常运行之后的方案运行从头开始 | 设计者定，同上 |
| W11 | 精简核对：Claude 在 `system/init` 之前出现的 `result` 事件不算精简没生效（会话没开始就结束了，什么都没做），这样接续不成能被认出来 | 设计者定，同上 |

## 4. 实测（2026-10-27）

做法：在 `~/issuerelay-sandbox/probe-1027`（临时 git 仓库）里，用与托盘程序相同的干净环境（登录 shell 的 PATH、系统代理）和 IssueRelay 会拼出的参数（精简运行）直接运行执行者；Linear 写操作只落在沙箱项目的需求上。

### 4.1 Linear 官方 MCP

- `list_issues` 与 `get_issue` 的 `fields` 都有 `statusType`，值是 Linear 的状态类型：`triage`、`backlog`、`unstarted`、`started`、`completed`、`canceled`。
- 只有 `get_issue` 的 `fields` 有 `relations`。工具说明写明「`fields` 非空时覆盖 include 选项，可选字段直接写进 `fields`」，IssueRelay 总是传 `fields`，所以用 `"relations"` 这个字段，不用 `includeRelations`。
- `relations` 的形状：`{"blocks":[],"blockedBy":[{"id":"KEL-44","title":"…"}],"relatedTo":[],"duplicateOf":null}`。`blockedBy` 的每一项只有编号和标题，没有状态：要知道挡住它的需求是否完成，得再读一次那条需求。
- 给需求加、删「被挡住」关系会更新这条需求的 `updatedAt`；挡住它的需求完成时不会（这条没实测，按 Linear 的数据模型推断），所以等着的任务要每轮重查（第 8 节）。

### 4.2 Claude Code 2.1.285

- 接续：`--resume <会话编号>` 与精简运行的全部参数一起用，照常先出 `system/init` 事件，会话编号不变，`mcp_servers` 仍只有 `linear-server`，精简核对照常生效；模型记得上一次的内容。
- 接续一个不存在的会话：标准错误输出 `No conversation found with session ID: …`，没有 `system/init`，有一条 `result`（`error_during_execution`），退出码 1。
- 花费：`result` 事件的 `total_cost_usd` 与 `modelUsage` 是整个会话累计的（接续那次的 0.0943946 = 第一次的 0.0551374 + 这次的 0.0392572）；顶层 `usage` 只是这次运行的 token。`modelUsage` 里的 `costBasis` 是 `unknown`：订阅登录时这是估计，不是扣费。
- 只出方案：`--tools Read,Grep,Glob` 加 `bypassPermissions`：`system/init` 的工具只剩 `Glob`、`Grep`、`Read` 和 Linear 的工具；模型读了文件、在沙箱需求下留了评论，没有写文件和执行命令的工具，目录里没有多出文件。
- 方案之后接着动手：不带 `--tools` 接续那个方案会话，`system/init` 列出完整的工具，模型照指示写了文件。

### 4.3 codex-cli 0.159.2

- 接续：`codex exec resume [选项] <会话编号> -` 接受 `--json`、`-c`、`--disable`、`--ignore-user-config`、`--skip-git-repo-check`、`-m`、`--dangerously-bypass-approvals-and-sandbox`，最后的 `-` 从标准输入读提示词；`thread.started` 的 `thread_id` 与原会话相同，模型记得上一次的内容。没有 `--approve-for-me`，也没有 `-s`。
- 接续一个不存在的会话：标准输出什么都没有，标准错误 `Error: thread/resume: thread/resume failed: no rollout found for thread id …`，退出码 1。
- token：`turn.completed` 的 `usage` 有 `input_tokens`、`cached_input_tokens`、`cache_write_input_tokens`、`output_tokens`、`reasoning_output_tokens`；没有花费。接续那次的 `input_tokens` 含整段历史，是这次真正处理的量。
- 只出方案：`-s read-only`（不加放开参数）时，写文件被沙箱拒绝，`echo` 这类命令照样能跑；但 Linear 的 `save_comment` 失败：`MCP tool call requires approval, but approval policy is never`。
- 再加 `-c mcp_servers.linear.default_tools_approval_mode=approve`（Codex 配置里 MCP 服务的工具审批方式，取值有 `auto`、`prompt`、`writes`、`approve`；值不是合法的 TOML 时按字面字符串用）：写文件仍被拒绝（`operation not permitted`），Linear 评论成功。
- 接续时没有 `-s`，用等价的 `-c sandbox_mode=read-only` 加上同样的放行设置：同一线程编号，写文件被拒绝，评论成功。
- 方案之后接着动手：用 `--dangerously-bypass-approvals-and-sandbox` 接续那个只读线程，模型写了文件。

### 4.4 结论

1. 两个执行者都能接续会话，并且与精简运行的参数兼容；接续不成时都以非 0 退出、且没有会话开始的事件（Claude 的 `system/init`，Codex 的 `thread.started`），可以据此判断（第 11.4 节）。
2. 只出方案能在技术上做到「能读、能在 Linear 留言、不能改文件」：Claude 用 `--tools Read,Grep,Glob`；Codex 用只读沙箱（新运行 `-s read-only`，接续 `-c sandbox_mode=read-only`）加放行 Linear 工具调用。Codex 的只读沙箱仍能执行不写文件的命令（例如 `git log`），这对读代码有用，也不会改动目录。
3. Claude 的花费是会话累计的，接续那次自己的花费要减去上一次的累计（第 7.2 节）。
4. 订阅登录时花费是估计（`costBasis: unknown`）。

## 5. 兼容（退回旧版本）

- **运行记录库**（`state.db`）：新增的都是可空列，每次打开时在缺的地方 `ALTER TABLE … ADD COLUMN`，`SCHEMA_VERSION` 保持 2。旧版本按列名读写，看不见新列；它新写的行里新列是空的，新版本当作「没有这项信息」。规则写进 `store.rs`：只加可空列、旧版本可以忽略的改动不升版本号；旧版本读不懂的改动（新的状态值、改了含义的列）必须升版本号。
- **不加新的任务状态值**：旧版本遇到不认识的状态会让整张表读不出来（`row_to_job`），退回后面板和命令行都会出错。所以「方案已出」记为 `needs_info`，靠新列 `mode`（`plan`）和 `report`（AI 自报的结果）区分；旧版本把它显示成「待补充信息」，意思相近。
- **配置文件**：新字段都有默认值，缺省时不写入文件，旧文件照常读。旧版本读到新字段会提醒不认识的键、下次保存时丢掉；同一项目有多个绑定会让旧版本的校验失败而启动不了。所以发布说明与安装说明要写：退回旧版本前，先删掉按标签分出来的绑定。
- **本机通信与面板**：新增字段都是可选的，没有任何结构用 `deny_unknown_fields`。新的等待原因代码只给同版本的面板用。

新增的列：

| 列 | 类型 | 含义 |
|---|---|---|
| `mode` | TEXT | `plan`：只出方案的运行；空：平常的运行 |
| `report` | TEXT | AI 最后一行 JSON 的 `result`（`in_testing`、`needs_info`、`failed`、`planned`），读不出时为空 |
| `session_id` | TEXT | 执行者的会话编号（Claude 的 `system/init`，Codex 的 `thread.started`），没见到时为空 |
| `resume_job_id` | INTEGER | 要接续哪次运行的会话；开始时接续不成就清掉（第 11 节） |
| `cost_micro_usd` | INTEGER | 这次运行的花费，单位百万分之一美元（Claude 才有） |
| `session_cost_micro_usd` | INTEGER | Claude 报的整个会话累计花费，算接续那次自己的花费用 |
| `input_tokens`、`output_tokens`、`cache_read_tokens`、`cache_write_tokens` | INTEGER | 这次运行的 token |
| `turns` | INTEGER | Claude 的 `num_turns`（Codex 没有对应的数，空着） |
| `details_started_at` | TEXT | 记下 `report`、`session_id` 与用量时这次运行的 `started_at`；与行上的 `started_at` 不同就不认这些值 |

花费用整数存，`Job` 等结构才能继续实现 `Eq`；显示时换算成美元。

最后一列防的是：退回 v0.2.4 后在面板上「重新运行」一条记录，旧版本不认识这些列，不会清空它们，运行结束后行上还留着上一次的会话与用量；再升级回来时，这些值的 `details_started_at` 与新的 `started_at` 对不上，读出来就当没有，接续也就不会拿到过期的会话。

## 6. 运行中的需求在 Linear 上被取消、完成、改派或删除

### 6.1 什么时候停

每次检查（`poll_inner`）在处理完候选需求之后，读一遍正在运行的需求（每条一次 `get_issue`，最多同时运行数那么多条）：

| Linear 上的情况 | 原因（存进运行记录） |
|---|---|
| 需求不存在了（Linear 报「Could not find referenced Issue.」） | `the issue no longer exists in Linear` |
| 状态类型是 `canceled`（取消、重复） | `the issue was canceled in Linear` |
| 状态类型是 `completed`（有人把它设成了 Done 一类的状态） | `the issue was completed in Linear` |
| 负责人不再是当前用户（含没有负责人） | `the issue was reassigned in Linear` |

不看标签：AI 在「需要补充信息」与「失败」两条路上会自己去掉 `ai-ready`，这时运行正要结束。也不看「回到 Todo」：同理。需求的状态是设置里的「待测试」状态时一概不停（全分支审查后加）：AI 把需求改到这里之后运行就要结束，团队说明可能让它顺手把需求交给审查的人，有的团队还把这个状态归为 `completed` 类。执行者已经退出、正在核对与记录结果的运行（`finish_job`）也不再停：`watch_running` 在运行登记表的锁里查它是否已在收尾，收尾开始时 `finish_job` 先在同一把锁里标上，所以停止原因要么在收尾取用之前记下，要么不记（审查发现原先在收尾期间记下的原因会留在表里，被同一任务以后手动停止的那次误用）。读失败时不停，只记一行日志，下一轮再查。旧的回复里没有 `statusType` 时，只拿状态名与设置里的「完成」状态名比，相同就算 `completed`。

### 6.2 怎么停

停的方式与用户点「停止」相同（`stop_run`：取消令牌，运行器结束整棵进程树），只是先把原因记进引擎内存里的一张表（任务编号 → 原因）。`finish_job` 看到退出方式是「被取消」且表里有原因时，用这条原因代替 `stopped by the user`；不论有没有用上，结束时都把这一项删掉。运行恰好同时自己结束（退出方式不是「被取消」）时，原因不用。

### 6.3 面板与通知

运行记录照常是「已取消」，面板把这几条原因翻译成各自的说明（例如「需求在 Linear 上被取消，已停止运行」）。不加新的通知（设计者定：用户自己点「停止」也没有通知；需要时以后再加开关）。

### 6.4 暂停时

暂停监控时引擎不查 Linear（`poll_once` 直接返回），所以这项检查也不做；恢复后的第一次检查补上。写进使用说明。

## 7. 用量：花费、token 与轮数

### 7.1 从哪里读

运行器逐行读执行者输出时，除了最后的消息（`FinalTextTracker`），一并记下：

- Claude：`system/init` 的 `session_id`；`result` 的 `total_cost_usd`（会话累计）、顶层 `usage` 的 `input_tokens`、`output_tokens`、`cache_read_input_tokens`、`cache_creation_input_tokens`（这次运行）、`num_turns`。
- Codex：`thread.started` 的 `thread_id`；每个 `turn.completed` 的 `usage` 累加：`input_tokens`、`output_tokens`、`cached_input_tokens`（记作快取读取）、`cache_write_input_tokens`（记作快取写入）。Codex 的 `input_tokens` 已经包含快取读取的部分，Claude 的不包含：显示时分别注明（第 12 节）。

数字不是有限的非负数、或字段类型不对，就当没有。会话编号只取上面两种事件（Claude 接续失败时的 `result` 也带着会话编号，但那个会话并没有开始，见第 4.2 节）。

### 7.2 存与算

`RunOutput` 多两项：`usage`（以上数字，都没有时为空）与 `session_id`。`finish_job` 记录结果时一并写进新列。这次运行自己的花费：没有接续时就是 `total_cost_usd`；接续了第 N 次运行的会话时，是这次的累计减去第 N 次记下的累计（减出负数或第 N 次没有累计值时为空）。手动「重新运行」与重新开始时清空这些列（`requeue`、`mark_started`）。

### 7.3 显示

- 运行详情：「用量」一行：花费（Claude 才有，标「估算」并说明订阅登录时不是实际扣费）、输入与输出 token、快取读取与写入、轮数（Claude 才有）；耗时照旧。
- 运行记录列表：有花费时在耗时旁显示花费。
- 命令行 `issuerelay jobs` 的表格加一栏花费；`--json` 与 MCP 的 `list_jobs` 带全部用量字段。

## 8. 被挡住先不派

- 读需求详情（`get_issue`）时多要 `relations`，`Issue` 多一项 `blocked_by`：挡住它的需求编号（形状不是 `KEY-123` 的项丢掉）；`list_issues` 读不到关系，这一项为「未读」。
- 一条需求「被挡住」：`blocked_by` 里至少有一条需求，其状态类型不是 `completed` 或 `canceled`。挡住它的需求已不存在的，不算挡住。每轮检查里读挡住它的需求失败时，这条需求沿用上次的结论，下一轮再读，不连累别的需求（全分支审查后改：原先整轮检查失败，一条读不到的挡住需求会让所有项目都停下）；开始运行前的那次读失败，只让这一条任务按启动失败暂缓。
- 什么时候查：开始运行前重读需求时查（重读本来就调 `get_issue`，关系一起读到）：被挡住就回到等待，原因写明。之后每轮检查只重查查过的候选需求（在等待或排队、上次运行已结束且 Linear 有新改动），再交给 `trigger::evaluate`；新出现的候选需求不在检查时多读一次，等开始前那次。这样一轮检查不会为每条新需求多调一次 Linear（实现时调整，原先的写法让每条新需求多读一次）。
- 次数控制：引擎在内存里记每条需求最近一次的结论（需求的 `updatedAt` 与查的时间），需求没改动且不到 2 分钟就沿用；同一轮里多条需求被同一条挡住时，那条只读一次。
- 等待原因：新代码 `blocked_by`（带挡住它的需求编号），任务停在「等待」，原因文字 `blocked by DEMO-3, DEMO-4, which are not done yet`。检查顺序排在绑定、目录、执行者之后：配置问题先显示。
- 只挡平常的运行（W9）：方案运行不改代码，先出方案正是它的用处，所以不查关系。
- 需求列表（面板）对 Todo 状态的候选需求也显示这条原因，只用上面记下的结论，不另外读 Linear。

## 9. 按标签分到不同目录

### 9.1 绑定

`Binding` 多一项 `labels`（分流标签，默认空，空时不写入文件）。含义：这个绑定接所属项目里带其中任一标签的需求；没有分流标签的绑定是这个项目的默认绑定。

校验（`Config::validate`）：

- 标签与触发标签同样要是单行、非空、不含引号与不可见字符；同一绑定里不重复。
- 同一项目最多一个默认绑定；同一项目的各绑定之间，分流标签不能重复（比较时去掉首尾空白、不分 ASCII 大小写）。
- 分流标签不能是触发标签（`ai-ready`）或方案标签（`ai-plan`）：所有候选需求都带着它们，分不出来。
- 绑定的 `id` 照旧不能重复；原来「同一项目只能一个绑定」的规则改成上面三条。

### 9.2 选绑定

`Config::route(issue)`：

1. 需求没有项目，或项目没有绑定：等待，`project_not_bound`（原样）。
2. 分流标签与需求标签有交集的绑定恰好一个：用它。
3. 不止一个：等待，新原因 `labels_match_several_bindings`（带对上的标签）。分流标签在同一项目内不重复，所以这只会在需求同时带两组标签时出现。
4. 一个也没有：用默认绑定；没有默认绑定就等待，新原因 `no_binding_for_labels`。

### 9.3 加、改、删

- 引擎的 `add_binding`：替换 `id` 相同的绑定，以及同一项目里与它冲突的（两个都是默认绑定，或分流标签有重叠）。这样命令行 `bind add` 不带标签时，与原来一样替换这个项目原来的绑定。
- 命令行：`issuerelay bind add … --label <标签>`（可重复）；不带标签时 `id` 照旧是项目 id，带标签时 `id` 是 `<项目 id>#<第一个标签>`，也可以用 `--id` 指定。`bind list` 显示分流标签。
- 面板：绑定对话框加「分流标签」（选填，从 Linear 的标签列表里选，可以多选）；绑定列表显示它们；新建带标签的绑定时 `id` 同命令行的规则，重名时加序号。实现时定（设计者定）：标签列表来自新加的只读面板命令 `labels`（`Engine::labels`：清洗、按 `same_name` 去重、按字母排序，命令表、应用清单与能力文件同步登记）；触发标签与方案标签不列出；同一项目另一个绑定已用的标签标明、不能勾；绑定上有而 Linear 已没有的标签标明、可以去掉；序号加在后面：`<id>-2`、`<id>-3`……（绑定改了标签后 `id` 不变，新绑定照规则得出的 `id` 可能已被占用）。「两个绑定会不会接同一批需求」与新绑定的 `id` 由 `app/src/lib/routing.ts` 按 core 的规则算，用例表 `app/src/lib/routing-cases.json` 由面板测试与 `app/src-tauri` 的测试共读。全分支审查后加：绑定页的卡片把 Linear 已经没有的分流标签标出来（只在有绑定带分流标签时才读 Linear 的标签），因为改了名的标签不会再出现在需求上，这些需求就默默去了默认绑定。
- 环境检查的绑定一行写出分流标签。

## 10. 只出方案

### 10.1 触发

- 设置多一项方案标签 `trigger.plan_label`（默认 `ai-plan`，在高级设置里改，校验同触发标签，且不能与触发标签相同）。
- 候选需求：指派给自己、状态 Todo、带触发标签（平常运行）或方案标签（只出方案）；两个都带时按平常运行。
- 每轮检查对两个标签各查一次候选需求，按需求合并；面板的需求列表同样合并两个标签的需求，取最近更新的 50 条。
- 任务记下模式（新列 `mode`）：入队时按当时的标签定，开始运行前重读需求时再定一次并写进记录（例如排队期间又加了 `ai-ready`）。

### 10.2 执行者参数（设计者定，按第 4 节）

- 精简（W8）：方案运行带精简运行的参数，不看绑定的精简开关，因为不精简时用户自己的其他 MCP 服务、插件与钩子都在，它们能写文件或推分支。所以方案运行要求执行者的精简检查通过（与精简的平常运行相同的就绪条件），否则等待 `lean_not_ready`。绑定目录就是用户主目录时，项目设置就是用户设置，隔不开，方案运行等待，新原因 `plan_needs_lean`（`planning runs need a bound folder other than the home folder`）。
- 「按个人设置」的绑定例外：不精简，沿用用户自己的设置，Claude 的权限模式照旧是 `dontAsk`，只加下面收紧内置工具的参数。原先的写法把它改成「完全放开」，实现后的自动安全审查指出这越过了用户自己收紧的权限（例如读工作目录以外的文件，再经 Linear 评论带出去），改为现在这样：能用什么由用户自己的规则决定；Linear 的工具要在用户设置里允许过，否则 AI 留不了方案、运行记为失败。Codex 同理只加只读沙箱，不加放行设置。
- Claude：在 `--permission-mode <档位>` 之后加 `--tools Read,Grep,Glob`（一个值，后面紧跟别的选项，与 `--mcp-config` 同样的写法要求）。内置的写文件、执行命令、联网等工具都不在；精简时 MCP 工具只有 Linear。权限档位照绑定。
- Codex：不用绑定档位的参数（完全放开、自动审查），新运行改用 `-s read-only`，接续时用 `-c sandbox_mode=read-only`；再加 `-c mcp_servers.linear.default_tools_approval_mode=approve` 放行 Linear 的工具调用（第 4.3 节）。方案运行总是精简的，`linear` 就是 IssueRelay 自己用 `-c` 定义的那个服务。
- Linear 的工具（全分支审查后加）：只留读取（`get_`、`list_`、`search_`、`extract_` 开头的）、`save_comment`（方案或提问）与 `save_issue`（去掉方案标签），其余 29 个写的工具都关掉，固定列表 `executor::PLAN_DENIED_LINEAR_TOOLS`，按 2026-10-27 Linear 列出的 69 个工具定（测试夹具 `crates/core/tests/fixtures/linear-tool-names.txt`，测试核对列表正好是其中会写的那些）。原因：Linear 的 diff 工具（`merge_diff`、`submit_diff_review`、`update_diff` 等）作用在需求关联的拉取请求上，不关的话需求里的文字可以让方案运行去审批或合并代码。Claude 加 `--disallowedTools mcp__linear-server__…`（一个值，逗号分隔）：拒绝规则在 `bypassPermissions` 下照样生效，2026-10-27 实测这几个工具不在 `system/init` 的工具列表里了；「按个人设置」的方案运行也加，IssueRelay 的一键配置给用户的 Linear 服务起的也是这个名字，名字对不上的规则什么也不拦。Codex 加 `-c mcp_servers.linear.disabled_tools=[…]`，只在精简运行时加：用户自己的配置里没有叫 `linear` 的服务时，这个设置会让 Codex 读配置失败（`invalid transport`，实测）。Codex 0.159.2 认得这个键（类型不对时报错，实测）；它确实让工具从模型的列表里消失，要等 Codex 的用量额度恢复后实测（验收报告）。Linear 以后新加的工具不在列表里，写进「待定问题」。
- Windows：Codex 的只读沙箱只在 macOS 实测过；Windows 待实机。提示词的固定规则同样禁止改文件。
- Codex 的方案运行遇到「按个人设置」的绑定（不精简）时不接续会话（全分支审查后加）：`exec resume` 没有 `-s`，`-c sandbox_mode=read-only` 改的是顶层的键，用户配置里当前 profile 自己设了 `sandbox_mode` 时会盖过它；新运行用的 `-s` 不会被盖过。精简的方案运行带 `--ignore-user-config`，没有这个问题。

### 10.3 提示词

`prompt::render_plan` 的固定规则（不可关闭）：

1. 只处理本需求；先用 Linear 工具读需求、父需求和全部评论。
2. 这是只出方案的运行：按需要阅读代码，但不改任何文件、不提交、不推送、不部署，也不改需求的状态。
3. 绝不把状态设为「完成」状态名，绝不推主分支，绝不发正式环境（三条固定规则原样保留）。
4. 写的每条评论都以 `🤖 IssueRelay` 开头。
5. 遵守工作目录及目标仓库里的 `AGENTS.md` / `CLAUDE.md`。
6. 把方案写成一条评论：做法、要改的文件、风险、待确认的问题、怎么验证。
7. 写完后去掉方案标签，状态保持不变。
8. 需求写得不清楚时，评论提问，去掉方案标签，结束。
9. 失败时评论原因，去掉方案标签，结束。
10. 评论用设置里的语言。
11. 最后一行输出一行 JSON：`{"result":"planned|needs_info|failed","summary":"…"}`。

团队说明（全局模板或绑定的覆盖）照常附在后面。

### 10.4 结果

- `outcome::Report` 多一种 `Planned`。
- 退出码为 0 且 AI 报告 `planned`：记为 `needs_info`，`report` 为 `planned`，面板显示「方案已出」；报告 `needs_info`：记为 `needs_info`，显示「待补充信息」。其余（非 0 退出、超时、卡住、报告 `failed` 或没有报告）照常是「失败」。
- 只出方案的运行照样在结束后读一次需求，记下 Linear 的 `updatedAt`（防重复规则要用）；不做「看起来没改动就再读」的等待（第 13 节那条只为平常运行的失败）。读失败时结果仍按它自己的报告定，不记成「无法核对」的失败，只在程序日志里记一行（全分支审查后改）。
- 用户看过方案后加上 `ai-ready`（Linear 有新改动），下一次检查按平常运行入队，并接续出方案的那个会话（第 11 节）。
- 通知：沿用「需要补充信息」的开关，方案已出时换成自己的文字（「方案已出，等你决定」）。

## 11. 重做时接续上一次的会话

### 11.1 什么时候接续

- 每轮检查为一条已经处理过的需求新建任务时（`reconcile` 里 `should_enqueue` 为真、上一条任务已结束），如果设置开着，就找这条需求最近一次有会话编号的运行（不一定是上一条：上一条可能排队时就被取消、从没开始），在新任务上记下 `resume_job_id`。
- 模式（W10）：平常接平常、方案接方案、方案之后的平常运行接方案的会话；平常运行之后的方案运行从头开始（方案运行收紧了工具，接着一个用过改文件工具的会话没有意义，也没实测过）。
- 开始运行时（`start_job`）再核对一次，全部满足才接续，否则清掉 `resume_job_id` 并在运行日志开头写一行原因，从头开始：
  - 上一条任务的执行者与这次相同，目录（规范化之后）也相同；
  - 上一条任务结束在 7 天以内（本机时钟，只用于这个估计）；
  - 会话编号的形状合格（UUID：16 进制与连字符，36 个字符；Codex 的线程编号同样是 UUID）；
  - Codex 的「自动审查」档位不接续：`codex exec resume` 没有 `--approve-for-me`（第 4.3 节），没有实测过等价的配置。Codex 的只出方案运行可以接续，用 `-c sandbox_mode=read-only` 代替 `-s read-only`（第 4.3 节）。
- 手动「重新运行」总是从头开始：`requeue` 清掉 `resume_job_id`。全分支审查后补了两处：开始运行前重读需求的那段时间里，用户先停止再重新运行了这条任务（行又回到排队，开始照样会成功），`start_job` 在登记运行之前再读一次这行的 `resume_job_id`，与开始时读到的不同就不开始这一次，交给下一轮按行上现在的样子开始；退回旧版本后旧版本重新运行过的行，`details_started_at` 还是这个版本上一次记录的、却已对不上 `started_at`，这时 `mode` 与 `resume_job_id` 都按没有读（与花费、会话同一条规则，`store::row_to_job`）。

### 11.2 参数

- Claude：在 `--permission-mode <档位>` 之后加 `--resume <会话编号>`。
- Codex：`exec resume`，选项与平常相同（`--json`、档位参数、`--skip-git-repo-check`、精简运行的参数、`-m`、推理力度），最后是会话编号和 `-`。

会话编号来自执行者自己的输出，不是 Linear 的文本，并且只接受 UUID 形状：与模型名一样，属于「经过校验的可变参数」，不是新的路径例外（`executor/mod.rs` 的模块说明一并更新）。

### 11.3 提示词

照常渲染全部固定规则与团队说明，只在第一行之后多一段：这是接着你之前对本需求的会话，它又回到了你这里，先读你上次运行之后新增的评论，再接着做。只出方案之后的平常运行，这一段改为：先读你写的方案和之后的评论，再按方案动手。

### 11.4 接续不成时

接续的那次运行如果以非 0 退出、并且没有出现会话开始的事件（Claude 的 `system/init`，Codex 的 `thread.started`），就算接续不成（会话已被执行者清掉等，第 4.2、4.3 节）：同一个任务立即从头再跑一次，运行日志开头写明「上一次的会话接续不成，从头开始」并附上原来的退出码，`resume_job_id` 清掉。这不是失败后的自动重试：前一次没有开始任何工作。

精简运行的核对原本把 `system/init` 之前出现的 `result` 事件当成精简没生效（`lean.rs` 的 `LeanCheck`），接续不成的 Claude 运行正是这样（第 4.2 节），会被记成「精简运行没有生效」而不是接续不成。所以核对改为：`init` 之前的 `result` 事件不算违规，它表示会话没开始就结束了（W11）；之后再有别的事件，照旧按「出现在 `init` 之前」拦下。一次全新的运行如果也这样结束（例如登录失效），记录的原因从「精简运行没有生效」变成「执行者以退出码 1 结束」，更贴切。

### 11.5 设置与显示

- 设置：「重做时接续上一次的会话」（`resume_sessions`，默认开）。
- 运行详情：接续的运行显示「接续 #N 的会话」，#N 可以点开那次运行。

## 12. 面板、命令行与文档

- 类型（`app/src/lib/types.ts`）与模拟后端：`Job` 加 `mode`、`report`、`session_id`、`resume_job_id`、`usage`；`Binding` 加 `labels`；`Settings` 加 `resume_sessions`、`trigger.plan_label`；等待原因加 `blocked_by`、`labels_match_several_bindings`、`no_binding_for_labels`。
- 页面：需求与运行记录给只出方案的运行加一个「方案」标记；状态徽章在 `needs_info` 加 `report = planned` 时显示「方案已出」；运行详情显示用量与接续来源；绑定对话框与列表显示分流标签；设置页加接续开关与方案标签；使用说明加：只出方案、按标签分到目录、被挡住先不派、Linear 上取消会停掉运行、接续会话、用量的含义。五种语言的翻译齐全。
- 截图比对：受影响的页面截图按新界面重新生成（`pnpm --dir app test:screens`，只在本机）。
- 命令行：`bind add --label`、`--id`；`bind list` 显示分流标签；`jobs` 的表格加模式与花费（花费的写法同面板：`$0.09`，不到一分写 `<$0.01`，没有花费的留空）。
- 运行详情：第 6 节的四种原因各有翻译好的标题，引擎的原句照旧在下面一行。
- 文档：README（触发条件、安全须知里只出方案的说明、退回旧版本的注意）、安装说明（退回旧版本前删掉分流绑定）、总设计（第 5、6、9 节对应处指向本文）、使用说明。

## 13. 测试

- 单元：`trigger`（模式判定、被挡住、分流选择）、`config`（分流标签校验、`add_binding` 的替换、未知键样例）、`store`（新列在旧库上补齐、旧版本形状的库照常读、`requeue` 清空）、`executor`（只出方案与接续的参数，各档位、精简与否）、`outcome`（用量与会话编号的解析，含第 4 节实测的事件形状）、`prompt`（两种固定规则都在，接续段）、`linear::parse`（`statusType`、`relations`）、`mcp`（新字段在连接时的检查）。
- 集成（假 Linear、假执行者）：运行中途需求被取消、改派、删除后停止并写对原因；用户停止仍是 `stopped by the user`；被挡住的需求等待、放开后入队；两条标签分到两个目录并行；只出方案的运行记成方案已出；重做时新任务带上 `resume_job_id`、参数里有 `--resume`；接续不成时同一任务从头再跑。
- 面板：各页面的新显示与五种语言的键齐全；绑定对话框的分流标签。
- 第 4 节的实测样本存进 `docs/gates/samples/2026-10-27/`（去掉真实需求内容，编号改成 `DEMO-N`）。

## 14. 实机验收（沙箱）

在沙箱 Linear 项目与 `~/issuerelay-sandbox` 上，用本分支构建的应用（换装要所有者在场回答钥匙串询问）逐项跑：

1. 运行中把需求改成 Canceled，运行在一次检查内停下，记录写明原因，进程树没有残留。
2. 运行详情显示花费与 token，与运行日志末尾的数字一致。
3. 被挡住的需求等待并写明是哪条；把挡住它的需求设为完成后入队。
4. 一个项目两个分流绑定（两个目录），两条需求按标签分开并行跑。
5. 只出方案（Claude、Codex 各一次）：Linear 上有方案评论，目录没有改动，方案标签被去掉。
6. 出完方案加 `ai-ready`，运行接续方案的会话（Claude、Codex 各一次）；一次退回重做同样接续；平常运行之后改加 `ai-plan`，方案运行从头开始。
7. 退回 v0.2.4：先删掉分流绑定，旧版本照常打开运行记录。

结果写进 `docs/gates/`；没做的写「待实机」。

## 15. 留给实现者

实现前请一位没参与设计的审读者以实现者的角度读过本文（2026-10-27），它提出的四个问题按 W8–W11 定下，另两处（最近一次有会话的运行、`details_started_at`）写进了第 5、11.1 节。

- 面板里花费的格式（小数位、千分位）、「方案」标记的样式，照现有设计变量。
- 被挡住结论的缓存放在引擎内存里，重启后重新查。
- 只出方案的 Claude 运行是否另外关掉 `Task`（子代理）：`--tools` 只给 `Read,Grep,Glob` 时子代理工具本来就不在，不另加参数。

## 16. 待定问题

1. 不精简的 Codex 方案运行，用户的 Linear 服务不叫 `linear` 时要自己放行 Linear 工具调用（第 10.2 节），是否值得在环境检查里另外提示。先在使用说明里写明。
2. 接续会话的 7 天上限是否要做成设置。
3. Linear 以后新加的写工具不在方案运行关掉的列表里（第 10.2 节）。可选做法：连接 Linear 时比对工具列表，出现不认识的工具就在程序日志里提醒，或改成只留读取与两个写工具的白名单（Codex 有 `enabled_tools`；Claude 没有只列 MCP 工具的白名单参数，要换成 `dontAsk` 加 `--allowedTools`，会改变档位的含义）。
