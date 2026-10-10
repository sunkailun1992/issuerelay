# M0：实测验证（2026-10-11）— 部分通过

## 结论

- macOS 上，Claude Code 与 Codex 都能按设计 6.1 的「完全放开」档参数无人值守完成一条测试需求：读 Linear、改测试仓库、推测试分支、改状态、写评论，全程没有卡在确认上。
- Rust（rmcp 3.5.1）经 OAuth 连上了 Linear MCP，8 个必需工具都在，返回形态已记录。
- 发现 6 处与设计或计划不一致（F1–F6），另有 2 处设计写法更新（G1–G2），都已改设计与计划。
- 未验证：Windows（暂无机器，待有机器后补做）；「自动审查」「按个人设置」两档权限；失败、超时等结果。后两类已排进计划 Task 23。

## 环境

- 实机日期：2026-10-10（执行者）、2026-10-11（Linear 探针）
- macOS 27.0.1；Claude Code 2.1.285；codex-cli 0.159.2；rustc 1.99.0；git 2.54.0；探针 `spikes/linear-mcp-probe`（rmcp 3.5.1）
- Windows：未验证（暂无 Windows 机器，见「未闭环」）
- 沙箱：一个专用的 Linear 测试项目，两条测试需求（下文记作 DEMO-1、DEMO-2，都指派给当前用户、带 `ai-ready`、状态 Todo）；本机一个裸仓库加一个工作副本，分支 `main` 与 `test`，工作副本的 `AGENTS.md` 规定「从 `test` 切 `feature/<编号>`，合回 `test` 并推送，永不推 `main`」。
- 两次执行者运行都在清空的环境里启动，只保留 `HOME`、`USER`、`LOGNAME`、`PATH`、`LANG`、`TERM`、`SHELL`；提示词经标准输入传入，内容与计划 Task 4 的模板一致。`PATH` 沿用了终端里的值，所以没有覆盖图形程序启动时的最小 `PATH`（见 F2）。

## Linear MCP

| 检查项 | 结果 |
|---|---|
| OAuth 回环授权（127.0.0.1 随机端口） | 通过。第一次失败：Linear 的授权服务器要求回调带 RFC 9207 的 `iss` 参数，探针原先只把 `code` 与 `state` 交给 rmcp，被拒绝（`missing required issuer`）；改为把完整回调地址交给 `OAuthState::handle_callback_url` 后通过（F5） |
| 授权在哪里完成 | 回调地址是本机 `127.0.0.1`，授权必须在运行程序的这台机器上完成。第 2、3 次用户在自己的浏览器里点了授权，但回调没有到达本机的监听端口（原因未确认）；第 4 次改在本机的应用内浏览器里完成 |
| 授予的 scope | `read`、`write` |
| 8 个必需工具都在 | 是（原始输出共 68 个工具，样本只留 8 个）。`list_issues` 接受 `assignee`、`label`、`state`、`limit`、`cursor`、`fields`，另有 `project`、`orderBy`、`updatedAt` |
| 翻页 | 字段是 `cursor`，配合 `hasNextPage`。第二页仍报 `hasNextPage` 为真，因为工作区里还有沙箱以外带 `ai-ready` 的需求（另查过第三页确认，未存样本）。`list_issues` 的末页没有观察到；`list_projects`、`list_issue_labels`、`list_teams` 的样本里能看到末页形态（`hasNextPage` 为假、没有 `cursor`）。计划让翻页在空页或游标不前进时也停止 |
| 需求字段形态 | `id` 是编号（如 `DEMO-1`）；有 `uuid`；`status` 是状态名字符串；`labels` 是名称数组；`priority` 是 `{value, name}`；`project` 是项目名；`projectId` 是项目的 `uuid`，与 `list_projects` 返回的项目 `id`（`P-<团队>-N` 编号）**不是**同一种 ID（F1） |
| 查不到需求时 | 工具错误（`isError` 为真），文字为 `{"error":"invalid_request","message":"Could not find referenced Issue.","status":400,...}`，里面没有 `not found`（F6）。样本文件开头的 `ERROR: ` 是探针按 `isError` 加的标记，不是服务端文本 |
| 当前用户 | `get_user` 传 `{"query":"me"}`；ID 字段是 `id`，与需求的 `assigneeId` 一致 |
| 状态列表 | `list_issue_statuses` 传 `team`，返回顶层数组 `[{id, type, name}]`；沙箱团队有 Backlog、Todo、In Progress、In Testing、Done、Canceled、Duplicate |
| 未观察到 | 子需求的 `parentId` 形态（样本里都是 `null`）；令牌续期（每次登录都很短）。两项都排进 Task 23 |

## 执行者

