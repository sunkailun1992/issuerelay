# IssueRelay 使用指南

IssueRelay 是面向 Linear 与本机 Codex、Claude Code 用户的桌面任务调度器。它把需求、工作目录和无人值守的 AI 运行连起来，保存队列与运行记录，让你能知道哪些任务在等待、正在执行或需要验收。

它适合已经用 Linear 管理需求、希望把明确的开发任务交给本机 AI 命令行执行的个人开发者，也适合团队成员各自在自己的电脑上使用。本文描述仓库当前源码中的工作流。安装、系统要求与卸载见[安装说明](install.md)，各安装包支持的功能见 [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases)。

从旧版升级时，先正常退出并备份应用数据。本版本将运行数据库迁移到 schema 7，旧版不能打开新数据库。回退到旧版时，先退出应用并使用升级前备份。 [升级与备份](install.md#升级前备份数据)

## 谁负责什么

| 角色 | 职责 |
|---|---|
| 你与主控 AI 会话 | 澄清目标、选择任务和执行者、维护项目规则、检查结果；主控入口可用 Codex 或 Claude，由你选择 |
| Linear | 保存需求、指派、标签、项目与依赖关系，作为触发和状态的来源 |
| IssueRelay | 发现需求、按绑定分流、排队、处理依赖与目录互斥、启动执行者、记录结果，并提供面板与控制接口 |
| Codex / Claude Code 执行者 | 在指定目录分析需求和代码；按运行模式出方案或实现、验证，操作 git 并向 Linear 留下结果 |

一次运行使用一个执行者。主控聊天负责确认任务和检查结果，后台会话按此次运行的权限、仓库规则和工具配置工作。需要另一会话复核或接手时，使用运行详情里的材料手动开始。

## 两种运行模式

**实现模式 `ai-ready`。** 执行者读取需求、改代码并验证。获准提测时，按项目约定交付测试分支与结果，将需求移到 `In Testing` 并留言。IssueRelay 核对 Linear 状态后显示「待验收」。派发范围也可以明确限定为本地实现与验证。

**方案模式：`ai-plan`。** 先读需求和代码，把方案写进 Linear 评论。这次运行限制改文件的能力，只出方案。认可方案后，可以从该次运行详情的「执行开发」准备实现模式的派发预览。方案运行仍会读取你有权访问的文件，并把内容发送给 AI 服务、写入 Linear；工具限制细节与「按个人设置」的例外见 [README 的安全须知](../README.md#安全须知)。

## 准备与一次任务的生命周期

先完成首次引导：授权 IssueRelay 访问 Linear，安装并登录至少一个执行者，为执行者配置并授权 Linear MCP，再绑定 Linear 项目与本地目录。IssueRelay 的 Linear 登录与 CLI 的登录分别处理。执行者可在设置与项目绑定中选择，不需要两个都安装。

默认需要 Linear 有 `ai-ready` 标签，以及 `Todo`、`In Progress`、`In Testing`、`Done` 状态；使用方案模式时再准备 `ai-plan`。工作区名字不同时，在「设置 → 高级 → 触发用的名字」里对应修改，IssueRelay 不会自动创建它们。

1. 在 Linear 写清目标与验收条件，指派给自己并设为 `Todo`。从聊天发起时，让主控会话整理本次任务的摘要和完整任务相关历史，保留相关的复现命令、报错、版本、决定和未决事项，附上 Wiki 页面或小节引用，再通过 IssueRelay MCP 提交上下文草稿。再由你或已授权的流程添加 `ai-ready` 或 `ai-plan`。
2. IssueRelay 发现候选需求。在桌面面板「需求」找到等待派发确认的任务，打开预览核对会话摘要、相关历史、范围、验收条件、配置的 Wiki 目录和引用，以及运行模式和执行去向。直接在 Linear 创建、没有前序会话的需求可确认空的会话上下文；提交了任务上下文的需求必须带任务相关历史。确认后，IssueRelay 将获准的上下文写入本机任务记录。
3. 确认后按项目绑定、目录、执行者就绪状态与依赖排队。同一目录一次只执行一条，同时受总并发上限约束。实现任务被未完成的 Linear 需求阻塞时先等待，方案模式不等待这些依赖。
4. 开跑前再次核对 Linear 需求和派发条件。需求、草稿或实际执行去向已变化时，重新预览。启动后，运行详情显示本轮使用的执行者和已确认的任务快照。
5. 实现交付后，审阅 diff、测试和实际行为。对「待验收」的运行，配置了 Wiki 目录时，按该库规则写入归档并登记回执。验收通过后，你可在面板或支持表单的会话中确认「标记完成」，IssueRelay 再把 Linear 状态设为 `Done`。

### 只做本地实现与验证

例如 `DEMO-1` 本轮只做本地实现并运行单测。在任务范围写明这一点，在派发预览中核对并确认。执行者按获准范围在本地工作，Linear 状态保持原样。实施执行者成功退出且报告 `local_complete` 时，运行显示「待验收」，详情注明本地开发和验证已完成。IssueRelay 无需回读 Linear 来确认本地结果；Linear 工单进入配置的提测状态、保留触发标签并仍指派给你后，最新一轮运行显示「标记完成」。缺少标签时先在 Linear 修正并刷新面板。确认时会再次检查 Linear 状态，并沿用正常流程的 Wiki 回执要求。

如果旧版本把已经完成的本地工作记录为「待补充信息」，先核对实际改动和验证结果。只有该需求最新、已结束且原报告为 `needs_info` 的实施运行可在桌面详情点「记录本地完成」并确认。有交接保留时，先释放交接。这次确认只将运行移入「待验收」，原报告、日志和结束时间可继续查看，交接材料也保留。操作不会写入 Linear 或重跑任务。

### 从方案进入开发

在已结束的方案运行详情点「执行开发」。如果该需求尚未满足实现条件，可以打开面板提供的 Linear 需求链接，将它设为 Todo、指派给自己并添加 `ai-ready`。回到面板重新读取预览，确认同一需求的实现派发。这一步只针对当前需求，子需求分别派发。

退回需求或从方案转实现时，开启接续设置、前次有可用会话、结束未满七天且执行者与目录相同等条件满足，才可能沿用前次会话。部分 Codex 模式不支持接续。手动「重新运行」需要再次确认派发预览，然后新开后台会话。已中断的尝试也可按下文的「继续上次」流程处理。

### 方案收口

方案拆成 Linear 子需求后，子需求各自派发、验收和标记完成。方案运行详情显示子需求进度，例如「子需求：3/5 已结束」，并不再提供整包「执行开发」。每个待收口方案的子需求最多每 2 分钟读取一次，每轮检查最多读 20 个，其余在打开详情时读取；已删除的子需求不计入。

子需求全部结束（完成或取消）且至少一个完成时，运行显示「可收口」，详情出现「完成方案」。确认后，IssueRelay 重新核对：该运行仍是这条需求的最新一轮且没有交接领取；配置了 Wiki 时已有归档回执；方案需求在 Linear 中存在且未取消；子需求仍全部结束。核对通过后，把方案需求设为配置的完成状态，并把方案运行记为已完成。Linear 中方案已是完成状态时只补本地记录。子需求不会被改动。

子需求读取失败时，详情显示原因，并保留原来的「执行开发」入口。子需求全部取消时不能在这里收口，请在 Linear 处理方案需求。可在桌面面板或支持原生确认的 MCP 会话里确认收口。收口后又出现未结束的子需求时，详情会提示，不会自动重开。

## 执行者、模型与推理力度

IssueRelay 依次采用以下设置，运行详情会显示实际采用的来源：

1. 需求上下文中明确指定的 `preferred_executor`。
2. 项目绑定中明确选择的执行者。
3. 登记需求的入口。IssueRelay 能识别部分 Codex 和 Claude MCP 客户端名称，并保留第一次识别到的入口。
4. 「设置 → 默认值」里的全局执行者。

例如，用 Codex 登记需求，且未指定执行者、绑定也选择「默认」时，后台用 Codex。明确选择 Claude 时会优先用 Claude。IssueRelay 不会因某个执行者额度耗尽而自动切换到另一个执行者。

可分别设置 Codex 和 Claude 的默认模型，也可在绑定里覆盖该执行者的模型。Codex 默认留空，沿用 CLI 默认模型。目录从本机 Codex CLI 读取，可手动刷新，读取失败时显示内置推荐。Claude 默认使用官方别名 `fable`，也可选择 `opus`、`sonnet` 或输入自己的模型。清空默认模型字段后由该 CLI 选择，别名解析随已安装版本与提供方而变。

Ultra 由支持它的 Codex 模型提供。Claude 使用自己的推理档位，Ultracode 独立控制动态工作流，需要 Claude Code 2.1.284 或更新版本、支持 `xhigh` 的模型和已启用的工作流。绑定里设置的推理力度优先于全局默认。Claude 运行在 IssueRelay 指定力度时，CLI 参数和该子进程的 `CLAUDE_CODE_EFFORT_LEVEL` 使用同一个值。未指定力度时沿用执行者的环境和设置。[Claude Code 模型配置](https://code.claude.com/docs/en/model-config#adjust-effort-level)

## 用 MCP 连接主控会话

Linear MCP 用于读写需求；IssueRelay MCP 用于查看和控制本机调度器，两者用途不同。主控会话需要后者时，在面板「设置 → MCP 控制服务」为 Codex 或 Claude Code 注册即可。它通过 `issuerelay mcp` 的标准输入输出通信，注册使用命令行程序的完整路径，不依赖 PATH。

也可手动注册，把 `<ISSUERELAY_CLI>` 换成已安装的 IssueRelay 命令行程序的完整路径，按所用客户端选一条：

```bash
claude mcp add --scope user issuerelay -- "<ISSUERELAY_CLI>" mcp
codex mcp add issuerelay -- "<ISSUERELAY_CLI>" mcp
```

先打开桌面程序。客户端显示的工具名可能带前缀：

| 工具 | 参数与作用 |
|---|---|
| `status` | 无参数；查看监控和队列状态 |
| `list_jobs` | 可选 `limit`，默认 20、最多 500；查看最近运行 |
| `pause` | 无参数；暂停轮询与启动新运行，已经运行的任务继续 |
| `resume` | 无参数；恢复监控并立即检查，不接续某个任务 |
| `poll_now` | 无参数；请求立即检查；暂停时不执行轮询，也不解除暂停 |
| `get_run` | 传正整数 `id` 或 `issue_identifier`，二选一。按需求编号读取最近一次运行。返回运行与交接详情，当前保留的标识在 `handoff.claim_id` |
| `get_issue_context` | 传 `issue_identifier`，读取最新草稿或已保存的任务上下文和 `revision` |
| `set_issue_context` | 必须传 `issue_identifier`、`expected_revision`、`request`、`scope` 和非空的 `conversation_history`。可带 `acceptance`、`source_refs`、`preferred_executor`。把待确认的草稿暂存在本机内存 |
| `get_handoff` | 传 `job_id`，直接返回交接对象，包括当前保留的 `claim_id`、检查点、本轮快照和复核提示词。配置 Wiki 时还包含归档材料和回执 |
| `claim_handoff` | 传 `job_id` 和 `executor`（`codex` 或 `claude`）。保留本机工作目录，直接返回包含本次 `claim_id` 的交接对象 |
| `release_handoff` | 传 `job_id` 和自己领取时取得的 `claim_id`，释放该次目录保留 |
| `record_archive` | 传 `job_id` 和相对当前 Wiki 根的 `page`，Git Wiki 还要传完整 `commit` SHA。登记与当前 Wiki 根绑定的回执 |
| `list_issues` / `get_plan_progress` / `refresh_plan_progress` | 查看候选需求、方案子需求进度及当前阻塞原因，刷新方案时传 `job_id` |
| `get_continuation_context` / `supplement_continuation_context` | 按 `job_id` 读续作材料或按 `expected_revision` 追加 `supplement` |
| `get_development_issue_url` | 按 `job_id` 取得方案转开发所需需求链接 |
| `get_logs` | 按 `job_id` 读取日志尾部，`lines` 可选。先核对当前会话的数据访问范围 |
| `list_bindings` / `get_settings` / `list_projects` / `list_labels` / `doctor` | 查询绑定、设置、项目、标签或执行器就绪状态 |
| `get_app_info` / `get_model_catalog` | 查询宿主版本、平台或指定 `executor` 的模型与推理档位 |
| `get_control_server_registration` / `check_named_linear_server` | 按 `executor` 查询 IssueRelay 控制服务或精简运行所需 Linear 服务的注册状态 |
| `get_autostart_status` / `get_cli_path_status` | 读取自启和 CLI 是否可用，不返回 PATH 或安装目录 |
| `get_update_status` / `check_update` | 读取上次检查或主动检查更新。安装和重启由桌面处理 |
| `reconnect_linear` | 经原生表单确认，用已保存的授权尝试重连，只返回状态 |
| `get_control_capabilities` | 查看支持的动作与客户端是否声明了确认表单 |
| `test_confirmation_form` | 打开无副作用的测试表单，确认、拒绝和取消都不会修改数据或联系宿主 |
| `preview_task_action` / `confirm_task_action` | 传类型固定的 `action`，预览后在客户端原生表单确认。确认时可传 `expected_preview` 校验预览指纹 |
| `prepare_chat_confirmation` | 本机 Codex 聊天确认已启用时，传同一 `action` 和已展示预览的指纹 `expected_preview`，取得一次性确认码与时限 |

用 Linear 需求编号调用 `get_run`、`get_issue_context` 或 `set_issue_context` 时，IssueRelay 会通过当前已连接的 Linear 解析需求的内部身份。不同工作区的同名编号按各自的需求处理。已知本地运行 ID 时，`get_run` 只传 `id` 即可完全在本机读取。

首次登记用 `expected_revision: 0`。修改前用 `get_issue_context` 取当前 `revision`，再把它传给 `expected_revision`。修订号冲突时，先重读并核对修改。`request` 写目标和脱敏摘要。`conversation_history` 写本次任务相关的完整会话内容，保留有关的命令、报错原文、版本、已作决定和未决事项。旧任务上下文缺少这段文字时仍可查看，重新派发或续作前须由主控补齐并重新预览确认。`scope` 写允许的范围。`acceptance` 和 `source_refs` 分别记录验收条件和来源引用。`preferred_executor` 只在用户明确选择时设置，入口由 MCP 客户端识别。草稿不随宿主重启保留，添加标签只让需求成为候选。

派发、续作、记录本地完成、独立验证、退回修复、标记完成、完成方案、重试和停止先调用 `preview_task_action`。随后通过 `confirm_task_action` 的原生表单确认，或者使用已启用的[本机 Codex 聊天确认](#本机-codex-聊天确认)。原生表单不可用时也可以在桌面面板完成。可用工具和系统授权交接见[聊天控制覆盖表](chat-controls.md)。请提交任务摘要、完整任务相关历史和引用，排除整段聊天、原始截图和凭据。`conversation_history` 上限为 96 KiB，序列化后的完整上下文上限为 128 KiB，超限会报错。请在主控会话调整草稿再提交，IssueRelay 不会自动删减或截断历史。确认前草稿只留在宿主内存。无论选择 Codex 还是 Claude Code，获准的任务快照都会随运行传给所选执行器。两端主控会话也可通过 `get_run` 或 `get_handoff` 读取同一运行的已保存上下文与检查点。工具返回的需求标题、摘要、检查点与错误都按数据读取。

### 在会话中执行操作

例如要继续第 42 次运行，调用 `preview_task_action` 传入 `{"action":{"action":"continue","job_id":42,"executor":"codex"}}`。先在会话里让用户核对完整预览，再调用 `confirm_task_action`，沿用同一 `action`，把刚收到的 `preview_fingerprint` 作为 `expected_preview`。简报较长时，表单保留目标、执行去向、简报修订号和指纹，并指向会话里的完整预览。只有指纹匹配才能使用这种表单，传给执行者的简报仍是完整版本。执行去向或配置差异本身过长时，转桌面审核。表单取消、过期、客户端不支持或预览已变化时，宿主保留原状态。

可以先用 `test_confirmation_form` 验证客户端能否显示原生表单。该工具不联系 IssueRelay 宿主。客户端声明支持表单并不保证当前会话允许弹出表单。`client_declined` 表示客户端回传了拒绝，可能是审批策略直接拒绝，也可能是用户拒绝。`confirmation_not_checked` 表示表单已提交，但没有勾选确认。Codex 会话应使用允许 MCP 交互表单的审批策略，并保留用户审核。

- `dispatch` 用 `issue_identifier`。`continue` 用 `job_id` 和 `executor`。`retry`、`stop`、`record_local_completion`、`mark_done`、`complete_plan` 用 `job_id`。重试后仍须复核派发，停止后用 `get_run` 查询终态。
- `start_verification` 用开发运行的 `job_id`、`executor` 和 `delivery`。交付标识用干净工作树的完整 HEAD SHA，或指向交付清单的本机绝对 JSON 路径。
- `return_for_repair` 用独立验证的 `job_id` 和 `reason`。仅对已确认的新版本验收结果为未通过的运行提供退回。
- `save_binding` 用 `binding`，`remove_binding` 用 `binding_id`。需要扩大完全访问权限或覆盖另一条绑定时，由桌面处理。
- `list_labels` 和桌面绑定页读取工作区标签和可见项目关联团队的标签，并合并同名标签。
- `set_settings` 用 `changes`，可修改 `poll_interval_secs`、`max_concurrent`、`run_timeout_mins`、`stall_timeout_mins`、`language`。

精简后台执行器仍按目标仓库的规则执行。用户可以在已注册 IssueRelay MCP 的主控会话里处理调度和验收。任务上下文与回执由宿主保存，本机的会话材料和日志不放进 Git。

### 本机 Codex 聊天确认

支持 `UserPromptSubmit` 提交钩子的本机 Codex 可以在聊天中直接确认任务操作，包括「完全访问」模式。此入口默认关闭。先注册 IssueRelay MCP，再按下面的步骤启用。 [Codex 提交钩子](https://learn.chatgpt.com/docs/hooks#userpromptsubmit)

1. 正常退出 IssueRelay。打开 IssueRelay 自己的 `config.toml`，macOS 位于 `~/Library/Application Support/com.sunkailun.issuerelay/`，Windows 位于 `%APPDATA%\com.sunkailun.issuerelay\`。在现有的 `[settings]` 下加入 `codex_chat_confirmation = true`，保存后重新打开 IssueRelay。
2. 在仓库目录运行下方的安装脚本。把占位值替换为安装包里 `issuerelay` 或 `issuerelay.exe` 的绝对路径，命令行程序与 IssueRelay 应用的主可执行文件同目录。脚本把一个固定命令加入 Codex 的 `hooks.json`，保留已有钩子。Windows 可用 `py -3` 代替 `python3`，脚本需要 Python 3.8 或更新版本。安装或移除时，先保存其他正在编辑的钩子配置。

   ```sh
   python3 scripts/install-codex-chat-confirmation-hook.py "<ISSUERELAY_CLI>"
   ```

3. 在 Codex 桌面设置的「Hooks」中审核并信任这条 `UserPromptSubmit` 钩子，也可在 Codex CLI 运行 `/hooks`。核对钩子的固定命令确实指向已安装的 IssueRelay。需要时重新打开 Codex 以加载新配置。

启用后，让 Codex 先用 `preview_task_action` 展示完整预览，再用 `prepare_chat_confirmation` 准备同一操作，把 `preview_fingerprint` 传为 `expected_preview`。核对目标、执行去向和影响后，在**同一个聊天**里原样发送返回的 `confirmation_text`。它的格式是 `确认 <32 位小写十六进制码>` 或 `confirm <32 位小写十六进制码>`。无需再点表单。钩子将宿主结果带回聊天。确认只对本次预览有效，过期、重新生成或预览变化后应重新核对。关闭开关或未安装可信钩子时，仍可使用原生表单或面板。

此入口适用于派发、续作、重试、停止、记录本地完成、标记完成和完成方案。绑定与设置变更、Linear 重连仍通过原生表单。它依赖可信的 Codex 客户端和本机账户，和同一账户下有完全访问权限的进程共享系统权限边界。停用时，在 IssueRelay 的 `[settings]` 将 `codex_chat_confirmation` 设为 `false` 并重启，可用安装脚本的 `--remove` 参数移除与原 CLI 路径匹配的钩子项。

## 查看需求与历次运行

「运行记录」默认每个需求一行，显示最新一次运行的状态、执行者和说明。点击次数展开该需求的历次运行，按运行编号查看时间、方案或开发用途、接续来源与结果，并可进入任一轮详情。状态筛选依据最新一轮；旧失败保留在展开历史中。面板读取最近 200 次运行，达到上限时提示可能有更早的记录未显示。

## 继续中断的运行

1. 打开最近一次已开始且因失败、取消，或实施时请求补充信息而停下的运行，点「继续上次」，选择 Codex 或 Claude Code。若有手动交接保留，请先释放。续作使用原项目工作目录。如果项目绑定改到了别处，需要先恢复原绑定。
2. 实施运行处于「待补充信息」时，先查看详情列出的所需信息，在继续窗口填写新增文字并点「保存补充并预览」。新增内容追加到原任务历史，保留原范围、验收条件和引用；保存本身不启动任务。缺少具体问题时，回到该需求的 Linear 评论与运行日志核对后再补充。核对预览中的原运行、接续方式、目录、执行器、模型、推理力度、权限、当前完整任务上下文和已记录的续作证据。上下文包括已提交的任务摘要、任务相关会话历史和 Wiki 引用。证据取自已记录的运行结果、检查点和 Git 快照，整段聊天、原始日志和截图不随续作发送。
3. 确认后，IssueRelay 保存一次新的尝试，或接管已经为该次续作排队且尚未开始的尝试。原运行保留。需求须在当前 Linear 工作区中仍指派给你并带原模式的标签。对请求补充信息的实施运行，补充后可以继续，即使执行器已移除原 `ai-ready`；切换成方案仍会被拒绝。实现任务可以保持 Todo 或 In Progress，方案任务仍使用 Todo。IssueRelay 在这一流程中不更新 Linear。
4. 同一执行器符合接续设置、原生会话 ID、原目录、七天期限和运行模式等条件时，预览显示恢复原会话，确认后用官方 CLI 接续。已确认恢复若在启动时失败，会显示失败原因并停下。切换执行器时，预览显示新会话交接，所选执行器在同一工作目录接收已确认的任务上下文和交接证据。同一执行器不符合原生恢复条件时，也可在预览明确显示新会话交接后再确认。

## 查看和管理原生会话

执行器发出原生会话启动事件后，运行详情即可显示会话定位，`get_run` 同时可读到 `session_id`。Codex 可复制 `codex://threads/<session-id>`，在桌面客户端打开对应会话。Claude Code 可复制 `claude --desktop --resume <session-id>`，等本次运行结束后使用。方案和开发运行结束后仍可在原工作目录通过 `codex resume <session-id>` 或 `claude --resume <session-id>` 打开。Codex 的有限近期文本预览通过官方 App Server 的只读查询加载。会话是否出现在项目侧栏由原生客户端决定。 [Codex 桌面深链](https://learn.chatgpt.com/docs/reference/commands#deep-links) · [Codex App Server](https://learn.chatgpt.com/docs/app-server#read-a-stored-thread-without-resuming) · [Claude Desktop 打开 CLI 会话](https://code.claude.com/docs/en/desktop#coming-from-the-cli)

原生会话由各自的官方客户端管理。Codex 的归档会保留会话记录，并尝试归档由它生成的后代会话。Claude Code 后台列表中的会话可在官方客户端处理，`claude rm <id>` 处理后台列表记录和符合条件的 Claude 工作树，原生会话记录仍可通过 `claude --resume` 访问。IssueRelay 的日志保留设置只管理自身的运行日志。原生会话存储不由 IssueRelay 直接操作。 [Codex 归档规则](https://learn.chatgpt.com/docs/app-server#archive-a-thread) · [Claude Code 后台会话移除规则](https://code.claude.com/docs/en/agent-view#what-deleting-a-session-removes)

独立验证使用运行自己的临时目录，这个目录会在执行结束后清理。通过 IssueRelay 保留的会话 ID 打开同一原生会话，可查看当时的任务、命令和结果。独立验证不恢复开发会话。

Claude Desktop 的 Code 会话也可以在侧栏菜单归档。归档前先保存需要保留的工作树改动。根据会话状态及操作选项，归档可能丢弃未提交改动或清理未跟踪文件。 [Claude Desktop 工作树归档规则](https://code.claude.com/docs/en/desktop#keep-uncommitted-changes-when-a-session-is-archived)

## 独立验证、退回与复验

1. 方案与开发正常派发。开发交付后可人工验收，或在开发运行详情点「开始独立验证」并选择 Codex 或 Claude Code。核对来源运行、模型、推理力度、权限、范围和交付版本，再确认。确认返回新的运行编号，收到执行者启动事件后详情才会显示真实会话 ID。一次只允许一个验收运行。
2. 验收在临时交付副本中读取任务和检查源码、运行允许的测试。原交付目录只读，临时副本可运行构建和测试，固定沙箱允许联网以访问本任务授权的原型、服务和依赖。预览显示这一权限，网络范围由获批任务约束。清单可包含一个或多个绑定目录内的仓库，每项记录 `repository`、完整基准提交 `base` 和 `files`，后者记录相对 `path` 及内容的 `sha256`。修改项目、切换版本或改变任务版本后，需要重新核对。副本只含 Git 跟踪文件和清单中批准的改动。依赖本机服务、外部原型或生成文件时，如验收端缺少可信输入或运行权限，应记录为「无法完成」。
3. 详情记录「通过」「未通过」或「无法完成」及验收证据。进程退出、取消、额度耗尽、版本改变或缺少对应版本的结构化报告，都不会得出通过或未通过的业务结论。无法完成且交付版本未变时，可以在开发运行另开验证。
4. 针对「未通过」预览「退回开发」，填写原因并确认。更新当前 Linear 任务的要求，沿用开发派发复核。退回记录保留所验版本、验收证据与原生会话；新的开发运行另开会话，并收到该轮失败证据。修复交付准备好后，以修复运行及新版本重新执行独立验收。
5. 通过后再按本项目要求归档并读回回执。配置 Wiki 的任务需有本轮开发与通过的验证回执，才可标记完成。已实际执行的方案、开发、验证、未通过和无法完成的运行分别保留归档入口。未启动的排队和等待项无归档回执。

## 手动接力、独立复核与归档

运行详情会保存执行者报告的最新检查点，以及可取得的分支、提交和改动文件等本轮快照。只有执行者返回可识别的结构化额度事件时，IssueRelay 才把中断标为「额度已用尽」并阻止自动重跑。重置时间以执行者报告为准。普通错误文字仍显示为一般失败，可以查看日志和已保存的交接资料。恢复时可以继续等待，或通过「继续上次」选择恢复原会话或新会话交接。仍可使用下面的手动交接。

1. **接手续作。** 在最近一次已结束、尚未完成的运行详情，选择执行者并点「领取交接」，或调用 `claim_handoff`。IssueRelay 会保留该本地目录，避免调度器再向它派工。点「复制交接提示词」，自己打开所选执行者的新会话，在同一目录核对工作树再继续。会话停止写入后，点「释放交接」或调用 `release_handoff`。用 MCP 操作时，请保存自己领取时返回的 `claim_id`，释放时传这个值。这样即使请求迟到，也不会清除后来重新领取的保留。
2. **独立复核。** 在运行详情点「复制独立复核提示词」，交给一个新会话。复核者依据需求与验收条件核对 diff、执行者报告的验证、剩余风险，并按授权做验证。最终是否验收通过由你判断。
3. **Wiki 归档。** 在「设置 → 高级 → 归档」选择已有的 LLM-Wiki 目录。留空时，完成运行无需归档回执。配置后，从运行详情复制 Wiki 归档材料，由有权限的主控会话按目标 Wiki 的规则写入、核对页面，并按要求提交和推送。然后调用 `record_archive`，填写运行 `job_id`、相对当前 Wiki 根的页面路径（使用 `/`）和 Git Wiki 要求的完整提交 SHA。IssueRelay 核对路径、提交中的页面；有 Git 远端时还核对已取得的远端跟踪历史。远端推送结果由主控会话在目标仓库实际读回，登记后再用同一运行编号查询回执。回执绑定登记时的 Wiki 根。换根或页面消失后，需要在当前目录重新登记。领取交接会清除旧回执，释放交接后才可重新登记。通过验收并取得有效回执后，可在面板或支持确认的会话中「标记完成」。

## 常见误解

- **关窗口就停了吗？** 关闭面板后，托盘程序仍继续监控。暂停监控、停止某次运行与退出程序是不同操作。
- **显示交付就代表通过验收吗？** 运行结果用于追踪执行情况；是否满足需求仍由你检查，不能只凭退出码或 AI 的文字报告决定。
- **换个主控入口会影响哪个执行者接单吗？** 登记入口只在需求没有显式选择、绑定也未指定执行者时参与选择。后台权限、工具和额度依各自配置与账号而定。

## 技能、额度与使用边界

默认精简运行主要携带 Linear MCP 与目标仓库的规则，个人会话中安装的技能、插件、hooks 和其他 MCP 服务不会因此自动进入执行者。安装了 Superpowers 不等于后台运行已经加载它。需要某种工作流时，先确认执行者实际收到的输入、项目规则和运行配置，再判断是否支持；不要只为加载技能盲目关闭精简运行。

执行权限、项目规则与工具配置共同影响结果。默认「完全放开」权限允许执行者无人值守执行命令；精简运行减少加载的个人配置，不构成安全沙箱。固定提示词要求不设 Done、不推主分支、不发正式环境，但仍要结合项目的分支保护与验收要求使用。

IssueRelay 在本机调度并保存配置、运行记录和日志，没有遥测。这不代表数据永不离机：Linear 需要接收需求与评论，AI 服务会收到执行者提交的必要输入；工具调用也可能访问外部服务。是否适合某个仓库，应按它允许使用的账号、工具与数据边界判断。
