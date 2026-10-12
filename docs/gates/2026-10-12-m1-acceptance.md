# M1：引擎与命令行实机验收（2026-10-12）— 部分通过（macOS 通过，Windows 待实机）

## 结论

- 在 macOS 上，开发宿主 `issuerelay-devhost` 加命令行 `issuerelay` 走通了完整流程：OAuth 登录、绑定、环境检查、发现需求、派活、Claude / Codex 无人值守完成、在 Linear 核对结果。建需求到开始运行用了 22–43 秒，要求是 1 分钟内。
- 结果判定全部按设计：成功的运行记为待验收；需求写得含糊记为待补充信息；做不到记为失败；超时记为失败，原因写超时；运行中停止宿主记为已停止。AI 只改测试分支，从不推 `main`，从不设 Done。
- 最终评审修过的进程树清理，在真实 Claude Code 上得到验证：Bash 工具开出的 shell 有自己独立的进程组，超时和停止时都被结束，沙箱里没有残留进程，也没有 `.git/index.lock`。
- 新发现 6 处（H1–H6），其中 H1 影响 M2 设计：执行者需要宿主传给它代理环境变量。
- 未验证：Windows（暂无机器）；令牌续期没有直接观察到（待观察）；「标记完成」属于 M2 面板。

## 环境

- 日期：2026-10-12（实测时间以 UTC 记）
- macOS 27.0.1；Claude Code 2.1.285；codex-cli 0.159.2；rustc 1.99.0；git 2.54.0
- IssueRelay：`main` 的 `361f11b`（debug 构建的 `issuerelay`、`issuerelay-devhost`）；本地全量验证 441 个测试全部通过，fmt、clippy 干净，`check-secrets --strict` 无输出
- 沙箱：沿用 M0 的 Linear 测试项目（下文记作 Demo Project，团队记作 Demo）。本次新建 13 条测试需求，记作 DEMO-3 至 DEMO-15，都指派给当前用户。本机裸仓库加工作副本 `~/issuerelay-sandbox/demo`，`AGENTS.md` 规定从 `test` 切 `feature/<编号>`、合回 `test` 并推送、永不推 `main`
- 宿主在终端里用 `env -i` 启动，只保留 `HOME`、`USER`、`LOGNAME`、`PATH`、`LANG`、`TERM`，外加本机 HTTP 代理的 `HTTPS_PROXY`、`HTTP_PROXY`、`NO_PROXY`（原因见 H1）。没有在 Claude Code 会话里启动：会话的环境里有 `CLAUDECODE`、会话 ID 等变量，执行者会继承它们
- 默认设置：检查间隔 30 秒，并发上限 2，单次运行超时 60 分钟（5b.2 时临时改为 2 分钟），卡住判定 15 分钟

## 步骤与结果

