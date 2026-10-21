# 精简运行遗留小项（PR #15）：沙箱实机核对（2026-10-21）— macOS 通过

## 结论

- 计划末节「待实机的检查」里 PR #15 的几项，在 macOS 沙箱上都看到了：
  - `doctor` 的绑定一行写出推理力度：设了 `high` 的绑定是 `codex (lean), effort high`，没设的那一行不写。
  - 精简运行日志开头的说明：Claude、Codex 各一次，都是运行日志的第一行，`runner` 流，以 `lean run: ` 开头。
  - 只差精简检查时，需求记为等待（`waiting_binding`），原因是 `executor codex is not ready for lean runs`；`issuerelay jobs` 与 MCP 控制服务的 `list_jobs` 相同。
  - Codex 的 Auth 栏：配置里设了 bearer token 时（`Bearer token`）出现新的精简问题；照它的建议删掉 token、运行 `codex mcp login linear` 以后，问题消失。本机平常的配置是 `OAuth`，精简可用，精简的 Codex 运行走到待验收。
- 没有看：面板需求列表里的那一行。开发宿主没有面板，与 2026-10-20 一样用命令行代替（所有者 2026-10-21 定），面板的文字由组件测试覆盖。
- 新的观察两条（O1、O2），都没有改代码。

## 环境

- 日期：2026-10-21，日志时间是 UTC（本机是 UTC+8），07:19–07:37。
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

## 新发现

| 编号 | 现象 | 处理 |
|---|---|---|
| O1 | 只差精简检查时，`doctor` 的绑定一行仍写通用的 `executor not ready`；具体原因在上面 `codex (lean runs)` 那一行，需求的等待原因已经分开写（c） | 观察项，没有改。绑定一行只回答这个绑定能不能派活（`BindingReport.executor_ready`）；要不要改成写明 `not ready for lean runs`，留给所有者 |
| O2 | 精简的 Codex 运行开头，Codex 在标准错误打了两次 `failed to refresh available models: request timed out` | Codex 自己更新模型列表时超时，运行照常完成。没有查平常（不精简）的运行有没有同样的输出，记作观察 |

## 与设计不一致之处

- 没有。

## 未闭环

- 面板需求列表里的「精简运行未就绪」：开发宿主没有面板，没有在真实面板上看；文字由 `Issues.test.tsx` 覆盖。
- 运行开头要不要要求 Claude 的 `linear-server` 是 `connected`（计划「留给所有者决定」表的最后一行）：这次没有造那种情形，仍未定。
- N1 的重读仍没在实机上走到：DEMO-41、DEMO-42 第一次核对就读到新值，宿主日志里没有「再读」。
- 系统通知：等 v0.2.2 装到本机以后核对。
- Windows：所有者决定暂不做。
- 沙箱里的测试需求：DEMO-41、DEMO-42 待验收（In Testing）。
