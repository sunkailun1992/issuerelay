# 测试分层规范

## 定位

IssueRelay 驱动的是真实的 Linear 和真实的 AI。纯逻辑在单元测试里完整覆盖；与外部的交互用假实现做集成测试；真实环境只在沙箱上验收，并写成报告。「测试通过」和「实机验收通过」永远分开记录。

## 分层

| 层 | 工具 | 对象 | 何时跑 |
|---|---|---|---|
| 单元测试 | `#[test]` / `#[tokio::test]`，写在源文件末尾的 `mod tests` | 配置、触发、调度、提示词、命令拼装、结果识别、解析、格式化 | 每次提交 |
| 集成测试 | `crates/*/tests/`，加上 `tests/common/` 的假实现 | 运行器 + `fake-agent`、Linear MCP + 假服务端、引擎 + 假 Linear + 脚本化运行器、IPC | 每次提交 |
| 平台测试 | `#[cfg(windows)]` / `#[cfg(unix)]` 的测试，CI 两个平台都跑 | `.cmd` 包装、命名管道、残留的套接字文件 | 推 main 与开 PR 时 |
| 实机验收 | 开发宿主（M2 起是托盘程序）+ 真实 Linear + 真实 Claude / Codex | 完整流程、耗时、失败与超时 | 每个里程碑收尾，写 `docs/gates/` |

## 单元与集成测试规则

- 不访问网络，不碰真实钥匙串，不依赖本机装的 `claude` / `codex`。
- 与时间有关的上限用毫秒级（例如卡住判定 400 毫秒），不写长时间的 `sleep`。
- 临时文件一律用 `tempfile`，不写本机固定路径。
- 每个状态、每个错误分支至少一个测试；修 bug 先写一个能复现它的测试。
- 测试数据只用虚构值：`DEMO-1`、`Demo Project`、`Jane Doe`、`user-1`、`proj-1`。

## 假实现清单

| 假实现 | 位置 | 替代什么 |
|---|---|---|
| `fake-agent` | `crates/core/src/bin/fake_agent.rs` | Claude Code / Codex 进程（按脚本输出、退出码、启动孙进程，包括离开运行进程组的孙进程） |
| `FakeLinearServer`、`PartialServer` | `crates/core/tests/common/fake_server.rs` | Linear MCP 服务端（经内存管道连接）：按 `tests/fixtures/linear-tools.json` 校验每次调用的参数，只返回请求的 `fields`，可编程返回文本、工具错误或永不应答 |
| `FakeLinear` | `crates/core/tests/common/fake_linear.rs` | `LinearApi` 的内存版；`fail_next` 让下一次任意调用失败，`fail_calls` 让接下来若干次调用失败，`fail_issue_reads` 让某条需求的单条读取一直失败（直到 `clear_issue_failures`） |
| 内存钥匙串 | `crates/core/src/linear/auth.rs` 的测试模块 | 系统钥匙串（测试不碰真实钥匙串，每个测试用自己的服务名） |
| `ScriptedRunner`、`FakeReadiness` | `crates/core/tests/common/engine_kit.rs` | 子进程运行与环境检查（`ScriptedRunner` 报告的进程号是虚构的，只进存储，不能让宿主去清扫） |
| `Orphan` | `crates/core/tests/common/orphan.rs` | 宿主崩溃后留下的执行者（macOS：领着自己进程组的 `fake-agent`）；测试结束时没被回收就整组结束并回收 |
| `FakeRunner` | `crates/core/src/doctor.rs` 的测试模块 | 检查命令的输出（计数调用）；`SystemRunner` 本身用 `fake-agent` 测超时 |

和时间有关的缓存测试用 tokio 的暂停时钟（`#[tokio::test(start_paused = true)]` 加 `tokio::time::advance`），不真的等待。

## 夹具与脱敏

- 真实样本放在 `docs/gates/samples/<里程碑>/`，测试需要时复制到 `crates/<crate>/tests/fixtures/`。
- 提交前替换：姓名改为 `Jane Doe`，邮箱改为 `jane@example.com`，需求标识改为 `DEMO-N`，UUID 改为 `00000000-0000-0000-0000-00000000000N`，项目改为 `Demo Project`，团队改为 `Demo`，URL 改为 `https://linear.app/demo/issue/DEMO-N`，路径改为 `~/…`，会话 ID 改为全零。字段名与结构保持原样。
- 不为了让测试通过去手改真实样本；需要别的形态就另写一份虚构夹具。

## 实机验收规则

- 只在 Linear 沙箱项目和本机沙箱仓库（用本地裸仓库充当远端）上进行，不碰真实业务仓库。
- 记录环境与版本、每一步的命令和结果、从建需求到被接走的耗时、Linear 状态与评论、推送结果、异常与未验证项。
- 没做的项写「待实机」，不能用测试结果代替。

## 验收报告模板

````markdown
# <里程碑>：<主题>（<日期>）— <通过 | 部分通过 | 待实机>

## 环境
- 系统与版本；Claude Code / Codex 版本；IssueRelay 提交

## 步骤与结果
| 步骤 | 命令 | 结果 |
|---|---|---|

## 与设计不一致之处

## 未闭环
````
