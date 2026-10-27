# IssueRelay 运行改进 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 实现设计里的六项改进：Linear 上取消、完成、改派或删除时停掉运行；记录用量；被挡住先不派；按标签分到不同目录；只出方案；重做时接续上一次的会话。

**Architecture:** 判断都在 `crates/core`：`store`（只加可空列）与 `model` 先扩字段；`executor::outcome` 的输出跟踪器多读用量与会话编号；`trigger` 负责模式、分流与被挡住的判定；`engine` 负责回查运行中的需求、被挡住的查询与缓存、接续会话的决定与接续不成的回退；`executor` 拼只出方案与接续的参数；`prompt` 多一种固定规则。面板、命令行只多字段与显示，不加 IPC 方法，也不加面板命令。

**Tech Stack:** Rust 1.99（edition 2024）、tokio、rusqlite、serde；React / TypeScript 面板（Vitest、Testing Library、Playwright 截图）；Tauri 2。

**Spec:** `docs/superpowers/specs/2026-10-27-issuerelay-run-improvements-design.md`（所有者 2026-10-27 定范围，细节为设计者定）。

## Global Constraints

- 工具链 Rust 1.99.0、edition 2024；面板用 `app/package.json` 的 Node 与 pnpm 版本。
- 子进程只用固定参数：提示词走标准输入，工作目录用 `current_dir`，不经 shell，参数里不放 Linear 的文本。新增的可变参数只有会话编号，且只接受 UUID 形状（36 个字符，16 进制与连字符，`8-4-4-4-12`）。
- `--permission-prompts none` 保留；三条固定规则（不设完成状态、不推主分支、不发正式环境）在两种提示词里都原样保留。
- `state.db` 的 `SCHEMA_VERSION` 保持 2，只加可空列；`JobState` 不加新值。
- 新配置字段都有默认值，等于默认值时不写入文件（`skip_serializing_if`），并加进 `config::unknown_keys` 的样例。
- 只出方案：Claude 参数 `--tools` 后面紧跟一个值 `Read,Grep,Glob`；Codex 新运行 `-s read-only`、接续 `-c sandbox_mode=read-only`，放行 `-c mcp_servers.linear.default_tools_approval_mode=approve`（只在精简运行或环境检查看到名为 `linear` 的服务时加）。
- 接续：Claude 在 `--permission-mode <档位>` 之后加 `--resume <编号>`；Codex 用 `exec resume`，选项同新运行，最后是编号与 `-`。Codex 的「自动审查」档位不接续。接续上限 7 天。
- 被挡住：状态类型为 `completed` 或 `canceled` 才算放开；结论缓存 2 分钟（需求 `updatedAt` 变了就重查）。
- 公开仓库：测试与夹具只用 `DEMO-N`、`Demo Project`、`Jane Doe`；第 4 节实测样本去掉真实需求内容后才进仓库。
- 面板：只有 `app/src/lib/api.ts` 碰 Tauri；外来文本只作清洗过的文本节点和细节行；新文案五种语言齐全。
- 提交：英文 Conventional Commits，每个 Task 一个提交，末尾 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。每个 Task 提交前跑它涉及的验证命令；全部完成后跑 AGENTS.md 的全套验证命令。

## Review Focus

- AI 自己在「需要补充信息」或「失败」路上去掉 `ai-ready`、改回 Todo：Linear 回查不能把这样的运行停掉。→ Task 3 的 `a_run_keeps_going_when_the_ai_takes_the_label_off_and_moves_it_to_todo`。
- 退回 v0.2.4：新版本写过的库，用旧版本的列清单读，每一行都照常读出。→ Task 1 的 `rows_written_now_read_with_the_old_column_list`。
- Claude 的会话已被清掉，接续以非 0 退出且没有 `system/init`：同一任务从头再跑，不记为失败。→ Task 8 的 `a_resume_that_cannot_start_runs_afresh_in_the_same_job`。
- 需求同时带同一项目两个分流绑定的标签：等待并写明，不随便挑一个。→ Task 6 的 `two_matching_label_bindings_make_the_issue_wait`。
- 不精简的 Codex 绑定、用户没有名为 `linear` 的服务：只出方案的参数里不加放行项（否则 Codex 启动失败）。→ Task 7 的 `codex_plan_run_adds_the_linear_approval_only_when_the_server_is_known`。

---

### Task 1: 运行记录的新列与模型字段

**Files:**
- Modify: `crates/core/src/model.rs`（`RunMode`、`RunUsage`、`Job` 新字段）
- Modify: `crates/core/src/store.rs`（新列、读写、清空）
- Modify: 所有 `Job { … }` 字面量（`trigger.rs`、`store.rs`、`model.rs` 的测试，`tests/common/engine_kit.rs`、`tests/engine.rs`、`app/src-tauri/src/test_support.rs`）