| 步骤 | 做法 | 结果 |
|---|---|---|
| 1 全量验证 | `cargo fmt --all --check`、`clippy -D warnings`、`cargo test --workspace`、`check-secrets --strict` | 通过：cli 11 + 10 + 5，core 221，doctor 12，engine 104，fake_agent 4，ipc 26，linear_mcp 23，runner 19，devhost 6，共 441 |
| 2 登录 | `issuerelay-devhost login` | 第一次打开授权页后没有点授权，10 分钟后超时。第二次在应用内浏览器点授权，回调到达本机监听端口，打印 `Logged in to Linear as …`。凭据写进钥匙串，没有弹窗 |
| 2 项目列表 | `issuerelay-devhost projects` | 列出 3 个项目，沙箱项目的 id 与 Linear 连接器给的项目 `uuid` 一致（M0 的 F1） |
| 2 宿主 | `issuerelay-devhost run` | 连上 Linear MCP（协议版本 2025-11-25），连接时检查工具和参数通过 |
| 2 绑定 | `issuerelay bind add … --dir ~/issuerelay-sandbox/demo --executor claude` | 保存成功 |
| 2 环境检查 | `issuerelay doctor` | 第一次 Claude 未就绪：`Linear MCP in claude: Failed to connect — ECONNRESET`。原因是没有代理变量（H1），带上代理变量重启宿主后全部就绪：Linear 已连接，claude、codex ready，工作区标签与状态齐全，绑定 ok。`doctor` 约 43 秒，主要耗在 `claude mcp list` |
| 3 Claude 完整流程 | DEMO-3：新增 `hello2.txt` | 建需求后 22.5 秒开始运行，2 分 23 秒结束，状态待验收，摘要入库。Linear：AI 改为 In Testing，评论以「🤖 IssueRelay」开头，写明变更、分支流程和验证，并说明不会设 Done。远端 `test` 多了对应提交，文件内容逐字节正确，`main` 未推送；本地仓库干净，功能分支已删 |
| 3 日志 | `issuerelay logs <id> --lines 5` | 原样的 Claude 事件流，末行是结果事件 |
| 4 Codex | 绑定改为 codex；DEMO-4：新增 `hello3.txt` | 43 秒开始，2 分 46 秒结束，待验收；远端 `test` 有提交，Linear 为 In Testing |
| 4 待补充信息 | DEMO-5：标题和描述都只有「改一下首页」 | 与 DEMO-4 同轮发现，同一目录排在 DEMO-4 之后（创建较早的先跑）。紧接着开始，1 分 36 秒结束，状态 `needs_info`。AI 留言问了三个具体问题，改回 Todo，去掉 `ai-ready`，没改代码 |
| 5 暂停 | `issuerelay pause` → 暂停期间建 DEMO-6 → 等 3 分钟 | `status` 显示 `Monitoring: paused`，DEMO-6 没被接走。暂停那一刻在跑的那一轮检查要刷新就绪缓存，约 25 秒后收尾，之后没有再检查 |
| 5 恢复 | `issuerelay resume` | 约 2 秒后 DEMO-6 开始运行（恢复时立即检查一次） |
| 5b.1 失败 | DEMO-6：修改不存在的 `ghost/config.yml`，写明不存在就判失败 | 1 分 37 秒结束，状态 `failed`，原因 `run finished but the issue status is "Todo", not "In Testing"`，摘要取自 AI 的失败报告。Linear：AI 留言说明原因，改回 Todo，去掉 `ai-ready`。`test` 无新提交，没建 `ghost/` |
| 5 MCP 控制服务 | 经用户同意，在仓库目录临时注册：`claude mcp add issuerelay --scope local -- …/issuerelay mcp` | 健康检查显示 ✔ Connected。另起一个无人值守的 Claude 对话，只允许它用 `status` 和 `list_jobs`，它正确报出监控状态、Linear 已连接、4 条运行各自的状态和失败原因。之后已 `claude mcp remove` |
| 5b.2 超时 | 停宿主 → 改 `run_timeout_mins = 2` → 重启 → 绑定 claude；DEMO-7：前台执行 `sleep 3600` | 35 秒开始，2 分 01 秒后记为 `failed`，原因 `run exceeded the time limit`。运行时，Claude 的 Bash 工具开出的 shell 是自己进程组的组长（父进程是 claude），它的子进程是 `sleep 3600`，脱离了执行者的组。超时后两者都已结束，精确匹配 `sleep 3600` 为空，沙箱里没有进程，没有 `.git/index.lock`。Linear 上需求停在 In Progress（AI 开始时设的，引擎不写 Linear），不再是候选 |
| 运行中停止 | 超时改回 60 并重启；DEMO-9：前台执行 `ping -c 3600 127.0.0.1` | Bash 工具的 shell 同样是自己进程组的组长，子进程是 ping。对宿主按 Ctrl+C：打印「stopping: waiting for running jobs to end…」，1.2 秒后记为 `canceled`（原因 `stopped by the user`），约 10 秒后宿主退出。ping 和它的 shell 都已结束，无 `.git/index.lock`。有一个工作目录在沙箱里的短暂 `bash`（SIGTERM 时启动的会话结束钩子），几秒内自行退出 |
| 运行中停止（第一次） | DEMO-8：前台执行 `sleep 3600` | 没测到停止：Claude Code 的 Bash 工具拦截了单独的 `sleep 3600`（H4），需求又禁止后台执行，AI 照规则判失败、留言、改回 Todo、去掉 `ai-ready`；本地记为 `failed` |
| 5b.3 Claude 自动审查 | `--permission auto`；DEMO-10 | 2 分 35 秒，待验收，已推送，没有卡在确认 |
| 5b.3 Claude 按个人设置 | `--permission personal`（`dontAsk`）；DEMO-11 | 失败。Linear 工具（`get_issue`、`list_comments`、`save_comment`、`save_issue`）和 `git fetch` 都被拒，AI 读不到需求，也不能留言或改状态。没有提交，Linear 上仍是 Todo + `ai-ready`；本地 `failed`。之后需求没有变化，引擎也没有重复派它（H3） |
| 5b.3 Codex 自动审查 | `--permission auto`（`--approve-for-me`）；DEMO-12 | 4 分 44 秒，待验收，已推送 |
| 5b.3 Codex 按个人设置 | `--permission personal`；DEMO-13 | 2 分 42 秒，待验收，已推送。本机 `~/.codex/config.toml` 是 `approval_policy = "never"`、`sandbox_mode = "danger-full-access"`，所以实际等同完全放开 |
| 5b.4 子需求 | 父需求 DEMO-14（不带 `ai-ready`）和子需求 DEMO-15（带 `ai-ready`） | `list_issues` 返回的 `parentId` 是编号（`KEY-N`），不是 uuid，与 `parse_issue` 的 `parent_identifier` 一致。子需求照常完成并推送。描述里写的编号会被 Linear 改写成 `<issue id=… href=…>KEY-N</issue>` 提及 |
| 5b.5 末页形态 | 用 Linear 连接器以 `limit` 5、`orderBy: createdAt` 沿 `cursor` 翻沙箱项目 | 前两页是 `{issues, hasNextPage: true, cursor}`，游标是当页最后一条需求的 uuid。末页是 `{issues, hasNextPage: false}`，**没有** `cursor` 键，与解析逻辑一致。`createdAt` 排序是从新到旧 |
| 5b.6 令牌续期 | 宿主从登录后一直运行 | 待观察：日志里没有续期记录；登录（14:10Z）后超过 1 小时，Linear 调用一直成功（15:15Z 仍正常接走需求）。Linear MCP 的授权服务默认签发 1 小时的访问令牌，这是推断，不是观察 |
| M0 未实测的写法 | — | 按 uuid 调 `get_issue`（启动前重读、结束后核对）在 15 次运行里都正常；`list_issues` 带 `state: "Todo"`、`label`、`assignee: "me"`、`orderBy: "createdAt"`，只返回 Todo + `ai-ready` 的需求 |

