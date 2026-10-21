# 精简运行遗留小项（PR #15）：沙箱实机核对与 v0.2.2 发布（2026-10-21）— macOS 通过

## 结论

- 计划末节「待实机的检查」里 PR #15 的几项，在 macOS 沙箱上都看到了：
  - `doctor` 的绑定一行写出推理力度：设了 `high` 的绑定是 `codex (lean), effort high`，没设的那一行不写。
  - 精简运行日志开头的说明：Claude、Codex 各一次，都是运行日志的第一行，`runner` 流，以 `lean run: ` 开头。
  - 只差精简检查时，需求记为等待（`waiting_binding`），原因是 `executor codex is not ready for lean runs`；`issuerelay jobs` 与 MCP 控制服务的 `list_jobs` 相同。
  - Codex 的 Auth 栏：配置里设了 bearer token 时（`Bearer token`）出现新的精简问题；照它的建议删掉 token、运行 `codex mcp login linear` 以后，问题消失。本机平常的配置是 `OAuth`，精简可用，精简的 Codex 运行走到待验收。
- 没有看：面板需求列表里的那一行。开发宿主没有面板，与 2026-10-20 一样用命令行代替（所有者 2026-10-21 定），面板的文字由组件测试覆盖。
- 系统通知（M2 起未闭环）：用从草稿装上的 0.2.2 托盘程序核对。四种通知（开始运行、待验收、需要补充信息、运行失败）都由 IssueRelay 发出、由系统显示：usernoted 的日志里每一则都有 `Presenting`。维护者当时不在电脑前，没有人亲眼看到。第一次核对时，系统设置里 IssueRelay 的「允许通知」是关着的：通知照样送到系统，却不显示，程序这边看不出来（O3）。
- v0.2.2：PR #16 合并（`ff3007d`），在 `main` 上试打包，推 `v0.2.2` 标签建出草稿，本机从草稿装上 0.2.2；09:52:42 UTC 由维护者明确授权公开，设为最新版本。
- 新的观察四条（O1–O4），都没有改代码。

## 环境

- 日期：2026-10-21，日志时间是 UTC（本机是 UTC+8）：沙箱核对 07:19–07:37；换装、系统通知与发布 09:04–09:53。
- 软件版本：macOS 27.0.1（26A434），Apple 芯片；Claude Code 2.1.285；codex-cli 0.159.2；rustc 1.99.0。
- IssueRelay：`main` 的 `19f66b7`（PR #15）的调试构建，开发宿主 `issuerelay-devhost` 加命令行 `issuerelay`，与托盘程序用同一个数据目录。
  - 之前退出已装的托盘程序（0.2.1，经退出事件正常结束，约 1 秒），备份 `config.toml` 与 `state.db`；之后恢复 `config.toml`（与备份逐字节相同），重新打开托盘程序，约 20 秒连上 Linear，没有重跑任何运行。
  - 开发宿主启动了两次。第一次 6 秒连上 Linear（OAuth），其间系统里有一个钥匙串授权进程（SecurityAgent）；维护者在电脑前，没有看到对话框。第二次是同一个构建，2 秒连上，没有 SecurityAgent。
- 执行者的环境：开发宿主用 `env -i` 启动，只留 `HOME`、`USER`、`LOGNAME`、`SHELL`、`TMPDIR`、系统默认的 `PATH`、`__CF_USER_TEXT_ENCODING` 与 `SSH_AUTH_SOCK`。第一段另加 `CODEX_HOME`，指向下面的临时文件夹（执行者继承宿主的 `CODEX_HOME`，`ExecEnv` 只改 PATH 与代理）。
- 沙箱：Demo Project 与 `~/issuerelay-sandbox/demo`（本地裸仓库充当远端）。测试需求是 DEMO-41、DEMO-42，建好后运行 `issuerelay poll`。
- Codex 的反面情形用一个临时的 `CODEX_HOME` 造，不改用户的 `~/.codex`（所有者 2026-10-21 定）：
  - 文件夹里只放一份 `config.toml`：`mcp_oauth_credentials_store = "file"`，加上 `linear` 服务（官方网址；`bearer_token_env_var` 指向一个没有设的变量名，不放任何真的 token）。
  - 在里面另做一次 `codex login`（ChatGPT，浏览器），登录存进这个文件夹的 `auth.json`；没有复制用户的登录。
  - 按 codex-cli 0.159.2 的源码（2026-10-21 读）：MCP 的 OAuth 登录默认存在钥匙串里，键由服务名与网址得出，不含 `CODEX_HOME`，所以各个 `CODEX_HOME` 共用用户已存的 Linear 登录；`mcp_oauth_credentials_store = "file"` 让读写都只用这个文件夹里的 `.credentials.json`。`codex mcp logout` 不管这项设置，总会先删钥匙串里的那一条，所以在临时的 `CODEX_HOME` 下不能运行它，否则会删掉用户真正的登录。
  - 用完删掉整个文件夹，没有运行任何 logout。之后用户 `~/.codex/auth.json` 的修改时间没变，`codex mcp list` 里 `linear` 仍是 `OAuth`。