**Interfaces:**
- Produces:
  - `pub enum RunMode { Implement, Plan }`，serde 名 `implement` / `plan`；存库时 `Plan` 写 `"plan"`，`Implement` 写 NULL；`RunMode::from_stored(Option<&str>) -> RunMode`（不认识的值当 `Implement`）。
  - `pub struct RunUsage { pub cost_micro_usd: Option<i64>, pub session_cost_micro_usd: Option<i64>, pub input_tokens: Option<u64>, pub output_tokens: Option<u64>, pub cache_read_tokens: Option<u64>, pub cache_write_tokens: Option<u64>, pub turns: Option<u32> }`，`Default`；`RunUsage::is_empty(&self) -> bool`。
  - `Job` 新字段（都 `#[serde(default)]`）：`mode: RunMode`、`report: Option<String>`、`session_id: Option<String>`、`resume_job_id: Option<i64>`、`usage: Option<RunUsage>`（全空时为 `None`）。
  - `Store::record_run(&self, id: i64, report: Option<&str>, session_id: Option<&str>, usage: Option<&RunUsage>) -> Result<()>`：同时记下 `details_started_at = started_at`；读的时候这一列与 `started_at` 不同就不认 `report`、`session_id` 与用量（设计第 5 节）。`mark_finished` 的签名不变。
  - `Store::set_mode(&self, id: i64, mode: RunMode) -> Result<()>`、`Store::set_resume_job(&self, id: i64, resume: Option<i64>) -> Result<()>`。
  - `Store::latest_with_session(&self, uuid: &str) -> Result<Option<Job>>`：这条需求最近一次会话还读得出的运行。
  - `NewJob` 多 `mode: RunMode` 与 `resume_job_id: Option<i64>`。
  - `Store::mark_started` 另清空 `report`、`session_id`、用量与 `details_started_at`（保留 `mode`、`resume_job_id`）；`Store::requeue` 另清空这些与 `resume_job_id`。

- [x] **Step 1: 写失败的测试（`store.rs` 测试模块）**
  - `new_columns_are_added_to_an_old_database_without_a_version_bump`：用 M1 / schema 2 形状的表（`ADDED_IN_SCHEMA_2` 那组测试的建表语句）建库，打开后 `pragma_table_info('jobs')` 含 `mode`、`report`、`session_id`、`resume_job_id`、`cost_micro_usd`、`session_cost_micro_usd`、`input_tokens`、`output_tokens`、`cache_read_tokens`、`cache_write_tokens`、`turns`，且 `user_version` 仍是 2。
  - `rows_written_now_read_with_the_old_column_list`：插入并 `mark_finished`（带用量与会话编号）后，用旧的 `COLUMNS` 字符串（复制一份 v0.2.4 的常量进测试）`SELECT`，每行都能被旧的 `row_to_job` 形状读出（列数 18，`state` 是已知值）。
  - `a_finished_run_keeps_its_report_session_and_usage`：`get` 读回的 `Job` 的 `report`、`session_id`、`usage` 与写入相同；全空用量读回 `None`。
  - `requeue_and_start_clear_the_run_details`：`requeue` 后 `report`、`session_id`、`resume_job_id`、`usage` 都空；`mark_started` 后 `report`、`session_id`、`usage` 都空，`mode` 等于传入值。
  - `plan_mode_is_stored_as_plan_and_implement_as_null`：直接查列值。
- [x] **Step 2: 跑测试确认失败** — `cargo test -p issuerelay-core --lib store`，预期编译失败或断言失败。
- [x] **Step 3: 实现**：`ADDED_LATER` 列表（列名、类型），每次打开都按 `pragma_table_info` 补缺的列（不论版本号），在 `store.rs` 顶部写明规则（只加可空列、旧版本可忽略的改动不升版本号；新状态值或改了含义必须升）。`COLUMNS` 追加新列，`row_to_job` 读新列。
- [x] **Step 4: 更新所有 `Job` 字面量**，跑 `cargo test -p issuerelay-core` 通过。
- [x] **Step 5: 提交** `feat(store): record run mode, report, session and usage in nullable columns`

### Task 2: 输出跟踪：用量与会话编号

**Files:**
- Modify: `crates/core/src/executor/outcome.rs`（`FinalTextTracker` 改名 `OutputTracker`，多读用量与会话）
- Modify: `crates/core/src/runner.rs`（`RunOutput` 多两项；`record` 用新跟踪器）
- Modify: `RunOutput { … }` 的全部字面量（`runner.rs`、`engine.rs`、`tests/common/engine_kit.rs` 等）
- Test: `crates/core/tests/fixtures/claude-stream-resumed.jsonl`、`codex-exec-turns.jsonl`（新，按第 4 节实测形状、去掉真实内容）

