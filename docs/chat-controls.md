# IssueRelay 聊天控制覆盖表

桌面面板和聊天控制共用 IssueRelay 宿主。下表对照桌面命令，说明聊天能否完成同一件事。`已支持` 表示原有 MCP 能力，`本轮接入` 表示当前开发范围，是否可用以对应版本的工具清单和验收记录为准。`桌面` 与 `系统授权` 行保留各自的交互入口，包括由执行器管理的原生会话。

本轮的任务操作通过类型固定的预览和确认流程进入同一个引擎。`preview_task_action` 展示目标、影响和需核对的状态，`confirm_task_action` 打开客户端原生确认表单，由用户确认。客户端不支持表单时，回到桌面面板。审批凭据由宿主与控制服务保管，权限、归属、版本和运行状态仍由宿主校验。执行回执区分请求受理和实际开始、停止或完成。`get_control_capabilities` 显示可预览的操作和客户端声明的表单能力。接入范围以 [控制服务](../crates/cli/src/mcp.rs) 和 [宿主 IPC](../crates/core/src/ipc/server.rs) 为准。

`test_confirmation_form` 验证当前会话的表单链路，不联系宿主。长简报的确认表单可以引用会话里的完整预览，前提是 `expected_preview` 仍与当前动作匹配。审批后的宿主复核和一次性提交照常执行。

## 面板命令覆盖

新增桌面命令时，在表中添加一行，并交付对应的聊天操作或写清系统授权、纯展示的理由。命令名须与 [面板命令登记表](../app/src-tauri/build.rs) 一致。覆盖测试核对登记，并检查标为支持的行引用了已实现的 MCP 工具或固定动作。授权、工具行为和实机结果分别验收。

