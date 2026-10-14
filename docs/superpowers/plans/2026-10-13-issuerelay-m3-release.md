# IssueRelay M3：安装与发布 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让同事不用命令行就能装上 IssueRelay：推送 `vX.Y.Z` 标签时，GitHub Actions 在 macOS 与 Windows 上测试、打出 macOS 通用 `.dmg` 与 Windows NSIS `.exe`、`.msi`（都带命令行程序 `issuerelay`），放进**一个**草稿 Release，由维护者确认后公开；程序能检查并安装签过名的更新；设置里能不用管理员权限把命令行加入 PATH；安装说明写清未公证、未签名的包怎样放行。

**Architecture:** 打包配置分两层：所有构建都读的 `app/src-tauri/tauri.conf.json`（打包目标、macOS ad-hoc 签名、NSIS 按用户安装、更新插件配置，不写 `version`，版本取工作区 `Cargo.toml`），和只在打包时用 `--config` 合并的 `tauri.bundle.conf.json` / `tauri.updater.conf.json`（`externalBin`，后者再打开更新包），所以 `tauri dev` 与 `ci.yml` 不需要先放好命令行程序。命令行程序由 `scripts/stage-cli.{sh,ps1}` 编译后放成 `app/src-tauri/binaries/issuerelay-<三元组>[.exe]`，Tauri 去掉后缀装在主程序 `issuerelay-app` 旁边；打包前 `scripts/release-checks.mjs panel` 确认要内嵌的 `app/dist` 是真正的面板，不是 M2 `build.rs` 的占位页。「把命令行加入 PATH」的判断全是 core `cli_path` 模块里的纯函数（macOS：`~/.local/bin` 里的符号链接；Windows：`HKCU\Environment\Path`），面板经三条新命令调用；`issuerelay start` 拉起托盘程序，MCP 服务从不拉起。自动更新用 `tauri-plugin-updater`：`update.rs` 里可测试的 `Updates<S: UpdateSource>` 管检查、下载验签与「每个版本只通知一次」，应用一侧的函数与 M2 的退出代码一样对 `R: tauri::Runtime` 泛型；「安装并重启」先下载并验签，再交给 M2 的退出状态机（`quit::request_update`），有任务在跑时问同一个「等待完成 / 立即停止」，宿主停下后进入 `QuitState::Installing`，装完（或装不上）才重启，其间关窗或再次退出都不结束进程。发布工作流 `release.yml`：`verify`（门禁与密钥扫描）→ `test` → 正式发布走 `create-release`（只建一个草稿）→ `bundle-release` → `verify-release`，只有这三个任务有写权限；手动运行与改了打包配置的 PR 是试打包，走只读的 `bundle-dry-run` → `verify-dry-run`。两种打包共用可复用工作流 `bundle.yml`（矩阵逐个运行，`tauri-apps/tauri-action` v1.0.0 按提交固定）；发布门禁（标签等于版本、标签提交在 `origin/main` 上、资产齐全、`latest.json` 的平台键、签名与地址）都是 `scripts/release-checks.mjs` 里带自测的检查，试打包对将要打的标签 `v<版本>` 与试打包产物跑同样的检查，没有哪道门第一次运行就在正式标签上。

**Tech Stack:** Tauri 2.12 bundler（dmg、msi、nsis）、`tauri-plugin-updater` 2.13、`semver`、`winreg` 与 `windows-sys`（用户 PATH）、GitHub Actions（`tauri-apps/tauri-action` v1.0.0、`pnpm/action-setup` v6.1.0、`actions/setup-node` v7、`Swatinem/rust-cache` v2.9.2、`actions/upload-artifact` v7、`actions/download-artifact` v8、actionlint）、`lipo`、Node 24（签名核对与发布门禁脚本）。

**Spec:** `docs/superpowers/specs/2026-10-10-issuerelay-design.md` 第 2.1、2.2、4.1、9.5、10、12、16、16.1、17、18 节；依赖 M2 第一部分 `docs/superpowers/plans/2026-10-13-issuerelay-m2-core.md` 与第二部分 `docs/superpowers/plans/2026-10-13-issuerelay-m2-app.md` 产出的接口（各任务的 Interfaces 写明用到的名字，以 M2 落地的代码为准）。技术调研（2026-10-13，带出处）只在本机 `.superpowers/sdd/2026-10-13-issuerelay-m3-release/research.md`，不进仓库；本计划用到的结论都已写进正文，执行时不必另读。

## Global Constraints

- M2 两部分的全部全局约束继续有效：硬规则、两个平台同等对待（每个 `#[cfg(unix)]` 都有 `#[cfg(windows)]` 实现）、测试不碰网络 / 真实钥匙串 / 真实注册表、虚构测试数据、公开仓库红线、英文注释中文文档、六条验证命令（`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`pnpm --dir app lint`、`pnpm --dir app test`、`bash scripts/check-secrets.sh --strict` 无输出；前面先跑 `pnpm --dir app build`）、英文 Conventional Commits 与写实际模型的 `Co-Authored-By`、分支与 PR、禁止强推。
- 分支 `feat/m3-release`，从合入 M2 第二部分之后的 `main` 切出，经 PR 合入。
- 计划里的 Rust 代码块不一定是 rustfmt 的排版：每次提交前先 `cargo fmt --all`，再跑六条验证命令。
- 用 M2 落地的真实名字与签名（2ebda38）：退出流程（`quit::{handle, request, on_run_event}`）对 `R: tauri::Runtime` 泛型，测试在 `tauri::test` 的 MockRuntime 上跑；M3 新加的 `update::*` 与改写的 `quit::{request, request_update, ask}` 同样泛型。前端的变更钩子照 `queries.ts` 里的 `useAction<Variables, Result>(run: (backend, variables) => Promise<Result>, refreshes, failureKey, options?)` 写在它旁边。M3 改 M2 的文件一律是编辑，不整份替换：M2 的测试一条不删，只按新的载荷或文案改写。
- 面板命令：每加一条命令，同一步里同时改 `commands.rs`、`lib.rs` 的 `generate_handler!`、`app/src-tauri/build.rs` 的 `PANEL_COMMANDS`、`app/src-tauri/capabilities/default.json` 的 `allow-<命令>`（下划线写成连字符），以及 `backend.ts`、`api.ts`、`mock.ts`；`lib.rs` 的 `the_panel_is_granted_exactly_the_registered_commands`、`the_acl_the_build_resolves_allows_only_the_panels_commands` 与 `api.test.ts` 的边界测试照常通过（AI_CODING_GUIDE 第 7 节「面板命令」）。M2 有 34 条命令，Task 2 之后 37 条，Task 3 之后 40 条。
- 版本号只有一个来源：工作区 `Cargo.toml` 的 `[workspace.package] version`。`tauri.conf.json` 不写 `version`（Tauri 取 crate 版本，`issuerelay-app` 用 `version.workspace = true`，且在工作区 `members` 里，tauri-action 才找得到版本）。标签为 `v` 加这个版本，发布工作流核对，不一致就失败。版本只用 `X.Y.Z`：MSI 不接受 `-rc.1` 这类非纯数字的预发布后缀。只由维护者在发布时升（设计 12、16.1）：升版本是一个普通的 PR 提交（工作区 `version`，加上 `cargo update -w` 更新的 `Cargo.lock`），合入 `main` 之后才打标签。验收里的试打包用当前版本；任何步骤都不运行 `git checkout Cargo.toml Cargo.lock`（会丢掉别人对这两个文件的未提交改动）；本机更新测试需要更高版本时，在 `mktemp -d` 下的一次性 worktree 里改，从不改主检出。
- 可执行文件名：主程序到处都是 `issuerelay-app`（Windows `issuerelay-app.exe`，M2 的 `mainBinaryName`），没有 `IssueRelay.exe`；命令行程序 `issuerelay`（`issuerelay.exe`）装在它旁边：macOS `IssueRelay.app/Contents/MacOS/issuerelay`，Windows 安装目录（NSIS 按用户安装默认 `%LOCALAPPDATA%\IssueRelay`）下的 `issuerelay.exe`。主程序名永远不能与 `issuerelay` 只差大小写（macOS 的 APFS 与 Windows 的 NTFS 不分大小写，两份文件会静默互相覆盖）。
- `externalBin` 只写在打包专用的配置里；`tauri.conf.json` 本身必须在没有放好命令行程序时也能 `cargo build` / `cargo clippy` / `cargo test`（Task 1 的配置测试守住这一条）。`app/src-tauri/binaries/` 是生成目录，git 忽略。打包之前先 `pnpm --dir app build`，再用 `node scripts/release-checks.mjs panel` 确认 `app/dist/index.html` 存在且不是 `build.rs` 的占位页（本机与 CI 都这样）。
- 签名：没有 Apple 证书时 macOS 包一律 ad-hoc 签名（`bundle.macOS.signingIdentity: "-"`）——未签名的 arm64 应用缺少封存的资源，下载后会被报「已损坏」；有 Apple 证书（secrets 里六项都在）时改用正式签名并公证。Windows 没有证书时不签名。证书、私钥、密码只放在 GitHub Actions secrets，只经步骤级 `env:` 交给用到它们的那几步，从不写进 `$GITHUB_ENV`；拉取请求触发的试打包拿不到任何 secret。仓库、日志、提交信息、聊天里都不能出现它们。
- 发布工作流的卫生：每个任务都有 `timeout-minutes` 和如实说明的注释（照 `ci.yml`；调用可复用工作流的任务不能写它，上限写在 `bundle.yml` 的任务里）；试打包只读（`contents: read`），只有标签触发的 `create-release`、`bundle-release`、`verify-release` 有 `contents: write`；外来值经 `--arg` 交给 jq，不拼进过滤器；临时目录一律 `mktemp -d` 建、用完删；shell 变量不跨步骤、不跨任务，每一步按同样的办法重新求出；`ci.yml` 用 actionlint 检查全部工作流。
- 自动更新：清单地址固定为 `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json`；只安装比当前版本新、且用仓库里那把公钥验签通过（并且签名里记的版本与清单一致，`requireSignedVersion: true`）的更新包；检查失败、清单缺本平台、验签失败、版本不新，都只提示，不安装，不影响监控；安装一律由用户点「安装并重启」，有任务在跑时走与退出相同的询问；安装期间关窗或再次退出都不结束进程。`tauri.conf.json` 与 `tauri.*.conf.json`（`plugins.updater.pubkey` 是更新的信任根）列进 `.github/CODEOWNERS`。
- 通知：系统通知仍是设计 9.5 的四种运行通知，各自可关；「有新版本」的通知不另设开关，只在设置里「自动检查更新」打开时由后台检查发出（每个版本一次），沿用 `notify.rs` 的翻译与发送；安装失败只在面板里显示，不发通知。
- 更新签名密钥：由主智能体在维护者本机生成（不交给子智能体，不在云端会话做），不设密码，私钥存 `~/.tauri/issuerelay.key`（权限 600），从不打印、不读进聊天、不提交、不进日志；只提交公钥。私钥由维护者自己用 `gh secret set` 上传。
- 发布工作流只建草稿 Release；公开由维护者在 GitHub 页面上点（对外动作）。任何一个平台没打出来、或 `latest.json` 缺平台，工作流失败并把草稿标成「incomplete: do not publish」，不会出现被当成完整版本的半套资产。
- 依赖版本（调研时的最新版，执行时用 `cargo add` / `pnpm add` 解析后锁定）：crate `tauri` 2.12.x、`tauri-plugin-updater` 2.13.x、`semver` 1.x；npm `@tauri-apps/cli` 不低于 2.12.0（从这版起签名里记录应用版本）。插件与 Tauri 的 API 名称以解析到的版本文档（docs.rs、v2.tauri.app）为准，与本计划写法不同时照文档写，并在报告里列出差异。
- `release.yml` 与 `bundle.yml` 里第三方的 action 一律按提交 SHA 固定并在行尾注明版本（`# v1.0.0`），GitHub 自己的 action（`actions/*`）用主版本标签，与 `ci.yml` 一致；actionlint 用作者发布的容器镜像并写明版本号（执行时核对最新发布）；Dependabot 的 github-actions 条目照常更新它们。
- 提交：每个提交步骤先 `git status --short`，`git add` 只加列出的路径（不 `git add docs`、`git add app` 这类整个目录），再 `git diff --cached --stat` 确认只有这些文件，提交信息结尾带写实际参与模型的 `Co-Authored-By` 行（下文写作 `Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>`，执行时换成真实模型名）。规则改动（在 `docs/ai-coding/`、`MAINTAINERS.md`、`CONTRIBUTING.md`、`AGENTS.md` 里新增、收紧或放宽规则）单独一个只含这些文件的 `docs` 提交（AI_CODING_GUIDE 第 11 节）；代码顺带的事实登记（目录表、关键文件索引、运行数据表、共用用例表与假实现清单、CODEOWNERS）跟代码同一个提交。
- 面板布局：新加的行（PATH 一行、更新横幅、版本与检查更新一行）写明换行规则，在 620 px 与 605 px 两个内容宽度下各量一遍（浏览器视口宽 900 px 与 885 px，第 12.7 节），结果写进提交说明；M2 Task 12 的 Playwright 布局测试若已落地，把新行加进去。
- Windows：没有 Windows 机器（设计 18.4）。Windows 代码必须在 CI 上编译通过、纯逻辑测试通过；注册表读写不在测试里碰真实注册表，Windows 实机行为一律记「待实机」。
- 测试、文档、示例里不出现本机路径：不写用户目录（`/Users/…`、`/home/…`、`C:\Users\…`）或外置卷（`/Volumes/…`）下的具体路径，`check-secrets.sh` 会拦下它们；家目录用 `/tmp/demo-home`，Windows 目录用 `D:\Profiles\demo` 或 `%LOCALAPPDATA%`。

## Review Focus

1. 有任务在跑时点「安装更新并重启」：要走与退出相同的询问（等待完成 / 立即停止，等待中仍可改为立即停止），不能直接重启丢掉运行；宿主停下后安装期间关窗或再次退出都不结束进程；等待中从托盘退出会丢掉已下载的更新，询问框随之改说「之后退出」；点「继续运行」后再从托盘退出，只退出、不装更新（Task 3：`quit.rs` 的 `the_quit_prompt_tells_the_panel_what_follows`、`a_verified_update_is_installed_where_quitting_would_exit`、`the_process_outlives_the_install_and_then_restarts` 与扩展到 `Installing` 的 `every_exit_request_before_quitting_is_done_is_held`，`update.rs` 的 `a_verified_download_waits_for_the_quit_flow_and_is_reused`，前端 `QuitDialog` 与 `UpdateBanner` 测试；Task 6 第 5 步实机）。
2. 更新清单拿不到（离线、代理、还没发布过）、清单里没有本平台、验签失败、版本不比当前新（含预发布与写坏的版本号）：只提示，不安装，不影响监控；后台检查失败不收回已经提示过的新版本；宿主停下后才装不上的更新在重启后由面板说明（Task 3：`update.rs` 的 `only_a_newer_version_is_offered`、`a_failed_check_only_informs`、`a_bad_signature_keeps_nothing_to_install`、`a_failed_check_keeps_the_version_already_offered`、`a_failed_install_is_shown_once_after_the_restart`；Task 6 第 6 步实机）。
3. 标签与 `Cargo.toml` 版本不一致、标签提交不在 `origin/main` 上、同一标签已有 Release、secrets 缺失、某个平台没打出来、`latest.json` 缺平台、缺签名或地址不指向本 Release 的资产、签名私钥与提交的公钥不配对、内嵌的是占位页：工作流明确失败或明确降级，草稿不会带着半套资产被当成完整版本；这些门禁在试打包里对将要打的标签与试打包产物同样运行（Task 1、4：`scripts/release-checks.mjs` 的自测含版本不符、提交不在 main 上、缺资产、缺平台键、地址不是资产、占位页；`verify`、`create-release` 的重复检查、`bundle.yml` 的包内容检查与签名核对、`verify-release` / `verify-dry-run`；Task 4 第 8 步的试运行）。
4. 用户没有管理员权限、`~/.local/bin` 不存在或不在 PATH、那里已有别人的同名文件、应用从磁盘映像或临时副本运行、Windows 用户 PATH 是带 `%VAR%` 的 `REG_EXPAND_SZ` 或已经有这一项（大小写或结尾反斜杠不同）：不需要管理员、不覆盖、不重复、不改坏其他条目，移除时只删自己这一项（Task 2：core `cli_path` 的纯函数测试与 macOS 链接读写测试）。
5. 托盘程序没运行时调用命令行（包括 AI 经 MCP 调用）：命令行说清楚并指向 `issuerelay start`；`issuerelay start` 把托盘程序拉起来（已在运行就只说明），等到它应答；MCP 服务只报错、不启动，也不叫 AI 自己去启动（避免 AI 静默启动有完全权限的程序）（Task 2：命令行 `start` 与提示的测试、`crates/cli/tests/mcp_tools.rs` 的断言）。

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `app/src-tauri/tauri.conf.json` | 所有构建都读的配置：打包目标、ad-hoc 签名、最低 macOS 13.3、NSIS 按用户安装、去掉 `version`；Task 3 加 `plugins.updater` | 1、3 |
| `app/src-tauri/tauri.bundle.conf.json`、`app/src-tauri/tauri.updater.conf.json` | 只在打包时 `--config` 合并：`externalBin`；后者另打开 `createUpdaterArtifacts` | 1、3 |
| `app/src-tauri/tests/packaging.rs` | 配置守卫测试（基础配置不含 `externalBin`、主程序名、版本来源、最低系统版本、更新配置） | 1、3 |
| `scripts/stage-cli.sh`、`scripts/stage-cli.ps1` | 编译命令行程序并放成 `binaries/issuerelay-<三元组>[.exe]`；`--universal` 时三份 | 1 |
| `scripts/release-checks.mjs` | 发布门禁（带自测）：内嵌的面板不是占位页（Task 1）；标签与版本、标签提交在 `origin/main` 上、资产齐全、`latest.json`（Task 4） | 1、4 |
| `.gitignore` | `app/src-tauri/binaries/` | 1 |
| `crates/core/src/paths.rs` | `paths::APP_EXECUTABLE`（主程序文件名，配置测试与 `issuerelay start` 共用） | 1 |
| `crates/core/src/cli_path.rs`、`crates/core/Cargo.toml` | 应用安装位置判断、「把命令行加入 PATH」的纯逻辑与 macOS 链接 / Windows 注册表读写 | 2 |
| `app/src-tauri/src/commands.rs`、`app/src-tauri/src/lib.rs`、`app/src-tauri/build.rs`、`app/src-tauri/capabilities/default.json` | 新命令及其登记：`cli_on_path`、`install_cli_on_path`、`remove_cli_from_path`（Task 2）；`update_status`、`check_update`、`install_update`（Task 3） | 2、3 |
| `crates/cli/src/main.rs`、`crates/cli/src/mcp.rs`、`crates/cli/tests/mcp_tools.rs` | `issuerelay start`、没运行时的提示；MCP 只报错不启动 | 2 |
| `app/src-tauri/src/update.rs`、`app/src-tauri/src/quit.rs`、`app/src-tauri/src/events.rs`、`app/src-tauri/src/notify.rs`、`app/src-tauri/src/i18n.rs` | 自动更新；退出状态机接上「安装并重启」（`Installing`）；`update-status` 事件；新版本通知（沿用 `notify.rs`）；Rust 侧用到的翻译键 | 3 |
| `app/src/lib/{types,backend,api,mock,mock-fixtures,errors}.ts`、`app/src/hooks/{queries,useEngineEvents}.ts` | 前端类型、命令封装、假后端、查询与事件 | 2、3 |
| `app/src/lib/shell-line.json`、`app/src/lib/update-failure-cases.json` | 面板与 Rust 共读的用例表：shell 那一行；更新失败代码 ↔ 错误代码 ↔ 文案键 | 2、3 |
| `app/src/lib/api.test.ts`、`app/src/App.test.tsx` | 边界测试（监听器 4 → 5 个）；启动失败页上的退出询问改用 `QuitPrompt` | 2、3 |
| `app/src/pages/Settings.tsx`、`app/src/components/{UpdateBanner,QuitDialog}.tsx`、`app/src/components/layout/AppShell.tsx` | 设置页的 PATH 开关与检查更新；更新横幅；退出询问区分「退出」与「安装更新」 | 2、3 |
| `app/src/i18n/locales/*.json` | 新文案（五种语言） | 2、3 |
| `scripts/check-secrets.sh` | 拦截更新签名私钥的两种写法 | 3 |
| `docs/release/updater-key.md` | 更新签名密钥的生成、保存、上传、轮换 | 3 |
| `.github/workflows/release.yml`、`.github/workflows/bundle.yml`、`scripts/verify-update-signature.mjs` | 发布工作流与可复用的打包工作流；用提交的公钥核对更新包签名 | 4 |
| `.github/workflows/ci.yml` | `secrets` 任务加一步 actionlint | 4 |
| `.github/CODEOWNERS` | `cli_path.rs`；`tauri.conf.json` 与 `tauri.*.conf.json`；发布脚本 | 2、3、4 |
| `docs/ai-coding/SECURITY_CODING_SPEC.md` | 由 PATH 得出的非秘密事实可以给面板（Task 2）；自动更新的信任根与外连（Task 3） | 2、3 |
| `MAINTAINERS.md`、`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md`、`docs/ai-coding/AI_CODING_GUIDE.md` | 发布步骤与 secrets；发布只从 `origin/main` 上的提交构建；版本规则（单独的规则提交） | 4 |
| `docs/install.md`、`README.md` | 安装说明（中文）；README 中英文「安装」一节 | 5 |
| `docs/gates/<日期>-m3-acceptance.md` | M3 实机验收报告 | 6 |

---

### Task 1: 打包配置与附带的命令行程序

设计第 12 节（命令行小程序作为附带程序一起安装）。调研结论：`tauri-build` 在应用 crate 的每次 cargo 构建里都检查 `externalBin` 指向的文件，缺了就编译失败，所以 `externalBin` 不能放进所有构建都读的 `tauri.conf.json`；通用包（`--target universal-apple-darwin`）要三份文件：`-aarch64-apple-darwin`、`-x86_64-apple-darwin` 与用 `lipo` 合并的 `-universal-apple-darwin`，打包器原样复制通用那份，不检查架构；Tauri 装进包时去掉三元组后缀。只靠链接器自带签名的 arm64 应用包 `codesign --verify` 失败（「code has no resources but signature indicates they must be present」），ad-hoc 封存后有效，Gatekeeper 只把它当作未公证。

**Files:**
- Modify: `app/src-tauri/tauri.conf.json`（顶层若还有 `version` 就删掉——M2 计划的修订版已经不写；`bundle` 改为下面的内容，`icon` 保持 M2 Task 11 的值）
- Create: `app/src-tauri/tauri.bundle.conf.json`
- Create: `app/src-tauri/tests/packaging.rs`；Modify: `crates/core/src/paths.rs`（`APP_EXECUTABLE`）
- Create: `scripts/stage-cli.sh`、`scripts/stage-cli.ps1`
- Create: `scripts/release-checks.mjs`（本任务只有 `panel` 门禁与它的自测；Task 4 加其余门禁）
- Modify: `.gitignore`（`app/src-tauri/binaries/`）
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 12 节）、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 4、6 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 2 节）、`CONTRIBUTING.md`（「开发环境」一节）——都是新文件与命令的事实登记，不是规则改动，跟代码同一个提交

**Interfaces:**
- Consumes（M2，不改）：`tauri.conf.json` 的 `productName: "IssueRelay"`、`mainBinaryName: "issuerelay-app"`、`identifier: "com.sunkailun.issuerelay"`、`bundle.icon`、`build.beforeBuildCommand: "pnpm build"`；`app/src-tauri/src/commands.rs` 的 `fn cli_next_to(exe: &Path) -> PathBuf`（`issuerelay` / `issuerelay.exe`，与主程序同目录）与 `fn cli_path() -> Option<PathBuf>`、`app_info()` 的 `AppInfo.cli_path`——Tauri 去掉三元组后缀后，命令行程序正好在 `cli_next_to(current_exe)`，这两个函数不用改；`app/src-tauri/build.rs` 的面板占位逻辑：`../dist/index.html` 不存在时写一个占位页，其中的句子是 `Run <code>pnpm --dir app build</code> to build the panel.`（`panel` 门禁认的就是结尾的 `to build the panel.`，自测核对 `build.rs` 仍写这一句）；根 `Cargo.toml` 的 `members = ["crates/*", "app/src-tauri"]`。
- Produces：`pub const issuerelay_core::paths::APP_EXECUTABLE: &str = "issuerelay-app"`；`app/src-tauri/tauri.bundle.conf.json`（`{"bundle":{"externalBin":["binaries/issuerelay"]}}`，打包时 `--config src-tauri/tauri.bundle.conf.json`，路径相对 `app/`）；`bash scripts/stage-cli.sh [--target <三元组> | --universal]` 与 `pwsh scripts/stage-cli.ps1 [-Target <三元组>]`，输出 `staged app/src-tauri/binaries/issuerelay-<三元组>[.exe]`；`node scripts/release-checks.mjs panel [--index <文件>]` 与 `node scripts/release-checks.mjs self-test`（退出码 0 = 通过，1 = 没通过，2 = 参数或读文件出错），导出 `PLACEHOLDER_MARKER`、`panelProblems(html)`；本机打包命令 `pnpm --dir app build && node scripts/release-checks.mjs panel && pnpm --dir app tauri build --bundles app,dmg --config src-tauri/tauri.bundle.conf.json`。

- [ ] **Step 1: 写失败的配置测试**

`app/src-tauri/tests/packaging.rs`：

```rust
//! The packaging configuration (design 12). `tauri.conf.json` is read by every build, including
//! `tauri dev` and ci.yml's clippy and tests; what only bundling needs is merged from
//! `tauri.bundle.conf.json` with `--config`.
use serde_json::{Value, json};

fn config(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn every_build_works_without_the_staged_cli() {
    let bundle = &config("tauri.conf.json")["bundle"];
    // tauri-build demands binaries/issuerelay-<triple> for every cargo build of this crate once
    // externalBin is in the base config.
    assert!(bundle.get("externalBin").is_none(), "externalBin belongs in tauri.bundle.conf.json");
    // Signing update payloads needs the private key; only the release workflow switches it on.
    assert_eq!(bundle["createUpdaterArtifacts"], false);
}

#[test]
fn the_version_comes_from_the_cargo_workspace() {
    assert!(config("tauri.conf.json").get("version").is_none(), "the workspace Cargo.toml is the only version");
}

#[test]
fn the_main_executable_never_collides_with_the_cli() {
    let base = config("tauri.conf.json");
    let main = base["mainBinaryName"].as_str().expect("mainBinaryName is set");
    assert_eq!(main, issuerelay_core::paths::APP_EXECUTABLE);
    // APFS and NTFS ignore case: the two files would silently replace each other in the bundle.
    assert!(!main.eq_ignore_ascii_case("issuerelay"));
}

#[test]
fn installers_are_per_user_and_mac_bundles_are_sealed() {
    let bundle = &config("tauri.conf.json")["bundle"];
    assert_eq!(bundle["targets"], json!(["app", "dmg", "msi", "nsis"]));
    assert_eq!(bundle["windows"]["nsis"]["installMode"], "currentUser");
    // Without an Apple certificate the app is signed ad hoc: an arm64 app with only the linker's
    // signature has no sealed resources, and macOS calls a downloaded copy damaged.
    assert_eq!(bundle["macOS"]["signingIdentity"], "-");
    // The panel uses Tailwind 4 (oklch, color-mix, @property), whose baseline is Safari 16.4:
    // the system WebKit of macOS 13.3 (ruling M3-R4).
    assert_eq!(bundle["macOS"]["minimumSystemVersion"], "13.3");
}

#[test]
fn bundling_adds_the_cli() {
    assert_eq!(config("tauri.bundle.conf.json")["bundle"]["externalBin"], json!(["binaries/issuerelay"]));
}
```

本任务在 `crates/core/src/paths.rs` 的 `APP_ID` 下面加上 `APP_EXECUTABLE`（Task 2 的 `issuerelay start` 也用它）：

```rust
/// The desktop app's executable (`mainBinaryName` in tauri.conf.json): `issuerelay-app`, and
/// `issuerelay-app.exe` on Windows. Never `issuerelay` in any case: the CLI sits next to it.
pub const APP_EXECUTABLE: &str = "issuerelay-app";
```

Run: `cargo test -p issuerelay-app --test packaging`
Expected: FAIL：`every_build_works_without_the_staged_cli`（没有 `createUpdaterArtifacts`）、`installers_are_per_user_and_mac_bundles_are_sealed`、`bundling_adds_the_cli`（文件不存在）；`the_version_comes_from_the_cargo_workspace` 只在 M2 的配置还写着 `"version"` 时失败（M2 修订版已不写，它一开始就通过）。

- [ ] **Step 2: 改配置**

`app/src-tauri/tauri.conf.json`：若还有 `"version": "0.1.0"` 一行就删掉；`bundle` 换成下面这段（`icon` 列表照 M2 Task 11 留下的原样，不改）：

```json
  "bundle": {
    "active": true,
    "targets": ["app", "dmg", "msi", "nsis"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"],
    "createUpdaterArtifacts": false,
    "macOS": {
      "minimumSystemVersion": "13.3",
      "signingIdentity": "-"
    },
    "windows": {
      "nsis": {
        "installMode": "currentUser"
      }
    }
  }
```

（打包器在每个平台只保留能打的目标：macOS 打 `app`、`dmg`，Windows 打 `msi`、`nsis`，一张列表两边通用。`minimumSystemVersion: "13.3"` 按裁定 M3-R4：面板用的 Tailwind 4 以 Safari 16.4 为基线，即 macOS 13.3 的系统 WebKit；执行时对照 Tailwind 4 文档里写的浏览器支持核对一次，不同就停下报告。`signingIdentity: "-"` 让打包器对附带程序、主程序、应用包依次执行 `codesign --force -s - --options runtime`；dmg 本身在 ad-hoc 时不签。环境变量 `APPLE_SIGNING_IDENTITY` 会覆盖这个值，Task 4 有证书时用它。NSIS 的 `currentUser` 本是默认值，写明是为了配置测试与 Review Focus 4。MSI 的官方模板固定为所有用户安装、需要管理员，没有按用户的选项，安装说明推荐 NSIS。）

`app/src-tauri/tauri.bundle.conf.json`：

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "bundle": {
    "externalBin": ["binaries/issuerelay"]
  }
}
```

（`externalBin` 的路径相对 `tauri.conf.json` 所在目录；`--config` 合并的是配置内容，路径照样相对 `app/src-tauri/` 解析。若解析到的 Tauri 版本把合并文件里的相对路径改为相对该文件，结果相同，因为两个文件在同一目录。）

`.gitignore` 加一行：

```gitignore
app/src-tauri/binaries/
```

Run: `cargo test -p issuerelay-app --test packaging` 与 `cargo build -p issuerelay-app`（`binaries/` 不存在时）
Expected: 测试 5/5 通过；构建成功——证明开发构建与 `ci.yml` 不需要先放命令行程序。

- [ ] **Step 3: 写 `scripts/stage-cli.sh`**

```bash
#!/usr/bin/env bash
# Builds the `issuerelay` CLI in release mode and stages it where Tauri's externalBin looks for
# it (app/src-tauri/tauri.bundle.conf.json): app/src-tauri/binaries/issuerelay-<target triple>,
# with .exe for Windows targets. Tauri strips the triple when it bundles the file next to the
# app's executable.
#
#   (no argument)       the host's triple, for `tauri build` without --target
#   --target <triple>   one triple
#   --universal         aarch64-apple-darwin and x86_64-apple-darwin, plus the lipo-merged
#                       issuerelay-universal-apple-darwin that `tauri build --target
#                       universal-apple-darwin` copies as it is: the build script needs all three
#
# Usage: bash scripts/stage-cli.sh [--target <triple> | --universal]
# Exit codes: 0 = staged, 2 = bad arguments, anything else = a failed build or copy.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() { echo "usage: bash scripts/stage-cli.sh [--target <triple> | --universal]"; }

out=app/src-tauri/binaries
mode=host
target=""
case "$#:${1-}" in
  0:) ;;
  1:--universal) mode=universal ;;
  2:--target)
    mode=target
    target=$2
    ;;
  1:-h | 1:--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

# Builds the CLI for one triple and copies it into place.
stage() {
  local triple=$1 ext=""
  [[ $triple == *-windows-* ]] && ext=.exe
  cargo build -p issuerelay-cli --release --locked --target "$triple"
  mkdir -p "$out"
  cp "target/$triple/release/issuerelay$ext" "$out/issuerelay-$triple$ext"
  echo "staged $out/issuerelay-$triple$ext"
}

case $mode in
  host) stage "$(rustc --print host-tuple)" ;;
  target) stage "$target" ;;
  universal)
    stage aarch64-apple-darwin
    stage x86_64-apple-darwin
    lipo -create -output "$out/issuerelay-universal-apple-darwin" \
      "$out/issuerelay-aarch64-apple-darwin" "$out/issuerelay-x86_64-apple-darwin"
    archs=$(lipo -archs "$out/issuerelay-universal-apple-darwin")
    [[ $archs == *x86_64* && $archs == *arm64* ]] || {
      echo "stage-cli: the universal CLI holds only: $archs" >&2
      exit 1
    }
    echo "staged $out/issuerelay-universal-apple-darwin ($archs)"
    ;;
esac
```

`chmod +x scripts/stage-cli.sh`（Git 记录可执行位；CI 里用 `bash` 调用，不依赖它）。

- [ ] **Step 4: 写 `scripts/stage-cli.ps1`**

```powershell
# Builds the issuerelay CLI in release mode and stages it where Tauri's externalBin looks for it
# (app/src-tauri/tauri.bundle.conf.json): app/src-tauri/binaries/issuerelay-<triple>.exe. Tauri
# strips the triple when it bundles the file next to issuerelay-app.exe. Windows releases are
# x64 only.
#
# Usage: pwsh scripts/stage-cli.ps1 [-Target <triple>]
# Exit codes: 0 = staged; a failed build or copy stops the script with an error.
param([string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Set-Location (Join-Path $PSScriptRoot '..')

cargo build -p issuerelay-cli --release --locked --target $Target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
$out = 'app/src-tauri/binaries'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$dest = Join-Path $out "issuerelay-$Target.exe"
Copy-Item "target/$Target/release/issuerelay.exe" $dest -Force
Write-Output "staged $dest"
```

- [ ] **Step 5: `scripts/release-checks.mjs` 的自测（先失败）**

发布门禁写成脚本里的纯函数，每道门都有通过与不通过的自测；本任务先放打包前的「面板不是占位页」一道（裁定 M3-AR6），Task 4 在同一个文件里加标签、`origin/main`、资产与 `latest.json` 几道（M3-AR7）。先写文件头、自测和命令行入口，门禁本身留到下一步：

```js
#!/usr/bin/env node
// Release gates: checks a release must pass, as pure functions over what they are given, so that
// `self-test` checks every gate, failing cases included, before any of them runs for real. The
// release workflow (.github/workflows/release.yml, bundle.yml) runs them for a tag and, in a dry
// run, for the tag that would be pushed and the dry run's bundles; developers run `panel` before
// bundling on their own machine.
//
// Usage: node scripts/release-checks.mjs <gate> [options]
//   panel [--index <file>]   the panel about to be embedded (app/dist/index.html by default) is the
//                            real one, not the placeholder page app/src-tauri/build.rs writes when
//                            the panel was not built
//   self-test                every gate against made-up input
// Exit codes: 0 = passed, 1 = a gate failed, 2 = bad arguments or unreadable input.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// ---- gates (Step 6) ----------------------------------------------------------------------------

// ---- self-test -----------------------------------------------------------------------------------

/** The line of build.rs that holds its placeholder page: the gate is tested against the real thing. */
function placeholderPage() {
  const buildRs = readFileSync(path.join(root, "app/src-tauri/build.rs"), "utf8");
  const line = buildRs.split("\n").find((text) => text.includes(PLACEHOLDER_MARKER));
  assert.ok(line, `build.rs no longer writes "${PLACEHOLDER_MARKER}": update PLACEHOLDER_MARKER with it`);
  return line;
}

const SELF_TESTS = {
  panel() {
    const built = '<!doctype html><html><head><title>IssueRelay</title><script type="module" src="/assets/index.js"></script></head><body><div id="root"></div></body></html>';
    assert.deepEqual(panelProblems(built), []);
    assert.match(panelProblems(null).join(), /is missing/);
    assert.match(panelProblems(placeholderPage()).join(), /placeholder/);
  },
};

function selfTest() {
  let failed = 0;
  for (const [name, test] of Object.entries(SELF_TESTS)) {
    try {
      test();
      console.log(`ok ${name}`);
    } catch (error) {
      failed += 1;
      console.log(`FAILED ${name}: ${error.message}`);
    }
  }
  return failed === 0 ? 0 : 1;
}

// ---- command line --------------------------------------------------------------------------------

/** Each gate's options, and what it does with them: the list of problems it found. */
const GATES = {
  panel: {
    options: { index: { type: "string" } },
    run: (values) => {
      const file = path.resolve(values.index ?? path.join(root, "app/dist/index.html"));
      return panelProblems(existsSync(file) ? readFileSync(file, "utf8") : null);
    },
  },
};

function main(argv) {
  const [name, ...rest] = argv;
  if (name === "self-test" && rest.length === 0) return selfTest();
  const gate = GATES[name];
  if (gate === undefined) throw new Error(`usage: node scripts/release-checks.mjs <${[...Object.keys(GATES), "self-test"].join("|")}> [options]`);
  const { values } = parseArgs({ args: rest, options: gate.options, strict: true });
  const problems = gate.run(values);
  for (const problem of problems) console.log(`FAILED ${name}: ${problem}`);
  if (problems.length === 0) console.log(`ok ${name}`);
  return problems.length === 0 ? 0 : 1;
}

try {
  process.exitCode = main(process.argv.slice(2));
} catch (error) {
  console.error(`release-checks: ${error.message}`);
  process.exitCode = 2;
}
```

Run: `node scripts/release-checks.mjs self-test; echo "exit $?"`
Expected: `FAILED panel: panelProblems is not defined`、`exit 1`。

- [ ] **Step 6: 实现 `panel` 门禁**

在 `// ---- gates (Step 6)` 那一行下面加：

```js
/** A sentence only the placeholder page of app/src-tauri/build.rs has (`self-test` checks build.rs still writes it). */
export const PLACEHOLDER_MARKER = "to build the panel.";

/**
 * What is wrong with the panel page about to be embedded into the app: missing, or the placeholder
 * build.rs writes so that a build without `pnpm build` still compiles. `html` is null when the file
 * does not exist. A release build embeds app/dist as it is, so the placeholder would ship.
 */
export function panelProblems(html) {
  if (html === null) return ["app/dist/index.html is missing: run `pnpm --dir app build` first"];
  if (html.includes(PLACEHOLDER_MARKER)) {
    return ["app/dist/index.html is build.rs's placeholder page, not the panel: run `pnpm --dir app build` first"];
  }
  return [];
}
```

Run:

```bash
node scripts/release-checks.mjs self-test; echo "exit $?"
pnpm --dir app build && node scripts/release-checks.mjs panel; echo "exit $?"
node scripts/release-checks.mjs panel --index app/src-tauri/build.rs; echo "exit $?"
node scripts/release-checks.mjs nonsense; echo "exit $?"
```

Expected，依次：`ok panel`、`exit 0`；`ok panel`、`exit 0`（真正的面板）；`FAILED panel: app/dist/index.html is build.rs's placeholder page, not the panel: …`、`exit 1`（`build.rs` 里有那一句，借它当一个占位页）；`release-checks: usage: …`、`exit 2`。

- [ ] **Step 7: 在本机核对脚本与打出的应用包**

先给固定的工具链装上两个 Apple 目标（在仓库目录里运行，`rust-toolchain.toml` 固定的 1.99.0 才会拿到它们）：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash -n scripts/stage-cli.sh && bash scripts/stage-cli.sh --help
bash scripts/stage-cli.sh --universal
ls app/src-tauri/binaries
```

Expected: `usage: …`；三行 `staged app/src-tauri/binaries/issuerelay-aarch64-apple-darwin`、`…-x86_64-apple-darwin`、`…-universal-apple-darwin (x86_64 arm64)`；目录里正好这三个文件。

再按本机三元组打一个应用包（通用包由 Task 4 的工作流打；本机只为看清包的结构）。先从托盘退出本机正在运行的 IssueRelay 与开发宿主（下面的 `status` 要看到「没在运行」）。打包前先构建面板并确认它不是 `build.rs` 的占位页（`tauri build` 的 `beforeBuildCommand` 也会构建，这一步让占位页在任何情况下都进不了包）。`CI=true` 让打包器跳过给 dmg 窗口排版的访达脚本，避免弹出「控制访达」的授权框（只影响 dmg 窗口的外观）：

```bash
bash scripts/stage-cli.sh
pnpm --dir app build
node scripts/release-checks.mjs panel
CI=true pnpm --dir app tauri build --bundles app,dmg --config src-tauri/tauri.bundle.conf.json
app=target/release/bundle/macos/IssueRelay.app
ls "$app/Contents/MacOS"
"$app/Contents/MacOS/issuerelay" --version
"$app/Contents/MacOS/issuerelay" status; echo "exit $?"
codesign --verify --deep --strict --verbose=2 "$app"
codesign -dv "$app" 2>&1 | grep -E '^(Signature|CodeDirectory)'
spctl --assess --type execute -vv "$app"; echo "spctl exit $?"
ls target/release/bundle/dmg
```

Expected：`ok panel`；`Contents/MacOS` 里正好有 `issuerelay-app` 和 `issuerelay`（三元组后缀已去掉）；`issuerelay 0.1.0`（工作区版本）；托盘程序没运行时 `Error: IssueRelay is not running`、`exit 1`（Task 2 再补上指向 `issuerelay start` 的提示）；`codesign --verify` 输出 `valid on disk` 与 `satisfies its Designated Requirement`；`Signature=adhoc`，`CodeDirectory` 行的 flags 含 `adhoc,runtime`；`spctl` 说 `rejected`（未公证的正常结果，**不是** `code has no resources but signature indicates they must be present`）；`dmg` 目录里有 `IssueRelay_0.1.0_aarch64.dmg`。任何一项不同（例如主程序不叫 `issuerelay-app`、附带程序带着后缀、签名标志不同），停下来把实际输出写进报告。打包产物都在 git 忽略的 `target/` 里，`app/src-tauri/binaries/` 同样被忽略，不必删。

- [ ] **Step 8: 文档（事实登记，跟代码同一个提交）**

- 设计第 12 节第一条改为：「Tauri 打包：macOS 出一个通用 `.dmg`（Apple 芯片与 Intel 通用，最低 macOS 13.3），Windows 出 NSIS `.exe`（按用户安装，不需要管理员，推荐）和 `.msi`（为所有用户安装，需要管理员）。命令行小程序作为附带程序（`externalBin`）装在主程序 `issuerelay-app` 旁边：macOS `IssueRelay.app/Contents/MacOS/issuerelay`，Windows 安装目录下的 `issuerelay.exe`。`externalBin` 只写在打包专用的 `app/src-tauri/tauri.bundle.conf.json` 里，由 `scripts/stage-cli.sh` / `stage-cli.ps1` 先把命令行程序编译好放进 `app/src-tauri/binaries/`；开发构建不需要它。打包前确认内嵌的是真正的面板（`scripts/release-checks.mjs panel`），不是 `build.rs` 的占位页。没有 Apple 证书时 macOS 包做 ad-hoc 签名（未签名的 arm64 应用下载后会被报「已损坏」）；设置里提供「把命令行加入 PATH」。」
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 4 节 `app/` 的布局加：`app/src-tauri/tauri.bundle.conf.json`（打包时合并的配置）、`app/src-tauri/binaries/`（生成目录，git 忽略，放 `stage-cli` 编译好的命令行程序）、`app/src-tauri/tests/`（配置守卫测试）；第 6 节加：`scripts/stage-cli.sh`、`scripts/stage-cli.ps1`：编译命令行程序并放到 Tauri 的 `externalBin` 找它的地方；`scripts/release-checks.mjs`：发布门禁（打包前的面板检查，Task 4 起还有标签、资产与 `latest.json`），`self-test` 检查每道门（用法见文件头）。
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 2 节的表加一行：`| 本机安装包（macOS） | bash scripts/stage-cli.sh && pnpm --dir app build && node scripts/release-checks.mjs panel && CI=true pnpm --dir app tauri build --bundles app,dmg --config src-tauri/tauri.bundle.conf.json |`。
- `CONTRIBUTING.md`「开发环境」一节末尾加一段：「想在本机打安装包：先 `bash scripts/stage-cli.sh`（Windows 上 `pwsh scripts/stage-cli.ps1`）把命令行程序放进 `app/src-tauri/binaries/`，再 `pnpm --dir app build`、`node scripts/release-checks.mjs panel`（确认内嵌的是真正的面板），最后 `pnpm --dir app tauri build --config src-tauri/tauri.bundle.conf.json`。只开发面板与托盘时不需要这一步。」（这是操作说明，不新增、不改变规则。）

Run: `pnpm --dir app build` 与六条验证命令
Expected: 全部通过。

- [ ] **Step 9: Commit**

```bash
git status --short
git add app/src-tauri/tauri.conf.json app/src-tauri/tauri.bundle.conf.json app/src-tauri/tests/packaging.rs \
  crates/core/src/paths.rs scripts/stage-cli.sh scripts/stage-cli.ps1 scripts/release-checks.mjs .gitignore \
  docs/superpowers/specs/2026-10-10-issuerelay-design.md docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md \
  docs/ai-coding/AI_CODING_GUIDE.md CONTRIBUTING.md
git diff --cached --stat
git commit -F - <<'EOF'
feat(app): bundle the CLI next to the tray app

The base tauri.conf.json stays buildable without the staged CLI: externalBin lives in
tauri.bundle.conf.json, merged with --config only when bundling. Installers are per user
(NSIS), mac bundles are signed ad hoc and need macOS 13.3 (Tailwind 4's Safari 16.4
baseline). scripts/release-checks.mjs refuses to bundle build.rs's placeholder panel.

Verified: the packaging tests, the release-checks self-test, a local app bundle (layout,
ad hoc signature, CLI version) and the six checks.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

Expected：`git diff --cached --stat` 只列出上面这些文件（工作区里别人的未跟踪文件，例如 M2 留下的截图或 Playwright 文件，不在其中）。

---

### Task 2: 「把命令行加入 PATH」与 `issuerelay start`

设计第 4.1 节（托盘程序没在运行时给出提示，并可代为启动）、第 10 节、第 12 节（设置里提供「把命令行加入 PATH」）。规则：

- macOS：在 `~/.local/bin/issuerelay` 建指向应用包里命令行程序的符号链接，不需要管理员；`~/.local/bin` 不存在就建；那里已有不是 IssueRelay 建的文件或链接时不覆盖，面板说明；指向另一份 IssueRelay（应用被移动或重装）的旧链接，打开开关时一步换成新的；用宿主解析出的登录 shell PATH（M2 第一部分的 `ExecEnv`，来源为 `LoginShell` 时才算数）判断 `~/.local/bin` 在不在 PATH，不在或读不到时在面板里给出要加到 shell 配置文件里的那一行，不替用户改 shell 配置文件。应用从磁盘映像或 Gatekeeper 的临时副本（App Translocation）运行时不建链接（弹出磁盘映像后链接就断了），请用户先把应用移到「应用程序」。
- Windows：把安装目录加到 `HKCU\Environment` 的 `Path`，保持原来的注册表类型（通常是 `REG_EXPAND_SZ`；没有这个值时新建为 `REG_EXPAND_SZ`），只读写原始值，绝不把展开后的进程 PATH 写回去；比较时展开 `%VAR%`、去掉引号与结尾反斜杠、不分大小写，已有就不加；其他条目逐字保留；结果超过 32,767 个字符就拒绝；写完广播 `WM_SETTINGCHANGE`（`lParam` 为 `"Environment"`，`SendMessageTimeoutW`，`SMTO_ABORTIFHUNG`，5 秒）；移除时只删这一项。不用 `setx`（它会把超过 1024 个字符的 PATH 截断）。
- `issuerelay start`：先问一次宿主；已在运行就只说明；没运行就启动——macOS 用 `/usr/bin/open -b com.sunkailun.issuerelay`（`-b` 按应用标识让 Launch Services 找到已安装的应用），Windows 启动命令行程序同目录下的 `issuerelay-app.exe`（不挂在当前控制台上，关掉终端不会带走它；找不到时说明从开始菜单启动）——然后最多等 20 秒直到宿主应答。只用固定参数，不经 shell。
- MCP 服务模式（`issuerelay mcp`）从不启动托盘程序；宿主没运行时工具报错，文字里请 AI 转告用户打开 IssueRelay，不提 `issuerelay start`（避免 AI 静默启动一个有完全权限的程序）。命令行的提示才指向 `issuerelay start`。
- 纯逻辑放在 core 的 `cli_path` 模块，三个平台都跑测试；macOS 链接的读写用临时目录测试；Windows 注册表读写只在 CI 上编译，测试不碰真实注册表，实机行为待实机（Task 6）。

**Files:**
- Create: `crates/core/src/cli_path.rs`；Modify: `crates/core/src/lib.rs`（`pub mod cli_path;`）、`crates/core/Cargo.toml`（Windows 的 `windows-sys` 加 `Win32_UI_WindowsAndMessaging` 特性）
- Modify: `app/src-tauri/src/commands.rs`（三条命令、文件头一句、`shell-line.json` 的测试）、`app/src-tauri/src/lib.rs`（`generate_handler!`）、`app/src-tauri/build.rs`（`PANEL_COMMANDS`）、`app/src-tauri/capabilities/default.json`（三条 `allow-*`）
- Modify: `crates/cli/src/main.rs`（`start` 子命令、没运行时的提示）、`crates/cli/src/mcp.rs`（只报错、不启动的提示）、`crates/cli/tests/mcp_tools.rs`
- Create: `app/src/lib/shell-line.json`（shell 那一行，面板与 Rust 测试共读）
- Modify: `app/src/lib/types.ts`、`app/src/lib/backend.ts`、`app/src/lib/api.ts`、`app/src/lib/mock.ts`、`app/src/lib/mock-fixtures.ts`、`app/src/hooks/queries.ts`、`app/src/pages/Settings.tsx`、`app/src/pages/Settings.test.tsx`、五个翻译文件
- Test（不改，必须照常通过）：`app/src/lib/api.test.ts`（「calls exactly the commands its capability allows」：三条新命令同时在 `api.ts` 与能力文件里）、`app/src-tauri/src/lib.rs` 的两个绊线测试
- Modify（事实登记，跟代码同一个提交）：`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 4.1、10.1、10.3、12 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 5 节）、`docs/ai-coding/TESTING_SPEC.md`（共用用例表加 `shell-line.json`）、`.github/CODEOWNERS`（`/crates/core/src/cli_path.rs`）
- Modify（规则，单独的 `docs` 提交）：`docs/ai-coding/SECURITY_CODING_SPEC.md`（第 5 节 `Host::env()` 一句）

**Interfaces:**
- Consumes（M2）：`commands.rs` 的 `cli_path() -> Option<PathBuf>`、`CmdResult<T>`、`error(e)`、`async fn host(state: &State<'_, AppState>) -> CmdResult<Host>`；`Host::env(&self) -> ExecEnv`；`issuerelay_core::env::{ExecEnv, EnvSource}`（`path: Option<String>`、`path_source: EnvSource::LoginShell`）；`paths::APP_ID`、`paths::APP_EXECUTABLE`（Task 1）；`ipc::client::{call, NotRunning}`；`crates/cli/src/main.rs` 的 `Cli`、`Command`、`request`、`write_line`、`fail`、`run`；`format::error_line`、`format::clean`；`mcp::ControlServer::forward`；`build.rs` 的 `PANEL_COMMANDS`（M2 34 条）与 `capabilities/default.json`；前端 `Backend`、`tauriBackend`、`call`、`createMockBackend`、`queryKeys`、`queries.ts` 里不导出的 `useAction<Variables, Result>(run: (backend, variables) => Promise<Result>, refreshes, failureKey, options?)`（返回 TanStack 的 `useMutation` 结果；新钩子写在它旁边）、`useCopyText()`、`cleanDisplayText`、`renderWithProviders`、`Settings.tsx` 的 `SwitchRow`、「系统」分组与 `Autostart`（先 `LoadingState` 后开关、读取失败是 `ErrorState` 的写法）、`ErrorState`、`LoadingState`、`Hint`、shadcn `Switch` 与 `Button`。
- 规则（裁定 F14）：`cli_setup` 读 `Host::env().path` 只为得出不含秘密的事实（`~/.local/bin` 在不在登录 shell 的 PATH 里），命令只返回这些事实与固定的 shell 那一行；`Host::env()` 本身和代理值从不交给网页视图。`SECURITY_CODING_SPEC.md` 第 5 节与 `commands.rs` 的文件头按这个意思改一句。
- Produces（core `issuerelay_core::cli_path`）：
  - `pub const SHELL_LINE: &str = r#"export PATH="$HOME/.local/bin:$PATH""#`、`pub const MAX_ENV_CHARS: usize = 32_767`
  - `pub enum AppLocation { Installed, NotInstalled, Development }`；`pub fn bundle_location(exe: &Path, read_only: bool) -> AppLocation`；`pub fn app_location(exe: &Path, read_only: bool, debug: bool) -> AppLocation`；`pub fn on_read_only_volume(path: &Path) -> bool`；`pub fn running_app_location() -> AppLocation`（Task 3 的更新也用它）
  - `pub enum LinkSlot { Empty, Link(PathBuf), Other }`、`pub enum LinkState { Missing, Ours, Stale(PathBuf), Foreign }`、`pub fn link_state(slot: &LinkSlot, cli: &Path) -> LinkState`、`pub fn link_path(home: &Path) -> PathBuf`、`pub fn unix_path_has(dir: &Path, path_var: &str, home: &Path) -> bool`
  - `pub fn expand_vars(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String`、`pub fn windows_path_has(dir: &str, value: &str, lookup: …) -> bool`、`pub fn windows_path_with(dir: &str, value: &str, lookup: …) -> Result<Option<String>, PathTooLong>`、`pub fn windows_path_without(dir: &str, value: &str, lookup: …) -> Option<String>`、`pub struct PathTooLong(pub usize)`、`pub fn decode_reg_string(bytes: &[u8]) -> Result<String, FromUtf16Error>`、`pub fn encode_reg_string(text: &str) -> Vec<u8>`
  - `pub enum CliPathStatus`（serde `tag = "kind"`：`{"kind":"off"}`、`{"kind":"on","shell_line":null|"…"}`、`{"kind":"stale","target":"…"}`、`{"kind":"blocked","path":"…"}`、`{"kind":"unavailable","reason":"missing_cli"|"not_installed"|"development"}`）、`pub enum CliUnavailable { MissingCli, NotInstalled, Development }`
  - `pub struct CliSetup { pub cli: Option<PathBuf>, pub location: AppLocation, pub home: PathBuf, pub shell_path: Option<String> }`、`CliSetup::for_running_app(cli: Option<PathBuf>, shell_path: Option<String>) -> anyhow::Result<CliSetup>`；`pub fn status(setup: &CliSetup) -> anyhow::Result<CliPathStatus>`、`pub fn install(…)`、`pub fn remove(…)`（同签名）
- Produces（应用命令，前端 camelCase）：`cli_on_path` / `install_cli_on_path` / `remove_cli_from_path`，无参数，都返回 `CliPathStatus`，登记在 `generate_handler!`、`PANEL_COMMANDS` 与能力文件（`allow-cli-on-path`、`allow-install-cli-on-path`、`allow-remove-cli-from-path`），命令共 37 条；`Backend.cliOnPath()`、`installCliOnPath()`、`removeCliFromPath()`；`queryKeys.cliPath`、`useCliPath()`、`useInstallCliOnPath()`、`useRemoveCliFromPath()`；TS 类型 `CliPathStatus`、`CliUnavailable`；`app/src/lib/shell-line.json`（`{"shell_line": "…"}`，与 core `SHELL_LINE` 由 `commands.rs` 的测试核对）与由它得出的假后端常量 `DEMO_SHELL_LINE`。
- Produces（命令行）：子命令 `issuerelay start`；常量 `NOT_RUNNING_HINT`（命令行）与 `MCP_NOT_RUNNING_HINT`（MCP）。

- [ ] **Step 1: core `cli_path` 的失败测试**

`crates/core/src/cli_path.rs` 先只放测试模块（下一步写实现）：

```rust
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    const INSTALLED: &str = "/Applications/IssueRelay.app/Contents/MacOS/issuerelay-app";
    const CLI: &str = "/Applications/IssueRelay.app/Contents/MacOS/issuerelay";

    #[test]
    fn only_an_app_bundle_on_a_writable_volume_counts_as_installed() {
        assert_eq!(bundle_location(Path::new(INSTALLED), false), AppLocation::Installed);
        // The user's own Applications folder is just as good.
        let personal = "/tmp/demo-home/Applications/IssueRelay.app/Contents/MacOS/issuerelay-app";
        assert_eq!(bundle_location(Path::new(personal), false), AppLocation::Installed);
        // A mounted disk image is read-only; so is Gatekeeper's randomized copy.
        assert_eq!(bundle_location(Path::new(INSTALLED), true), AppLocation::NotInstalled);
        let translocated =
            "/private/var/folders/ab/xyz/T/AppTranslocation/0A1B2C3D/d/IssueRelay.app/Contents/MacOS/issuerelay-app";
        assert_eq!(bundle_location(Path::new(translocated), false), AppLocation::NotInstalled);
        // Outside an app bundle: a development build.
        assert_eq!(bundle_location(Path::new("/tmp/demo/target/release/issuerelay-app"), false), AppLocation::Development);
        assert_eq!(bundle_location(Path::new("/tmp/demo/Contents/MacOS/issuerelay-app"), false), AppLocation::Development);
        // A debug build never counts as installed, wherever it is.
        assert_eq!(app_location(Path::new(INSTALLED), false, true), AppLocation::Development);
    }

    #[test]
    fn the_link_is_ours_stale_or_someone_elses() {
        let cli = Path::new(CLI);
        assert_eq!(link_state(&LinkSlot::Empty, cli), LinkState::Missing);
        assert_eq!(link_state(&LinkSlot::Link(PathBuf::from(CLI)), cli), LinkState::Ours);
        // IssueRelay's link to a copy that was moved or reinstalled elsewhere.
        let old = PathBuf::from("/tmp/demo-home/Downloads/IssueRelay.app/Contents/MacOS/issuerelay");
        assert_eq!(link_state(&LinkSlot::Link(old.clone()), cli), LinkState::Stale(old));
        // Anything else is never touched: another program's link, or a plain file.
        assert_eq!(link_state(&LinkSlot::Link(PathBuf::from("/opt/demo/bin/issuerelay")), cli), LinkState::Foreign);
        let other_app = PathBuf::from("/Applications/Other.app/Contents/MacOS/other");
        assert_eq!(link_state(&LinkSlot::Link(other_app), cli), LinkState::Foreign);
        assert_eq!(link_state(&LinkSlot::Other, cli), LinkState::Foreign);
        assert_eq!(link_path(Path::new("/tmp/demo-home")), Path::new("/tmp/demo-home/.local/bin/issuerelay"));
    }

    #[test]
    fn the_shell_path_is_read_with_tilde_home_and_trailing_slashes() {
        let home = Path::new("/tmp/demo-home");
        let bin = Path::new("/tmp/demo-home/.local/bin");
        for path in [
            "/usr/bin:/tmp/demo-home/.local/bin:/bin",
            "/tmp/demo-home/.local/bin/",
            "~/.local/bin:/usr/bin",
            "$HOME/.local/bin",
            "${HOME}/.local/bin",
            "/usr/bin:/tmp/demo-home//.local/bin",
        ] {
            assert!(unix_path_has(bin, path, home), "{path}");
        }
        for path in ["/usr/bin:/bin", "/tmp/demo-home/.local/bin2", "/tmp/demo-home/.local", ""] {
            assert!(!unix_path_has(bin, path, home), "{path}");
        }
    }

    fn lookup(name: &str) -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "LOCALAPPDATA" => Some(r"D:\Profiles\demo\AppData\Local".into()),
            "USERPROFILE" => Some(r"D:\Profiles\demo".into()),
            _ => None,
        }
    }

    const INSTALL_DIR: &str = r"D:\Profiles\demo\AppData\Local\IssueRelay";

    #[test]
    fn environment_references_expand_and_unknown_ones_stay() {
        assert_eq!(expand_vars(r"%LOCALAPPDATA%\IssueRelay", &lookup), INSTALL_DIR);
        assert_eq!(expand_vars(r"%localappdata%\IssueRelay", &lookup), INSTALL_DIR);
        assert_eq!(expand_vars(r"%NOPE%\bin", &lookup), r"%NOPE%\bin");
        assert_eq!(expand_vars("50%", &lookup), "50%");
        assert_eq!(expand_vars("%%", &lookup), "%%");
    }

    #[test]
    fn the_install_dir_is_found_however_it_is_written() {
        for value in [
            r"%LOCALAPPDATA%\IssueRelay",
            r"d:\profiles\DEMO\appdata\local\issuerelay\",
            r#""D:\Profiles\demo\AppData\Local\IssueRelay""#,
            r"C:\Tools; D:\Profiles\demo\AppData\Local\IssueRelay ;",
        ] {
            assert!(windows_path_has(INSTALL_DIR, value, &lookup), "{value}");
        }
        assert!(!windows_path_has(INSTALL_DIR, r"D:\Profiles\demo\AppData\Local\IssueRelay2", &lookup));
        assert!(!windows_path_has(INSTALL_DIR, "", &lookup));
    }

    #[test]
    fn adding_appends_once_and_keeps_every_other_entry_as_written() {
        let value = r"%USERPROFILE%\bin;;C:\Tools";
        assert_eq!(
            windows_path_with(INSTALL_DIR, value, &lookup).unwrap(),
            Some(format!(r"%USERPROFILE%\bin;;C:\Tools;{INSTALL_DIR}"))
        );
        assert_eq!(windows_path_with(INSTALL_DIR, r"C:\Tools;", &lookup).unwrap(), Some(format!(r"C:\Tools;{INSTALL_DIR}")));
        assert_eq!(windows_path_with(INSTALL_DIR, "", &lookup).unwrap(), Some(INSTALL_DIR.to_string()));
        // Already there under another spelling: nothing to write.
        assert_eq!(windows_path_with(INSTALL_DIR, r"%LOCALAPPDATA%\IssueRelay\", &lookup).unwrap(), None);
        // Windows allows 32,767 characters per user variable.
        let long = "x".repeat(MAX_ENV_CHARS - 10);
        assert!(matches!(windows_path_with(INSTALL_DIR, &long, &lookup), Err(PathTooLong(_))));
    }

    #[test]
    fn removing_takes_out_only_the_install_dir() {
        let value = format!(r"%USERPROFILE%\bin;{INSTALL_DIR};;C:\Tools;%LOCALAPPDATA%\IssueRelay\;");
        assert_eq!(
            windows_path_without(INSTALL_DIR, &value, &lookup),
            Some(r"%USERPROFILE%\bin;;C:\Tools;".to_string())
        );
        assert_eq!(windows_path_without(INSTALL_DIR, INSTALL_DIR, &lookup), Some(String::new()));
        assert_eq!(windows_path_without(INSTALL_DIR, r"C:\Tools", &lookup), None);
    }

    #[test]
    fn registry_strings_round_trip_and_bad_text_is_refused() {
        let encoded = encode_reg_string(r"%USERPROFILE%\bin;C:\Tools");
        assert_eq!(&encoded[encoded.len() - 2..], &[0, 0], "NUL-terminated");
        assert_eq!(decode_reg_string(&encoded).unwrap(), r"%USERPROFILE%\bin;C:\Tools");
        // Without the terminator the text is the same.
        assert_eq!(decode_reg_string(&encoded[..encoded.len() - 2]).unwrap(), r"%USERPROFILE%\bin;C:\Tools");
        // A lone surrogate is not text: the value is left alone rather than rewritten lossily.
        assert!(decode_reg_string(&[0x00, 0xd8, 0x41, 0x00]).is_err());
    }

    #[test]
    fn the_status_reads_as_the_panel_expects() {
        let on = CliPathStatus::On { shell_line: Some(SHELL_LINE.into()) };
        assert_eq!(
            serde_json::to_value(on).unwrap(),
            serde_json::json!({"kind": "on", "shell_line": "export PATH=\"$HOME/.local/bin:$PATH\""})
        );
        let unavailable = CliPathStatus::Unavailable { reason: CliUnavailable::NotInstalled };
        assert_eq!(serde_json::to_value(unavailable).unwrap(), serde_json::json!({"kind": "unavailable", "reason": "not_installed"}));
    }

    #[cfg(unix)]
    mod link {
        use super::*;

        /// A fake installed app with its CLI, and a home without `~/.local/bin`.
        fn setup(root: &Path, shell_path: Option<&str>) -> CliSetup {
            let macos = root.join("Applications/IssueRelay.app/Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            let cli = macos.join("issuerelay");
            std::fs::write(&cli, "cli").unwrap();
            let home = root.join("home");
            std::fs::create_dir_all(&home).unwrap();
            CliSetup { cli: Some(cli), location: AppLocation::Installed, home, shell_path: shell_path.map(str::to_string) }
        }

        #[test]
        fn turning_it_on_links_the_cli_and_says_what_the_shell_still_needs() {
            let root = tempfile::tempdir().unwrap();
            let setup = setup(root.path(), Some("/usr/bin:/bin"));
            assert_eq!(status(&setup).unwrap(), CliPathStatus::Off);
            let on = install(&setup).unwrap();
            assert_eq!(on, CliPathStatus::On { shell_line: Some(SHELL_LINE.to_string()) });
            let link = link_path(&setup.home);
            assert_eq!(std::fs::read_link(&link).unwrap(), setup.cli.clone().unwrap());
            // Doing it again changes nothing.
            assert_eq!(install(&setup).unwrap(), on);
            // With ~/.local/bin on the login shell's PATH nothing is left to do.
            let bin = setup.home.join(".local/bin");
            let reachable = CliSetup { shell_path: Some(format!("/usr/bin:{}", bin.display())), ..setup.clone() };
            assert_eq!(status(&reachable).unwrap(), CliPathStatus::On { shell_line: None });
            // An unreadable login shell PATH shows the line too.
            let unknown = CliSetup { shell_path: None, ..setup.clone() };
            assert_eq!(status(&unknown).unwrap(), CliPathStatus::On { shell_line: Some(SHELL_LINE.to_string()) });
            assert_eq!(remove(&setup).unwrap(), CliPathStatus::Off);
            assert!(std::fs::symlink_metadata(&link).is_err());
        }

        #[test]
        fn a_file_issuerelay_did_not_make_is_never_touched() {
            let root = tempfile::tempdir().unwrap();
            let setup = setup(root.path(), None);
            let link = link_path(&setup.home);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::fs::write(&link, "someone else's script").unwrap();
            let blocked = CliPathStatus::Blocked { path: link.display().to_string() };
            assert_eq!(install(&setup).unwrap(), blocked);
            assert_eq!(remove(&setup).unwrap(), blocked);
            assert_eq!(std::fs::read_to_string(&link).unwrap(), "someone else's script");
            // Another program's link is left alone as well.
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("/opt/demo/bin/issuerelay", &link).unwrap();
            assert_eq!(install(&setup).unwrap(), blocked);
            assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("/opt/demo/bin/issuerelay"));
        }

        #[test]
        fn a_link_to_a_moved_copy_is_repointed() {
            let root = tempfile::tempdir().unwrap();
            let setup = setup(root.path(), None);
            let link = link_path(&setup.home);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            let old = root.path().join("Downloads/IssueRelay.app/Contents/MacOS/issuerelay");
            std::os::unix::fs::symlink(&old, &link).unwrap();
            assert_eq!(status(&setup).unwrap(), CliPathStatus::Stale { target: old.display().to_string() });
            install(&setup).unwrap();
            assert_eq!(std::fs::read_link(&link).unwrap(), setup.cli.clone().unwrap());
        }

        #[test]
        fn nothing_is_linked_from_a_disk_image_a_development_build_or_without_the_cli() {
            let root = tempfile::tempdir().unwrap();
            let base = setup(root.path(), None);
            for (location, cli, reason) in [
                (AppLocation::NotInstalled, base.cli.clone(), CliUnavailable::NotInstalled),
                (AppLocation::Development, base.cli.clone(), CliUnavailable::Development),
                (AppLocation::Installed, None, CliUnavailable::MissingCli),
            ] {
                let setup = CliSetup { location, cli, ..base.clone() };
                assert_eq!(install(&setup).unwrap(), CliPathStatus::Unavailable { reason });
                assert!(std::fs::symlink_metadata(link_path(&setup.home)).is_err());
            }
        }
    }
}
```

Run: `cargo test -p issuerelay-core cli_path`
Expected: 编译失败（实现还不存在）。

- [ ] **Step 2: 实现 `cli_path.rs`**

在测试模块上方写：

```rust
//! Putting the bundled `issuerelay` command on PATH without administrator rights (design 12),
//! and telling where the running app is installed. macOS: a symbolic link
//! `~/.local/bin/issuerelay` to the CLI inside the app bundle; a file IssueRelay did not make is
//! never overwritten and shell profiles are never edited (the panel shows the line to add).
//! Windows: the install directory in the user's own `Path` (`HKCU\Environment`), kept as the
//! registry type it had, compared without regard to case or spelling, never added twice;
//! removing takes out only that entry. The decisions are pure functions over what was read and
//! are tested on every platform; the reads and writes sit in `platform`.
use std::{
    path::{Path, PathBuf},
    string::FromUtf16Error,
};

use anyhow::{Context, Result};
use serde::Serialize;

/// The line the panel shows when `~/.local/bin` is not on the login shell's PATH (zsh, bash).
pub const SHELL_LINE: &str = r#"export PATH="$HOME/.local/bin:$PATH""#;
/// The longest value Windows allows for a user environment variable, in UTF-16 units.
pub const MAX_ENV_CHARS: usize = 32_767;

/// Where the running app is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppLocation {
    /// Installed: the CLI next to it may be linked, and an update may replace it.
    Installed,
    /// macOS: run from the disk image or from Gatekeeper's randomized read-only copy (App
    /// Translocation). A link to it breaks and an update cannot replace it; the app should be
    /// moved to Applications first.
    NotInstalled,
    /// A development build: a debug build, or an executable outside an app bundle.
    Development,
}

/// `<…>/<Name>.app/Contents/MacOS/<file>`.
fn in_app_bundle(path: &Path) -> bool {
    let mut up = path.ancestors().skip(1);
    let (macos, contents, bundle) = (up.next(), up.next(), up.next());
    macos.and_then(Path::file_name).is_some_and(|name| name == "MacOS")
        && contents.and_then(Path::file_name).is_some_and(|name| name == "Contents")
        && bundle.and_then(Path::extension).is_some_and(|ext| ext == "app")
}

/// macOS: where an executable is, judged by its path. `read_only` says that its volume is mounted
/// read-only, which is true of a disk image and of a translocated copy.
pub fn bundle_location(exe: &Path, read_only: bool) -> AppLocation {
    if !in_app_bundle(exe) {
        return AppLocation::Development;
    }
    let translocated = exe.components().any(|part| part.as_os_str() == "AppTranslocation");
    if read_only || translocated { AppLocation::NotInstalled } else { AppLocation::Installed }
}

/// Where the app whose executable is `exe` runs from; `debug` is `cfg!(debug_assertions)`.
/// Windows has no disk image or translocation: a release build counts as installed.
pub fn app_location(exe: &Path, read_only: bool, debug: bool) -> AppLocation {
    if debug {
        AppLocation::Development
    } else if cfg!(target_os = "macos") {
        bundle_location(exe, read_only)
    } else {
        AppLocation::Installed
    }
}

/// Whether `path` is on a volume mounted read-only.
#[cfg(unix)]
pub fn on_read_only_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is NUL-terminated and outlives the call; statvfs fills `stats` when it
    // returns 0, and `stats` is read only then.
    unsafe {
        libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) == 0
            && stats.assume_init().f_flag & libc::ST_RDONLY != 0
    }
}

/// Whether `path` is on a volume mounted read-only. Windows installs never are.
#[cfg(windows)]
pub fn on_read_only_volume(_path: &Path) -> bool {
    false
}

/// `app_location` for this process.
pub fn running_app_location() -> AppLocation {
    match std::env::current_exe() {
        Ok(exe) => app_location(&exe, on_read_only_volume(&exe), cfg!(debug_assertions)),
        Err(e) => {
            tracing::warn!(error = %e, "cannot tell where the app runs from");
            AppLocation::Development
        }
    }
}

/// What sits where the link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSlot {
    Empty,
    /// A symbolic link, and the path it points to as written.
    Link(PathBuf),
    /// A file, a folder or anything else that is not a symbolic link.
    Other,
}

/// The link compared with the CLI it should point to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Missing,
    /// It points at this app's CLI.
    Ours,
    /// IssueRelay's link to another copy of the app (moved or reinstalled); it may be repointed.
    Stale(PathBuf),
    /// Not made by IssueRelay: never changed or removed.
    Foreign,
}

/// A CLI inside an app bundle, as IssueRelay links it.
fn is_bundled_cli(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "issuerelay") && in_app_bundle(path)
}

pub fn link_state(slot: &LinkSlot, cli: &Path) -> LinkState {
    match slot {
        LinkSlot::Empty => LinkState::Missing,
        LinkSlot::Link(target) if target == cli => LinkState::Ours,
        LinkSlot::Link(target) if is_bundled_cli(target) => LinkState::Stale(target.clone()),
        LinkSlot::Link(_) | LinkSlot::Other => LinkState::Foreign,
    }
}

/// `~/.local/bin/issuerelay`.
pub fn link_path(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join("issuerelay")
}

/// `path` without trailing or doubled separators.
fn tidy(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Whether `dir` is an entry of the PATH value `path_var`. `~`, `$HOME` and `${HOME}` at the
/// start of an entry stand for `home`; a trailing `/` does not count.
pub fn unix_path_has(dir: &Path, path_var: &str, home: &Path) -> bool {
    let wanted = tidy(dir);
    path_var.split(':').map(str::trim).filter(|entry| !entry.is_empty()).any(|entry| {
        let expanded = match ["~", "$HOME", "${HOME}"].iter().find_map(|prefix| entry.strip_prefix(prefix)) {
            Some("") => home.to_path_buf(),
            Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
            _ => PathBuf::from(entry),
        };
        tidy(&expanded) == wanted
    })
}

/// `%NAME%` references in `text` replaced by `lookup(NAME)`; unknown ones stay as written.
pub fn expand_vars(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A `Path` entry as compared: references expanded, blanks, quotes and trailing separators
/// removed, lower case (Windows paths ignore case).
fn comparable(entry: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    expand_vars(entry.trim(), lookup)
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// Whether the user `Path` value names `dir`.
pub fn windows_path_has(dir: &str, value: &str, lookup: &dyn Fn(&str) -> Option<String>) -> bool {
    let wanted = comparable(dir, lookup);
    !wanted.is_empty() && value.split(';').any(|entry| comparable(entry, lookup) == wanted)
}

/// The new value would be longer than Windows allows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the user Path would be {0} characters long, more than Windows allows ({MAX_ENV_CHARS}); it was not changed")]
pub struct PathTooLong(pub usize);

/// The user `Path` with `dir` appended, or `None` when it is already there. Every other entry
/// keeps its exact text, `%VAR%` references and empty entries included.
pub fn windows_path_with(
    dir: &str,
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<String>, PathTooLong> {
    if windows_path_has(dir, value, lookup) {
        return Ok(None);
    }
    let joined = if value.is_empty() {
        dir.to_string()
    } else if value.ends_with(';') {
        format!("{value}{dir}")
    } else {
        format!("{value};{dir}")
    };
    let length = joined.encode_utf16().count();
    if length >= MAX_ENV_CHARS {
        return Err(PathTooLong(length));
    }
    Ok(Some(joined))
}

/// The user `Path` without the entries that name `dir`, or `None` when none does. The other
/// entries keep their exact text and order, empty ones included.
pub fn windows_path_without(dir: &str, value: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let wanted = comparable(dir, lookup);
    let entries: Vec<&str> = value.split(';').collect();
    let kept: Vec<&str> = entries.iter().copied().filter(|entry| comparable(entry, lookup) != wanted).collect();
    (kept.len() != entries.len()).then(|| kept.join(";"))
}

/// A registry string (UTF-16LE, usually NUL-terminated) as text; text that is not valid UTF-16
/// is an error, so it is never rewritten lossily.
pub fn decode_reg_string(bytes: &[u8]) -> Result<String, FromUtf16Error> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    let end = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
    String::from_utf16(&units[..end])
}

/// `text` as a NUL-terminated UTF-16LE registry string.
pub fn encode_reg_string(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

/// Why the CLI cannot be put on PATH from where the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliUnavailable {
    /// No `issuerelay` next to the app.
    MissingCli,
    /// macOS: run from the disk image or a translocated copy.
    NotInstalled,
    /// A development build.
    Development,
}

/// What the settings page shows for "put the command line on PATH".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CliPathStatus {
    /// IssueRelay has not put the CLI on PATH.
    Off,
    /// On PATH through IssueRelay. `shell_line` (macOS): `~/.local/bin` is not on the login
    /// shell's PATH, or that PATH could not be read; the panel shows this line to add.
    On { shell_line: Option<String> },
    /// macOS: `~/.local/bin/issuerelay` is IssueRelay's link to another copy of the app.
    Stale { target: String },
    /// macOS: `~/.local/bin/issuerelay` is something IssueRelay did not make; left alone.
    Blocked { path: String },
    /// Not possible from where the app runs.
    Unavailable { reason: CliUnavailable },
}

/// What the decisions read from the system, gathered by the caller.
#[derive(Debug, Clone)]
pub struct CliSetup {
    /// The CLI next to the app, when it is there.
    pub cli: Option<PathBuf>,
    pub location: AppLocation,
    /// The user's home folder (the macOS link lives in it).
    pub home: PathBuf,
    /// The login shell's PATH when it could be read (macOS).
    pub shell_path: Option<String>,
}

impl CliSetup {
    /// The setup of this process: where it runs from and the user's home folder.
    pub fn for_running_app(cli: Option<PathBuf>, shell_path: Option<String>) -> Result<Self> {
        Ok(Self {
            cli,
            location: running_app_location(),
            home: dirs::home_dir().context("this system has no home folder")?,
            shell_path,
        })
    }

    fn usable_cli(&self) -> Result<&Path, CliUnavailable> {
        match self.location {
            AppLocation::Development => Err(CliUnavailable::Development),
            AppLocation::NotInstalled => Err(CliUnavailable::NotInstalled),
            AppLocation::Installed => self.cli.as_deref().ok_or(CliUnavailable::MissingCli),
        }
    }
}

/// Whether the CLI is on PATH through IssueRelay.
pub fn status(setup: &CliSetup) -> Result<CliPathStatus> {
    match setup.usable_cli() {
        Ok(cli) => platform::status(setup, cli),
        Err(reason) => Ok(CliPathStatus::Unavailable { reason }),
    }
}

/// Puts the CLI on PATH; returns the new status (a blocked or unavailable one changes nothing).
pub fn install(setup: &CliSetup) -> Result<CliPathStatus> {
    match setup.usable_cli() {
        Ok(cli) => platform::install(setup, cli),
        Err(reason) => Ok(CliPathStatus::Unavailable { reason }),
    }
}

/// Takes the CLI off PATH (only what IssueRelay added); returns the new status.
pub fn remove(setup: &CliSetup) -> Result<CliPathStatus> {
    platform::remove(setup)?;
    status(setup)
}

#[cfg(unix)]
mod platform {
    use std::io;

    use super::*;

    fn read_slot(link: &Path) -> io::Result<LinkSlot> {
        match std::fs::symlink_metadata(link) {
            Ok(meta) if meta.file_type().is_symlink() => Ok(LinkSlot::Link(std::fs::read_link(link)?)),
            Ok(_) => Ok(LinkSlot::Other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(LinkSlot::Empty),
            Err(e) => Err(e),
        }
    }

    pub fn status(setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let link = link_path(&setup.home);
        let slot = read_slot(&link).with_context(|| format!("cannot read {}", link.display()))?;
        Ok(match link_state(&slot, cli) {
            LinkState::Missing => CliPathStatus::Off,
            LinkState::Ours => {
                let bin = link.parent().expect("the link sits in a folder");
                let reachable =
                    setup.shell_path.as_deref().is_some_and(|path| unix_path_has(bin, path, &setup.home));
                CliPathStatus::On { shell_line: (!reachable).then(|| SHELL_LINE.to_string()) }
            }
            LinkState::Stale(target) => CliPathStatus::Stale { target: target.display().to_string() },
            LinkState::Foreign => CliPathStatus::Blocked { path: link.display().to_string() },
        })
    }

    pub fn install(setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let link = link_path(&setup.home);
        let bin = link.parent().expect("the link sits in a folder");
        let slot = read_slot(&link).with_context(|| format!("cannot read {}", link.display()))?;
        match link_state(&slot, cli) {
            LinkState::Ours | LinkState::Foreign => {}
            LinkState::Missing => {
                std::fs::create_dir_all(bin).with_context(|| format!("cannot create {}", bin.display()))?;
                std::os::unix::fs::symlink(cli, &link).with_context(|| format!("cannot create {}", link.display()))?;
            }
            LinkState::Stale(_) => {
                // In one step: a new link under a temporary name, renamed over the old one.
                let temporary = bin.join(format!(".issuerelay-link-{}", std::process::id()));
                let _ = std::fs::remove_file(&temporary);
                std::os::unix::fs::symlink(cli, &temporary)
                    .with_context(|| format!("cannot create {}", temporary.display()))?;
                std::fs::rename(&temporary, &link).with_context(|| format!("cannot replace {}", link.display()))?;
            }
        }
        status(setup, cli)
    }

    pub fn remove(setup: &CliSetup) -> Result<()> {
        let link = link_path(&setup.home);
        if let LinkSlot::Link(target) = read_slot(&link).with_context(|| format!("cannot read {}", link.display()))?
            && (setup.cli.as_deref() == Some(target.as_path()) || is_bundled_cli(&target))
        {
            std::fs::remove_file(&link).with_context(|| format!("cannot remove {}", link.display()))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use std::io;

    use winreg::{
        RegKey, RegValue,
        enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, RegType},
    };

    use super::*;

    fn lookup(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn open() -> Result<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
            .context("cannot open the user environment (HKCU\\Environment)")
    }

    /// The user `Path` as stored, and its registry type; a missing value reads as an empty
    /// `REG_EXPAND_SZ`. Never the expanded process PATH.
    fn read(key: &RegKey) -> Result<(String, RegType)> {
        match key.get_raw_value("Path") {
            Ok(value) => {
                anyhow::ensure!(
                    matches!(value.vtype, RegType::REG_SZ | RegType::REG_EXPAND_SZ),
                    "the user Path has an unexpected registry type; it was not changed"
                );
                let text = decode_reg_string(&value.bytes).context("the user Path is not valid text; it was not changed")?;
                Ok((text, value.vtype))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((String::new(), RegType::REG_EXPAND_SZ)),
            Err(e) => Err(e).context("cannot read the user Path"),
        }
    }

    /// Writes the user `Path` with the type it had and tells running programs.
    fn write(key: &RegKey, text: &str, vtype: RegType) -> Result<()> {
        key.set_raw_value("Path", &RegValue { bytes: encode_reg_string(text).into(), vtype })
            .context("cannot write the user Path")?;
        broadcast();
        Ok(())
    }

    /// Tells running programs (Explorer, and so new terminals) that the environment changed.
    fn broadcast() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        let name: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
        let mut result = 0usize;
        // SAFETY: `name` is NUL-terminated and outlives the call; `result` is a valid out pointer.
        let sent = unsafe {
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, name.as_ptr() as isize, SMTO_ABORTIFHUNG, 5000, &mut result)
        };
        if sent == 0 {
            tracing::warn!("could not tell running programs that PATH changed; terminals opened later may need a new sign-in");
        }
    }

    fn dir_of(cli: &Path) -> Result<String> {
        Ok(cli.parent().context("the CLI has no folder")?.display().to_string())
    }

    pub fn status(_setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let (value, _) = read(&open()?)?;
        Ok(if windows_path_has(&dir_of(cli)?, &value, &lookup) {
            CliPathStatus::On { shell_line: None }
        } else {
            CliPathStatus::Off
        })
    }

    pub fn install(setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let key = open()?;
        let (value, vtype) = read(&key)?;
        if let Some(changed) = windows_path_with(&dir_of(cli)?, &value, &lookup)? {
            write(&key, &changed, vtype)?;
        }
        status(setup, cli)
    }

    pub fn remove(setup: &CliSetup) -> Result<()> {
        let Some(cli) = setup.cli.as_deref() else {
            return Ok(());
        };
        let key = open()?;
        let (value, vtype) = read(&key)?;
        if let Some(changed) = windows_path_without(&dir_of(cli)?, &value, &lookup) {
            write(&key, &changed, vtype)?;
        }
        Ok(())
    }
}
```

`crates/core/src/lib.rs` 加 `pub mod cli_path;`（按字母序放在 `config` 前面）。`crates/core/Cargo.toml` 的 `[target.'cfg(windows)'.dependencies]` 里 `windows-sys` 的特性列表加 `"Win32_UI_WindowsAndMessaging"`（注释改为「…the named pipe's security descriptor…, and the `WM_SETTINGCHANGE` broadcast after the user PATH changes (`cli_path`)」）。

（`winreg` 0.56 的 `RegValue.bytes` 可能是 `Vec<u8>` 也可能是 `Cow<[u8]>`：`encode_reg_string(text).into()` 与 `&value.bytes` 两种都能编译；`windows-sys` 0.61 的 `HWND_BROADCAST`、`SendMessageTimeoutW` 参数类型（`LPARAM = isize`、结果指针 `*mut usize`）以解析到的版本为准，CI 的 Windows 任务编译通过即可，不同就照文档改并在报告里说明。macOS 上 `libc::statvfs` 的 `f_flag` 与 `ST_RDONLY` 同为 `c_ulong`。）

Run: `cargo test -p issuerelay-core cli_path` 与 `cargo clippy -p issuerelay-core --all-targets -- -D warnings`
Expected: PASS（macOS 上含 `link` 子模块的 4 个读写测试）。

- [ ] **Step 3: 应用命令与登记（三处登记在同一步里改）**

先建面板与 Rust 测试共读的用例表 `app/src/lib/shell-line.json`（AI_CODING_GUIDE 第 12.9 节：面板重复 core 的事实时，两边读同一个文件）：

```json
{
  "shell_line": "export PATH=\"$HOME/.local/bin:$PATH\""
}
```

`app/src-tauri/src/commands.rs` 的测试模块加：

```rust
    #[test]
    fn the_shell_line_the_panel_shows_is_cores() {
        // The mock backend shows the line from the same file (`DEMO_SHELL_LINE`).
        let shared: serde_json::Value =
            serde_json::from_str(include_str!("../../src/lib/shell-line.json")).unwrap();
        assert_eq!(shared["shell_line"], issuerelay_core::cli_path::SHELL_LINE);
    }
```

`commands.rs` 文件头里关于 `Host::env()` 的那一句（「`Host::env()` holds the real proxy values … never return it or anything taken from it; …」）改为：

```rust
//! permission. `Host::env()` holds the real proxy values (user name and password unmasked):
//! neither it nor any proxy value ever reaches the panel. The commands use it to start
//! processes, and `cli_setup` reads its PATH only to tell whether `~/.local/bin` is on it, a fact
//! without secrets (the environment check shows the PATH anyway); the environment the panel sees
//! is `Engine::doctor`'s, with the proxy values masked.
```

`commands.rs` 的 `use issuerelay_core::{…}` 里加 `cli_path::{CliPathStatus, CliSetup}`、`env::EnvSource`，文件末尾（`request_quit` 之后）加：

```rust
/// What the PATH switch needs: the CLI next to the app, where the app runs from, and the login
/// shell's PATH the host resolved (only a PATH read from the login shell tells whether
/// `~/.local/bin` is on it). The PATH is only read here: the commands answer with what follows
/// from it (on PATH or not, and the fixed `SHELL_LINE`), never with the PATH itself.
async fn cli_setup(state: &State<'_, AppState>) -> CmdResult<CliSetup> {
    let env = host(state).await?.env();
    let shell_path = (env.path_source == EnvSource::LoginShell).then_some(env.path).flatten();
    CliSetup::for_running_app(cli_path(), shell_path).map_err(error)
}

/// Whether the command line is on PATH through IssueRelay (design 12).
#[tauri::command]
pub async fn cli_on_path(state: State<'_, AppState>) -> CmdResult<CliPathStatus> {
    issuerelay_core::cli_path::status(&cli_setup(&state).await?).map_err(error)
}

/// Puts the command line on PATH without administrator rights: `~/.local/bin` on macOS, the
/// user's own `Path` on Windows.
#[tauri::command]
pub async fn install_cli_on_path(state: State<'_, AppState>) -> CmdResult<CliPathStatus> {
    issuerelay_core::cli_path::install(&cli_setup(&state).await?).map_err(error)
}

/// Takes the command line off PATH; only what IssueRelay added is removed.
#[tauri::command]
pub async fn remove_cli_from_path(state: State<'_, AppState>) -> CmdResult<CliPathStatus> {
    issuerelay_core::cli_path::remove(&cli_setup(&state).await?).map_err(error)
}
```

（模块名 `cli_path` 与 M2 的函数 `cli_path()` 同名不冲突——一个在类型命名空间、一个在值命名空间——但为了好读，调用一律写全路径 `issuerelay_core::cli_path::status`。）

三处登记，同一步里改（AI_CODING_GUIDE 第 7 节「面板命令」）：

- `app/src-tauri/src/lib.rs` 的 `generate_handler!` 列表在 `commands::request_quit,` 之后加：

  ```rust
              commands::cli_on_path,
              commands::install_cli_on_path,
              commands::remove_cli_from_path,
  ```

- `app/src-tauri/build.rs` 的 `PANEL_COMMANDS` 在 `"request_quit",` 之后加：

  ```rust
      "cli_on_path",
      "install_cli_on_path",
      "remove_cli_from_path",
  ```

- `app/src-tauri/capabilities/default.json` 的 `permissions` 在 `"allow-request-quit"` 之后加三条（上一行补逗号），`description` 里「links, folders, notifications and autostart go through the commands」改为「links, folders, notifications, autostart and the command line on PATH go through the commands」：

  ```json
      "allow-request-quit",
      "allow-cli-on-path",
      "allow-install-cli-on-path",
      "allow-remove-cli-from-path"
  ```

命令共 37 条（M2 的 34 条加这 3 条）。前端的 `backend.ts`、`api.ts`、`mock.ts` 在 Step 7 补；在那之前 `api.test.ts` 的「calls exactly the commands its capability allows」会失败（能力文件多了三条 `allow-*`），这是预期的。

Run: `cargo test -p issuerelay-app` 与 `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: PASS，含 `the_panel_is_granted_exactly_the_registered_commands`、`the_acl_the_build_resolves_allows_only_the_panels_commands`（命令表、应用清单、能力文件三者一致）与 `the_shell_line_the_panel_shows_is_cores`。

- [ ] **Step 4: 命令行的失败测试**

`crates/cli/src/main.rs` 测试模块的 `use` 补 `std::{cell::Cell, collections::VecDeque, time::Duration}` 与 `issuerelay_core::ipc::client::NotRunning`，加：

```rust
    #[test]
    fn start_is_a_subcommand_of_its_own() {
        assert!(matches!(parse(&["start"]).command, Command::Start));
        // Listed with the others, so `no_subcommand_marks_done_retries_or_cancels` sees it too.
        let mut names = Vec::new();
        collect_names(&Cli::command(), &mut names);
        assert!(names.iter().any(|n| n == "start"));
    }

    /// Answers `start`'s status questions from a script: `Err(NotRunning)` until the list says
    /// otherwise.
    fn answers(script: Vec<anyhow::Result<Value>>) -> (impl Fn() -> std::future::Ready<anyhow::Result<Value>>, std::rc::Rc<Cell<usize>>) {
        let script = std::rc::Rc::new(std::cell::RefCell::new(VecDeque::from(script)));
        let asked = std::rc::Rc::new(Cell::new(0));
        let count = asked.clone();
        let ask = move || {
            count.set(count.get() + 1);
            std::future::ready(script.borrow_mut().pop_front().unwrap_or_else(|| Err(NotRunning.into())))
        };
        (ask, asked)
    }

    #[tokio::test]
    async fn start_launches_the_app_only_when_it_is_not_running() {
        let launched = Cell::new(0);
        let launch = || {
            launched.set(launched.get() + 1);
            Ok(())
        };
        let (ask, _) = answers(vec![Ok(json!({}))]);
        assert_eq!(start(ask, launch, 3, Duration::ZERO).await.unwrap(), "IssueRelay is already running.");
        assert_eq!(launched.get(), 0);

        let launch = || {
            launched.set(launched.get() + 1);
            Ok(())
        };
        let (ask, asked) = answers(vec![Err(NotRunning.into()), Err(NotRunning.into()), Ok(json!({}))]);
        assert_eq!(start(ask, launch, 3, Duration::ZERO).await.unwrap(), "IssueRelay started.");
        assert_eq!(launched.get(), 1);
        assert_eq!(asked.get(), 3);
    }

    #[tokio::test]
    async fn start_reports_an_app_that_never_answers_and_other_failures() {
        let (ask, _) = answers(vec![]);
        let error = start(ask, || Ok(()), 2, Duration::ZERO).await.unwrap_err();
        assert!(error.to_string().contains("did not answer"), "{error}");
        // A failure other than "not running" is no reason to start a second copy.
        let (ask, _) = answers(vec![Err(anyhow!("permission denied"))]);
        let launched = Cell::new(false);
        let error = start(ask, || {
            launched.set(true);
            Ok(())
        }, 2, Duration::ZERO)
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "permission denied");
        assert!(!launched.get());
        // The launcher's own failure is the command's.
        let (ask, _) = answers(vec![]);
        let error = start(ask, || Err(anyhow!("no app")), 2, Duration::ZERO).await.unwrap_err();
        assert_eq!(error.to_string(), "no app");
    }

    #[test]
    fn a_stopped_app_points_to_issuerelay_start() {
        let mut shown = Vec::new();
        fail(&anyhow::Error::new(NotRunning), &mut shown);
        assert_eq!(
            String::from_utf8(shown).unwrap(),
            "Error: IssueRelay is not running; start it with `issuerelay start`\n"
        );
        // Other failures get no hint.
        let mut shown = Vec::new();
        fail(&anyhow!("IssueRelay did not answer in time"), &mut shown);
        assert_eq!(String::from_utf8(shown).unwrap(), "Error: IssueRelay did not answer in time\n");
    }
```

`doctor_tells_a_terminal_it_is_waiting_and_nothing_else_does` 的 `quick` 列表加 `Command::Start`。

`crates/cli/tests/mcp_tools.rs` 的 `exposes_only_safe_control_tools_and_reports_engine_down` 在现有的 `assert!(text.starts_with("error:") && text.contains("not running"), …)` 之后加：

```rust
    // The MCP server never starts the app and never tells the AI to: the user opens it.
    assert!(text.contains("ask the user to open the IssueRelay app"), "{text}");
    assert!(!text.contains("issuerelay start"), "{text}");
```

Run: `cargo test -p issuerelay-cli`
Expected: 编译失败（`Command::Start`、`start` 不存在）。

- [ ] **Step 5: 实现 `start` 与两处提示**

`crates/cli/src/main.rs`：

- `use` 改为 `std::{future::Future, io::{self, IsTerminal, Write}, path::{Component, Path, PathBuf}, process::ExitCode, time::Duration}`、`issuerelay_core::{config::{…}, ipc::client::{self, NotRunning}, paths::AppPaths}`（`anyhow::Result` 不变；`Context` 只在两个平台的 `launch_app` 里局部导入，Linux 上不会出现未使用的导入）。
- `Command` 在 `Mcp` 前加：

```rust
    /// Start the IssueRelay tray app if it is not running, and wait until it answers.
    Start,
```

- `request` 的 `Command::Mcp => unreachable!(…)` 改为 `Command::Mcp | Command::Start => unreachable!("handled before sending a request"),`。
- 在 `announce_wait` 之后加：

```rust
/// Printed after "IssueRelay is not running" in the CLI's own failure line.
const NOT_RUNNING_HINT: &str = "start it with `issuerelay start`";
/// How long `issuerelay start` waits for the started app to answer: 40 tries, 0.5 s apart.
const START_ATTEMPTS: u32 = 40;
const START_POLL: Duration = Duration::from_millis(500);

/// `issuerelay start`: launches the tray app unless it already answers, then waits until it
/// does. `ask` asks the host for its status and `launch` starts the app; both are passed in so
/// the tests run neither. Only "not running" leads to a launch: any other failure (no
/// permission, another user's pipe) is reported instead of starting a second copy.
async fn start<A, Fut>(ask: A, launch: impl FnOnce() -> Result<()>, attempts: u32, every: Duration) -> Result<String>
where
    A: Fn() -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    match ask().await {
        Ok(_) => return Ok("IssueRelay is already running.".into()),
        Err(e) if e.downcast_ref::<NotRunning>().is_none() => return Err(e),
        Err(_) => {}
    }
    launch()?;
    for _ in 0..attempts {
        tokio::time::sleep(every).await;
        match ask().await {
            Ok(_) => return Ok("IssueRelay started.".into()),
            Err(e) if e.downcast_ref::<NotRunning>().is_none() => return Err(e),
            Err(_) => {}
        }
    }
    anyhow::bail!(
        "IssueRelay was started but did not answer within {} seconds; open it from the menu bar or the notification area to see why",
        (every * attempts).as_secs()
    )
}

/// Starts the tray app through Launch Services, which finds the installed app by its bundle
/// identifier. Fixed arguments, no shell.
#[cfg(target_os = "macos")]
fn launch_app() -> Result<()> {
    use anyhow::Context as _;
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", issuerelay_core::paths::APP_ID])
        .status()
        .context("could not run /usr/bin/open")?;
    anyhow::ensure!(status.success(), "macOS could not find the IssueRelay app; open it once from Applications");
    Ok(())
}

/// Starts `issuerelay-app.exe` from this program's own folder, detached from this console so
/// that closing the terminal or Ctrl+C leaves the tray app running. Fixed arguments, no shell.
#[cfg(windows)]
fn launch_app() -> Result<()> {
    use std::os::windows::process::CommandExt;

    use anyhow::Context as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let app = std::env::current_exe()?.with_file_name(format!("{}.exe", issuerelay_core::paths::APP_EXECUTABLE));
    anyhow::ensure!(app.is_file(), "{} is missing; start IssueRelay from the Start menu", app.display());
    std::process::Command::new(&app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .with_context(|| format!("could not start {}", app.display()))?;
    Ok(())
}

/// Linux is not packaged (design 2.2): the user starts the tray app.
#[cfg(not(any(target_os = "macos", windows)))]
fn launch_app() -> Result<()> {
    anyhow::bail!("`issuerelay start` works on macOS and Windows; start the IssueRelay app yourself")
}
```

- `fail` 改为（一行清洗过的原因；宿主没运行时补上提示）：

```rust
fn fail(error: &anyhow::Error, err: &mut impl Write) -> ExitCode {
    let mut line = format::error_line(error);
    if error.downcast_ref::<NotRunning>().is_some() {
        line = format!("{line}; {NOT_RUNNING_HINT}");
    }
    let _ = writeln!(err, "{line}");
    ExitCode::FAILURE
}
```

- `run` 里 `if matches!(cli.command, Command::Mcp) { … }` 之后加：

```rust
    if matches!(cli.command, Command::Start) {
        let text = start(|| client::call(&endpoint, "status", json!({})), launch_app, START_ATTEMPTS, START_POLL).await?;
        write_line(&mut io::stdout().lock(), &text)?;
        return Ok(());
    }
```

并在 `Command::Mcp` 那一支上方加注释：`// The MCP server never starts the tray app: an AI must not start a program with full access by itself; a stopped app is an error the AI passes on to the user.`

`crates/cli/src/mcp.rs`：`use` 补 `issuerelay_core::ipc::client::NotRunning`，加常量并改 `forward` 的错误分支：

```rust
/// Added to a tool's "not running" error. The MCP server does not start the app, and the AI is
/// asked to leave that to the user rather than to start a program with full access itself.
pub const MCP_NOT_RUNNING_HINT: &str = "ask the user to open the IssueRelay app; this server does not start it";
```

```rust
            Err(e) if e.downcast_ref::<NotRunning>().is_some() => {
                Err(format::clean(&format!("error: {e:#}; {MCP_NOT_RUNNING_HINT}")))
            }
            Err(e) => Err(format::clean(&format!("error: {e:#}"))),
```

`forward` 的文档注释补一句：「A stopped app gets `MCP_NOT_RUNNING_HINT`; the server never starts it.」

Run: `cargo test -p issuerelay-cli` 与 `cargo clippy -p issuerelay-cli --all-targets -- -D warnings`
Expected: PASS。

- [ ] **Step 6: 前端的失败测试**

`app/src/pages/Settings.test.tsx` 加（导入补 `import { DEMO_SHELL_LINE } from "@/lib/mock-fixtures";`，`import type` 那一行补 `CliPathStatus`）。PATH 一行照 M2 的 `Autostart`：状态读到之前是加载行，没有开关，所以 `findByRole` 等的是数据真正到了，点下去的开关不会是读取中被禁用的那个（第 12.6 节；TESTING_SPEC「等条件，不等固定时间」）：

```tsx
test("the command line goes on PATH from the system group and the shell line is shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const install = vi.spyOn(backend, "installCliOnPath");
  await renderWithProviders(<Settings />, { backend });
  // Only there once the state is read: before that the row is a loading row.
  const toggle = await screen.findByRole("switch", { name: "Command line on PATH" });
  expect(toggle).not.toBeChecked();
  expect(toggle).toBeEnabled();
  await userEvent.click(toggle);
  expect(install).toHaveBeenCalledTimes(1);
  expect(await screen.findByText(DEMO_SHELL_LINE)).toBeInTheDocument();
  expect(screen.getByText(/Add this line to your shell profile/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeChecked();
});

test("a file IssueRelay did not make blocks the switch and is named, cleaned", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "blocked", path: "~/.local/bin/issuerelay\u001b[31m" });
  await renderWithProviders(<Settings />, { backend });
  const note = await screen.findByText(/already exists and was not made by IssueRelay/);
  expect(note.textContent).toContain("~/.local/bin/issuerelay");
  // The system's text goes through cleanDisplayText (AI_CODING_GUIDE 12.4).
  expect(note.textContent).not.toContain("\u001b");
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeDisabled();
});

test("from the disk image the switch explains what to do first", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "unavailable", reason: "not_installed" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText(/Move it to Applications first/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeDisabled();
});

test("when the PATH setting cannot be read the row says so, with a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = vi.fn().mockRejectedValue(new BackendError("cannot read the link"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not read the PATH setting")).toBeInTheDocument();
  expect(screen.getByText("cannot read the link")).toBeInTheDocument();
  expect(screen.queryByRole("switch", { name: "Command line on PATH" })).not.toBeInTheDocument();
  const system = screen.getByRole("region", { name: "System" });
  await userEvent.click(within(system).getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(backend.cliOnPath).toHaveBeenCalledTimes(2));
});
```

Run: `pnpm --dir app test`
Expected: 失败（`installCliOnPath`、`DEMO_SHELL_LINE` 等不存在；`api.test.ts` 的边界测试也失败，因为能力文件已有三条新命令而 `api.ts` 还没有）。

- [ ] **Step 7: 前端实现**

`app/src/lib/types.ts` 加：

```ts
export type CliUnavailable = "missing_cli" | "not_installed" | "development";
export type CliPathStatus =
  | { kind: "off" }
  | { kind: "on"; shell_line: string | null }
  | { kind: "stale"; target: string }
  | { kind: "blocked"; path: string }
  | { kind: "unavailable"; reason: CliUnavailable };
```

`app/src/lib/backend.ts` 的 `Backend` 在 `setAutostart` 之后加 `cliOnPath(): Promise<CliPathStatus>;`、`installCliOnPath(): Promise<CliPathStatus>;`、`removeCliFromPath(): Promise<CliPathStatus>;`（类型导入补 `CliPathStatus`）。`api.ts` 的 `tauriBackend` 在 `setAutostart` 之后加：

```ts
  cliOnPath: () => call("cli_on_path"),
  installCliOnPath: () => call("install_cli_on_path"),
  removeCliFromPath: () => call("remove_cli_from_path"),
```

`mock-fixtures.ts` 加（shell 那一行只有 `shell-line.json` 一份，`commands.rs` 的测试拿它对照 core 的 `SHELL_LINE`）：

```ts
import shellLine from "./shell-line.json";

/** The line the panel shows when ~/.local/bin is not on the login shell's PATH: core's `cli_path::SHELL_LINE`. */
export const DEMO_SHELL_LINE: string = shellLine.shell_line;
```

`mock.ts`：从 `./mock-fixtures` 的导入补 `DEMO_SHELL_LINE`，类型导入补 `CliPathStatus`；在 `let autostart = false;` 之后加 `let cliPath: CliPathStatus = { kind: "off" };`，返回对象在 `setAutostart` 之后加：

```ts
    cliOnPath: () => reply(() => cliPath),
    installCliOnPath: () =>
      reply(() => {
        // The demo user's login shell does not have ~/.local/bin yet, so the line is shown.
        cliPath = { kind: "on", shell_line: DEMO_SHELL_LINE };
        return cliPath;
      }),
    removeCliFromPath: () =>
      reply(() => {
        cliPath = { kind: "off" };
        return cliPath;
      }),
```

`hooks/queries.ts`：`queryKeys` 加 `cliPath: ["cliPath"] as const,`，类型导入补 `CliPathStatus`，在 `useAutostart` 之后加查询，在 `useSetAutostart` 之后加两个变更钩子（照 `useAction` 的签名，写在它旁边，不导出它）：

```ts
/** Whether the command line is on PATH through IssueRelay, and what is still to do (design 12). */
export function useCliPath() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.cliPath, queryFn: () => backend.cliOnPath() });
}
```

```ts
export function useInstallCliOnPath() {
  return useAction<void, CliPathStatus>((backend) => backend.installCliOnPath(), [queryKeys.cliPath], "actions.failed.cli_path");
}

export function useRemoveCliFromPath() {
  return useAction<void, CliPathStatus>((backend) => backend.removeCliFromPath(), [queryKeys.cliPath], "actions.failed.cli_path");
}
```

`pages/Settings.tsx`：从 `@/hooks/queries` 的导入补 `useCliPath`、`useInstallCliOnPath`、`useRemoveCliFromPath`，补 `import { useCopyText } from "@/hooks/useCopyText";`，类型导入补 `CliPathStatus`；在 `Autostart` 之后加两个组件，「系统」分组里 `<Autostart />` 下面放 `<CliOnPath />`。换行规则：开关一行沿用 `SwitchRow`（名字在左、开关在右，名字很短）；shell 那一行是 `wrap-anywhere` 的等宽文字，「复制」按钮在同一个 `flex-wrap` 容器里，605 px 放不下时换到下一行；`stale` / `blocked` 的路径在句子里，整句 `wrap-anywhere`（第 12.7 节）：

```tsx
/** What the PATH row says under its switch. The paths are the system's text: cleaned, and allowed to break anywhere. */
function CliPathNote({ status }: { status: CliPathStatus }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  switch (status.kind) {
    case "off":
      return null;
    case "on": {
      const line = status.shell_line;
      if (line === null) return <p className="text-xs text-muted-foreground">{t("settings.cli.on_new_terminal")}</p>;
      return (
        <div className="flex flex-col gap-1">
          <p className="text-xs text-muted-foreground">{t("settings.cli.shell_line")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <code className="min-w-0 rounded bg-muted px-2 py-1 font-mono text-xs wrap-anywhere text-foreground select-text">{line}</code>
            <Button size="sm" variant="outline" onClick={() => copy(line)}>
              {t("common.copy")}
            </Button>
          </div>
        </div>
      );
    }
    case "stale":
      return <p className="text-xs text-muted-foreground wrap-anywhere">{t("settings.cli.stale", { target: cleanDisplayText(status.target) })}</p>;
    case "blocked":
      return <p className="text-xs text-destructive wrap-anywhere">{t("settings.cli.blocked", { path: cleanDisplayText(status.path) })}</p>;
    case "unavailable":
      return <p className="text-xs text-muted-foreground">{t(`settings.cli.unavailable.${status.reason}`)}</p>;
  }
}

/**
 * Putting the command line on PATH (design 12). It acts at once and is not saved with the page,
 * like starting at login; the switch shows only once the state is read, so it is never shown in a
 * state it is not in, and it cannot be used while a file IssueRelay did not make is in the way or
 * where the app runs from does not allow it.
 */
function CliOnPath() {
  const { t } = useTranslation();
  const id = useId();
  const status = useCliPath();
  const install = useInstallCliOnPath();
  const remove = useRemoveCliFromPath();
  if (status.isError && !status.isFetching) {
    return <ErrorState error={status.error} headline="settings.cli.load_failed" onRetry={() => void status.refetch()} />;
  }
  if (status.data === undefined) return <LoadingState rows={1} />;
  const current = status.data;
  return (
    <div className="flex flex-col gap-1">
      <SwitchRow
        id={id}
        label={t("settings.cli.title")}
        note={t("settings.cli.description")}
        checked={current.kind === "on"}
        disabled={install.isPending || remove.isPending || current.kind === "blocked" || current.kind === "unavailable"}
        onChange={(on) => (on ? install.mutate() : remove.mutate())}
      />
      <CliPathNote status={current} />
    </div>
  );
}
```

翻译键（`en` / `zh-CN` 如下；`zh-TW`、`ja`、`ko` 按 M2 Task 2 的写法同步翻译，`{{…}}` 与命令文本原样保留）。`missing_cli` 只会出现在已安装的应用里（开发版先得到 `development`，`CliSetup::usable_cli` 先看位置），所以它的文字只针对安装包：

| 键 | en | zh-CN |
|---|---|---|
| `settings.cli.title` | Command line on PATH | 把命令行加入 PATH |
| `settings.cli.description` | Lets you type `issuerelay` in a terminal. No administrator rights needed. | 之后在终端里可以直接输入 `issuerelay`，不需要管理员权限。 |
| `settings.cli.on_new_terminal` | On. Open a new terminal to use it. | 已加入。新开一个终端即可使用。 |
| `settings.cli.shell_line` | Add this line to your shell profile (for zsh, ~/.zshrc), then open a new terminal: | 把下面这一行加到 shell 配置文件（zsh 是 ~/.zshrc），再新开一个终端： |
| `settings.cli.stale` | The link points at another copy of IssueRelay ({{target}}). Turn this on to point it here. | 链接指向另一份 IssueRelay（{{target}}）。打开开关即改为指向这一份。 |
| `settings.cli.blocked` | {{path}} already exists and was not made by IssueRelay. Remove or rename it, then try again. | {{path}} 已存在，且不是 IssueRelay 建的。删除或改名后再试。 |
| `settings.cli.unavailable.missing_cli` | The command-line program is missing from this copy of IssueRelay. Install IssueRelay again from the release page. | 这份 IssueRelay 里缺少命令行程序。请从发布页重新安装 IssueRelay。 |
| `settings.cli.unavailable.not_installed` | IssueRelay runs from the disk image or a temporary copy. Move it to Applications first. | IssueRelay 正从磁盘映像或临时副本运行。请先把它移到「应用程序」。 |
| `settings.cli.unavailable.development` | Not available in a development build. | 开发版不提供这一项。 |
| `settings.cli.load_failed` | Could not read the PATH setting | 读不到 PATH 设置 |
| `actions.failed.cli_path` | Could not change the command line on PATH | 没能修改「把命令行加入 PATH」 |

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS（含缺键守卫、五种语言键对齐检查，以及不改一字的 `api.test.ts`：`tauriBackend` 调到的命令正好是能力文件给的 37 条，监听仍是 4 个）。

再量布局（AI_CODING_GUIDE 第 12.7 节）：`pnpm --dir app dev` 打开设置页（假后端），打开开关让 shell 那一行出现；再用假后端把 `cliOnPath` 临时改成 `blocked`（路径用一个很长的虚构路径，例如 `/tmp/demo-home/` 加 80 个字符的目录名）看一眼，看完改回。浏览器视口宽 900 px 与 885 px（内容区约 620 px 与 605 px），浅色与深色各一次：开关够得着，shell 那一行与路径断行而不横向溢出，「复制」按钮放不下时在下一行。结果写进提交说明。M2 Task 12 的 Playwright 布局测试若已落地，在它的设置页用例里加上打开了 PATH 开关的状态。

- [ ] **Step 8: 文档与提交**

事实登记（跟代码同一个提交）：

- 设计第 4.1 节「命令行小程序」一条改为：「…托盘程序没在运行时给出提示：命令行指向 `issuerelay start`，它启动托盘程序（macOS 经 `open -b com.sunkailun.issuerelay`，Windows 启动同目录的 `issuerelay-app.exe`），已在运行就只说明，并等到宿主应答；MCP 服务只报错，请 AI 转告用户打开应用，从不自己启动（AI 不能静默启动有完全权限的程序）。」第 10.1 节的子命令列表加 `start`；第 10.3 节补「宿主没运行时工具返回错误（`isError`），文字里请 AI 转告用户打开 IssueRelay」；第 12 节「把命令行加入 PATH」写实际行为：「macOS 在 `~/.local/bin/issuerelay` 建指向应用包内命令行程序的链接（不覆盖别人的文件，旧链接自动改指向；`~/.local/bin` 不在登录 shell 的 PATH 里时面板给出要加的一行，不改 shell 配置文件；从磁盘映像运行时不建）；Windows 把安装目录加进用户级 `Path`（保持 `REG_EXPAND_SZ`，不分大小写去重，广播 `WM_SETTINGCHANGE`，移除时只删这一项）。都不需要管理员。注册 MCP 控制服务用的是命令行程序的绝对路径，不依赖 PATH。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 把命令行加入 PATH、应用安装位置 | crates/core/src/cli_path.rs |`。
- `docs/ai-coding/TESTING_SPEC.md`「面板与 core 共用的用例表」加一行：`| app/src/lib/shell-line.json | mock-fixtures.ts 的 DEMO_SHELL_LINE（Settings 的测试按它找那一行） | app/src-tauri/src/commands.rs 的测试，对照 core 的 cli_path::SHELL_LINE |`。
- `.github/CODEOWNERS` 加 `/crates/core/src/cli_path.rs   @sunkailun1992`（会改用户的 PATH 与注册表）。

Run: `pnpm --dir app build` 与六条验证命令
Expected: 全部通过。

先提交规则（裁定 F14，只含规则文件，第 11 节）：`docs/ai-coding/SECURITY_CODING_SPEC.md` 第 5 节最后几条里的「`Host::env()` 带着真实的代理值（用户名和密码没打码）：命令只拿它启动进程，从不返回它或由它得出的任何东西；面板看到的执行环境是 `Engine::doctor` 给的、代理已打码的那份。」整句改为：

「`Host::env()` 带着真实的代理值（用户名和密码没打码）：它本身和任何代理值都不交给网页视图，命令拿它启动进程；由其中 PATH 得出的不含秘密的事实可以返回（`cli_on_path` 只回答 `~/.local/bin` 在不在登录 shell 的 PATH 里，外加固定的那一行 shell 命令；环境检查的报告本来就带着 PATH）；面板看到的执行环境是 `Engine::doctor` 给的、代理已打码的那份。」

```bash
git status --short
git add docs/ai-coding/SECURITY_CODING_SPEC.md
git diff --cached --stat
git commit -F - <<'EOF'
docs(ai-coding): let the panel see facts derived from the PATH

Host::env() and the proxy values still never reach the webview; a command may answer
with a non-secret fact derived from the PATH (whether ~/.local/bin is on it), as the
environment check already shows the PATH.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

再提交代码：

```bash
git status --short
git add crates/core/src/cli_path.rs crates/core/src/lib.rs crates/core/Cargo.toml Cargo.lock \
  app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs app/src-tauri/build.rs app/src-tauri/capabilities/default.json \
  crates/cli/src/main.rs crates/cli/src/mcp.rs crates/cli/tests/mcp_tools.rs \
  app/src/lib/shell-line.json app/src/lib/types.ts app/src/lib/backend.ts app/src/lib/api.ts app/src/lib/mock.ts \
  app/src/lib/mock-fixtures.ts app/src/hooks/queries.ts app/src/pages/Settings.tsx app/src/pages/Settings.test.tsx \
  app/src/i18n/locales/en.json app/src/i18n/locales/zh-CN.json app/src/i18n/locales/zh-TW.json \
  app/src/i18n/locales/ja.json app/src/i18n/locales/ko.json \
  docs/superpowers/specs/2026-10-10-issuerelay-design.md docs/ai-coding/AI_CODING_GUIDE.md \
  docs/ai-coding/TESTING_SPEC.md .github/CODEOWNERS
git diff --cached --stat
git commit -F - <<'EOF'
feat: put the CLI on PATH from the settings and start the tray app from the CLI

core cli_path decides and does it without administrator rights (a ~/.local/bin link on
macOS, the user's own Path on Windows); the panel gets three commands, registered in
generate_handler!, PANEL_COMMANDS and the capability (37 commands). `issuerelay start`
launches the tray app; the MCP server never does.

Layout: the PATH row checked at 620 px and 605 px content width, light and dark; the
shell line and long paths wrap anywhere, Copy moves under the line when needed.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

---

### Task 3: 自动更新

设计第 12 节（Tauri updater，更新包用单独的签名密钥签名，私钥只放在 GitHub Actions secrets，从 GitHub Releases 拉取）与第 9.5 节（通知）。调研结论（`tauri-plugin-updater` 2.13.3、`tauri-cli` 2.12.2 的源码）：

- `plugins.updater.pubkey` 必填，值是 `.pub` 文件那一行 base64 的内容（不接受文件路径）；配置缺了它，插件初始化就失败，应用起不来——所以公钥必须在注册插件的同一个提交里写进 `tauri.conf.json`。
- `bundle.createUpdaterArtifacts: true` 让打包器签名更新包，没有私钥时打包失败，所以基础配置保持 `false`，发布工作流用 `--config src-tauri/tauri.updater.conf.json` 打开。产物：macOS `IssueRelay.app.tar.gz` 与 `.sig`；Windows 安装包本身（`-setup.exe`、`.msi`）与各自的 `.sig`。
- 私钥经环境变量 `TAURI_SIGNING_PRIVATE_KEY`（内容或路径）给构建，密码 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；设置了 `CI` 而没有密码变量时用空密码。签名私钥与公钥不配对时 `tauri build` 只警告、不失败（Task 4 用脚本核对）。`tauri-cli` 2.12.0 起签名的可信注释里记录应用版本，`requireSignedVersion: true` 要求它与清单宣布的版本一致，拦住「高版本号配旧的合法包」的篡改。
- `Updater::check()`：远端版本不比当前新（语义化版本比较，`0.1.0-rc.1` 小于 `0.1.0`）时返回 `Ok(None)`；清单没有本平台条目时返回 `TargetsNotFound` 错误（即使版本不新）；拿不到有效清单是 `ReleaseNotFound` 或网络、解析错误。`Update::download` 先验签再返回字节，验签失败（`Minisign` 等）什么都不留；`Update::install(bytes)` 在 macOS 原地替换应用包（2.13.2 起失败不会删掉已装的应用），在 Windows 启动安装程序（NSIS 带 `/P /UPDATE /R`）后立即退出本进程。
- 重启：`AppHandle::restart()` 在主线程上调用时跳过 `Exit` 事件，单实例插件的锁可能还没释放，新实例会把参数交给旧实例后自己退出，结果程序没在运行；在其他线程调用、或用 `request_restart()` 时先发 `Exit`。本任务在后台线程装完之后经退出状态机的 `Restart` 一步调用 `request_restart()`。macOS 上新实例经 Launch Services 从应用包启动。
- 安装发生在宿主停下之后、重启之前（`QuitState::Installing`）：这段时间进程不能因为关窗或另一次退出请求而结束，否则 macOS 上换到一半的应用包或 Windows 上没启动的安装程序会留下问题。装不上时程序按原版本重启（宿主已经停了，不能就地恢复），原因写进数据目录里的一个小文件，重启后由面板顶部的横幅显示一次——不发通知（裁定 F15）。
- `tauri-plugin-process` 只给前端的 `relaunch()` 用；本计划只在 Rust 侧调用，不加它（草稿里的这个依赖删掉）。更新插件只在 Rust 侧用，`capabilities/default.json` 不加插件权限，只加本程序三条新命令的 `allow-*`。
- `latest.json` 的 `notes` 是 Release 的正文（Task 4 写的是安装说明链接与签名、自动更新的实际情况，给下载页看的），不是写给已安装程序看的更新说明：面板不显示它，`UpdateStatus` 也不带它。

**Step 0 由主智能体（控制者）在派发本任务之前执行**：在维护者本机生成更新签名密钥对，并把公钥写进 `tauri.conf.json`；不交给子智能体，不在云端会话做。实现者从 Step 1 开始，`app/src-tauri/tauri.conf.json` 里已经有 `plugins.updater`。

**Files:**
- Modify: `app/src-tauri/tauri.conf.json`（Step 0 写入 `plugins.updater`）
- Create: `app/src-tauri/tauri.updater.conf.json`
- Modify: `app/src-tauri/Cargo.toml`、`Cargo.lock`（加 `tauri-plugin-updater`、`semver`；`chrono` 从 `[dev-dependencies]` 移到 `[dependencies]`，带 `serde` 特性；`tokio` 已是 `full`，不动）
- Create: `app/src-tauri/src/update.rs`
- Modify: `app/src-tauri/src/lib.rs`（模块、插件、状态、后台检查、`generate_handler!`）、`app/src-tauri/build.rs`（`PANEL_COMMANDS`）、`app/src-tauri/capabilities/default.json`（三条 `allow-*`）、`app/src-tauri/src/quit.rs`、`app/src-tauri/src/events.rs`（`UPDATE_STATUS`；`QUIT_REQUESTED` 的文档）、`app/src-tauri/src/notify.rs`（新版本通知沿用同一套翻译与发送；模块文档）、`app/src-tauri/src/commands.rs`、`app/src-tauri/src/i18n.rs`（`USED_KEYS`）、`app/src-tauri/tests/packaging.rs`
- Modify: `crates/core/src/config.rs`（`auto_update` 的文档注释）
- Create: `app/src/lib/update-failure-cases.json`（更新失败代码 ↔ 错误代码 ↔ 文案键，Rust 与 TS 的测试共读）
- Modify: `app/src/lib/{types,backend,api,mock,errors}.ts`、`app/src/lib/errors.test.ts`、`app/src/lib/api.test.ts`（监听 4 → 5 个）、`app/src/hooks/{queries,useEngineEvents}.ts`、`app/src/components/QuitDialog.tsx`、`app/src/components/QuitDialog.test.tsx`（M2 的七个测试全部保留、改用 `QuitPrompt`，另加三个）、`app/src/App.test.tsx`（启动失败页上的退出询问改用 `QuitPrompt`）、`app/src/components/layout/AppShell.tsx`、`app/src/pages/Settings.tsx`、`app/src/pages/Settings.test.tsx`（M2 的「automatic updates … take effect later」测试按新旁注改写）、五个翻译文件
- Create: `app/src/components/UpdateBanner.tsx`、`app/src/components/UpdateBanner.test.tsx`
- Modify: `scripts/check-secrets.sh`（拦截更新签名私钥）
- Create: `docs/release/updater-key.md`
- Modify（事实登记，跟代码同一个提交）：`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 9.5、12 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 5 节关键文件、第 6 节运行数据）、`docs/ai-coding/TESTING_SPEC.md`（假实现清单、共用用例表）、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 5 节）、`.github/CODEOWNERS`（`tauri.conf.json`、`tauri.*.conf.json`）
- Modify（规则，单独的 `docs` 提交）：`docs/ai-coding/AI_CODING_GUIDE.md`（第 7 节「面板的类型」一句）、`docs/ai-coding/SECURITY_CODING_SPEC.md`（自动更新一段）

**Interfaces:**
- Consumes（M2，2ebda38）：`quit.rs` 的 `QuitState::{Idle, Asking, Waiting, Stopping, Done}`、`QuitEvent::{Requested { host_ready, running }, Wait, StopNow, Stopped, WindowClosed}`、`QuitStep::{Nothing, Ask(usize), ShowPanel, Stop(StopMode), HideWindow, Exit}`、纯函数 `next`、`may_exit`，泛型的 `handle<R: Runtime>(app: &AppHandle<R>, event) -> QuitStep`、`request<R: Runtime>`、`on_run_event<R: Runtime>`、私有的 `no_host_to_stop<R: Runtime>`，测试模块的 `exit_requests_while(state)` 与 `every_exit_request_before_quitting_is_done_is_held`；`test_support::run_mock_app`；`window::on_event`（`HideWindow` 就拦下关窗）；`events::QUIT_REQUESTED`；`state::AppState::{host, try_host, quit}`；`logging::debug_failed`；`i18n::{Translator, USED_KEYS}`（`Translator::for_settings`、`t`、`t_with(key, &[(name, value)])`）；`notify::show` 的发送写法；`Host::engine().settings().auto_update`、`Host::engine().status().running`；`commands.rs` 的 `CmdResult`；`issuerelay_core::paths::AppPaths::system()` 与 `data_dir`；`issuerelay_core::text::clean_text`；Task 2 的 `issuerelay_core::cli_path::{AppLocation, running_app_location}` 与 37 条命令；前端 `Backend`、`BackendError`、`ERROR_CODES`（`not_a_linear_link`、`login_cancelled`）、`describeError`、`useAction<Variables, Result>`、`queryKeys`、`useAppInfo`、`useLang`、`formatRelative(iso, lang)`、`DetailLine`、`ErrorState`、`subscription`、`useEngineEvents`（返回停掉每个订阅的清理函数）、`QuitDialog`（M2 的 `quit.title`、`quit.body`、`quit.wait`、`quit.stop`、`quit.cancel`、`quit.waiting`）、`AppShell` 内容区的 `<Outlet />`、Settings 的「系统」分组、`SwitchRow` 与 `auto_update` 开关、翻译键 `shell.version`、`renderWithProviders`（返回 `client`）、`createMockBackend`、shadcn `Alert` 与 `Button`。
- Produces（Rust）：
  - `update::FIRST_CHECK`（60 s）、`update::CHECK_EVERY`（6 h）、`update::INSTALL_FAILURE_FILE`（`"update-install-failure.txt"`）
  - `pub enum FailureCode { Unreachable, NoPlatform, BadSignature, Unavailable, InstallFailed, Other }`（serde snake_case）、`FailureCode::ALL`、`FailureCode::error_code(self) -> &'static str`（`update_unreachable`、`update_no_platform`、`update_bad_signature`、`update_unavailable`、`update_install_failed`、`update_failed`；三者的对应由 `app/src/lib/update-failure-cases.json` 钉住）、`pub struct Failure { pub code: FailureCode, pub detail: String }`、`Failure::new(code, detail)`
  - `pub enum UpdateStatus { Unchecked, UpToDate { checked_at }, Available { version }, Failed(Failure) }`（serde `tag = "kind"`：`{"kind":"unchecked"}`、`{"kind":"up_to_date","checked_at":"…"}`、`{"kind":"available","version":"0.2.0"}`、`{"kind":"failed","code":"unreachable","detail":"…"}`）
  - `pub trait UpdateSource`（`type Offer`、`version`、`check`、`download`）、`pub struct PluginSource<R: Runtime>(pub AppHandle<R>)`、`pub struct Pending<O> { pub offer: Arc<O>, pub bytes: Vec<u8> }`、`pub struct Updates<S>`：`new(source, current)`、`status()`、`show_failure(failure)`、`check()`、`prepare()`、`pending_version()`、`take_pending()`、`clear_pending()`、`should_notify(version)`；`pub type AppUpdates<R = tauri::Wry> = Updates<PluginSource<R>>`；`pub fn is_newer(current: &str, offered: &str) -> bool`、`pub fn failure_code(&tauri_plugin_updater::Error) -> FailureCode`；`pub fn record_install_failure(data_dir: &Path, detail: &str)`、`pub fn take_install_failure(data_dir: &Path) -> Option<Failure>`
  - 泛型的应用函数（`R: tauri::Runtime`）：`update::{pending, take_pending, clear_pending}(app: &AppHandle<R>)`、`async fn update::check(app) -> UpdateStatus`、`async fn update::install(app) -> Result<(), Failure>`、`fn update::install_and_restart(app, pending)`、`fn update::spawn_checker(app: AppHandle<R>)`
  - `quit`：`QuitState::Installing`、`QuitEvent::Installed`、`QuitStep::{Install, Restart}`、纯函数 `installing_instead(outcome, update_pending)`；`pub enum QuitPurpose { Quit, Update }`、`pub struct QuitPrompt { pub running: usize, pub purpose: QuitPurpose }`（`quit-requested` 的载荷从「运行中的条数」改为 `{"running":2,"purpose":"quit"|"update"}`）；泛型的 `request<R>`（先丢掉已下载的更新）、`request_update<R>`、私有的 `ask<R>`
  - `notify::show_update_available<R: Runtime>(app, host, version)`（第五种通知没有自己的开关：只有后台检查调用它，而后台检查只在 `auto_update` 打开时运行）
  - `events::UPDATE_STATUS = "update-status"`（载荷：`UpdateStatus`）
  - 命令：`update_status() -> UpdateStatus`（立即返回）、`check_update() -> UpdateStatus`、`install_update() -> Result<(), String>`（失败时是 `FailureCode::error_code`），登记在 `generate_handler!`、`PANEL_COMMANDS`、能力文件（`allow-update-status`、`allow-check-update`、`allow-install-update`）；命令共 40 条
- Produces（前端）：类型 `UpdateStatus`、`UpdateFailureCode`、`QuitPurpose`、`QuitPrompt`；`Backend.updateStatus()`、`checkUpdate()`、`installUpdate()`、`onUpdateStatus(listener)`，`onQuitRequested(listener: (prompt: QuitPrompt) => void)`；`ERROR_CODES` 保留 `not_a_linear_link`、`login_cancelled`，再加六个 `update_*`；`queryKeys.update`、`useUpdateStatus()`、`useCheckUpdate()`、`useInstallUpdate()`；`updateFailureKey(code)`（`lib/errors.ts`）；`UpdateBanner()`。
- Produces（密钥）：`~/.tauri/issuerelay.key`（私钥，600，不进仓库）、`~/.tauri/issuerelay.key.pub`；`tauri.conf.json` 的 `plugins.updater.pubkey`；文档 `docs/release/updater-key.md`。

- [ ] **Step 0（主智能体）: 生成密钥对并写入公钥**

在维护者的 Mac 上、仓库根目录运行。私钥从不显示：命令的标准输出直接丢弃（带 `-w` 时它只含两个路径，丢弃是预防），之后只看文件权限、只读 `.pub`：

```bash
test ! -e ~/.tauri/issuerelay.key || { echo "~/.tauri/issuerelay.key exists already: stop and ask the maintainer"; exit 1; }
mkdir -p ~/.tauri && chmod 700 ~/.tauri
# --ci: no prompts and no password (GitHub secrets cannot be empty, so no password secret is
# needed, and no password ever has to pass through the chat).
pnpm --dir app tauri signer generate --ci -w ~/.tauri/issuerelay.key > /dev/null
chmod 600 ~/.tauri/issuerelay.key
stat -f '%Sp %z bytes' ~/.tauri/issuerelay.key ~/.tauri/issuerelay.key.pub
```

Expected：两行，第一行（私钥）以 `-rw-------` 开头，第二行（公钥）以 `-rw-r--r--` 开头，字节数不必核对；没有别的输出（CLI 关于空密码的警告写在标准错误上，可以出现）。若 `generate` 报错，停下来报告错误原文（错误里不会有私钥：私钥只写进文件）。

把公钥写进 `tauri.conf.json`（这段只读 `.pub` 文件）：

```bash
node -e '
const fs = require("fs");
const os = require("os");
const file = "app/src-tauri/tauri.conf.json";
const config = JSON.parse(fs.readFileSync(file, "utf8"));
config.plugins = {
  ...(config.plugins ?? {}),
  updater: {
    pubkey: fs.readFileSync(os.homedir() + "/.tauri/issuerelay.key.pub", "utf8").trim(),
    endpoints: ["https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json"],
    windows: { installMode: "passive" },
    requireSignedVersion: true,
  },
};
fs.writeFileSync(file, JSON.stringify(config, null, 2) + "\n");
'
git diff --stat app/src-tauri/tauri.conf.json
```

（`JSON.stringify` 会把行内数组展开成多行，只是排版。）结果里 `plugins.updater` 为：`pubkey`（`.pub` 文件那一行）、`endpoints`（上面固定的一条）、`windows.installMode: "passive"`（Windows 上显示一个小进度窗、不需要操作；NSIS 按用户安装不弹管理员授权）、`requireSignedVersion: true`。

核对 Step 7 要加进 `check-secrets.sh` 的私钥特征确实认得出这把私钥——只打印命中的行数，不打印内容：

```bash
grep -c -E 'dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V[5]' ~/.tauri/issuerelay.key
```

Expected：`1`。若是 `0`，只解码文件开头 48 个字符看注释行（`head -c 48 ~/.tauri/issuerelay.key | base64 -d; echo`——这 36 个字节全在「untrusted comment: …」注释行里，不含密钥数据），按看到的注释改 Step 7 的两个特征，并在报告里说明。

把 `tauri.conf.json` 的改动交给实现者（不单独提交：公钥与注册插件在同一个提交里，Step 9）。在最终报告里给维护者这条命令，由维护者自己运行（上传凭据是用户的事）：

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo sunkailun1992/issuerelay < ~/.tauri/issuerelay.key
```

- [ ] **Step 1: 配置与 Rust 的失败测试**

`app/src-tauri/tests/packaging.rs` 加：

```rust
#[test]
fn updates_come_signed_from_the_fixed_manifest() {
    let updater = &config("tauri.conf.json")["plugins"]["updater"];
    assert_eq!(
        updater["endpoints"],
        json!(["https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json"])
    );
    assert_eq!(updater["requireSignedVersion"], true);
    assert_eq!(updater["windows"]["installMode"], "passive");
    // The .pub file's one base64 line: a minisign public key box, which starts with
    // "untrusted comment: minisign public k". A file path, or anything else, is refused here.
    let pubkey = updater["pubkey"].as_str().expect("pubkey is set");
    assert!(pubkey.starts_with("dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBr"), "not a minisign public key");
    assert!(!pubkey.contains(char::is_whitespace));
}

#[test]
fn release_builds_add_the_cli_and_the_update_payloads() {
    let release = &config("tauri.updater.conf.json")["bundle"];
    assert_eq!(release["externalBin"], config("tauri.bundle.conf.json")["bundle"]["externalBin"]);
    assert_eq!(release["createUpdaterArtifacts"], true);
}
```

`app/src-tauri/src/quit.rs` 的测试模块：顶上的 `use super::QuitEvent::{Requested, StopNow, Stopped, Wait, WindowClosed};` 补 `Installed`；M2 的 `closing_the_window_while_quitting_does_not_exit_early` 的状态列表补 `QuitState::Installing`；M2 的 `every_exit_request_before_quitting_is_done_is_held` 的循环改为 `for state in [QuitState::Waiting, QuitState::Stopping, QuitState::Installing]`，注释末尾补一句「On the mock runtime the hook goes through the generic `request`, which drops a pending update (`update::clear_pending`; no updater state is managed here, so there is none) and asks (`ask`), as in the app.」。再加：

```rust
    #[test]
    fn the_quit_prompt_tells_the_panel_what_follows() {
        let quit = QuitPrompt { running: 2, purpose: QuitPurpose::Quit };
        assert_eq!(serde_json::to_value(quit).unwrap(), serde_json::json!({"running": 2, "purpose": "quit"}));
        let update = QuitPrompt { running: 1, purpose: QuitPurpose::Update };
        assert_eq!(serde_json::to_value(update).unwrap(), serde_json::json!({"running": 1, "purpose": "update"}));
    }

    #[test]
    fn a_verified_update_is_installed_where_quitting_would_exit() {
        // Quitting is done (the host stopped, or there was none) and a verified update waits:
        // it is installed instead, and quitting is not done until the install has returned.
        assert_eq!(
            installing_instead((QuitState::Done, QuitStep::Exit), true),
            (QuitState::Installing, QuitStep::Install)
        );
        assert_eq!(installing_instead((QuitState::Done, QuitStep::Exit), false), (QuitState::Done, QuitStep::Exit));
        // Every other step goes on as quitting does, update or not.
        for outcome in [
            (QuitState::Asking, QuitStep::Ask(1)),
            (QuitState::Waiting, QuitStep::Stop(StopMode::WaitForRuns)),
            (QuitState::Waiting, QuitStep::ShowPanel),
            (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns)),
            (QuitState::Done, QuitStep::Nothing),
        ] {
            assert_eq!(installing_instead(outcome, true), outcome, "{outcome:?}");
        }
    }

    #[test]
    fn the_process_outlives_the_install_and_then_restarts() {
        // While the update installs nothing ends the process: a closed window only hides, and
        // another Quit, an answer or a late `Stopped` change nothing.
        assert_eq!(next(QuitState::Installing, WindowClosed), (QuitState::Installing, QuitStep::HideWindow));
        for event in [Requested { host_ready: true, running: 0 }, Wait, StopNow, Stopped] {
            assert_eq!(next(QuitState::Installing, event), (QuitState::Installing, QuitStep::Nothing), "{event:?}");
        }
        assert!(!may_exit(QuitState::Installing), "an exit request is held while installing");
        // The install returned, installed or not: restart; only now may the process end.
        assert_eq!(next(QuitState::Installing, Installed), (QuitState::Done, QuitStep::Restart));
        assert!(may_exit(QuitState::Done));
        for state in [QuitState::Idle, QuitState::Asking, QuitState::Waiting, QuitState::Stopping, QuitState::Done] {
            assert_eq!(next(state, Installed), (state, QuitStep::Nothing), "{state:?}");
        }
    }
```

（「安装并重启」与退出走同一个 `next`：M2 的 `quitting_asks_only_while_runs_are_in_progress`、`waiting_can_still_become_stopping_now` 等测试不改，它们就是 Review Focus 1 在状态机一层的证据；`installing_instead` 只在「退出」那一步换成「安装」。）

更新失败代码的用例表 `app/src/lib/update-failure-cases.json`（Rust 的 `FailureCode`、`error_code()`，TS 的 `UpdateFailureCode`、`ERROR_CODES`、`updateFailureKey` 与文案键 `errors.<error_code>` 都由读它的测试钉住，AI_CODING_GUIDE 第 12.9 节）：

```json
[
  { "code": "unreachable", "error_code": "update_unreachable" },
  { "code": "no_platform", "error_code": "update_no_platform" },
  { "code": "bad_signature", "error_code": "update_bad_signature" },
  { "code": "unavailable", "error_code": "update_unavailable" },
  { "code": "install_failed", "error_code": "update_install_failed" },
  { "code": "other", "error_code": "update_failed" }
]
```

新建 `app/src-tauri/src/update.rs`，先只放测试模块：

```rust
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};

    use super::*;

    /// What the fake source offers.
    #[derive(Debug)]
    struct FakeOffer(String);

    /// Answers checks and downloads from a script and counts the downloads.
    struct FakeSource {
        check: Mutex<Result<Option<String>, Failure>>,
        download: Mutex<Result<Vec<u8>, Failure>>,
        downloads: Mutex<usize>,
    }

    impl FakeSource {
        fn new(check: Result<Option<&str>, Failure>, download: Result<&[u8], Failure>) -> Self {
            Self {
                check: Mutex::new(check.map(|offered| offered.map(str::to_string))),
                download: Mutex::new(download.map(<[u8]>::to_vec)),
                downloads: Mutex::new(0),
            }
        }
    }

    impl UpdateSource for FakeSource {
        type Offer = FakeOffer;

        fn version(offer: &FakeOffer) -> &str {
            &offer.0
        }

        async fn check(&self) -> Result<Option<FakeOffer>, Failure> {
            self.check.lock().unwrap().clone().map(|offered| offered.map(FakeOffer))
        }

        async fn download(&self, _offer: &FakeOffer) -> Result<Vec<u8>, Failure> {
            *self.downloads.lock().unwrap() += 1;
            self.download.lock().unwrap().clone()
        }
    }

    fn failure(code: FailureCode) -> Failure {
        Failure::new(code, "boom")
    }

    fn fake_updates(check: Result<Option<&str>, Failure>, download: Result<&[u8], Failure>) -> Updates<FakeSource> {
        Updates::new(FakeSource::new(check, download), "0.1.0")
    }

    #[tokio::test]
    async fn only_a_newer_version_is_offered() {
        for (offered, newer) in [("0.2.0", true), ("v0.1.1", true), ("0.1.0", false), ("0.0.9", false), ("0.1.0-rc.1", false), ("latest", false)] {
            let updates = fake_updates(Ok(Some(offered)), Ok(b"payload"));
            let status = updates.check().await;
            assert_eq!(matches!(status, UpdateStatus::Available { .. }), newer, "{offered}");
            // Nothing that is not newer can be downloaded, let alone installed.
            assert_eq!(updates.prepare().await.is_ok(), newer, "{offered}");
        }
        let updates = fake_updates(Ok(None), Ok(b"payload"));
        assert!(matches!(updates.check().await, UpdateStatus::UpToDate { .. }));
    }

    #[tokio::test]
    async fn a_failed_check_only_informs() {
        for code in [FailureCode::Unreachable, FailureCode::NoPlatform] {
            let updates = fake_updates(Err(failure(code)), Ok(b"payload"));
            assert_eq!(updates.check().await, UpdateStatus::Failed(failure(code)));
            assert_eq!(updates.status(), UpdateStatus::Failed(failure(code)));
            assert!(updates.prepare().await.is_err());
            assert_eq!(updates.pending_version(), None);
            assert_eq!(*updates.source.downloads.lock().unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn a_bad_signature_keeps_nothing_to_install() {
        let updates = fake_updates(Ok(Some("0.2.0")), Err(failure(FailureCode::BadSignature)));
        updates.check().await;
        assert_eq!(updates.prepare().await.unwrap_err().code, FailureCode::BadSignature);
        assert_eq!(updates.pending_version(), None, "nothing unverified waits for the quit flow");
        assert!(updates.take_pending().is_none());
        // The offer itself stays: the banner still says a new version exists.
        assert!(matches!(updates.status(), UpdateStatus::Available { .. }));
    }

    #[tokio::test]
    async fn a_verified_download_waits_for_the_quit_flow_and_is_reused() {
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.check().await;
        assert_eq!(updates.prepare().await.unwrap(), "0.2.0");
        assert_eq!(updates.pending_version().as_deref(), Some("0.2.0"));
        // "Keep running" in the prompt, then "install and restart" again: no second download.
        assert_eq!(updates.prepare().await.unwrap(), "0.2.0");
        assert_eq!(*updates.source.downloads.lock().unwrap(), 1);
        // A plain quit drops it (quit::request), so quitting never installs.
        updates.clear_pending();
        assert_eq!(updates.pending_version(), None);
        updates.prepare().await.unwrap();
        let pending = updates.take_pending().expect("downloaded");
        assert_eq!(pending.bytes, b"payload");
        assert!(updates.take_pending().is_none(), "installed once");
    }

    #[tokio::test]
    async fn a_failed_check_keeps_the_version_already_offered() {
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.check().await;
        *updates.source.check.lock().unwrap() = Err(failure(FailureCode::Unreachable));
        // This check's outcome is the failure; what the panel keeps showing is the offer.
        assert_eq!(updates.check().await, UpdateStatus::Failed(failure(FailureCode::Unreachable)));
        assert_eq!(updates.status(), UpdateStatus::Available { version: "0.2.0".into() });
    }

    #[test]
    fn each_version_notifies_once() {
        let updates = fake_updates(Ok(None), Ok(b""));
        assert!(updates.should_notify("0.2.0"));
        assert!(!updates.should_notify("0.2.0"));
        assert!(updates.should_notify("0.3.0"));
    }

    #[test]
    fn plugin_errors_become_codes() {
        use tauri_plugin_updater::Error as E;
        assert_eq!(failure_code(&E::ReleaseNotFound), FailureCode::Unreachable);
        assert_eq!(failure_code(&E::TargetsNotFound(vec!["darwin-aarch64".into()])), FailureCode::NoPlatform);
    }

    #[test]
    fn the_panel_reads_the_status() {
        let failed = UpdateStatus::Failed(Failure::new(FailureCode::BadSignature, "The signature verification failed"));
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            serde_json::json!({"kind": "failed", "code": "bad_signature", "detail": "The signature verification failed"})
        );
        let available = UpdateStatus::Available { version: "0.2.0".into() };
        assert_eq!(serde_json::to_value(available).unwrap(), serde_json::json!({"kind": "available", "version": "0.2.0"}));
        // A detail with control characters arrives cleaned.
        assert_eq!(Failure::new(FailureCode::Other, "bad\u{1b}[2Jline\n").detail, "bad [2Jline");
    }

    #[test]
    fn the_failure_codes_follow_the_shared_case_table() {
        // The panel's codes, the error codes `install_update` answers with and, as
        // `errors.<error_code>`, the translation keys: one table, read by the TS tests too.
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../src/lib/update-failure-cases.json")).unwrap();
        assert_eq!(cases.len(), FailureCode::ALL.len(), "one case per code");
        for (case, code) in cases.iter().zip(FailureCode::ALL) {
            assert_eq!(case["code"], serde_json::to_value(code).unwrap(), "{code:?}");
            assert_eq!(case["error_code"], code.error_code(), "{code:?}");
        }
    }

    #[test]
    fn a_failed_install_is_shown_once_after_the_restart() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(take_install_failure(data.path()), None, "nothing recorded");
        // Written by the process that failed to install, just before it restarts as it is.
        record_install_failure(data.path(), "cannot replace\u{1b}[2J the app\n");
        let failure = take_install_failure(data.path()).expect("recorded before the restart");
        assert_eq!(failure.code, FailureCode::InstallFailed);
        assert_eq!(failure.detail, "cannot replace [2J the app");
        assert_eq!(take_install_failure(data.path()), None, "shown once: the file is gone");
        // The restarted app starts with it on show, until a check has something to say.
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.show_failure(failure.clone());
        assert_eq!(updates.status(), UpdateStatus::Failed(failure));
    }

    #[test]
    fn the_quit_flow_helpers_run_on_any_runtime() {
        // Generic over the runtime like quit.rs, so the quit flow that calls them runs on tauri's
        // mock runtime in quit.rs's tests: without updater state nothing is pending.
        let app = mock_builder()
            .manage(crate::state::AppState::new(false))
            .build(mock_context(noop_assets()))
            .unwrap();
        assert!(!pending(app.handle()));
        assert!(take_pending(app.handle()).is_none());
        clear_pending(app.handle());
        // With the state the app manages: still nothing until a verified download exists.
        app.manage(AppUpdates::new(PluginSource(app.handle().clone()), "0.1.0"));
        assert!(!pending(app.handle()));
        assert_eq!(app.state::<AppUpdates<MockRuntime>>().status(), UpdateStatus::Unchecked);
    }
}
```

Run: `cargo test -p issuerelay-app`
Expected: 编译失败（`update` 模块、`QuitPrompt`、`QuitState::Installing`、`installing_instead` 不存在；配置测试缺 `tauri.updater.conf.json`）。

- [ ] **Step 2: 依赖与配置文件**

```bash
cargo add -p issuerelay-app tauri-plugin-updater semver
# chrono is a dev-dependency of the app crate (test_support uses it); update.rs needs it as a
# normal one. Move it rather than add a second entry.
cargo remove -p issuerelay-app --dev chrono
cargo add -p issuerelay-app chrono --features serde
```

（解析到的 `tauri-plugin-updater` 应是 2.13.x；版本、`Update` 的字段与 `Error` 的变体以它的 docs.rs 为准，与下面的写法不同时照文档改，在报告里列出。`chrono` 移到 `[dependencies]` 后测试照样能用，`tokio` 已是 `features = ["full"]`，`tokio::time::sleep` 不用另加特性。）

`app/src-tauri/tauri.updater.conf.json`：

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "bundle": {
    "externalBin": ["binaries/issuerelay"],
    "createUpdaterArtifacts": true
  }
}
```

（发布工作流每次只传一个 `--config`：没有私钥时传 `tauri.bundle.conf.json`，有私钥时传这一个，所以 `externalBin` 两边都写；配置测试保证两边一致。）

`crates/core/src/config.rs` 里 `auto_update` 的文档注释改为 `/// Check for updates automatically and offer them in the panel (design 12); installing always waits for the user's "install and restart".`（只改注释）。

- [ ] **Step 3: 实现 `update.rs`**

在测试模块上方写。应用一侧的函数都对 `R: tauri::Runtime` 泛型，与 M2 的 `quit::handle` 一样，所以退出流程在 MockRuntime 上的测试照样编译、照样跑（裁定 M3-AR3）：

```rust
//! Automatic updates (design 12). The updater plugin reads
//! `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json` a minute
//! after the start and every six hours while `settings.auto_update` is on, and whenever the
//! settings page asks. Only a newer version is offered, and only a payload that verifies against
//! the public key in tauri.conf.json (and names the announced version, `requireSignedVersion`)
//! is ever kept. Installing waits for the user's "install and restart", which goes through the
//! quit flow (`quit::request_update`): with runs in progress the panel asks the same "wait for
//! completion / stop now" as quitting, and once the host has stopped the quit flow installs
//! (`QuitState::Installing`) and then restarts. A failed check, a missing platform, a bad
//! signature or a version that is not newer is shown and logged and changes nothing else:
//! monitoring goes on. An install that fails after the host stopped restarts the app as it is and
//! is shown by the panel after the restart (`record_install_failure`), never as a notification.
//! Everything that takes the app is generic over the runtime, like quit.rs.
use std::{
    future::Future,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use chrono::{DateTime, Utc};
use issuerelay_core::{
    cli_path::{AppLocation, running_app_location},
    host::Host,
    paths::AppPaths,
    text::clean_text,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::UpdaterExt;

use crate::{
    events, logging, notify,
    quit::{self, QuitEvent},
    state::AppState,
};

/// The first background check after the start.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The background checks after the first.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// What a failed install leaves in the data folder for the next start: the process restarts as it
/// is right after the failure, and the panel then shows the cleaned reason once.
pub const INSTALL_FAILURE_FILE: &str = "update-install-failure.txt";

/// Why a check, a download or an install failed, as a code the panel translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// No valid release manifest could be fetched: offline, a proxy, or nothing published yet.
    Unreachable,
    /// The manifest has no entry for this platform.
    NoPlatform,
    /// The payload's signature does not verify against the public key, or names another version.
    BadSignature,
    /// IssueRelay cannot replace itself from where it runs: a development build, the disk image
    /// or a translocated copy.
    Unavailable,
    /// The verified payload could not be installed.
    InstallFailed,
    Other,
}

impl FailureCode {
    /// Every code, in the order of app/src/lib/update-failure-cases.json, which the panel's tests
    /// read too.
    pub const ALL: [FailureCode; 6] = [
        Self::Unreachable,
        Self::NoPlatform,
        Self::BadSignature,
        Self::Unavailable,
        Self::InstallFailed,
        Self::Other,
    ];

    /// The error code `install_update` answers with; the panel translates it (`errors.*`).
    pub fn error_code(self) -> &'static str {
        match self {
            Self::Unreachable => "update_unreachable",
            Self::NoPlatform => "update_no_platform",
            Self::BadSignature => "update_bad_signature",
            Self::Unavailable => "update_unavailable",
            Self::InstallFailed => "update_install_failed",
            Self::Other => "update_failed",
        }
    }
}

/// A failure: its code and the cleaned error line the panel shows under the translated headline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub code: FailureCode,
    pub detail: String,
}

impl Failure {
    pub fn new(code: FailureCode, detail: impl std::fmt::Display) -> Self {
        Self { code, detail: clean_text(&detail.to_string()) }
    }
}

/// What the panel shows about updates: `update_status`, `check_update` and `update-status`.
/// The release's own text (`notes` in latest.json) is written for the download page, not for an
/// installed copy, and is not carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Not checked since the start.
    Unchecked,
    /// The last check found nothing newer than the running version.
    UpToDate { checked_at: DateTime<Utc> },
    /// A newer version is offered.
    Available { version: String },
    /// The check failed, or (after a restart) the previous install did; nothing was installed.
    Failed(Failure),
}

/// Where updates come from: the updater plugin in the app, a fake in the tests.
pub trait UpdateSource: Send + Sync + 'static {
    /// An update a check found; kept until it is downloaded and installed.
    type Offer: Send + Sync + 'static;
    fn version(offer: &Self::Offer) -> &str;
    /// The newest release when it is newer than the running version.
    fn check(&self) -> impl Future<Output = Result<Option<Self::Offer>, Failure>> + Send;
    /// The offer's payload, verified against the public key: nothing unverified comes back.
    fn download(&self, offer: &Self::Offer) -> impl Future<Output = Result<Vec<u8>, Failure>> + Send;
}

/// A downloaded and verified update waiting for the quit flow.
pub struct Pending<O> {
    pub offer: Arc<O>,
    pub bytes: Vec<u8>,
}

/// The update state the app keeps: the last check, the offer it found, the verified download
/// waiting for the quit flow, and the version that already notified.
pub struct Updates<S: UpdateSource> {
    source: S,
    current: String,
    status: Mutex<UpdateStatus>,
    offer: Mutex<Option<Arc<S::Offer>>>,
    pending: Mutex<Option<Pending<S::Offer>>>,
    notified: Mutex<Option<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `offered` is a newer semantic version than `current`; a leading `v` is allowed and
/// anything unparsable is not newer.
pub fn is_newer(current: &str, offered: &str) -> bool {
    match (semver::Version::parse(current), semver::Version::parse(offered.trim_start_matches('v'))) {
        (Ok(current), Ok(offered)) => offered > current,
        _ => false,
    }
}

impl<S: UpdateSource> Updates<S> {
    pub fn new(source: S, current: &str) -> Self {
        Self {
            source,
            current: current.to_string(),
            status: Mutex::new(UpdateStatus::Unchecked),
            offer: Mutex::new(None),
            pending: Mutex::new(None),
            notified: Mutex::new(None),
        }
    }

    /// What the panel shows now.
    pub fn status(&self) -> UpdateStatus {
        lock(&self.status).clone()
    }

    /// Starts with a failure on show: the previous run's failed install (`take_install_failure`).
    pub fn show_failure(&self, failure: Failure) {
        *lock(&self.status) = UpdateStatus::Failed(failure);
    }

    /// Asks the source and returns this check's outcome. Only a newer version becomes an offer.
    /// A failed check is logged and leaves a version already offered on show.
    pub async fn check(&self) -> UpdateStatus {
        let outcome = match self.source.check().await {
            Ok(Some(offer)) if is_newer(&self.current, S::version(&offer)) => {
                let status = UpdateStatus::Available { version: S::version(&offer).to_string() };
                *lock(&self.offer) = Some(Arc::new(offer));
                status
            }
            // The plugin already drops versions that are not newer; a manifest that slips one
            // through is treated the same.
            Ok(_) => {
                *lock(&self.offer) = None;
                UpdateStatus::UpToDate { checked_at: Utc::now() }
            }
            Err(failure) => {
                tracing::warn!(code = ?failure.code, detail = %failure.detail, "the update check failed");
                let outcome = UpdateStatus::Failed(failure);
                let mut status = lock(&self.status);
                if !matches!(*status, UpdateStatus::Available { .. }) {
                    *status = outcome.clone();
                }
                return outcome;
            }
        };
        *lock(&self.status) = outcome.clone();
        outcome
    }

    /// Downloads and verifies the offered version for "install and restart" and returns it.
    /// Nothing is kept unless the payload verified; a download already waiting is reused.
    pub async fn prepare(&self) -> Result<String, Failure> {
        let offer = lock(&self.offer)
            .clone()
            .ok_or_else(|| Failure::new(FailureCode::Other, "no newer version is offered; check for updates first"))?;
        let version = S::version(&offer).to_string();
        if lock(&self.pending).as_ref().is_some_and(|pending| S::version(&pending.offer) == version) {
            return Ok(version);
        }
        let bytes = self.source.download(&offer).await?;
        *lock(&self.pending) = Some(Pending { offer, bytes });
        Ok(version)
    }

    pub fn pending_version(&self) -> Option<String> {
        lock(&self.pending).as_ref().map(|pending| S::version(&pending.offer).to_string())
    }

    /// The verified download, once: the quit flow installs it.
    pub fn take_pending(&self) -> Option<Pending<S::Offer>> {
        lock(&self.pending).take()
    }

    /// Drops the verified download: the user quit instead of installing.
    pub fn clear_pending(&self) {
        lock(&self.pending).take();
    }

    /// Whether a background check that found `version` shows a notification: once per version.
    pub fn should_notify(&self, version: &str) -> bool {
        let mut notified = lock(&self.notified);
        if notified.as_deref() == Some(version) {
            return false;
        }
        *notified = Some(version.to_string());
        true
    }
}

/// The updater plugin as the source of updates.
pub struct PluginSource<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> UpdateSource for PluginSource<R> {
    type Offer = tauri_plugin_updater::Update;

    fn version(offer: &Self::Offer) -> &str {
        &offer.version
    }

    async fn check(&self) -> Result<Option<Self::Offer>, Failure> {
        let updater = self.0.updater().map_err(plugin_failure)?;
        updater.check().await.map_err(plugin_failure)
    }

    async fn download(&self, offer: &Self::Offer) -> Result<Vec<u8>, Failure> {
        // `download` verifies the minisign signature (and the version it names) before it
        // returns; a payload that does not verify is an error and nothing is kept.
        offer.download(|_, _| {}, || {}).await.map_err(plugin_failure)
    }
}

fn plugin_failure(error: tauri_plugin_updater::Error) -> Failure {
    Failure::new(failure_code(&error), &error)
}

/// The plugin's errors as codes (variants of `tauri_plugin_updater::Error` in 2.13.x; the `{ .. }`
/// patterns match whatever fields a variant has).
pub fn failure_code(error: &tauri_plugin_updater::Error) -> FailureCode {
    use tauri_plugin_updater::Error as E;
    match error {
        E::ReleaseNotFound { .. } | E::Reqwest { .. } | E::Serialization { .. } => FailureCode::Unreachable,
        E::TargetsNotFound { .. } => FailureCode::NoPlatform,
        E::Minisign { .. }
        | E::Base64 { .. }
        | E::SignatureUtf8 { .. }
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion { .. } => FailureCode::BadSignature,
        _ => FailureCode::Other,
    }
}

/// The update state the app manages; `tauri::Wry` in the app, tauri's mock runtime in tests.
pub type AppUpdates<R = tauri::Wry> = Updates<PluginSource<R>>;

/// Whether a verified update waits for the quit flow.
pub fn pending<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<AppUpdates<R>>().is_some_and(|updates| updates.pending_version().is_some())
}

pub fn take_pending<R: Runtime>(app: &AppHandle<R>) -> Option<Pending<tauri_plugin_updater::Update>> {
    app.try_state::<AppUpdates<R>>().and_then(|updates| updates.take_pending())
}

pub fn clear_pending<R: Runtime>(app: &AppHandle<R>) {
    if let Some(updates) = app.try_state::<AppUpdates<R>>() {
        updates.clear_pending();
    }
}

/// Checks now, records the outcome and tells an open panel (`update-status` carries what the
/// panel should keep showing).
pub async fn check<R: Runtime>(app: &AppHandle<R>) -> UpdateStatus {
    let updates = app.state::<AppUpdates<R>>();
    let outcome = updates.check().await;
    logging::debug_failed(app.emit(events::UPDATE_STATUS, updates.status()), "telling the panel about the update check");
    outcome
}

/// "Install and restart": refuses where the app cannot replace itself, downloads and verifies the
/// offered version, then hands over to the quit flow, which asks about runs in progress exactly
/// as quitting does and installs once the host has stopped.
pub async fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), Failure> {
    match running_app_location() {
        AppLocation::Installed => {}
        AppLocation::NotInstalled => {
            return Err(Failure::new(FailureCode::Unavailable, "IssueRelay runs from the disk image or a temporary copy"));
        }
        AppLocation::Development => {
            return Err(Failure::new(FailureCode::Unavailable, "a development build does not install updates"));
        }
    }
    app.state::<AppUpdates<R>>().prepare().await?;
    quit::request_update(app);
    Ok(())
}

/// Installs a verified download once the quit flow has stopped the host (`QuitState::Installing`)
/// and then lets it restart (`QuitEvent::Installed`). macOS swaps the app bundle in place and the
/// restart opens the new one; on Windows `install` starts the NSIS installer (`/P /UPDATE /R`),
/// which ends this process and starts the new version itself. If installing fails the app
/// restarts as it is, so monitoring resumes, and the panel says why after the restart.
pub fn install_and_restart<R: Runtime>(app: &AppHandle<R>, pending: Pending<tauri_plugin_updater::Update>) {
    let app = app.clone();
    // Off the main thread: the restart that follows then delivers the exit event first, so the
    // single-instance lock is released before the new instance starts.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = pending.offer.install(&pending.bytes) {
            tracing::error!(error = %e, version = %pending.offer.version, "installing the update failed; restarting the current version");
            match AppPaths::system() {
                Ok(paths) => record_install_failure(&paths.data_dir, &e.to_string()),
                Err(error) => tracing::warn!(error = %format!("{error:#}"), "cannot keep the install failure for the panel"),
            }
        }
        quit::handle(&app, QuitEvent::Installed);
    });
}

/// Keeps a failed install's cleaned error line for the next start, whose panel shows it once
/// (`take_install_failure`); a failure to write it is only logged.
pub fn record_install_failure(data_dir: &Path, detail: &str) {
    let file = data_dir.join(INSTALL_FAILURE_FILE);
    let written = std::fs::create_dir_all(data_dir).and_then(|()| std::fs::write(&file, clean_text(detail)));
    if let Err(e) = written {
        tracing::warn!(error = %e, "cannot keep the install failure for the panel");
    }
}

/// The failed install the previous run left, once: the file is removed as it is read.
pub fn take_install_failure(data_dir: &Path) -> Option<Failure> {
    let file = data_dir.join(INSTALL_FAILURE_FILE);
    let detail = std::fs::read_to_string(&file).ok()?;
    if let Err(e) = std::fs::remove_file(&file) {
        tracing::warn!(error = %e, "cannot remove the install failure note; the panel may show it again");
    }
    Some(Failure::new(FailureCode::InstallFailed, detail))
}

/// The host, when the background check may run: started, and automatic checks on.
async fn auto_check_host<R: Runtime>(app: &AppHandle<R>) -> Option<Host> {
    let host = app.state::<AppState>().host().await.ok()?;
    host.engine().settings().auto_update.then_some(host)
}

/// The background check: a minute after the start, then every six hours, only while
/// `settings.auto_update` is on, which is also the only switch of the "new version" notice
/// (design 9.5's four switchable notifications are about runs). A newer version notifies once and
/// shows the banner. Development builds check only when asked.
pub fn spawn_checker<R: Runtime>(app: AppHandle<R>) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if let Some(host) = auto_check_host(&app).await
                && let UpdateStatus::Available { version } = check(&app).await
                && app.state::<AppUpdates<R>>().should_notify(&version)
            {
                notify::show_update_available(&app, &host, &version);
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}
```

（`AppHandle::request_restart` 在 tauri 2.12.3 的文档里有（退出状态机的 `Restart` 一步调用它）；解析到的版本没有它时，`Restart` 改为 `tauri_plugin_single_instance::destroy(app); app.restart();`——先释放单实例锁再重启——并在报告里说明。`tauri_plugin_updater::Update` 若不是 `Sync`，`Arc<Update>` 编译不过，就把 `Pending.offer` 与 `Updates.offer` 改为直接存 `Update`（它实现 `Clone`），在报告里说明。`PluginSource<R>` 的 `check` / `download` 的 future 必须是 `Send`（`UpdateSource` 的约束）；解析到的插件版本做不到时停下报告。）

- [ ] **Step 4: 接上退出流程、事件、通知、命令与插件**

`events.rs` 加 `pub const UPDATE_STATUS: &str = "update-status";`（文档注释：`/// The update state the panel should show (an \`update::UpdateStatus\`).`），并把 `QUIT_REQUESTED` 的文档注释（现在写着「the payload is the number of runs in progress」）改为：

```rust
/// Quitting asks the panel "wait or stop now"; the payload is a `quit::QuitPrompt`: how many runs
/// are in progress and whether IssueRelay then quits or installs a downloaded update.
pub const QUIT_REQUESTED: &str = "quit-requested";
```

`quit.rs`（编辑 M2 的文件；M2 的函数签名、纯函数 `next` 的已有分支与全部测试保留）：

- `use` 加 `serde::Serialize`；`use crate::{events, logging, state::AppState, window};` 改为 `use crate::{events, logging, state::AppState, update, window};`。
- 模块文档末尾补一段：「"Install and restart" (`quit::request_update`) goes through the same steps. Where quitting would exit, a verified update waiting in `update` is installed instead (`installing_instead`): quitting stays `Installing`, so a closed window only hides and an exit request is held, until the install has returned (`Installed`) and IssueRelay restarts. A plain quit (`request`) drops a downloaded update first, so quitting never installs it.」
- `QuitState` 在 `Done` 前加：

```rust
    /// The host has stopped and a verified update is being installed: the process must outlive
    /// the install, so a closed window only hides and an exit request is held; `Installed` ends it.
    Installing,
```

- `QuitEvent` 末尾加：

```rust
    /// The update's install returned, installed or not (`update::install_and_restart`).
    Installed,
```

- `QuitStep` 的 `Ask` 与 `ShowPanel` 的文档注释改写，末尾加两步：

```rust
    /// Show the panel and ask (`quit-requested` with a `QuitPrompt`).
    Ask(usize),
    /// Show the panel again: it still shows the waiting prompt with "stop now", and is told again
    /// what follows, which a tray Quit may just have changed.
    ShowPanel,
```

```rust
    /// Install the verified update in the background; `Installed` follows.
    Install,
    /// Restart into the installed version, or into the current one when installing failed.
    Restart,
```

- `next` 在最后的 `_ => (state, QuitStep::Nothing),` 之前加一个分支（关窗已由现有的 `(_, E::WindowClosed) => HideWindow` 管住，`Installing` 不是 `Done`，所以只隐藏；其余事件在 `Installing` 里落到 `_`，什么都不做）：

```rust
        (S::Installing, E::Installed) => (S::Done, QuitStep::Restart),
```

- 在 `QuitStep` 之后加载荷与 `installing_instead`：

```rust
/// What follows once quitting is done: IssueRelay exits, or installs the downloaded update and
/// restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuitPurpose {
    Quit,
    Update,
}

/// The panel's question (`quit-requested`): how many runs are in progress, and what follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QuitPrompt {
    pub running: usize,
    pub purpose: QuitPurpose,
}

/// `next`'s outcome when a verified update waits for quitting to end: where quitting would exit,
/// IssueRelay installs the update instead, and quitting is not done until the install has
/// returned (`Installed`). Any other outcome stands.
pub fn installing_instead(outcome: (QuitState, QuitStep), update_pending: bool) -> (QuitState, QuitStep) {
    match outcome {
        (QuitState::Done, QuitStep::Exit) if update_pending => (QuitState::Installing, QuitStep::Install),
        outcome => outcome,
    }
}
```

- `handle` 改为（签名不变，仍对 `R: Runtime` 泛型；`Stop` 一支原样保留）：

```rust
pub fn handle<R: Runtime>(app: &AppHandle<R>, event: QuitEvent) -> QuitStep {
    let state = app.state::<AppState>();
    let (step, install) = {
        let mut quit = state.quit.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = next(*quit, event);
        // Taken under the quit lock where quitting would exit: a tray Quit that drops the update
        // (`request`) comes either before, and IssueRelay quits, or after, and finds it installing.
        let install = (outcome == (QuitState::Done, QuitStep::Exit))
            .then(|| update::take_pending(app))
            .flatten();
        let (next_state, step) = installing_instead(outcome, install.is_some());
        *quit = next_state;
        (step, install)
    };
    match step {
        QuitStep::Nothing | QuitStep::HideWindow => {}
        QuitStep::Ask(running) => {
            window::show(app);
            tell_panel(app, running);
        }
        QuitStep::ShowPanel => {
            window::show(app);
            // Still waiting: say again what follows. A tray Quit has just dropped a downloaded
            // update, so IssueRelay now quits instead of installing it, and the prompt says so.
            if let QuitEvent::Requested { running, .. } = event {
                tell_panel(app, running);
            }
        }
        QuitStep::Stop(mode) => match state.try_host() {
            Some(host) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    host.stop(mode).await;
                    // Only now: the runs have ended with their trees, their results are recorded
                    // and the single-instance lock is released. `Stopped` leads to `Done`, and
                    // only that transition asks for `Exit`.
                    handle(&app, QuitEvent::Stopped);
                });
            }
            None => {
                no_host_to_stop(app, "asked to stop the host");
                handle(app, QuitEvent::Stopped);
            }
        },
        QuitStep::Exit => app.exit(0),
        QuitStep::Install => {
            if let Some(pending) = install {
                update::install_and_restart(app, pending);
            }
        }
        QuitStep::Restart => app.request_restart(),
    }
    step
}

/// Asks the panel (`quit-requested`): how many runs are in progress and what follows, an exit or
/// installing the downloaded update.
fn tell_panel<R: Runtime>(app: &AppHandle<R>, running: usize) {
    let purpose = if update::pending(app) { QuitPurpose::Update } else { QuitPurpose::Quit };
    logging::debug_failed(
        app.emit(events::QUIT_REQUESTED, QuitPrompt { running, purpose }),
        "asking the panel about quitting",
    );
}
```

- `request` 换成三个函数（都对 `R: Runtime` 泛型；M2 的「没有宿主可停」日志留在 `ask` 里）：

```rust
/// The tray's Quit, Cmd+Q and an exit request: a quit, so an update downloaded for "install and
/// restart" is dropped first and quitting never installs it (while the panel waits for runs to
/// install, this turns it into a plain quit, and the prompt says so).
pub fn request<R: Runtime>(app: &AppHandle<R>) {
    update::clear_pending(app);
    ask(app);
}

/// "Install and restart", once the update is downloaded and verified: with runs in progress the
/// same question as quitting; once the host has stopped, `handle` installs instead of exiting.
pub fn request_update<R: Runtime>(app: &AppHandle<R>) {
    ask(app);
}

/// Starts quitting with the host as it is now.
fn ask<R: Runtime>(app: &AppHandle<R>) {
    let host = app.state::<AppState>().try_host();
    let running = match &host {
        Some(host) => host.engine().status().running,
        None => {
            no_host_to_stop(app, "quitting");
            0
        }
    };
    handle(
        app,
        QuitEvent::Requested {
            host_ready: host.is_some(),
            running,
        },
    );
}
```

- `forced_exit` 的第一个判断改为 `if may_exit(*state) || *state == QuitState::Installing { return ForcedExit::Done; }`（宿主在安装之前就停了，系统强制结束时没有什么要再停），测试模块加：

```rust
    #[test]
    fn a_forced_exit_while_installing_stops_nothing_again() {
        let quit = Mutex::new(QuitState::Installing);
        let outcome = forced_exit(&quit, Some(std::future::pending::<()>()), Duration::from_millis(50));
        assert_eq!(outcome, ForcedExit::Done, "the host stopped before the install began");
        assert_eq!(*quit.lock().unwrap(), QuitState::Installing);
    }
```

（`on_run_event` 不变：`Restart` 一步发出的 `ExitRequested` 带退出码，到那时已是 `Done`，照样放行；`Installing` 里的退出请求被拦下，经 `request` 走到 `next`，什么都不做。`window::on_event` 不变：`Installing` 里关窗得到 `HideWindow`。）

`notify.rs`（新版本通知沿用同一套翻译与发送，不另写一份，裁定 F25）：模块文档改为：

```rust
//! System notifications (design 9.5): the four moments of a run, each of which can be switched
//! off, plus the notice of a new version, which has no switch of its own: only the background
//! update check shows it (`update::spawn_checker`), and that check runs only while "check for
//! updates automatically" is on. A notification names the issue (or the version) and says what
//! happened in the user's language; the engine's own text stays in the panel, under a translated
//! headline. A failed update install is shown in the panel, never here.
```

`use tauri::AppHandle;` 改为 `use tauri::{AppHandle, Runtime};`；`show` 的最后三行（`app.notification()…`）换成 `deliver(app, title, body);`，再加：

```rust
/// Title and body of the new-version notice: the version and a translated sentence.
pub fn update_content(version: &str, t: &Translator) -> (String, String) {
    (t.t_with("notify.update_available", &[("version", version)]), t.t("notify.update_available_body"))
}

/// The notice that a background check found `version` (once per version, `Updates::should_notify`).
pub fn show_update_available<R: Runtime>(app: &AppHandle<R>, host: &Host, version: &str) {
    let (title, body) = update_content(version, &Translator::for_settings(&host.engine().settings()));
    deliver(app, title, body);
}

/// Hands a notification to the system; a failure is only logged.
fn deliver<R: Runtime>(app: &AppHandle<R>, title: String, body: String) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "could not show a notification");
    }
}
```

测试模块加：

```rust
    #[test]
    fn the_new_version_notice_names_the_version_in_the_users_language() {
        let (title, body) = update_content("0.2.0", &Translator::new("en"));
        assert_eq!(title, "IssueRelay 0.2.0 is available");
        assert_eq!(body, "Open the panel to install it.");
    }
```

`commands.rs` 的 `use crate::{…}` 加 `update::{self, AppUpdates, UpdateStatus}`，末尾加：

```rust
/// The last update check's outcome, for a panel opened after it; answers at once.
#[tauri::command]
pub fn update_status(updates: State<'_, AppUpdates>) -> UpdateStatus {
    updates.status()
}

/// Checks for an update now (the settings page's button); never installs anything.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> UpdateStatus {
    update::check(&app).await
}

/// "Install and restart": downloads and verifies the offered update, then hands over to the quit
/// flow. A failure is answered as a code (`update_*`) the panel translates and changes nothing.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> CmdResult<()> {
    update::install(&app).await.map_err(|failure| {
        tracing::warn!(code = ?failure.code, detail = %failure.detail, "the update was not installed");
        failure.code.error_code().to_string()
    })
}
```

`commands.rs` 文件头里列举错误代码的那句（`not_a_linear_link`、`login_cancelled`）补上「and the `update_*` codes of `install_update`」。

`lib.rs`：`pub mod update;`（放在 `pub mod tray;` 之后）；`tauri::Builder::default()` 链里在 `.plugin(tauri_plugin_dialog::init())` 之后加 `.plugin(tauri_plugin_updater::Builder::new().build())`；`.setup` 的闭包开头加：

```rust
            let updates = update::AppUpdates::new(
                update::PluginSource(app.handle().clone()),
                env!("CARGO_PKG_VERSION"),
            );
            // The previous run could not install an update and restarted as it was: the banner
            // says why, once.
            if let Some(failure) = issuerelay_core::paths::AppPaths::system()
                .ok()
                .and_then(|paths| update::take_install_failure(&paths.data_dir))
            {
                updates.show_failure(failure);
            }
            app.manage(updates);
```

并在 `tauri::async_runtime::spawn(startup::start(app.handle().clone()));` 之后加 `update::spawn_checker(app.handle().clone());`。

三处登记，同一步里改：

- `lib.rs` 的 `generate_handler!` 在 `commands::remove_cli_from_path,` 之后加 `commands::update_status,`、`commands::check_update,`、`commands::install_update,`（各占一行）。
- `build.rs` 的 `PANEL_COMMANDS` 在 `"remove_cli_from_path",` 之后加 `"update_status",`、`"check_update",`、`"install_update",`。
- `capabilities/default.json` 在 `"allow-remove-cli-from-path"` 之后加 `"allow-update-status"`、`"allow-check-update"`、`"allow-install-update"`（上一行补逗号）；`description` 里「… and the command line on PATH go through the commands」改为「…, the command line on PATH and updates go through the commands」。

命令共 40 条（M2 的 34 条、Task 2 的 3 条、这里的 3 条）。

`i18n.rs` 的 `USED_KEYS` 末尾加 `"notify.update_available",`、`"notify.update_available_body",`（翻译文件 Step 6 补）。

Run: `cargo test -p issuerelay-app` 与 `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: 除下面两条外全部 PASS，含 M2 的绊线测试（命令表、应用清单、能力文件都是 40 条）与扩展到 `Installing` 的 `every_exit_request_before_quitting_is_done_is_held`（在 MockRuntime 上走泛型的 `request` → `update::clear_pending` → `ask` → `handle`）。`USED_KEYS` 的检查与 `the_new_version_notice_names_the_version_in_the_users_language` 要等 Step 6 补好翻译才通过，在那之前是仅有的两条失败。`api.test.ts` 也要等 Step 6（能力文件多了三条命令）。

- [ ] **Step 5: 前端的失败测试**

`app/src/components/UpdateBanner.test.tsx`（断言「没有」之前先等查询真的有了答复：假后端在下一个宏任务才答，一次 `act` 冲不到它，TESTING_SPEC「等条件，不等固定时间」）：

```tsx
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { UpdateStatus } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { UpdateBanner } from "./UpdateBanner";

const AVAILABLE: UpdateStatus = { kind: "available", version: "0.2.0" };

test("a newer version shows the banner, and installing is the backend's job", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  const install = vi.spyOn(backend, "installUpdate");
  const quit = vi.spyOn(backend, "quit");
  const requestQuit = vi.spyOn(backend, "requestQuit");
  await renderWithProviders(<UpdateBanner />, { backend });
  expect(await screen.findByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Install and restart" }));
  expect(install).toHaveBeenCalledTimes(1);
  // The panel never quits or restarts by itself: the Rust side asks about runs first.
  expect(quit).not.toHaveBeenCalled();
  expect(requestQuit).not.toHaveBeenCalled();
});

test("a failed install says why in the user's language and keeps the banner", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  backend.installUpdate = vi.fn().mockRejectedValue(new BackendError("update_bad_signature"));
  await renderWithProviders(<UpdateBanner />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Install and restart" }));
  expect(await screen.findByText("The update's signature did not verify; nothing was installed")).toBeInTheDocument();
  expect(screen.getByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
});

test("no banner while up to date; Later hides it for that version", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { client, unmount } = await renderWithProviders(<UpdateBanner />, { backend });
  // The answer (up to date) has arrived, so the absence below is a real one.
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  expect(screen.queryByRole("button", { name: "Install and restart" })).not.toBeInTheDocument();
  unmount();
  backend.updateStatus = async () => AVAILABLE;
  await renderWithProviders(<UpdateBanner />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Later" }));
  expect(screen.queryByText("IssueRelay 0.2.0 is available")).not.toBeInTheDocument();
});

test("a background check that finds a version shows the banner in an open panel", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let announce: (status: UpdateStatus) => void = () => {};
  const listen = vi.fn(async (listener: (status: UpdateStatus) => void) => {
    announce = listener;
    return () => {};
  });
  backend.onUpdateStatus = listen;
  const { client } = await renderWithProviders(
    <>
      <EngineEventsBridge />
      <UpdateBanner />
    </>,
    { backend },
  );
  await waitFor(() => expect(listen).toHaveBeenCalled());
  // The first read (up to date) has settled, so it cannot overwrite the event afterwards.
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  await act(async () => announce(AVAILABLE));
  expect(await screen.findByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
});

test("an update that could not be installed before the restart is shown once, with why", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async (): Promise<UpdateStatus> => ({ kind: "failed", code: "install_failed", detail: "cannot replace the app" });
  await renderWithProviders(<UpdateBanner />, { backend });
  expect(await screen.findByText("The update could not be installed")).toBeInTheDocument();
  expect(screen.getByText("cannot replace the app")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(screen.queryByText("The update could not be installed")).not.toBeInTheDocument();
});

test("a failed background check stays out of the banner", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async (): Promise<UpdateStatus> => ({ kind: "failed", code: "unreachable", detail: "offline" });
  const { client } = await renderWithProviders(<UpdateBanner />, { backend });
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  expect(screen.queryByText("Could not reach the update server")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Install and restart" })).not.toBeInTheDocument();
});
```

`app/src/components/QuitDialog.test.tsx` **编辑，不替换**：M2 的七个测试全部保留，只把载荷改成 `QuitPrompt`，再加三个。

- 导入补 `import type { QuitPrompt, QuitPurpose } from "@/lib/types";`。
- 前两个测试（「quitting with runs in progress asks…」「keep running closes the prompt…」）里 `let ask: (running: number) => void = () => {};` 改为 `let ask: (prompt: QuitPrompt) => void = () => {};`，`ask(2)` 改为 `ask({ running: 2, purpose: "quit" })`，`ask(1)` 改为 `ask({ running: 1, purpose: "quit" })`。
- 辅助函数 `askedWith` 改为下面这样；用它的五个测试（escape、asked again、while waiting、refused wait、stopping twice）一字不改：

```tsx
/** A dialog that the backend asks to open with `running` runs; the answer is whatever `quit` does. */
async function askedWith(running: number, quit: () => Promise<void>, purpose: QuitPurpose = "quit") {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (prompt: QuitPrompt) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn(quit);
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask({ running, purpose }));
  return {
    backend,
    ask: (count: number, next: QuitPurpose = "quit") => act(async () => ask({ running: count, purpose: next })),
  };
}
```

- 文件末尾加：

```tsx
test("installing an update with runs in progress asks the same question and says it restarts", async () => {
  const { backend } = await askedWith(1, async () => {}, "update");
  expect(screen.getByText(/1 run\(s\) are in progress/)).toBeInTheDocument();
  expect(screen.getByText("IssueRelay restarts to install the update once these runs are settled.")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(backend.quit).toHaveBeenCalledWith("wait");
  expect(screen.getByText("Waiting for the runs to finish, then IssueRelay installs the update and restarts.")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenLastCalledWith("stop");
});

test("a plain quit says nothing about an update", async () => {
  await askedWith(1, async () => {});
  expect(screen.queryByText(/install the update/)).not.toBeInTheDocument();
});

test("a tray Quit while waiting to install keeps waiting and says IssueRelay quits instead", async () => {
  const { ask } = await askedWith(2, async () => {}, "update");
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(screen.getByText(/then IssueRelay installs the update and restarts/)).toBeInTheDocument();
  // The tray's Quit dropped the downloaded update; the app says again what follows.
  await ask(2, "quit");
  expect(screen.getByText("Waiting for the runs to finish, then IssueRelay quits.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Stop now" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Wait for completion" })).not.toBeInTheDocument();
});
```

`app/src/App.test.tsx` 的「the quit prompt works while the panel is replaced by the start failure」：`let ask: (running: number) => void = () => {};` 改为 `let ask: (prompt: QuitPrompt) => void = () => {};`，`ask(1)` 改为 `ask({ running: 1, purpose: "quit" })`，`import type { StartupState, StatusView } from "@/lib/types";` 补 `QuitPrompt`。

`app/src/lib/api.test.ts` 的「the panel calls exactly the commands its capability allows…」里 `expect(listen).toHaveBeenCalledTimes(4);` 改为 `expect(listen).toHaveBeenCalledTimes(5);`（第五个是 `onUpdateStatus`）。

`app/src/lib/errors.test.ts` 加（导入：`./backend` 那一行补 `ERROR_CODES`，`./errors` 那一行补 `updateFailureKey`，再加 `import { UPDATE_FAILURE_CODES } from "./types";` 与 `import cases from "./update-failure-cases.json";`；第二个测试守住 M2 的 `login_cancelled`）：

```ts
test("update failures follow the case table the Rust side reads too", () => {
  expect(cases.map((entry) => entry.code)).toEqual([...UPDATE_FAILURE_CODES]);
  for (const { code, error_code } of cases) {
    expect(ERROR_CODES).toContain(error_code);
    expect(updateFailureKey(code as (typeof UPDATE_FAILURE_CODES)[number])).toBe(`errors.${error_code}`);
    // `install_update` answers with the error code, which reads as the same translated headline.
    expect(describeError(new BackendError(error_code), t, "actions.failed.install_update")).toEqual({
      headline: t(`errors.${error_code}`),
      detail: null,
    });
  }
});

test("a cancelled browser login is still a code, not a failure", () => {
  expect(new BackendError("login_cancelled").code).toBe("login_cancelled");
});
```

`app/src/pages/Settings.test.tsx`：

- M2 的「automatic updates are saved with the page, and say they take effect later」改名为「automatic updates are saved with the page, and say when they check」，其中 `toHaveAccessibleDescription("Updates arrive in a later version; your choice is kept until then.")` 改为 `toHaveAccessibleDescription("Checks a minute after start and every 6 hours. Installing always waits for your click.")`，其余不变。
- 加：

```tsx
test("checking for updates shows the version and the result; a failure is a translated headline with the detail", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.checkUpdate = vi.fn().mockResolvedValue({ kind: "failed", code: "unreachable", detail: "error sending request" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Version 0.1.0")).toBeInTheDocument();
  expect(await screen.findByText(/Up to date/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("Could not reach the update server")).toBeInTheDocument();
  expect(screen.getByText("error sending request")).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败（组件、方法、类型与用例表的读取方不存在；`api.test.ts` 的监听数还是 4）。

- [ ] **Step 6: 前端实现与文案**

`types.ts` 加，并把 `QuitPrompt` 用到 `onQuitRequested`（见 `backend.ts`）：

```ts
/** The update failure codes (`FailureCode` in update.rs); update-failure-cases.json pins both sides. */
export const UPDATE_FAILURE_CODES = ["unreachable", "no_platform", "bad_signature", "unavailable", "install_failed", "other"] as const;
export type UpdateFailureCode = (typeof UPDATE_FAILURE_CODES)[number];
export type UpdateStatus =
  | { kind: "unchecked" }
  | { kind: "up_to_date"; checked_at: string }
  | { kind: "available"; version: string }
  | { kind: "failed"; code: UpdateFailureCode; detail: string };
export type QuitPurpose = "quit" | "update";
export interface QuitPrompt { running: number; purpose: QuitPurpose }
```

`backend.ts`：`ERROR_CODES` 是 M2 的两条加六条，`login_cancelled` 留着（取消浏览器登录不算失败，`useLinearLogin` 靠它不报错）：

```ts
/** Errors a command answers with as a code rather than a sentence; the panel translates them (`errors.*`). */
export const ERROR_CODES = [
  "not_a_linear_link",
  "login_cancelled",
  "update_unreachable",
  "update_no_platform",
  "update_bad_signature",
  "update_unavailable",
  "update_install_failed",
  "update_failed",
] as const;
```

`Backend` 在 `requestQuit` 之后加 `updateStatus(): Promise<UpdateStatus>;`、`checkUpdate(): Promise<UpdateStatus>;`、`installUpdate(): Promise<void>;`，在 `onStartupFailed` 之后加 `onUpdateStatus(listener: (status: UpdateStatus) => void): Promise<() => void>;`，并把 `onQuitRequested(listener: (running: number) => void)` 改为 `onQuitRequested(listener: (prompt: QuitPrompt) => void)`（类型导入补 `QuitPrompt`、`UpdateStatus`）。`api.ts`（类型导入补 `QuitPrompt`、`UpdateStatus`）：`requestQuit` 之后加三条命令，`onQuitRequested` 改写，末尾加 `onUpdateStatus`：

```ts
  updateStatus: () => call("update_status"),
  checkUpdate: () => call("check_update"),
  installUpdate: () => call("install_update"),
```

```ts
  onQuitRequested: (listener) => on<QuitPrompt>("quit-requested", listener),
```

```ts
  onUpdateStatus: (listener) => on<UpdateStatus>("update-status", listener),
```

`mock.ts`（类型导入补 `QuitPrompt`、`UpdateStatus`；`ago` 已经从 `./mock-fixtures` 导入）：`listeners.quit` 改为 `new Set<(prompt: QuitPrompt) => void>()`，加 `update: new Set<(status: UpdateStatus) => void>()`；在 `let autostart = false;` 之后加（名字不用 `update`：M2 的 `mock.ts` 已有改任务的 `const update = (id, change) => …`）：

```ts
  // What the app's last update check found; the fake never sends `update-status` by itself.
  let updateState: UpdateStatus = { kind: "up_to_date", checked_at: ago(30) };
```

返回对象在 `requestQuit` 之后加 `updateStatus: () => reply(() => updateState),`、`checkUpdate: () => reply(() => updateState),`、`installUpdate: () => reply(() => undefined),`，在 `onStartupFailed` 之后加 `onUpdateStatus: (listener) => subscribe(listeners.update, listener),`。

`lib/errors.ts` 加（文案键由用例表得出，不另写一份对应关系）：

```ts
import updateFailures from "./update-failure-cases.json";
import type { UpdateFailureCode } from "./types";

/** The translation key of an update failure's headline: `errors.<error code>`, by update-failure-cases.json. */
export function updateFailureKey(code: UpdateFailureCode): string {
  const entry = updateFailures.find((candidate) => candidate.code === code);
  return `errors.${entry?.error_code ?? "update_failed"}`;
}
```

`hooks/queries.ts`：`queryKeys` 加 `update: ["update"] as const,`，类型导入补 `UpdateStatus`；在 `useAppInfo` 之后加查询，在 `useRemoveCliFromPath` 之后加两个变更钩子（照 `useAction<Variables, Result>` 的签名）：

```ts
/** What the app's last update check found; `update-status` keeps it current (`useEngineEvents`). */
export function useUpdateStatus() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.update, queryFn: () => backend.updateStatus() });
}
```

```ts
/** Checks for an update now: what this check found is the action's `data`; the kept status is read again. */
export function useCheckUpdate() {
  return useAction<void, UpdateStatus>((backend) => backend.checkUpdate(), [queryKeys.update], "actions.failed.check_update");
}

/** "Install and restart": the Rust side downloads, verifies and goes through the quit flow. */
export function useInstallUpdate() {
  return useAction<void, void>((backend) => backend.installUpdate(), [], "actions.failed.install_update");
}
```

`hooks/useEngineEvents.ts`：文档注释补「the app's update check (`update-status`) is put into the cache as it comes」；在 `stopResync` 之后加订阅，清理函数里一并停掉它：

```ts
    const stopUpdate = subscription(backend.onUpdateStatus((status) => client.setQueryData(queryKeys.update, status)));
    return () => {
      stopEvents();
      stopResync();
      stopUpdate();
    };
```

`components/UpdateBanner.tsx`。换行规则：标题里的版本号可以在词中断行；两个按钮在 `flex-wrap` 里，605 px 放不下时上下排；失败的细节是 `DetailLine`（`break-words`）：

```tsx
import { CircleAlert, Download } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { DetailLine } from "@/components/states";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useInstallUpdate, useUpdateStatus } from "@/hooks/queries";
import { updateFailureKey } from "@/lib/errors";

/**
 * Above every page (design 12): a newer version with "Install and restart", or, once after a
 * restart, an update that could not be installed and why. Installing asks the Rust side, which
 * downloads and verifies the update and then goes through the quit flow: the panel itself never
 * quits or restarts. "Later" hides an offer for that version and "Dismiss" the failure, until the
 * panel is opened again. A failed check is left to the settings page: monitoring goes on, so it
 * does not nag here.
 */
export function UpdateBanner() {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const install = useInstallUpdate();
  const [laterFor, setLaterFor] = useState<string | null>(null);
  const [failureSeen, setFailureSeen] = useState(false);
  const shown = status.data;
  if (shown?.kind === "available" && laterFor !== shown.version) {
    return (
      <Alert className="mb-4" role="status">
        <Download className="size-4" aria-hidden />
        <AlertTitle className="wrap-anywhere">{t("update.available", { version: shown.version })}</AlertTitle>
        <AlertDescription className="space-y-2">
          <p className="text-muted-foreground">{t("update.restart_note")}</p>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={install.isPending} onClick={() => install.mutate()}>
              {install.isPending ? t("update.downloading") : t("update.install")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setLaterFor(shown.version)}>
              {t("update.later")}
            </Button>
          </div>
        </AlertDescription>
      </Alert>
    );
  }
  if (shown?.kind === "failed" && shown.code === "install_failed" && !failureSeen) {
    return (
      <Alert className="mb-4" role="alert">
        <CircleAlert className="size-4" aria-hidden />
        <AlertTitle>{t(updateFailureKey(shown.code))}</AlertTitle>
        <AlertDescription className="space-y-2">
          <DetailLine text={shown.detail} />
          <Button size="sm" variant="ghost" onClick={() => setFailureSeen(true)}>
            {t("update.dismiss")}
          </Button>
        </AlertDescription>
      </Alert>
    );
  }
  return null;
}
```

`layout/AppShell.tsx`：导入 `UpdateBanner`，内容区里 `<Outlet />` 上方放 `<UpdateBanner />`（同在最大宽度与内边距之内）。

`components/QuitDialog.tsx` 整个组件改为下面这样（导入照旧，另补 `useRef` 与类型 `QuitPrompt`）。等待中的询问框收到新的问题时只更新「之后做什么」，不回到提问：

```tsx
/**
 * The question that quitting with runs in progress asks (design 4.1, 6.2): wait for them, stop
 * them now, or keep running. "Install and restart" asks the same (design 12) and says that
 * IssueRelay then installs the update and restarts. Waiting can still become stopping, so once
 * chosen the prompt stays up with "stop now" until the app exits; a question that comes while it
 * waits only says again what follows (the tray's Quit drops a downloaded update, so IssueRelay then
 * quits instead). "Keep running" only closes the prompt: the app treats a question nobody answers
 * like no question at all and asks again the next time.
 */
export function QuitDialog() {
  const backend = useBackend();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  // Kept after the prompt closes, so that it does not read "0 runs" while it fades out.
  const [prompt, setPrompt] = useState<QuitPrompt>({ running: 0, purpose: "quit" });
  const [waiting, setWaiting] = useState(false);
  const [stopping, setStopping] = useState(false);
  // Read by the subscription, which is made once per backend: whether the prompt waits right now.
  const waitingNow = useRef(false);
  const markWaiting = (value: boolean) => {
    waitingNow.current = value;
    setWaiting(value);
  };

  useEffect(
    () =>
      subscription(
        backend.onQuitRequested((next) => {
          setPrompt(next);
          if (waitingNow.current) return;
          setStopping(false);
          setOpen(true);
        }),
      ),
    [backend],
  );

  const answer = (mode: QuitMode) => {
    const mark = mode === "wait" ? markWaiting : setStopping;
    mark(true);
    backend.quit(mode).catch((error: unknown) => {
      // The app did not take the answer: back to the question, and say why.
      mark(false);
      reportFailure(error, t, "common.error_title");
    });
  };

  const update = prompt.purpose === "update";
  return (
    <AlertDialog
      open={open}
      // Escape and a click outside answer "keep running", but only while the question is open.
      onOpenChange={(next) => {
        if (!next && !waiting && !stopping) setOpen(false);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("quit.title")}</AlertDialogTitle>
          <AlertDialogDescription>
            {waiting ? t(update ? "update.waiting" : "quit.waiting") : t("quit.body", { count: prompt.running })}
          </AlertDialogDescription>
          {update && !waiting && <p className="text-sm text-muted-foreground">{t("update.restart_body")}</p>}
        </AlertDialogHeader>
        <AlertDialogFooter>
          {!waiting && <AlertDialogCancel disabled={stopping}>{t("quit.cancel")}</AlertDialogCancel>}
          <Button variant="destructive" disabled={stopping} onClick={() => answer("stop")}>
            {t("quit.stop")}
          </Button>
          {!waiting && (
            <Button disabled={stopping} onClick={() => answer("wait")}>
              {t("quit.wait")}
            </Button>
          )}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
```

`pages/Settings.tsx`：导入补 `useCheckUpdate`、`useUpdateStatus`（`@/hooks/queries`；`useAppInfo` 已在）、`import { useLang } from "@/hooks/useLang";`、`import { formatRelative } from "@/lib/format";`、`import { updateFailureKey } from "@/lib/errors";`，`@/components/states` 那一行补 `DetailLine`。在 `CliOnPath` 之后加版本与检查更新一行。换行规则：版本与按钮在 `flex-wrap justify-between` 里，按钮放不下时换到下一行；结果是一行短句，失败的细节是 `DetailLine`：

```tsx
/** The running version, "Check for updates", and what the last check found (design 12). */
function UpdateCheck() {
  const { t } = useTranslation();
  const lang = useLang();
  const info = useAppInfo();
  const status = useUpdateStatus();
  const check = useCheckUpdate();
  // This check's own outcome first; otherwise what the app keeps (a background check's, or a
  // failed install's after the restart).
  const shown = check.data ?? status.data;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">{info.data ? t("shell.version", { version: info.data.version }) : t("common.loading")}</span>
        <Button size="sm" variant="outline" disabled={check.isPending} onClick={() => check.mutate()}>
          {check.isPending && <LoaderCircle aria-hidden className="animate-spin" />}
          {t("update.check")}
        </Button>
      </div>
      {shown?.kind === "up_to_date" && (
        <p className="text-xs text-muted-foreground">{t("update.up_to_date", { time: formatRelative(shown.checked_at, lang) })}</p>
      )}
      {shown?.kind === "available" && <p className="text-xs text-muted-foreground">{t("update.available", { version: shown.version })}</p>}
      {shown?.kind === "failed" && (
        <div>
          <p className="text-xs text-destructive">{t(updateFailureKey(shown.code))}</p>
          <DetailLine text={shown.detail} />
        </div>
      )}
    </div>
  );
}
```

「系统」分组的最终顺序（Task 2 与本任务合起来）：`<Autostart />`、`<CliOnPath />`、M2 的「自动检查更新」`SwitchRow`（不变，旁注 `settings.auto_update_note` 换成新文案）、`<UpdateCheck />`。版本号沿用 M2 侧栏已有的键 `shell.version`，不另加 `update.version`。

翻译键（`en` / `zh-CN` 如下；`zh-TW`、`ja`、`ko` 同步翻译，`{{…}}` 原样保留；`errors.update_*` 六个键与 `update-failure-cases.json` 一一对应）：

| 键 | en | zh-CN |
|---|---|---|
| `update.available` | IssueRelay {{version}} is available | IssueRelay {{version}} 已发布 |
| `update.install` | Install and restart | 安装并重启 |
| `update.downloading` | Downloading and verifying… | 正在下载并校验… |
| `update.later` | Later | 稍后 |
| `update.dismiss` | Dismiss | 知道了 |
| `update.restart_note` | IssueRelay restarts to install it; if runs are in progress you are asked first. | 安装时 IssueRelay 会重启；有任务在运行时会先问你。 |
| `update.restart_body` | IssueRelay restarts to install the update once these runs are settled. | 这些运行处理完后，IssueRelay 会重启并安装更新。 |
| `update.waiting` | Waiting for the runs to finish, then IssueRelay installs the update and restarts. | 正在等待运行结束，之后 IssueRelay 会安装更新并重启。 |
| `update.check` | Check for updates | 检查更新 |
| `update.up_to_date` | Up to date (checked {{time}}) | 已是最新版本（{{time}}检查） |
| `errors.update_unreachable` | Could not reach the update server | 连不上更新服务器 |
| `errors.update_no_platform` | The new version has no package for this computer yet | 新版本还没有适合这台电脑的安装包 |
| `errors.update_bad_signature` | The update's signature did not verify; nothing was installed | 更新包的签名校验没有通过，什么都没安装 |
| `errors.update_unavailable` | Updates install only into the installed app: move IssueRelay to Applications first | 只能更新已安装的应用：请先把 IssueRelay 移到「应用程序」 |
| `errors.update_install_failed` | The update could not be installed | 更新没能安装 |
| `errors.update_failed` | The update check failed | 检查更新失败 |
| `actions.failed.check_update` | Could not check for updates | 没能检查更新 |
| `actions.failed.install_update` | Could not install the update | 没能安装更新 |
| `settings.auto_update_note` | Checks a minute after start and every 6 hours. Installing always waits for your click. | 启动 1 分钟后检查一次，之后每 6 小时一次；安装总要等你点。 |
| `notify.update_available` | IssueRelay {{version}} is available | IssueRelay {{version}} 已发布 |
| `notify.update_available_body` | Open the panel to install it. | 打开面板即可安装。 |

Run: `pnpm --dir app test && pnpm --dir app lint` 与 `cargo test -p issuerelay-app`
Expected: 全部 PASS（含 `USED_KEYS` 检查、`the_new_version_notice_names_the_version_in_the_users_language`、缺键守卫、五种语言对齐、`api.test.ts` 的 40 条命令与 5 个监听、两边读用例表的测试）。

再量布局（第 12.7 节）：`pnpm --dir app dev`，用假后端把 `updateState` 临时改成 `{ kind: "available", version: "0.2.0" }` 看横幅与设置页的版本一行，再改成 `{ kind: "failed", code: "install_failed", detail: <一段 200 个字符的虚构原因> }` 看失败的横幅，看完改回。浏览器视口宽 900 px 与 885 px（内容区约 620 px 与 605 px），浅色与深色各一次：按钮都够得着，标题与细节断行而不横向溢出。结果写进提交说明。M2 Task 12 的 Playwright 布局测试若已落地，把横幅（有新版本）加进它的概览页用例。

- [ ] **Step 7: `check-secrets.sh` 拦截更新签名私钥**

`scripts/check-secrets.sh` 的 `PATTERNS` 在 `'AKIA[0-9A-Z]{16}'` 之后加：

```bash
  # A Tauri updater signing key: the minisign secret key box, as text or base64-encoded as
  # `tauri signer generate` writes it (docs/release/updater-key.md). The brackets keep plans
  # and docs that quote these lines from matching themselves.
  'untrusted comment: rsign encrypted secret k[e]y'
  'dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V[5]'
```

文件头第一段的列举补上「Tauri updater signing keys」。核对两个特征（不碰真正的私钥，用自己造的字符串）：

```bash
# The sample strings are put together from two halves, so this plan never holds a marker itself.
printf '%s%s\n' 'untrusted comment: rsign encrypted secret' ' key' | grep -c -E 'untrusted comment: rsign encrypted secret k[e]y'
printf '%s%s\n' 'xdW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V' '5Cg' | grep -c -E 'dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V[5]'
bash scripts/check-secrets.sh --strict; echo "exit $?"
```

Expected：`1`、`1`、`exit 0` 且没有别的输出（仓库里只有公钥，它的开头是 `…bWluaXNpZ24gcHVibGlj…` 而不是私钥的开头）。Step 0 已经核对过这个特征认得出真正的私钥。

- [ ] **Step 8: 文档（事实登记跟代码同一个提交，规则另提交）**

`docs/release/updater-key.md`（中文）：

~~~markdown
# 更新签名密钥

IssueRelay 的自动更新只安装用这把密钥签过名的更新包（设计第 12 节）。公钥提交在 `app/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`；私钥只在两处：维护者本机的 `~/.tauri/issuerelay.key`（权限 600）和 GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`。私钥不进仓库、不进聊天、不进日志和截图；`scripts/check-secrets.sh` 会拦下它。

## 生成（只做一次）

由主智能体或维护者在维护者自己的电脑上、仓库根目录运行；已有 `~/.tauri/issuerelay.key` 时不要重新生成（会让已安装的程序再也收不到更新）：

```bash
mkdir -p ~/.tauri && chmod 700 ~/.tauri
pnpm --dir app tauri signer generate --ci -w ~/.tauri/issuerelay.key > /dev/null
chmod 600 ~/.tauri/issuerelay.key
```

`--ci` 不提问、不设密码：GitHub 的 secret 不能是空值，没有密码就不需要密码 secret，也不会有密码要经过聊天或命令行历史。私钥靠文件权限与备份保护。`-w` 把私钥写进文件、公钥写进同名的 `.pub` 文件；命令的输出丢弃只是预防。

公钥写进配置（只读 `.pub`）：照 M3 计划 Task 3 Step 0 的 `node -e` 命令，或手动把 `~/.tauri/issuerelay.key.pub` 的那一行粘进 `plugins.updater.pubkey`（公钥可以公开）。

## 上传到 GitHub（维护者自己运行）

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo sunkailun1992/issuerelay < ~/.tauri/issuerelay.key
```

从文件读入，私钥不会出现在屏幕或 shell 历史里；不要用 `--body`。不建 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（现在的密钥没有密码；工作流里 `CI=true`，CLI 用空密码）。上传后 `gh secret list --repo sunkailun1992/issuerelay` 能看到这个名字（看不到值）。

## 备份

把 `~/.tauri/issuerelay.key` 作为附件存进密码管理器。丢了私钥，已安装的程序就再也收不到自动更新（它们只认这把公钥），用户只能手动下载安装带新公钥的版本。

## 核对

发布工作流用 `scripts/verify-update-signature.mjs` 拿提交的公钥核对每个更新包的签名，并核对签名里记的版本（`tauri build` 在私钥与公钥不配对时只警告、不失败，所以要单独核对）。

## 轮换与泄露

- 计划内轮换：生成新密钥对之前，先用旧私钥签发一个把 `pubkey` 换成新公钥的版本；已安装的程序装上它以后才认新公钥。之后的版本用新私钥签，替换 secret。
- 私钥泄露：攻击者要发出能被安装的更新，还得能往本仓库的 Releases 上传资产（需要写权限）或冒充 github.com 的 HTTPS。立刻按上一条轮换；同时检查仓库的协作者、Actions 日志与 Releases。
~~~

事实登记（跟代码同一个提交）：

- 设计第 12 节「自动更新」一条改为：「使用 Tauri updater（`tauri-plugin-updater`）：设置里「自动检查更新」打开时，启动 1 分钟后检查一次、之后每 6 小时一次，也可以在设置里手动检查；清单固定为 `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json`，只接受比当前版本新、用仓库里的公钥验签通过、且签名里记的版本与清单一致的更新包。后台检查发现新版本时发一次系统通知（每个版本一次，没有单独的开关，跟随「自动检查更新」）、面板顶部显示横幅；安装由用户点「安装并重启」：先下载并验签，有任务在跑时走与退出相同的询问（等待完成 / 立即停止），宿主停下后安装并重启（Windows 交给 NSIS 安装程序），安装期间关窗或再次退出都不结束进程；等待中从托盘退出则丢掉已下载的更新、只退出。检查失败、清单缺本平台、验签失败、版本不新只提示，不影响监控；宿主停下后才装不上时按原版本重启，面板顶部说明原因（不发通知）。更新包用单独的签名密钥签名，私钥只在维护者本机与 GitHub Actions secrets（`docs/release/updater-key.md`）；`tauri.conf.json` 里的公钥是更新的信任根。」
- 设计第 9.5 节改为：「「开始运行」「进入待验收」「需要补充信息」「失败」四种运行通知，可以分别关闭；另有自动更新发现新版本时的一条通知（每个版本一次），它没有单独的开关：只有设置里「自动检查更新」打开时才会后台检查、才会发它（第 12 节）。更新装不上只在面板里说明，不发通知。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 自动更新（检查、验签、经退出流程安装） | app/src-tauri/src/update.rs |`；第 6 节的表加一行：`| 装不上的更新 | <数据目录>/com.sunkailun.issuerelay/update-install-failure.txt | 「安装并重启」在宿主停下后没装上时写下清洗过的一行原因，程序按原版本重启后读出、删除，面板顶部的横幅显示一次 |`。
- `docs/ai-coding/TESTING_SPEC.md` 假实现清单加一行：`| FakeSource | app/src-tauri/src/update.rs 的测试模块 | 更新插件（按脚本给出检查结果与下载结果，并计数下载）；测试不访问网络 |`；「面板与 core 共用的用例表」加一行：`| app/src/lib/update-failure-cases.json | errors.test.ts（UPDATE_FAILURE_CODES、ERROR_CODES、updateFailureKey 与 errors.* 文案） | app/src-tauri/src/update.rs 的测试（FailureCode 的 serde 名与 error_code） |`。
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 5 节登记 `docs/release/`（发布相关说明：更新签名密钥）。
- `.github/CODEOWNERS` 加两行（`plugins.updater.pubkey` 是更新的信任根，改它就能换掉已安装程序信任的签名者）：

  ```text
  /app/src-tauri/tauri.conf.json     @sunkailun1992
  /app/src-tauri/tauri.*.conf.json   @sunkailun1992
  ```

规则（单独的 `docs` 提交，只含规则文件，第 11 节）：

- `docs/ai-coding/AI_CODING_GUIDE.md` 第 7 节「面板的类型」一条末尾补：「`quit-requested` 的载荷是 `QuitPrompt { running, purpose }`，`update-status` 的载荷是 `UpdateStatus`；改它们时同步 `app/src/lib/types.ts`、`mock.ts` 与读它们的组件（`QuitDialog`、`UpdateBanner`）。」
- `docs/ai-coding/SECURITY_CODING_SPEC.md` 第 8 节「供应链」末尾加一条：「自动更新（M3）：程序只向 GitHub Releases 发出检查更新的请求（清单 `…/releases/latest/download/latest.json`，安装包经 `api.github.com` 的资产地址下载），不带任何用户数据，也不算遥测。`tauri.conf.json` 的 `plugins.updater.pubkey` 是更新的信任根：只安装用这把公钥验签通过、签名里记的版本与清单一致（`requireSignedVersion`）且比当前版本新的更新包，从不安装未签名或更旧的版本，也不在开发版或从磁盘映像运行时安装；改这把公钥（`tauri.conf.json`、`tauri.*.conf.json`）要经 CODEOWNERS 评审。私钥只在维护者本机与 GitHub Actions secrets，`check-secrets.sh` 拦截它（`docs/release/updater-key.md`）。」

Run: `pnpm --dir app build` 与六条验证命令
Expected: 全部通过。

- [ ] **Step 9: Commit**

先提交代码。`AI_CODING_GUIDE.md` 在这个提交里只带第 5、6 节的事实登记：先只改这两节，提交后再改第 7 节（下面的规则提交）。

```bash
git status --short
git add app/src-tauri/tauri.conf.json app/src-tauri/tauri.updater.conf.json app/src-tauri/Cargo.toml Cargo.lock \
  app/src-tauri/build.rs app/src-tauri/capabilities/default.json app/src-tauri/tests/packaging.rs \
  app/src-tauri/src/update.rs app/src-tauri/src/lib.rs app/src-tauri/src/quit.rs app/src-tauri/src/events.rs \
  app/src-tauri/src/notify.rs app/src-tauri/src/commands.rs app/src-tauri/src/i18n.rs crates/core/src/config.rs \
  app/src/lib/update-failure-cases.json app/src/lib/types.ts app/src/lib/backend.ts app/src/lib/api.ts \
  app/src/lib/api.test.ts app/src/lib/mock.ts app/src/lib/errors.ts app/src/lib/errors.test.ts \
  app/src/hooks/queries.ts app/src/hooks/useEngineEvents.ts app/src/components/UpdateBanner.tsx \
  app/src/components/UpdateBanner.test.tsx app/src/components/QuitDialog.tsx app/src/components/QuitDialog.test.tsx \
  app/src/App.test.tsx app/src/components/layout/AppShell.tsx app/src/pages/Settings.tsx app/src/pages/Settings.test.tsx \
  app/src/i18n/locales/en.json app/src/i18n/locales/zh-CN.json app/src/i18n/locales/zh-TW.json \
  app/src/i18n/locales/ja.json app/src/i18n/locales/ko.json scripts/check-secrets.sh docs/release/updater-key.md \
  docs/superpowers/specs/2026-10-10-issuerelay-design.md docs/ai-coding/AI_CODING_GUIDE.md \
  docs/ai-coding/TESTING_SPEC.md docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md .github/CODEOWNERS
git diff --cached --stat
git commit -F - <<'EOF'
feat(app): add signed automatic updates from GitHub Releases

The updater plugin checks latest.json on GitHub Releases (a minute after start and every six
hours while automatic checks are on) and keeps only payloads that verify against the committed
public key. "Install and restart" goes through the quit flow: with runs in progress the same
question as quitting, then QuitState::Installing until the install returns, then a restart.
quit-requested now carries QuitPrompt { running, purpose }. Three commands, registered in
generate_handler!, PANEL_COMMANDS and the capability (40 commands).

Layout: the update banner and the version row checked at 620 px and 605 px content width,
light and dark; titles and details wrap, the buttons move under each other when needed.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

再提交规则：

```bash
git status --short
git add docs/ai-coding/AI_CODING_GUIDE.md docs/ai-coding/SECURITY_CODING_SPEC.md
git diff --cached --stat
git commit -F - <<'EOF'
docs(ai-coding): describe the quit and update payloads and the updater's trust root

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

两次提交前都确认 `git diff --cached` 里没有 `~/.tauri` 下的任何文件（它们本来就在仓库之外），`bash scripts/check-secrets.sh --strict` 无输出。主智能体在本任务的报告里把 Step 0 的 `gh secret set` 命令交给维护者。

---

### Task 4: 发布工作流

设计第 12 节（推送版本标签时在 macOS 和 Windows 上构建、测试、打包，发布到 GitHub Releases）、16.1 节（维护者发布、升版本号）。调研结论（`tauri-apps/tauri-action` v1.0.0 源码）：

- 用 `@v1`，本计划固定到 v1.0.0 的提交 `1deb371b0cd8bd54025b384f1cd735e725c4060f`。输入 `includeUpdaterJson` 已改名为 `uploadUpdaterJson`（默认 true）；`updaterJsonPreferNsis: true` 让 `latest.json` 的通用键 `windows-x86_64` 指向 NSIS 安装包（默认指向 MSI）；输出 `releaseId`、`artifactPaths`（生成文件的 JSON 数组，不上传时也有）、`appVersion`。
- 没有 `tagName`、`releaseId` 时只构建、不上传；两个矩阵任务各自「先找后建」Release，可能建出两个同标签的草稿；`latest.json` 由每个任务下载、合并、删除旧资产再上传，两个任务交错时可能丢掉一个平台。所以：单独的 `create-release` 任务只建一个草稿并输出 `releaseId`，矩阵任务只经 `releaseId` 上传，并且逐个运行（`max-parallel: 1`），最后的 `verify-release` 再核对一遍。
- `latest.json` 里的 `url` 是 GitHub API 的资产地址（`api.github.com/repos/…/releases/assets/<id>`），更新插件带 `Accept: application/octet-stream` 下载，会被重定向到文件；只有草稿被公开、且它是仓库最新的正式版本时，`…/releases/latest/download/latest.json` 才指向它。
- 通用包 `--target universal-apple-darwin` 写出 `darwin-aarch64`、`darwin-x86_64`、`darwin-universal`（及 `-app` 变体）三组键，都指向同一个 `.app.tar.gz`；需要两个 Apple 目标都装在固定的 1.99.0 工具链上。资产名：`IssueRelay_<版本>_universal.dmg`、`IssueRelay_<版本>_universal.app.tar.gz`（与 `.sig`）、`IssueRelay_<版本>_x64-setup.exe`（与 `.sig`）、`IssueRelay_<版本>_x64_en-US.msi`（与 `.sig`）。
- 上传工作流产物时 `upload-artifact` 的压缩包会丢掉 Unix 权限，所以只传 `.dmg` 与安装包，不传 `.app` 目录。macOS 上 tauri-action 默认设置 `TAURI_BUNDLER_DMG_IGNORE_CI=true`。`macos-latest` 是 macOS 26 arm64，`windows-latest` 是 Windows Server 2025。
- 可选的 secrets 只经步骤级 `env:` 交给用到它们的那一步，从不写进 `$GITHUB_ENV`（写进去的值会进入之后的每一步，包括 `pnpm build` 与 cargo 的构建脚本）。空字符串的 `APPLE_CERTIFICATE` 之类会让打包器去导入一个空证书，所以 Apple 的六个值只出现在「有 Apple 证书」那一个 tauri-action 步骤里，没有证书时走另一个不带它们的步骤；更新签名私钥只在打开更新包时给（`createUpdaterArtifacts: false` 时 CLI 不读它；执行时按解析到的 tauri-cli 核对，空值有影响就把 tauri-action 步骤再按有无私钥分开）。
- 拉取请求触发的工作流可能运行 PR 里改过的构建脚本（pnpm、cargo），所以试打包只给只读的令牌（`contents: read`）；只有推送标签的正式发布才有 `contents: write`。权限按任务给、不能按矩阵条目或表达式给，所以打包分成两个任务（`bundle-release`、`bundle-dry-run`），共用可复用工作流 `bundle.yml`，调用方决定它的权限与 secrets。
- 发布门禁写成 `scripts/release-checks.mjs` 里的纯函数，每道门都有通过与不通过的自测（版本不符、提交不在 main 上、缺资产、缺平台键、签名为空、地址不是本 Release 的资产、通用键指向 MSI）；`verify` 先跑自测，试打包对将要打的标签 `v<版本>`、将要打标签的提交（`origin/main`）和试打包产物跑同样的门，所以没有哪道门第一次运行就在正式标签上（裁定 M3-AR7）。tauri-action 写的真实 `latest.json` 只在正式发布时才有（试打包没有 Release 可传），试打包检查的是由产物得出的「将会写出的」`latest.json`，真实那份由 `verify-release` 检查，不对就标记草稿。

工作流的形状：

- 触发：推送 `v*` 标签 = 正式发布；`workflow_dispatch` 与改了打包相关文件的 `pull_request` = 试打包（同样的门禁、测试和打包，产物存为工作流产物，不建 Release）。GitHub 只在工作流文件已经在默认分支上时才允许手动运行它，所以合入之前的第一次试打包靠 M3 自己的 PR 触发。拉取请求触发的试打包拿不到任何 secret（调用 `bundle.yml` 时传空值）。
- `verify`（ubuntu）：门禁自测；读工作区版本；标签等于 `v<版本>`（试打包用将要打的标签）；标签提交是 `origin/main` 的祖先（试打包用 `origin/main` 本身）；`check-secrets --strict`；记下哪些可选 secrets 齐全（只看是否为空，从不打印值）。
- `test`（macOS、Windows）：六条验证命令。
- `create-release`（ubuntu，只在正式发布时，`contents: write`）：同一标签已有 Release 就失败；否则建一个草稿，正文写安装说明链接、签名与自动更新的实际情况，输出 `id`。
- `bundle-release`（只在正式发布时，`contents: write`）/ `bundle-dry-run`（只在试打包时，`contents: read`）：都调用 `bundle.yml`——矩阵（macOS 通用包、Windows x64）逐个运行：装工具链与依赖，`stage-cli`，构建面板并确认不是占位页，按 secrets 选配置与签名，`tauri-action` 构建（正式发布时经 `releaseId` 上传并合并 `latest.json`），核对包内容，有更新包时用提交的公钥核对签名，试打包时上传工作流产物。
- `verify-release`（ubuntu，只要草稿建成了就运行，`contents: write`）：草稿仍是草稿；资产齐全；有更新包时真实 `latest.json` 的版本、每个平台键的签名与地址（指向本 Release 的哪个资产）；任何一项不对就失败并把草稿标题改成「incomplete: do not publish」。
- `verify-dry-run`（ubuntu，只在试打包时，`contents: read`）：下载两个平台的工作流产物，跑同样的资产门与（有更新包时）「将会写出的」`latest.json` 门。
- 权限：顶层 `contents: read`；`contents: write` 只给 `create-release`、`bundle-release`、`verify-release`。每个任务都有 `timeout-minutes` 和如实说明的注释（调用可复用工作流的任务不能写，上限在 `bundle.yml` 里）。

**Files:**
- Create: `.github/workflows/release.yml`、`.github/workflows/bundle.yml`
- Create: `scripts/verify-update-signature.mjs`
- Modify: `scripts/release-checks.mjs`（Task 1 建的文件：加标签、`origin/main`、资产、`latest.json` 几道门与自测）
- Modify: `.github/workflows/ci.yml`（`secrets` 任务加一步 actionlint；必需检查的名字不变）
- Modify（事实登记，跟代码同一个提交）：`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 6 节）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 12、18 节）、`.github/CODEOWNERS`（`/scripts/stage-cli.sh`、`/scripts/stage-cli.ps1`、`/scripts/verify-update-signature.mjs`、`/scripts/release-checks.mjs`）
- Modify（规则，单独的 `docs` 提交）：`MAINTAINERS.md`（英文摘要的发布一条、「发布」一节、「分支保护」表里 `secrets` 一项）、`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md`（第 7.1 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 10 节）

**Interfaces:**
- Consumes：Task 1 的 `scripts/stage-cli.sh --universal`、`scripts/stage-cli.ps1`、`app/src-tauri/tauri.bundle.conf.json`、`scripts/release-checks.mjs`（`panel`、`self-test`、导出的 `PLACEHOLDER_MARKER`）；Task 3 的 `app/src-tauri/tauri.updater.conf.json`、`plugins.updater.pubkey`；M2 的 `app/package.json`（`packageManager` 字段、`lint`/`test`/`build`/`tauri` 脚本）、`app/pnpm-lock.yaml`、`ci.yml` 的 `test` 任务（`rustup toolchain install 1.99.0 …`、`Swatinem/rust-cache@v2`、`pnpm/action-setup@v6`、`actions/setup-node@v7` 与 Node 24，以及每个任务的 `timeout-minutes` 和说明）与 `secrets` 任务；`scripts/check-secrets.sh`；`.github/dependabot.yml` 现有的 github-actions 条目（它也更新按 SHA 固定的 action，不用改）。
- Produces：工作流 `release`（任务 `verify`、`test`、`create-release`、`bundle-release`、`bundle-dry-run`、`verify-release`、`verify-dry-run`；`verify` 的输出 `version`、`updater`、`apple`、`windows`，`create-release` 的输出 `id`）；可复用工作流 `bundle`（`workflow_call` 输入 `version`、`updater`、`apple`、`windows`、`release-id`，可选 secrets 同名）；试打包的工作流产物 `bundles-macos-universal`、`bundles-windows-x64`；`node scripts/verify-update-signature.mjs --version <X.Y.Z> --artifacts '<JSON 数组>' [--pubkey <base64>]`（退出码 0 / 1 / 2）；`node scripts/release-checks.mjs` 的 `workspace-version`、`tag --tag <标签> [--manifest <Cargo.toml>]`、`on-main --commit <提交> [--ref <引用>]`、`assets --version <X.Y.Z> --updater true|false --mode release|dry-run --names <文件>`、`latest-json --version <X.Y.Z> --latest <文件> --assets <文件>`（每行 `<id>\t<名字>`）、`latest-json-dry-run --version <X.Y.Z> --dir <目录>`，导出 `workspaceVersion`、`tagProblems`、`onMainProblems`、`expectedAssets`、`assetProblems`、`PLATFORM_KEYS`、`latestJsonProblems`、`wouldBeLatestJson`；secrets 名：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（不建）、`APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID`、`WINDOWS_CERTIFICATE`、`WINDOWS_CERTIFICATE_PASSWORD`。

- [ ] **Step 1: 写签名核对脚本**

`scripts/verify-update-signature.mjs`：

```js
#!/usr/bin/env node
// Checks Tauri updater signatures (minisign) against the public key committed in
// app/src-tauri/tauri.conf.json, and that each signature's trusted comment names the release
// version. `tauri build` only warns when the signing key does not match the public key, so the
// release workflow runs this on every update payload; a mismatch fails the release.
//
// Usage: node scripts/verify-update-signature.mjs --version <X.Y.Z> --artifacts '<JSON array>' [--pubkey <base64>]
//   --artifacts  tauri-action's `artifactPaths` output; every `<file>.sig` in it is checked
//                against `<file>`, and at least one must be there
//   --pubkey     another public key instead of the committed one (the local self-test)
// Exit codes: 0 = every signature verified, 1 = one did not, 2 = bad arguments or unreadable files.
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// DER header of an Ed25519 SubjectPublicKeyInfo; the 32 key bytes follow it.
const ED25519_SPKI = Buffer.from("302a300506032b6570032100", "hex");

/** The text lines of a base64-encoded minisign box, the way Tauri stores keys and signatures. */
function boxLines(base64Text) {
  return Buffer.from(base64Text.trim(), "base64").toString("utf8").split("\n").map((line) => line.replace(/\r$/, ""));
}

function publicKey(base64Key) {
  const [comment, encoded] = boxLines(base64Key);
  if (!comment?.startsWith("untrusted comment:") || !encoded) throw new Error("the pubkey is not a minisign public key");
  const raw = Buffer.from(encoded, "base64");
  if (raw.length !== 42 || raw.subarray(0, 2).toString("latin1") !== "Ed") throw new Error("unexpected minisign public key format");
  return {
    keyId: raw.subarray(2, 10),
    key: createPublicKey({ key: Buffer.concat([ED25519_SPKI, raw.subarray(10)]), format: "der", type: "spki" }),
  };
}

/** Why `payload` does not verify against `pub`, or null when it does. */
function problem(pub, payload, version) {
  const [, sigLine, trustedLine, globalLine] = boxLines(readFileSync(`${payload}.sig`, "utf8"));
  const sig = Buffer.from(sigLine ?? "", "base64");
  if (sig.length !== 74) return "the .sig file is not a minisign signature";
  if (!sig.subarray(2, 10).equals(pub.keyId)) {
    return "it was signed with another key: the TAURI_SIGNING_PRIVATE_KEY secret does not match the committed pubkey";
  }
  const data = readFileSync(payload);
  // "ED" signs the BLAKE2b-512 hash of the file (what Tauri writes); "Ed" signs the file itself.
  const message = sig.subarray(0, 2).toString("latin1") === "ED" ? createHash("blake2b512").update(data).digest() : data;
  if (!verify(null, message, pub.key, sig.subarray(10))) return "the signature does not match the file";
  if (!trustedLine?.startsWith("trusted comment: ")) return "the signature has no trusted comment";
  const trusted = trustedLine.slice("trusted comment: ".length);
  const globalSig = Buffer.from(globalLine ?? "", "base64");
  if (!verify(null, Buffer.concat([sig.subarray(10), Buffer.from(trusted, "utf8")]), pub.key, globalSig)) {
    return "the trusted comment was altered";
  }
  if (!trusted.split("\t").includes(`version:${version}`)) {
    return `the signature does not name version ${version} (trusted comment: ${trusted})`;
  }
  return null;
}

function main() {
  const { values } = parseArgs({
    options: { version: { type: "string" }, artifacts: { type: "string" }, pubkey: { type: "string" } },
  });
  if (!values.version || !values.artifacts) {
    throw new Error("usage: node scripts/verify-update-signature.mjs --version <X.Y.Z> --artifacts '<JSON array>' [--pubkey <base64>]");
  }
  const config = JSON.parse(readFileSync(path.join(root, "app/src-tauri/tauri.conf.json"), "utf8"));
  const pub = publicKey(values.pubkey ?? config.plugins?.updater?.pubkey ?? "");
  const signatures = JSON.parse(values.artifacts).filter((file) => file.endsWith(".sig"));
  if (signatures.length === 0) throw new Error("no update signature among the artifacts");
  let failed = false;
  for (const signature of signatures) {
    const payload = signature.slice(0, -".sig".length);
    const why = problem(pub, payload, values.version);
    console.log(why === null ? `verified ${path.basename(payload)}` : `FAILED ${path.basename(payload)}: ${why}`);
    failed ||= why !== null;
  }
  return failed ? 1 : 0;
}

try {
  process.exitCode = main();
} catch (error) {
  console.error(`verify-update-signature: ${error.message}`);
  process.exitCode = 2;
}
```

用一把一次性的测试密钥自测（不碰 `~/.tauri/issuerelay.key`；`tauri signer sign` 的参数以 `pnpm --dir app tauri signer sign --help` 为准）：

```bash
SCRATCH=$(mktemp -d)
pnpm --dir app tauri signer generate --ci -w "$SCRATCH/test.key" > /dev/null
printf 'payload' > "$SCRATCH/IssueRelay.app.tar.gz"
pnpm --dir app tauri signer sign -f "$SCRATCH/test.key" -p "" --app-version 0.1.0 "$SCRATCH/IssueRelay.app.tar.gz" > /dev/null
PUB=$(cat "$SCRATCH/test.key.pub")
ARTIFACTS="[\"$SCRATCH/IssueRelay.app.tar.gz\",\"$SCRATCH/IssueRelay.app.tar.gz.sig\"]"
node scripts/verify-update-signature.mjs --version 0.1.0 --artifacts "$ARTIFACTS" --pubkey "$PUB"; echo "exit $?"
node scripts/verify-update-signature.mjs --version 0.1.1 --artifacts "$ARTIFACTS" --pubkey "$PUB"; echo "exit $?"
node scripts/verify-update-signature.mjs --version 0.1.0 --artifacts "$ARTIFACTS"; echo "exit $?"
printf 'tampered' > "$SCRATCH/IssueRelay.app.tar.gz"
node scripts/verify-update-signature.mjs --version 0.1.0 --artifacts "$ARTIFACTS" --pubkey "$PUB"; echo "exit $?"
node scripts/verify-update-signature.mjs --version 0.1.0 --artifacts '[]'; echo "exit $?"
rm -rf "$SCRATCH"
```

Expected，依次：`verified IssueRelay.app.tar.gz` / `exit 0`；`FAILED …: the signature does not name version 0.1.1 …` / `exit 1`；用提交的公钥核对测试密钥的签名：`FAILED …: it was signed with another key …` / `exit 1`；改过的文件：`FAILED …: the signature does not match the file` / `exit 1`；`verify-update-signature: no update signature among the artifacts` / `exit 2`。第一条若是 `the signature does not match the file`，先确认 `.sig` 的第二行解码后前两个字节是 `ED`（预哈希）还是 `Ed`，脚本两种都处理；仍不对就停下来报告。


- [ ] **Step 2: 发布门禁（`scripts/release-checks.mjs`，先写失败的自测）**

把文件头的用法注释换成完整的一份：

```js
// Usage: node scripts/release-checks.mjs <gate> [options]
//   panel [--index <file>]          the panel about to be embedded (app/dist/index.html by default)
//                                   is the real one, not build.rs's placeholder page
//   tag --tag <tag> [--manifest <Cargo.toml>]
//                                   the tag names the workspace version, which is X.Y.Z
//   on-main --commit <sha> [--ref <ref>]
//                                   the commit is on the ref (origin/main by default)
//   assets --version <X.Y.Z> --updater true|false --mode release|dry-run --names <file>
//                                   every asset the release (or the dry run) must have is listed
//                                   in <file>, one name per line
//   latest-json --version <X.Y.Z> --latest <file> --assets <file>
//                                   latest.json announces the version, and every platform key has
//                                   a signature and points at the right asset of the release
//                                   (<file>: one "<id><TAB><name>" line per asset)
//   latest-json-dry-run --version <X.Y.Z> --dir <dir>
//                                   the same for the latest.json a release of the dry run's
//                                   bundles in <dir> would get
//   workspace-version               prints the version in the root Cargo.toml (not a gate)
//   self-test                       every gate against made-up input, passing and failing cases
// Exit codes: 0 = passed, 1 = a gate failed, 2 = bad arguments or unreadable input.
```

导入改为：

```js
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
```

`SELF_TESTS` 里在 `panel` 之后加四组（含裁定要求的反例：版本不符、缺资产、缺平台键；`on-main` 在 `mkdtemp` 建的一次性 git 仓库里测，用完删掉）：

```js
  tag() {
    const manifest = '[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "0.2.0"\nedition = "2024"\n';
    assert.equal(workspaceVersion(manifest), "0.2.0");
    assert.throws(() => workspaceVersion('[package]\nversion = "0.2.0"\n'), /no \[workspace\.package\] version/);
    assert.deepEqual(tagProblems("v0.2.0", "0.2.0"), []);
    assert.match(tagProblems("v0.2.1", "0.2.0").join(), /does not name the workspace version 0\.2\.0/);
    assert.match(tagProblems("0.2.0", "0.2.0").join(), /expected v0\.2\.0/);
    assert.match(tagProblems("v0.2.0-rc.1", "0.2.0-rc.1").join(), /not X\.Y\.Z/);
    // The repository's own Cargo.toml reads.
    assert.match(workspaceVersion(readFileSync(path.join(root, "Cargo.toml"), "utf8")), /^\d+\.\d+\.\d+$/);
  },
  "on-main"() {
    const repo = mkdtempSync(path.join(tmpdir(), "release-checks-"));
    try {
      const git = (...args) =>
        execFileSync("git", ["-c", "user.name=Demo", "-c", "user.email=demo@example.com", "-c", "commit.gpgsign=false", ...args], {
          cwd: repo,
          stdio: "pipe",
        })
          .toString()
          .trim();
      git("init", "--quiet", "--initial-branch=main");
      git("commit", "--quiet", "--allow-empty", "-m", "one");
      const onMain = git("rev-parse", "HEAD");
      git("switch", "--quiet", "-c", "side");
      git("commit", "--quiet", "--allow-empty", "-m", "side work");
      const offMain = git("rev-parse", "HEAD");
      assert.deepEqual(onMainProblems(onMain, "main", repo), []);
      assert.match(onMainProblems(offMain, "main", repo).join(), /is not on main/);
    } finally {
      rmSync(repo, { recursive: true, force: true });
    }
  },
  assets() {
    const release = expectedAssets("0.2.0", true, "release");
    assert.deepEqual(assetProblems(release, "0.2.0", true, "release"), []);
    assert.deepEqual(assetProblems(release.filter((name) => !name.endsWith(".msi")), "0.2.0", true, "release"), [
      "missing asset IssueRelay_0.2.0_x64_en-US.msi",
    ]);
    // Bundles of another version are none of this version's assets.
    assert.equal(assetProblems(expectedAssets("0.1.9", true, "release"), "0.2.0", true, "release").length, release.length);
    // Without the update key: no payloads, no signatures, no latest.json.
    assert.deepEqual(expectedAssets("0.2.0", false, "release"), [
      "IssueRelay_0.2.0_universal.dmg",
      "IssueRelay_0.2.0_x64-setup.exe",
      "IssueRelay_0.2.0_x64_en-US.msi",
    ]);
    // A dry run keeps the bundler's own name of the macOS payload and has no latest.json.
    assert.ok(expectedAssets("0.2.0", true, "dry-run").includes("IssueRelay.app.tar.gz.sig"));
    assert.ok(!expectedAssets("0.2.0", true, "dry-run").includes("latest.json"));
  },
  "latest-json"() {
    const assets = [
      { id: "11", name: "IssueRelay_0.2.0_universal.app.tar.gz" },
      { id: "22", name: "IssueRelay_0.2.0_x64-setup.exe" },
      { id: "33", name: "IssueRelay_0.2.0_x64_en-US.msi" },
    ];
    const api = (id) => `https://api.github.com/repos/demo/issuerelay/releases/assets/${id}`;
    const entry = (id) => ({ signature: "c2lnbmF0dXJl", url: api(id) });
    const idFor = { ".app.tar.gz": "11", "-setup.exe": "22", ".msi": "33" };
    const good = {
      version: "0.2.0",
      platforms: Object.fromEntries(Object.entries(PLATFORM_KEYS).map(([key, suffix]) => [key, entry(idFor[suffix])])),
    };
    assert.deepEqual(latestJsonProblems(good, "0.2.0", assets), []);
    assert.match(latestJsonProblems({ ...good, version: "0.1.9" }, "0.2.0", assets).join(), /announces 0\.1\.9, not 0\.2\.0/);
    const withoutUniversal = { ...good, platforms: { ...good.platforms } };
    delete withoutUniversal.platforms["darwin-universal"];
    assert.match(latestJsonProblems(withoutUniversal, "0.2.0", assets).join(), /no entry for darwin-universal/);
    const unsigned = { ...good, platforms: { ...good.platforms, "windows-x86_64-msi": { signature: "", url: api("33") } } };
    assert.match(latestJsonProblems(unsigned, "0.2.0", assets).join(), /windows-x86_64-msi has no signature/);
    const elsewhere = { ...good, platforms: { ...good.platforms, "darwin-aarch64": entry("99") } };
    assert.match(latestJsonProblems(elsewhere, "0.2.0", assets).join(), /no asset of this release/);
    const msiForAll = { ...good, platforms: { ...good.platforms, "windows-x86_64": entry("33") } };
    assert.match(latestJsonProblems(msiForAll, "0.2.0", assets).join(), /windows-x86_64 points at .*\.msi, not the -setup\.exe payload/);
    // A dry run's would-be latest.json passes the same gate; a missing signature fails it.
    const bundles = ["IssueRelay.app.tar.gz", "IssueRelay_0.2.0_x64-setup.exe", "IssueRelay_0.2.0_x64_en-US.msi"].map((name) => ({ id: name, name }));
    assert.deepEqual(latestJsonProblems(wouldBeLatestJson("0.2.0", bundles, () => "c2ln"), "0.2.0", bundles), []);
    assert.match(latestJsonProblems(wouldBeLatestJson("0.2.0", bundles, () => ""), "0.2.0", bundles).join(), /has no signature/);
  },
```

Run: `node scripts/release-checks.mjs self-test; echo "exit $?"`
Expected: `ok panel`，另外四行 `FAILED tag: workspaceVersion is not defined` 之类，`exit 1`。

再在 `// ---- gates` 一节里 `panelProblems` 之后加门禁本身：

```js
/** The workspace version: `version` in the `[workspace.package]` table of the root Cargo.toml. */
export function workspaceVersion(cargoToml) {
  const table = cargoToml.split(/^\[/m).find((part) => part.startsWith("workspace.package]"));
  const match = table?.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("Cargo.toml has no [workspace.package] version");
  return match[1];
}

/** What is wrong with releasing `version` under `tag`. */
export function tagProblems(tag, version) {
  const problems = [];
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    problems.push(`the workspace version ${version} is not X.Y.Z (MSI refuses pre-release suffixes like -rc.1)`);
  }
  if (tag !== `v${version}`) problems.push(`tag ${tag} does not name the workspace version ${version} in Cargo.toml (expected v${version})`);
  return problems;
}

/** Whether `commit` is on `ref` (it, or one of its ancestors), as git says. */
export function onMainProblems(commit, ref, cwd = root) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", commit, ref], { cwd, stdio: "ignore" });
    return [];
  } catch (error) {
    if (error.status === 1) return [`${commit} is not on ${ref}: a release is built only from a commit of main`];
    throw new Error(`git merge-base could not compare ${commit} with ${ref}`);
  }
}

/**
 * The assets a release (`mode` "release") or a dry run's bundles ("dry-run") of `version` must
 * have. tauri-action renames the macOS update payload when it uploads it; in a dry run it keeps
 * the bundler's own name. (Names from tauri-action v1.0.0 and tauri-bundler 2.10.2; the first
 * dry run shows the real ones.)
 */
export function expectedAssets(version, updater, mode) {
  const names = [`IssueRelay_${version}_universal.dmg`, `IssueRelay_${version}_x64-setup.exe`, `IssueRelay_${version}_x64_en-US.msi`];
  if (!updater) return names;
  const payload = mode === "release" ? `IssueRelay_${version}_universal.app.tar.gz` : "IssueRelay.app.tar.gz";
  names.push(payload, `${payload}.sig`, `IssueRelay_${version}_x64-setup.exe.sig`, `IssueRelay_${version}_x64_en-US.msi.sig`);
  if (mode === "release") names.push("latest.json");
  return names;
}

export function assetProblems(names, version, updater, mode) {
  const present = new Set(names);
  return expectedAssets(version, updater, mode)
    .filter((name) => !present.has(name))
    .map((name) => `missing asset ${name}`);
}

/**
 * The platform keys tauri-action v1.0.0 writes into latest.json for a universal macOS build and
 * the Windows installers (with `updaterJsonPreferNsis`), and the payload each must point at.
 */
export const PLATFORM_KEYS = {
  "darwin-aarch64": ".app.tar.gz",
  "darwin-aarch64-app": ".app.tar.gz",
  "darwin-x86_64": ".app.tar.gz",
  "darwin-x86_64-app": ".app.tar.gz",
  "darwin-universal": ".app.tar.gz",
  "darwin-universal-app": ".app.tar.gz",
  "windows-x86_64": "-setup.exe",
  "windows-x86_64-nsis": "-setup.exe",
  "windows-x86_64-msi": ".msi",
};

/** The asset a latest.json `url` names: GitHub's API asset URL ends in its id, a download URL (or a dry run's) in its name. */
function assetAt(url, assets) {
  const last = String(url ?? "").split("/").pop();
  return assets.find((asset) => String(asset.id) === last || asset.name === last);
}

/** What is wrong with `latest` for `version`, given the release's assets (`{ id, name }`). */
export function latestJsonProblems(latest, version, assets) {
  const problems = [];
  if (latest?.version !== version && latest?.version !== `v${version}`) problems.push(`latest.json announces ${latest?.version}, not ${version}`);
  for (const [key, suffix] of Object.entries(PLATFORM_KEYS)) {
    const entry = latest?.platforms?.[key];
    if (!entry) {
      problems.push(`latest.json has no entry for ${key}`);
      continue;
    }
    if (typeof entry.signature !== "string" || entry.signature.trim() === "") problems.push(`latest.json's ${key} has no signature`);
    const asset = assetAt(entry.url, assets);
    if (!asset) problems.push(`latest.json's ${key} points at ${entry.url}, which is no asset of this release`);
    else if (!asset.name.endsWith(suffix)) problems.push(`latest.json's ${key} points at ${asset.name}, not the ${suffix} payload`);
  }
  return problems;
}

/** The latest.json a release of these bundles would get, the way tauri-action writes it, with the bundle names as urls. */
export function wouldBeLatestJson(version, assets, signatureOf) {
  const platforms = {};
  for (const [key, suffix] of Object.entries(PLATFORM_KEYS)) {
    const asset = assets.find((candidate) => candidate.name.endsWith(suffix));
    if (asset) platforms[key] = { signature: signatureOf(asset.name), url: asset.name };
  }
  return { version, platforms };
}
```

命令行一节：`GATES` 里在 `panel` 之后加下面几道，`main` 开头在 `self-test` 那一行之后加 `workspace-version`，并加四个小工具：

```js
  tag: {
    options: { tag: { type: "string" }, manifest: { type: "string" } },
    run: (values) => tagProblems(required(values, "tag"), workspaceVersion(readFileSync(values.manifest ?? path.join(root, "Cargo.toml"), "utf8"))),
  },
  "on-main": {
    options: { commit: { type: "string" }, ref: { type: "string" } },
    run: (values) => onMainProblems(required(values, "commit"), values.ref ?? "origin/main"),
  },
  assets: {
    options: { version: { type: "string" }, updater: { type: "string" }, mode: { type: "string" }, names: { type: "string" } },
    run: (values) => assetProblems(lines(required(values, "names")), required(values, "version"), choice(values, "updater", ["true", "false"]) === "true", choice(values, "mode", ["release", "dry-run"])),
  },
  "latest-json": {
    options: { version: { type: "string" }, latest: { type: "string" }, assets: { type: "string" } },
    run: (values) => {
      const assets = lines(required(values, "assets")).map((line) => {
        const [id, name = ""] = line.split("\t");
        return { id, name };
      });
      return latestJsonProblems(JSON.parse(readFileSync(required(values, "latest"), "utf8")), required(values, "version"), assets);
    },
  },
  "latest-json-dry-run": {
    options: { version: { type: "string" }, dir: { type: "string" } },
    run: (values) => {
      const files = readdirSync(required(values, "dir"), { recursive: true, withFileTypes: true })
        .filter((entry) => entry.isFile())
        .map((entry) => path.join(entry.parentPath, entry.name));
      const assets = files.map((file) => ({ id: path.basename(file), name: path.basename(file) }));
      const signatureOf = (name) => {
        const sig = files.find((file) => path.basename(file) === `${name}.sig`);
        return sig === undefined ? "" : readFileSync(sig, "utf8").trim();
      };
      const version = required(values, "version");
      return latestJsonProblems(wouldBeLatestJson(version, assets, signatureOf), version, assets);
    },
  },
```

```js
  if (name === "workspace-version" && rest.length === 0) {
    console.log(workspaceVersion(readFileSync(path.join(root, "Cargo.toml"), "utf8")));
    return 0;
  }
```

```js
function required(values, name) {
  if (values[name] === undefined) throw new Error(`--${name} is required`);
  return values[name];
}

function choice(values, name, allowed) {
  const value = required(values, name);
  if (!allowed.includes(value)) throw new Error(`--${name} is one of ${allowed.join(", ")}`);
  return value;
}

/** The non-empty lines of a file. */
function lines(file) {
  return readFileSync(file, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
}
```

Run:

```bash
node scripts/release-checks.mjs self-test; echo "exit $?"
node scripts/release-checks.mjs workspace-version
node scripts/release-checks.mjs tag --tag "v$(node scripts/release-checks.mjs workspace-version)"; echo "exit $?"
node scripts/release-checks.mjs tag --tag v9.9.9; echo "exit $?"
git fetch --no-tags origin main && node scripts/release-checks.mjs on-main --commit "$(git rev-parse FETCH_HEAD)" --ref FETCH_HEAD; echo "exit $?"
node scripts/release-checks.mjs assets --version 0.1.0 --updater maybe --mode release --names /dev/null; echo "exit $?"
```

Expected，依次：`ok panel`、`ok tag`、`ok on-main`、`ok assets`、`ok latest-json`、`exit 0`；工作区版本（例如 `0.1.0`）；`ok tag`、`exit 0`；`FAILED tag: tag v9.9.9 does not name the workspace version …`、`exit 1`；`ok on-main`、`exit 0`；`release-checks: --updater is one of true, false`、`exit 2`。

- [ ] **Step 3: 写 `.github/workflows/bundle.yml` 与 `.github/workflows/release.yml`**

`.github/workflows/bundle.yml`（可复用；权限由调用它的任务给）：

```yaml
name: bundle

# Builds the installers for release.yml, on macOS (universal) and Windows (x64), one platform at a
# time: into the draft release when `release-id` is set (the calling job then has
# `contents: write`), as workflow artifacts otherwise (a dry run, called read-only). Optional
# secrets reach only the steps that use them, as step-level env, and are never written to
# $GITHUB_ENV; a dry run of a pull request is called without any.
on:
  workflow_call:
    inputs:
      version:
        type: string
        required: true
      updater:
        description: '"true" when the update-signing key is set and may be used'
        type: string
        required: true
      apple:
        type: string
        required: true
      windows:
        type: string
        required: true
      release-id:
        description: The draft release to upload to; empty in a dry run
        type: string
        required: false
        default: ""
    secrets:
      TAURI_SIGNING_PRIVATE_KEY:
        required: false
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD:
        required: false
      APPLE_CERTIFICATE:
        required: false
      APPLE_CERTIFICATE_PASSWORD:
        required: false
      APPLE_SIGNING_IDENTITY:
        required: false
      APPLE_ID:
        required: false
      APPLE_PASSWORD:
        required: false
      APPLE_TEAM_ID:
        required: false
      WINDOWS_CERTIFICATE:
        required: false
      WINDOWS_CERTIFICATE_PASSWORD:
        required: false

jobs:
  bundle:
    strategy:
      fail-fast: false
      # One at a time: tauri-action merges latest.json by downloading, editing and uploading it
      # again, and two jobs doing that together can lose one platform's entries.
      max-parallel: 1
      matrix:
        include:
          - name: macos-universal
            os: macos-latest
            target: universal-apple-darwin
          - name: windows-x64
            os: windows-latest
            target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.os }}
    # The heaviest job of a release: on macOS the app and the CLI are compiled for both
    # architectures, from a cold cache at worst, then signed (and notarized with a certificate).
    # 90 minutes is well above that and far below GitHub's default of six hours.
    timeout-minutes: 90
    env:
      VERSION: ${{ inputs.version }}
      UPDATER: ${{ inputs.updater }}
      RELEASE: ${{ inputs.release-id != '' }}
      # One --config at a time: externalBin only, or externalBin and the update payloads.
      CONFIG: ${{ inputs.updater == 'true' && 'src-tauri/tauri.updater.conf.json' || 'src-tauri/tauri.bundle.conf.json' }}
    steps:
      - uses: actions/checkout@v7
      - uses: pnpm/action-setup@ea17c68df8912ef543352723c149a84f56e3d413 # v6.1.0
        with:
          package_json_file: app/package.json
      - uses: actions/setup-node@v7
        with:
          node-version: 24
          cache: pnpm
          cache-dependency-path: app/pnpm-lock.yaml
      - name: Rust toolchain and targets
        shell: bash
        # The toolchain rust-toolchain.toml pins; `stable` would get the targets instead.
        run: |
          if [[ "$RUNNER_OS" == macOS ]]; then
            rustup toolchain install 1.99.0 --profile minimal --target aarch64-apple-darwin --target x86_64-apple-darwin
          else
            rustup toolchain install 1.99.0 --profile minimal --target x86_64-pc-windows-msvc
          fi
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          key: bundle-${{ matrix.target }}
      - run: pnpm --dir app install --frozen-lockfile
      - name: Stage the CLI (macOS, both architectures and the universal file)
        if: runner.os == 'macOS'
        run: bash scripts/stage-cli.sh --universal
      - name: Stage the CLI (Windows)
        if: runner.os == 'Windows'
        shell: pwsh
        run: ./scripts/stage-cli.ps1
      - name: Build the panel and check it is not build.rs's placeholder
        shell: bash
        run: |
          pnpm --dir app build
          node scripts/release-checks.mjs panel
      - name: Use the Windows signing certificate
        if: runner.os == 'Windows' && inputs.windows == 'true'
        shell: pwsh
        env:
          WINDOWS_CERTIFICATE: ${{ secrets.WINDOWS_CERTIFICATE }}
          WINDOWS_CERTIFICATE_PASSWORD: ${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}
        # Tauri merges tauri.windows.conf.json by itself when it builds on Windows; the file holds
        # the certificate's thumbprint, not a secret. The PFX is removed once imported.
        run: |
          $config = 'app/src-tauri/tauri.windows.conf.json'
          if (Test-Path $config) { throw "$config is in the repository; merge the signing settings into it instead" }
          $pfx = Join-Path $env:RUNNER_TEMP 'issuerelay-signing.pfx'
          [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
          $password = ConvertTo-SecureString $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
          $cert = Import-PfxCertificate -FilePath $pfx -CertStoreLocation Cert:\CurrentUser\My -Password $password
          Remove-Item $pfx
          @{ bundle = @{ windows = @{ certificateThumbprint = $cert.Thumbprint; digestAlgorithm = 'sha256'; timestampUrl = 'http://timestamp.digicert.com' } } } |
            ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 $config
      - name: Build the bundles
        id: tauri
        # Every build but a macOS one with an Apple certificate: macOS signs ad hoc ("-" in
        # tauri.conf.json), Windows signs with the thumbprint above when there is one.
        if: runner.os != 'macOS' || inputs.apple != 'true'
        uses: tauri-apps/tauri-action@1deb371b0cd8bd54025b384f1cd735e725c4060f # v1.0.0
        env:
          GITHUB_TOKEN: ${{ github.token }}
          # Only read when tauri.updater.conf.json switches the update payloads on.
          TAURI_SIGNING_PRIVATE_KEY: ${{ inputs.updater == 'true' && secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ inputs.updater == 'true' && secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
        with:
          projectPath: app
          args: --target ${{ matrix.target }} --config ${{ env.CONFIG }}
          # Empty in a dry run: then the action only builds.
          releaseId: ${{ inputs.release-id }}
          uploadUpdaterJson: ${{ inputs.updater == 'true' }}
          updaterJsonPreferNsis: true
          retryAttempts: 1
      - name: Build the bundles (Developer ID signed and notarized)
        id: tauri-apple
        if: runner.os == 'macOS' && inputs.apple == 'true'
        uses: tauri-apps/tauri-action@1deb371b0cd8bd54025b384f1cd735e725c4060f # v1.0.0
        env:
          GITHUB_TOKEN: ${{ github.token }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ inputs.updater == 'true' && secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ inputs.updater == 'true' && secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
          # APPLE_SIGNING_IDENTITY overrides the ad hoc "-"; the CLI imports the certificate into a
          # temporary keychain and notarizes with the Apple ID.
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
        with:
          projectPath: app
          args: --target ${{ matrix.target }} --config ${{ env.CONFIG }}
          releaseId: ${{ inputs.release-id }}
          uploadUpdaterJson: ${{ inputs.updater == 'true' }}
          updaterJsonPreferNsis: true
          retryAttempts: 1
      - name: The files the build produced
        id: built
        shell: bash
        env:
          PLAIN: ${{ steps.tauri.outputs.artifactPaths }}
          SIGNED: ${{ steps.tauri-apple.outputs.artifactPaths }}
        run: echo "paths=${PLAIN:-$SIGNED}" >> "$GITHUB_OUTPUT"
      - name: Check the macOS bundle
        if: runner.os == 'macOS'
        run: |
          app=target/universal-apple-darwin/release/bundle/macos/IssueRelay.app
          ls -l "$app/Contents/MacOS"
          for exe in issuerelay-app issuerelay; do
            archs=$(lipo -archs "$app/Contents/MacOS/$exe")
            echo "$exe: $archs"
            if [[ "$archs" != *x86_64* || "$archs" != *arm64* ]]; then
              echo "::error::$exe in the bundle is not universal ($archs)"
              exit 1
            fi
          done
          codesign --verify --deep --strict --verbose=2 "$app"
          printed=$("$app/Contents/MacOS/issuerelay" --version)
          if [[ "$printed" != "issuerelay $VERSION" ]]; then
            echo "::error::the bundled CLI says '$printed'"
            exit 1
          fi
      - name: Check the Windows installers
        if: runner.os == 'Windows'
        shell: pwsh
        run: |
          $bundle = 'target/x86_64-pc-windows-msvc/release/bundle'
          $setup = Get-ChildItem "$bundle/nsis/*-setup.exe" | Select-Object -First 1
          $listing = & 7z l $setup.FullName
          foreach ($name in 'issuerelay-app.exe', 'issuerelay.exe') {
            if (-not ($listing -match [regex]::Escape($name))) { throw "$name is not in $($setup.Name)" }
          }
          if (-not (Get-ChildItem "$bundle/msi/*.msi")) { throw 'no .msi was built' }
          $printed = & 'target/x86_64-pc-windows-msvc/release/issuerelay.exe' --version
          if ($printed -ne "issuerelay $env:VERSION") { throw "the CLI says '$printed'" }
      - name: Check the update signatures against the committed public key
        if: env.UPDATER == 'true'
        shell: bash
        env:
          ARTIFACTS: ${{ steps.built.outputs.paths }}
        run: node scripts/verify-update-signature.mjs --version "$VERSION" --artifacts "$ARTIFACTS"
      - name: List the bundles to keep
        if: env.RELEASE != 'true'
        id: bundles
        shell: bash
        env:
          ARTIFACTS: ${{ steps.built.outputs.paths }}
        # The .app folder is left out: a zipped folder loses its Unix permissions; the .dmg holds it.
        run: |
          {
            echo "paths<<ISSUERELAY_EOF"
            node -e 'for (const p of JSON.parse(process.env.ARTIFACTS)) if (!p.endsWith(".app")) console.log(p)'
            echo "ISSUERELAY_EOF"
          } >> "$GITHUB_OUTPUT"
      - name: Keep the bundles as workflow artifacts
        if: env.RELEASE != 'true'
        uses: actions/upload-artifact@v7
        with:
          name: bundles-${{ matrix.name }}
          path: ${{ steps.bundles.outputs.paths }}
          if-no-files-found: error
          retention-days: 7
```

`.github/workflows/release.yml`：

```yaml
name: release

# A `vX.Y.Z` tag on a commit of main builds the installers and puts them in ONE draft GitHub
# release; a maintainer publishes it by hand (MAINTAINERS.md, "发布"). A manual run and a pull
# request that changes the packaging are dry runs: the same gates (scripts/release-checks.mjs, for
# the tag that would be pushed and for the dry run's bundles), tests and bundles, kept as workflow
# artifacts, and no release. Only the jobs of a tag push can write to the repository; optional
# secrets are used only when they are set, and never in a pull request.
on:
  push:
    tags: ["v*"]
  pull_request:
    paths:
      - .github/workflows/release.yml
      - .github/workflows/bundle.yml
      - scripts/stage-cli.sh
      - scripts/stage-cli.ps1
      - scripts/verify-update-signature.mjs
      - scripts/release-checks.mjs
      - app/src-tauri/tauri.conf.json
      - app/src-tauri/tauri.bundle.conf.json
      - app/src-tauri/tauri.updater.conf.json
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false

jobs:
  verify:
    runs-on: ubuntu-latest
    # A checkout, two git commands, the secret scan and the gates' self-test: a minute or two.
    # The limit only stops a hung step long before GitHub's default of six hours.
    timeout-minutes: 10
    outputs:
      version: ${{ steps.version.outputs.version }}
      updater: ${{ steps.secrets.outputs.updater }}
      apple: ${{ steps.secrets.outputs.apple }}
      windows: ${{ steps.secrets.outputs.windows }}
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
      - uses: actions/setup-node@v7
        with:
          node-version: 24
      - name: The release gates pass their own tests
        run: node scripts/release-checks.mjs self-test
      - name: Read the workspace version
        id: version
        run: |
          version=$(node scripts/release-checks.mjs workspace-version)
          echo "version=$version" >> "$GITHUB_OUTPUT"
          echo "workspace version: $version"
      - name: The tag names the workspace version
        env:
          # A release checks its own tag; a dry run the tag that would be pushed (MAINTAINERS.md).
          TAG: ${{ github.event_name == 'push' && github.ref_name || format('v{0}', steps.version.outputs.version) }}
        run: node scripts/release-checks.mjs tag --tag "$TAG"
      - name: The tagged commit is on main
        env:
          IS_RELEASE: ${{ github.event_name == 'push' }}
        # A release checks the tagged commit; a dry run the commit a tag would be put on (origin/main).
        run: |
          git fetch --no-tags origin main
          if [[ "$IS_RELEASE" == true ]]; then commit=$GITHUB_SHA; else commit=$(git rev-parse FETCH_HEAD); fi
          node scripts/release-checks.mjs on-main --commit "$commit" --ref FETCH_HEAD
      - run: bash scripts/check-secrets.sh --strict
      - name: Which optional secrets are set
        id: secrets
        # Only whether each secret is empty is tested; no value is printed. Pull requests never use any.
        env:
          HAS_UPDATER_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY != '' }}
          HAS_APPLE: ${{ secrets.APPLE_CERTIFICATE != '' && secrets.APPLE_CERTIFICATE_PASSWORD != '' && secrets.APPLE_SIGNING_IDENTITY != '' && secrets.APPLE_ID != '' && secrets.APPLE_PASSWORD != '' && secrets.APPLE_TEAM_ID != '' }}
          HAS_WINDOWS: ${{ secrets.WINDOWS_CERTIFICATE != '' && secrets.WINDOWS_CERTIFICATE_PASSWORD != '' }}
          IS_PR: ${{ github.event_name == 'pull_request' }}
        run: |
          use() { if [[ "$IS_PR" != true && "$1" == true ]]; then echo true; else echo false; fi; }
          {
            echo "updater=$(use "$HAS_UPDATER_KEY")"
            echo "apple=$(use "$HAS_APPLE")"
            echo "windows=$(use "$HAS_WINDOWS")"
          } | tee -a "$GITHUB_OUTPUT"

  test:
    needs: verify
    # The six checks, as ci.yml's test job: 45 minutes there for the first uncached compile of the
    # Tauri app (Windows slowest); the same here.
    timeout-minutes: 45
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v7
      - uses: pnpm/action-setup@ea17c68df8912ef543352723c149a84f56e3d413 # v6.1.0
        with:
          package_json_file: app/package.json
      - uses: actions/setup-node@v7
        with:
          node-version: 24
          cache: pnpm
          cache-dependency-path: app/pnpm-lock.yaml
      - run: rustup toolchain install 1.99.0 --profile minimal --component rustfmt --component clippy
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
      - run: pnpm --dir app install --frozen-lockfile
      - run: pnpm --dir app lint
      - run: pnpm --dir app test
      # The Tauri crate embeds the built panel, as in ci.yml.
      - run: pnpm --dir app build
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: bash scripts/check-secrets.sh --strict
        shell: bash

  create-release:
    needs: [verify, test]
    if: github.event_name == 'push'
    runs-on: ubuntu-latest
    # Two API calls: seconds. The limit only stops a hung call.
    timeout-minutes: 10
    permissions:
      contents: write
    outputs:
      id: ${{ steps.create.outputs.id }}
    steps:
      - name: Create the draft release
        id: create
        env:
          GH_TOKEN: ${{ github.token }}
          TAG: ${{ github.ref_name }}
          VERSION: ${{ needs.verify.outputs.version }}
          UPDATER: ${{ needs.verify.outputs.updater }}
          APPLE: ${{ needs.verify.outputs.apple }}
          WINDOWS: ${{ needs.verify.outputs.windows }}
        run: |
          set -euo pipefail
          existing=$(gh api "repos/$GITHUB_REPOSITORY/releases" --paginate | jq -r --arg tag "$TAG" '.[] | select(.tag_name == $tag) | .id')
          if [[ -n "$existing" ]]; then
            echo "::error::a release for $TAG exists already (id $existing); delete that draft first, or release the next version"
            exit 1
          fi
          notes=$(mktemp)
          trap 'rm -f "$notes"' EXIT
          {
            echo "Install guide / 安装说明: https://github.com/$GITHUB_REPOSITORY/blob/$TAG/docs/install.md"
            echo
            echo "- macOS (Apple silicon and Intel, macOS 13.3 or later): \`IssueRelay_${VERSION}_universal.dmg\`"
            echo "- Windows: \`IssueRelay_${VERSION}_x64-setup.exe\` (recommended: installs for your user, no administrator rights) or \`IssueRelay_${VERSION}_x64_en-US.msi\` (installs for all users, needs administrator rights)"
            echo
            if [[ "$APPLE" == true ]]; then
              echo "- macOS: signed with a Developer ID and notarized by Apple."
            else
              echo "- macOS: not notarized by Apple (signed ad hoc). The first start needs one approval: System Settings → Privacy & Security → Open Anyway. / 未经 Apple 公证：第一次打开要在「系统设置 → 隐私与安全性」里点「仍要打开」。"
            fi
            if [[ "$WINDOWS" == true ]]; then
              echo "- Windows: the installers are code-signed."
            else
              echo "- Windows: the installers are not code-signed; SmartScreen asks once (More info → Run anyway). / 安装包未签名：SmartScreen 提示时点「更多信息 → 仍要运行」。"
            fi
            if [[ "$UPDATER" != true ]]; then
              echo "- This release has no automatic-update package: installed copies do not update to it by themselves; download and install it by hand. / 本版本不含自动更新包：已安装的程序不会自动更新到这个版本，请手动下载安装。"
            fi
          } > "$notes"
          id=$(gh api -X POST "repos/$GITHUB_REPOSITORY/releases" \
            -f tag_name="$TAG" -f name="IssueRelay $TAG" -F draft=true -F prerelease=false \
            -F body=@"$notes" --jq .id)
          echo "id=$id" >> "$GITHUB_OUTPUT"
          echo "draft release $id created for $TAG"

  bundle-release:
    needs: [verify, test, create-release]
    if: github.event_name == 'push'
    # Uploads into the draft. (A job that calls a reusable workflow cannot set timeout-minutes;
    # bundle.yml's job has the limit.)
    permissions:
      contents: write
    uses: ./.github/workflows/bundle.yml
    with:
      version: ${{ needs.verify.outputs.version }}
      updater: ${{ needs.verify.outputs.updater }}
      apple: ${{ needs.verify.outputs.apple }}
      windows: ${{ needs.verify.outputs.windows }}
      release-id: ${{ needs.create-release.outputs.id }}
    secrets:
      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
      APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
      APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
      APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
      APPLE_ID: ${{ secrets.APPLE_ID }}
      APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
      APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
      WINDOWS_CERTIFICATE: ${{ secrets.WINDOWS_CERTIFICATE }}
      WINDOWS_CERTIFICATE_PASSWORD: ${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}

  bundle-dry-run:
    needs: [verify, test]
    if: github.event_name != 'push'
    # Read-only: a pull request's own build scripts run here. A pull request gets no secret at all;
    # a manual run (on the default branch) gets them, so the update signatures are checked too.
    permissions:
      contents: read
    uses: ./.github/workflows/bundle.yml
    with:
      version: ${{ needs.verify.outputs.version }}
      updater: ${{ needs.verify.outputs.updater }}
      apple: ${{ needs.verify.outputs.apple }}
      windows: ${{ needs.verify.outputs.windows }}
    secrets:
      TAURI_SIGNING_PRIVATE_KEY: ${{ github.event_name != 'pull_request' && secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ github.event_name != 'pull_request' && secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
      APPLE_CERTIFICATE: ${{ github.event_name != 'pull_request' && secrets.APPLE_CERTIFICATE || '' }}
      APPLE_CERTIFICATE_PASSWORD: ${{ github.event_name != 'pull_request' && secrets.APPLE_CERTIFICATE_PASSWORD || '' }}
      APPLE_SIGNING_IDENTITY: ${{ github.event_name != 'pull_request' && secrets.APPLE_SIGNING_IDENTITY || '' }}
      APPLE_ID: ${{ github.event_name != 'pull_request' && secrets.APPLE_ID || '' }}
      APPLE_PASSWORD: ${{ github.event_name != 'pull_request' && secrets.APPLE_PASSWORD || '' }}
      APPLE_TEAM_ID: ${{ github.event_name != 'pull_request' && secrets.APPLE_TEAM_ID || '' }}
      WINDOWS_CERTIFICATE: ${{ github.event_name != 'pull_request' && secrets.WINDOWS_CERTIFICATE || '' }}
      WINDOWS_CERTIFICATE_PASSWORD: ${{ github.event_name != 'pull_request' && secrets.WINDOWS_CERTIFICATE_PASSWORD || '' }}

  verify-release:
    needs: [verify, create-release, bundle-release]
    if: ${{ always() && needs.create-release.result == 'success' }}
    runs-on: ubuntu-latest
    # Two API calls, one small download and the gates: well under a minute.
    timeout-minutes: 10
    permissions:
      contents: write
    env:
      GH_TOKEN: ${{ github.token }}
      RELEASE_ID: ${{ needs.create-release.outputs.id }}
      VERSION: ${{ needs.verify.outputs.version }}
      UPDATER: ${{ needs.verify.outputs.updater }}
    steps:
      - uses: actions/checkout@v7
      - uses: actions/setup-node@v7
        with:
          node-version: 24
      - name: The draft has every asset and latest.json every platform
        run: |
          set -euo pipefail
          work=$(mktemp -d)
          trap 'rm -rf "$work"' EXIT
          draft=$(gh api "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID" --jq .draft)
          if [[ "$draft" != true ]]; then
            echo "::error::release $RELEASE_ID is no longer a draft"
            exit 1
          fi
          gh api "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID/assets" --paginate --jq '.[] | "\(.id)\t\(.name)"' > "$work/assets.tsv"
          cut -f2 "$work/assets.tsv" | tee "$work/names.txt"
          failed=0
          node scripts/release-checks.mjs assets --version "$VERSION" --updater "$UPDATER" --mode release --names "$work/names.txt" || failed=1
          if [[ "$UPDATER" == true ]]; then
            id=$(awk -F '\t' '$2 == "latest.json" { print $1 }' "$work/assets.tsv")
            if [[ -n "$id" ]]; then
              gh api "repos/$GITHUB_REPOSITORY/releases/assets/$id" -H "Accept: application/octet-stream" > "$work/latest.json"
              node scripts/release-checks.mjs latest-json --version "$VERSION" --latest "$work/latest.json" --assets "$work/assets.tsv" || failed=1
            fi
          fi
          exit "$failed"
      - name: Mark an incomplete draft
        if: failure()
        run: |
          gh api -X PATCH "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID" \
            -f name="IssueRelay $GITHUB_REF_NAME (incomplete: do not publish)" > /dev/null
          echo "::error::the draft is incomplete and was renamed; see MAINTAINERS.md, 发布"

  verify-dry-run:
    needs: [verify, bundle-dry-run]
    if: github.event_name != 'push'
    runs-on: ubuntu-latest
    # Downloads the dry run's bundles (a few hundred megabytes) and runs the gates: minutes.
    timeout-minutes: 15
    permissions:
      contents: read
    env:
      VERSION: ${{ needs.verify.outputs.version }}
      UPDATER: ${{ needs.verify.outputs.updater }}
    steps:
      - uses: actions/checkout@v7
      - uses: actions/setup-node@v7
        with:
          node-version: 24
      - uses: actions/download-artifact@v8
        with:
          pattern: bundles-*
          path: dry-run
      - name: The bundles a release of this version would have, and its latest.json
        run: |
          set -euo pipefail
          names=$(mktemp)
          trap 'rm -f "$names"' EXIT
          find dry-run -type f -exec basename {} \; | sort | tee "$names"
          failed=0
          node scripts/release-checks.mjs assets --version "$VERSION" --updater "$UPDATER" --mode dry-run --names "$names" || failed=1
          if [[ "$UPDATER" == true ]]; then
            node scripts/release-checks.mjs latest-json-dry-run --version "$VERSION" --dir dry-run || failed=1
          fi
          exit "$failed"
```

（几处要在第一次运行时看实际行为、与这里不同就照实改并写进报告：`releaseId` 为空字符串时 tauri-action 是否只构建；`--config` 的相对路径是否按 `projectPath`（`app/`）解析；`createUpdaterArtifacts: false` 时空的 `TAURI_SIGNING_PRIVATE_KEY` 是否被忽略（不被忽略就把不带私钥的构建再分出一个步骤）；可复用工作流收到空字符串的 secret 时是否当作没有；`7z l` 能否列出 NSIS 安装包里的文件——不能时改为检查 `target/x86_64-pc-windows-msvc/release/` 下的两个 exe，并在报告里说明；`gh api … -H "Accept: application/octet-stream"` 能否跟随重定向取到草稿里的 `latest.json`——不能时改用 `curl -fsSL -H "Authorization: Bearer $GH_TOKEN" -H "Accept: application/octet-stream"`；`expectedAssets` 与 `PLATFORM_KEYS` 写的是调研读到的 tauri-action v1.0.0 与 tauri-bundler 2.10.2 的名字，与第一次试打包或第一个草稿不同就照实改脚本与它的自测。）

- [ ] **Step 4: 静态检查与固定的 action 版本**

```bash
ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }; puts "yaml ok"' .github/workflows/release.yml .github/workflows/bundle.yml
git ls-remote https://github.com/tauri-apps/tauri-action 'refs/tags/v1.0.0' 'refs/tags/v1.0.0^{}'
git ls-remote https://github.com/pnpm/action-setup 'refs/tags/v6.1.0' 'refs/tags/v6.1.0^{}'
git ls-remote https://github.com/Swatinem/rust-cache 'refs/tags/v2.9.2' 'refs/tags/v2.9.2^{}'
git ls-remote https://github.com/rhysd/actionlint 'refs/tags/v*' | tail -3
```

Expected：`yaml ok`；三个仓库各返回标签（附注标签还会多一行 `^{}`），指向提交的那一行分别是 `1deb371b0cd8bd54025b384f1cd735e725c4060f`、`ea17c68df8912ef543352723c149a84f56e3d413`、`6323deb102c322ba6fcbdcafc7e3dddab59af2b6`；不一致时用 `ls-remote` 给出的提交替换 `release.yml` / `bundle.yml` 里的 SHA，并在报告里写明。最后一行列出 actionlint 最新的几个发布，Step 5 写最新那个的版本号。本机有 Docker 时再跑一次 `docker run --rm -v "$PWD:/repo" --workdir /repo rhysd/actionlint:<版本> -color`（无输出即通过）；没有 Docker 就以推送后 CI 的 `secrets` 任务为准，不为此安装工具。

- [ ] **Step 5: `ci.yml` 用 actionlint 检查工作流**

M2 的 `ci.yml` 已经用 `pnpm/action-setup@v6` 与 Node 24（`packageManager` 是 pnpm 10），不用改它的 pnpm 与 Node 两步。在 `secrets` 任务（ubuntu，必需检查之一，名字不变）的 `check-secrets` 之后加一步：

```yaml
      # actionlint checks every workflow: expressions, job outputs and needs, reusable workflow
      # inputs, and the shell in `run:` through shellcheck. The author's container image, pinned
      # to a release (Step 4 of the M3 plan: the newest at the time).
      - name: Lint the workflows
        uses: docker://rhysd/actionlint:1.7.7
        with:
          args: -color
```

（`1.7.7` 换成 Step 4 查到的最新发布；`docker://` 只能在 Linux 运行器上用，`secrets` 任务正是 ubuntu。actionlint 报的问题——包括 shellcheck 对 `run:` 脚本的意见——照改，不加忽略。）

Run: 推送前本机能跑就跑 Step 4 的 Docker 命令；推送后看 `ci` 的 `secrets` 任务。
Expected: `secrets` 任务通过，两条命令都没有输出问题。

- [ ] **Step 6: 文档（事实登记）与代码提交**

- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 6 节加：`.github/workflows/release.yml`：版本标签触发的打包与草稿 Release，手动运行与改了打包配置的 PR 只试打包；`.github/workflows/bundle.yml`：两种打包共用的可复用工作流（权限由调用的任务给）；`scripts/verify-update-signature.mjs`：用提交的公钥核对更新包签名；`scripts/release-checks.mjs` 一条补上「标签与版本、`origin/main`、资产与 `latest.json` 的门禁」（用法见文件头）。
- 设计第 12 节「GitHub Actions」一条改为：「GitHub Actions（`release.yml`）：推送版本标签时先跑门禁（`scripts/release-checks.mjs`：标签与版本、标签提交在 `main` 上）并扫描密钥，在 macOS 和 Windows 上跑全部验证命令，建一个草稿 Release，逐个平台打包上传，最后核对资产与 `latest.json` 齐全；由维护者确认后公开。手动运行与改了打包配置的 PR 只试打包（只读，拉取请求拿不到 secrets），对将要打的标签与试打包产物跑同样的门禁。」第 18 节第 2 条后补：「现状（M3）：没有证书时 macOS 包做 ad-hoc 签名（不是完全不签：未签名的 arm64 包下载后会被报『已损坏』），Windows 不签名；secrets 里放齐 Apple 或 Windows 证书后，发布工作流自动改用正式签名（macOS 同时公证）。」
- `.github/CODEOWNERS` 加四行（发布流水线；`/.github/` 已经覆盖两个工作流）：

  ```text
  /scripts/stage-cli.sh                 @sunkailun1992
  /scripts/stage-cli.ps1                @sunkailun1992
  /scripts/verify-update-signature.mjs  @sunkailun1992
  /scripts/release-checks.mjs           @sunkailun1992
  ```

Run: `pnpm --dir app build`、六条验证命令与 `node scripts/release-checks.mjs self-test`
Expected: 全部通过。

```bash
git status --short
git add .github/workflows/release.yml .github/workflows/bundle.yml .github/workflows/ci.yml .github/CODEOWNERS \
  scripts/verify-update-signature.mjs scripts/release-checks.mjs \
  docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md docs/superpowers/specs/2026-10-10-issuerelay-design.md
git diff --cached --stat
git commit -F - <<'EOF'
ci: build installers and draft a GitHub release on version tags

release.yml: the gates (scripts/release-checks.mjs, self-tested, also run by dry runs for the
tag that would be pushed and the dry run's bundles), the six checks, one draft release, the
bundles of the reusable bundle.yml one platform at a time, then the draft's assets and
latest.json. Only a tag push can write; dry runs are read-only and pull requests get no
secrets, which reach only the steps that use them. Every job has a time limit, and ci.yml
lints the workflows with actionlint.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

- [ ] **Step 7: 发布规则（单独的 `docs` 提交）**

`MAINTAINERS.md`：英文摘要里「Releases start at M3…」一条改为「Releases start at M3: a maintainer bumps the version in a PR, then pushes a `vX.Y.Z` tag on main; the workflow builds the installers into one draft release, and a maintainer publishes it by hand. Contributors never bump the version. The update-signing key lives only on the maintainer's machine and in GitHub Actions secrets.」「分支保护」表里「必需检查」一行的 `secrets` 后面补「（含 actionlint 检查工作流）」。「## 发布」一节整节换成：

~~~markdown
## 发布

M3 起由维护者发布；外部贡献者的 PR 不改版本号。工作流 `.github/workflows/release.yml`（打包在 `bundle.yml`），门禁 `scripts/release-checks.mjs`，安装说明 `docs/install.md`，更新签名密钥 `docs/release/updater-key.md`。

1. **升版本号**：开一个普通的 PR，把工作区 `Cargo.toml` 的 `[workspace.package] version` 改成新版本，再运行 `cargo update -w`（只更新本仓库 crate 在 `Cargo.lock` 里的版本），两个文件一起提交；0.x 阶段升次版本号时，`crates/cli`、`crates/devhost`、`app/src-tauri` 对 `issuerelay-core` 的版本要求一起改（`AI_CODING_GUIDE.md` 第 10 节）。只用 `X.Y.Z`（MSI 不接受 `-rc.1` 这类非纯数字的预发布后缀），推送前按 `origin/main` 的最新值顺延；CI 全绿后合入 `main`。不在本地临时改版本号再改回来。
2. **推标签**（本地会话）：

   ```bash
   git fetch origin
   version=$(git show origin/main:Cargo.toml | sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\(.*\)"$/\1/p')
   git tag -a "v$version" origin/main -m "IssueRelay v$version"
   git push origin "v$version"
   ```

3. **等工作流全绿**：`verify`（门禁自测、标签 = 版本、标签提交在 `origin/main` 上、密钥扫描）→ `test`（macOS、Windows 六条验证命令）→ `create-release`（建一个草稿）→ `bundle-release`（先 macOS 通用包，后 Windows）→ `verify-release`（资产齐全、`latest.json` 每个平台都有签名、地址指向本 Release 的资产）。`verify-release` 失败时，草稿标题会被改成「incomplete: do not publish」。这些门禁在每次试打包里已经对将要打的标签跑过。
4. **检查草稿**：资产有 `IssueRelay_<版本>_universal.dmg`、`_x64-setup.exe`、`_x64_en-US.msi`；有更新签名私钥时还有 `_universal.app.tar.gz` 与三个 `.sig`、`latest.json`。发布说明里关于签名与自动更新的说明符合实际。至少在一台 Mac 上从草稿下载 dmg 装一遍。
5. **公开**：在 GitHub 的草稿页点「Publish release」。这是对外动作，只由维护者亲手点，工作流和 AI 都不公开。公开后 `…/releases/latest/download/latest.json` 才指向新版本，已安装的程序在下次检查时看到更新。
6. **失败或放弃**：没公开的草稿可以删掉（Releases 页面删除草稿，再 `git push origin --delete "v$version"` 删标签），修好后重推同一个标签；已经公开的版本号永不重用，修复用下一个版本号。

### 发布用的 secrets

| Secret | 是否必需 | 作用 | 没有时 |
|---|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 自动更新需要 | 更新签名私钥（`docs/release/updater-key.md`） | 不生成更新包与 `latest.json`，发布说明写明「不含自动更新包」 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 否 | 私钥密码；现在的密钥没有密码，不建这个 secret | — |
| `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` | 否（六个齐全才用） | Developer ID 签名与公证 | ad-hoc 签名、不公证：用户第一次打开要在「隐私与安全性」里放行一次 |
| `WINDOWS_CERTIFICATE`、`WINDOWS_CERTIFICATE_PASSWORD` | 否（两个齐全才用） | Windows 代码签名（PFX 文件的 base64 与密码） | 不签名：SmartScreen 提示一次 |

secrets 只由管理员在仓库设置里改，只经步骤级 `env:` 交给用到它们的步骤；拉取请求触发的试打包拿不到它们，试打包只有只读权限。
~~~

- `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 7.1 节改为：「可以做实机验收、打版本标签、发布安装包。发布只从已推送到 `origin/main` 的提交构建：发布工作流核对标签提交在 `origin/main` 上、标签等于工作区版本，只建草稿 Release；公开由维护者在 GitHub 页面上点。升版本号是合入 `main` 的普通 PR，不在本地临时改了再改回。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 10 节最后一条改为：「M3 起推送 `vX.Y.Z` 标签触发发布（`.github/workflows/release.yml`）：标签必须等于工作区版本、标签提交必须在 `origin/main` 上（`scripts/release-checks.mjs`，试打包同样检查），工作流只建草稿，步骤见 `MAINTAINERS.md`「发布」。」

```bash
git status --short
git add MAINTAINERS.md docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md docs/ai-coding/AI_CODING_GUIDE.md
git diff --cached --stat
git commit -F - <<'EOF'
docs: describe how a maintainer releases from a version tag

The version bump is an ordinary PR; the tag goes on origin/main; the workflow only drafts
the release and a maintainer publishes it. Release secrets reach only the steps that use
them, and dry runs are read-only.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

- [ ] **Step 8: 试打包（M3 的 PR）**

推送分支并开草稿 PR（已开过就只推送）；改了打包配置，`release` 工作流随 PR 运行。先同步（AI_ENGINEERING_GUARDRAILS 第 7.3 节）：`main` 前进了而本分支还没推送过，就 `git rebase origin/main` 并重跑六条验证命令；已经推送过就 `git merge origin/main`，不 rebase 已推送的提交。

```bash
git fetch origin
git status -sb
git push -u origin feat/m3-release
mktemp -d   # first time only: prints a folder for the PR description
```

第一次开 PR 时，把按 `.github/pull_request_template.md` 填好的描述用写文件的工具写进上面打印的目录下的 `pr.md`，再运行（`<目录>` 换成那个目录；变量不跨命令保留，所以直接写路径），之后删掉那个目录：

```bash
gh pr create --draft --title "feat: installers, CLI on PATH, signed auto-update and the release workflow (M3)" --body-file "<目录>/pr.md"
rm -rf "<目录>"
run=$(gh run list --workflow release.yml --branch feat/m3-release --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" --exit-status
```

（刚推送时运行可能还没出现在列表里：`run` 为空或是上一次的编号时，几秒后再运行一次 `gh run list` 那一行。）

Expected：`verify`、两个 `test`、`bundle-dry-run` 的两个打包任务、`verify-dry-run` 成功，`create-release`、`bundle-release`、`verify-release` 跳过；`verify` 里门禁自测五行 `ok`，`tag` 与 `on-main` 两道门通过（将要打的标签、`origin/main`），输出 `updater=false`、`apple=false`、`windows=false`（拉取请求从不使用 secrets）；两个包内容检查与面板检查通过；`verify-dry-run` 的资产门通过（没有更新包时只查三个安装包）。下载产物核对（临时目录用完即删，Task 6 自己再下载）：

```bash
work=$(mktemp -d)
run=$(gh run list --workflow release.yml --branch feat/m3-release --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" -D "$work"
find "$work" -type f | sed "s|$work/||" | sort
dmg=$(find "$work" -name 'IssueRelay_*_universal.dmg' | head -1)
mkdir "$work/mnt" && hdiutil attach -nobrowse -readonly -mountpoint "$work/mnt" "$dmg" > /dev/null
lipo -archs "$work/mnt/IssueRelay.app/Contents/MacOS/issuerelay" "$work/mnt/IssueRelay.app/Contents/MacOS/issuerelay-app"
codesign --verify --deep --strict "$work/mnt/IssueRelay.app" && echo "signature ok"
hdiutil detach "$work/mnt" > /dev/null
rm -rf "$work"
```

Expected：`bundles-macos-universal/…/IssueRelay_<版本>_universal.dmg`、`bundles-windows-x64/…/IssueRelay_<版本>_x64-setup.exe`、`…/IssueRelay_<版本>_x64_en-US.msi`（`<版本>` 是工作区版本；没有 `.sig`：试打包不签更新包）；两个 `lipo` 都是 `x86_64 arm64`；`signature ok`。任何一处与预期不同（资产名、目录结构、tauri-action 的行为），修好、推送、重跑，并在报告里写明差异与改法（资产名不同就同时改 `release-checks.mjs` 的 `expectedAssets` 与它的自测）。

---

### Task 5: 安装说明

设计第 12 节（README 写清未签名包的放行步骤）、第 17 节（同事按说明用安装包装好）。调研结论：macOS 15 起，按住 Control 点按再选「打开」不再能绕过 Gatekeeper，只能在「系统设置 → 隐私与安全性」点「仍要打开」（被拦后大约一小时内可点，要输入登录密码）；macOS 14 及更早仍可用 Control 点按 → 打开。Windows 未签名的安装包会出现「Windows 已保护你的电脑」，点「更多信息」才有「仍要运行」；每个新版本都从零信誉开始；开启了「智能应用控制」（Windows 11 的强制模式）的电脑可能直接拦截未签名程序，没有按应用放行的办法。MSI 一律为所有用户安装、需要管理员；NSIS 默认按用户安装到 `%LOCALAPPDATA%\IssueRelay`。弹出磁盘映像或从「下载」直接运行的应用可能跑在 Gatekeeper 的只读临时副本里。

**Files:**
- Create: `docs/install.md`
- Modify: `README.md`（中英文「安装 / Install」一节，放在「它怎么工作」之前）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 12 节「代码签名」一条）

**Interfaces:**
- Consumes：Task 1 的资产名与安装位置；Task 2 的设置项「把命令行加入 PATH」与 `issuerelay start`；Task 3 的「自动检查更新」「检查更新」「安装并重启」；Task 4 的 Releases 草稿与发布说明；M2 的首次引导、环境检查、开机自启与数据目录（设计第 11 节）。
- Produces：`docs/install.md`（中文；各节标题：下载哪个文件、macOS 安装、macOS 第一次打开、Windows 安装、首次引导、把命令行加入 PATH、自动更新、卸载、常见问题），README 的链接；Task 6 按它安装。

- [ ] **Step 1: 写 `docs/install.md`**

~~~markdown
# 安装 IssueRelay

IssueRelay 是每个开发者自己装一份的托盘程序（macOS、Windows）。安装包在 [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases/latest)。

## 下载哪个文件

| 电脑 | 文件 | 说明 |
|---|---|---|
| Mac（Apple 芯片或 Intel） | `IssueRelay_<版本>_universal.dmg` | 一个包两种芯片通用 |
| Windows（推荐） | `IssueRelay_<版本>_x64-setup.exe` | 只装给你自己，不需要管理员权限 |
| Windows（为所有用户安装） | `IssueRelay_<版本>_x64_en-US.msi` | 需要管理员权限；之后的更新也会要求管理员确认 |

只从上面这个 Releases 页面下载。系统要求：macOS 13.3 及以后（Apple 芯片或 Intel）；Windows 10 或 11（x64）。每个版本的发布说明写明这一版是否经过 Apple 公证、Windows 安装包是否签名，以及是否带自动更新包。

## macOS 安装

1. 双击 `.dmg`，把 IssueRelay 拖进「应用程序」。没有管理员权限时，拖进你自己的「应用程序」文件夹（在访达里前往 `~/Applications`，没有就新建）。
2. 推出磁盘映像。不要直接在磁盘映像里或「下载」文件夹里运行 IssueRelay：那样「把命令行加入 PATH」和自动更新都用不了。

## macOS 第一次打开

没有经过 Apple 公证的版本（发布说明里写着「not notarized」），第一次打开会被 macOS 拦下来，只需放行一次；经过公证的版本没有这一步，直接打开即可：

- **macOS 15 及以后**：双击 IssueRelay，看到提示后点「完成」；打开「系统设置 → 隐私与安全性」，在「安全性」下面点 IssueRelay 旁边的「仍要打开」（被拦后大约一小时内可以点），输入登录密码，再确认一次。之后就能正常打开。
- **macOS 14 及更早**：按住 Control 点按 IssueRelay，选「打开」，再点「打开」。
- **终端里的办法**（只对从上面的 Releases 下载的包这么做）：`xattr -dr com.apple.quarantine /Applications/IssueRelay.app`（装在个人「应用程序」里就把路径换成 `~/Applications/IssueRelay.app`）。它去掉下载标记，macOS 就不再为这个应用做首次检查。
- 如果提示「已损坏，无法打开」：下载的文件不完整或不是从 Releases 下载的，删掉后重新下载。

## Windows 安装

1. 双击 `-setup.exe`。没有代码签名的安装包（发布说明里写着「not code-signed」）会出现「Windows 已保护你的电脑」：点「更多信息」，再点「仍要运行」。签过名的安装包通常不会这样，但新证书在积累信誉之前也可能出现同样的提示。
2. 按提示安装；默认装到 `%LOCALAPPDATA%\IssueRelay`，不需要管理员权限。
3. 若安装包或程序被直接拦截、没有「仍要运行」：打开「Windows 安全中心 → 应用和浏览器控制 → 智能应用控制」查看是否开启。公司管理的电脑请联系管理员。

## 首次引导

第一次打开时 IssueRelay 会带你走一遍：连接 Linear、检查 Claude Code 与 Codex、绑定项目目录。之后随时可以在面板的「环境检查」里重新检查，或从那里重新走一遍引导。

## 把命令行加入 PATH

想在终端里直接输入 `issuerelay`：打开面板「设置 → 系统」，打开「把命令行加入 PATH」。不需要管理员权限。

- **macOS**：IssueRelay 在 `~/.local/bin/issuerelay` 建一个指向应用里命令行程序的链接。如果面板提示 `~/.local/bin` 还不在 PATH 里，把面板给出的那一行加到 shell 配置文件（zsh 是 `~/.zshrc`），再新开一个终端。那里已有别的同名文件时，IssueRelay 不会覆盖它，面板会告诉你。
- **Windows**：IssueRelay 把安装目录加进你自己的 PATH。新开的终端才看得到；已经开着的终端要重开。
- 托盘程序没在运行时，`issuerelay status` 会提示；运行 `issuerelay start` 即可启动托盘程序。
- 在 Claude Code、Codex 里注册 IssueRelay 的 MCP 控制服务（「设置 → MCP 控制服务」）用的是命令行程序的完整路径，不依赖 PATH。托盘程序没运行时，这些工具只会报错，不会替你启动它。

## 自动更新

- 「设置 → 系统 → 自动检查更新」打开时（默认打开），IssueRelay 启动 1 分钟后检查一次，之后每 6 小时一次；也可以在那里点「检查更新」。
- 有新版本时会收到一条通知，面板顶部出现横幅。点「安装并重启」：IssueRelay 先下载并校验签名；有任务在运行时，会像退出时一样问你「等待完成」还是「立即停止」；之后安装并重启。校验不通过的更新不会安装。
- Windows 上安装更新前，请先关掉正在运行 `issuerelay` 的终端窗口或 AI 工具，否则安装程序可能无法替换它。
- 某个版本的发布说明写着「不含自动更新包」时，已安装的程序不会自动更新到它，需要手动下载安装。

## 卸载

1. 先在面板里关掉「把命令行加入 PATH」和「开机自启」。
2. **macOS**：退出 IssueRelay，把它从「应用程序」拖到废纸篓。想连数据一起删：删除 `~/Library/Application Support/com.sunkailun.issuerelay`（设置、绑定、运行记录与日志），并在「钥匙串访问」里搜索 `com.sunkailun.issuerelay`，删除 Linear 的登录凭据。
3. **Windows**：在「设置 → 应用 → 已安装的应用」里卸载 IssueRelay。想连数据一起删：删除 `%APPDATA%\com.sunkailun.issuerelay` 和 `%LOCALAPPDATA%\com.sunkailun.issuerelay`，并在「凭据管理器 → Windows 凭据」里删除 `com.sunkailun.issuerelay` 的条目。
4. 忘了第 1 步时：macOS 手动删 `~/.local/bin/issuerelay`；Windows 在「编辑账户的环境变量」里从 `Path` 删掉 IssueRelay 的安装目录。

## 常见问题

- **找不到 claude 或 codex**：打开「环境检查」，看 PATH 来源是不是「登录 shell」。IssueRelay 从你的登录 shell 读取 PATH；在终端里能运行、在 IssueRelay 里找不到时，把它们的目录加到 shell 配置文件里，重启 IssueRelay。
- **需要代理才能访问 Linear 或 GitHub**：IssueRelay 依次读取它自己的环境、登录 shell 和系统设置里的代理；「环境检查」里能看到用的是哪个（密码已打码）。自动更新同样经这个网络访问 `github.com` 与 `api.github.com`。
- **Linear 授权**：在「环境检查」或「设置 → 高级」里重新授权，或改用个人 API key。
- **更新失败**：面板会说明原因（连不上更新服务器、签名不对、还没有适合这台电脑的安装包等），什么都不会被安装，监控照常进行。稍后再试，或从 Releases 手动下载。
~~~

- [ ] **Step 2: README 的「安装」一节**

`README.md` 在「它怎么工作」之前加：

```markdown
## 安装 / Install

从 [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases/latest) 下载：Mac 用 `IssueRelay_<版本>_universal.dmg`，Windows 用 `IssueRelay_<版本>_x64-setup.exe`（不需要管理员权限）。需要 macOS 13.3 及以后。没有经过 Apple 公证或 Windows 代码签名的版本（每个版本的发布说明写明），第一次打开要放行一次；经过公证、签名的版本没有这一步。步骤、把命令行加入 PATH、自动更新与卸载见 [安装说明](docs/install.md)。

Download from [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases/latest): `IssueRelay_<version>_universal.dmg` for macOS, `IssueRelay_<version>_x64-setup.exe` for Windows (no administrator rights needed). macOS 13.3 or later. Unsigned builds (each release's notes say whether it is notarized by Apple and code-signed for Windows) need one approval at the first start (macOS 15+: System Settings → Privacy & Security → Open Anyway; Windows: More info → Run anyway); signed builds, when available, do not. See [docs/install.md](docs/install.md) (Chinese) for the steps, putting the CLI on PATH, automatic updates and uninstalling.
```

设计第 12 节「代码签名」一条改为：「代码签名：见第 18 节。未公证 / 未签名时，`docs/install.md` 与 README 写清放行步骤：macOS 15 起在『系统设置 → 隐私与安全性』点『仍要打开』（14 及更早：Control 点按 → 打开），Windows SmartScreen『更多信息 → 仍要运行』。」

- [ ] **Step 3: 只读审阅**

派一个子智能体只读审阅 `docs/install.md` 与 README 新加的一节（不执行安装、不改文件），角度：一位没用过 IssueRelay 的同事在一台干净的 macOS 用户环境里，只看这份说明能否装好、放行、完成首次引导、把命令行加入 PATH、更新与卸载；指出含糊、顺序不对、与 Task 1–4 的实际行为不符（资产名、设置项名、按钮名）的地方。主智能体按意见修订。

- [ ] **Step 4: 提交**

Run: 六条验证命令
Expected: 全部通过（`check-secrets` 不会命中：文档里的路径都以 `~`、`%LOCALAPPDATA%` 或 `/Applications` 开头）。

```bash
git status --short
git add docs/install.md README.md docs/superpowers/specs/2026-10-10-issuerelay-design.md
git diff --cached --stat
git commit -F - <<'EOF'
docs: add installation instructions

docs/install.md and the README's install section; the signing state is worded per release
(unsigned builds need one approval, signed builds do not), as each release's notes state it.

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
```

---

### Task 6: M3 实机验收与第一个草稿 Release

由主智能体在本机执行（实机验收只由主智能体做；AGENTS.md）。把调研里「未确认」的几项变成明确的检查：macOS 15 / 26 上 Gatekeeper 的提示与放行流程；ad-hoc 签名的版本更新后会不会再弹钥匙串授权；Windows 上正在运行的 `issuerelay.exe` 会不会挡住 NSIS 更新或卸载（待实机）；`request_restart()` 与单实例插件配合后，更新完程序是否确实在运行。报告写 `docs/gates/<日期>-m3-acceptance.md`（按 `TESTING_SPEC.md` 的模板，真实数据一律脱敏，截图不进仓库）。

**需要用户的地方：** 「仍要打开」要输入登录密码；钥匙串授权框；在浏览器里重新授权 Linear；上传更新签名私钥（`gh secret set`）；合并 PR；公开草稿 Release。这些涉及用户的凭据或对外动作，主智能体只在那一步请用户操作，其余自行完成。

**Files:**
- Create: `docs/gates/<日期>-m3-acceptance.md`
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 17 节 M3 一行的完成情况；按实测更新第 12 节里与实际不符的说法）、`docs/install.md`（按实测改提示文字）

**Interfaces:**
- Consumes：Task 4 Step 8 的试打包运行（本任务按运行编号重新下载产物，不靠别的任务留下的 shell 变量或临时文件）；Task 1 的 `scripts/stage-cli.sh`、`tauri.bundle.conf.json`；Task 3 的 `~/.tauri/issuerelay.key`（只以路径交给构建，从不读出）；Task 5 的 `docs/install.md`；M2 Task 13 的沙箱项目与沙箱仓库。
- Produces：验收报告；`v<工作区版本>` 标签与一个通过 `verify-release` 的草稿 Release（由维护者公开）。

- [ ] **Step 1: 前提**

Task 4 Step 8 的试打包全绿；`gh secret list --repo sunkailun1992/issuerelay` 里有 `TAURI_SIGNING_PRIVATE_KEY`（只看名字）。没有时请维护者运行 Task 3 Step 0 的 `gh secret set` 命令；维护者决定先不上传时，第 8 步只核对打包（`updater=false`，没有签名可核对），第 9–10 步照常做，草稿会是「不含自动更新包」的版本，报告里写明。

- [ ] **Step 2: 按说明安装（macOS，不需要管理员）**

本任务的临时文件都放在一个 `mktemp -d` 建的目录里（Step 6 结束时删掉）。shell 变量不跨步骤保留，所以后面每一步开头都用同一段命令按名字重新找到它。先建目录，再按运行编号下载 Task 4 Step 8 的试打包产物，模拟浏览器下载（给 dmg 加上浏览器会加的隔离标记；能请用户用 Safari 从 PR 的产物页下载更好）后，按 `docs/install.md` 拖进个人「应用程序」：

```bash
ACCEPT=$(mktemp -d "${TMPDIR:-/tmp}/issuerelay-m3-acceptance.XXXXXX")
run=$(gh run list --workflow release.yml --branch feat/m3-release --event pull_request --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" -n bundles-macos-universal -D "$ACCEPT/dry-run"
dmg=$(find "$ACCEPT/dry-run" -name 'IssueRelay_*_universal.dmg' | head -1)
xattr -w com.apple.quarantine "0083;$(printf '%x' "$(date +%s)");Safari;" "$dmg"
open "$dmg"   # drag IssueRelay to ~/Applications in Finder, then eject the disk image
```

后面的步骤开头用这三行重新求出 `ACCEPT`（找不到或不止一个就停下）：

```bash
set -- "${TMPDIR:-/tmp}"/issuerelay-m3-acceptance.*
[[ $# -eq 1 && -d $1 ]] || { echo "expected one acceptance folder, found: $*"; exit 1; }
ACCEPT=$1
```

记录：本机 macOS 版本（`sw_vers -productVersion`）；拷贝后的应用是否带隔离标记（`xattr -l ~/Applications/IssueRelay.app | grep quarantine`）。

- [ ] **Step 3: Gatekeeper（调研未确认 1）**

双击 `~/Applications/IssueRelay.app`：记下提示框的原文（截图只留本机）；按 `docs/install.md` 在「隐私与安全性」点「仍要打开」（用户输入密码）；记下按钮出现的位置与文字。确认提示不是「已损坏」。另拷一份到临时目录，用 `xattr -dr com.apple.quarantine` 的办法验证终端路径，验证后删掉那份。本机若不是 macOS 15 或 26，这两个版本记「待实机」（同事的电脑，见第 8 步）。文字与 `docs/install.md` 不符就改文档。

- [ ] **Step 4: 首次引导、PATH 与 `issuerelay start`（Review Focus 4、5 实机）**

1. 首次引导走一遍，用沙箱项目跑通一条需求（发现 → 运行 → 待验收 → 标记完成），确认安装包里的程序与 M2 验收时一致。
2. 「设置 → 系统 → 把命令行加入 PATH」：`ls -l ~/.local/bin/issuerelay` 指向 `~/Applications/IssueRelay.app/Contents/MacOS/issuerelay`；面板是否显示 shell 那一行与本机登录 shell 的 PATH 一致；新开终端 `~/.local/bin/issuerelay status`（PATH 里有 `~/.local/bin` 时直接 `issuerelay status`）有输出；经链接运行命令行程序时 macOS 是否另要放行一次（记录）。
3. 在 `~/.local/bin` 放一个自己的 `issuerelay` 文件（先关开关、删链接）：面板显示「已存在，且不是 IssueRelay 建的」，开关不可用，文件没被动；删掉后恢复。
4. 从托盘退出 → `issuerelay status` 输出 `Error: IssueRelay is not running; start it with \`issuerelay start\``、退出码 1；`issuerelay start` 输出 `IssueRelay started.` 且托盘图标出现；再运行一次输出 `IssueRelay is already running.`。
5. MCP：设置里把控制服务注册到 Claude Code，退出托盘程序，在 Claude Code 里调用 IssueRelay 的 `status` 工具：返回错误，文字含 `ask the user to open the IssueRelay app`，托盘程序没有被启动（`pgrep -x issuerelay-app` 无输出）。之后移除注册。
6. 从磁盘映像直接运行一次（挂载 dmg 后双击里面的应用）：「把命令行加入 PATH」显示「请先把它移到应用程序」且不可用；「安装并重启」（第 5 步的本机更新环境里）返回「只能更新已安装的应用」。

- [ ] **Step 5: 本机更新测试：A → B（Review Focus 1；调研未确认 2、4）**

用两份本机构建测试真正的更新：A 是当前版本，B 是下一个补丁版本、用真正的私钥签名的构建；两者的更新地址都临时指向本机。这两份只用于测试，不进仓库、不发布。B 的版本号只在 `$ACCEPT` 下的一次性 worktree 里改（`cargo update -w` 同步那里的 `Cargo.lock`，`stage-cli.sh` 的 `--locked` 才过得去），主检出的 `Cargo.toml`、`Cargo.lock` 从头到尾不动，也就没有「改回去」这一步（裁定 M3-AR4）：

```bash
set -- "${TMPDIR:-/tmp}"/issuerelay-m3-acceptance.*
[[ $# -eq 1 && -d $1 ]] || { echo "expected one acceptance folder, found: $*"; exit 1; }
ACCEPT=$1
mkdir -p "$ACCEPT/A" "$ACCEPT/serve"
bash scripts/stage-cli.sh
pnpm --dir app build && node scripts/release-checks.mjs panel
# A: the current version, its updater pointed at this machine (a test build; nothing is committed).
CI=true pnpm --dir app tauri build --bundles app --config '{"bundle":{"externalBin":["binaries/issuerelay"]},"plugins":{"updater":{"endpoints":["http://127.0.0.1:8765/latest.json"],"dangerousInsecureTransportProtocol":true}}}'
ditto target/release/bundle/macos/IssueRelay.app "$ACCEPT/A/IssueRelay.app"
# B: the next patch version, in a throwaway worktree of the same commit, signed with the real key,
# which is passed as a path and never read.
git worktree add --detach "$ACCEPT/b-tree" HEAD
current=$(node scripts/release-checks.mjs workspace-version)
next=$(echo "$current" | awk -F. '{printf "%d.%d.%d", $1, $2, $3 + 1}')
sed -i '' "/^\[workspace\.package\]/,/^\[/ s/^version = \"$current\"$/version = \"$next\"/" "$ACCEPT/b-tree/Cargo.toml"
cargo update -w --manifest-path "$ACCEPT/b-tree/Cargo.toml"
pnpm --dir "$ACCEPT/b-tree/app" install --frozen-lockfile
bash "$ACCEPT/b-tree/scripts/stage-cli.sh"
TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/issuerelay.key" CI=true pnpm --dir "$ACCEPT/b-tree/app" tauri build --bundles app --config '{"bundle":{"externalBin":["binaries/issuerelay"],"createUpdaterArtifacts":true},"plugins":{"updater":{"endpoints":["http://127.0.0.1:8765/latest.json"],"dangerousInsecureTransportProtocol":true}}}'
cp "$ACCEPT/b-tree/target/release/bundle/macos/IssueRelay.app.tar.gz" "$ACCEPT/b-tree/target/release/bundle/macos/IssueRelay.app.tar.gz.sig" "$ACCEPT/serve/"
node scripts/verify-update-signature.mjs --version "$next" --artifacts "[\"$ACCEPT/serve/IssueRelay.app.tar.gz\",\"$ACCEPT/serve/IssueRelay.app.tar.gz.sig\"]"
git status --short
```

（`--config` 写在一个 JSON 里：一次只传一个。`dangerousInsecureTransportProtocol` 只用于这两份测试构建。Expected：`verified IssueRelay.app.tar.gz`（用提交的公钥核对 B 的签名）；最后的 `git status --short` 里没有这一步带来的改动——worktree 在主检出之外。）

写本机清单并在另一个终端里起一个只监听本机的服务（测试结束时 Ctrl+C）：

```bash
set -- "${TMPDIR:-/tmp}"/issuerelay-m3-acceptance.*
[[ $# -eq 1 && -d $1 ]] || { echo "expected one acceptance folder, found: $*"; exit 1; }
ACCEPT=$1
next=$(node "$ACCEPT/b-tree/scripts/release-checks.mjs" workspace-version)
sig=$(cat "$ACCEPT/serve/IssueRelay.app.tar.gz.sig")
cat > "$ACCEPT/serve/latest.json" <<EOF
{
  "version": "$next",
  "notes": "Local update test",
  "pub_date": "2026-10-20T08:00:00Z",
  "platforms": {
    "darwin-aarch64": { "signature": "$sig", "url": "http://127.0.0.1:8765/IssueRelay.app.tar.gz" }
  }
}
EOF
python3 -m http.server 8765 --bind 127.0.0.1 --directory "$ACCEPT/serve"
```

测试：

1. 退出第 2 步装的应用，把它挪开（不删）再换上 A：`mv ~/Applications/IssueRelay.app "$ACCEPT/installed.app" && ditto "$ACCEPT/A/IssueRelay.app" ~/Applications/IssueRelay.app`（先重新求出 `ACCEPT`），从访达打开；登录 Linear（用户操作），确认连上。
2. 在沙箱项目建一条会跑一两分钟的需求，等它开始运行。
3. 「设置 → 检查更新」：显示「IssueRelay <next> 已发布」，顶部出现横幅。点「安装并重启」：出现与退出相同的询问（「1 个运行正在进行…」，并说明会重启安装更新）。选「等待完成」→ 说明变为「等运行结束后安装更新并重启」→ 关掉面板窗口再从托盘打开（等待中关窗只隐藏，程序不退出）→ 再改选「立即停止」：运行记为已取消、沙箱里没有残留进程（`ps -axo pid,ppid,pgid,command | grep -E 'claude|codex'`），程序安装并重启。
4. 重启后：面板侧栏版本为 `<next>`；`pgrep -x issuerelay-app | wc -l` 为 `1`（单实例插件没有挡住新实例，调研未确认 4）；托盘与面板正常；再 `open -b com.sunkailun.issuerelay` 只把面板带到前面。
5. 钥匙串（调研未确认 2）：重启后是否弹出「想要使用钥匙串中的机密信息」之类的授权框（原文记录；由用户决定点「始终允许」还是「允许」，记录点了哪个）；Linear 是否仍是已连接；再从托盘退出、重新打开一次，是否还弹。开机自启打开时，`~/Library/LaunchAgents/` 里的条目是否仍指向 `~/Applications/IssueRelay.app`，「登录项与扩展」里显示为什么。
6. 再来一次「点安装并重启 → 继续运行（Keep running）→ 从托盘退出」：程序只退出、不安装（版本仍是装着的那个）。再来一次「点安装并重启 → 等待完成 → 从托盘退出」：询问框仍在等待，文字改为「…之后 IssueRelay 退出」；运行结束后程序只退出、不安装。

- [ ] **Step 6: 失败的更新只提示（Review Focus 2 实机）**

让一条沙箱运行在进行中，依次把本机清单改成下面四种，每次点「检查更新」或「安装并重启」，记录面板的标题与细节行，确认什么都没安装、运行继续、托盘状态不变：

1. 签名换成另一把一次性密钥签的签名，版本仍为 `<next>`：「更新包的签名校验没有通过，什么都没安装」。生成那份签名（不碰真正的私钥）：

   ```bash
   set -- "${TMPDIR:-/tmp}"/issuerelay-m3-acceptance.*
   [[ $# -eq 1 && -d $1 ]] || { echo "expected one acceptance folder, found: $*"; exit 1; }
   ACCEPT=$1
   next=$(node "$ACCEPT/b-tree/scripts/release-checks.mjs" workspace-version)
   pnpm --dir app tauri signer generate --ci -w "$ACCEPT/other.key" > /dev/null
   cp "$ACCEPT/serve/IssueRelay.app.tar.gz" "$ACCEPT/other.tar.gz"
   pnpm --dir app tauri signer sign -f "$ACCEPT/other.key" -p "" --app-version "$next" "$ACCEPT/other.tar.gz" > /dev/null
   cat "$ACCEPT/other.tar.gz.sig"   # a signature, safe to show: paste it into the "signature" field of latest.json
   ```
2. `version` 改成当前版本：「已是最新版本」。
3. 停掉本机服务：「连不上更新服务器」（横幅若已显示，仍保留）。
4. `platforms` 里只留一个 `windows-x86_64`：「新版本还没有适合这台电脑的安装包」。

测试完：停掉服务，退出 IssueRelay，把第 2 步的安装放回去，再收掉一次性 worktree 与临时目录（里面只有本任务建的测试构建、下载与一次性密钥）：

```bash
set -- "${TMPDIR:-/tmp}"/issuerelay-m3-acceptance.*
[[ $# -eq 1 && -d $1 ]] || { echo "expected one acceptance folder, found: $*"; exit 1; }
ACCEPT=$1
mv ~/Applications/IssueRelay.app "$ACCEPT/tested.app"
mv "$ACCEPT/installed.app" ~/Applications/IssueRelay.app
git worktree remove --force "$ACCEPT/b-tree"
rm -rf "$ACCEPT"
git worktree list
```

Expected：`git worktree list` 只剩主检出。主检出的 `app/src-tauri/binaries/` 里仍是当前版本的命令行程序（B 是在 worktree 里编的），不用动。

- [ ] **Step 7: 报告（合入之前）**

写 `docs/gates/<日期>-m3-acceptance.md`：环境（macOS 版本、芯片、安装位置）、第 2–6 步每项的结果与原文（脱敏：`DEMO-N`、`Demo Project`）、与设计或文档不一致之处及改法。「未闭环」至少列出：

- macOS 15、26 上的 Gatekeeper 提示与「仍要打开」流程（本机不是这两个版本时）；「同事按说明用安装包装好并跑通」（设计第 17 节的完成标志，待同事）。
- Windows 全部待实机（没有 Windows 机器）：NSIS 按用户安装不要管理员与 SmartScreen 的提示原文；智能应用控制开启时的表现；「把命令行加入 PATH」的往返（打开、新终端 `issuerelay --version`、关闭；用户 `Path` 里其他条目逐字不变，`REG_EXPAND_SZ` 不变成 `REG_SZ`）；从终端 `issuerelay start`、关掉终端后托盘程序仍在；**正在运行的 `issuerelay.exe` 是否挡住 NSIS 更新与卸载**（调研未确认 3：在一个终端里跑 `issuerelay logs <编号> --lines 5000` 之类的长命令或让 AI 客户端开着 `issuerelay mcp`，再触发「安装并重启」与卸载；若被挡住，候选改法是 NSIS 的 `NSIS_HOOK_PREINSTALL` / `NSIS_HOOK_PREUNINSTALL` 钩子把 `issuerelay.exe` 也交给重启管理器，或在安装说明里要求先关掉）；MSI 安装要管理员、更新经 `msiexec` 弹管理员确认；NSIS 卸载程序是否提供「删除应用数据」以及它删哪些目录；另见文末「评审遗留的小项」里的「待实机的检查」：更新失败的两条路径（被占用的 `issuerelay.exe` 让 NSIS 更新失败、MSI 更新时拒绝管理员确认）与 MSI 更新后重新启动的程序是否带着管理员令牌，以及 macOS 15 / 26 的放行与钥匙串各条，逐条记入报告，没做的留作未闭环。
- `latest.json` 里的下载地址是 `api.github.com` 的资产地址：同一出口 IP 的未登录 API 限额（每小时 60 次）在多人同时更新时是否够用，待观察。

更新设计第 17 节 M3 一行：「安装包、发布流程、自动更新、把命令行加入 PATH 已在 macOS 实机验收（`docs/gates/<日期>-m3-acceptance.md`）；Windows 与同事安装待实机。」

```bash
git status --short
git add "docs/gates/$(date +%Y-%m-%d)-m3-acceptance.md" docs/superpowers/specs/2026-10-10-issuerelay-design.md docs/install.md
git diff --cached --stat
git commit -F - <<'EOF'
docs(m3): record the installer acceptance on macOS

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
git fetch origin
git push
```

（报告的文件名用写报告当天的日期；`docs/install.md` 只在实测改了文字时才有改动，没有就从 `git add` 里去掉。推送前按 AI_ENGINEERING_GUARDRAILS 第 7.3 节同步。）

然后请维护者按 `MAINTAINERS.md` 合并 M3 的 PR（CI 与 `release` 试打包全绿）。

- [ ] **Step 8: 合入后用私钥试打包一次**

合入后 `release.yml` 已在默认分支上，可以手动运行；这一次用上更新签名私钥，验证签名与核对脚本，仍不建 Release：

```bash
gh workflow run release.yml --ref main
run=$(gh run list --workflow release.yml --branch main --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" --exit-status
work=$(mktemp -d)
gh run download "$run" -D "$work"
find "$work" -type f | sed "s|$work/||" | sort
rm -rf "$work"
```

Expected：`verify` 的输出 `updater=true`（私钥已上传时）；`verify-dry-run` 通过（资产门查到 `.app.tar.gz` 与三个 `.sig`，由产物得出的 `latest.json` 每个平台键都有签名）；`bundle-dry-run` 的两个打包任务里「Check the update signatures against the committed public key」步骤输出 `verified IssueRelay.app.tar.gz`、`verified IssueRelay_<版本>_x64-setup.exe`、`verified IssueRelay_<版本>_x64_en-US.msi`；产物里有 `.app.tar.gz` 与三个 `.sig`。失败时修好再来，不推标签。

（刚触发时运行可能还没出现在列表里：`run` 为空或是上一次的编号时，几秒后再运行一次 `gh run list` 那一行。）

- [ ] **Step 9: 推标签，产出草稿 Release**

```bash
git fetch origin
version=$(git show origin/main:Cargo.toml | sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\(.*\)"$/\1/p')
git ls-remote --tags origin "v$version" | grep -q . && { echo "v$version exists already: stop and ask the maintainer"; exit 1; }
git tag -a "v$version" origin/main -m "IssueRelay v$version"
git push origin "v$version"
run=$(gh run list --workflow release.yml --event push --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" --exit-status
gh release list --limit 3
```

（同样，刚推送时运行可能还没出现，几秒后再查一次 `run`。）Expected：工作流全绿（`verify`、两个 `test`、`create-release`、`bundle-release` 的两个打包任务、`verify-release`；`bundle-dry-run` 与 `verify-dry-run` 跳过）；`gh release list` 里有一个 `Draft` 状态的 `IssueRelay v<版本>`，标题不含「incomplete」。从草稿下载 dmg 在本机装一遍（第 2–3 步的简版）。失败时按 `MAINTAINERS.md`「发布」的「失败或放弃」一条删草稿与标签，修好后重推同一个标签（不要用「Re-run failed jobs」修补草稿）。

**不公开草稿**：把草稿链接、资产列表与 `verify-release` 的结果交给维护者，由维护者检查后在 GitHub 页面上点「Publish release」。

- [ ] **Step 10: 补记第一个草稿**

在一个只改文档的小 PR 里，把第 8–9 步的结果（运行编号、资产列表、`latest.json` 的平台键）补进验收报告，设计第 17 节注明「v<版本> 草稿已生成，待维护者公开」：

```bash
git fetch origin
git switch -c docs/m3-first-draft origin/main
# Edit the acceptance report and the design's section 17 here, then:
git status --short
git add docs/gates/*-m3-acceptance.md docs/superpowers/specs/2026-10-10-issuerelay-design.md
git diff --cached --stat
git commit -F - <<'EOF'
docs(m3): record the first draft release

Co-Authored-By: <实际参与的模型> <noreply@anthropic.com>
EOF
git push -u origin docs/m3-first-draft
mktemp -d   # prints a folder for the PR description
```

把按 `.github/pull_request_template.md` 填好的描述写进上面打印的目录下的 `pr.md`，再 `gh pr create --title "docs(m3): record the first draft release" --body-file "<目录>/pr.md"`，之后 `rm -rf "<目录>"`。

---

## 自查记录（写计划时）

- **设计覆盖**：2.1 安装包、自动更新、命令行小程序——Task 1、3、4；2.2 第一版不做代码签名——默认 ad-hoc（macOS）与不签名（Windows），secrets 齐全时自动改用正式签名（Task 1、4）；4.1 命令行「托盘程序没在运行时给出提示，并可代为启动」——Task 2（`issuerelay start`，MCP 不启动）；9.5 通知——Task 3（四种运行通知不变，新版本通知跟随「自动检查更新」，装不上只在面板里说）；10.1 / 10.3 命令行子命令与 MCP 服务——Task 2；12 打包目标、附带程序、把命令行加入 PATH、自动更新、代码签名的放行步骤、版本号——Task 1–5；16 验证命令——每个任务都跑六条，发布工作流的 `test` 任务也跑六条；16.1 维护者发布、只由维护者升版本号（升版本是普通 PR）——Task 4 的 `MAINTAINERS.md`；17 M3 完成标志（同事按说明装好）——Task 6，同事安装与 Windows 待实机；18.2 代码签名——Task 4 的可选 secrets 与设计第 18 节的现状说明；18.4 Windows 验证环境——Task 6 的待实机清单。
- **占位扫描**：没有「待定」「以后再写」。需要执行时才有的值都由具体命令产生：公钥由 Task 3 Step 0 的 `node -e` 从 `.pub` 文件写入；工作区版本由 `scripts/release-checks.mjs workspace-version`（CI 与本机同一个解析）或 `MAINTAINERS.md` 的 `sed` 从 `Cargo.toml` 读出；Rust 与 npm 的依赖版本由 `cargo add` / `pnpm add` 解析后锁定；actionlint 的版本由 Task 4 Step 4 查最新发布。只有三类写法要在执行时换成真实的值：提交信息的 `Co-Authored-By: <实际参与的模型> …`（写实际参与的模型）、验收报告文件名里的日期（命令里用 `$(date +%Y-%m-%d)` 求出）、PR 描述文件所在的 `<目录>`（`mktemp -d` 打印出来的那个，变量不跨命令保留）。`zh-TW`、`ja`、`ko` 的文案按 M2 Task 2 的规则由实现者翻译（与 M2 计划的做法相同：计划给出 `en` 与 `zh-CN` 的确切文案）。几处库细节（`tauri_plugin_updater::Error` 的变体、`Update` 是否 `Sync`、`request_restart`、`winreg` 的 `RegValue`、`windows-sys` 的函数签名、tauri-action 对空 `releaseId` 的处理与它写的资产名和 `latest.json` 平台键、空的 `TAURI_SIGNING_PRIVATE_KEY` 与空字符串 secret 的处理、`7z` 列 NSIS 内容、`gh api` 跟随重定向、Tailwind 4 的最低 Safari 版本）都写明了按解析到的版本核对与不同时的改法。
- **与 M2 落地的代码一致（2ebda38）**：主程序名 `mainBinaryName: "issuerelay-app"`（由 `paths::APP_EXECUTABLE` 与配置测试共同守住）；`cli_next_to`、`cli_path`、`app_info().cli_path` 不改；面板命令由 M2 的 34 条加到 37 条（Task 2 的 `cli_on_path`、`install_cli_on_path`、`remove_cli_from_path`）再到 40 条（Task 3 的 `update_status`、`check_update`、`install_update`），每条在同一步里登记进 `generate_handler!`、`build.rs` 的 `PANEL_COMMANDS`、`capabilities/default.json` 的 `allow-*` 与 `backend.ts`、`api.ts`、`mock.ts`，`lib.rs` 的两个绊线测试与 `api.test.ts`（监听 4 → 5 个）守住；退出状态机的 `QuitState`、`QuitEvent`、`QuitStep` 加的是新成员（`Installing`、`Installed`、`Install`、`Restart`），`next` 已有的分支不改，`handle`、`request`、`request_update`、`ask` 与 `update::*` 都对 `R: Runtime` 泛型，M2 在 MockRuntime 上的退出测试扩展到 `Installing` 后照样跑；`QUIT_REQUESTED` 的载荷从 `usize` 改为 `QuitPrompt { running, purpose }`，用到它的地方全在 Task 3 里改：`events.rs` 的文档、`quit.rs` 的 `QuitStep::Ask` 文档与 `handle`、`backend.ts` 的 `onQuitRequested`、`api.ts` 的 `on<QuitPrompt>`、`mock.ts` 的 `listeners.quit`、`QuitDialog.tsx` 与它的测试（M2 的七个全部保留）、`App.test.tsx` 的启动失败页测试；`ERROR_CODES` 保留 `not_a_linear_link` 与 `login_cancelled`，加六个 `update_*`，与 Rust 的 `error_code()` 和文案键由 `update-failure-cases.json` 钉住；shell 那一行由 `shell-line.json` 钉住；假后端的更新状态叫 `updateState`（M2 已有 `update` 这个辅助函数）；变更钩子照 M2 的 `useAction<Variables, Result>(run(backend, variables), refreshes, failureKey, options?)` 写在 `queries.ts` 里；新版本通知沿用 `notify.rs` 的翻译与发送；版本号用 M2 的 `shell.version`；`Settings.auto_update` 沿用 M2 第一部分的字段，只改文档注释，M2 钉住旧旁注的测试随文案改写。
- **调研里未确认的事项 → 验收或检查步骤**：

| 未确认事项（调研） | 在哪里核对 |
|---|---|
| macOS 15 / 26 上未签名与 ad-hoc 包的提示原文、「仍要打开」流程 | Task 6 第 2–3 步（本机版本）；15 / 26 记待实机 |
| `xattr -dr com.apple.quarantine` 在 15 / 26 上可用 | Task 6 第 3 步 |
| 经 `~/.local/bin` 链接运行命令行程序要不要另行放行 | Task 6 第 4 步第 2 项 |
| ad-hoc 签名的新版本更新后是否再弹钥匙串授权、Linear 是否仍连接 | Task 6 第 5 步第 5 项 |
| 更新后开机自启条目与「登录项」的显示 | Task 6 第 5 步第 5 项 |
| `request_restart()` 与单实例插件：更新后程序确实在运行、只有一个实例 | Task 6 第 5 步第 4 项 |
| 「安装并重启」走等待 / 停止询问，安装期间关窗不结束进程（面板流程，调研建议用 `request_restart` / 退出事件） | Task 3 的 `quit.rs` 与前端测试；Task 6 第 5 步第 3、6 项 |
| 正在运行的 `issuerelay.exe` 挡不挡 NSIS 更新与卸载 | Task 6 第 7 步待实机清单（带测试方法与候选改法） |
| Windows 用户 PATH 的往返（`%VAR%` 条目逐字保留、类型不变） | Task 2 的纯函数测试；Task 6 第 7 步待实机 |
| 智能应用控制与 SmartScreen 的实际表现 | `docs/install.md`；Task 6 第 7 步待实机 |
| 通用包里的命令行程序真是双架构 | `stage-cli.sh --universal` 的自检；`bundle.yml` 的包内容检查；Task 4 Step 8 |
| 工作区继承的版本是否进入资产名 | Task 1 Step 7；Task 4 Step 8 的 `verify-dry-run` 与正式发布的 `verify-release`（同一个 `assets` 门） |
| tauri-action 写进 `latest.json` 的平台键（含 `darwin-universal*`）与资产地址 | `release-checks.mjs` 的 `PLATFORM_KEYS` 与自测；试打包检查由产物得出的那份；真实那份第一次出现在正式标签的 `verify-release`，不对就标记草稿 |
| 两个矩阵任务并发合并 `latest.json` 丢平台、重复草稿 | `create-release` 只建一次 + `max-parallel: 1` + `verify-release` |
| 空的 `APPLE_SIGNING_IDENTITY` 等环境变量的处理 | 不出现空值：Apple 的六个值只在「有 Apple 证书」那一个 tauri-action 步骤里；空的 `TAURI_SIGNING_PRIVATE_KEY` 在不打更新包时是否被忽略，第一次试打包核对（Task 4 Step 3 的说明） |
| 私钥与公钥不配对时 `tauri build` 只警告 | `scripts/verify-update-signature.mjs`（Task 4 Step 1 自测、`bundle.yml` 里运行、Task 6 第 8 步） |
| 从磁盘映像或临时副本运行时建链接、更新 | `cli_path::bundle_location` 的测试；Task 6 第 4 步第 6 项 |
| GUI 启动的 AI 客户端看不到 `~/.local/bin` | MCP 注册一直用绝对路径（M2）；`docs/install.md` 写明 |
| `api.github.com` 资产地址的未登录限额 | Task 6 第 7 步「未闭环」 |

- **Review Focus → 测试**：第 1 条——`quit.rs` 的 `the_quit_prompt_tells_the_panel_what_follows`、`a_verified_update_is_installed_where_quitting_would_exit`、`the_process_outlives_the_install_and_then_restarts`、`a_forced_exit_while_installing_stops_nothing_again`、扩展到 `Installing` 的 `every_exit_request_before_quitting_is_done_is_held` 与 M2 的退出状态机测试，`update.rs` 的 `a_verified_download_waits_for_the_quit_flow_and_is_reused`、`the_quit_flow_helpers_run_on_any_runtime`，`QuitDialog.test.tsx` 的「installing an update with runs in progress…」「a tray Quit while waiting to install…」，`UpdateBanner.test.tsx` 的第一个测试，Task 6 第 5 步；第 2 条——`update.rs` 的 `only_a_newer_version_is_offered`、`a_failed_check_only_informs`、`a_bad_signature_keeps_nothing_to_install`、`a_failed_check_keeps_the_version_already_offered`、`a_failed_install_is_shown_once_after_the_restart`，`UpdateBanner.test.tsx` 的第二、五、六个测试，`Settings.test.tsx` 的检查更新测试，Task 6 第 6 步；第 3 条——`release-checks.mjs` 的自测（`panel`、`tag`、`on-main`、`assets`、`latest-json` 各有反例），`release.yml` 的 `verify`（自测、标签、`origin/main`、密钥扫描、secrets）、`create-release`（同标签已有 Release）、`bundle.yml`（面板、包内容、签名核对）、`verify-release` 与 `verify-dry-run`（资产、`latest.json`、标记草稿），Task 4 Step 8 与 Task 6 第 8–9 步实际运行；第 4 条——`cli_path.rs` 的全部纯函数测试与 `link` 子模块的四个读写测试、`Settings.test.tsx` 的四个 PATH 测试、Task 6 第 4 步；第 5 条——`main.rs` 的 `start_launches_the_app_only_when_it_is_not_running`、`start_reports_an_app_that_never_answers_and_other_failures`、`a_stopped_app_points_to_issuerelay_start`，`mcp_tools.rs` 的两条新断言，Task 6 第 4 步第 4–5 项。
- **2026-10-14 按预检修订**：本计划写在 M2 桌面应用落地之前；预检（只读，对照 M2 落地的 2ebda38）找出 38 处问题，控制者给出裁定 M3-AR1–AR8 与逐条裁定，本次修订全部照办：新命令同一步登记进 `PANEL_COMMANDS`、能力文件与绊线测试，命令数 34 → 37 → 40（AR1）；Task 3 编辑而不替换 M2 的文件，`ERROR_CODES` 留着 `login_cancelled`，监听数 4 → 5，假后端的状态改名 `updateState`，`QuitPrompt` 改到每个用到它的地方，`QuitDialog` 的七个旧测试全部保留（AR2）；更新与退出的函数对 `R: Runtime` 泛型，在 MockRuntime 上测（AR3）；验收不再「升版本、打包、改回去」，升版本是普通 PR，更高版本的测试构建在一次性 worktree 里做（AR4）；最低 macOS 13.3（AR5）；打包前确认内嵌的不是占位页（AR6）；发布门禁成为带反例自测的 `scripts/release-checks.mjs`，试打包对将要打的标签与试打包产物跑同样的门（AR7）；每个任务都有时限，试打包只读、只有标签发布能写，secrets 只经步骤级 `env:`，jq 用 `--arg`，临时目录用完即删、变量每步重新求出，CI 用 actionlint 检查工作流（AR8）；以及逐条裁定（`useAction` 的真实签名、测试等真实条件、由 PATH 得出的非秘密事实与规则改写、不加第五个通知开关且装不上只在面板里说、新行的换行规则与 620 / 605 px 检查、规则改动单独提交、明确的 `git add` 与 `Co-Authored-By`、用例表、沿用 `notify.rs`、`missing_cli` 文案、`cleanDisplayText`、CODEOWNERS 与安全规范的更新段落、安装期间与托盘退出的边界、订阅的清理、按签名情况措辞的安装说明）。照原样接受、未改的：预检「未检查」一节的各项（`tauri-plugin-updater` 的 API、tauri-action 对 `releaseId` 的行为、Windows 上的编译、`gh api` 的重定向）仍由实现者按解析到的版本文档核对，由 CI 与试打包确认。修订时另外改正了几处新发现的不一致：M2 钉住旧「自动更新」旁注的测试要随文案改写；「系统」分组合起来的顺序（开机自启、PATH、自动检查更新、版本与检查更新）；`latest.json` 的 `notes` 是写给下载页的 Release 正文，面板不再显示它；Task 6 不再依赖 Task 4 留下的 `$SCRATCH`，自己按运行编号下载产物。

## 评审遗留的小项

本计划执行中，各任务评审、对整条 M3 分支的最终评审与最终修复波决定接受、没有修复的项，以及还要在真实机器上确认的事。SDD 工作区在合并后删除，这里是这些结论唯一的存放处；写后续计划或做验收时逐条认领，或写明不做及原因。

**最终评审接受的遗留项**

| 内容 | 接受的理由 | 认领 |
|---|---|---|
| 第一个任务把文档单独拆成了第二个提交，与计划里的一个提交不同 | squash 合并之后没有区别 | 不做 |
| `latestJsonProblems` 的 `where = null` 默认值偏宽：不传仓库时，地址的来源不核对 | 正式发布只有 `latest-json` 一个调用方，它由必填的 `--repo`（校验格式）和 `--sigs` 组出 `where`；默认值只给试打包那道门用；自测把两种模式都钉住了 | 不做 |
| 草稿复查在构建之前，竞态窗口只是缩小、没有关上：复查之后、上传之前，草稿仍可能被公开或删除 | 彻底关上要改成「先构建，复查草稿，再上传」，属于重做；`verify-release` 会再复查一次，维护者也只在全绿之后才公开 | 以后要关上它时：把上传从 tauri-action 里拆出来，构建完再复查、再上传 |
| `quit::on_run_event` 文档注释里「Not about quitting, but …」那句不成句，`quit.rs` 里同一条平台说明重复了五处 | 每处都在解释紧挨着的代码，属于观感问题 | 下次改 `quit.rs` 时顺手 |
| 两条日语提示（Dock 开关的说明、自动更新的说明）为了在最小窗口不折行而缩短，措辞没有母语者看过 | 在 900x600 与 885x600 的窗口里量过：一行放得下，余量 28 px 以上；意思没变 | 有日语使用者时复核 |

**评审时登记的后续改进**

| 内容 | 认领 |
|---|---|
| 有第二个协作者之后，给发布用的 secrets 建一个只允许 `v*` 标签使用的 Environment（现在只有所有者有写权限；这是仓库设置，由所有者决定） | 有第二个协作者时 |
| 把构建与签名拆成两步（先无 secrets 构建，再单独签名）：现在 cargo 的构建脚本与过程宏在带着签名 secrets 与可写令牌的环境里运行（`MAINTAINERS.md`「发布用的 secrets」写明） | 视需要，与上一行一起考虑 |
| NSIS 安装包的 PREINSTALL（与 PREUNINSTALL）钩子把 `issuerelay.exe` 也登记给重启管理器：模板只让它关主程序，Claude Code、Codex 里运行着的 `issuerelay mcp` 会占着命令行；或更新前由应用自己结束它启动的命令行子进程（`on_before_exit`） | 实机确认被占用的 `issuerelay.exe` 会挡住更新之后（见下面「待实机的检查」第 1 行） |
| `cli_path::expand_vars` 与 `env::expand_references` 是两份几乎一样的 `%变量%` 展开，合成一份 | 下次改其中一个时 |
| Windows 上 `issuerelay start` 起的托盘程序继承终端的环境，而且只用了 `DETACHED_PROCESS` 与 `CREATE_NEW_PROCESS_GROUP`：终端在作业对象里运行、关掉时又会结束整个作业的话，托盘程序可能跟着被结束；可以试加 `CREATE_BREAKAWAY_FROM_JOB`（作业要允许脱离才有用） | Windows 实机确认之后 |
| `issuerelay start --json` 打印的仍是文字，不是 JSON | 下次改 `start` 时 |
| Windows 上任何非调试构建都算「已安装」（`cli_path::app_location`），包括从「下载」文件夹或临时目录直接运行的副本；这时「把命令行加入 PATH」会把那个目录写进 `Path` | 下次改 `cli_path.rs` 时：Windows 也要有「不是从安装目录运行」的判断 |
| 本计划里还写着旧的锁定式 PATH 开关：Task 2 的设置页代码里 `disabled` 含 `current.kind === "blocked"`，对应的测试名也说「blocks the switch」，Task 6 第 4 步第 3 项写「开关不可用」。落地的行为是：开关保持关闭、仍然可点，点了而那个文件还在时，面板仍写「已存在，且不是 IssueRelay 建的」，什么也不改 | 不改计划文字；执行 Task 6 第 4 步第 3 项时按落地的行为做 |

**第三个任务留下的小项**

| 内容 | 认领 |
|---|---|
| `install_and_restart`（要用 `AppPaths::system()` 取数据目录，规则不许测试里用真实数据目录）与 `setup` 的其余接线没有单元测试。已测的部分：`install_leaving_notes`（标记与安装的先后、失败原因）、标记的读写、`startup::shows_panel` 与 `starts_hidden`，以及 `update::manage`（更新状态和退出流程读的那份句柄都被管理，且是同一份） | Task 6 第 5 步的实机更新 |
| 设置页版本一行还有一个很窄的缺口：按钮上一次的失败还显示着，之后后台检查成功、而且找到的正好是应用早就保留的同一个版本（状态内容完全相同，对象不变）时，行里的失败不会被换掉，要再点一次或离开设置页 | 不做，除非有人碰到 |
| 点「安装并重启」到应用重启之间（没有任务在跑时是停宿主加安装，几秒钟）面板没有「正在安装」的反馈：命令一返回，按钮就恢复原样。不能简单地在成功之后禁用它，因为选了「继续运行」之后用户要能再点；要做，得让 Rust 在进入 `Installing` 时发一个事件 | 视实机体验 |
| 两个平台上的真实更新（下载、验签、等待任务、安装、重启）都没有验证过 | Task 6 第 5 步（macOS）；Windows 实机验收 |

**Dock 图标（`show_in_dock`）留下的小项**

| 内容 | 认领 |
|---|---|
| 激活策略的接线（`lib.rs` 里登录启动时的 `hide`、`startup.rs` 的 `refresh_dock`、`commands.rs` 保存设置后的 `refresh_dock`、`window.rs` 的 `show` 与 `hide`）没有自动测试：MockRuntime 的 `set_activation_policy` 什么也不做，看不出策略有没有被设置。已测的部分：决策表（`dock_policy`、`DockSetting`，含宿主没能启动的一行）、由启动状态得出设置的两支（启动中、启动失败）、两个策略与 AppKit 激活策略的对应 | 不为它造接缝；macOS 实机验收时看程序坞图标：开机自启的头几秒、关掉面板之后、更新重启之后、关掉设置之后、宿主没能启动时 |

**待实机的检查**

| 内容 | 由谁认领 |
|---|---|
| Windows：NSIS 更新遇到被占用的 `issuerelay.exe`（Claude Code、Codex 里运行着 `issuerelay mcp`，或开着 `issuerelay` 的终端）：安装程序怎么说，IssueRelay 是否保持关闭、能否从开始菜单重新打开，卸载同理 | Windows 实机验收（Task 6 第 7 步的 Windows 清单；候选改法是上面的 NSIS 钩子） |
| Windows：MSI 更新时拒绝管理员确认：IssueRelay 是否保持关闭、旧版本能否从开始菜单正常打开 | 同上 |
| Windows：MSI 更新后重新启动的程序是否带着管理员令牌（带的话，执行者也会以管理员身份运行，真实的 `claude`、`codex` 就在提权环境里跑） | 同上 |
| Windows：安装前留下的「显示面板」标记在重启后确实让面板显示出来（NSIS 的 `/R` 带着原来的参数重启，含 `--autostart`）；安装程序一启动进程就退出这一点，目前是读更新插件的源码得出的 | 同上 |
| Windows：`docs/install.md` 与设计第 12 节里标了「待 Windows 实机」的各条：安装、SmartScreen 与智能应用控制的提示原文、PATH 的写入与广播、更新、环境检查里「PATH 来自」显示「系统设置」 | 同上 |
| macOS 15、26 上 Gatekeeper 的提示原文与「仍要打开」流程；点了「仍要打开」之后，包内的命令行程序（经 `~/.local/bin` 的链接运行）会不会又被拦一次，拦的话要 `xattr -dr`，并写进 `docs/install.md` | Task 6 第 3、4 步（本机的版本）；15、26 在同事的电脑上 |
| ad hoc 签名的版本更新后，钥匙串是否再问、该点哪个按钮、Linear 是否仍连接（`docs/install.md` 已按预期写了「始终允许」，以及点「拒绝」的后果） | Task 6 第 5 步第 5 项 |
| 更新后的重启：`request_restart()` 与单实例插件配合，程序在运行且只有一个实例；开机自启条目与「登录项」的显示；macOS 上换不了应用包时面板顶部的说明 | Task 6 第 5 步第 4、5 项 |
| 第一次带私钥的手动试打包与第一次公开的更新：插件下载 API 资产地址时带 `Accept: application/octet-stream`，命令行在 CI 里用空密码签名 | Task 6 第 8 步，以及第一次公开之后的更新检查 |

**M3 实机验收的结果与新发现**（`docs/gates/2026-10-14-m3-acceptance.md`）

| 内容 | 结果或认领 |
|---|---|
| 上表里 macOS 的几行 | 在本机 macOS 27 上：<br>• Gatekeeper 的提示与「仍要打开」流程按说明走通；放行后，包里的命令行程序经链接运行，不再被拦。<br>• 更新后钥匙串会再问一次，输入密码后 Linear 仍连接。<br>• 更新后程序在运行，且只有一个实例。<br>• 第一次带私钥的手动试打包全绿。<br>仍待：macOS 15、26；第一次公开之后的更新检查。 |
| 应用被随机路径隔离（App Translocation）时，面板说「请先把它移到应用程序」，但应用其实已经在里面。用终端拷贝的应用会这样（验收 N2） | 换一句提示，说明原因与「终端里的办法」；下次改 `cli_path` 时做 |
| 无人值守运行照常加载用户的插件、钩子与 MCP 服务；运行被取消时，插件钩子另起的后台进程不在进程树里（验收 N3） | 并入精简运行的后续任务（设计第 18 节第 6 条），默认值由维护者决定 |
| 启动后的第一轮检查总记为失败（`connecting to Linear`）。连上后引擎会立即再查一次，但要等这一轮跑完才恢复（验收 N4） | 还在连接时的检查不记为失败，只显示「正在连接」；与 M2 验收的 N4 一起看 |
| 第一个版本发布之前，更新检查显示「连不上更新服务器」，实际原因是清单不存在（404）（验收 N5） | 清单不存在时改说「还没有发布的版本」；下次改 `update.rs` 时做 |
