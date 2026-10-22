# IssueRelay 精简运行 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 无人值守运行默认只带 Linear 的 MCP 服务和目标仓库自己的规则与设置（精简运行），新增推理力度设置；环境检查保证精简运行有它要的 Linear 服务与登录，运行时核对保证精简确实生效，否则结束运行。

**Architecture:** 全部判断在 `crates/core`：
- `config` 加精简开关、推理力度与生效状态；
- 新文件 `executor/lean.rs` 放精简参数、用户级 `CLAUDE.md` 路径与运行时核对；
- `executor::build` 改为接一组选项；`runner` 读输出时核对，并多一种结束方式；
- `doctor` 多出精简运行的两个结论，`trigger` 与 `engine` 按绑定取用。

命令行与面板只多字段，不加 IPC 方法，也不加面板命令。规则文件里的第三处固定参数例外先单独提交。

**Tech Stack:** Rust 1.99（edition 2024）、tokio、serde / serde_json、toml（已有依赖）；React / TypeScript 面板（Vitest、Testing Library、Playwright 截图）；Tauri 2。

**Spec:** `docs/superpowers/specs/2026-10-19-issuerelay-lean-run-design.md`（用户 2026-10-20 批准）。涉及总设计 `docs/superpowers/specs/2026-10-10-issuerelay-design.md` 第 6.1、6.4、8、9.2、14、18 节；实测依据见 `docs/gates/2026-10-14-m2-acceptance.md`「精简运行评估」与 `docs/gates/2026-10-14-m3-acceptance.md` 的 N3。

## 计划对设计的三处细化（主智能体定，交付计划时请用户确认）

1. **拦下运行的原因写成英文说明**：拦下的运行，原因和现有原因（`decide()` 里的「stopped by the user」等）一样，写成英文说明存进 `Job.reason`。面板照现在的做法，按状态显示翻译过的「执行失败」标题，原因作细节行。现在的运行原因都是自由文本，没有结构化代码；为这一条单独加代码要改表结构，所以留给 M2 遗留 N3（把所有运行原因改成结构化代码并翻译）一起做。Task 9 同步改设计第 6.3、8 节的说法。
2. **两条精简登录检查每次都跑**：每次环境检查都跑，不先看有没有精简绑定。每个执行者多一条一秒内的命令，换来检查结果不依赖配置，也不用在缓存里处理「开关刚改」的情形。Task 9 同步改设计第 7 节。
3. **默认推理力度只在面板改**：只在面板设置页改。IPC 没有改设置的方法，为它加方法属于极高风险（IPC 方法表）。命令行的 `bind add` 只设绑定自己的推理力度。

## Global Constraints

- 工具链 Rust 1.99.0、edition 2024；面板用 `app/package.json` 的 Node 与 pnpm 版本。包名：`issuerelay-core`、`issuerelay-cli`、`issuerelay-devhost`、`issuerelay-app`。
- 子进程只用固定参数：提示词走标准输入，工作目录用 `current_dir`，不经 shell，参数里不放 Linear 的文本。Task 1 之后有三处例外：
  - ① `setup.rs` 的命令行程序路径；
  - ② `env.rs` 的登录 shell；
  - ③ 精简 Claude 运行 `--settings` 里的用户级 `CLAUDE.md` 绝对路径。
- `--permission-prompts none` 必须保留；三档权限的映射（设计 6.1）不变。
- Linear 服务名与网址只有一份常量：Claude `linear-server`、Codex `linear`，网址 `LINEAR_MCP_URL`（`https://mcp.linear.app/mcp`）。
- 精简参数逐字按设计第 5.1、5.2 节（Task 3 列出完整顺序）；精简 Claude 运行另加环境变量 `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`。
- 推理力度只有五个值 `low`、`medium`、`high`、`xhigh`、`max`。Claude 传 `--effort <值>`，Codex 传 `-c model_reasoning_effort=<值>`。
- 精简不适用的两种情形：生效档位是「按个人设置」；绑定目录就是用户主目录。
- 面板规则：
  - 只有 `app/src/lib/api.ts` 碰 Tauri；不加命令。
  - 外来文本只作清洗过的文本节点和细节行。
  - 新文案五种语言齐全；620 与 605 px 内容宽度下够得着；对话框在 900×600 与 885×600 下「取消」「保存」可见。
- 测试不访问网络、不碰真实钥匙串、不依赖本机装的 `claude` / `codex`。测试数据只用虚构值：`DEMO-1`、`Demo Project`、`proj-1`、`/srv/demo-home`。临时文件用 `tempfile`。
- 示例路径不要写成用户主目录、外接磁盘或 Windows 用户目录下的样子：`scripts/check-secrets.sh` 的 `PATTERNS` 不分真假一律拦下。用 `/srv/demo-home`、`D:\Profiles\Demo User` 这类路径，或在测试运行时用 tempdir 拼出。
- 公开仓库红线：不出现令牌、API key、公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径。
- 两个平台同等对待：只在一个平台编译的代码要有另一个平台的实现，或写明为什么不需要；Windows 代码只能在 CI 上跑。
- 注释与标识符用英文（`docs/ai-coding/AI_COMMENT_STYLE_GUIDE.md`），文档用中文。
- 每个任务提交前：
  - Rust 任务跑 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`bash scripts/check-secrets.sh --strict`；
  - 面板任务先 `pnpm --dir app build`，再加 `pnpm --dir app lint`、`pnpm --dir app test`；
  - 再用 `git status --short`、`git diff --stat` 确认只提交了这次相关的文件。
- 提交信息用英文 Conventional Commits，正文写做了什么、为什么、怎样验证的，末尾 `Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>`。规则改动单独一个 `docs` 提交。
- 分支 `feat/lean-run`（工作区 `../issuerelay-worktrees/lean-run`）。开工、提交前、推送前各 `git fetch` 一次；禁止强推；经 PR 合入 `main`。
- `executor`、`runner`、`prompt`、`linear::auth`、`ipc::protocol` 由主智能体串行修改（`AGENTS.md`「多智能体协作」）；子智能体可以做面板、命令行与评审。

## Review Focus

1. **用户主目录的路径带空格、中文、单引号、反斜杠或 Windows 盘符**：`--settings` 的 JSON 原样到达执行者。带 `"`、`%`、`!`、`^` 或控制字符时不排除，运行日志写明（Task 3 `the_settings_argument_encodes_the_path_as_json`、Windows 的 `.cmd` 往返测试；Task 6 `an_unpassable_home_leaves_a_note_and_no_settings_argument`）。
2. **Claude 升级后 `init` 事件变了**：多一个内置 MCP 服务、字段改了名、`init` 之前先出了 AI 消息。运行要被结束、记为失败，原因写清多了什么（Task 4 的 `LeanCheck` 单元测试与 `a_lean_violation_ends_the_run_like_a_stop`）。
3. **用户只有 claude.ai 的 Linear 连接器，或 Linear 服务起了别的名字**：精简绑定显示「执行者未就绪」并给出添加 `linear-server` 的命令；完整配置的绑定照常可用（Task 5 `lean_linear_needs_the_exact_name_and_url`、`lean_bindings_wait_for_the_lean_checks`）。
4. **在面板里编辑一个关掉精简、设了推理力度的绑定，只改目录就保存**：两项保持原值（Task 8 `editing_keeps_lean_off_and_the_effort`）。
5. **旧版本写的配置（没有新字段）升级后读入**：精简为开、推理力度不设；`lean = false` 写回再读一致；新键不被报成「不认识」（Task 2 `a_binding_from_before_lean_runs_reads_as_lean`、`lean_is_written_only_when_off`、`every_key_a_save_writes_is_known`）。

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `AGENTS.md`、`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md`、`docs/ai-coding/SECURITY_CODING_SPEC.md`、`docs/ai-coding/AI_DESIGN_PATTERN_GUIDE.md`、总设计「一页概览」 | 第三处固定参数例外 | 1 |
| `crates/core/src/config.rs` | `Effort`、`LeanStatus`；绑定的 `lean` 与 `effort`；设置的 `default_effort`；`effort_for`、`lean_for` | 2 |
| `crates/core/src/executor/lean.rs`（新） | 两个 Linear 服务名常量；精简参数；用户级 `CLAUDE.md` 路径；`LeanCheck` | 3、4 |
| `crates/core/src/executor/mod.rs` | `RunOptions`、`Lean`；`CommandSpec` 的 `env`、`lean`、`notes`；`build` 的新参数 | 3 |
| `crates/core/src/setup.rs` | 改用 `lean.rs` 的服务名常量 | 3 |
| `crates/core/src/runner.rs` | 写入 `notes`、应用 `env`、精简核对、`ExitKind::LeanNotApplied` | 4 |
| `crates/core/src/engine.rs` | `decide` 与 `finish_job` 处理新结束方式；`EngineDeps` 的主目录；`start_job` 组装选项；就绪按绑定取用 | 4、5、6 |
| `crates/core/src/doctor.rs` | 精简的 Linear 服务判定、两条登录检查、Codex 模型服务检查、`Readiness`、`BindingReport.lean` | 5 |
| `crates/core/src/trigger.rs` | `evaluate` 按精简与否取就绪 | 5 |
| `crates/core/src/host.rs` | 精简检查用的空目录；把主目录交给引擎 | 5、6 |
| `crates/cli/src/main.rs`、`crates/cli/src/format.rs` | `bind add --lean`、`--effort`；列表与 `doctor` 的显示 | 7 |
| `app/src/lib/types.ts`、`binding-labels.ts`、`settings-form.ts`、`mock-fixtures.ts`、`mock.ts` | 类型、标签、设置表单、模拟数据 | 8 |
| `app/src/pages/BindingDialog.tsx`、`Bindings.tsx`、`Settings.tsx`、`Doctor.tsx`、`Help.tsx` 与测试 | 开关、下拉、卡片、环境检查、使用说明 | 8 |
| `app/src/i18n/locales/{en,zh-CN,zh-TW,ja,ko}.json`、`app/tests/screens.spec.ts` 的截图 | 文案与截图基线 | 8 |
| 总设计、`README.md`、`AI_ENGINEERING_GUARDRAILS.md` 第 4 节、本功能的设计 | 文档同步 | 9 |
| `docs/gates/<日期>-lean-run-acceptance.md`（新） | 沙箱实机验收报告 | 11 |

