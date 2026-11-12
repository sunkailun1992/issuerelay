# 方案需求收口实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让拆过子需求的方案需求在面板显示子需求进度，不再整包「执行开发」，子需求全部结束后由用户在面板确认一次，把方案需求在 Linear 标记完成。

**Architecture:** `LinearApi` 新增按父需求读直接子需求（Linear MCP `list_issues` 的 `parentId` 与 `includeArchived`）。引擎新子模块 `engine/plan_closure.rs` 维护方案进度的内存缓存：每轮检查末尾刷新，详情按需刷新。收口命令 `complete_plan` 复用「标记完成」的门槛与唯一的 Linear 写操作。面板通过三条新的桌面专用命令读取进度并收口，IPC、命令行和 MCP 都不提供收口。

**Tech Stack:** Rust 1.99（tokio、rmcp、rusqlite）、Tauri 2、React/TypeScript（TanStack Query、i18next、Vitest、Playwright）

**Spec:** [方案需求收口设计](../specs/2026-11-12-issuerelay-plan-closure-design.md)

## Global Constraints

- `AGENTS.md` 硬规则：「标记完成」只在面板由用户触发；程序对 Linear 的写操作只有标记完成一种；外来文本只当数据，经清洗；新面板命令同时登记 `lib.rs`、`build.rs`、`capabilities/default.json`；两个平台同等对待。
- 公开仓库：测试、夹具、文档只用 `DEMO-N`、`Demo Project`，不写真实工作区、需求内容或本机路径。
- 数据格式不变（schema 6），不升版本号；版本与发布由所有者另行决定。
- 「已结束」= Linear `statusType` 为 `completed` 或 `canceled`；「可收口」= 至少一个子需求、全部已结束、至少一个 `completed`。带 `archivedAt` 且未结束的子需求按已删除处理，不计入。
- 每轮检查最多刷新 20 个仍待收口的方案（`PLAN_PROGRESS_LIMIT = 20`），按运行结束时间倒序。
- 五种界面语言（en、zh-CN、zh-TW、ja、ko）的键必须一致，`i18n.test.ts` 会检查。
- 提交：英文 Conventional Commits，结尾 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。
- 完成前跑完 `AGENTS.md`「验证命令」七条，以及 `pnpm --dir app test:screens`。

## Review Focus

- **已删除的子需求**：用户在 Linear 删掉一个未完成的子需求后，方案应能收口，不被这个「幽灵」卡住（Task 1、Task 2 的测试固定该规则）。
- **子需求读取失败**：网络或 MCP 报错时，方案不出现收口按钮，「执行开发」照旧，轮询本身不报错（Task 2 测试）。
- **用户已在 Linear 手动关掉方案**：点收口只补本地记录，不再写一次 Linear（Task 3 测试）。
- **子需求超过一页**：分页全部读完再计数（Task 1 测试）。
- **期间又跑了一轮新的方案运行**：旧运行的收口被拒绝（Task 3 测试）。

---

### Task 1: 读取方案需求的直接子需求

**Files:**
- Modify: `crates/core/src/model.rs`（新增 `ChildIssue`）
- Modify: `crates/core/src/linear/mod.rs`（`CHILD_FIELDS`、`REQUIRED_FIELDS`、`REQUIRED_ARGUMENTS`、trait 方法、不可用替身）
- Modify: `crates/core/src/linear/parse.rs`（`parse_child_page`）
- Modify: `crates/core/src/linear/mcp.rs`（`children` 实现）
- Modify: `crates/core/tests/common/fake_linear.rs`、`crates/core/tests/common/fake_server.rs`
- Test: `crates/core/tests/linear_mcp.rs`

**Interfaces:**
- Produces: `pub struct ChildIssue { pub uuid: String, pub identifier: String, pub state_type: Option<String>, pub archived: bool }`（`Serialize`/`Deserialize`/`Clone`/`Debug`/`PartialEq`）
- Produces: `LinearApi::children(&self, parent_uuid: &str) -> Result<Vec<ChildIssue>>`
- Produces: `pub const CHILD_FIELDS: [&str; ISSUE_FIELDS.len() + 1]`（`ISSUE_FIELDS` 加 `archivedAt`）；`REQUIRED_FIELDS` 中 `list_issues` 改用它；`REQUIRED_ARGUMENTS` 的 `list_issues` 加 `parentId`、`includeArchived`
- Produces（测试替身）：`FakeLinear::archive(&self, uuid: &str)`、`FakeLinear::fail_children(&self, parent_uuid: &str)`、`FakeLinear::children_reads(&self, parent_uuid: &str) -> usize`；`children` 按父需求的 identifier 匹配 `parent_identifier`