**Interfaces:**
- Consumes: Task 1 的 `RunUsage`。
- Produces:
  - `pub struct OutputTracker`：`new(kind)`、`observe(&mut self, line: &str)`、`final_text(&self) -> Option<&str>`、`into_output(self) -> TrackedOutput`。
  - `pub struct TrackedOutput { pub final_text: Option<String>, pub usage: Option<RunUsage>, pub session_id: Option<String> }`。
  - `RunOutput { pub exit: ExitKind, pub final_text: Option<String>, pub usage: Option<RunUsage>, pub session_id: Option<String> }`。
  - `pub fn is_session_id(text: &str) -> bool`（UUID 形状）放在 `outcome.rs`。

- [x] **Step 1: 写失败的测试（`outcome.rs`）**
  - `claude_usage_comes_from_the_result_event`：`result` 行 `{"type":"result","subtype":"success","total_cost_usd":0.0943946,"num_turns":3,"usage":{"input_tokens":4,"output_tokens":13,"cache_read_input_tokens":20786,"cache_creation_input_tokens":4364}}` → `session_cost_micro_usd = Some(94395)`（四舍五入）、`input_tokens = Some(4)`、`output_tokens = Some(13)`、`cache_read_tokens = Some(20786)`、`cache_write_tokens = Some(4364)`、`turns = Some(3)`；`cost_micro_usd` 由引擎算，这里为 `None`。
  - `claude_session_comes_from_init_only`：只有 `result`（带 `session_id`）而没有 `system/init` 时 `session_id` 为 `None`；有 `{"type":"system","subtype":"init","session_id":"b88d7053-d27f-4825-b81c-80f6fbf109d0"}` 时取它。
  - `codex_usage_adds_up_every_turn`：两行 `turn.completed`，`input_tokens` 15088 与 30202 → 45290；`cached_input_tokens` 记入 `cache_read_tokens`；`cache_write_input_tokens` 记入 `cache_write_tokens`；`turns` 与花费为 `None`；`thread.started` 的 `thread_id` 是会话编号。
  - `odd_numbers_are_left_out`：负数、`NaN`、字符串数字都当没有；全没有时 `usage` 为 `None`。
  - `session_ids_must_look_like_uuids`：`is_session_id("01a17cd5-323b-7e51-8637-4bced9551eaa")` 真；带空格、太短、`--x` 假。
- [x] **Step 2: 跑测试确认失败** — `cargo test -p issuerelay-core --lib outcome`。
- [x] **Step 3: 实现** 跟踪器；`runner::run` 结束时把 `into_output()` 的三项放进 `RunOutput`。`failed()` 等处补 `usage: None, session_id: None`。
- [x] **Step 4: 跑 `cargo test -p issuerelay-core`**（含 `tests/runner.rs`）通过。
- [x] **Step 5: 提交** `feat(runner): read usage and the session id from executor output`

### Task 3: Linear 上取消、完成、改派或删除时停掉运行

**Files:**
- Modify: `crates/core/src/model.rs`（`Issue.state_type: Option<String>`）、`linear/mod.rs`（`ISSUE_FIELDS` 加 `statusType`）、`linear/parse.rs`（读 `statusType`）
- Modify: `crates/core/src/engine.rs`（`stop_causes`、`watch_running`、`finish_job` 的原因）
- Modify: 所有 `Issue { … }` 字面量；`tests/fixtures/linear-tools.json` 不改（M0 的列表已有 `statusType`）
- Test: `crates/core/tests/engine.rs`、`linear/parse.rs` 测试

**Interfaces:**
- Produces:
  - `Issue.state_type: Option<String>`（小写的状态类型，`#[serde(default)]`）。
  - `pub enum LinearStop { Deleted, Canceled, Completed, Reassigned }`（`engine.rs`），`fn reason(self) -> &'static str` 依次为 `the issue no longer exists in Linear`、`the issue was canceled in Linear`、`the issue was completed in Linear`、`the issue was reassigned in Linear`。
  - `pub fn linear_stop(issue: Option<&Issue>, me: &str, done_name: &str) -> Option<LinearStop>`（纯函数，规则见设计第 6.1 节；没有 `state_type` 时只按状态名与 `done_name` 比）。

- [x] **Step 1: 写失败的测试**
  - `parse.rs`：`status_type_is_read_when_present`（`"statusType":"canceled"` → `Some("canceled")`；缺省 → `None`）。
  - `engine.rs` 单元：`linear_stop_maps_each_case`（`None` → `Deleted`；`canceled` → `Canceled`；`completed` → `Completed`；负责人是别人或空 → `Reassigned`；`started` 且负责人是我 → `None`；没有状态类型、状态名为 `Done` → `Completed`）。
  - `tests/engine.rs`（`Harness::new(1, true)`，跑起来后改 Linear 再 `poll_once`）：
    - `a_run_stops_when_its_issue_is_canceled_in_linear`：`state_type = Some("canceled")` 后一次 `poll_once`，任务为 `Canceled`，原因 `the issue was canceled in Linear`。
    - `a_run_stops_when_its_issue_is_reassigned`、`..._deleted`（从 `FakeLinear` 移除；新加 `FakeLinear::remove(uuid)`）、`..._completed`。
    - `a_run_keeps_going_when_the_ai_takes_the_label_off_and_moves_it_to_todo`：去掉标签、状态改 Todo 后 `poll_once`，任务仍在运行。
    - `a_failed_read_does_not_stop_a_run`：`fail_issue_reads` 后 `poll_once`，任务仍在运行。
    - `a_user_stop_is_still_stopped_by_the_user`：`cancel_job` 后原因仍是 `stopped by the user`。