`crates/core/src/executor/lean.rs` 在 CODEOWNERS 已列的 `/crates/core/src/executor/` 之下，不用另加。

---

### Task 1: 规则文件：第三处固定参数例外

**Files:**
- Modify: `AGENTS.md`（「项目边界（硬规则）」的「子进程只用固定参数」一条与「禁止事项」的第三条）
- Modify: `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md`（第 1 节第 3 条）
- Modify: `docs/ai-coding/SECURITY_CODING_SPEC.md`（第 4 节第 1 条）
- Modify: `docs/ai-coding/AI_DESIGN_PATTERN_GUIDE.md`（反模式一节第 32 行附近）
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（「一页概览」的「安全底线」）
- 不改：历史计划里引用旧说法的段落（例如 `2026-10-13-issuerelay-m2-core.md`），它们是当时的记录。

**Interfaces:**
- Produces: 后续任务引用的规则文字。

- [ ] **Step 1: 六处（`AGENTS.md` 两处，另外四个文件各一处）把「仅有两处例外」改成「仅有三处例外」，并在 ② 之后加上 ③**

③ 的文字逐字用设计第 9 节：

> ③ 精简运行（Claude）的 `--settings` 里，唯一的可变部分是用户级 `CLAUDE.md` 的绝对路径：由 IssueRelay 从用户主目录（或执行者环境里的 `CLAUDE_CONFIG_DIR`）得出，经 JSON 编码；不是绝对路径或含不安全字符时不加这一项，并在运行日志里写明。

- [ ] **Step 2: 核对**

Run: `grep -rn "两处例外" AGENTS.md docs/ai-coding docs/superpowers/specs/2026-10-10-issuerelay-design.md`
Expected: 没有输出。

Run: `grep -rln "三处例外" AGENTS.md docs/ai-coding docs/superpowers/specs/2026-10-10-issuerelay-design.md | wc -l`
Expected: `5`（按文件计；`AGENTS.md` 里有两处）。

Run: `bash scripts/check-secrets.sh --strict`
Expected: 没有输出。

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md docs/ai-coding/SECURITY_CODING_SPEC.md docs/ai-coding/AI_DESIGN_PATTERN_GUIDE.md docs/superpowers/specs/2026-10-10-issuerelay-design.md
git commit -m "docs(rules): allow the user instructions path in lean Claude runs"
```

---

### Task 2: 配置：推理力度、精简开关与生效状态

**Files:**
- Modify: `crates/core/src/config.rs`（`ExecutorKind` 之后加 `Effort`；`Settings`、`Binding`、`Config`；`unknown_keys` 的样本；测试模块）
- Modify: 列出全部字段的 `Binding { .. }` 字面量，补上 `lean: true, effort: None`：
  - `crates/core/src/config.rs` 427、536、1057 行附近；
  - `crates/core/src/store.rs` 485、`crates/core/src/trigger.rs` 157、`crates/core/src/doctor.rs` 2014；
  - `crates/cli/src/main.rs` 150；
  - `crates/core/tests/host.rs` 856、`crates/core/tests/doctor.rs` 201、`crates/core/tests/common/engine_kit.rs` 218、`crates/core/tests/engine.rs` 520 与 860。

**Interfaces:**
- Produces:
  - `pub enum Effort { Low, Medium, High, Xhigh, Max }`
    - derive `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`，`#[serde(rename_all = "kebab-case")]`，序列化为 `low` … `max`；
    - `pub const ALL: [Effort; 5]`；
    - `pub fn as_str(self) -> &'static str`；
    - `pub fn parse(text: &str) -> Option<Effort>`：去掉两端空白、不分大小写，同 `ExecutorKind::parse`。
  - `pub enum LeanStatus { On, Off, Personal, HomeDirectory }`
    - derive 同上（不要 `Hash`），`#[serde(rename_all = "snake_case")]`；
    - `pub fn applies(self) -> bool`（只有 `On` 为真）。
  - `Binding.lean: bool`：`#[serde(default = "lean_by_default", skip_serializing_if = "is_lean_by_default")]`，缺省为真，只在为假时写出。
  - `Binding.effort: Option<Effort>`：`#[serde(default, skip_serializing_if = "Option::is_none")]`。
  - `Settings.default_effort: Option<Effort>`：`#[serde(skip_serializing_if = "Option::is_none")]`，`Default` 里为 `None`。
  - `Config::effort_for(&self, binding: &Binding) -> Option<Effort>`：绑定的值，否则设置的默认。
  - `Config::lean_for(&self, binding: &Binding, home: Option<&Path>) -> LeanStatus`，依次判断：
    1. `!binding.lean` → `Off`；
    2. `permission_for(binding) == PermissionProfile::Personal` → `Personal`；
    3. `home` 有值，并且两个目录 `std::fs::canonicalize` 后相同（规范化失败就按原样比较）→ `HomeDirectory`；
    4. 否则 `On`。

- [ ] **Step 1: 写失败的测试**（`config.rs` 测试模块）

```rust
#[test]
fn effort_names_parse_case_insensitively_and_round_trip() {
    for effort in Effort::ALL {
        assert_eq!(Effort::parse(&effort.as_str().to_uppercase()), Some(effort));
    }
    assert_eq!(Effort::parse(" max "), Some(Effort::Max));
    assert_eq!(Effort::parse("turbo"), None);
    let mut cfg = Config::default();
    cfg.bindings.push(Binding { effort: Some(Effort::Xhigh), ..binding() });
    assert!(toml::to_string_pretty(&cfg).unwrap().contains("effort = \"xhigh\""));
}

#[test]
fn an_unknown_effort_in_the_file_is_an_error() {
    // binding_toml() 加一行 effort = "turbo"，load_error() 返回错误
}

#[test]
fn a_binding_from_before_lean_runs_reads_as_lean() {
    // binding_toml() 不含 lean / effort：读出 lean == true、effort == None，settings.default_effort == None
}

#[test]
fn lean_is_written_only_when_off() {
    // lean 为真时保存的文本不含 "lean"；为假时含 "lean = false"；两种都读回相等
}

#[test]
fn effort_for_takes_the_binding_then_the_settings() {
    // (None, None) -> None；设置 High -> High；绑定 Max 加设置 High -> Max
}

#[test]
fn lean_for_is_off_personal_home_or_on() {
    // home = tempdir：目录就是 home -> HomeDirectory；别的目录 -> On；lean false -> Off；
    // 绑定 Personal -> Personal；设置默认 Personal、绑定不设 -> Personal；lean false 加 Personal -> Off
}
```

同时改 `every_key_a_save_writes_is_known`：样本绑定加 `lean: false, effort: Some(Effort::Max)`，`sample.settings.default_effort = Some(Effort::High)`；绑定键列表加 `"lean"`、`"effort"`，设置键加 `"default_effort"`。`unknown_keys` 里的样本（427 行附近）按同样的值设好。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p issuerelay-core config::tests`
Expected: 编译失败（`Effort`、`lean` 等不存在）。

- [ ] **Step 3: 在 `config.rs` 实现上面 Interfaces 列出的类型与方法，并补齐所有 `Binding` 字面量**

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --workspace`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(config): add lean runs and reasoning effort to bindings and settings"
```

---

### Task 3: 执行者参数：精简与推理力度

**Files:**
- Create: `crates/core/src/executor/lean.rs`（在 `executor/mod.rs` 里 `pub mod lean;`）
- Modify: `crates/core/src/executor/mod.rs`（模块文档、`CommandSpec`、`build`、测试）
- Modify: `crates/core/src/setup.rs:22-23`（删掉私有常量，改用 `lean.rs` 的）
- Modify: `crates/core/src/engine.rs:1359-1366`：先改成新签名，只传 `profile` 与 `model`，`effort: None, lean: None`，Task 6 再接上真实的值。

**Interfaces:**
- Consumes: `Effort`（Task 2）。
- Produces（`executor::lean`）：
  - `pub const CLAUDE_LINEAR_SERVER: &str = "linear-server";`
  - `pub const CODEX_LINEAR_SERVER: &str = "linear";`
  - `pub const DISABLE_AUTO_MEMORY: (&str, &str) = ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");`
  - `pub fn claude_args(user_instructions: Option<&Path>) -> Vec<String>`
  - `pub fn codex_args() -> Vec<String>`
  - `pub fn user_instructions_path(home: Option<&Path>, claude_config_dir: Option<&Path>) -> Option<PathBuf>`：有 `claude_config_dir` 时是 `<它>/CLAUDE.md`，否则是 `<home>/.claude/CLAUDE.md`；两者都没有为 `None`。不看文件存不存在。
  - `pub fn passable(path: &Path) -> bool`：路径是绝对的、是合法 UTF-8，并且不含控制字符与 `"`、`%`、`!`、`^`。这几个字符在 Windows 批处理里会被解释，而用户主目录几乎不会有。
- Produces（`executor`）：
  - `CommandSpec` 新增：
    - `pub env: Vec<(String, String)>`
    - `pub lean: bool`
    - `pub notes: Vec<String>`
  - `pub struct RunOptions<'a> { pub profile: PermissionProfile, pub model: Option<&'a str>, pub effort: Option<Effort>, pub lean: Option<Lean<'a>> }`
  - `pub struct Lean<'a> { pub user_instructions: Option<&'a Path> }`：只对 Claude 有用；`None` 表示不排除。
  - `pub fn build(kind: ExecutorKind, program: PathBuf, options: &RunOptions<'_>, cwd: &Path, prompt: String) -> CommandSpec`；`notes` 留空，由引擎填。

**参数顺序**（测试逐字核对）：

```text
Claude: -p --output-format stream-json --verbose --permission-prompts none --permission-mode <档位>
        [--model <模型>] [--effort <推理力度>]
        [--strict-mcp-config --mcp-config {"mcpServers":{"linear-server":{"type":"http","url":"https://mcp.linear.app/mcp"}}}
         --setting-sources project,local [--settings {"claudeMdExcludes":["<路径>"]}]]
Codex:  exec --json [<档位参数>] [--skip-git-repo-check]
        [--ignore-user-config --disable hooks --disable plugins --disable apps --disable memories
         --disable skill_mcp_dependency_install
         -c mcp_servers.linear.url=https://mcp.linear.app/mcp -c mcp_servers.linear.required=true
         -c mcp_servers.linear.startup_timeout_sec=30 -c skills.include_instructions=false]
        [-m <模型>] [-c model_reasoning_effort=<推理力度>] -
```