| 组合 | 退出码 | 用时 | Linear 状态 | 推送 | 卡在确认 |
|---|---|---|---|---|---|
| macOS + Claude + 完全放开 | 0 | 215 秒（墙钟；`result` 事件的 `duration_ms` 为 209 秒） | Todo → In Progress → In Testing；一条以「🤖 IssueRelay」开头的评论；`ai-ready` 保留 | `test` 新增一个提交，只加了 `hello.txt`；`main` 未动；没有留下远端 feature 分支 | 否 |
| macOS + Codex + 完全放开 | 0 | 138 秒（墙钟） | 同上 | `test` 新增一个提交，只改了 `README.md`；`main` 未动；临时 feature 分支已删 | 否 |
| Windows + Claude / Codex | — | — | — | — | 未验证 |

- Claude：`claude -p --output-format stream-json --verbose --permission-prompts none --permission-mode bypassPermissions`。标准输入传提示词有效，计划里的两种备用写法都不需要。最终消息在 `type` 为 `result` 的事件的 `result` 字段，末行是结果 JSON。
- Codex：`codex exec --json --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check -`，工作目录不是 git 仓库。事件有 `thread.started`、`turn.started`、`item.started` / `item.completed`、`turn.completed`。最终消息是最后一个 `item.type` 为 `agent_message` 的 `item.completed` 的 `item.text`，末行是结果 JSON，与计划假设一致。
- Codex 的事件计数：`item.completed` 46 个（`agent_message` 4、`mcp_tool_call` 8、`command_execution` 29、`reasoning` 4、`file_change` 1），`item.started` 38 个。
- 只测了「完全放开」档；另外两档的参数只据帮助信息。
- Linear 上的结果经 Linear 连接器读回核对；远端分支用 `git log` 核对。

### 环境检查命令的输出

| 命令 | 退出码 | 输出位置与形态 |
|---|---|---|
| `claude auth status` | 0 | 标准输出，多行 JSON，含 `"loggedIn": true` 和账号字段（样本只保留 `loggedIn`） |
| `codex login status` | 0 | **标准错误**输出一行 `Logged in using ChatGPT`，标准输出为空 |
| `claude mcp list` | 0 | 首行 `Checking MCP server health…`，之后每个服务器一行：`<名称>: <地址或命令> [(HTTP)] - <状态>`；状态有 `✔ Connected`、`! Needs authentication`、`✘ Failed to connect — …`、`- Not configured` |
| `codex mcp list` | 0 | 两张表：本地命令型（Name / Command / Args / Env / Cwd / Status / Auth）和 HTTP 型（Name / Url / Bearer Token Env Var / Status / Auth）；Linear 一行为 `linear  https://mcp.linear.app/mcp  -  enabled  OAuth` |

## 与设计不一致之处及处理

| # | 发现 | 处理 |
|---|---|---|
| F1 | 需求的 `projectId` 是项目的 `uuid`，项目的 `id` 却是 `P-<团队>-N` 编号；照原计划用项目 `id` 做绑定，绑定会永远匹配不上需求 | 设计第 7 节与计划 Global Constraints 写明口径；计划 Task 16 的 `parse_projects` 优先取 `uuid`，`m0_samples_parse` 新增「需求的 projectId 等于项目 uuid」断言，假 Linear 服务端的数据改成同样形态 |
| F2 | （推断，待 M2 实机验证）从访达或开机自启启动的 macOS 图形程序只有最小 `PATH`，找不到装在 Homebrew 或 npm 全局目录下的 `claude`、`codex`（`codex` 还要靠 `PATH` 找到 `node`）。M0 的运行都从终端启动，没有覆盖这一点 | 设计第 8 节加一条：托盘程序启动时经登录 shell 读取 `PATH`，用它查找执行者并传给子进程；读取失败时在环境检查里提示。M2 实现并在实机验收里覆盖 |
| F3 | 无人值守运行会加载用户级的指令文件、插件、hooks 和全部 MCP 服务器：两次运行都按用户级指令去调用了与本需求无关的个人知识库 MCP；Claude 输出了 132 条 `system` 事件，其中不少来自连不上的其他 MCP 服务器 | 不阻塞。设计第 14 节写明，README 的安全须知补一句；设计第 18 节新增待定问题：M2 评估按绑定提供「只加载 Linear MCP」的精简运行方式 |
| F4 | `codex login status` 把结果写到标准错误 | 计划 Task 18 的 `parse_codex_login` 已同时读取标准输出和标准错误；测试补上「只有标准错误有内容」的用例 |
| F5 | Linear 的授权服务器要求回调带 RFC 9207 的 `iss`；只把 `code`、`state` 交给 rmcp 会被拒绝 | 探针已改为 `handle_callback_url`；计划 Task 17 同样返回完整回调地址并调用 `handle_callback_url`，先读出 OAuth 的 `error` 参数再交给 rmcp（rmcp 不读它），任何 `/callback` 请求都结束等待；设计第 7 节写明 |
| F6 | 查不到需求时的错误文字是 `Could not find referenced Issue.` | 计划 Task 16 的 `is_not_found` 同时认 `could not find` 与 `not found`，假服务端改用真实文字 |
| G1 | 设计 6.1 原表把提示词和目录写进参数（`claude -p <提示词>`，`codex exec <提示词> -C <目录> -o <结果文件>`）；计划早已改为标准输入加子进程工作目录，M0 按计划写法跑通 | 设计 6.1 改成实测写法，另外两档权限标为待实机 |
| G2 | `claude mcp add` 默认只对当前目录生效，而 IssueRelay 会在多个绑定目录启动 Claude | M0 用 `--scope user` 注册；设计第 8 节与计划 Task 4 同步改为 `--scope user` |