- [x] **Step 2: 跑测试确认失败** — `cargo test -p issuerelay-core --test engine linear_stop` 等。
- [x] **Step 3: 实现**：`Inner.stop_causes: Mutex<HashMap<i64, LinearStop>>`；`poll_inner` 在 `reconcile` 之后调 `watch_running(&linear, &me.id, cfg)`：读 `running` 的任务编号与各自 `issue_uuid`（从 store 读），每条 `linear.issue(uuid)`，读失败只记 `tracing::warn!`；判定要停时先记原因再 `stop_run`，记一行 `tracing::info!(job_id, issue, reason, "stopping the run: …")`。`finish_job` 开头 `take` 掉这一项，退出方式是 `ExitKind::Canceled` 时用它的 `reason()` 代替 `decide` 给的原因。
- [x] **Step 4: 更新所有 `Issue` 字面量**（加 `state_type: None`），跑 `cargo test -p issuerelay-core` 通过。
- [x] **Step 5: 提交** `feat(engine): stop a run when Linear cancels, completes, reassigns or deletes its issue`

### Task 4: 记录用量

**Files:**
- Modify: `crates/core/src/engine.rs`（`finish_job` 写用量、算这次运行自己的花费）
- Modify: `tests/common/engine_kit.rs`（`Script` 多 `usage`、`session_id`）
- Test: `crates/core/tests/engine.rs`

**Interfaces:**
- Consumes: Task 1 的 `FinishedRun`、Task 2 的 `RunOutput`。
- Produces: `fn own_cost(session_total: Option<i64>, previous_total: Option<i64>, resumed: bool) -> Option<i64>`（`engine.rs`，私有）：没有接续时等于 `session_total`；接续时为差值，负数或缺值为 `None`。

- [x] **Step 1: 写失败的测试**
  - 单元 `own_cost_subtracts_the_previous_total_only_when_resumed`：`(Some(94395), None, false) → Some(94395)`；`(Some(94395), Some(55137), true) → Some(39258)`；`(Some(10), Some(20), true) → None`；`(Some(10), None, true) → None`。
  - `tests/engine.rs` `a_finished_run_records_its_usage_and_session`：脚本给 `usage` 与 `session_id`，结束后 `latest()` 的 `usage.cost_micro_usd == session_cost_micro_usd`、`session_id` 一致、`report == Some("in_testing")`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**：`finish_job` 用 `FinishedRun` 一次写入；`report` 取解析出的 `Report` 的名字（`outcome::Report::as_str()`，Task 7 加 `planned` 前先有三个）；接续时从 `resume_job_id` 那行读 `usage.session_cost_micro_usd` 算差。
- [x] **Step 4: 跑 `cargo test -p issuerelay-core` 通过。**
- [x] **Step 5: 提交** `feat(engine): record what each run cost and used`

### Task 5: 被挡住先不派

**Files:**
- Modify: `crates/core/src/model.rs`（`Issue.blocked_by: Option<Vec<String>>`）
- Modify: `crates/core/src/linear/mod.rs`（`ISSUE_DETAIL_FIELDS`，`REQUIRED_FIELDS` 里 `get_issue` 用它）、`linear/mcp.rs`（`issue()` 用它）、`linear/parse.rs`（读 `relations.blockedBy[].id`）
- Modify: `crates/core/src/trigger.rs`（`WaitReason::BlockedBy`、`evaluate` 多一个参数）
- Modify: `crates/core/src/engine.rs`（被挡住的查询与缓存，`poll_inner` 的前置步骤、`start_job`、`issues()`）
- Modify: `tests/common/fake_linear.rs`（`issue()` 回带 `blocked_by`；`list` 系列清空它，模拟 `list_issues` 读不到关系）、`tests/common/fake_server.rs`（`get_issue` 接受 `relations` 字段）
- Test: `trigger.rs`、`parse.rs`、`mcp.rs`、`tests/engine.rs`、`tests/linear_mcp.rs`