<!-- panel-commands:start -->
| 面板命令 | 聊天状态 | 对应操作或保留理由 |
|---|---|---|
| `app_info` | 本轮接入 | `get_app_info` 返回宿主版本和平台。系统语言和 CLI 路径由桌面展示。 [宿主响应](../crates/core/src/ipc/server.rs#L71) |
| `startup_state` | 桌面 | 初始化和启动错误页由应用展示。聊天在宿主未就绪时提示用户打开或检查应用。 |
| `status` | 已支持 | `status` 查看监控、连接和队列。 |
| `list_jobs` | 已支持 | `list_jobs` 查看有限数量的近期运行。 |
| `get_handoff` | 已支持 | `get_handoff` 读取运行交接、复核和归档材料。 |
| `claim_handoff` | 已支持 | `claim_handoff` 按运行和执行者保留目录。 |
| `release_handoff` | 已支持 | `release_handoff` 使用该次保留的 `claim_id` 释放目录。 |
| `model_catalog` | 本轮接入 | `get_model_catalog` 按执行者读取可选模型、推理档位和目录来源。 [模型目录](../crates/core/src/ipc/server.rs#L186) |
| `job` | 已支持 | `get_run` 按运行 ID 或需求编号读取运行。 |
| `job_log` | 本轮接入 | `get_logs` 按运行读取受限日志尾部。CLI 也有 `issuerelay logs`。 |
| `read_codex_session` | 系统授权 | 由执行器官方客户端管理原生会话。桌面只能通过 Codex 官方接口预览已结束运行关联的有限文本。两端聊天使用 `get_handoff` 获取任务范围内的交接材料，原生会话正文保留在提供方边界。 [桌面预览入口](../crates/core/src/host.rs#L506) |
| `issues` | 本轮接入 | `list_issues` 读取 IssueRelay 发现的候选需求与阻塞原因。 |
| `get_dispatch_preview` | 本轮接入 | `preview_task_action` 的 `dispatch` 操作预览单条需求的派发目标、上下文与去向。 |
| `approve_dispatch` | 本轮接入 | `confirm_task_action` 的 `dispatch` 操作提交有效派发预览。 |
| `get_continuation_preview` | 本轮接入 | `preview_task_action` 的 `continue` 操作按原运行和选定执行者预览续作。 |
| `get_continuation_context` | 本轮接入 | `get_continuation_context` 按原运行读取续作所需上下文和修订号。 |
| `supplement_continuation_context` | 本轮接入 | `supplement_continuation_context` 追加指定中断运行的待确认材料，沿用修订号校验。 |
| `approve_continuation` | 本轮接入 | `confirm_task_action` 的 `continue` 操作确认原会话恢复或交接方式。 |
| `get_development_issue_url` | 本轮接入 | `get_development_issue_url` 提供已结束方案对应的 Linear 需求链接，由用户打开。 |
| `pause` | 已支持 | `pause` 暂停监控与新派发，当前运行继续。 |
| `resume` | 已支持 | `resume` 恢复监控，可触发检查。 |
| `poll_now` | 已支持 | `poll_now` 请求立即检查，沿用暂停规则。 |
| `cancel_job` | 本轮接入 | `stop` 操作预览并确认停止单个运行，回执区分停止请求和进程树实际退出。 |
| `retry_job` | 本轮接入 | `retry` 操作确认重新进入待派发复核，后续派发另行确认。 |
| `record_local_completion` | 本轮接入 | `record_local_completion` 操作确认指定旧运行的本地证据，记录为待验收。 |
| `mark_done` | 本轮接入 | `mark_done` 操作确认后，按实时 Linear 条件和归档要求标记单条最新运行完成。 |
| `plan_progress` | 本轮接入 | `get_plan_progress` 读取各方案的子需求缓存进度。 |
| `refresh_plan_progress` | 本轮接入 | `refresh_plan_progress` 刷新一条方案的 Linear 子需求进度。 |
| `complete_plan` | 本轮接入 | `complete_plan` 操作预览子需求状态，确认后按实时条件收口一条方案。 |
| `bindings` | 本轮接入 | `list_bindings` 查询当前项目绑定，返回值省略自定义提示词。CLI 也有 `issuerelay bind list`。 |
| `save_binding` | 本轮接入 | `save_binding` 预览并确认一个项目绑定的变更，显示实际权限与精简模式。校验绝对目录、路由重叠及权限。新增或扩大完全访问权限仍需在桌面处理，原有自定义提示词保留。 [绑定校验](../crates/core/src/engine/control_actions.rs#L301) |
| `remove_binding` | 本轮接入 | `remove_binding` 按 `binding_id` 预览并确认删除一个绑定。配置自预览后变化时，宿主要求重新预览。 [变更与复核](../crates/core/src/engine/control_actions.rs#L242) |
| `projects` | 本轮接入 | `list_projects` 通过当前 Linear 连接查询绑定可用项目。 |
| `labels` | 本轮接入 | `list_labels` 查询当前 Linear 工作区和可见项目关联团队的路由标签。 |
| `settings` | 本轮接入 | `get_settings` 读取宿主配置，返回值省略提示词模板。凭据由系统钥匙串管理。 |
| `save_settings` | 本轮接入 | `set_settings` 预览并确认轮询间隔、总并发上限、运行超时、无输出超时和界面语言这五项。默认权限、完全访问确认、Wiki 根目录和提示词模板继续由桌面处理。 [字段白名单](../crates/core/src/workflow.rs#L207) |
| `doctor` | 本轮接入 | `doctor` 返回宿主与执行者就绪状态及诊断计数。环境变量和原始诊断文字留给桌面或 `issuerelay doctor`。 [聊天诊断输出](../crates/cli/src/mcp.rs#L760) |
| `linear_login` | 系统授权 | OAuth 在系统浏览器完成。聊天可提示登录并读回连接状态。 |
| `linear_cancel_login` | 系统授权 | 用户在桌面取消正在等待的授权流程。 |
| `linear_use_api_key` | 系统授权 | 用户在受保护界面录入，凭据由系统钥匙串保存。聊天仅显示连接状态。 |
| `linear_logout` | 系统授权 | 登出会清理系统钥匙串内的 Linear 凭据，由用户在桌面确认。 |
| `linear_reconnect` | 本轮接入 | `reconnect_linear` 由用户在客户端原生表单确认后尝试复用已保存连接。需要重新授权时使用桌面入口。 [重连确认](../crates/cli/src/mcp.rs) |
| `setup_action` | 系统授权 | 注册执行者的 MCP 或打开登录流程，由桌面及执行者官方客户端处理。 |
| `control_servers` | 本轮接入 | `get_control_server_registration` 按执行者查询 IssueRelay 控制服务的注册状态。返回 `registered`、`not_registered`、`not_installed` 或 `unknown`。 [注册检查](../crates/core/src/ipc/server.rs#L193) |
| `linear_server_named` | 本轮接入 | `check_named_linear_server` 按执行者查询精简运行使用的命名 Linear MCP 是否已注册。授权与连接就绪另查 `doctor`。 [命名服务检查](../crates/cli/src/mcp.rs#L902) |
| `open_linear` | 桌面 | 由桌面打开经过检查的 Linear 链接，聊天可呈现该链接供用户打开。 |
| `choose_directory` | 系统授权 | 目录由用户在系统选择器中选择。聊天可引用用户已提供且经过验证的目录。 |
| `autostart_enabled` | 本轮接入 | `get_autostart_status` 只读返回已启用、未启用或不可用。 [桌面诊断](../app/src-tauri/src/diagnostics.rs) |
| `set_autostart` | 系统授权 | 改写系统登录时启动设置，由用户在桌面确认。 |
| `quit` | 桌面 | 桌面负责等待或停止运行的退出流程。单次停止运行使用任务操作。 |
| `request_quit` | 桌面 | 托盘和启动错误页打开原生退出流程。 |
| `cli_on_path` | 本轮接入 | `get_cli_path_status` 只读返回命令入口状态，路径、链接目标和 shell 配置留在桌面。 [桌面诊断](../app/src-tauri/src/diagnostics.rs) |
| `install_cli_on_path` | 系统授权 | 改变用户 PATH 或命令链接，由桌面引导并确认。 |
| `remove_cli_from_path` | 系统授权 | 由桌面确认，移除 IssueRelay 自己创建的 PATH 项或链接。 |
| `update_status` | 本轮接入 | `get_update_status` 读取上次检查状态和可用版本。 [桌面诊断](../app/src-tauri/src/diagnostics.rs) |
| `check_update` | 本轮接入 | `check_update` 使用桌面更新器请求检查并返回状态和可用版本。安装由用户在桌面确认。 [桌面诊断](../app/src-tauri/src/diagnostics.rs) |
| `install_update` | 系统授权 | 由桌面完成下载、校验、退出与重启，保留原生安装流程。 |
<!-- panel-commands:end -->

需求分组、筛选、展开旧轮次和语言显示由各客户端处理。聊天读取运行、交接和计划进度后可以自行呈现；凭据、原始供应商存储和操作系统弹窗继续由原入口处理。
