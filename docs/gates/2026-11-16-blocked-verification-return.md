# Blocked 验证退回诊断或修复：本地源码通过，实机待验

## 行为与边界

已有报告的最新终态独立验证，结论为 Failed 或 Blocked 时，都可沿原 `return_for_repair` 预览与人工确认退回开发。Blocked 使用「退回诊断或修复」说明，保留无法完成的原结论、报告和证据，不改记业务失败或通过。动作参数仍为 verifier 的运行编号和退回原因，没有新增命令、权限或绕过确认的接口。

引擎保留 latest source、latest verifier、无活动验证、无手动接手、未曾退回及提交时完整复核。Passed、活动运行、recommendation 缺失均拒绝。正常停止而没有报告的 fallback Blocked，不在面板出现退回入口。已被新 verifier 替代的历史验证也隐藏入口。

退回只记录本地工作流决定、清除旧派发批准并触发检查，不写 Linear 中间状态，也不直接启动开发。符合条件的新 Linear 修订仍须重新审核派发；新 repair 建立关联并使用新的开发会话。诊断或修复完成后，旧交付指纹不能用于复验，需要新交付及新的独立验证会话和临时副本。

五种现有语言均有 Blocked 对应的按钮、说明、原因、退回回执和后续派发引导；Failed 原文案保留。确认有效期、一次性消费及轮询时保持审核对话框的既有修复保持不变。

## 合成覆盖

- [workflow.rs](../../crates/core/tests/workflow.rs) 使用实际 Engine、内存 FakeLinear 与真实进程 runner 执行仓库 fake-agent。Failed 和 Blocked 分别走开发、验证、诊断或修复、补充续作、新交付复验；每例五次合成子进程调用，不是真实 Codex 或 Claude 调用。
- 对比 verifier 整个 Job、recommendation 原始序列化字节、状态与证据，确认退回及后续关联不改原结论。覆盖 queued/running 拒绝、claim 后旧预览提交拒绝、重复确认/重复退回拒绝、Passed 拒绝、Linear revision 准入、新 repair 关联、原绑定目录及不续旧验证会话。
- 新开发已为 LocalComplete 而交付指纹仍未变化时，明确核到 `the repair must deliver a new version`，随后新交付复验通过，并核新 scratch 和不 resume。
- [acceptance_brief.rs](../../crates/core/tests/acceptance_brief.rs) 经正常 Stop 确认路径产生无报告、无 recommendation 的 canceled verifier，确认不能退回，没有直接写数据库制造批准或终态。
- [AcceptanceCard.test.tsx](../../app/src/components/AcceptanceCard.test.tsx) 和 [mock.test.ts](../../app/src/lib/mock.test.ts) 覆盖五语正常预览及显式确认、Blocked 分类保留、不自动派活、无报告及活动状态隐藏、旧 verifier 隐藏。既有 Failed、过期、失效 token、提交中状态及派发对话框轮询回归一起执行。

## 本机验证

| 检查 | 结果 |
|---|---|
| `pnpm --dir app build` | 退出 0，先于最终 Rust 门禁 |
| `cargo fmt --all --check` | 退出 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出 0 |
| `cargo test --workspace` | 1222 通过、0 失败、3 个 opt-in provider live 测试按默认忽略 |
| `pnpm --dir app lint` | 退出 0 |
| `pnpm --dir app test` | 58 文件、1351 测试通过 |
| `bash scripts/check-secrets.sh --strict` | 退出 0 |
| `pnpm --dir app test:screens --workers=2` | 212 通过、8 个 README 图生成项按默认跳过；未更新基准图 |

Blocked 完整返修链先在原实现的正常退回预览处失败，最小修复后通过。五语入口和 mock 准入先红后绿；旧 verifier 的面板负例也先红后绿。全部夹具均为 DEMO 合成数据。

## 尚未验证

本轮未启动真实提供方会话、连接真实 Linear、运行 Windows CI 或做 Windows 实机验收。没有升版、推送、打包、安装或重启正式 App。当前安装的 0.6.1 未加载本轮源码变更，不能用本地测试结果声称正式 App 已支持该入口；实际新版运行仍待后续发布与实机验收。