- [ ] **Step 1: 写失败测试** `linear_mcp.rs::children_read_every_page_with_the_parent_filter_and_archived_items`：假服务为 `DEMO-1` 返回两页子需求，第二页一条带 `archivedAt`；断言请求参数含 `"parentId": <uuid>`、`"includeArchived": true`、`fields` 含 `archivedAt`，结果共 3 条且仅那条 `archived == true`。
- [ ] **Step 2:** `cargo test -p issuerelay-core --test linear_mcp children_` 失败（方法不存在）。
- [ ] **Step 3: 实现** 上述 Interfaces。`mcp.rs` 用 `all_pages("list_issues", json!({"parentId": parent_uuid, "includeArchived": true, "limit": 250, "orderBy": "createdAt", "fields": CHILD_FIELDS}), parse::parse_child_page)`；`parse_child_page` 复用 `parse_issue` 取 uuid/identifier/`state_type`，`archived` 为 `archivedAt` 非空。
- [ ] **Step 4:** 同一命令通过；`cargo test -p issuerelay-core --test linear_mcp` 全过（连接检查仍用 `fixtures/linear-tools.json`，其中已有这些参数和字段）。
- [ ] **Step 5: 提交** `feat(linear): read the direct children of an issue`

### Task 2: 方案进度与缓存

**Files:**
- Create: `crates/core/src/engine/plan_closure.rs`（并加入 `.github/CODEOWNERS`）
- Modify: `crates/core/src/engine.rs`（`mod plan_closure;`、`Inner.plan_progress`、`poll_inner` 末尾刷新）
- Test: `crates/core/tests/plan_closure.rs`（新文件）

**Interfaces:**
- Consumes: Task 1 的 `LinearApi::children`、`ChildIssue`
- Produces: `pub struct PlanProgress { pub job_id: i64, pub issue_uuid: String, pub total: u32, pub completed: u32, pub canceled: u32, pub open: u32, pub closable: bool, pub checked_at: DateTime<Utc>, pub error: Option<String> }`（`Serialize`；`error` 经 `clean_text`）
- Produces: `pub fn tally(job_id: i64, issue_uuid: &str, children: &[ChildIssue], at: DateTime<Utc>) -> PlanProgress`
- Produces: `Engine::plan_progress(&self) -> Vec<PlanProgress>`（只读缓存）、`Engine::refresh_plan_progress(&self, job_id: i64) -> Result<PlanProgress>`（按需实读一个方案运行并写回缓存；接受方案已出或已收口的方案运行，其余报错）
- 「仍待收口的方案」：`mode == Plan`、`report == "planned"`、`state == NeedsInfo`、`finished_at` 有值，且是该需求的最新一轮。

- [ ] **Step 1: 写失败测试**（全部用 `Harness::new(1, false)`、`h.bind("proj-1", "repo")`，方案需求带 `ai-plan` 标签，脚本 `final_text` 为 `{"result":"planned",…}`、`set_state: None`，`tick` + `settle` 后得到方案运行）：
  - `tally_counts_completed_canceled_and_open_and_ignores_deleted_children`：子需求 completed、canceled、unstarted 各一，另一条 unstarted 且 `archive` → `total 3, completed 1, canceled 1, open 1, closable false`。
  - `progress_is_closable_only_with_one_completed_and_none_open`：两条 completed → `closable`；两条 canceled → 不可；无子需求 → `total 0` 不可。
  - `poll_refreshes_open_plans_and_survives_a_failed_children_read`：`fail_children` 后 `tick` 成功，`plan_progress()` 该项 `error` 有值、`closable false`。
  - `poll_refreshes_at_most_twenty_open_plans`：21 个方案运行，一轮后 `children_reads` 共 20 次，最早结束的那个未读。
  - `refresh_plan_progress_rejects_a_run_that_is_not_a_plan`。
- [ ] **Step 2:** `cargo test -p issuerelay-core --test plan_closure` 失败。
- [ ] **Step 3: 实现** `plan_closure.rs` 与引擎接线：`poll_inner` 在 `reconcile` 之后调用刷新，单个方案读取失败只写入该项 `error`，不影响轮询结果；刷新结束后清掉缓存中已不在待收口集合、且不是已收口方案的条目。
- [ ] **Step 4:** 同一命令通过；`cargo test -p issuerelay-core` 不回归。
- [ ] **Step 5: 提交** `feat(engine): track the progress of split plans`