两段 JSON 都用 `serde_json` 生成，不手拼。`-c` 的键里用 `CODEX_LINEAR_SERVER`，网址用 `LINEAR_MCP_URL`，值不加引号。精简 Claude 运行的 `env` 为 `[DISABLE_AUTO_MEMORY]`，其他情形为空；`lean` 等于 `options.lean.is_some()`。

- [ ] **Step 1: 写失败的测试**（`executor/mod.rs` 与 `executor/lean.rs` 的测试模块）

先给现有测试加一个辅助函数 `options(profile, model)`（`effort: None, lean: None`），现有断言一律不改：完整配置下的参数逐字不变。再加：

```rust
#[test]
fn claude_lean_arguments_follow_the_design() {
    // lean 带 /srv/demo-home/.claude/CLAUDE.md：args 等于上面的 Claude 序列（档位 bypassPermissions，没有模型与推理力度），
    // 最后两项是 "--settings" 与 {"claudeMdExcludes":["/srv/demo-home/.claude/CLAUDE.md"]}
}

#[test]
fn claude_lean_without_a_passable_path_has_no_settings_argument() {}

#[test]
fn the_lean_mcp_config_names_only_the_linear_server() {
    // 解析 "--mcp-config" 后面那一项：mcpServers 只有 CLAUDE_LINEAR_SERVER 一个键，type 是 "http"，url 是 LINEAR_MCP_URL
}

#[test]
fn the_settings_argument_encodes_the_path_as_json() {
    // 路径 /srv/de mo/李四's\x/.claude/CLAUDE.md：解析后 claudeMdExcludes == [路径原文]
}

#[test]
fn lean_claude_runs_turn_auto_memory_off_and_full_runs_set_nothing() {
    // 精简 Claude：env == [("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")]，lean == true；
    // 完整配置的 Claude 与精简的 Codex：env 为空
}

#[test]
fn codex_lean_arguments_follow_the_design_and_keep_the_stdin_marker_last() {
    // 仓库目录、档位 Full、模型 gpt-5.5、推理力度 High：args 等于上面的 Codex 序列，最后一项是 "-"
}

#[test]
fn effort_reaches_both_executors_in_every_profile() {
    // Effort::ALL × 三档 × 精简开关：Claude 恰好一次 ["--effort", e.as_str()]；
    // Codex 恰好一次 ["-c", "model_reasoning_effort=<e>"]，并且最后一项是 "-"
}

#[test]
fn no_effort_means_no_effort_argument() {}
```

`args_never_contain_prompt_or_paths` 扩展：精简且带一个刁钻路径（含空格、中文、单引号）时，只有 `--settings` 后面那一项含路径；其余各项都不含路径的任何片段，也不含提示词。

`lean.rs` 里：
- `user_instructions_path_prefers_claude_config_dir`：`/opt/demo-claude` → `/opt/demo-claude/CLAUDE.md`；只有 home → `/srv/demo-home/.claude/CLAUDE.md`；都没有 → `None`。
- `passable_paths`：
  - 通过：`/srv/demo-home/.claude/CLAUDE.md`、`/srv/李 四/.claude/CLAUDE.md`、`/srv/o'brien/.claude/CLAUDE.md`；Windows 上还有 `D:\Profiles\Demo User\.claude\CLAUDE.md`。
  - 不通过：相对路径，以及含 `\n`、`"`、`%`、`!`、`^` 的路径。

Windows 专用（`#[cfg(windows)]`）的 `json_arguments_survive_an_npm_style_cmd_shim`：
- 在临时目录写 `echo-args.js`（`process.stdout.write(JSON.stringify(process.argv.slice(2)))`）和仿 npm 的 `echo-args.cmd`（`@"<node 的路径>" "%~dp0echo-args.js" %*`）。
- 用 `std::process::Command` 带着 `build` 生成的精简 Claude 参数（路径用 `D:\Profiles\Demo User\李\.claude\CLAUDE.md`）与精简 Codex 参数各跑一次，解析输出等于原参数。
- `which::which("node")` 找不到时打印一行说明后返回；CI 的 Windows 任务装了 Node 24。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p issuerelay-core executor::`
Expected: 编译失败（`RunOptions` 等不存在）。

- [ ] **Step 3: 实现 `lean.rs` 与 `build` 的新签名**

改 `executor/mod.rs` 的模块文档：可变参数除了校验过的模型名，还有固定枚举的推理力度，以及规则第三处例外允许的那一个路径。`setup.rs` 改用两个常量。`engine.rs` 改成新签名，见 Files。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --workspace`
Expected: 全部通过（`setup` 的现有测试照旧通过，它们仍写着原来的服务名字面量）。

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/executor crates/core/src/setup.rs crates/core/src/engine.rs
git commit -m "feat(executor): add lean run and reasoning effort arguments"
```

---

### Task 4: 运行时核对：精简没有生效就结束运行

**Files:**
- Modify: `crates/core/src/executor/lean.rs`（`LeanCheck`）
- Modify: `crates/core/src/runner.rs`：
  - `ExitKind`（37-48）；
  - 生成子进程的闭包，在 `env.apply(c)` 之后应用 `spec.env`；
  - 打开日志后先写 `notes`；
  - 读输出的循环（632-646）与 `record`（807-817）。
- Modify: `crates/core/src/engine.rs`：
  - `decide`（400-439）；
  - `finish_job` 里 Linear 读不到时的分支（1508-1519）；
  - 其余匹配 `ExitKind` 的地方编译通过即可。
- Test: `crates/core/tests/runner.rs`、`crates/core/tests/engine.rs`

**Interfaces:**
- Consumes: `CommandSpec.{env, lean, notes}`（Task 3）、`CLAUDE_LINEAR_SERVER`、`CODEX_LINEAR_SERVER`。
- Produces:
  - `pub struct LeanCheck`，`pub fn new(kind: ExecutorKind) -> Self`，`pub fn observe(&mut self, line: &str) -> Option<String>`；返回 `Some(说明)` 表示违反。
  - `ExitKind::LeanNotApplied(String)`。
  - `decide`：`ExitKind::LeanNotApplied(detail)` → `(JobState::Failed, Some(format!("lean run did not take effect: {detail}")))`。

**核对规则：**
- 不是 JSON 的行不看。
- Claude，在 `init` 之前：
  - `type` 不是 `system` 的事件 → ``a `<type>` event came before the init event``（缺 `type` 写 `unknown`）。
- Claude，`init` 事件（`type` 为 `system`、`subtype` 为 `init`）：
  - `mcp_servers` 不是数组 → ``the init event has no `mcp_servers` list``；
  - 数组不是恰好一项 `linear-server`（空数组也算）→ ``the init event lists the MCP servers <名字按字母排序、逗号分隔，空的写 none> instead of only `linear-server` ``；
  - 有 `memory_paths`，并且其中有非空的字符串值 → `auto memory is on`。
  - 核对过 `init` 以后不再看后面的行。
- Codex：
  - `item.started` 或 `item.completed` 里 `item.type` 为 `mcp_tool_call`、`item.server` 不是 `linear` → ``a call to the MCP server `<server>` ``。

**runner 的行为：**
- `spec.lean` 为真时建 `LeanCheck`。
- 每行标准输出先照常写进日志，再交给它。
- 第一次违反就 `break ExitKind::LeanNotApplied(说明)`；之后的拆除与用户点「停止」相同（`guard.end()`）。循环结束后排空的那几行照常记日志，不再核对。
- `notes` 每条按现有截断提示的写法记成 stderr 行，以 `[IssueRelay] ` 开头。（后来改为 `runner` 流、不带前缀，见末节「运行日志开头的说明」一行。）
- `spec.env` 在 `env.apply(c)` 之后设置，所以 `REMOVED_VARS` 不会把它去掉。
- `finish_job`：Linear 读不到时，结束方式为 `LeanNotApplied` 的运行仍记 `Failed`，原因保留精简那一句，不换成 `could not verify the result in Linear: …`。

- [ ] **Step 1: 写失败的测试**

`lean.rs`：

```rust
#[test] fn the_recorded_claude_stream_passes() {}          // tests/fixtures/claude-stream.jsonl 每一行都返回 None
#[test] fn the_recorded_codex_stream_passes() {}           // tests/fixtures/codex-exec.jsonl 每一行都返回 None
#[test] fn another_mcp_server_in_init_is_a_violation() {}  // linear-server 加 demo-tools -> 说明含 "demo-tools"
#[test] fn an_init_without_mcp_servers_is_a_violation() {}
#[test] fn a_memory_path_with_a_value_is_a_violation() {}  // {"memory_paths":{"auto":"/srv/demo-home/.claude/projects/x/memory/"}}
#[test] fn an_empty_memory_path_passes() {}                // {"memory_paths":{}}，以及 {"memory_paths":{"auto":""}}
#[test] fn hook_events_before_init_pass() {}               // system/hook_started、system/hook_response，再 init
#[test] fn an_assistant_event_before_init_is_a_violation() {}
#[test] fn lines_after_init_are_not_checked() {}
#[test] fn non_json_lines_are_ignored() {}
#[test] fn a_codex_call_to_another_server_is_a_violation() {} // item.started，server 为 "demo-tools"
```

`tests/runner.rs`（假执行者）：
- `a_lean_violation_ends_the_run_like_a_stop`：脚本先打一行带两个服务的 `init`，再 `sleep_ms` 30000；`spec.lean = true` → `exit` 匹配 `LeanNotApplied(_)`，10 秒内返回，日志里有那行 `init`。
- `a_full_run_is_not_checked`：同样的 `init`，`sleep_ms` 50，`exit` 0；`spec.lean = false` → `Exited(Some(0))`。
- `notes_come_first_in_the_log`：`notes = ["demo note"]` → 日志第一行含 `[IssueRelay] demo note`。（后来改为 `runner` 流的 `demo note`，同上。）
- `the_runs_own_variables_reach_the_executor`：`env = [("ISSUERELAY_DEMO_VAR", "1")]`，脚本用假执行者已有的 `{"print_env": "ISSUERELAY_DEMO_VAR"}` → 日志含 `ISSUERELAY_DEMO_VAR=1`；`env` 为空时是 `ISSUERELAY_DEMO_VAR=<unset>`。

`engine.rs` 的单元测试：
- `a_lean_violation_is_a_failure_with_its_reason`：`decide` 返回 `(Failed, "lean run did not take effect: x")`。

`tests/engine.rs`：
- `a_lean_violation_keeps_its_reason_when_linear_cannot_be_read`：`ScriptedRunner` 返回 `LeanNotApplied`，Linear 读需求失败 → 状态 `Failed`，原因以 `lean run did not take effect` 开头。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p issuerelay-core lean`
Expected: 编译失败（`LeanCheck`、`LeanNotApplied` 不存在）。

