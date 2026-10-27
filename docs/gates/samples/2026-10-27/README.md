# 2026-10-27 实测样本

运行改进设计（`docs/superpowers/specs/2026-10-27-issuerelay-run-improvements-design.md`）第 4 节的实测记录，结论写在那一节。

- 环境：macOS 27.0、Claude Code 2.1.285、codex-cli 0.159.2、Linear 官方 MCP。
- 做法：在 `~/issuerelay-sandbox/probe-1027`（临时 git 仓库）里，用托盘程序那样的干净环境（`env -i`，只带登录 shell 的 PATH 与系统代理）和 IssueRelay 会拼出的精简运行参数直接运行执行者，提示词经标准输入传入。Linear 写操作只落在沙箱项目的一条需求上（几条以 `🤖 IssueRelay probe` 开头的评论）。
- 公共参数：
  - Claude：`claude -p --output-format stream-json --verbose --permission-prompts none --permission-mode bypassPermissions --model claude-haiku-5-5`，加精简运行的 `--strict-mcp-config --mcp-config '{"mcpServers":{"linear-server":{"type":"http","url":"https://mcp.linear.app/mcp"}}}' --setting-sources project,local --settings '{"claudeMdExcludes":["~/.claude/CLAUDE.md"]}'`，环境变量 `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`。标准错误里的 `[claude-code:unrecognized_model]` 一行是这个模型名引起的提示，不影响结果。
  - Codex：精简运行的 `--ignore-user-config --disable hooks --disable plugins --disable apps --disable memories --disable skill_mcp_dependency_install -c mcp_servers.linear.url=https://mcp.linear.app/mcp -c mcp_servers.linear.required=true -c mcp_servers.linear.startup_timeout_sec=30 -c skills.include_instructions=false`，加 `-c model_reasoning_effort=low`，最后的 `-` 从标准输入读提示词。
- 去掉的内容：家目录写成 `~`；Linear 的需求编号改成 `DEMO-N`，人名改成 `Jane Doe`，挡住需求的那条的标题换成占位；会话、线程、评论、需求与用户的编号换成 `00000000-0000-4000-8000-…` 的占位，同一个编号在各个文件里换成同一个占位，所以接续前后的会话编号仍然相同（`…-000000000000` 是故意给的不存在的会话）；Anthropic 的请求与消息编号写成 `req_demo`、`msg_demo`；去掉了 Claude 的 `rate_limit_event`（账号的用量，与结论无关）。其余照原样，包括模型的回答：Codex 用中文回答，是因为精简运行照样读用户级的 `~/.codex/AGENTS.md`。

| 文件 | 在公共参数之外 | 退出码 | 看什么 |
|---|---|---|---|
| `claude/resume-1-fresh.jsonl` | 新会话，让它记住一个词 | 0 | `system/init` 的 `mcp_servers` 只有 `linear-server`；`result` 的 `session_id`、`total_cost_usd`（0.0551374）、`modelUsage` 的 `costBasis` 是 `unknown` |
| `claude/resume-2-resumed.jsonl` | `--resume <上一个文件的 session_id>` | 0 | 照常有 `system/init`，会话编号不变，记得那个词；`total_cost_usd`（0.0943946）与 `modelUsage` 是整个会话累计的，顶层 `usage` 只是这次 |
| `claude/resume-missing.jsonl`、`claude/resume-missing.stderr.txt` | `--resume 00000000-0000-4000-8000-000000000000` | 1 | 没有 `system/init`，只有一条 `result`（`error_during_execution`） |
| `claude/plan.jsonl` | `--tools Read,Grep,Glob` | 0 | `system/init` 的工具只剩 `Glob`、`Grep`、`Read` 与 Linear 的工具；读了文件、在需求下留了评论，写文件与执行命令都没有工具 |
| `claude/plan-then-implement.jsonl` | 不带 `--tools`，`--resume <plan.jsonl 的会话>` | 0 | 工具恢复完整，照指示写了文件 |
| `claude/plan-disallowed-tools.jsonl` | `--tools Read,Grep,Glob --disallowedTools mcp__linear-server__merge_diff,mcp__linear-server__submit_diff_review,mcp__linear-server__update_diff,mcp__linear-server__delete_comment`（全分支审查之后补测） | 0 | `bypassPermissions` 下拒绝规则照样生效：这四个工具不在 `system/init` 的工具列表里（Linear 的工具从 69 个变成 65 个），`save_comment`、`save_issue` 还在；模型也说没有 `merge_diff` |
| `codex/resume-1-fresh.jsonl` | `codex exec --json --dangerously-bypass-approvals-and-sandbox … -`，让它记住一个词 | 0 | `thread.started` 的 `thread_id`；`turn.completed` 的 `usage` 只有 token |
| `codex/resume-2-resumed.jsonl` | `codex exec resume --json --dangerously-bypass-approvals-and-sandbox … <上一个文件的 thread_id> -` | 0 | 同一个 `thread_id`，记得那个词；`input_tokens` 含整段历史 |
| `codex/resume-missing.stderr.txt` | `codex exec resume … 00000000-0000-4000-8000-000000000000 -` | 1 | 标准输出什么都没有，标准错误如文件 |
| `codex/plan-read-only.jsonl` | `codex exec --json -s read-only … -` | 0 | 写文件被沙箱拒绝，`echo` 照样能跑；Linear 的评论因为要审批而失败 |
| `codex/plan-read-only-approve.jsonl` | 再加 `-c mcp_servers.linear.default_tools_approval_mode=approve` | 0 | 写文件仍被拒绝，评论成功 |
| `codex/plan-resumed-read-only.jsonl` | `codex exec resume --json … -c sandbox_mode=read-only -c mcp_servers.linear.default_tools_approval_mode=approve <plan-read-only.jsonl 的 thread_id> -` | 0 | 同一个线程，写文件被拒绝，评论成功 |
| `codex/plan-then-implement.jsonl` | `codex exec resume --json --dangerously-bypass-approvals-and-sandbox … <plan-read-only-approve.jsonl 的 thread_id> -` | 0 | 写了文件 |
| `codex/plan-disabled-tools.jsonl` | `codex exec --json -s read-only …` 加 `-c mcp_servers.linear.default_tools_approval_mode=approve -c 'mcp_servers.linear.disabled_tools=["merge_diff","submit_diff_review","update_diff","delete_comment"]'`（全分支审查之后补测，2026-10-28 00:0x） | 0 | Codex 列出的 Linear 工具是 65 个，关掉的 4 个都不在，模型说没有 `merge_diff` |
| `linear/get_issue-fields.json`、`linear/get_issue-blocked-by.json` | Linear MCP 的 `get_issue`，`fields` 见文件里的 `request` | — | `statusType` 与 `relations` 的形状；`blockedBy` 的每一项只有编号与标题 |