**Interfaces:**
- Produces:
  - `pub const ISSUE_DETAIL_FIELDS: [&str; 15]` = `ISSUE_FIELDS`（14 个，含 Task 3 的 `statusType`）加 `relations`。
  - `WaitReason::BlockedBy(Vec<String>)`；`describe()` = `blocked by DEMO-3, DEMO-4, which are not done yet`；`WaitReasonView::BlockedBy { issues: Vec<String> }`，代码 `blocked_by`。
  - `trigger::evaluate(issue, cfg, me, ready, home, blockers: &[String]) -> Eligibility`：绑定、目录、执行者都通过后，`blockers` 非空则 `Waiting(BlockedBy(blockers.to_vec()))`。
  - 引擎私有：`async fn unfinished_blockers(&self, linear: &dyn LinearApi, issue: &Issue) -> Result<Vec<String>>`（读需求详情得到 `blocked_by`，再逐条读挡住它的需求；`None` 或状态类型为 `completed` / `canceled` 的不算；同一轮同一条只读一次；结论按 `(uuid, updated_at)` 缓存 2 分钟）。

- [x] **Step 1: 写失败的测试**
  - `parse.rs`：`blocked_by_lists_the_blocking_identifiers`（形状照第 4.1 节，`"blockedBy":[{"id":"DEMO-4","title":"…"}]` → `Some(vec!["DEMO-4"])`；没有 `relations` → `None`；`id` 形状不对的项丢掉）。
  - `mcp.rs`：`get_issue_asks_for_relations_and_list_issues_does_not`（`REQUIRED_FIELDS` 的两项）；M0 工具列表仍通过连接检查。
  - `trigger.rs`：`a_blocked_issue_waits_after_the_binding_checks`（未绑定时仍是 `ProjectNotBound`；绑定好后是 `BlockedBy`）、`blocked_by_has_a_code_the_panel_translates`。
  - `tests/engine.rs`：
    - `a_blocked_issue_waits_until_its_blocker_is_done`：DEMO-1 被 DEMO-2（`started`）挡住 → `WaitingBinding`、原因含 `DEMO-2`；把 DEMO-2 的状态类型改成 `completed` 并跳过缓存（`Engine::forget_blocker_checks()`，`#[doc(hidden)] pub` 只给测试）后 `tick` → 运行。
    - `a_blocker_that_no_longer_exists_does_not_block`。
    - `a_failed_blocker_read_fails_the_poll_and_changes_nothing`。
    - `blocker_checks_are_reused_within_two_minutes`：同一 `updated_at` 的第二次 `poll_once` 不再读挡住它的需求（`FakeLinear` 记 `issue()` 的调用次数）。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**；`reconcile` 拿到 `HashMap<String, Vec<String>>`（uuid → 未完成的挡住者），只为会入队或重新评估的候选需求查；`start_job` 用重读到的 `blocked_by` 再查一次；`issues()` 只用缓存，缓存没有时为 Todo 候选需求查。
- [x] **Step 4: 更新 `Issue` 字面量**（`blocked_by: None`），跑 `cargo test -p issuerelay-core` 通过。
- [x] **Step 5: 提交** `feat(engine): hold back issues that Linear marks as blocked`

### Task 6: 按标签分到不同目录

**Files:**
- Modify: `crates/core/src/config.rs`（`Binding.labels`、校验、`route`、`unknown_keys` 样例）
- Modify: `crates/core/src/trigger.rs`（`evaluate` 用 `route`；两种新等待原因）
- Modify: `crates/core/src/engine.rs`（`add_binding` 的替换规则）
- Modify: `crates/core/src/doctor.rs`（`BindingReport.labels`）
- Modify: `crates/cli/src/main.rs`、`crates/cli/src/format.rs`（`bind add --label/--id`，`bind list` 显示）
- Modify: 所有 `Binding { … }` 字面量（加 `labels: Vec::new()`）
- Test: `config.rs`、`trigger.rs`、`tests/engine.rs`、`crates/cli` 测试

**Interfaces:**
- Produces:
  - `Binding.labels: Vec<String>`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`）。
  - `pub enum Route<'a> { Bound(&'a Binding), NotBound, SeveralMatch(Vec<String>), NoMatch }`；`Config::route(&self, issue: &Issue) -> Route<'_>`（`SeveralMatch` 带需求上对上的标签，按需求标签的顺序）。
  - `WaitReason::LabelsMatchSeveralBindings(Vec<String>)`（`labels match several bindings: …`，代码 `labels_match_several_bindings`，字段 `labels`）、`WaitReason::NoBindingForLabels`（`no binding of the project takes the issue's labels`，代码 `no_binding_for_labels`）。
  - `pub fn binding_id_for(project_id: &str, labels: &[String]) -> String`（`config.rs`）：没有标签为项目 id，有标签为 `<项目 id>#<第一个标签>`。
  - `doctor::BindingReport.labels: Vec<String>`。