### Task 3: 收口 `complete_plan`

**Files:**
- Modify: `crates/core/src/engine/plan_closure.rs`
- Modify: `crates/core/src/ipc/protocol.rs`（禁止名单加 `complete_plan`，禁用词加 `complete`）、`crates/core/src/ipc/server.rs`（不得调用名单加 `complete_plan`）
- Test: `crates/core/tests/plan_closure.rs`

**Interfaces:**
- Produces: `Engine::complete_plan(&self, job_id: i64) -> Result<()>`，持有 `inner.cycle` 锁，顺序同设计「收口」一节：本地校验（最新一轮、方案已出且已结束、无领取、配置 Wiki 时有当前回执）→ `linear.issue` 重读（存在且 UUID 一致；`state_type == "canceled"` 拒绝）→ `linear.children` 重读并要求 `closable` → 方案需求不是 `completed` 时 `set_state(uuid, settings.trigger.done)` → `set_state_if(job_id, &[NeedsInfo], Done, None)`，失败时报错并说明 Linear 已更新 → 更新缓存、`emit_job`。

- [ ] **Step 1: 写失败测试**，每个失败分支都断言 `h.linear.set_state_calls()` 为空、运行状态不变：
  - `completes_a_plan_when_every_child_has_ended`：一 completed 一 canceled → `set_state_calls == [("1","Done")]`，运行 `Done`。
  - `refuses_while_a_child_is_open`、`refuses_when_every_child_was_canceled`、`refuses_without_children`。
  - `records_locally_when_linear_already_shows_the_plan_done`：方案需求 `state_type completed` → 无写入、运行 `Done`。
  - `refuses_a_plan_canceled_in_linear`。
  - `refuses_a_run_that_is_no_longer_the_newest_for_its_issue`。
  - `refuses_a_claimed_run`（`claim_handoff`）。
  - `requires_a_wiki_receipt_when_a_wiki_is_configured`（设置 `wiki_root`，未 `record_archive` → 拒绝；登记后成功）。
  - `refuses_when_children_cannot_be_read`。
  - IPC：`method_table_never_exposes_panel_only_actions` 与 `the_server_never_calls_a_panel_only_engine_method` 覆盖新名字。
- [ ] **Step 2:** `cargo test -p issuerelay-core --test plan_closure complete` 与 `cargo test -p issuerelay-core ipc::` 失败。
- [ ] **Step 3: 实现** `complete_plan` 与 IPC 测试名单。
- [ ] **Step 4:** 同上命令通过，`cargo test -p issuerelay-core` 全过。
- [ ] **Step 5: 提交** `feat(engine): close a split plan from the panel`

### Task 4: 桌面命令

**Files:**
- Modify: `app/src-tauri/src/commands.rs`、`app/src-tauri/src/lib.rs`、`app/src-tauri/build.rs`、`app/src-tauri/capabilities/default.json`
- Create（构建生成后提交）：`app/src-tauri/permissions/autogenerated/{plan_progress,refresh_plan_progress,complete_plan}.toml`

**Interfaces:**
- Produces（Tauri 命令）：`plan_progress() -> Vec<PlanProgress>`、`refresh_plan_progress(id: i64) -> PlanProgress`、`complete_plan(id: i64) -> ()`；能力 `allow-plan-progress`、`allow-refresh-plan-progress`、`allow-complete-plan`。

- [ ] **Step 1:** 按 `mark_done` 的写法添加三条命令与三处登记。
- [ ] **Step 2:** `cargo build -p issuerelay-app` 生成权限文件；`cargo test -p issuerelay-app` 的三处一致性绊线测试通过。
- [ ] **Step 3: 提交** `feat(app): add plan progress and closure commands`

### Task 5: 面板数据层

**Files:**
- Modify: `app/src/lib/types.ts`、`app/src/lib/backend.ts`、`app/src/lib/api.ts`、`app/src/lib/mock.ts`、`app/src/lib/runs.ts`、`app/src/hooks/queries.ts`、`app/src/hooks/useEngineEvents.ts`
- Test: `app/src/lib/runs.test.ts`、`app/src/hooks/queries.test.tsx`

