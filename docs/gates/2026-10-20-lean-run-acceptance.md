# 精简运行：沙箱实机验收与 v0.2.0、v0.2.1 发布（2026-10-20）— macOS 通过，Windows 待实机

## 结论

- 精简的 Claude 运行（设计第 11 节第 1 项）通过：只加载 `linear-server` 与内置插件，没有用户级 `CLAUDE.md`、自动记忆与用户钩子；`init` 是运行日志的第一行。DEMO-32 被误判为失败（N1）；修正后的 DEMO-40 走到待验收。
- 中途停止（第 2 项）通过：执行者、它为工具开的 shell（已离开执行者的进程组）、`ping` 都被结束，没有脱离进程树的后台进程。
- 关掉精简（第 3 项）通过：与以前一样加载全部配置。
- 精简的 Codex 运行（第 4 项）通过：DEMO-39 走到待验收，所有 `mcp_tool_call` 的服务都是 `linear`，会话存档里没有用户的插件与技能，只多读了用户级 `~/.codex/AGENTS.md`（L6 已接受）。第一次（DEMO-34）被 Codex 的用量上限挡住。
- 推理力度（第 4a 项）：设了就按 IssueRelay 的设置传，都不设时不传。不精简的运行里，用户设置 `env` 里的推理力度盖过了传的值（N2）。
- 按个人设置（第 5 项）通过：精简不适用，照常加载全部配置。
- 环境检查（第 6 项）通过：两个执行者的精简检查都显示可用；`lean-check/` 里的精简登录检查不受上层目录的项目设置影响。
- 新发现 N1–N5 见下。N1（结果核对读到旧值，快的运行被判失败）已在本 PR 修好；DEMO-40 这次第一次核对就读到了新值，没有走到重读，重读的行为由测试覆盖。
- 未验证：Windows（第 7 项）；面板上的显示（开发宿主没有面板，见「与设计不一致之处」）。

## 环境

- 日期：2026-10-20，日志时间是 UTC（本机是 UTC+8）。两段：03:14–03:52（DEMO-31 至 DEMO-38）与 06:28–06:43（DEMO-39、DEMO-40）。
- 软件版本：macOS 27.0.1（26A434），Apple 芯片；Claude Code 2.1.285；codex-cli 0.159.2；rustc 1.99.0。
- IssueRelay：分支 `feat/lean-run`（PR #7）的调试构建，开发宿主 `issuerelay-devhost` 加命令行 `issuerelay`，与托盘程序用同一个数据目录。第一段用 `13ccea2`，第二段用含 N1 修正的 `2c432e5`。
  - 每段之前退出已装的托盘程序（0.1.0），备份 `config.toml` 与 `state.db`；之后恢复配置（与备份逐字节相同），重新打开托盘程序，没有重跑任何运行。
  - 第二段的构建是新的。开发宿主启动后约 2 分 44 秒才连上 Linear（06:28:33 启动，06:31:17 连上），其间系统里有一个钥匙串授权进程（SecurityAgent）在运行；维护者没有看到对话框，也没有输入密码，等待的原因没有查明。
- 执行者的环境：开发宿主用 `env -i` 启动，只留 `HOME`、`USER`、`LOGNAME`、`SHELL`、`TMPDIR`、系统默认的 `PATH`、`__CF_USER_TEXT_ENCODING` 与 `SSH_AUTH_SOCK`，接近由 launchd 启动的托盘程序。DEMO-31 那次是从 Claude Code 会话的终端直接启动的，见 N3。
- 沙箱：Demo Project 与 `~/issuerelay-sandbox/demo`（本地裸仓库充当远端）。每项之前用 `issuerelay bind add` 改这个绑定的执行者、权限档、精简与推理力度。第一段在设置里把默认推理力度设为 `high`，第二段没有设。
- 测试需求：DEMO-31 至 DEMO-40。每条建好后运行 `issuerelay poll`，7 到 12 秒内开始运行。
- 本机的用户配置（只记数量与类别）：
  - Claude：`~/.claude/settings.json` 开了 54 个插件，有 `PreToolUse` 与 `SessionEnd` 两类钩子，`env` 里设了 `CLAUDE_CODE_EFFORT_LEVEL`；`~/.claude.json` 有 6 个用户级 MCP 服务（含 `linear-server`）；有用户级 `~/.claude/CLAUDE.md`。
  - Codex：`config.toml` 里有 7 个 MCP 服务（含 `linear`，用 OAuth）与 23 个插件；有 11 个用户技能、用户级 `~/.codex/AGENTS.md` 与 `hooks.json`（`PreToolUse`）。
