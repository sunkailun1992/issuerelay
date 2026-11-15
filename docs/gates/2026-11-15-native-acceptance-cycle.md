# 原生 Codex 失败、修复与复验闭环验收

## 实现范围

0.6.1 修复独立验证确认中多次暂存简报的修订号丢失：确认精确保留已审核的内容与修订号，继续保留事务回滚、CAS、真实漂移与重复操作拒绝，也不授予普通开发派发批准。四项引擎回归覆盖持久化第 7 版后多次暂存至第 10 版，以及提交失败后草稿保留。

新增 `crates/core/tests/acceptance_cycle_live.rs`，默认忽略。它通过生产 Host、Engine、SQLite、IPC 和 ProcessJobRunner，执行四次真实 Codex 调用：初始开发创建故意不通过的合成交付；独立验证报告失败；退回确认并更新合成 Linear 修订后，新开发会话修复交付；新的独立验证会话检查新指纹并报告通过。退回、暂存简报和确认只是本地工作流动作，不计作额外原生会话。

全部任务与状态来自 `DEMO` 夹具，没有真实 Linear。测试专用封装为两个开发进程隔离用户集成并配置只读 stdio Linear MCP，以满足开发提示词的读需求步骤；保留生产 stdin、权限参数、真实进程调用和输出 hooks。它不等于未经改动的生产 CLI 配置，也不覆盖真实 Linear 认证或写操作。验证进程沿用生产独立验证启动配置。Codex 固定使用 `gpt-6.1-sol`、`ultra`，需要本机已有登录，不复制凭据。

测试的 operator 通过隔离宿主 IPC 提交预览，是受控合成批准，不替代真实客户端原生表单验收。MCP 确认分支由 `crates/cli/tests/mcp_tools.rs` 的合成场景另行验证。客户端 `client_declined` 不会提交任务；现成桌面或显式启用的聊天确认仍需要用户重新核对和确认。

## 运行方法

先完成正式面板构建，确认有现成 Python 3 与 Codex，随后由维护者明确启动四次有实际推理用量的会话。证据目录必须在公开仓库外：

```sh
ISSUERELAY_NATIVE_CYCLE=1 ISSUERELAY_ACCEPTANCE_EVIDENCE=<PRIVATE_EVIDENCE_DIR> \
cargo test -p issuerelay-core --test acceptance_cycle_live \
  codex_failed_verification_repair_and_reverification_native_cycle -- --ignored --exact --nocapture
```

可先用 `cargo test -p issuerelay-core --test acceptance_cycle_live --no-run` 只检查编译。未设置双重开关时，不应执行真实测试。CLI 参数先依据本机 `codex-cli 0.159.2` 的实际 `--help` 与 `exec --help` 核对；准备阶段没有启动推理或修改全局配置。

## 证据要求与当前边界

每轮通过 IPC 读回真实 job、运行中的合法 session ID、终态和原生只读会话。四个 session 必须不同，两个开发轮在原目录、新修复不恢复旧会话，两个 verifier 在不同 scratch。检查活动 writer 排它、重复 token 拒绝、退回前及未更新 Linear 修订时开发拒绝、失败关联继承、旧交付拒绝复验和新交付关联。

固定验收脚本在初始开发、修复和复验后逐字核对，两次开发提交只能改变 `delivery.txt`。真实 verifier 的原始命令事件必须包含该脚本、实际失败／通过退出码和对应输出；不能只凭模型的最终 JSON。两个开发阶段各自要有新的只读合成 Linear 调用。原始 CLI 参数、提示词、日志、实际 session、IPC 与原生会话读回只保存在仓库外，公开报告只登记获准的脱敏概要。

## macOS 实际执行结果

2026-11-15 在 Apple silicon 本机、隔离宿主与合成 Linear 上完成真实四会话测试，`codex-cli 0.159.2`，四轮实际原生记录均为 `gpt-6.1-sol / ultra`。完整重跑退出 0，单项测试通过；引擎、IPC 运行详情和原生只读会话均已读回。

| 阶段 | 真实结果 |
|---|---|
| 初始开发 | Codex 只提交故意不满足后续验收标准的合成文件，原工作树干净；`local_complete` |
| 独立验证 | 新 scratch 与新原生会话执行固定检查，实际退出 1；`verification_failed` |
| 退回后的修复开发 | 新运行、新原生会话，完整继承失败证据，仅提交目标文件修复；`local_complete` |
| 新交付独立复验 | 交付指纹改变，再用新的 scratch 与原生会话执行同一检查，实际退出 0；`verification_passed` |

四个原生 session 不同，修复没有 resume，两个开发轮各自读取只读合成 Linear；来源运行、失败运行、修复关联和最终验证关联一致。旧交付复验、重复 token、退回前以及 Linear 修订未前进时的开发派发均拒绝。固定检查脚本逐字不变，验证没有改原交付，隔离宿主统一停止并清理其进程。

首次真实运行虽完成四个进程，但最终测试用原文子串比较 JSON 编码后的失败证据，对反斜杠产生假阴性，因此保留为失败。实际提示词 JSON 中证据完整；仅修改测试为结构化解析后逐项比较整数运行 ID、完整交付、失败状态与全部有序证据，再补反斜杠、引号、真实换行和错误关联等合成拒绝回归，随后从新的证据目录完整重跑通过。没有改生产提示词或重写旧失败记录。

本机最终门禁：Rust 1,199 passed / 0 failed / 3 默认 ignored；前端 1,320 项通过，正式面板构建、lint、fmt、workspace/all-targets Clippy、严格秘密扫描与 diff 检查均通过。三个默认忽略项包含单轮 Codex、单轮 Claude 与此四轮测试，四轮实跑是另行显式执行。MCP 新增 5 项参数化测试，覆盖 Codex/Claude 启动验证及退回动作的 24 个合成确认分支。

## 尚未覆盖的实机边界

真实客户端原生确认表单、真实 Linear、正式业务宿主、Claude 多轮执行与 Windows 原生实机未由此测试验证。两个开发轮保留绑定的 Full 权限；夹具隔离任务数据、配置和宿主目录，不证明操作系统层对其他账户、文件及网络全面隔离。真实提供方记录、命令、原生会话和失败尝试留在仓库外，公开结论只包含脱敏概要。
