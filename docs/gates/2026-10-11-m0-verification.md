# M0 实测验证（2026-10-11）

本报告记录 M0 的实机结果：Claude Code、Codex 能否按设计 6.1 的参数无人值守完成一条测试需求；Rust（rmcp）能否经 OAuth 连上 Linear MCP 并读到需求。样本已脱敏，放在 `samples/m0/`。

## 环境

- macOS 27.0.1；Claude Code 2.1.285；codex-cli 0.159.2；rustc 1.99.0；git 2.54.0
- Windows：未验证（暂无 Windows 机器，见「未闭环」）
- 沙箱：一个专用的 Linear 测试项目，两条测试需求（下文记作 DEMO-1、DEMO-2，都指派给当前用户、带 `ai-ready`、状态 Todo）；本机一个裸仓库加一个工作副本，分支 `main` 与 `test`，工作副本的 `AGENTS.md` 规定「从 `test` 切 `feature/<编号>`，合回 `test` 并推送，永不推 `main`」。
- 两次运行都在清空的环境里启动（只保留 `HOME`、`USER`、`LOGNAME`、`PATH`、`LANG`、`TERM`、`SHELL`），模拟托盘程序启动子进程的条件；提示词经标准输入传入，内容与计划 Task 4 的模板一致。

## Linear MCP

用 `spikes/linear-mcp-probe`（rmcp 3.5.1）实测，样本在 `samples/m0/linear/`。

| 检查项 | 结果 |
|---|---|
| OAuth 回环授权（127.0.0.1 随机端口） | 通过。第一次失败：Linear 的授权服务器要求回调带 RFC 9207 的 `iss` 参数，探针原先只把 `code` 与 `state` 交给 rmcp，被拒绝（`missing required issuer`）；改为把完整回调地址交给 `OAuthState::handle_callback_url` 后通过（F5） |
| 授权在哪台机器上完成 | 必须在运行程序的这台机器的浏览器里完成：回调地址是本机 `127.0.0.1`，在其他设备上点授权，回调到不了本机。实测第 2、3 次就是这样，最后在本机浏览器里完成 |
| 授予的 scope | `read`、`write` |
| 8 个必需工具都在 | 是（共提供 68 个工具）。`list_issues` 接受 `assignee`、`label`、`state`、`limit`、`cursor`、`fields`，另有 `project`、`orderBy`、`updatedAt` |
| 翻页游标字段 | `cursor`，配合 `hasNextPage`；第二页仍报 `hasNextPage` 为真，引擎要一直翻到它为假 |
| 需求字段形态 | `id` 是编号（如 `DEMO-1`）；有 `uuid`；`status` 是状态名字符串；`labels` 是名称数组；`priority` 是 `{value, name}`；`project` 是项目名；`projectId` 是项目的 `uuid`，与 `list_projects` 返回的项目 `id`（`P-<团队>-N` 编号）**不是**同一种 ID（F1） |
| 查不到需求时的返回 | 工具错误：`ERROR: {"error":"invalid_request","message":"Could not find referenced Issue.","status":400,...}`，文字里没有 `not found`（F6） |
| 当前用户 | `get_user` 传 `{"query":"me"}`；ID 字段是 `id`，与需求的 `assigneeId` 一致 |
| 状态列表 | `list_issue_statuses` 传 `team`，返回顶层数组 `[{id, type, name}]`；沙箱团队有 Backlog、Todo、In Progress、In Testing、Done、Canceled、Duplicate |
| 未观察到 | 子需求的 `parentId` 形态（样本里都是 `null`），留到 M1 验收 |

## 执行者

| 组合 | 命令 | 退出码 | 用时 | Linear 状态变化 | 推送结果 | 卡在确认 | 备注 |
|---|---|---|---|---|---|---|---|
| macOS + Claude + 完全放开 | `claude -p --output-format stream-json --verbose --permission-prompts none --permission-mode bypassPermissions`，提示词走标准输入 | 0 | 215 秒 | Todo → In Progress → In Testing；一条以「🤖 IssueRelay」开头的评论；`ai-ready` 保留 | `test` 新增一个提交，只加了 `hello.txt`；`main` 未动；没有留下远端 feature 分支 | 否 | 标准输入传提示词有效，计划里的两种备用写法都不需要。最终消息在 `type` 为 `result` 的事件的 `result` 字段，末行是结果 JSON |
| macOS + Codex + 完全放开 | `codex exec --json --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check -`，提示词走标准输入，工作目录不是 git 仓库 | 0 | 138 秒 | Todo → In Progress → In Testing；一条以「🤖 IssueRelay」开头的评论；`ai-ready` 保留 | `test` 新增一个提交，只改了 `README.md`；`main` 未动；临时 feature 分支已删 | 否 | 事件：`thread.started`、`turn.started`、`item.started` / `item.completed`、`turn.completed`。最终消息是最后一个 `item.type` 为 `agent_message` 的 `item.completed` 的 `item.text`，末行是结果 JSON，与计划假设一致 |
| Windows + Claude | — | — | — | — | — | — | 未验证 |
| Windows + Codex | — | — | — | — | — | — | 未验证 |