- 核对方法：运行日志与会话存档用两个只打印数量和名称的脚本读，不打印内容。用户级说明文件用其中只有它才有的几个短语计数。

## 步骤与结果

| 项 | 做法 | 结果 |
|---|---|---|
| 6 环境检查 | `issuerelay doctor` | `claude (lean runs): ready`，`codex (lean runs): ready`；沙箱绑定显示 `(lean)`。改用 `env -i` 重启后相同。 |
| 6 上层目录 | 在临时目录里做两个反面核对：(a) 项目设置 `.claude/settings.json` 的 `env` 改成走 Bedrock；(b) 用临时 `CLAUDE_CONFIG_DIR`，把用户设置的 `env` 改成走 Bedrock | (a) 在没被信任的目录里，`auth status` 不套用项目设置的 `env`，仍报 `firstParty`。所以 `lean-check/` 往上层找到项目设置，也影响不了检查。<br>(b) 平常的 `auth status` 报第三方（Bedrock），`--setting-sources project,local auth status` 不报。两者不一致，精简登录检查能拦下。 |
| 1 精简 Claude | DEMO-32（`env -i` 启动）：新增一个文件，推到 `test` | • 运行日志第一行就是 `init`，之前没有任何事件（也没有钩子事件）。<br>• `mcp_servers` 只有 `linear-server`（connected），`mcp__` 工具都来自它；没有 `memory_paths`；插件只有内置的 `cc-plugin-agents-md` 与 `cc-plugin-telemetry`；18 个技能。<br>• 会话存档里，用户级 `CLAUDE.md` 的四个短语都出现 0 次；沙箱的 `AGENTS.md` 读到了。<br>• 用户的自动记忆目录没有新文件；沙箱仓库里插件的 `.remember/` 还是 10-14 的。<br>• AI 推了提交 `43885c7`，写了评论，把需求改成 In Testing；运行却被判失败，原因是读到的状态是 Todo（N1）。 |
| 1 对照 | DEMO-31：同上，但开发宿主从 Claude Code 会话的终端启动 | 精简部分与 DEMO-32 相同，走到待验收（提交 `722ee05`）。会话存档的推理力度是 `max`：会话的 `CLAUDE_CODE_EFFORT_LEVEL` 漏进了执行者（N3）。 |
| 1 修正后 | DEMO-40（第二段，`env -i` 启动）：同样的快任务 | • 44 秒走到待验收；提交 `af745e0`，合并进 `test`（`834ad19`），`main` 没动。<br>• 精简的各项与 DEMO-32 相同：`init` 是第一行，只有 `linear-server`，没有 `memory_paths`，只有内置插件，用户级 `CLAUDE.md` 的短语 0 次，自动记忆目录没有新文件。<br>• 第一次核对就读到 In Testing，主机日志里没有「再读」的记录（N1 的旧快照这次没有出现）。 |
| 4a 推理力度 | 第一段：设置默认 `high`，Claude 绑定不另设；Codex 绑定另设 `max`。第二段：设置与 Claude 绑定都不设 | • DEMO-32 的命令行有 `--effort high`，会话存档的 `effort` 都是 `high`。<br>• DEMO-34、DEMO-39 的 `turn_context.effort` 是 `max`。<br>• DEMO-40 的命令行没有 `--effort`；会话存档的 `effort` 是 `medium`，也就是 Claude 自己的默认。 |
| 3 关掉精简 | DEMO-33，绑定 `--lean off` | • 命令行没有精简参数，也没有关自动记忆的变量，只有 `--effort high`。<br>• `init` 之前有 30 个用户插件的 `SessionStart` 钩子事件；`init` 里有 39 个 MCP 服务、59 个插件、297 个技能，有 `memory_paths`。<br>• 会话存档里有用户级 `CLAUDE.md`；插件往沙箱仓库的 `.remember/` 写了新文件。<br>• 走到待验收（提交 `c459f6d`）。会话存档的推理力度是 `max`，不是传的 `high`（N2）。 |
| 4 精简 Codex | DEMO-34，Codex 绑定，权限完全放开，推理力度 `max` | • 参数与设计第 5.2 节逐项一致。<br>• `turn_context`：`effort` 是 `max`，模型是 Codex 的默认模型。<br>• 会话存档里没有用户技能：一个用户技能的名字在平常的会话里出现 27 次，这里 0 次。<br>• 读了用户级 `~/.codex/AGENTS.md`（L6 已接受）。<br>• 到了 `turn.started`，说明必需的 `linear` 服务启动成功。随后被 Codex 的用量上限挡住，以退出码 1 结束（运行记录的原因是 `executor exited with code 1`），需求没动。 |
| 4 重做 | DEMO-39（第二段），同一个绑定 | • 约 2 分钟走到待验收；提交 `3aa57c2` 推到 `test`，`main` 没动。<br>• 运行日志里 6 次 `mcp_tool_call`，服务都是 `linear`。<br>• 会话存档：`effort` 是 `max`；没有用户插件的标识（`…@openai-…`）、插件目录或用户技能目录；「Available skills」只出现在基础指令的通用说明里，下面没有条目；用户自己的其他 MCP 服务一个都没出现。<br>• 读了用户级 `~/.codex/AGENTS.md`：存档里出现的用户技能名都来自这份文件的文字。 |
| 5 按个人设置 | DEMO-35，绑定 `--permission personal` | `doctor` 显示 `claude (full configuration: personal settings)`，运行照常加载全部配置。用户设置里没有允许 Linear 的工具，`-p` 运行时被拒，AI 改不了需求，运行失败。这是「按个人设置」的本意。 |
| 2 中途停止 | DEMO-38：先在前台运行 `ping -c 90 127.0.0.1`，再新增文件。`ping` 开始后，给开发宿主发一次 SIGINT | • 第一次信号走 `StopMode::StopRuns`，与面板的「停止」一样，经运行的停止令牌结束进程树。<br>• 2 秒内宿主退出，运行记为已取消。<br>• 停止前记下的执行者、它为工具开的 shell（`setsid`，自成进程组）、`ping`、`tail` 全部结束。<br>• 之后没有新出现、父进程为 1 的非系统进程。 |
| 2 没成功的两次 | DEMO-36：先 `sleep 90`；DEMO-37：先在前台 `sleep 90` | DEMO-36：AI 把 `sleep` 放到后台，`-p` 会话随即结束；运行 28 秒后失败，需求停在 In Progress。<br>DEMO-37：Claude Code 拦下单独的前台 `sleep 90`。AI 按提示词改回 Todo，去掉 `ai-ready`，并写评论说明原因（N4）。 |