## 用户决定带来的设计更新

- 仓库已于 2026-10-10 按用户要求公开，自动更新在 M3 即可启用（设计第 12 节、第 18 节第 3 条）。
- 应用图标与托盘图标在 M2 出方案由用户选定（设计 9.6）。
- 仓库要能接受外部贡献、开发者共同维护（设计 16.1；计划新增 Task 6b）。
- 暂无 Windows 机器，Windows 先只靠 CI（设计第 18 节第 4 条）。

## 样本来源与处理

样本都来自上面两次真实运行和一次探针抓取，提交前按 `docs/ai-coding/TESTING_SPEC.md` 脱敏：姓名改为 `Jane Doe`，邮箱改为 `jane@example.com`，工作区改为 `demo`，团队改为 `Demo` / `DEMO`，需求改为 `DEMO-1`（hello.txt）、`DEMO-2`（README），项目改为 `Demo Project` / `P-DEMO-1`，路径改为 `~/…`；会话 ID 全零，其余 UUID 与消息、请求、工具调用 ID 按出现顺序编号，Linear 样本和执行者样本共用一套编号，所以需求、项目、团队之间的引用仍然对得上。除此之外做过的裁剪：

| 样本 | 处理 |
|---|---|
| `executors/claude-stream.jsonl` | 只保留 4 个事件：`system/init`、一次 Linear `get_issue` 工具调用、最后一条助手文本、`result`。`init` 只保留类型、工作目录、会话 ID、工具（5 个内置工具与用到的 5 个 Linear 工具）、MCP 服务器（只留 Linear）、模型、权限模式、密钥来源、版本、输出风格；`result` 只保留主要字段。最后一条文本和 `result` 里删掉了一段列出操作者其他 MCP 服务器授权状态的话，其余原样 |
| `executors/codex-exec.jsonl` | 只保留 7 个事件：`thread.started`、`turn.started`、一次 Linear `get_issue` 调用的开始与完成、第一条完成的命令、最后一条 `agent_message`、`turn.completed`，内容原样 |
| `executors/claude-mcp-list.txt` | 首行、空行，以及每种状态各一行真实输出（需要授权、HTTP 已连接、本地命令已连接、连接失败、未配置），加 Linear 一行；其他服务器带有本机路径和操作者的工具清单，已删除 |
| `executors/codex-mcp-list.txt` | 本地命令表的表头与一行（家目录改为 `~`，补空格保持对齐），HTTP 表的表头与 Linear 一行；行尾空格已去掉 |
| `executors/claude-auth-status.json` | 只保留 `{"loggedIn": true}` |
| `executors/codex-login-status.txt` | 标准错误里的原文 |
| `linear/list_projects.json` | 只保留沙箱项目；工作区里另外两个项目整条删除。沙箱项目的摘要、描述、链接换成中性文字 |
| `linear/get_user_me.json` | 姓名、显示名、邮箱、头像、创建与更新时间换成占位值；个人状态保留原格式，时间换成占位值 |
| `linear/list_teams.json` | 团队的创建与更新时间换成占位值（原值与账号创建时间相同，可用于识别） |
| `linear/get_issue_missing.json` | 请求 ID 改为全零；开头的 `ERROR: ` 是探针加的标记 |
| `linear/tools.json` | 只保留 8 个必需工具的定义，内容原样 |
| 其余 `linear/*.json` | 只做上面的统一替换 |

## 未闭环

- Windows：M0 的 Windows 实测跳过（暂无机器）。Windows 先只靠 CI（`windows-latest` 上的编译与测试）覆盖；M2、M3 的 Windows 实机验收待有机器后补做，报告里如实写「待实机」。Windows 凭据管理器单条上限 2560 字节（UTF-16），OAuth 凭据 JSON 在 Windows 上的长度没有量过，也留到 Windows 实机。
- 「自动审查」「按个人设置」两档权限、失败、超时、需要补充信息、子需求 `parentId`、翻页末页形态、令牌续期：都排进计划 Task 23（Step 4 与 Step 5b）。
- F2 的最小 `PATH` 问题是推断，M2 实机验收覆盖。