Codex 的事件计数：`item.completed` 46（`agent_message` 4、`mcp_tool_call` 8、`command_execution` 29、`reasoning` 4、`file_change` 1），`item.started` 38。

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
| F2 | 从访达或开机自启启动的 macOS 图形程序只有最小 PATH，找不到装在 Homebrew 或 npm 全局目录下的 `claude`、`codex`（`codex` 还要靠 PATH 找到 `node`）。M1 的开发宿主在终端里运行，不受影响 | 设计第 8 节加一条：托盘程序启动时经登录 shell 读取 PATH，用它查找执行者并传给子进程；读取失败时在环境检查里提示。M2 实现并在实机验收里覆盖 |
| F3 | 无人值守运行会加载用户级的指令文件、插件、hooks 和全部 MCP 服务器：两次运行都按用户级指令去调用了与本需求无关的个人知识库 MCP；Claude 输出了 132 条 `system` 事件，其中不少来自连不上的其他 MCP 服务器 | 不阻塞。设计第 18 节新增待定问题：M2 评估按绑定提供「只加载 Linear MCP」的精简运行方式；README 的安全须知补一句「AI 会使用你本机 Claude Code / Codex 的全部配置」 |
| F4 | `codex login status` 把结果写到标准错误 | 计划 Task 18 的 `parse_codex_login` 已同时读取标准输出和标准错误，无需改动；测试补一条「只有标准错误有内容」的用例 |
| F5 | Linear 的授权服务器要求回调带 RFC 9207 的 `iss`；只把 `code`、`state` 交给 rmcp 会被拒绝 | 探针已改为 `handle_callback_url`；计划 Task 17 同样改为返回完整回调地址并调用 `handle_callback_url`，任何 `/callback` 请求都交给 rmcp 判断（OAuth 错误回调立即失败，不会一直等）；设计第 7 节写明 |
| F6 | 查不到需求时的错误文字是 `Could not find referenced Issue.` | 计划 Task 16 的 `is_not_found` 同时认 `could not find` 与 `not found`，假服务端改用真实文字 |

## 样本来源与处理

样本都来自上面两次真实运行和一次探针抓取，提交前按 `docs/ai-coding/TESTING_SPEC.md` 脱敏：姓名改为 `Jane Doe`，邮箱改为 `jane@example.com`，工作区改为 `demo`，团队改为 `Demo` / `DEMO`，需求改为 `DEMO-1`（hello.txt）、`DEMO-2`（README），项目改为 `Demo Project` / `P-DEMO-1`，路径改为 `~/…`；会话 ID 全零，其余 UUID 与消息、请求、工具调用 ID 按出现顺序编号，跨文件保持一致，所以需求、项目、团队之间的引用仍然对得上。除此之外做过的裁剪：

| 样本 | 处理 |
|---|---|
| `executors/claude-stream.jsonl` | 只保留 4 个事件：`system/init`、一次 Linear `get_issue` 工具调用、最后一条助手文本、`result`。`init` 只保留类型、工作目录、会话 ID、工具（5 个内置工具与用到的 5 个 Linear 工具）、MCP 服务器（只留 Linear）、模型、权限模式、密钥来源、版本、输出风格；`result` 只保留主要字段。最后一条文本和 `result` 里删掉了一段列出操作者其他 MCP 服务器授权状态的话，其余原样 |
| `executors/codex-exec.jsonl` | 只保留 7 个事件：`thread.started`、`turn.started`、一次 Linear `get_issue` 调用的开始与完成、第一条完成的命令、最后一条 `agent_message`、`turn.completed`，内容原样 |
| `executors/claude-mcp-list.txt` | 首行、空行，以及每种状态各一行真实输出（需要授权、HTTP 已连接、本地命令已连接、连接失败、未配置），加 Linear 一行；其他服务器带有本机路径和操作者的工具清单，已删除 |
| `executors/codex-mcp-list.txt` | 本地命令表的表头与一行（家目录改为 `~`，补空格保持对齐），HTTP 表的表头与 Linear 一行；行尾空格已去掉 |
| `executors/claude-auth-status.json` | 只保留 `{"loggedIn": true}` |
| `executors/codex-login-status.txt` | 标准错误里的原文 |
| `linear/list_projects.json` | 只保留沙箱项目；工作区里另外两个项目整条删除。沙箱项目的摘要、描述、链接换成中性文字 |
| `linear/get_user_me.json` | 姓名、显示名、邮箱、头像、个人状态、创建与更新时间换成占位值 |
| `linear/get_issue_missing.json` | 请求 ID 改为全零 |
| `linear/tools.json` | 只保留 8 个必需工具的定义，内容原样 |
| 其余 `linear/*.json` | 只做上面的统一替换 |

## 未闭环

- Windows：M0 的 Windows 实测跳过（暂无机器）。Windows 先只靠 CI（`windows-latest` 上的编译与测试）覆盖；M2、M3 的 Windows 实机验收待有机器后补做，报告里如实写「待实机」。
- 只测了「完全放开」档位；「自动」「只读」档位的实际行为留到 M1 验收（Task 23）。
- 失败、超时、需要补充信息三种结果没有在 M0 实测，留到 M1 验收。