## 新发现

| 编号 | 发现 | 处理 |
|---|---|---|
| N1 | 结果核对读到了 Linear 的旧值。DEMO-32 在 03:21:54 开始前读过一次（Todo）。AI 在 03:22:09 改成 In Progress，03:22:25.9 改成 In Testing；引擎 03:22:32 核对时读到的仍是 Todo，`updatedAt` 还是建需求的时间，于是判失败。IssueRelay 自己不缓存；看起来 Linear MCP 对同一连接的读取会在一段时间内返回旧快照。DEMO-31 跑了约 85 秒，没有遇到。精简运行起得快、跑得快（DEMO-32 约 38 秒，不精简的 DEMO-33 约 2 分 18 秒），更容易落进这段时间。 | 本 PR 修（`1b45f15`）：运行正常结束、按读到的状态要判失败、而需求的 `updatedAt` 还是开始前读到的值时，间隔 15、30、45 秒再读，以最后一次为准；运行被停止或程序退出时不再等。真正没动需求的运行因此晚约 90 秒才标为失败；AI 留在 Todo 并说要补充信息的，不再读。测试见 `crates/core/tests/engine.rs` 新增的七个；主设计第 13 节加了一行。DEMO-40 用修正后的构建跑同样的快任务，第一次核对就读到新值：修正在实机上没有走到，旧快照何时出现还说不准。 |
| N2 | 用户设置的 `env` 里的 `CLAUDE_CODE_EFFORT_LEVEL` 盖过命令行的 `--effort`。不精简的运行读用户设置，所以 IssueRelay 传的 `--effort high` 没有生效，会话存档是 `max`。设计第 5.6 节「不精简时它会盖过用户设置里的推理力度」，对用 `env` 设的推理力度不成立。精简运行不读用户设置，不受影响。 | 记入计划的遗留项，由维护者决定：在设计、帮助与 README 里写明这个例外，或另想办法。IssueRelay 不改用户的设置。 |
| N3 | 从 Claude Code 会话的终端启动开发宿主，会把这个会话的一整串环境变量（`CLAUDE_CODE_EFFORT_LEVEL`、`ANTHROPIC_BASE_URL`、`CLAUDE_CODE_CHILD_SESSION` 等）传给执行者；`ExecEnv` 只去掉 `CLAUDECODE` 与 `CLAUDE_CODE_ENTRYPOINT`。打包的托盘程序由 launchd 启动，不受影响。 | 属验收方法：之后的运行都用 `env -i` 启动开发宿主。记入遗留：要不要再去掉 Claude Code 会话自己的变量（`ANTHROPIC_BASE_URL` 等可能是用户的网关，不能一概去掉）。 |
| N4 | 需求要求「先等一会儿」时，无人值守运行会失败：放到后台的命令让 `-p` 会话提前结束（DEMO-36），单独的前台 `sleep` 又被 Claude Code 拦下（DEMO-37）。真实需求里少见，但跑测试、等服务起来时会遇到。 | 记入遗留，由维护者决定是否在提示词里加一条（提示词固定规则属极高风险，要单独决定）。 |
| N5 | 用 `issuerelay bind add` 改完绑定后马上运行 `doctor`，两个绑定都报 `executor not ready`；几秒后再查就正常了。 | 记入遗留：查清原因。猜测是配置变了以后，就绪检查重做期间给出了空结果。 |