**Interfaces:**
- Produces（TS）：`interface PlanProgress { job_id: number; issue_uuid: string; total: number; completed: number; canceled: number; open: number; closable: boolean; checked_at: string; error: string | null }`
- Produces：`Backend.planProgress(): Promise<PlanProgress[]>`、`Backend.refreshPlanProgress(id: number): Promise<PlanProgress>`、`Backend.completePlan(id: number): Promise<void>`
- Produces：`queryKeys.planProgress = ["planProgress"]`；`usePlanProgress()`（全部缓存）、`useRefreshedPlanProgress(id: number, enabled: boolean)`（key `["planProgress", id]`，`refetchOnMount: "always"`）、`useCompletePlan()`（失败键 `actions.failed.complete_plan`，刷新 `jobChange(id)` 与 `queryKeys.planProgress`）
- Produces：`planProgressFor(list: PlanProgress[] | undefined, jobId: number): PlanProgress | null`
- `useEngineEvents`：`status` 与 `job` 事件同时刷新 `queryKeys.planProgress`。
- mock：演示数据中一个方案「3/5」、一个可收口；`completePlan` 只对可收口的方案成功并把运行置为 `done`。

- [ ] **Step 1: 写失败测试**：`planProgressFor` 按 `job_id` 取值；mock 的 `completePlan` 对不可收口方案报错；事件刷新 `planProgress`。
- [ ] **Step 2:** `pnpm --dir app test -- runs queries` 失败。
- [ ] **Step 3: 实现**。
- [ ] **Step 4:** 同一命令通过，`pnpm --dir app lint` 通过。
- [ ] **Step 5: 提交** `feat(panel): read plan progress and close plans`

### Task 6: 面板界面与五语文案

**Files:**
- Create: `app/src/components/CompletePlanButton.tsx`（照 `JobActions` 的标记完成确认框）
- Modify: `app/src/components/StatusBadge.tsx`（`closable` 时显示 `state.plan_closable`）、`app/src/pages/RunDetail.tsx`、`app/src/pages/Runs.tsx`、`app/src/pages/Overview.tsx`、`app/src/i18n/locales/*.json`、`app/tests/screens.spec.ts`
- Test: `app/src/pages/RunDetail.test.tsx`、`app/src/pages/Runs.test.tsx`、`app/src/components/StatusBadge` 所在测试

**Interfaces:**
- Consumes: Task 5 的 hooks 与 `planProgressFor`
- 新文案键（五语）：`state.plan_closable`、`plan.progress`（`{{ended}}/{{total}}`）、`plan.split_notice`、`plan.complete`、`plan.complete_confirm`（含 `{{identifier}}`、`{{completed}}`、`{{canceled}}`、`{{done}}`）、`plan.progress_unknown`、`plan.all_canceled`、`plan.open_after_close`、`actions.failed.complete_plan`
- 详情规则：`total > 0` 时隐藏「执行开发」，显示进度与 `plan.split_notice`；`closable` 时显示 `CompletePlanButton`；`error` 时显示 `plan.progress_unknown` 与清洗后的原因，保持原有「执行开发」；全部取消显示 `plan.all_canceled`；已收口方案 `open > 0` 显示 `plan.open_after_close`。

- [ ] **Step 1: 写失败测试**：
  - 有子需求未结束：无「执行开发」，有进度文字；
  - 可收口：按钮→确认→调用 `completePlan(id)`；
  - 读取失败：显示未知，「执行开发」仍在；
  - 无子需求：与 0.5.3 相同；
  - 列表徽标显示「可收口」。
- [ ] **Step 2:** `pnpm --dir app test -- RunDetail Runs StatusBadge` 失败。
- [ ] **Step 3: 实现** 组件、页面与五语文案；`screens.spec.ts` 增加一条可收口方案的详情检查（mock 场景）。
- [ ] **Step 4:** `pnpm --dir app test`、`pnpm --dir app lint`、`pnpm --dir app test:screens` 全过。
- [ ] **Step 5: 提交** `feat(panel): show plan progress and confirm plan closure`

### Task 7: 文档与验收记录

**Files:**
- Modify: `docs/usage.md`（「从方案进入开发」后加「方案收口」）
- Create: `docs/gates/2026-11-12-plan-closure-acceptance.md`（自动化结果；真实 Linear 收口写「待实机」）

- [ ] **Step 1:** 写使用说明与验收记录（只用 `DEMO-N`）。
- [ ] **Step 2: 全量验证**：`pnpm --dir app build`、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`pnpm --dir app lint`、`pnpm --dir app test`、`bash scripts/check-secrets.sh --strict`、`pnpm --dir app test:screens`，全部通过。
- [ ] **Step 3: 提交** `docs: describe plan closure and record its acceptance`