## 新发现

| 编号 | 发现 | 处理 |
|---|---|---|
| H1 | 本机需要经代理访问 Linear。Claude Code（Bun 运行时）只认 `HTTPS_PROXY` 等环境变量，不读 macOS 的系统代理；宿主自己的 Linear 客户端（reqwest）会读系统代理。对比测得：终端的登录 shell（没有代理变量）✘、`env -i` ✘、`env -i` 加代理变量 ✔。Claude 桌面应用会自己注入这些变量，普通终端不会。执行者继承宿主的环境，所以宿主没带代理变量时，每次运行都连不上 Linear。环境检查提前发现了这一点，并带出了真实原因 | M2：托盘程序从系统代理设置（或登录 shell）取 `HTTPS_PROXY`、`HTTP_PROXY`、`NO_PROXY` 传给执行者；环境检查的提示可以点明代理。M1 的做法：启动 devhost 时带上这三个变量 |
| H2 | 标题是中文时，Linear 链接末尾带中文 slug。`safe_url` 只放行 ASCII，链接被丢弃，提示词里只剩编号。运行没受影响 | 后续：保留 `https://linear.app/<工作区>/issue/<KEY-N>`（去掉 slug），或对非 ASCII 字符做百分号编码 |
| H3 | Claude 的「按个人设置」（`dontAsk`）会拒绝用户设置里没有预先允许的一切操作，包括 Linear MCP 工具和 git 命令 | 设计 6.1 写明：只有预先允许了 Linear 工具和 git 命令才能用；M2 面板的说明照写 |
| H4 | Claude Code 的 Bash 工具会拦截单独的长时间 `sleep`，提示改用后台执行或 Monitor | 只影响验收时怎么模拟长时间工作，不是产品问题；模拟改用 `ping -c` |
| H5 | 无人值守运行会加载用户的全部 MCP 服务器和插件（M0 的 F3），并拉取插件市场的更新，Claude 启动最慢约 3 分钟 | 计划 M2 已列「精简运行方式」的评估 |
| H6 | 执行者结束时，会话结束钩子会以独立进程（`bash`）短暂运行，工作目录在沙箱里，几秒内自行退出。离开进程组的工具 shell 在超时和停止时都被结束了 | 符合最终评审对残留的说明：执行者退出后才启动、自己会退出的钩子不追。之后的验收照样在每次运行后查残留 |

另外：AI 的评论是繁体中文，因为本机系统首选语言是 `zh-Hant-TW`，而评论语言设的是「跟随界面」，这符合设计。想用简体，把 `comment_language` 设为 `zh-CN`。

## 与设计不一致之处

- 设计 6.1：「自动审查」两档都能用，Codex 的 `--approve-for-me` 明显更慢。Claude「按个人设置」依赖用户预先授权（H3）。Codex「按个人设置」完全取决于 `~/.codex/config.toml`。已更新设计 6.1。
- 计划 Task 23 写的绑定目录是 `~/issuerelay-sandbox`，实际仓库在 `~/issuerelay-sandbox/demo`（M0 的布局）；本次按实际目录绑定。
- 执行者需要代理环境变量（H1），设计第 8 节只提到图形程序要取登录 shell 的 `PATH`；已在计划 M2 表加一行。

## 未闭环

- Windows：全部待实机，包括凭据在 Windows 凭据管理器里的长度、命名管道权限、`CREATE_NO_WINDOW` / `CREATE_NEW_PROCESS_GROUP`、代理行为。
- 令牌续期：待观察，需要一次能直接确认的长时间运行。
- 「标记完成」：属于 M2 面板。
- 父进程先退出的孤儿，以及执行者正常退出后留下的后台进程，按设计找不到。本次 15 次运行后的检查都没发现这类残留。
- 沙箱里留下的 DEMO-7、DEMO-9（In Progress）、DEMO-11（Todo + `ai-ready`）都是测试需求，保持原样。