- [x] **Step 1: 写失败的测试**
  - `config.rs`：`a_project_may_have_one_default_binding_and_label_bindings`；`two_default_bindings_of_a_project_are_refused`；`a_label_on_two_bindings_of_a_project_is_refused`（大小写、首尾空白不同也算同一个）；`the_trigger_and_plan_labels_cannot_route`；`routing_labels_must_be_plain_names`；`route_picks_the_label_binding_then_the_default`；`every_key_a_save_writes_is_known` 更新样例（带一个标签）。
  - `trigger.rs`：`two_matching_label_bindings_make_the_issue_wait`（`LabelsMatchSeveralBindings(["Frontend","Backend"])`）；`no_default_and_no_matching_label_waits`。
  - `tests/engine.rs`：`two_label_bindings_run_in_their_own_folders_in_parallel`（`max_concurrent = 2`，两条需求各带一个标签，两次运行的 `spec.cwd` 是两个目录、同时在跑）；`adding_a_default_binding_replaces_the_projects_old_default`；`a_label_binding_does_not_replace_the_default`。
  - `crates/cli`：`bind_add_sends_labels_and_derives_the_id`（`--label Frontend` → `id = "proj-1#Frontend"`，`labels = ["Frontend"]`；`--id x` 优先）。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**；`binding_for_project` 删除，调用处改用 `route`；`add_binding` 替换同 `id` 的，以及同项目里冲突的（都是默认，或标签有交集）。
- [x] **Step 4: 更新全部 `Binding` 字面量**，跑 `cargo test --workspace` 通过。
- [x] **Step 5: 提交** `feat(config): route a project's issues to folders by label`

### Task 7: 只出方案

**Files:**
- Modify: `crates/core/src/config.rs`（`TriggerNames.plan_label`，默认 `ai-plan`，校验）
- Modify: `crates/core/src/trigger.rs`（`candidate_mode`）
- Modify: `crates/core/src/engine.rs`（两个标签的候选查询与合并、模式入库、方案运行的结果判定、不做「看似没改动」的重读）
- Modify: `crates/core/src/prompt.rs`（`render_plan`）
- Modify: `crates/core/src/executor/mod.rs`（`RunOptions.plan`、`RunOptions.linear_tools_approval`）
- Modify: `crates/core/src/executor/outcome.rs`（`Report::Planned`、`Report::as_str`）
- Modify: `app/src-tauri/src/notify.rs`（方案已出的文字）
- Modify: 所有 `TriggerNames { … }` 字面量
- Test: 以上各文件的测试与 `tests/engine.rs`

**Interfaces:**
- Produces:
  - `TriggerNames.plan_label: String`（`#[serde(default)]` 走结构体默认值 `ai-plan`）；校验：单行名字、与 `label` 不同（`same_name`）。
  - `pub fn candidate_mode(issue: &Issue, names: &TriggerNames, me: &str) -> Option<RunMode>`；`is_candidate` 改为 `candidate_mode(..).is_some()`。
  - `prompt::render_plan(input: &PromptInput<'_>) -> String`（固定规则逐条按设计第 10.3 节，第一行同 `render`：`You are working unattended on exactly one Linear issue: {id} ({url}).`）。
  - `executor::RunOptions` 新字段：`plan: bool`、`linear_tools_approval: bool`、`resume: Option<&'a str>`（Task 8 用）。
  - `Report::Planned(String)`、`Report::as_str(&self) -> &'static str`（`in_testing`、`needs_info`、`failed`、`planned`）。
  - `pub fn decide_plan(exit: &ExitKind, report: Option<&Report>) -> (JobState, Option<String>)`（`engine.rs`）：结束方式不是干净退出时同 `decide`；干净退出且 `Planned` 或 `NeedsInfo` → `NeedsInfo`；其余 → `Failed`，原因 `the planning run finished without a plan`。

- [x] **Step 1: 写失败的测试**
  - `config.rs`：`plan_label_defaults_to_ai_plan_and_must_differ_from_the_trigger_label`。
  - `trigger.rs`：`candidate_mode_prefers_implement_when_both_labels_are_there`、`plan_label_alone_is_a_plan_candidate`。
  - `prompt.rs`：`plan_prompt_keeps_the_three_fixed_rules`（含 `Never set the status to "Done"`、`Never push to main, master or any other primary branch`、`Never deploy to production`），`plan_prompt_forbids_changing_files_and_status`，`plan_prompt_asks_to_remove_the_plan_label`，`plan_prompt_ends_with_the_planned_result_line`。
  - `executor/mod.rs`：`claude_plan_run_limits_the_built_in_tools`（`--permission-mode <档位>` 之后紧跟 `--tools`、`Read,Grep,Glob`）；`codex_plan_run_is_read_only`（新运行有 `-s`、`read-only`，没有 `--dangerously-bypass-approvals-and-sandbox` 与 `--approve-for-me`）；`codex_plan_run_adds_the_linear_approval_only_when_the_server_is_known`（`linear_tools_approval: false` 时没有 `default_tools_approval_mode`）。
  - `outcome.rs`：`planned_is_a_report`。
  - `engine.rs` 单元：`decide_plan_maps_outcomes`。
  - `tests/engine.rs`：`a_plan_labeled_issue_runs_a_planning_run`（`spec.stdin` 含 `planning run`，`spec.args` 含 `--tools`，结束后 `NeedsInfo`、`report == Some("planned")`、`mode == Plan`）；`adding_ai_ready_after_a_plan_runs_the_implementation`（Linear 有新改动后新任务 `mode == Implement`）；`a_queued_plan_job_becomes_an_implementation_when_ai_ready_is_added`。
  - `app/src-tauri/src/notify.rs`：`a_finished_plan_has_its_own_text`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**：`poll_inner` 对 `label` 与 `plan_label` 各调一次 `candidate_issues`，按 uuid 合并；`issues()` 对两个标签各调一次 `labeled_issues`，按 uuid 合并、按 `updated_at` 倒序取 50；`enqueue` 写模式；`start_job` 按重读到的需求定模式（`mark_started` 写入），按模式选 `render` 或 `render_plan`、设 `RunOptions.plan`；`linear_tools_approval` = 这次运行是精简运行，或 Codex 的报告 `lean_linear_mcp` 为真；`finish_job` 按任务模式选 `decide` 或 `decide_plan`，方案运行不进 `STALE_REREAD_PAUSES` 的重读。