## 步骤与结果

| 项 | 做法 | 结果 |
|---|---|---|
| a 推理力度 | 沙箱绑定 `issuerelay bind add … --executor codex --permission full --lean on --effort high`，运行 `issuerelay doctor` | `Binding Demo Project -> codex (lean), effort high: executor not ready`（这时 Codex 的精简检查不过，见下一行），修好以后是 `… effort high: ok`。另一个绑定没有设推理力度，那一行是 `claude (lean): ok`，不写 `effort`。 |
| d 反例 | 第一段，临时 `CODEX_HOME` 里设了 bearer token 变量 | • `codex mcp list` 的 `linear` 一行：`enabled`、`Bearer token`。<br>• `doctor`：`codex: ready`；``codex (lean runs): not ready (the Linear MCP server `linear` in codex signs in with a token from codex's configuration (`Bearer token`), which lean runs leave out; to use lean runs, remove the token from its entry (`bearer_token_env_var` or an `Authorization` header) and sign in with `codex mcp login linear`, or turn lean runs off for this binding)``。 |
| c 等待原因 | 建 DEMO-41（新增一个文件，推到 `test`），`issuerelay poll` | • 1 秒内记为等待：`issuerelay jobs` 是 `waiting_binding … — executor codex is not ready for lean runs`，`status` 是 `Waiting for binding: 1`。<br>• MCP 控制服务 `list_jobs` 的 `reason` 相同。<br>• 随后 `issuerelay pause`，免得修好以后它在临时配置下开跑。 |
| d 照建议做 | 删掉临时配置里的 `bearer_token_env_var`，再运行 `codex mcp login linear`（浏览器授权） | • 删掉以后 Auth 栏是 `Not logged in`，`doctor` 里 Codex 的平常检查也不过：`codex: not ready (Linear MCP in codex: enabled Not logged in)`。这时的等待原因按设计会回到 `executor_not_ready`（监看已暂停，没有再判定）。<br>• 登录以后 Auth 栏是 `OAuth`，登录写在临时文件夹的 `.credentials.json` 里；`doctor` 是 `codex (lean runs): ready`，绑定一行 `ok`。DEMO-41 仍在等待。 |
| d 正例、b Codex | 停掉开发宿主，不带 `CODEX_HOME` 重启（用户平常的配置），`issuerelay resume` | • `codex mcp list` 的 `linear` 一行是 `enabled`、`OAuth`，`doctor` 显示精简可用。<br>• DEMO-41 以精简的 Codex 运行（权限完全放开，推理力度 `high`），07:32:03 开始，2 分 28 秒走到待验收。<br>• 运行日志第一行是 `runner` 流的 `lean run: started with only the Linear MCP server, without the user's Codex configuration, hooks, plugins, apps or memories; the user-level AGENTS.md is still read`。<br>• 6 次 `mcp_tool_call`，服务都是 `linear`；提交 `a7fa6f0` 推到 `test`，`main` 没动；AI 写了评论，把需求改成 In Testing。<br>• 运行开头 Codex 在标准错误打了两次 `failed to refresh available models: request timed out`（O2），这次运行照常完成。 |
| b Claude | 沙箱绑定改回 Claude 精简（`--permission auto`，不设推理力度），建 DEMO-42 | • 建好 2 秒后开始（07:35:33），31 秒走到待验收。<br>• 运行日志第一行是 `runner` 流的 `lean run: started with only the Linear MCP server and the repository's own settings; none of the user's own settings, plugins or hooks, and no auto memory`；第二行是 `init`，`mcp_servers` 只有 `linear-server`（connected），没有 `memory_paths`。<br>• 提交 `e3e2f46` 合并进 `test`（`fd8d377`），`main` 没动；AI 写了评论，改成 In Testing。 |

## 发布 v0.2.2

- 内容：PR #16（本报告的沙箱核对部分、计划末节，版本 `0.2.1` → `0.2.2`）。所有者定：必需的三项通过就合并，不等 PR 上的打包试跑，打包交给合并后 `main` 上的手动试打包。08:08 UTC 时 `secrets`、`test (macos-latest)`（6 分 42 秒）、`test (windows-latest)`（7 分 45 秒）与 `verify` 都已通过；08:08:52 UTC 以所有者权限经管理员绕过 squash 合并为 `ff3007d`，PR 里写明了。
- 试打包：在 `main` 的 `ff3007d` 上手动运行 release 工作流（运行 39567132179，08:09–08:35 UTC），全部通过：`updater=true`；两个打包任务的更新签名都核对通过（`verified IssueRelay.app.tar.gz`、`verified IssueRelay_0.2.2_x64_en-US.msi`、`verified IssueRelay_0.2.2_x64-setup.exe`）；`verify-dry-run` 输出 `ok assets` 与 `ok latest-json-dry-run`。
- 标签：附注标签 `v0.2.2` 打在 `ff3007d`。标签上的运行（39570583668）一次全部通过，包括 `bundle-release` 与 `verify-release`；草稿 08:36:34 UTC 建好。
- 草稿的资产（字节），与 `MAINTAINERS.md` 第 5 步要求的集合一致：

  | 文件 | 大小 |
  |---|---|
  | `IssueRelay_0.2.2_universal.dmg` | 27639752 |
  | `IssueRelay_0.2.2_universal.app.tar.gz` 与 `.sig` | 28322889 / 428 |
  | `IssueRelay_0.2.2_x64-setup.exe` 与 `.sig` | 10009422 / 440 |
  | `IssueRelay_0.2.2_x64_en-US.msi` 与 `.sig` | 14405632 / 440 |
  | `latest.json` | 5354 |

- 在本机核对草稿：dmg 没有下载标记；里面的 `IssueRelay.app` 是 0.2.2，`codesign --verify --deep --strict` 通过（ad hoc 签名），主程序是 x86_64 加 arm64，包里有命令行程序。
- 发布说明：在工作流生成的说明后面补了「改动」与「升级须知」：在 Codex 里用 bearer token 连 Linear、没有 OAuth 登录的人，升级后 Codex 的精简绑定会停在「精简运行未就绪」；已安装的程序会提示这个更新；Windows 只经 CI 测试。
- 本机换装（第 5 步，公开之前）：09:05 UTC，维护者同意后，确认没有运行在进行，经退出事件正常结束 0.2.1（宿主日志：`the system is ending IssueRelay without asking; stopping the runs first`），把 0.2.1 移到会话的临时目录作备份，从草稿的 dmg 拷入 0.2.2 并打开。约 10 秒连上 Linear：打开后 1 秒出现钥匙串授权进程（SecurityAgent），维护者看到对话框，选了总是允许（Always Allow）。命令行输出 `issuerelay 0.2.2`；`doctor` 里 Claude 与 Codex 的平常与精简检查都是 ready，两个绑定都是 `claude (lean): ok`。
- 公开：维护者在会话里明确说公开（09:52 UTC）。09:52:42 UTC 公开并设为最新版本；`releases/latest` 跳转到 v0.2.2，`releases/latest/download/latest.json` 返回 200，版本 0.2.2，9 个平台键都带签名，地址的格式与 v0.2.1 相同。

## 系统通知

- 环境：从草稿装上的 0.2.2 托盘程序（由 LaunchServices 打开）；沙箱绑定是 Claude 精简、`--permission auto`；设置里四种通知都开着。证据来自系统统一日志里 usernoted 与 NotificationCenter 的记录，以及 usernoted 的通知数据库里 IssueRelay 那几条（只读了 IssueRelay 的）。截图工具看不到横幅与通知中心（系统界面被过滤），所以没有截图。
- 第一次（DEMO-43，快任务，09:07–09:09 UTC）：两则通知都送到了系统，标题是需求编号与标题，正文分别是「已開始執行。」与「可以驗收了：需求已進入測試狀態。」（面板语言跟随系统，本机是繁体中文）。日志里两则都有 `Delivering`，通知中心也存下了，但没有 `Presenting`；数据库里这两条的 `style` 是 0。维护者起初回答看到了「开始运行」，到通知中心查看时什么都没找到。系统设置里 IssueRelay 的「允许通知」当时是关着的：维护者查看后打开了它（提示样式「暂时」，桌面、通知中心、锁定屏幕都勾选），打开后的设置页截图核对过。
- 打开以后（09:32–09:40 UTC），每一则在日志里都有 `Delivering` 与 `Presenting`，数据库里的 `style` 是 1：
  - DEMO-44（快任务）：开始运行、待验收。
  - DEMO-45（故意没写清楚改哪个文件、改成什么）：开始运行；AI 留言提问，把需求改回 Todo 并去掉 `ai-ready`，记为需要补充信息，通知随之发出。
  - DEMO-46（沙箱绑定临时改为 `--permission personal`，个人设置没有放行 Linear 的工具）：开始运行；运行判为失败（`run finished but the issue status is "Todo", not "In Testing"`），通知随之发出。之后把绑定改回，`config.toml` 与开工前的备份逐字节相同。
- 维护者当时不在电脑前，这六则都没有人亲眼看到。

## 新发现

| 编号 | 现象 | 处理 |
|---|---|---|
| O1 | 只差精简检查时，`doctor` 的绑定一行仍写通用的 `executor not ready`；具体原因在上面 `codex (lean runs)` 那一行，需求的等待原因已经分开写（c） | 观察项，没有改。绑定一行只回答这个绑定能不能派活（`BindingReport.executor_ready`）；要不要改成写明 `not ready for lean runs`，留给所有者。之后（同日，所有者说「升级版本吧」）随 v0.2.3 改了：`BindingReport.lean_not_ready`，命令行写 `executor not ready for lean runs`，面板的环境检查页与绑定页写「Codex 的精简运行还没有就绪」（简体中文；五种语言都有） |
| O2 | 精简的 Codex 运行开头，Codex 在标准错误打了两次 `failed to refresh available models: request timed out` | Codex 自己更新模型列表时超时，运行照常完成。没有查平常（不精简）的运行有没有同样的输出，记作观察 |
| O3 | 系统设置里 IssueRelay 的「允许通知」关着时，通知照样送到系统、存进通知中心的数据库，却不显示；程序、面板与 `doctor` 都看不出来，维护者也以为是开着的 | 观察项，没有改。可能的后续：环境检查读出通知的授权状态，关着时提醒（要先确认通知插件在 macOS 上能不能读到）；留给所有者。之后查过：`tauri-plugin-notification` 2.6.0 在桌面上的 `permission_state()` 总是返回 `Granted`，读不到；要读得自己调 macOS 的 `UNUserNotificationCenter`、Windows 的 `ToastNotifier.Setting`，并在签名后的安装包里实测。随 v0.2.3 只在安装说明的常见问题与 README 里写明去哪里打开「允许通知」 |
| O4 | `issuerelay bind list` 不写权限档：`--permission personal` 的绑定（运行用完整配置，不精简）列出来是 `[claude]`，与精简的绑定看起来一样；`doctor` 的绑定一行写明 `(full configuration: personal settings)` | 观察项，没有改：`bind list` 只写执行者、`lean off` 与推理力度（`crates/cli/src/format.rs` 的 `bindings`）；要不要加上权限档，留给所有者。之后随 v0.2.3 加上了：绑定自己设了权限档时写 `permission <值>`，例如 `[claude, permission personal]` |

## 与设计不一致之处

- 没有。

## 未闭环

- 面板需求列表里的「精简运行未就绪」：开发宿主没有面板，没有在真实面板上看；文字由 `Issues.test.tsx` 覆盖。
- 运行开头要不要要求 Claude 的 `linear-server` 是 `connected`（计划「留给所有者决定」表的最后一行）：这次没有造那种情形，仍未定。
- N1 的重读仍没在实机上走到：DEMO-41、DEMO-42 第一次核对就读到新值，宿主日志里没有「再读」。
- 系统通知：四种都由系统显示（见「系统通知」），但没有人亲眼看到；维护者在场时随便一条运行即可补看。
- Windows：所有者决定暂不做。
- 本机：托盘程序是 0.2.2（从草稿装上）。0.2.1 的备份原先留在会话的临时目录里，10:27 UTC 按维护者的话与其他临时文件一起删掉了；要回退就用 GitHub 上 v0.2.1 的 dmg。
- 沙箱里的测试需求：DEMO-41 至 DEMO-44 待验收（In Testing）；DEMO-45 是 Todo，`ai-ready` 已被 AI 去掉；DEMO-46 是 Todo、仍带 `ai-ready`，运行记为失败，它的 `updatedAt` 没变，所以不会被重新派活。