## 发布 v0.2.0

- 版本号：PR #9 把 `0.1.0` 升到 `0.2.0`，11:45:36 合并为 `79ce58f`。它的 CI 里 Windows 的 `test` 第一次失败在 `crates/core/tests/ipc.rs` 的空闲断开测试（暂停的时钟在等真实管道断开时自己往前跳），只重跑这个任务后通过；测试本身由 PR #10 修好，12:28:57 合并为 `42afe2a`，只改测试文件。
- 试打包（`MAINTAINERS.md`「发布」第 2 步）：在 `main` 的 `79ce58f` 上手动运行 release 工作流（运行 39417306280），全部通过：`verify` 输出 `updater=true`，两个打包任务分别打出 `verified IssueRelay.app.tar.gz`、`verified IssueRelay_0.2.0_x64-setup.exe` 与 `verified IssueRelay_0.2.0_x64_en-US.msi`，`verify-dry-run` 通过。
- 标签：`v0.2.0` 打在当时的 `origin/main`，也就是 `42afe2a`。PR #10 只改测试文件，不在发布相关文件之列，所以没有重做试打包（第 7 步）。标签上的运行（39422712075）全部通过，包括两个平台的 `release-test`、`bundle-release` 与 `verify-release`。
- 草稿的资产（字节），与第 5 步要求的集合一致：

  | 文件 | 大小 |
  |---|---|
  | `IssueRelay_0.2.0_universal.dmg` | 27635050 |
  | `IssueRelay_0.2.0_universal.app.tar.gz` 与 `.sig` | 28316006 / 428 |
  | `IssueRelay_0.2.0_x64-setup.exe` 与 `.sig` | 10014227 / 440 |
  | `IssueRelay_0.2.0_x64_en-US.msi` 与 `.sig` | 14401536 / 440 |
  | `latest.json` | 5354 |

- 在本机核对草稿（没有安装）：
  - `latest.json`：版本 `0.2.0`，9 个平台键，地址都指向本 Release 的资产，macOS 各键的签名等于 `.app.tar.gz.sig` 的内容。
  - 用 `gh` 下载 dmg（没有下载标记），只读挂载：`IssueRelay.app` 的版本是 0.2.0，标识 `com.sunkailun.issuerelay`，最低系统 13.3；`codesign --verify --deep --strict` 通过（ad hoc 签名）；主程序与命令行程序都是 x86_64 加 arm64；包里的 `issuerelay --version` 输出 `issuerelay 0.2.0`。
  - 没有按第 5 步从草稿装一遍：换装要先退出正在监看的托盘程序，维护者决定不等，直接发布（2026-10-20）。本机仍是 0.1.0，之后经自动更新装上 0.2.0。
- 发布说明：在工作流生成的说明后面补了「新功能」与「升级须知」：已有的绑定会变成精简运行，Linear 服务用了别的名字时精简绑定会停在「执行者未就绪」；用户设置 `env` 里的推理力度在不精简的运行里仍然生效（N2）；0.1.0 会在程序里提示这个更新；Windows 只经 CI 测试。
- 公开：维护者在会话里让 AI 直接发布（第 6 步写的是维护者亲手点，与 v0.1.0 一样由维护者当场授权）。2026-10-20 12:59:28 UTC 公开，并设为最新版本；`releases/latest` 跳转到 v0.2.0，`releases/latest/download/latest.json` 返回 200，版本 0.2.0。
- Windows：所有者决定这次不做 Windows 实机验证（2026-10-20），发布说明里写明。

## 发布 v0.2.1