- [x] **Step 4: 更新 `TriggerNames` 字面量**，跑 `cargo test --workspace` 通过。
- [x] **Step 5: 提交** `feat: add planning runs for issues labeled ai-plan`

### Task 8: 重做时接续上一次的会话

**Files:**
- Modify: `crates/core/src/config.rs`（`Settings.resume_sessions`，默认 `true`）
- Modify: `crates/core/src/executor/mod.rs`（`RunOptions.resume`）
- Modify: `crates/core/src/prompt.rs`（`PromptInput.continuation: Continuation`）
- Modify: `crates/core/src/engine.rs`（入队时记 `resume_job_id`、开始时核对、接续不成的回退）
- Test: 以上各文件与 `tests/engine.rs`

**Interfaces:**
- Consumes: Task 2 的 `RunOutput.session_id`、`is_session_id`；Task 7 的 `RunOptions`。
- Produces:
  - `pub enum Continuation { None, Rework, AfterPlan }`（`prompt.rs`）；`Rework` 与 `AfterPlan` 各在第一行之后加一段（设计第 11.3 节）。
  - `fn resume_target(&self, job: &Job, kind: ExecutorKind, binding: &Binding, profile: PermissionProfile, plan: bool) -> Result<Option<(i64, String, RunMode)>, &'static str>`（引擎私有）：返回要接续的任务编号、会话编号与那次的模式，或不接续的原因（写进运行日志）。条件按设计第 11.1 节；Codex 的 `Auto` 档位返回 `Err("codex runs with automatic review cannot resume a session")`。
  - `executor::build` 在 `options.resume = Some(id)` 时：Claude 在 `--permission-mode <档位>` 之后插 `--resume`、编号；Codex 以 `exec`、`resume` 开头，其余选项同新运行（方案运行用 `-c`、`sandbox_mode=read-only` 代替 `-s`、`read-only`），最后是编号与 `-`。编号不过 `is_session_id` 时不接续（`build` 内再校验一次，丢掉 `resume`）。

- [x] **Step 1: 写失败的测试**
  - `executor/mod.rs`：`claude_resume_follows_the_permission_mode`；`codex_resume_puts_the_id_before_the_stdin_marker`（`["exec","resume","--json","--dangerously-bypass-approvals-and-sandbox", …精简参数…, "<id>", "-"]`）；`codex_plan_resume_uses_the_sandbox_setting`；`an_id_that_is_not_a_uuid_is_not_passed`。
  - `prompt.rs`：`continuation_keeps_every_fixed_rule`（三种 `Continuation` 都含九条固定规则）。
  - `tests/engine.rs`：
    - `a_reworked_issue_resumes_the_previous_session`：第一次运行脚本给 `session_id`，退回重做（Linear 改回 Todo、`updated_at` 前进）后新任务 `resume_job_id == Some(第一次)`，第二次的 `spec.args` 含 `--resume` 与那个编号，`spec.stdin` 含接续段。
    - `a_manual_retry_starts_afresh`：失败后 `retry_job`，参数里没有 `--resume`。
    - `resuming_is_skipped_for_another_folder_or_executor_or_when_off`：三种情形参数里都没有 `--resume`，日志开头有原因行。
    - `a_resume_that_cannot_start_runs_afresh_in_the_same_job`：接续那次脚本给 `Exited(Some(1))` 且无 `session_id`，同一任务紧接着第二次运行没有 `--resume`，最后按第二次的结果记录，`resume_job_id` 为空，日志开头写明接续不成。
    - `a_plan_session_is_resumed_by_the_implementation`：方案运行之后加 `ai-ready`，实现运行带 `--resume` 且提示词含「先读你写的方案」那一段。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**：`reconcile` 入队时（`should_enqueue` 为真且上一条已结束、有会话编号、设置开着）给 `NewJob.resume_job_id`；`start_job` 用 `resume_target` 定下，接续不成的原因进 `spec.notes` 并 `set_resume_job(id, None)`；运行任务里，接续那次以非 0 退出且 `session_id` 为空时，用同时准备好的新运行参数（不带 `--resume`，`notes` 加「上一次的会话接续不成，从头开始（退出码 N）」）在同一任务里再跑一次，并 `set_resume_job(id, None)`。