- [ ] **Step 3: 按上面的规则实现**

只在 runner 读标准输出的地方接 `LeanCheck`；`record` 改成同时交回写日志的结果和违反说明。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --workspace`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(runner): stop a lean run whose configuration did not take effect"
```

---

### Task 5: 环境检查：精简运行要的 Linear 服务与登录

**Files:**
- Modify: `crates/core/src/doctor.rs`：
  - `CommandRunner`（46-62）、`SystemRunner`；
  - `ExecutorReport`（289-309）、`check_executor_detailed`（331-419）；
  - `SystemReadiness`（767-912）、`ready_set`（925-931）、`BindingReport` 与 `check_bindings`（683-708）；
  - 测试里的假对象。
- Modify: `crates/core/src/trigger.rs:84-108`（`evaluate`）与它的测试。
- Modify: `crates/core/src/engine.rs`：
  - `readiness()`（883-886）；
  - `doctor()`（1783-1818）；
  - 调用 `evaluate` 与 `check_bindings` 的地方。
- Modify: `crates/core/src/host.rs:365-386`：建 `paths.data_dir.join("lean-check")`，传给 `SystemReadiness::new`。
- Test: `crates/core/tests/doctor.rs`、`crates/core/tests/host.rs`

**Interfaces:**
- Consumes: `LeanStatus`、`Config::lean_for`（Task 2），`CLAUDE_LINEAR_SERVER`、`CODEX_LINEAR_SERVER`（Task 3）。
- Produces:
  - `ExecutorReport` 新增 `pub lean_linear_mcp: bool`、`pub lean_logged_in: bool`、`pub lean_problems: Vec<String>`。
    - `pub fn ready_lean(&self) -> bool`：`ready()` 的前两项，加上这两个新结论。
  - `pub struct Readiness { pub full: HashSet<ExecutorKind>, pub lean: HashSet<ExecutorKind> }`
    - `pub fn ready(&self, kind: ExecutorKind, lean: LeanStatus) -> bool`：`lean.applies()` 时查 `lean`，否则查 `full`。
  - `pub fn ready_set(reports: &[ExecutorReport]) -> Readiness`
  - `pub fn mcp_list_has_lean_linear(kind: ExecutorKind, stdout: &str) -> bool`
    - Claude：行首的服务名（`: ` 之前）正好是 `linear-server`，网址那一项正好等于 `LINEAR_MCP_URL`，状态是 Connected。
    - Codex：第一列正好是 `linear`，网址正好，`enabled`，不含 `not logged in`。
  - `CommandRunner` 新方法 `async fn run_in(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<CmdOutput>`：默认实现忽略 `cwd` 调 `run`；`SystemRunner` 在 `cwd` 里运行，其余与 `run` 相同。
  - `SystemReadiness::new(ttl: Duration, env: SharedEnv, lean_check_dir: PathBuf)`，`with_refresh(…, lean_check_dir: PathBuf)`。
  - `pub fn codex_custom_provider(config_text: &str) -> Result<Option<String>>`：
    - 有 `profile = "<p>"` 时看 `profiles.<p>.model_provider`，否则看顶层 `model_provider`；
    - 值不是 `openai` 就返回它；读不懂的 TOML 返回 `Err`。
  - `BindingReport` 新增 `pub lean: LeanStatus`；`executor_ready` 按 `Readiness::ready(executor, lean)`。
  - `pub fn check_bindings(cfg: &Config, ready: &Readiness, home: Option<&Path>) -> Vec<BindingReport>`
  - `pub fn evaluate(issue: &Issue, cfg: &Config, me: &str, ready: &Readiness, home: Option<&Path>) -> Eligibility`：未就绪时仍是 `WaitReason::ExecutorNotReady(kind)`。

**检查的做法：**
- `check_executor_detailed` 同时跑四条命令：原来三条，加一条精简登录检查。
  - Claude：`run_in(program, ["--setting-sources", "project,local", "auth", "status"], lean_check_dir)`。
  - Codex：`run(program, ["-c", "cli_auth_credentials_store=file", "login", "status"])`。
- Claude 的 `lean_logged_in`：两份 `auth status` 的 JSON 都解析出 `loggedIn`、`authMethod`、`apiProvider`；精简那份已登录，并且后两项与平常那份相同才为真。
- Codex 的 `lean_logged_in`：
  - 精简那条已登录，并且登录方式（`Logged in using ` 之后的文字，去掉空白、转小写）与平常那条相同；
  - 并且 `codex_custom_provider` 读 `$CODEX_HOME/config.toml` 的结果是 `Ok(None)`。没有 `CODEX_HOME` 时读 `<主目录>/.codex/config.toml`；文件不存在算 `Ok(None)`。
- 检查目录建不起来时，`lean_logged_in` 为假，问题写 `could not prepare the folder for the lean sign-in check: <错误>`。