- 内容：PR #12（所有者让主智能体决定的六件事：精简的 Codex 运行正常结束时要认出一次 Linear 调用；用户设置的 `env` 里有网关或代理时 Claude 的精简判为不可用；其余四件写进文档），版本 `0.2.0` → `0.2.1`。2026-10-20 16:29:52 UTC 合并为 `15922ee`。
- PR #12 的必需检查 `test (macos-latest)` 没有运行：GitHub 注明「The job was not acquired by Runner of type hosted even after multiple attempts」（macOS arm64 执行器容量不足），15 分钟后取消。所有者决定用同一个提交上已通过的 `release-test (macos-latest)`（同样的验证命令）作为 macOS 的结果，PR 里写明了。
- 试打包：在 `main` 的 `15922ee` 上手动运行 release 工作流（运行 39457997798），全部通过：`updater=true`，两个打包任务的更新签名都核对通过，`verify-dry-run` 通过。
- 标签：`v0.2.1` 打在 `15922ee`。标签上的运行（39462904857）第一次失败：`release-test (macos-latest)` 同样没分到执行器，被取消，`create-release` 跳过，没有建出 Release。这个标签还没有任何 Release，所以只重跑失败的任务（第 2 次）；`MAINTAINERS.md` 不让重跑的，是草稿已被标成「incomplete」的情形。第 2 次全部通过，包括 `bundle-release` 与 `verify-release`。
- 草稿的资产（字节），与第 5 步要求的集合一致：

  | 文件 | 大小 |
  |---|---|
  | `IssueRelay_0.2.1_universal.dmg` | 27654065 |
  | `IssueRelay_0.2.1_universal.app.tar.gz` 与 `.sig` | 28331120 / 428 |
  | `IssueRelay_0.2.1_x64-setup.exe` 与 `.sig` | 10012143 / 440 |
  | `IssueRelay_0.2.1_x64_en-US.msi` 与 `.sig` | 14397440 / 440 |
  | `latest.json` | 5354 |

- 在本机核对草稿：`latest.json` 的版本是 `0.2.1`，9 个平台键，地址都指向本 Release 的资产，macOS 各键的签名等于 `.app.tar.gz.sig`。dmg 里的 `IssueRelay.app` 是 0.2.1，`codesign --verify --deep --strict` 通过（ad hoc 签名），主程序与命令行程序都是 x86_64 加 arm64，包里的 `issuerelay --version` 输出 `issuerelay 0.2.1`。
- 发布说明：在工作流生成的说明后面补了「改动」与「升级须知」：用户设置的 `env` 设了网关或代理时，Claude 的精简绑定会停在「执行者未就绪」；已安装的程序会提示这个更新；Windows 只经 CI 测试。
- 公开：维护者让 AI 一路做到公开。2026-10-20 18:00:35 UTC 公开并设为最新版本；`releases/latest` 跳转到 v0.2.1，`releases/latest/download/latest.json` 返回 200，版本 0.2.1。
- 本机更新托盘程序：18:01 UTC，确认没有运行在进行，退出 0.1.0，把它移到废纸篓，从草稿下载的 dmg 拷入 0.2.1（没有下载标记）并打开。5 分钟后它仍是「connecting to Linear」，系统里有一个钥匙串授权进程（SecurityAgent）在等回应：新构建是 ad hoc 签名，要先在钥匙串里得到同意才读得到 Linear 凭据，这 5 分钟里没有人回应。按事先说好的做法，退出 0.2.1，换回备份的 0.1.0（同一个签名），18 秒后连上 Linear，监看恢复。本机仍是 0.1.0，等维护者在程序里点更新、在钥匙串对话框里同意后才是 0.2.1。

## 与设计不一致之处

- 设计第 5.6 节的推理力度一句，见 N2。
- 设计第 11 节第 1 项要看面板显示「待验收」：开发宿主没有面板，用 `issuerelay jobs` 看状态。面板的精简显示由截图与组件测试覆盖。
- 第 8 节举的是「AI 的消息、工具调用或结果」不能出现在 `init` 之前（最终评审的遗留项）：实测精简运行里 `init` 之前什么都没有，不精简的运行之前是用户插件的钩子事件。

## 未闭环

- 本机的托盘程序仍是 0.1.0：更新到 0.2.1 要维护者在钥匙串对话框里同意（见「发布 v0.2.1」）。
- Windows（第 7 项）全部待实机，清单见设计第 5.5 节与计划的遗留项。
- N1 的重读在实机上没有走到：DEMO-40 没有遇到旧快照。
- `~/.claude` 是符号链接时的排除：本机不是符号链接，没有测。
- 沙箱里的测试需求保持原样：DEMO-31、DEMO-33、DEMO-39、DEMO-40 待验收；DEMO-32 在 Linear 是 In Testing，记录是失败（N1）；DEMO-34、DEMO-35 是 Todo；DEMO-36、DEMO-38 停在 In Progress；DEMO-37 是 Todo，`ai-ready` 已被 AI 去掉。