- [x] **Step 4: 跑 `cargo test --workspace` 通过。**
- [x] **Step 5: 提交** `feat(engine): resume the previous executor session when an issue comes back`

### Task 9: 面板

**Files:**
- Modify: `app/src/lib/types.ts`、`app/src/lib/mock.ts`、`app/src/lib/mock-fixtures.ts`
- Modify: `app/src/components/StatusBadge.tsx`（方案已出）、`JobActions.tsx`（如需）、新 `app/src/components/RunUsage.tsx`
- Modify: `app/src/pages/Issues.tsx`、`Runs.tsx`、`RunDetail.tsx`、`Bindings.tsx`、`BindingDialog.tsx`、`Settings.tsx`、`Help.tsx` / `components/HelpSections.tsx`
- Modify: `app/src/i18n/locales/{zh-CN,zh-TW,en,ja,ko}.json`
- Test: 对应的 `*.test.tsx`、`app/src/lib/*.test.ts`

**Interfaces:**
- Consumes: Task 1–8 的 Rust 字段（serde 名照搬）：`Job.mode`、`report`、`session_id`、`resume_job_id`、`usage`；`Binding.labels`；`Settings.resume_sessions`、`trigger.plan_label`；等待原因代码 `blocked_by`（`issues`）、`labels_match_several_bindings`（`labels`）、`no_binding_for_labels`；`BindingReport.labels`。
- Produces: `formatCost(microUsd: number): string`（`app/src/lib/format.ts`，小于 1 美分显示 `<$0.01`，否则两位小数）、`formatTokens(n: number): string`（千分位）。

- [x] **Step 1: 写失败的测试**：`format.test.ts`（`formatCost(94395) === "$0.09"`、`formatCost(4000) === "<$0.01"`、`formatTokens(20786) === "20,786"`）；`RunDetail.test.tsx`（有用量时显示花费、token、轮数与「估算」说明；Codex 无花费时不显示花费；接续的运行显示「接续 #N 的会话」并链到那次）；`StatusBadge.test.tsx`（`needs_info` + `report: "planned"` 显示方案已出）；`Issues.test.tsx`（三种新等待原因的文字；方案标记）；`BindingDialog.test.tsx`（选分流标签后保存的绑定带 `labels`，新 `id` 按 `<项目>#<标签>`，重名加序号）；`Settings.test.tsx`（接续开关、方案标签随「保存」提交）；`i18n.test.ts` 照旧核对五种语言的键。
- [x] **Step 2: 跑 `pnpm --dir app test` 确认失败。**
- [x] **Step 3: 实现** 页面与翻译。
- [x] **Step 4: 跑 `pnpm --dir app lint`、`pnpm --dir app test` 通过；本机跑 `pnpm --dir app test:screens`，按新界面更新受影响的截图。**
- [x] **Step 5: 提交** `feat(app): show planning runs, usage, routing labels and resumed sessions`

### Task 10: 命令行输出与文档

**Files:**
- Modify: `crates/cli/src/format.rs`（`jobs` 加模式与花费栏）
- Modify: `README.md`、`docs/install.md`（退回旧版本的注意）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 5.1、5.2、5.4、6.1、6.3、9.2、13 节指向新设计）
- Create: `docs/gates/samples/2026-10-27/`（第 4 节实测样本，去掉真实内容）、`docs/gates/2026-10-27-run-improvements-acceptance.md`（实测记录，实机验收各项先写「待实机」）
- Test: `crates/cli` 的格式测试

- [x] **Step 1: 写失败的测试**：`jobs_table_shows_mode_and_cost`（有花费的行显示 `$0.09`，方案运行的模式栏为 `plan`）。
- [x] **Step 2: 跑测试确认失败；Step 3: 实现；Step 4: `cargo test --workspace` 与 `bash scripts/check-secrets.sh --strict` 通过。**
- [x] **Step 5: 提交** `docs: describe the run improvements and record the probes`

### Task 11: 全套验证与审查

- [x] `pnpm --dir app build`，然后 AGENTS.md 的六条验证命令全部通过。
- [x] 派一个新的审查子智能体审全分支（对照设计与本计划），按结论修正并重跑验证。
- [x] 更新验收报告的自动测试部分；实机验收（设计第 14 节）等所有者在场再做（2026-10-28 做完，见验收报告）。