**`lean_problems` 的文字**（英文，与现有问题同一种写法）：
- Claude 缺精简服务：``lean runs use the Linear MCP server named `linear-server` in claude (register it for all directories with: claude mcp add --scope user --transport http linear-server https://mcp.linear.app/mcp)``
- Codex 缺精简服务：``lean runs use the Linear MCP server named `linear` in codex (register it with: codex mcp add linear --url https://mcp.linear.app/mcp)``
- 这个名字已被别的网址占用：``the Linear MCP server `linear-server` in claude points to <它的网址>, but lean runs need https://mcp.linear.app/mcp; change it yourself or turn lean runs off for this binding``。Codex 同句式；这时不给一键添加，因为添加会失败。
- 登录方式不同：`in lean runs claude would sign in differently (<平常的 authMethod> instead of <精简的 authMethod>); turn lean runs off for this binding`；Codex 同句式。
- 精简时没登录：`in lean runs <执行者> is not signed in (its sign-in comes from the user settings); turn lean runs off for this binding`
- 自定义模型服务：``codex uses the model provider `<p>`; lean runs would use the default provider, so turn lean runs off for this binding``

- [ ] **Step 1: 写失败的测试**

`doctor.rs`：
- `lean_linear_needs_the_exact_name_and_url`
  - Claude：`linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected` 为真。
  - Claude 为假：`linear: https://mcp.linear.app/mcp (HTTP) - ✔ Connected`、`claude.ai Linear: https://mcp.linear.app/mcp - ✔ Connected`、`linear-server: https://mcp.linear.app/sse (HTTP) - ✔ Connected`、`linear-server: … - ! Needs authentication`。
  - Codex：`linear  https://mcp.linear.app/mcp  -  enabled  OAuth` 为真；`linear-alt  https://mcp.linear.app/mcp  -  enabled  OAuth` 与 `… Not logged in` 为假。
- `claude_lean_login_must_match_the_usual_one`：
  - 两份都是 `{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}` → 为真。
  - 精简那份 `authMethod` 为 `api_key` → 为假，问题含 `would sign in differently`。
  - 精简那份 `loggedIn` 为假 → 为假。
- `the_claude_lean_login_check_runs_in_the_empty_folder`：假 runner 记下 `run_in` 的目录，等于传入的 `lean_check_dir`。
- `codex_lean_login_reads_the_file_store_and_must_match`。
- `a_custom_codex_model_provider_rules_out_lean_runs`：
  - `model_provider = "azure"` → `Some("azure")`；
  - `model_provider = "openai"` → `None`；
  - `profile = "work"` 加 `[profiles.work] model_provider = "demo"` → `Some("demo")`；
  - 空文本 → `None`；`model_provider = ` → `Err`。
- `check_executor_runs_its_commands_together`：`BarrierRunner::new(4, …)`。
- `ready_set_keeps_full_and_lean_apart`。
- `binding_reports_say_whether_lean_runs_apply`：
  - 精简绑定只有 `full` 就绪时 `executor_ready` 为假；
  - 「按个人设置」与主目录的绑定按 `full` 判定，并报 `Personal` / `HomeDirectory`。

`trigger.rs`：
- `lean_bindings_wait_for_the_lean_checks`
- `personal_and_home_bindings_use_the_full_checks`

`tests/host.rs`：
- `the_lean_check_folder_is_created_in_the_data_folder`

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p issuerelay-core doctor:: trigger::`
Expected: 编译失败。

- [ ] **Step 3: 按上面的接口与做法实现**

`engine.rs` 里调用 `evaluate` 与 `check_bindings` 时，主目录先传 `None`，Task 6 再接上 `EngineDeps.user_home`。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --workspace`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(doctor): check the Linear server and the sign-in lean runs need"
```

---

### Task 6: 引擎：按绑定派精简运行与推理力度

**Files:**
- Modify: `crates/core/src/engine.rs`：
  - `EngineDeps`（126-138）与 `Inner`；
  - `start_job`（1332-1366）；
  - 调用 `evaluate`、`check_bindings` 的地方，改用 `user_home`。
- Modify: `crates/core/src/host.rs`：填 `EngineDeps` 的两个新字段。
- Modify: `crates/core/tests/common/engine_kit.rs`：测试用的 `EngineDeps` 默认 `user_home: None, claude_config_dir: None`，提供设置它们的办法。
- Test: `crates/core/tests/engine.rs`

**Interfaces:**
- Consumes：
  - `Config::lean_for`、`Config::effort_for`（Task 2）；
  - `RunOptions`、`Lean`、`lean::user_instructions_path`、`lean::passable`（Task 3）；
  - `Readiness`、`check_bindings`、`evaluate`（Task 5）。
- Produces：
  - `EngineDeps` 新增 `pub user_home: Option<PathBuf>`、`pub claude_config_dir: Option<PathBuf>`。
  - 宿主填 `dirs::home_dir()`；`claude_config_dir` 取宿主环境里非空的 `CLAUDE_CONFIG_DIR`。

**`start_job` 的做法：**
- `lean = cfg.lean_for(&binding, user_home)`。
- 选项：`RunOptions { profile: cfg.permission_for(&binding), model, effort: cfg.effort_for(&binding), lean }`。精简适用时 `lean` 为 `Some(Lean { user_instructions })`，否则 `None`。
- `user_instructions` 是 `lean::user_instructions_path(user_home, claude_config_dir)`，再过一遍 `lean::passable`。
- 精简适用、执行者是 Claude，但没得到可传的路径时，往 `spec.notes` 加一条，之后的运行照常开始：
  - 有路径但不可传：`the user-level CLAUDE.md path cannot be passed safely, so this lean run still reads it`
  - 得不出路径：`could not tell where the user-level CLAUDE.md is, so this lean run may still read it`

- [ ] **Step 1: 写失败的测试**（`tests/engine.rs`，看 `ScriptedRunner.specs`）

```rust
#[tokio::test] async fn a_lean_claude_binding_starts_with_the_lean_arguments() {}
// spec.lean 为真；args 含 "--strict-mcp-config"；"--settings" 后面那项含 <user_home>/.claude/CLAUDE.md；
// env 含 CLAUDE_CODE_DISABLE_AUTO_MEMORY=1
#[tokio::test] async fn a_binding_with_lean_off_starts_with_the_full_configuration() {}
#[tokio::test] async fn a_personal_binding_never_runs_lean() {}
#[tokio::test] async fn a_binding_in_the_home_folder_never_runs_lean() {}
#[tokio::test] async fn the_binding_effort_wins_over_the_default() {}   // 设置 High、绑定 Max -> ["--effort", "max"]
#[tokio::test] async fn an_unpassable_home_leaves_a_note_and_no_settings_argument() {}
// user_home 是名字里带 % 的临时目录 -> 没有 "--settings"，notes 有一条
#[tokio::test] async fn a_codex_lean_binding_starts_with_the_codex_lean_arguments() {}
#[tokio::test] async fn claude_config_dir_moves_the_excluded_path() {}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p issuerelay-core --test engine lean`
Expected: 失败（还没有精简参数）或编译失败（`user_home` 不存在）。

- [ ] **Step 3: 按上面的做法实现**

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --workspace`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(engine): start runs with the binding's lean setting and effort"
```

---

### Task 7: 命令行：`bind add` 的两个选项与显示

**Files:**
- Modify: `crates/cli/src/main.rs`：`BindCommand::Add`（75-91）、解析函数（99-106）、`request()` 的 Add 分支（140-161）、测试（352-528）。
- Modify: `crates/cli/src/format.rs`：
  - `bindings`（289-311）；
  - `doctor` 的执行者行（185-196）与绑定行（229-247）；
  - 测试。

**Interfaces:**
- Consumes: `Effort::parse`、`LeanStatus`（Task 2），`ExecutorReport` 与 `BindingReport` 的新字段（Task 5）。
- Produces（用户看到的文字）：
  - `bind add` 新增两个选项：
    - `--lean <on|off>`：省略为开；只在 `off` 时发 `"lean": false`。
    - `--effort <low|medium|high|xhigh|max>`：省略不发。
  - 读不懂的值报错：`` expected `on` or `off`, got `<值>` ``，以及 ``expected one of `low`, `medium`, `high`, `xhigh` or `max`, got `<值>` ``。
  - `bind list`：默认的那一行不变；精简关掉时方括号里加 `, lean off`，设了推理力度时加 `, effort <值>`。例如 `p1  Demo Project  ->  demo  [claude, lean off, effort max]`。
  - `doctor` 的绑定行：报告里有 `lean` 时，执行者名后面加
    - ` (lean)`
    - ` (full configuration)`
    - ` (full configuration: personal settings)`
    - ` (full configuration: home folder)`

    例如 `Binding Demo Project -> claude (lean): ok`。没有这个字段（旧宿主）时不加。
  - `doctor` 的执行者行：报告里有 `lean_linear_mcp` 时，在原行之后另起一行 `<kind> (lean runs): ready`，或 `<kind> (lean runs): not ready (<lean_problems 用 "; " 连起来>)`。没有这个字段时不打印。

- [ ] **Step 1: 写失败的测试**
  - `bind_add_sends_lean_and_effort_only_when_given`：
    - 不给两个选项时，JSON 与现在逐字相同；
    - `--lean off --effort max` 时多 `"lean": false, "effort": "max"`。
  - `unknown_lean_and_effort_values_are_refused`
  - `binding_lines_name_lean_off_and_effort`
  - `doctor_binding_lines_say_whether_runs_are_lean`：四种状态各一行；没有字段时与现在相同。
  - `doctor_prints_the_lean_readiness_of_each_executor`

  现有测试（`every_subcommand_sends_its_method_and_params`、`a_binding_line_says_what_is_wrong`、`bindings_logs_and_simple_messages` 等）不改断言，必须照旧通过。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p issuerelay-cli`
Expected: 新测试失败。

- [ ] **Step 3: 实现**

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --workspace`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): set and show lean runs and reasoning effort"
```

---

### Task 8: 面板：精简开关、推理力度、环境检查与说明

**Files:**
- Modify: `app/src/lib/types.ts`（`Binding` 130-139、`Settings` 107-128、`ExecutorReport` 150-157、`BindingReport` 159-165）
- Modify: `app/src/lib/binding-labels.ts`（11-23）、`app/src/lib/settings-form.ts`（`Draft` 29-42、`draftOf` 47-66、`settingsOf` 114-140）
- Modify: `app/src/pages/BindingDialog.tsx`（状态 60-76、`build` 165-181、字段 263-325）
- Modify: `app/src/pages/Settings.tsx`（「默认」一组 612-639）
- Modify: `app/src/pages/Bindings.tsx`（`BindingCard` 213-263）、`app/src/pages/Doctor.tsx`、`app/src/pages/Help.tsx`
- Modify: `app/src/i18n/locales/{en,zh-CN,zh-TW,ja,ko}.json`
- Modify: `app/src/lib/mock-fixtures.ts`（`DEMO_BINDINGS` 45-56、`DEMO_DOCTOR` 161-180、`FRESH_DOCTOR` 183-189）
- Test: `app/src/pages/BindingDialog.test.tsx`、`Bindings.test.tsx`、`Settings.test.tsx`、`Doctor.test.tsx`、`Help.test.tsx`、`app/src/lib/settings-form.test.ts`
- 截图基线：`docs/gates/screenshots/m2/` 下的 bindings、settings、doctor 图，以及 `readme/` 下的两张

**Interfaces:**
- Consumes：Rust 一侧的字段名与取值（Task 2、5）。
- Produces（`types.ts`）：
  - `export type Effort = "low" | "medium" | "high" | "xhigh" | "max";`
  - `export const EFFORTS: readonly Effort[]`
  - `export type LeanStatus = "on" | "off" | "personal" | "home_directory";`
  - `Binding` 加 `lean?: boolean`（缺省表示开）、`effort?: Effort`。
  - `Settings` 加 `default_effort?: Effort`。
  - `ExecutorReport` 加 `lean_linear_mcp: boolean; lean_logged_in: boolean; lean_problems: string[]`。
  - `BindingReport` 加 `lean: LeanStatus`。
  - `bindingLabels` 加 `effort(level?: Effort): string`：
    - 有值时是 `effort.<值>`；
    - 没有值时是 `effort.default`，`{{name}}` 用设置里的默认档位；设置也没有时用 `effort.executor`；设置还没读到时用现有的 `unknown`。

**行为：**
- 绑定对话框：
  - 「精简运行」用 `Switch`，状态初值 `initial?.lean ?? true`。
  - 「推理力度」用 `Select`：「默认（…）」加五个档位，初值 `initial?.effort ?? DEFAULT`。
  - `build()` 在关掉时写 `lean: false`，选了档位时写 `effort`；开着、不选时两项都不写，所以保存的对象与现在一样。
  - 生效档位是「按个人设置」时，开关不可用，提示换成 `bindings.lean_personal_hint`，开关的值不变。
- 设置页「默认」一组加「默认推理力度」：`none`（`effort.executor`）加五个档位；选 `none` 时 `default_effort` 不出现在保存的对象里。
- 绑定卡片多两行：
  - 「精简运行」：有检查结果时用 `BindingReport.lean`，否则按 `binding.lean`，显示 `bindings.lean_state.<状态>`；
  - 「推理力度」：`labels.effort(binding.effort)`。
- 环境检查页：每个执行者多一节「精简运行」。
  - `program !== null && logged_in && lean_linear_mcp && lean_logged_in`（与 Rust 的 `ready_lean` 相同）时显示 `doctor.lean_ready`；
  - 否则显示翻译过的 `doctor.lean_not_ready` 标题，`lean_problems` 逐条作细节行。
- 使用说明页加一段：
  - 精简运行做什么；
  - 什么时候关掉：要用别的 MCP 服务、仓库的 `.codex/` 配置、用户级拒绝规则或自定义模型服务；
  - 推理力度在哪里设。

**文案：**五种语言都要。下面给英文与简体的建议，日、韩、繁体照意思翻：

| 键 | en | zh-CN |
|---|---|---|
| `bindings.lean` | Lean run | 精简运行 |
| `bindings.lean_hint` | Loads only Linear and the repository's own rules and settings: not your other MCP servers, plugins, hooks, auto memory or user-level instructions. | 只带 Linear 与仓库自己的规则和设置，不带你的其他 MCP 服务、插件、钩子、自动记忆和用户级指令。 |
| `bindings.lean_personal_hint` | Personal-settings runs always use your whole setup, so lean runs do not apply. | 「按个人设置」沿用你的全部配置，精简运行不适用。 |
| `bindings.lean_state.on` / `off` / `personal` / `home_directory` | On / Off / Not applicable (personal settings) / Not applicable (home folder) | 开 / 关 / 不适用（按个人设置）/ 不适用（主目录） |
| `bindings.effort` | Reasoning effort | 推理力度 |
| `bindings.effort_hint` | Lean runs do not read your own settings, so set the effort here. | 精简运行不读你自己的设置，推理力度要在这里设。 |
| `effort.low` … `effort.max` | Low / Medium / High / Extra high / Max | 低 / 中 / 高 / 很高 / 最高 |
| `effort.default` / `effort.executor` | Default ({{name}}) / The executor's own setting | 默认（{{name}}）/ 执行者自己的设置 |
| `settings.default_effort` / `settings.default_effort_hint` | Default reasoning effort / Used by bindings that do not set their own; when empty, each executor uses its own setting. | 默认推理力度 / 绑定没有另设时用它；不设就用执行者自己的设置。 |
| `doctor.lean_title` / `doctor.lean_ready` / `doctor.lean_not_ready` | Lean runs / Ready / Not ready | 精简运行 / 可用 / 不可用 |
| `help.lean_title` / `help.lean_body` | 按上面「使用说明页」三点写 | 同左 |

- [ ] **Step 1: 写失败的测试**

`BindingDialog.test.tsx`：
- `saving with lean on and no effort keeps the binding shape`：现有的「键正好是 directory、id、project_id、project_name」断言照旧成立。
- `turning lean off saves lean false`
- `editing keeps lean off and the effort`：初值 `lean: false, effort: "max"`，只改目录就保存 → 两项原样保存。
- `under personal settings the lean switch is disabled and explains why`
- `a chosen effort is saved and the default saves none`
- 对话框在 900×600 与 885×600 下的可见性测试照旧通过。

`Bindings.test.tsx`：
- 卡片显示四种精简状态与推理力度。
- 设置未知时「Default (Unknown)」的个数从 2 变成 3。

`Settings.test.tsx`、`settings-form.test.ts`：
- `default_effort` 可选可清；表单「每个字段都能保存」的测试把它列进去。

`Doctor.test.tsx`：
- 精简可用显示「可用」。
- 不可用时标题翻译过，`lean_problems` 作细节行。

`Help.test.tsx`：
- 有精简运行一节。

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm --dir app test`
Expected: 新测试失败。

- [ ] **Step 3: 实现，更新模拟数据**

`DEMO_BINDINGS` 的 proj-2 改为 `lean: false, effort: "high"`；`DEMO_DOCTOR` 里 Claude 精简可用，Codex 有一条 `lean_problems`；绑定报告加 `lean`。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm --dir app build && pnpm --dir app lint && pnpm --dir app test`
Expected: 全部通过（i18n 的键对齐测试同时通过）。

- [ ] **Step 5: 更新截图基线**

Run: `pnpm --dir app screenshots && pnpm --dir app readme-screenshots`
Expected: 只有 bindings、settings、doctor 与 README 的图有变化，逐张看一遍。

- [ ] **Step 6: Commit**

```bash
git add app docs/gates/screenshots
git commit -m "feat(app): lean runs and reasoning effort in the panel"
```

---

### Task 9: 文档

**Files:**
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`：
  - 6.1 的表加两行：「精简运行（默认开）」写两个执行者的参数，「推理力度」写两个执行者的写法；
  - 6.4 加精简运行的就绪条件；
  - 9.2 的「项目绑定」「设置」两行加新字段；
  - 第 14 节安全一条改为「默认精简，关掉时才加载全部配置」；
  - 第 18 节第 6 条改为已定，指向新设计。
- Modify: `docs/superpowers/specs/2026-10-19-issuerelay-lean-run-design.md`：第 6.3、7、8 节改成本计划开头三处细化的说法。
- Modify: `README.md`「安全须知」：替换「AI 用的是你本机…的全部配置」那一条，写清
  - 精简运行默认开，它做什么、不做什么（不是沙箱）；
  - 什么时候关掉；
  - Codex 仍读 `~/.codex/AGENTS.md`；
  - 用户级拒绝规则在精简运行里不生效；
  - 管理员下发了 MCP 配置的机器上，Claude 的精简运行会在启动时失败。
- Modify: `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 4 节「执行者」一行，加「精简运行的参数与精简运行设计第 5 节一致；精简绑定的运行时核对仍在」。

- [ ] **Step 1: 按上面改**

- [ ] **Step 2: 核对**

Run: `bash scripts/check-secrets.sh --strict`
Expected: 没有输出。

Run: `grep -n "全部配置" README.md`
Expected: 只剩「关掉精简运行才加载全部配置」这类说法。

- [ ] **Step 3: Commit**

```bash
git add README.md docs
git commit -m "docs: describe lean runs and the reasoning effort setting"
```

---

### Task 10: 全量验证、推送与 PR

- [ ] **Step 1: 全量验证**

Run:

```bash
pnpm --dir app install
pnpm --dir app build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir app lint
pnpm --dir app test
bash scripts/check-secrets.sh --strict
pnpm --dir app exec tsc -b --force --noEmit
```

Expected: 全部通过，记下 `cargo test` 与 Vitest 的测试数。

- [ ] **Step 2: 同步并推送**

`git fetch`。`origin/main` 前进了，就把还没推送的提交 rebase 上去，重跑 Step 1。

```bash
git push -u origin feat/lean-run
```

- [ ] **Step 3: 开 PR**

按 `.github/pull_request_template.md` 填写，写明：
- 风险级别：极高，执行者参数；
- 规则改动在单独的提交里；
- 测试数；
- Windows 待实机的各条；
- 实机验收在 Task 11 做，结果补进 PR。

正文末尾加 `🤖 Generated with [Claude Code](https://claude.com/claude-code)`。

- [ ] **Step 4: CI**

CI 三项（macOS、Windows、secrets）全绿。Windows 的 `.cmd` 往返测试在 CI 日志里是跑过的，不是「跳过」。

---

### Task 11: 沙箱实机验收与合并

**前提，先问用户，同意后才做：**
- 验收要退出正在运行的 IssueRelay（它也服务工作项目）。验收期间工作项目里不要有指派给自己、带 `ai-ready`、状态 Todo 的需求。
- 新的 ad hoc 构建第一次读钥匙串会要密码，由用户输入。打包的应用第一次打开有 Gatekeeper 提示：先说明是预期中的，再由用户自己双击打开。

**Files:**
- Create: `docs/gates/<日期>-lean-run-acceptance.md`（需求编号与名称脱敏为 `DEMO-N`、`Demo Project`）

- [ ] **Step 1: 用分支的代码起宿主**

`cargo run -p issuerelay-devhost -- run`，或按 M2 的做法打包应用。只在沙箱项目「IssueRelay Sandbox」与 `~/issuerelay-sandbox` 上建需求。

- [ ] **Step 2: 按设计第 11 节逐条做，记下结果**

1. 精简的 Claude 绑定跑一条需求，到「待验收」。逐项核对：
   - 运行日志的 `init` 里只有 `linear-server`，没有 `memory_paths`，插件只有内置的；
   - 钩子事件只有沙箱仓库自己的；
   - 会话存档的 `instructions` 附件里没有用户级 `CLAUDE.md`；
   - 自动记忆目录里没有新文件；
   - 沙箱仓库里没有插件写的目录。
2. 精简运行中途点「停止」，结束后没有脱离进程树的后台进程（M3 N3 的回归）。
3. 关掉精简的绑定跑一条需求，照常加载全部配置。
4. 精简的 Codex 绑定跑一条需求，到「待验收」。会话存档里没有插件与技能目录，`mcp_tool_call` 的服务都是 `linear`。
   - 4a：设置里设一个默认推理力度、给一个绑定另设一档，各跑一条：Claude 会话存档的 `effort`、Codex 的 `turn_context.effort` 与所设一致。
5. 「按个人设置」的绑定：面板显示精简不适用，运行照常加载全部配置。
6. 环境检查：两个执行者的精简运行都显示可用。
7. Windows：写「待实机」，清单是设计第 5.5 节两条与上面第 1、4 条。

- [ ] **Step 3: 恢复**

停掉开发宿主或测试用的应用，重新打开用户原来的 `~/Applications/IssueRelay.app`（有提示时由用户自己打开）。

- [ ] **Step 4: 写报告并提交**

```bash
git add docs/gates
git commit -m "docs(gates): record the lean run acceptance on the sandbox"
git push
```

- [ ] **Step 5: 合并**

CI 全绿后，以所有者权限经管理员绕过 squash 合并，并在 PR 里写明（设计 16.1）。

合并后升版本号、打标签、发布另问用户，不在本计划里。

## 评审遗留的小项

本计划执行中，Task 3–6 的逐任务评审与整条分支的最终评审接受、没有修复的项，留给所有者决定的事，以及还要在真实机器上确认的事。各评审的严重与应该修的问题都已修复并补了测试。SDD 工作区在合并后删除，这里是这些结论唯一的存放处；写后续计划或做验收时逐条认领，或写明不做及原因。

2026-10-20 的小项清理（PR #8）认领了「最终评审」与「逐任务评审」两张表里属于测试、注释与小改动的各行；每行的「认领」一栏写明做了什么，没做的写明原因；推送后做了一次独立评审，它找出的问题也在同一 PR 里修了。「留给所有者决定」与实机验收那张表没有动。

2026-10-21 的第二次清理（PR #15）按所有者当天的决定，认领了「最终评审」表剩下的三行和「逐任务评审」表的 Codex Auth 一行，并查了实机验收表的 N5；`memory_paths` 与面板一键修复两行，所有者确认维持不做。查 N5 引出的一个新问题（执行者检查失败时程序日志不记）与复审提出的一个（Claude 靠固定请求头连上的 Linear 服务）加进了「留给所有者决定」表，未定。

2026-10-21 v0.2.2 发布以后，所有者看过 v0.2.2 验收的新观察与这两行的建议，说「升级版本吧」：日志那一行照建议做，与验收报告的 O1、O4 和 O3 的文档提示一起随 v0.2.3；`linear-server` 那一行仍未定。

2026-10-22 v0.2.3 公开以后，所有者看过 v0.2.3 验收的新观察 P1–P4 与 `linear-server` 那一行的建议，定：P1、P3 照建议做，随 v0.2.4；P4 不改；`linear-server` 那一行维持现状并关闭。至此「留给所有者决定」表的各行都已定。

**留给所有者决定**

| 内容 | 建议 |
|---|---|
| Codex 的运行时核对只认 `item.started` / `item.completed` 里类型为 `mcp_tool_call` 的项目：Codex 改了事件名或改用 `item.updated`，核对就悄悄失效，违反第 8 节「读不懂就报错」的取向 | 精简的 Codex 运行从头到尾没有一条可辨认的 `linear` 调用，就判为「精简没有生效」。**已定（2026-10-20，所有者让主智能体决定）**：照建议做，只看正常退出的运行（以非 0 退出的照现有规则判定，例如撞到用量上限的 Codex），随 v0.2.1 |
| 精简没有生效时，按设计「像用户点停止」先给 SIGTERM 和宽限：只有 `--setting-sources` 也失效、用户插件真的载入时，SIGTERM 可能触发插件的会话结束钩子（M3 N3） | 直接 SIGKILL（Claude 在 `init` 时还没有工具进程），或维持现状并在设计第 8 节写明取舍。**已定（2026-10-20，所有者让主智能体决定）**：维持现状，取舍写进精简运行设计第 8 节：直接 SIGKILL 可能在 Codex 运行中途留下 git 锁文件之类的半成品 |
| 用户设置的 `env` 里只设了 `ANTHROPIC_BASE_URL`（公司网关），`authMethod` 与 `apiProvider` 却与平常相同时，精简登录检查看不出区别，精简运行会绕过网关直连（反目标里举的例子；Task 5 与最终评审都提到） | 环境检查另读用户设置（`~/.claude/settings.json`，设了 `CLAUDE_CONFIG_DIR` 时读那里）的 `env`，有 `ANTHROPIC_BASE_URL` 就判精简不可用。**已定（2026-10-20，所有者让主智能体决定）**：照建议做，代理变量也算（问题只写变量名，不写值），随 v0.2.1 |
| 用户设置的 `env` 里设了 `CLAUDE_CODE_EFFORT_LEVEL` 时，它盖过命令行的 `--effort`：不精简的运行读用户设置，IssueRelay 设的推理力度不生效，设计第 5.6 节「不精简时它会盖过用户设置里的推理力度」对这种设法不成立（实机验收 N2） | 在设计第 5.6 节、帮助与 README 里写明这个例外；IssueRelay 不改用户的设置。**已定（2026-10-20，所有者让主智能体决定）**：照建议只写进文档：精简运行设计第 5.6 节与 README（v0.2.0 的发布说明已写） |
| 需求要求先等一会儿时，无人值守运行会失败：放到后台的命令让 `-p` 会话提前结束，单独的前台 `sleep` 又被 Claude Code 拦下（实机验收 N4） | 提示词加一条「要等的命令放在前台，不要放到后台」（提示词固定规则属极高风险，要单独决定）；或者不改，在帮助里写明。**已定（2026-10-20，所有者让主智能体决定）**：不改提示词，写进 README 的安全须知（改固定规则要另做实机验证） |
| 开发宿主从 Claude Code 会话的终端启动时，执行者继承这个会话的环境变量（`CLAUDE_CODE_EFFORT_LEVEL`、`CLAUDE_CODE_CHILD_SESSION` 等）；`ExecEnv` 只去掉 `CLAUDECODE` 与 `CLAUDE_CODE_ENTRYPOINT`。托盘程序由 launchd 启动，不受影响（实机验收 N3） | 再去掉 Claude Code 会话自己的变量；`ANTHROPIC_BASE_URL` 一类可能是用户的网关，不能一概去掉。**已定（2026-10-20，所有者让主智能体决定）**：不改 `ExecEnv`，在测试规范「实机验收规则」写明用 `env -i` 启动开发宿主 |
| 执行者检查由就绪变成未就绪时，宿主的程序日志什么都不记（2026-10-21 查 N5 时发现：那一次的原因只在 `doctor` 的执行者几行里，验收时又被过滤掉了） | 在宿主日志里记一条带问题的警告，只在就绪与否变化时记，免得每次检查都记；或维持现状，`doctor` 的执行者几行已经写明原因。**已定（2026-10-21，所有者看过建议后说「升级版本吧」）**：照建议做，随 v0.2.3：执行者能接的运行变少时记警告（`executor no longer ready`，或 `executor no longer ready for lean runs`），带上问题；变多时记一行（`executor ready now`，只差精简时 `executor ready now, but not for lean runs`，带上精简问题）；没有变化不记，宿主启动后的第一次检查也不记（`doctor::log_readiness_change`） |
| Claude 的 `linear-server` 靠用户配置里固定的请求头连上、又没有存 OAuth 登录时，`mcp list` 照样显示 Connected，精简判为可用，精简运行里 Linear 应当会要求授权（设计第 7 节的已知限制，推测，未实测；复审 2026-10-21 提出） | 把运行开头的核对（设计第 8 节）改为要求 `linear-server` 的状态是 `connected`，在开头拦下并写明原因；或维持现状（第 8 节：状态不管，AI 照常报失败）。要先在沙箱上看清 `init` 里这时的状态。**已定（2026-10-22，所有者看过建议后）**：维持现状，这一行关闭。理由：这个情形是推测的（用户配置里靠固定请求头连 Linear，又没存 OAuth 登录）；现状下运行里的 AI 用不了 Linear，运行照样判为失败，AI 的摘要写明原因；实测要另起一个服务名让 Claude Code 去连，可能写进它在钥匙串里的登录记录；`init` 时的状态还可能是 `pending`，一律要求 `connected` 会拦下正常的运行。真遇到时再改 |

**最终评审接受的遗留项**

| 内容 | 认领 |
|---|---|
| 精简运行设计第 6.2 节说 `bind list` 与 `doctor` 的绑定行都写精简生效值与推理力度；实际 `bind list` 只能写 `lean off` 与 `effort <值>`（原始绑定分不出「不适用」），`doctor` 的绑定行有精简状态、没有推理力度（`BindingReport` 没有 `effort`） | 已做（PR #15，所有者 2026-10-21 定两条一起做）：`BindingReport` 加 `effort`（绑定自己的，否则设置的默认值；都没设时为 `null`），`doctor` 的绑定一行写 `, effort <值>`；第 6.2 节改成照实现写：`bind list` 只写与默认不同的设定，`doctor` 写精简的生效值与推理力度 |
| 第 8 节举的是「AI 的消息、工具调用或结果」，代码在 `init` 之前遇到任何不是 `system` 的事件都判违规（失败即关闭，与第 8 节第一句一致） | 已做（随 v0.2.1）：第 8 节改为「任何不是 `system` 的事件」，并记下实机验收里精简运行在 `init` 之前什么事件都没有 |
| 跑完的运行看不出当时是不是精简的：只有被拦下的运行在原因里写明 | 已做（PR #15，所有者 2026-10-21 定不等 N3）：每条精简运行的日志开头写一行 `runner` 流的固定说明，以 `lean run: ` 开头，Claude 与 Codex 各一句（`executor::lean::log_note`）；用户级 `CLAUDE.md` 传不过去时，那一行说明排在它后面。结构化原因仍留给 N3 |
| 精简绑定只差精简检查时，等待原因仍是通用的「执行者未就绪」，而环境检查页前三项全绿 | 已做（PR #15 里单独一个提交，所有者 2026-10-21 定）：`WaitReason::LeanNotReady`，面板代码 `lean_not_ready`（五种语言），运行记录、命令行与 MCP 控制服务写 `executor <名字> is not ready for lean runs`；平常检查也没过的仍是 `executor_not_ready` |
| `RunDetail.test.tsx` 没有 `lean run did not take effect: …` 原因的显示用例（走现有的原因细节行，行为应当正确） | 已做（PR #8）：`a lean run that did not take effect fails like any other run, the engine's words under the headline` |

**实机验收（2026-10-20）接受的遗留项**

| 内容 | 认领 |
|---|---|
| 用 `issuerelay bind add` 改完绑定后马上运行 `doctor`，两个绑定都报 `executor not ready`，几秒后再查就正常（实机验收 N5） | 查过（2026-10-21），与配置变更无关，没有改代码：`bind add` 只保存配置并发出 `EngineEvent::Config`（只有面板订阅），不碰就绪检查的缓存；`doctor` 每次都重新检查（`executor_reports(true)`），绑定一行用的就是同一份结果。当时（10-20 03:38:58Z）的输出只留了 `^Binding` 两行：一个按完整配置、一个按精简判定，都报未就绪，说明那一次 Claude 的平常检查（登录或 `mcp list`）本身失败了；原因在被滤掉的执行者几行里，宿主日志也不记执行者检查的失败。8 秒后开始的下一次检查全部就绪。下次遇到先看 `doctor` 未过滤的执行者几行或 `--json`；要不要在执行者检查由就绪变成未就绪时写一条程序日志，见「留给所有者决定」表的最后一行 |

**逐任务评审接受的遗留项**

| 内容 | 认领 |
|---|---|
| `lean::user_instructions_path` 的测试比的是 `PathBuf`，没有钉住最后进 JSON 的字符串（Windows 的分隔符） | 已做（PR #8）：`the_derived_path_goes_into_the_settings_json_with_the_platform_separator` 比的是进 `--settings` 的 JSON 字符串 |
| `passable` 放行的字符集在 Unix 上几乎没被钉住（让它也挡 `&`、`(`、`)` 测试照过），也没有非 UTF-8 路径不通过的用例 | 已做（PR #8）：`passable_paths` 加了 Windows 文件名可用、shim 原样传过去的标点（含 `&`、`(`、`)`）；`a_path_that_is_not_unicode_never_goes_in`（Unix 上非 UTF-8 的字节，Windows 上落单的代理项） |
| Windows 的 npm 式 shim 测试把 node 的绝对路径写进 `.cmd`，cmd 按 OEM 代码页读，node 路径含非 ASCII 字符时会误报失败 | 已做（PR #8）：npm 式 shim 改用 PATH 上的 `node`，与 npm 自己的 shim 一样先从 `PATHEXT` 去掉 `.JS`；这个测试只在 Windows 上跑，由 CI 核对 |
| `lean_args_hold_the_instructions_path_only_in_the_settings_argument` 与 `lean_claude_runs_turn_auto_memory_off_and_full_runs_set_nothing` 可以断言得更多（临时目录的根不出现在其余参数里；完整配置的 Codex 的 `env` 与 `lean`） | 已做（PR #8）：前者另查放 `tricky_dir` 的临时目录不出现在其余参数里；后者另查精简的 Claude 标为精简、完整配置的 Codex 不设变量也不标精简 |
| 注释：`lean.rs`、`doctor.rs` 里几条实测事实没写版本与日期（注释规范第 4 节）；`passable` 的理由可以写明那四个字符是给会重新解析参数的 shim 留的余量；`--mcp-config` 会吃多个值，所以后面必须紧跟一个选项；`doctor.rs` 有一行约 130 列的文档注释；`LeanCheck::done` 的注释写「seen and passed」，其实任何 `init` 都会设它 | 已做（PR #8）：两个文件的实测事实写了版本与日期（2026-10-19 的实测、2026-10-10 的 M0 样本、2026-10-20 的实机验收）；`passable` 写明那四个字符是余量；`--mcp-config` 那一项写明为什么后面要紧跟选项；`doctor.rs` 那行和同一 PR 在 `runner.rs` 的 `TreeGuard` 文档里留下的一行约 120 列都折回 100 列；`LeanCheck::done` 改成任何 `init` 都会设它 |
| 引擎测试 `a_lean_violation_keeps_its_reason_when_linear_cannot_be_read` 没有断言走的是读不到 Linear 的分支（`verified_updated_at == None`） | 已做（PR #8） |
| 精简没有生效时，最后一次读 Linear 的错误既不在原因里，也没写进程序日志 | 已做（PR #8）：带任务与需求编号的警告；测试 `a_lean_violation_whose_result_cannot_be_read_logs_why` |
| 测试没有钉住：精简停止时整棵进程树被结束（可比照 `an_escaped_grandchild_is_ended_on_cancel`）；`spec.env` 盖过 `ExecEnv::apply` 设的同名变量；`LeanCheck` 的缺 `type`、`result` 早于 `init`、Codex 缺 `server` 三种情形 | 已做（PR #8）：`an_escaped_grandchild_is_ended_when_lean_stops_the_run`（假执行者加了 `wait_for` 一步）、`the_runs_own_variables_win_over_the_executor_environment`、`an_event_without_a_type_before_init_is_a_violation`、`a_result_before_init_is_a_violation`、`a_codex_call_that_names_no_server_is_a_violation` |
| 排空阶段不再核对（计划的决定）：执行者退出时还在队列里的违规行只写进日志，Codex 的运行这时按现有规则判定 | 已做（PR #8）：照这个改；因别的原因停下的运行保留原来的结束方式。测试 `a_violation_read_after_the_executor_exited_still_fails_a_lean_run`（Unix；假执行者加了 `late_out` 一步：Windows 上运行器在退出后马上结束 Job Object，晚到的那个进程随之结束，造不出这种情形）；独立评审后另补两个分支：`a_clean_lean_run_whose_result_comes_after_the_exit_passes`（排空沿用主循环的核对，干净运行的结果在排空里不算违规）与 `the_violation_that_stopped_a_lean_run_is_the_one_it_keeps`（已被拦下的运行在排空里再读到违规，原因与说明都不变） |
| 运行日志开头的说明用 `stream: "stderr"`，面板时间线把它显示成执行者的标准错误；拦下时也没有一行 IssueRelay 自己的说明 | 已做（PR #8）：开头的说明改用 `runner` 流，去掉 `[IssueRelay] ` 前缀；拦下时补一行 `lean run did not take effect: <细节>; stopping the run`，排空时才判定的写 `…; the executor had already exited`；`AI_CODING_GUIDE.md` 第 6 节的运行日志一行同步 |
| `memory_paths` 里不是字符串的值（例如 `{"auto": true}`）会放行 | 若有执行者这样写再改。小项清理没做：这个条件还没出现。2026-10-21 所有者确认维持不做：收紧以后，执行者万一改用别的值表示关闭，会拦下所有精简运行 |
| Codex 的 `mcp list` 一行 Auth 栏是 `Bearer token` 或 `Unsupported` 时，精简的 Linear 判定也算可用；精简运行丢掉用户配置，只有已存的 OAuth 能用，`required=true` 会让运行一开始就退出 | 已做（PR #15，所有者 2026-10-21 定先读源码、实机核对后补）：按 codex-cli 0.159.2 的源码（2026-10-21 读），Auth 栏只在存着可用的 OAuth 登录时是 `OAuth`，配置里设了 token 时总是 `Bearer token`；精简判定改为要求 `OAuth`。精简问题写明这一栏显示什么：`Bearer token` 时建议删掉配置里的 token 再 `codex mcp login linear`（只登录、不删 token 解不开，独立评审发现），其他情形建议直接登录，或关掉精简；设了 token 又另存了 OAuth 登录的，判定偏保守。Claude 一侧不加 `(HTTP)`：Claude Code 按服务名保存登录（设计第 4.2 节），连接类型不影响；但 Claude 有同类的已知限制（靠固定请求头连上、没存 OAuth 登录时 `mcp list` 照样 Connected），写进设计第 7 节，没有改。可能的后续：让 Codex 的这项检查用精简运行自己的配置跑 `mcp list`，Auth 栏就等于精简运行实际看到的（要先在沙箱上核对能不能这样跑）。实机核对见下面「待实机的检查」 |
| Codex 用 API key 登录时 `login status` 可能印出遮罩过的 key（`Logged in using an API key - <遮罩>`，仓库里没有样本），它会进精简问题的文字 | 已做（PR #8）：codex-cli 0.159.2 的源码印的是前 8 位与后 5 位；精简问题只写开头的词（到第一个既不是字母也不是空格的字符为止，所以 ` - ` 之后的遮罩 key 不写，以后换了印法也不写；什么都不剩时写 `an unknown method`），两次登录仍整段比较，换了 key 的写 `would sign in with an api key, but not the one usual runs use`。测试 `a_lean_problem_names_a_codex_api_key_login_without_the_key`、`a_codex_sign_in_is_shown_by_the_words_it_starts_with` |
| 几条裁定没有测试钉住：`mcp list` 崩溃时不加精简问题；平常登录为否时不做精简登录比较；`lean_linear_needs_the_exact_name_and_url` 没有 `plugin:` 前缀的反例；`codex_custom_providers_are_found` 没有「顶层与档案都设了」的情形 | 已做（PR #8）：`a_crashed_mcp_list_adds_no_lean_problem`、`a_usual_login_that_says_logged_out_is_not_compared_for_lean_runs`，另两个测试各补了反例与情形 |
| 面板的精简一节没有一键修复按钮：报告分不清「缺这个服务」与「这个名字指向别的网址」 | 要做时给 `ExecutorReport` 加一个结构化的精简 Linear 结论。小项清理没做：这是新功能，留给所有者。2026-10-21 所有者确认这次不做 |

**待实机的检查**

- 精简运行设计第 11 节在 macOS 上的各条已在 2026-10-20 做完，见 `docs/gates/2026-10-20-lean-run-acceptance.md`：包括 `claudeMdExcludes` 的排除在 macOS 上生效，`lean-check/` 里的精简登录检查不受上层目录的项目设置影响。
- 还要在 Windows 上做：设计第 5.5 节与第 11 节第 7 项，包括 `claudeMdExcludes` 的排除（glob 字符、Windows 反斜杠），以及 Windows 上 `dirs::home_dir()` 与 Node 的 `os.homedir()`（先看 `USERPROFILE`）是否一致。
- `~/.claude` 是符号链接时的排除：验收那台机器不是符号链接，没有测。
- 已核对：`1b45f15` 的 CI（Windows）日志里，`.cmd` 往返测试 `json_arguments_survive_npm_and_pnpm_style_cmd_shims` 是 `ok`，跑过，没有跳过。
- PR #15 的实机核对已在 2026-10-21 做完，见 `docs/gates/2026-10-21-lean-leftovers-acceptance.md`，在 macOS 沙箱上都看到了：精简运行日志开头的那一行（Claude、Codex 各一次，用 `issuerelay logs` 看）；只差精简检查时需求记为等待，原因是 `executor codex is not ready for lean runs`（命令行与 MCP 控制服务）；Codex 的 Auth 栏：配置里设了 bearer token 时出现新的精简问题，照它的建议删掉 token、`codex mcp login linear` 以后问题消失，只有 OAuth 登录时精简可用；`doctor` 绑定一行的 `, effort <值>`。面板需求列表里的那一行没有看：开发宿主没有面板，所有者定用命令行代替。反面情形用一个临时的 `CODEX_HOME` 造（`mcp_oauth_credentials_store = "file"`），没有改用户的执行者配置；在这样的 `CODEX_HOME` 下不要运行 `codex mcp logout`，它会删掉各个 `CODEX_HOME` 共用的钥匙串登录（报告的「环境」一节）。
- v0.2.3 的改动（2026-10-21）：只差精简检查时 `doctor` 与面板的绑定一行写明只差精简运行；`bind list` 写绑定自己的权限档；执行者能接的运行变化时宿主日志记一行。已在实机上看到的（2026-10-22 发布 v0.2.3 时，见 `docs/gates/2026-10-22-v0.2.3-release.md`）：`bind list` 列出沙箱绑定 `[claude, permission auto]`（分支编出的命令行对 0.2.2 托盘程序，以及从草稿装上的 0.2.3）；0.2.3 的 `doctor` 全部就绪，精简的 Claude 冒烟运行（DEMO-47）走到待验收，宿主启动后没有多余的就绪日志。其余几项 2026-10-22 公开后在已装的 0.2.3 上补做了，都看到了（同一报告「公开后的实机核对」）：绑定自己设 `--permission personal` 时 `bind list` 显示 `[claude, permission personal]`；只差精简检查时绑定一行是 `executor not ready for lean runs`，宿主日志有 `executor no longer ready for lean runs`，修好后有 `executor ready now`。只差精简的情形改用 Claude 造：把数据目录里空的 `lean-check` 文件夹换成同名的文件，Claude 的精简登录检查就不过，不用新登录，也不改用户的配置；没有用临时的 `CODEX_HOME`。没有看面板里绑定的那一行；`executor ready now, but not for lean runs` 没有出现。
