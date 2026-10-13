# IssueRelay M3：安装与发布 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让同事不用命令行就能装上 IssueRelay：推送 `vX.Y.Z` 标签时，GitHub Actions 在 macOS 与 Windows 上测试、打出 macOS 通用 `.dmg` 与 Windows NSIS `.exe`、`.msi`（都带命令行程序 `issuerelay`），放进**一个**草稿 Release，由维护者确认后公开；程序能检查并安装签过名的更新；设置里能不用管理员权限把命令行加入 PATH；安装说明写清未公证、未签名的包怎样放行。

**Architecture:** 打包配置分两层：所有构建都读的 `app/src-tauri/tauri.conf.json`（打包目标、macOS ad-hoc 签名、NSIS 按用户安装、更新插件配置，不写 `version`，版本取工作区 `Cargo.toml`），和只在打包时用 `--config` 合并的 `tauri.bundle.conf.json` / `tauri.updater.conf.json`（`externalBin`，后者再打开更新包），所以 `tauri dev` 与 `ci.yml` 不需要先放好命令行程序。命令行程序由 `scripts/stage-cli.{sh,ps1}` 编译后放成 `app/src-tauri/binaries/issuerelay-<三元组>[.exe]`，Tauri 去掉后缀装在主程序 `issuerelay-app` 旁边。「把命令行加入 PATH」的判断全是 core `cli_path` 模块里的纯函数（macOS：`~/.local/bin` 里的符号链接；Windows：`HKCU\Environment\Path`），面板经三条新命令调用；`issuerelay start` 拉起托盘程序，MCP 服务从不拉起。自动更新用 `tauri-plugin-updater`：`update.rs` 里可测试的 `Updates<S: UpdateSource>` 管检查、下载验签与「每个版本只通知一次」，「安装并重启」先下载并验签，再交给 M2 的退出状态机（`quit::request_update`），有任务在跑时问同一个「等待完成 / 立即停止」。发布工作流：`verify` → `test` → `create-release`（只建一个草稿，输出 `releaseId`）→ `bundle`（矩阵，逐个运行，`tauri-apps/tauri-action` v1.0.0 按提交固定）→ `verify-release`（资产与 `latest.json` 齐全）；手动运行与改了打包配置的 PR 只试打包，产物存为工作流产物。

**Tech Stack:** Tauri 2.12 bundler（dmg、msi、nsis）、`tauri-plugin-updater` 2.13、`semver`、`winreg` 与 `windows-sys`（用户 PATH）、GitHub Actions（`tauri-apps/tauri-action` v1.0.0、`pnpm/action-setup` v6.1.0、`actions/setup-node` v7、`Swatinem/rust-cache` v2.9.2、`actions/upload-artifact` v7）、`lipo`、Node 24（签名核对脚本）。

**Spec:** `docs/superpowers/specs/2026-10-10-issuerelay-design.md` 第 2.1、2.2、4.1、10、12、16、16.1、17、18 节；依赖 M2 第一部分 `docs/superpowers/plans/2026-10-13-issuerelay-m2-core.md` 与第二部分 `docs/superpowers/plans/2026-10-13-issuerelay-m2-app.md` 产出的接口（各任务的 Interfaces 写明用到的名字）。技术调研（2026-10-13，带出处）只在本机 `.superpowers/sdd/2026-10-13-issuerelay-m3-release/research.md`，不进仓库；本计划用到的结论都已写进正文，执行时不必另读。

## Global Constraints

- M2 两部分的全部全局约束继续有效：硬规则、两个平台同等对待（每个 `#[cfg(unix)]` 都有 `#[cfg(windows)]` 实现）、测试不碰网络 / 真实钥匙串 / 真实注册表、虚构测试数据、公开仓库红线、英文注释中文文档、六条验证命令（`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`pnpm --dir app lint`、`pnpm --dir app test`、`bash scripts/check-secrets.sh --strict` 无输出）、英文 Conventional Commits 与写实际模型的 `Co-Authored-By`、分支与 PR、禁止强推。
- 分支 `feat/m3-release`，从合入 M2 第二部分之后的 `main` 切出，经 PR 合入。
- 计划里的 Rust 代码块不一定是 rustfmt 的排版：每次提交前先 `cargo fmt --all`，再跑六条验证命令。
- 版本号只有一个来源：工作区 `Cargo.toml` 的 `[workspace.package] version`。`tauri.conf.json` 不写 `version`（Tauri 取 crate 版本，`issuerelay-app` 用 `version.workspace = true`，且在工作区 `members` 里，tauri-action 才找得到版本）。标签为 `v` 加这个版本，发布工作流核对，不一致就失败。版本只用 `X.Y.Z`：MSI 不接受 `-rc.1` 这类非纯数字的预发布后缀。只由维护者在发布时升（设计 12、16.1）。
- 可执行文件名：主程序到处都是 `issuerelay-app`（Windows `issuerelay-app.exe`，M2 的 `mainBinaryName`），没有 `IssueRelay.exe`；命令行程序 `issuerelay`（`issuerelay.exe`）装在它旁边：macOS `IssueRelay.app/Contents/MacOS/issuerelay`，Windows 安装目录（NSIS 按用户安装默认 `%LOCALAPPDATA%\IssueRelay`）下的 `issuerelay.exe`。主程序名永远不能与 `issuerelay` 只差大小写（macOS 的 APFS 与 Windows 的 NTFS 不分大小写，两份文件会静默互相覆盖）。
- `externalBin` 只写在打包专用的配置里；`tauri.conf.json` 本身必须在没有放好命令行程序时也能 `cargo build` / `cargo clippy` / `cargo test`（Task 1 的配置测试守住这一条）。`app/src-tauri/binaries/` 是生成目录，git 忽略。
- 签名：没有 Apple 证书时 macOS 包一律 ad-hoc 签名（`bundle.macOS.signingIdentity: "-"`）——未签名的 arm64 应用缺少封存的资源，下载后会被报「已损坏」；有 Apple 证书（secrets 里六项都在）时改用正式签名并公证。Windows 没有证书时不签名。证书、私钥、密码只放在 GitHub Actions secrets，只在存在时传入，拉取请求触发的试打包从不使用；仓库、日志、提交信息、聊天里都不能出现。
- 自动更新：清单地址固定为 `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json`；只安装比当前版本新、且用仓库里那把公钥验签通过（并且签名里记的版本与清单一致，`requireSignedVersion: true`）的更新包；检查失败、清单缺本平台、验签失败、版本不新，都只提示，不安装，不影响监控；安装一律由用户点「安装并重启」，有任务在跑时走与退出相同的询问。
- 更新签名密钥：由主智能体在维护者本机生成（不交给子智能体，不在云端会话做），不设密码，私钥存 `~/.tauri/issuerelay.key`（权限 600），从不打印、不读进聊天、不提交、不进日志；只提交公钥。私钥由维护者自己用 `gh secret set` 上传。
- 发布工作流只建草稿 Release；公开由维护者在 GitHub 页面上点（对外动作）。任何一个平台没打出来、或 `latest.json` 缺平台，工作流失败并把草稿标成「incomplete: do not publish」，不会出现被当成完整版本的半套资产。
- 依赖版本（调研时的最新版，执行时用 `cargo add` / `pnpm add` 解析后锁定）：crate `tauri` 2.12.x、`tauri-plugin-updater` 2.13.x、`semver` 1.x；npm `@tauri-apps/cli` 不低于 2.12.0（从这版起签名里记录应用版本）。插件与 Tauri 的 API 名称以解析到的版本文档（docs.rs、v2.tauri.app）为准，与本计划写法不同时照文档写，并在报告里列出差异。
- `release.yml` 里第三方的 action 一律按提交 SHA 固定并在行尾注明版本（`# v1.0.0`），GitHub 自己的 action（`actions/*`）用主版本标签，与 `ci.yml` 一致；Dependabot 的 github-actions 条目照常更新它们。
- Windows：没有 Windows 机器（设计 18.4）。Windows 代码必须在 CI 上编译通过、纯逻辑测试通过；注册表读写不在测试里碰真实注册表，Windows 实机行为一律记「待实机」。
- 测试、文档、示例里不出现本机路径：不写用户目录（`/Users/…`、`/home/…`、`C:\Users\…`）或外置卷（`/Volumes/…`）下的具体路径，`check-secrets.sh` 会拦下它们；家目录用 `/tmp/demo-home`，Windows 目录用 `D:\Profiles\demo` 或 `%LOCALAPPDATA%`。

## Review Focus

1. 有任务在跑时点「安装更新并重启」：要走与退出相同的询问（等待完成 / 立即停止，等待中仍可改为立即停止），不能直接重启丢掉运行；点「继续运行」后再从托盘退出，只退出、不装更新（Task 3：`quit.rs` 的 `QuitPrompt` 测试、`update.rs` 的 `a_verified_download_waits_for_the_quit_flow_and_is_reused`、前端 `QuitDialog` 与 `UpdateBanner` 测试；Task 6 第 5 步实机）。
2. 更新清单拿不到（离线、代理、还没发布过）、清单里没有本平台、验签失败、版本不比当前新（含预发布与写坏的版本号）：只提示，不安装，不影响监控；后台检查失败不收回已经提示过的新版本（Task 3：`update.rs` 的 `only_a_newer_version_is_offered`、`a_failed_check_only_informs`、`a_bad_signature_keeps_nothing_to_install`、`a_failed_check_keeps_the_version_already_offered`；Task 6 第 6 步实机）。
3. 标签与 `Cargo.toml` 版本不一致、标签提交不在 `origin/main` 上、同一标签已有 Release、secrets 缺失、某个平台没打出来、`latest.json` 缺平台或缺签名、签名私钥与提交的公钥不配对：工作流明确失败或明确降级，草稿不会带着半套资产被当成完整版本（Task 4：`verify` 任务三项检查、`create-release` 的重复检查、`bundle` 的包内容检查与签名核对、`verify-release` 的资产与平台检查；Task 4 第 7 步的试运行）。
4. 用户没有管理员权限、`~/.local/bin` 不存在或不在 PATH、那里已有别人的同名文件、应用从磁盘映像或临时副本运行、Windows 用户 PATH 是带 `%VAR%` 的 `REG_EXPAND_SZ` 或已经有这一项（大小写或结尾反斜杠不同）：不需要管理员、不覆盖、不重复、不改坏其他条目，移除时只删自己这一项（Task 2：core `cli_path` 的纯函数测试与 macOS 链接读写测试）。
5. 托盘程序没运行时调用命令行（包括 AI 经 MCP 调用）：命令行说清楚并指向 `issuerelay start`；`issuerelay start` 把托盘程序拉起来（已在运行就只说明），等到它应答；MCP 服务只报错、不启动，也不叫 AI 自己去启动（避免 AI 静默启动有完全权限的程序）（Task 2：命令行 `start` 与提示的测试、`crates/cli/tests/mcp_tools.rs` 的断言）。

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `app/src-tauri/tauri.conf.json` | 所有构建都读的配置：打包目标、ad-hoc 签名、NSIS 按用户安装、去掉 `version`；Task 3 加 `plugins.updater` | 1、3 |
| `app/src-tauri/tauri.bundle.conf.json`、`app/src-tauri/tauri.updater.conf.json` | 只在打包时 `--config` 合并：`externalBin`；后者另打开 `createUpdaterArtifacts` | 1、3 |
| `app/src-tauri/tests/packaging.rs` | 配置守卫测试（基础配置不含 `externalBin`、主程序名、版本来源、更新配置） | 1、3 |
| `scripts/stage-cli.sh`、`scripts/stage-cli.ps1` | 编译命令行程序并放成 `binaries/issuerelay-<三元组>[.exe]`；`--universal` 时三份 | 1 |
| `.gitignore` | `app/src-tauri/binaries/` | 1 |
| `crates/core/src/paths.rs` | `paths::APP_EXECUTABLE`（主程序文件名，配置测试与 `issuerelay start` 共用） | 1 |
| `crates/core/src/cli_path.rs`、`crates/core/Cargo.toml` | 应用安装位置判断、「把命令行加入 PATH」的纯逻辑与 macOS 链接 / Windows 注册表读写 | 2 |
| `app/src-tauri/src/commands.rs`、`app/src-tauri/src/lib.rs` | 新命令：`cli_on_path`、`install_cli_on_path`、`remove_cli_from_path`（Task 2）；`update_status`、`check_update`、`install_update`（Task 3） | 2、3 |
| `crates/cli/src/main.rs`、`crates/cli/src/mcp.rs`、`crates/cli/tests/mcp_tools.rs` | `issuerelay start`、没运行时的提示；MCP 只报错不启动 | 2 |
| `app/src-tauri/src/update.rs`、`app/src-tauri/src/quit.rs`、`app/src-tauri/src/events.rs`、`app/src-tauri/src/i18n.rs` | 自动更新；退出状态机接上「安装并重启」；`update-status` 事件；Rust 侧用到的翻译键 | 3 |
| `app/src/lib/{types,backend,api,mock,mock-fixtures}.ts`、`app/src/hooks/{queries,useEngineEvents}.ts` | 前端类型、命令封装、假后端、查询与事件 | 2、3 |
| `app/src/pages/Settings.tsx`、`app/src/components/{UpdateBanner,QuitDialog}.tsx`、`app/src/components/layout/AppShell.tsx` | 设置页的 PATH 开关与检查更新；更新横幅；退出询问区分「退出」与「安装更新」 | 2、3 |
| `app/src/i18n/locales/*.json` | 新文案（五种语言） | 2、3 |
| `scripts/check-secrets.sh` | 拦截更新签名私钥的两种写法 | 3 |
| `docs/release/updater-key.md` | 更新签名密钥的生成、保存、上传、轮换 | 3 |
| `.github/workflows/release.yml`、`scripts/verify-update-signature.mjs` | 发布工作流；用提交的公钥核对更新包签名 | 4 |
| `.github/workflows/ci.yml` | pnpm 与 Node 的 action 版本与发布工作流一致 | 4 |
| `MAINTAINERS.md`、`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` | 发布步骤与 secrets；发布只从 `origin/main` 上的提交构建 | 4 |
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
- Modify: `.gitignore`（`app/src-tauri/binaries/`）
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 12 节）、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 4、6 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 2 节）、`CONTRIBUTING.md`（「开发环境」一节）

**Interfaces:**
- Consumes（M2，不改）：`tauri.conf.json` 的 `productName: "IssueRelay"`、`mainBinaryName: "issuerelay-app"`、`identifier: "com.sunkailun.issuerelay"`、`bundle.icon`；`app/src-tauri/src/commands.rs` 的 `fn cli_next_to(exe: &Path) -> PathBuf`（`issuerelay` / `issuerelay.exe`，与主程序同目录）与 `fn cli_path() -> Option<PathBuf>`、`app_info()` 的 `AppInfo.cli_path`——Tauri 去掉三元组后缀后，命令行程序正好在 `cli_next_to(current_exe)`，这两个函数不用改；`app/src-tauri/build.rs` 的面板占位逻辑；根 `Cargo.toml` 的 `members = ["crates/*", "app/src-tauri"]`。
- Produces：`pub const issuerelay_core::paths::APP_EXECUTABLE: &str = "issuerelay-app"`；`app/src-tauri/tauri.bundle.conf.json`（`{"bundle":{"externalBin":["binaries/issuerelay"]}}`，打包时 `--config src-tauri/tauri.bundle.conf.json`，路径相对 `app/`）；`bash scripts/stage-cli.sh [--target <三元组> | --universal]` 与 `pwsh scripts/stage-cli.ps1 [-Target <三元组>]`，输出 `staged app/src-tauri/binaries/issuerelay-<三元组>[.exe]`；本机打包命令 `pnpm --dir app tauri build --bundles app,dmg --config src-tauri/tauri.bundle.conf.json`。

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
    assert_eq!(bundle["macOS"]["minimumSystemVersion"], "12.0");
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
      "minimumSystemVersion": "12.0",
      "signingIdentity": "-"
    },
    "windows": {
      "nsis": {
        "installMode": "currentUser"
      }
    }
  }
```

（打包器在每个平台只保留能打的目标：macOS 打 `app`、`dmg`，Windows 打 `msi`、`nsis`，一张列表两边通用。`signingIdentity: "-"` 让打包器对附带程序、主程序、应用包依次执行 `codesign --force -s - --options runtime`；dmg 本身在 ad-hoc 时不签。环境变量 `APPLE_SIGNING_IDENTITY` 会覆盖这个值，Task 4 有证书时用它。NSIS 的 `currentUser` 本是默认值，写明是为了配置测试与 Review Focus 4。MSI 的官方模板固定为所有用户安装、需要管理员，没有按用户的选项，安装说明推荐 NSIS。）

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

- [ ] **Step 5: 在本机核对脚本与打出的应用包**

先给固定的工具链装上两个 Apple 目标（在仓库目录里运行，`rust-toolchain.toml` 固定的 1.99.0 才会拿到它们）：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash -n scripts/stage-cli.sh && bash scripts/stage-cli.sh --help
bash scripts/stage-cli.sh --universal
ls app/src-tauri/binaries
```

Expected: `usage: …`；三行 `staged app/src-tauri/binaries/issuerelay-aarch64-apple-darwin`、`…-x86_64-apple-darwin`、`…-universal-apple-darwin (x86_64 arm64)`；目录里正好这三个文件。

再按本机三元组打一个应用包（通用包由 Task 4 的工作流打；本机只为看清包的结构）。`CI=true` 让打包器跳过给 dmg 窗口排版的访达脚本，避免弹出「控制访达」的授权框（只影响 dmg 窗口的外观）：

```bash
bash scripts/stage-cli.sh
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

Expected：`Contents/MacOS` 里正好有 `issuerelay-app` 和 `issuerelay`（三元组后缀已去掉）；`issuerelay 0.1.0`（工作区版本）；托盘程序没运行时 `Error: IssueRelay is not running`、`exit 1`（Task 2 再补上指向 `issuerelay start` 的提示）；`codesign --verify` 输出 `valid on disk` 与 `satisfies its Designated Requirement`；`Signature=adhoc`，`CodeDirectory` 行的 flags 含 `adhoc,runtime`；`spctl` 说 `rejected`（未公证的正常结果，**不是** `code has no resources but signature indicates they must be present`）；`dmg` 目录里有 `IssueRelay_0.1.0_aarch64.dmg`。任何一项不同（例如主程序不叫 `issuerelay-app`、附带程序带着后缀、签名标志不同），停下来把实际输出写进报告。核对完删掉 `app/src-tauri/binaries/` 以外的打包产物不是必须的（都在 git 忽略的 `target/` 里）。

- [ ] **Step 6: 文档**

- 设计第 12 节第一条改为：「Tauri 打包：macOS 出一个通用 `.dmg`（Apple 芯片与 Intel 通用），Windows 出 NSIS `.exe`（按用户安装，不需要管理员，推荐）和 `.msi`（为所有用户安装，需要管理员）。命令行小程序作为附带程序（`externalBin`）装在主程序 `issuerelay-app` 旁边：macOS `IssueRelay.app/Contents/MacOS/issuerelay`，Windows 安装目录下的 `issuerelay.exe`。`externalBin` 只写在打包专用的 `app/src-tauri/tauri.bundle.conf.json` 里，由 `scripts/stage-cli.sh` / `stage-cli.ps1` 先把命令行程序编译好放进 `app/src-tauri/binaries/`；开发构建不需要它。没有 Apple 证书时 macOS 包做 ad-hoc 签名（未签名的 arm64 应用下载后会被报「已损坏」）；设置里提供「把命令行加入 PATH」。」
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 4 节 `app/` 的布局加：`app/src-tauri/tauri.bundle.conf.json`（打包时合并的配置）、`app/src-tauri/binaries/`（生成目录，git 忽略，放 `stage-cli` 编译好的命令行程序）、`app/src-tauri/tests/`（配置守卫测试）；第 6 节加：`scripts/stage-cli.sh`、`scripts/stage-cli.ps1`：编译命令行程序并放到 Tauri 的 `externalBin` 找它的地方（用法见文件头）。
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 2 节的表加一行：`| 本机安装包（macOS） | bash scripts/stage-cli.sh && CI=true pnpm --dir app tauri build --bundles app,dmg --config src-tauri/tauri.bundle.conf.json |`。
- `CONTRIBUTING.md`「开发环境」一节末尾加一段：「想在本机打安装包：先 `bash scripts/stage-cli.sh`（Windows 上 `pwsh scripts/stage-cli.ps1`）把命令行程序放进 `app/src-tauri/binaries/`，再 `pnpm --dir app tauri build --config src-tauri/tauri.bundle.conf.json`。只开发面板与托盘时不需要这一步。」

Run: 六条验证命令
Expected: 全部通过。

- [ ] **Step 7: Commit**

```bash
git add app/src-tauri/tauri.conf.json app/src-tauri/tauri.bundle.conf.json app/src-tauri/tests/packaging.rs \
  crates/core/src/paths.rs scripts/stage-cli.sh scripts/stage-cli.ps1 .gitignore docs CONTRIBUTING.md
git commit -m "feat(app): bundle the CLI next to the tray app"
```

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
- Modify: `app/src-tauri/src/commands.rs`（三条命令）、`app/src-tauri/src/lib.rs`（注册）
- Modify: `crates/cli/src/main.rs`（`start` 子命令、没运行时的提示）、`crates/cli/src/mcp.rs`（只报错、不启动的提示）、`crates/cli/tests/mcp_tools.rs`
- Modify: `app/src/lib/types.ts`、`app/src/lib/backend.ts`、`app/src/lib/api.ts`、`app/src/lib/mock.ts`、`app/src/lib/mock-fixtures.ts`、`app/src/hooks/queries.ts`、`app/src/pages/Settings.tsx`、`app/src/pages/Settings.test.tsx`、五个翻译文件
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 4.1、10.1、10.3、12 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 5 节）、`.github/CODEOWNERS`（`/crates/core/src/cli_path.rs`）

**Interfaces:**
- Consumes（M2）：`commands.rs` 的 `cli_path() -> Option<PathBuf>`、`CmdResult<T>`、`error(e)`、`host(&state) -> CmdResult<Host>`；`Host::env(&self) -> ExecEnv`；`issuerelay_core::env::{ExecEnv, EnvSource}`（`path: Option<String>`、`path_source: EnvSource::LoginShell`）；`paths::APP_ID`、`paths::APP_EXECUTABLE`（Task 1）；`ipc::client::{call, NotRunning}`；`crates/cli/src/main.rs` 的 `Cli`、`Command`、`request`、`write_line`、`fail`、`run`；`format::error_line`、`format::clean`；`mcp::ControlServer::forward`；前端 `Backend`、`tauriBackend`、`call`、`createMockBackend`、`queryKeys`、`useAction(fn, invalidate, failureKey)`（返回 TanStack 的 `useMutation` 结果）、`renderWithProviders`、`Settings` 页的「系统」分组、shadcn `Switch`。
- Produces（core `issuerelay_core::cli_path`）：
  - `pub const SHELL_LINE: &str = r#"export PATH="$HOME/.local/bin:$PATH""#`、`pub const MAX_ENV_CHARS: usize = 32_767`
  - `pub enum AppLocation { Installed, NotInstalled, Development }`；`pub fn bundle_location(exe: &Path, read_only: bool) -> AppLocation`；`pub fn app_location(exe: &Path, read_only: bool, debug: bool) -> AppLocation`；`pub fn on_read_only_volume(path: &Path) -> bool`；`pub fn running_app_location() -> AppLocation`（Task 3 的更新也用它）
  - `pub enum LinkSlot { Empty, Link(PathBuf), Other }`、`pub enum LinkState { Missing, Ours, Stale(PathBuf), Foreign }`、`pub fn link_state(slot: &LinkSlot, cli: &Path) -> LinkState`、`pub fn link_path(home: &Path) -> PathBuf`、`pub fn unix_path_has(dir: &Path, path_var: &str, home: &Path) -> bool`
  - `pub fn expand_vars(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String`、`pub fn windows_path_has(dir: &str, value: &str, lookup: …) -> bool`、`pub fn windows_path_with(dir: &str, value: &str, lookup: …) -> Result<Option<String>, PathTooLong>`、`pub fn windows_path_without(dir: &str, value: &str, lookup: …) -> Option<String>`、`pub struct PathTooLong(pub usize)`、`pub fn decode_reg_string(bytes: &[u8]) -> Result<String, FromUtf16Error>`、`pub fn encode_reg_string(text: &str) -> Vec<u8>`
  - `pub enum CliPathStatus`（serde `tag = "kind"`：`{"kind":"off"}`、`{"kind":"on","shell_line":null|"…"}`、`{"kind":"stale","target":"…"}`、`{"kind":"blocked","path":"…"}`、`{"kind":"unavailable","reason":"missing_cli"|"not_installed"|"development"}`）、`pub enum CliUnavailable { MissingCli, NotInstalled, Development }`
  - `pub struct CliSetup { pub cli: Option<PathBuf>, pub location: AppLocation, pub home: PathBuf, pub shell_path: Option<String> }`、`CliSetup::for_running_app(cli: Option<PathBuf>, shell_path: Option<String>) -> anyhow::Result<CliSetup>`；`pub fn status(setup: &CliSetup) -> anyhow::Result<CliPathStatus>`、`pub fn install(…)`、`pub fn remove(…)`（同签名）
- Produces（应用命令，前端 camelCase）：`cli_on_path` / `install_cli_on_path` / `remove_cli_from_path`，无参数，都返回 `CliPathStatus`；`Backend.cliOnPath()`、`installCliOnPath()`、`removeCliFromPath()`；`queryKeys.cliPath`、`useCliPath()`、`useInstallCliOnPath()`、`useRemoveCliFromPath()`；TS 类型 `CliPathStatus`、`CliUnavailable`；假后端常量 `DEMO_SHELL_LINE`。
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

- [ ] **Step 3: 应用命令**

`app/src-tauri/src/commands.rs` 的 `use issuerelay_core::{…}` 里加 `cli_path::{CliPathStatus, CliSetup}`、`env::EnvSource`，文件末尾（`request_quit` 之后）加：

```rust
/// What the PATH switch needs: the CLI next to the app, where the app runs from, and the login
/// shell's PATH the host resolved (only a PATH read from the login shell tells whether
/// `~/.local/bin` is on it).
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

（模块名 `cli_path` 与 M2 的函数 `cli_path()` 同名不冲突——一个在类型命名空间、一个在值命名空间——但为了好读，调用一律写全路径 `issuerelay_core::cli_path::status`。）`lib.rs` 的 `generate_handler!` 列表在 `commands::request_quit,` 之后加 `commands::cli_on_path, commands::install_cli_on_path, commands::remove_cli_from_path,`（命令共 36 条）。

Run: `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: PASS。

- [ ] **Step 4: 命令行的失败测试**

`crates/cli/src/main.rs` 测试模块的 `use` 补 `std::{cell::Cell, collections::VecDeque, time::Duration}` 与 `issuerelay_core::ipc::client::NotRunning`，加：

```rust
    #[test]
    fn start_is_a_subcommand_that_sends_nothing_itself() {
        assert!(matches!(parse(&["start"]).command, Command::Start));
        // It is no panel-only action either (`no_subcommand_marks_done_retries_or_cancels`).
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

`app/src/pages/Settings.test.tsx` 加（`import type { CliPathStatus } from "@/lib/types";`）：

```tsx
test("the command line goes on PATH from the system group and the shell line is shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const install = vi.spyOn(backend, "installCliOnPath");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Command line on PATH" }));
  expect(install).toHaveBeenCalledTimes(1);
  expect(await screen.findByText('export PATH="$HOME/.local/bin:$PATH"')).toBeInTheDocument();
  expect(screen.getByText(/Add this line to your shell profile/)).toBeInTheDocument();
});

test("a file IssueRelay did not make blocks the switch and is named", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "blocked", path: "~/.local/bin/issuerelay" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText(/~\/\.local\/bin\/issuerelay already exists and was not made by IssueRelay/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeDisabled();
});

test("from the disk image the switch explains what to do first", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "unavailable", reason: "not_installed" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText(/Move it to Applications first/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeDisabled();
});
```

Run: `pnpm --dir app test`
Expected: 失败（`installCliOnPath` 等不存在）。

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

`app/src/lib/backend.ts` 的 `Backend` 在 `setAutostart` 之后加 `cliOnPath(): Promise<CliPathStatus>;`、`installCliOnPath(): Promise<CliPathStatus>;`、`removeCliFromPath(): Promise<CliPathStatus>;`（类型导入补 `CliPathStatus`）。`api.ts` 的 `tauriBackend` 加：

```ts
  cliOnPath: () => call("cli_on_path"),
  installCliOnPath: () => call("install_cli_on_path"),
  removeCliFromPath: () => call("remove_cli_from_path"),
```

`mock-fixtures.ts` 加 `export const DEMO_SHELL_LINE = 'export PATH="$HOME/.local/bin:$PATH"';`（与 core `SHELL_LINE` 相同）。`mock.ts` 在 `let autostart = false;` 之后加 `let cliPath: CliPathStatus = { kind: "off" };`，返回对象在 `setAutostart` 之后加：

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

`hooks/queries.ts`：`queryKeys` 加 `cliPath: ["cliPath"] as const`；加 `useCliPath()`（`useQuery({ queryKey: queryKeys.cliPath, queryFn: () => backend.cliOnPath() })`）、`useInstallCliOnPath()` 与 `useRemoveCliFromPath()`（`useAction(() => backend.installCliOnPath(), [queryKeys.cliPath], "actions.failed.cli_path")`，移除同理）。

`pages/Settings.tsx` 的「系统」分组在开机自启下面加一行（`useCliPath`、两个变更钩子）：

- `Switch`，`aria-label`/标签 `settings.cli.title`，`checked = status.kind === "on"`；打开调 `installCliOnPath`，关上调 `removeCliFromPath`；`blocked`、`unavailable`、读取中或变更进行中时 `disabled`。
- 标签下一行说明 `settings.cli.description`。
- 按状态再显示一行：`on` 且 `shell_line` 为空 → `settings.cli.on_new_terminal`；`on` 且有 `shell_line` → `settings.cli.shell_line` 加一个等宽的 `<code>`（内容为 `shell_line`）与「复制」按钮（`common.copy`，`navigator.clipboard.writeText`）；`stale` → `settings.cli.stale`（`target`）；`blocked` → `settings.cli.blocked`（`path`）；`unavailable` → `settings.cli.unavailable.<reason>`；`useCliPath` 失败 → `ErrorState`（标题 `settings.cli.load_failed`，带重试）。路径与 `shell_line` 一律作为文本节点渲染。

翻译键（`en` / `zh-CN` 如下；`zh-TW`、`ja`、`ko` 按 M2 Task 2 的写法同步翻译，`{{…}}` 与命令文本原样保留）：

| 键 | en | zh-CN |
|---|---|---|
| `settings.cli.title` | Command line on PATH | 把命令行加入 PATH |
| `settings.cli.description` | Lets you type `issuerelay` in a terminal. No administrator rights needed. | 之后在终端里可以直接输入 `issuerelay`，不需要管理员权限。 |
| `settings.cli.on_new_terminal` | On. Open a new terminal to use it. | 已加入。新开一个终端即可使用。 |
| `settings.cli.shell_line` | Add this line to your shell profile (for zsh, ~/.zshrc), then open a new terminal: | 把下面这一行加到 shell 配置文件（zsh 是 ~/.zshrc），再新开一个终端： |
| `settings.cli.stale` | The link points at another copy of IssueRelay ({{target}}). Turn this on to point it here. | 链接指向另一份 IssueRelay（{{target}}）。打开开关即改为指向这一份。 |
| `settings.cli.blocked` | {{path}} already exists and was not made by IssueRelay. Remove or rename it, then try again. | {{path}} 已存在，且不是 IssueRelay 建的。删除或改名后再试。 |
| `settings.cli.unavailable.missing_cli` | The command-line program is not next to the app. In a development build, run cargo build -p issuerelay-cli first. | 应用旁边没有命令行程序。开发版请先运行 cargo build -p issuerelay-cli。 |
| `settings.cli.unavailable.not_installed` | IssueRelay runs from the disk image or a temporary copy. Move it to Applications first. | IssueRelay 正从磁盘映像或临时副本运行。请先把它移到「应用程序」。 |
| `settings.cli.unavailable.development` | Not available in a development build. | 开发版不提供这一项。 |
| `settings.cli.load_failed` | Could not read the PATH setting | 读不到 PATH 设置 |
| `actions.failed.cli_path` | Could not change the command line on PATH | 没能修改「把命令行加入 PATH」 |

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS（含缺键守卫与五种语言键对齐检查）。浏览器里看设置页浅色与深色各一次。

- [ ] **Step 8: 文档与提交**

- 设计第 4.1 节「命令行小程序」一条改为：「…托盘程序没在运行时给出提示：命令行指向 `issuerelay start`，它启动托盘程序（macOS 经 `open -b com.sunkailun.issuerelay`，Windows 启动同目录的 `issuerelay-app.exe`），已在运行就只说明，并等到宿主应答；MCP 服务只报错，请 AI 转告用户打开应用，从不自己启动（AI 不能静默启动有完全权限的程序）。」第 10.1 节的子命令列表加 `start`；第 10.3 节补「宿主没运行时工具返回错误（`isError`），文字里请 AI 转告用户打开 IssueRelay」；第 12 节「把命令行加入 PATH」写实际行为：「macOS 在 `~/.local/bin/issuerelay` 建指向应用包内命令行程序的链接（不覆盖别人的文件，旧链接自动改指向；`~/.local/bin` 不在登录 shell 的 PATH 里时面板给出要加的一行，不改 shell 配置文件；从磁盘映像运行时不建）；Windows 把安装目录加进用户级 `Path`（保持 `REG_EXPAND_SZ`，不分大小写去重，广播 `WM_SETTINGCHANGE`，移除时只删这一项）。都不需要管理员。注册 MCP 控制服务用的是命令行程序的绝对路径，不依赖 PATH。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 把命令行加入 PATH、应用安装位置 | crates/core/src/cli_path.rs |`；`.github/CODEOWNERS` 加 `/crates/core/src/cli_path.rs   @sunkailun1992`（会改用户的 PATH 与注册表）。

Run: 六条验证命令
Expected: 全部通过。

```bash
git add crates app docs .github Cargo.lock app/pnpm-lock.yaml
git commit -m "feat: put the CLI on PATH from the settings and start the tray app from the CLI"
```

---

### Task 3: 自动更新

设计第 12 节（Tauri updater，更新包用单独的签名密钥签名，私钥只放在 GitHub Actions secrets，从 GitHub Releases 拉取）。调研结论（`tauri-plugin-updater` 2.13.3、`tauri-cli` 2.12.2 的源码）：

- `plugins.updater.pubkey` 必填，值是 `.pub` 文件那一行 base64 的内容（不接受文件路径）；配置缺了它，插件初始化就失败，应用起不来——所以公钥必须在注册插件的同一个提交里写进 `tauri.conf.json`。
- `bundle.createUpdaterArtifacts: true` 让打包器签名更新包，没有私钥时打包失败，所以基础配置保持 `false`，发布工作流用 `--config src-tauri/tauri.updater.conf.json` 打开。产物：macOS `IssueRelay.app.tar.gz` 与 `.sig`；Windows 安装包本身（`-setup.exe`、`.msi`）与各自的 `.sig`。
- 私钥经环境变量 `TAURI_SIGNING_PRIVATE_KEY`（内容或路径）给构建，密码 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；设置了 `CI` 而没有密码变量时用空密码。签名私钥与公钥不配对时 `tauri build` 只警告、不失败（Task 4 用脚本核对）。`tauri-cli` 2.12.0 起签名的可信注释里记录应用版本，`requireSignedVersion: true` 要求它与清单宣布的版本一致，拦住「高版本号配旧的合法包」的篡改。
- `Updater::check()`：远端版本不比当前新（语义化版本比较，`0.1.0-rc.1` 小于 `0.1.0`）时返回 `Ok(None)`；清单没有本平台条目时返回 `TargetsNotFound` 错误（即使版本不新）；拿不到有效清单是 `ReleaseNotFound` 或网络、解析错误。`Update::download` 先验签再返回字节，验签失败（`Minisign` 等）什么都不留；`Update::install(bytes)` 在 macOS 原地替换应用包（2.13.2 起失败不会删掉已装的应用），在 Windows 启动安装程序（NSIS 带 `/P /UPDATE /R`）后立即退出本进程。
- 重启：`AppHandle::restart()` 在主线程上调用时跳过 `Exit` 事件，单实例插件的锁可能还没释放，新实例会把参数交给旧实例后自己退出，结果程序没在运行；在其他线程调用、或用 `request_restart()` 时先发 `Exit`。本任务一律在后台线程用 `request_restart()`。macOS 上新实例经 Launch Services 从应用包启动。
- `tauri-plugin-process` 只给前端的 `relaunch()` 用；本计划只在 Rust 侧调用，不加它（草稿里的这个依赖删掉）。更新插件只在 Rust 侧用，`capabilities/default.json` 不加权限。

**Step 0 由主智能体（控制者）在派发本任务之前执行**：在维护者本机生成更新签名密钥对，并把公钥写进 `tauri.conf.json`；不交给子智能体，不在云端会话做。实现者从 Step 1 开始，`app/src-tauri/tauri.conf.json` 里已经有 `plugins.updater`。

**Files:**
- Modify: `app/src-tauri/tauri.conf.json`（Step 0 写入 `plugins.updater`）
- Create: `app/src-tauri/tauri.updater.conf.json`
- Modify: `app/src-tauri/Cargo.toml`、`Cargo.lock`（`tauri-plugin-updater`、`semver`；`chrono` 若还不是普通依赖就加上，带 `serde` 特性）
- Create: `app/src-tauri/src/update.rs`
- Modify: `app/src-tauri/src/lib.rs`、`app/src-tauri/src/quit.rs`、`app/src-tauri/src/events.rs`、`app/src-tauri/src/commands.rs`、`app/src-tauri/src/i18n.rs`（`USED_KEYS`）、`app/src-tauri/tests/packaging.rs`
- Modify: `crates/core/src/config.rs`（`auto_update` 的文档注释）
- Modify: `app/src/lib/{types,backend,api,mock,mock-fixtures,errors}.ts`、`app/src/hooks/{queries,useEngineEvents}.ts`、`app/src/components/QuitDialog.tsx`、`app/src/components/QuitDialog.test.tsx`、`app/src/components/layout/AppShell.tsx`、`app/src/pages/Settings.tsx`、`app/src/pages/Settings.test.tsx`、五个翻译文件
- Create: `app/src/components/UpdateBanner.tsx`、`app/src/components/UpdateBanner.test.tsx`
- Modify: `scripts/check-secrets.sh`（拦截更新签名私钥）
- Create: `docs/release/updater-key.md`
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 12 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 5、7 节）、`docs/ai-coding/TESTING_SPEC.md`（假实现清单）、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 5 节）

**Interfaces:**
- Consumes（M2）：`quit.rs` 的 `QuitState`、`QuitEvent::Requested { host_ready, running }`、`QuitStep::{Ask, Exit, …}`、`next`、`handle`、`request`、`on_run_event`（Exit 走 `app.exit(0)`；`ExitRequested` 带退出码时放行）；`events::QUIT_REQUESTED`；`state::AppState::{host, try_host}`；`logging::debug_failed`；`i18n::{Translator, USED_KEYS, resolve_language}`（`Translator::for_settings`、`t`、`t_with`）；`Host::engine().settings().auto_update`、`Host::engine().status().running`；`commands.rs` 的 `CmdResult`；前端 `Backend`、`BackendError`、`ERROR_CODES`、`describeError`、`useAction`、`queryKeys`、`useAppInfo`、`formatRelative`、`DetailLine`、`ErrorState`、`subscription`、`useEngineEvents`、`QuitDialog`（M2 的 `quit.title`、`quit.body`、`quit.wait`、`quit.stop`、`quit.cancel`、`quit.waiting`）、`AppShell`、Settings 的「系统」分组与 `auto_update` 开关、`renderWithProviders`、`createMockBackend`；Task 2 的 `issuerelay_core::cli_path::{AppLocation, running_app_location}`。
- Produces（Rust）：
  - `update::FIRST_CHECK`（60 s）、`update::CHECK_EVERY`（6 h）
  - `pub enum FailureCode { Unreachable, NoPlatform, BadSignature, Unavailable, InstallFailed, Other }`（serde snake_case）、`FailureCode::error_code(self) -> &'static str`（`update_unreachable`、`update_no_platform`、`update_bad_signature`、`update_unavailable`、`update_install_failed`、`update_failed`）、`pub struct Failure { pub code: FailureCode, pub detail: String }`、`Failure::new(code, detail)`
  - `pub enum UpdateStatus { Unchecked, UpToDate { checked_at }, Available { version, notes }, Failed(Failure) }`（serde `tag = "kind"`：`{"kind":"unchecked"}`、`{"kind":"up_to_date","checked_at":"…"}`、`{"kind":"available","version":"0.2.0","notes":null}`、`{"kind":"failed","code":"unreachable","detail":"…"}`）
  - `pub trait UpdateSource`（`type Offer`、`version`、`notes`、`check`、`download`）、`pub struct PluginSource(pub AppHandle)`、`pub struct Pending<O> { pub offer: Arc<O>, pub bytes: Vec<u8> }`、`pub struct Updates<S>`：`new(source, current)`、`status()`、`check()`、`prepare()`、`pending_version()`、`take_pending()`、`clear_pending()`、`should_notify(version)`；`pub type AppUpdates = Updates<PluginSource>`；`pub fn is_newer(current: &str, offered: &str) -> bool`、`pub fn failure_code(&tauri_plugin_updater::Error) -> FailureCode`
  - `pub fn update::{pending, take_pending, clear_pending}(app: &AppHandle)`、`pub async fn update::check(app) -> UpdateStatus`、`pub async fn update::install(app) -> Result<(), Failure>`、`pub fn update::install_and_restart(app, pending)`、`pub fn update::spawn_checker(app: AppHandle)`
  - `quit`：`pub enum QuitPurpose { Quit, Update }`、`pub struct QuitPrompt { pub running: usize, pub purpose: QuitPurpose }`（`quit-requested` 的载荷从「运行中的条数」改为 `{"running":2,"purpose":"quit"|"update"}`）、`pub fn request_update(app: &AppHandle)`；`request` 改为先丢掉已下载的更新
  - `events::UPDATE_STATUS = "update-status"`（载荷：`UpdateStatus`）
  - 命令：`update_status() -> UpdateStatus`（立即返回）、`check_update() -> UpdateStatus`、`install_update() -> Result<(), String>`（失败时是 `FailureCode::error_code`）；命令共 39 条
- Produces（前端）：类型 `UpdateStatus`、`UpdateFailureCode`、`QuitPurpose`、`QuitPrompt`；`Backend.updateStatus()`、`checkUpdate()`、`installUpdate()`、`onUpdateStatus(listener)`，`onQuitRequested(listener: (prompt: QuitPrompt) => void)`；`ERROR_CODES` 加六个 `update_*`；`queryKeys.update`、`useUpdateStatus()`、`useCheckUpdate()`、`useInstallUpdate()`；`updateFailureKey(code)`（`lib/errors.ts`）；`UpdateBanner()`。
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

`app/src-tauri/src/quit.rs` 的测试模块加：

```rust
    #[test]
    fn the_quit_prompt_tells_the_panel_what_follows() {
        let quit = QuitPrompt { running: 2, purpose: QuitPurpose::Quit };
        assert_eq!(serde_json::to_value(quit).unwrap(), serde_json::json!({"running": 2, "purpose": "quit"}));
        let update = QuitPrompt { running: 1, purpose: QuitPurpose::Update };
        assert_eq!(serde_json::to_value(update).unwrap(), serde_json::json!({"running": 1, "purpose": "update"}));
    }
```

（「安装并重启」与退出走同一个 `next`：M2 的 `quitting_asks_only_while_runs_are_in_progress`、`waiting_can_still_become_stopping_now` 等测试不改，它们就是 Review Focus 1 在状态机一层的证据。）

新建 `app/src-tauri/src/update.rs`，先只放测试模块：

```rust
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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

        fn notes(_offer: &FakeOffer) -> Option<&str> {
            Some("Faster checks")
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
        assert_eq!(updates.status(), UpdateStatus::Available { version: "0.2.0".into(), notes: Some("Faster checks".into()) });
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
    fn the_panel_reads_the_status_and_the_error_codes() {
        let failed = UpdateStatus::Failed(Failure::new(FailureCode::BadSignature, "The signature verification failed"));
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            serde_json::json!({"kind": "failed", "code": "bad_signature", "detail": "The signature verification failed"})
        );
        let available = UpdateStatus::Available { version: "0.2.0".into(), notes: None };
        assert_eq!(serde_json::to_value(available).unwrap(), serde_json::json!({"kind": "available", "version": "0.2.0", "notes": null}));
        assert_eq!(FailureCode::Unreachable.error_code(), "update_unreachable");
        assert_eq!(FailureCode::Other.error_code(), "update_failed");
        // A detail with control characters arrives cleaned.
        assert_eq!(Failure::new(FailureCode::Other, "bad\u{1b}[2Jline\n").detail, "bad [2Jline");
    }
}
```

Run: `cargo test -p issuerelay-app`
Expected: 编译失败（`update` 模块与 `QuitPrompt` 不存在；配置测试缺 `tauri.updater.conf.json`）。

- [ ] **Step 2: 依赖与配置文件**

```bash
cargo add -p issuerelay-app tauri-plugin-updater semver
cargo add -p issuerelay-app chrono --features serde   # skip if chrono is already a normal dependency
```

（解析到的 `tauri-plugin-updater` 应是 2.13.x；版本、`Update` 的字段与 `Error` 的变体以它的 docs.rs 为准，与下面的写法不同时照文档改，在报告里列出。）

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

在测试模块上方写：

```rust
//! Automatic updates (design 12). The updater plugin reads
//! `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json` a minute
//! after the start and every six hours while `settings.auto_update` is on, and whenever the
//! settings page asks. Only a newer version is offered, and only a payload that verifies against
//! the public key in tauri.conf.json (and names the announced version, `requireSignedVersion`)
//! is ever kept. Installing waits for the user's "install and restart", which goes through the
//! quit flow (`quit::request_update`): with runs in progress the panel asks the same "wait for
//! completion / stop now" as quitting. A failed check, a missing platform, a bad signature or a
//! version that is not newer is shown and logged and changes nothing else: monitoring goes on.
use std::{
    future::Future,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use chrono::{DateTime, Utc};
use issuerelay_core::{
    cli_path::{AppLocation, running_app_location},
    text::clean_text,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::{events, i18n, logging, quit, state::AppState};

/// The first background check after the start.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The background checks after the first.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Release notes are cut to this many characters; the banner shows a few lines of them.
const MAX_NOTES_CHARS: usize = 4000;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Not checked since the start.
    Unchecked,
    /// The last check found nothing newer than the running version.
    UpToDate { checked_at: DateTime<Utc> },
    /// A newer version is offered; `notes` is the release text, shown as plain text.
    Available { version: String, notes: Option<String> },
    /// The check failed; nothing was installed.
    Failed(Failure),
}

/// Where updates come from: the updater plugin in the app, a fake in the tests.
pub trait UpdateSource: Send + Sync + 'static {
    /// An update a check found; kept until it is downloaded and installed.
    type Offer: Send + Sync + 'static;
    fn version(offer: &Self::Offer) -> &str;
    fn notes(offer: &Self::Offer) -> Option<&str>;
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

    /// Asks the source and returns this check's outcome. Only a newer version becomes an offer.
    /// A failed check is logged and leaves a version already offered on show.
    pub async fn check(&self) -> UpdateStatus {
        let outcome = match self.source.check().await {
            Ok(Some(offer)) if is_newer(&self.current, S::version(&offer)) => {
                let status = UpdateStatus::Available {
                    version: S::version(&offer).to_string(),
                    notes: S::notes(&offer).map(|notes| notes.chars().take(MAX_NOTES_CHARS).collect()),
                };
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
pub struct PluginSource(pub AppHandle);

impl UpdateSource for PluginSource {
    type Offer = tauri_plugin_updater::Update;

    fn version(offer: &Self::Offer) -> &str {
        &offer.version
    }

    fn notes(offer: &Self::Offer) -> Option<&str> {
        offer.body.as_deref()
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

/// The update state the app manages.
pub type AppUpdates = Updates<PluginSource>;

/// Whether a verified update waits for the quit flow.
pub fn pending(app: &AppHandle) -> bool {
    app.try_state::<AppUpdates>().is_some_and(|updates| updates.pending_version().is_some())
}

pub fn take_pending(app: &AppHandle) -> Option<Pending<tauri_plugin_updater::Update>> {
    app.try_state::<AppUpdates>().and_then(|updates| updates.take_pending())
}

pub fn clear_pending(app: &AppHandle) {
    if let Some(updates) = app.try_state::<AppUpdates>() {
        updates.clear_pending();
    }
}

/// Checks now, records the outcome and tells an open panel (`update-status` carries what the
/// panel should keep showing).
pub async fn check(app: &AppHandle) -> UpdateStatus {
    let updates = app.state::<AppUpdates>();
    let outcome = updates.check().await;
    logging::debug_failed(app.emit(events::UPDATE_STATUS, updates.status()), "telling the panel about the update check");
    outcome
}

/// "Install and restart": refuses where the app cannot replace itself, downloads and verifies the
/// offered version, then hands over to the quit flow, which asks about runs in progress exactly
/// as quitting does and installs once the host has stopped.
pub async fn install(app: &AppHandle) -> Result<(), Failure> {
    match running_app_location() {
        AppLocation::Installed => {}
        AppLocation::NotInstalled => {
            return Err(Failure::new(FailureCode::Unavailable, "IssueRelay runs from the disk image or a temporary copy"));
        }
        AppLocation::Development => {
            return Err(Failure::new(FailureCode::Unavailable, "a development build does not install updates"));
        }
    }
    app.state::<AppUpdates>().prepare().await?;
    quit::request_update(app);
    Ok(())
}

/// Installs a verified download once the quit flow has stopped the host, and restarts into it.
/// macOS swaps the app bundle in place and relaunches it; on Windows `install` starts the NSIS
/// installer (`/P /UPDATE /R`), which ends this process and starts the new version itself. If
/// installing fails the app restarts as it is, so monitoring resumes, and a notification says
/// why.
pub fn install_and_restart(app: &AppHandle, pending: Pending<tauri_plugin_updater::Update>) {
    let app = app.clone();
    // Off the main thread: `request_restart` then delivers the exit event first, so the
    // single-instance lock is released before the new instance starts.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = pending.offer.install(&pending.bytes) {
            tracing::error!(error = %e, version = %pending.offer.version, "installing the update failed; restarting the current version");
            notify(&app, "app.name", None, "notify.update_install_failed");
        }
        app.request_restart();
    });
}

/// A translator in the user's language: the settings' when the host runs, the system's otherwise.
fn translator(app: &AppHandle) -> i18n::Translator<'static> {
    match app.state::<AppState>().try_host() {
        Some(host) => i18n::Translator::for_settings(&host.engine().settings()),
        None => i18n::Translator::new(i18n::resolve_language("system", sys_locale::get_locale().as_deref())),
    }
}

/// Shows a system notification whose title and body are translation keys.
fn notify(app: &AppHandle, title_key: &str, version: Option<&str>, body_key: &str) {
    let t = translator(app);
    let vars: Vec<(&str, &str)> = version.map(|version| ("version", version)).into_iter().collect();
    let title = t.t_with(title_key, &vars);
    if let Err(e) = app.notification().builder().title(title).body(t.t(body_key)).show() {
        tracing::warn!(error = %e, "could not show the update notification");
    }
}

/// Whether the background check runs: automatic checks on, and the host started.
async fn auto_check_enabled(app: &AppHandle) -> bool {
    match app.state::<AppState>().host().await {
        Ok(host) => host.engine().settings().auto_update,
        Err(_) => false,
    }
}

/// The background check: a minute after the start, then every six hours, while
/// `settings.auto_update` is on. A newer version notifies once and shows the banner. Development
/// builds check only when asked.
pub fn spawn_checker(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if auto_check_enabled(&app).await
                && let UpdateStatus::Available { version, .. } = check(&app).await
                && app.state::<AppUpdates>().should_notify(&version)
            {
                notify(&app, "notify.update_available", Some(&version), "notify.update_available_body");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}
```

（`AppHandle::request_restart` 在 tauri 2.12.3 的文档里有；解析到的版本没有它时，改为 `tauri_plugin_single_instance::destroy(&app); app.restart();`——先释放单实例锁再重启——并在报告里说明。`tauri_plugin_updater::Update` 若不是 `Sync`，`Arc<Update>` 编译不过，就把 `Pending.offer` 与 `Updates.offer` 改为直接存 `Update`（它实现 `Clone`），在报告里说明。应用 crate 的 `tokio` 依赖（M2 的 `state.rs` 用它的 `sync`）还需要 `time` 特性给 `tokio::time::sleep`：没有就 `cargo add -p issuerelay-app tokio --features sync,time`。）

- [ ] **Step 4: 接上退出流程、事件、命令与插件**

`events.rs` 加 `pub const UPDATE_STATUS: &str = "update-status";`（文档注释：`/// The update state the panel should show (an \`update::UpdateStatus\`).`）。

`quit.rs`：

- `use` 加 `serde::Serialize`，`crate::{…}` 里加 `update`。
- 在 `QuitStep` 之后加：

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
```

- `handle` 里两个分支改为：

```rust
        QuitStep::Ask(running) => {
            window::show(app);
            let purpose = if update::pending(app) { QuitPurpose::Update } else { QuitPurpose::Quit };
            logging::debug_failed(app.emit(events::QUIT_REQUESTED, QuitPrompt { running, purpose }), "asking the panel about quitting");
        }
```

```rust
        QuitStep::Exit => match update::take_pending(app) {
            Some(pending) => update::install_and_restart(app, pending),
            None => app.exit(0),
        },
```

- `request` 换成三个函数：

```rust
/// The tray's Quit, Cmd+Q and the system's exit request: a quit, so an update downloaded for
/// "install and restart" is dropped and quitting never installs it.
pub fn request(app: &AppHandle) {
    update::clear_pending(app);
    ask(app);
}

/// "Install and restart", once the update is downloaded and verified: with runs in progress the
/// same question as quitting; once the host has stopped, `handle` installs instead of exiting.
pub fn request_update(app: &AppHandle) {
    ask(app);
}

/// Starts quitting with the host as it is now.
fn ask(app: &AppHandle) {
    let host = app.state::<AppState>().try_host();
    let running = host.as_ref().map_or(0, |host| host.engine().status().running);
    handle(app, QuitEvent::Requested { host_ready: host.is_some(), running });
}
```

- 模块文档补一句：「"Install and restart" (`request_update`) goes through the same steps; when they end, a verified update waiting in `update` is installed instead of exiting.」

（`on_run_event` 不变：`request_restart()` 发出的 `ExitRequested` 带退出码，照样放行。）

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

`lib.rs`：`pub mod update;`；`tauri::Builder::default()` 链里在 `tauri_plugin_dialog::init()` 之后加 `.plugin(tauri_plugin_updater::Builder::new().build())`；`.setup` 里第一行加

```rust
            app.manage(update::AppUpdates::new(update::PluginSource(app.handle().clone()), env!("CARGO_PKG_VERSION")));
```

并在 `tauri::async_runtime::spawn(startup::start(…))` 之后加 `update::spawn_checker(app.handle().clone());`；`generate_handler!` 加 `commands::update_status, commands::check_update, commands::install_update,`。

`i18n.rs` 的 `USED_KEYS` 末尾加 `"app.name", "notify.update_available", "notify.update_available_body", "notify.update_install_failed",`（翻译文件 Step 6 补）。

Run: `cargo test -p issuerelay-app` 与 `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: PASS（`USED_KEYS` 的检查要等 Step 6 补好翻译才通过；在那之前它是唯一失败的测试）。

- [ ] **Step 5: 前端的失败测试**

`app/src/components/UpdateBanner.test.tsx`：

```tsx
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { UpdateStatus } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { UpdateBanner } from "./UpdateBanner";

const AVAILABLE: UpdateStatus = { kind: "available", version: "0.2.0", notes: "Faster checks\u001b[31m\nSecond line" };

test("a newer version shows the banner, and installing is the backend's job", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  const install = vi.spyOn(backend, "installUpdate");
  const quit = vi.spyOn(backend, "quit");
  const requestQuit = vi.spyOn(backend, "requestQuit");
  await renderWithProviders(<UpdateBanner />, { backend });
  expect(await screen.findByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
  expect(screen.getByText(/Faster checks/).textContent).not.toContain("\u001b");
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
  const { unmount } = await renderWithProviders(<UpdateBanner />, { backend });
  await act(async () => {});
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
  await renderWithProviders(
    <>
      <EngineEventsBridge />
      <UpdateBanner />
    </>,
    { backend },
  );
  await waitFor(() => expect(listen).toHaveBeenCalled());
  // Let the first read (up to date) settle, so it cannot overwrite the event.
  await act(async () => {});
  await act(async () => announce(AVAILABLE));
  expect(await screen.findByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
});
```

`app/src/components/QuitDialog.test.tsx` 换成（M2 的两个测试改用 `QuitPrompt`，加一个「安装更新」的测试）：

```tsx
import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import type { QuitPrompt } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { QuitDialog } from "./QuitDialog";

async function openWith(prompt: QuitPrompt) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (prompt: QuitPrompt) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn().mockResolvedValue(undefined);
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask(prompt));
  return backend;
}

test("quitting with runs in progress asks, and waiting can still become stopping", async () => {
  const backend = await openWith({ running: 2, purpose: "quit" });
  expect(screen.getByText(/2 run\(s\) are in progress/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(backend.quit).toHaveBeenCalledWith("wait");
  expect(screen.getByText(/Waiting for the runs to finish, then IssueRelay quits/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenLastCalledWith("stop");
});

test("keep running closes the prompt without quitting", async () => {
  const backend = await openWith({ running: 1, purpose: "quit" });
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  expect(backend.quit).not.toHaveBeenCalled();
  expect(screen.queryByText(/run\(s\) are in progress/)).not.toBeInTheDocument();
});

test("installing an update with runs in progress asks the same question and says it restarts", async () => {
  const backend = await openWith({ running: 1, purpose: "update" });
  expect(screen.getByText(/1 run\(s\) are in progress/)).toBeInTheDocument();
  expect(screen.getByText("IssueRelay restarts to install the update once these runs are settled.")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(backend.quit).toHaveBeenCalledWith("wait");
  expect(screen.getByText("Waiting for the runs to finish, then IssueRelay installs the update and restarts.")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenLastCalledWith("stop");
});
```

`app/src/pages/Settings.test.tsx` 加：

```tsx
test("checking for updates shows the version and the result; a failure is a translated headline with the detail", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.checkUpdate = vi.fn().mockResolvedValue({ kind: "failed", code: "unreachable", detail: "error sending request" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Version 0.1.0")).toBeInTheDocument();
  expect(screen.getByText(/Up to date/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("Could not reach the update server")).toBeInTheDocument();
  expect(screen.getByText("error sending request")).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败（组件、方法与类型不存在）。

- [ ] **Step 6: 前端实现与文案**

`types.ts` 加，并把 M2 的 `onQuitRequested` 载荷类型换掉：

```ts
export type UpdateFailureCode = "unreachable" | "no_platform" | "bad_signature" | "unavailable" | "install_failed" | "other";
export type UpdateStatus =
  | { kind: "unchecked" }
  | { kind: "up_to_date"; checked_at: string }
  | { kind: "available"; version: string; notes: string | null }
  | { kind: "failed"; code: UpdateFailureCode; detail: string };
export type QuitPurpose = "quit" | "update";
export interface QuitPrompt { running: number; purpose: QuitPurpose }
```

`backend.ts`：`ERROR_CODES` 改为 `["not_a_linear_link", "update_unreachable", "update_no_platform", "update_bad_signature", "update_unavailable", "update_install_failed", "update_failed"] as const`；`Backend` 加 `updateStatus(): Promise<UpdateStatus>;`、`checkUpdate(): Promise<UpdateStatus>;`、`installUpdate(): Promise<void>;`、`onUpdateStatus(listener: (status: UpdateStatus) => void): Promise<() => void>;`，并把 `onQuitRequested(listener: (running: number) => void)` 改为 `onQuitRequested(listener: (prompt: QuitPrompt) => void)`。`api.ts`：

```ts
  updateStatus: () => call("update_status"),
  checkUpdate: () => call("check_update"),
  installUpdate: () => call("install_update"),
  onQuitRequested: (listener) => on<QuitPrompt>("quit-requested", listener),
  onUpdateStatus: (listener) => on<UpdateStatus>("update-status", listener),
```

`mock.ts`：`listeners.quit` 的类型改为 `Set<(prompt: QuitPrompt) => void>`，加 `update: new Set<(status: UpdateStatus) => void>()`；`let update: UpdateStatus = { kind: "up_to_date", checked_at: ago(30) };`；返回对象加 `updateStatus: () => reply(() => update)`、`checkUpdate: () => reply(() => update)`、`installUpdate: () => reply(() => undefined)`、`onUpdateStatus: (listener) => subscribe(listeners.update, listener)`（假后端自己不发这个事件；`ago` 从 `mock-fixtures.ts` 导入）。

`lib/errors.ts` 加：

```ts
/** The translation key of an update failure's headline: the same texts as the `update_*` error codes. */
export function updateFailureKey(code: UpdateFailureCode): string {
  return code === "other" ? "errors.update_failed" : `errors.update_${code}`;
}
```

`hooks/queries.ts`：`queryKeys.update = ["update"] as const`；`useUpdateStatus()`（`useQuery({ queryKey: queryKeys.update, queryFn: () => backend.updateStatus() })`）；`useCheckUpdate()` = `useAction(() => backend.checkUpdate(), [queryKeys.update], "actions.failed.check_update")`；`useInstallUpdate()` = `useAction(() => backend.installUpdate(), [], "actions.failed.install_update")`。`hooks/useEngineEvents.ts` 再订阅一个：`subscription(backend.onUpdateStatus((status) => client.setQueryData(queryKeys.update, status)))`。

`components/UpdateBanner.tsx`：

```tsx
import { Download } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useInstallUpdate, useUpdateStatus } from "@/hooks/queries";
import { cleanDisplayLines } from "@/lib/clean";

/**
 * Above every page while a newer version is offered (design 12). "Install and restart" asks the
 * Rust side, which downloads and verifies the update and then goes through the quit flow: the
 * panel itself never quits or restarts. "Later" hides the banner for that version until the
 * panel is opened again.
 */
export function UpdateBanner() {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const install = useInstallUpdate();
  const [dismissed, setDismissed] = useState<string | null>(null);
  const offer = status.data?.kind === "available" ? status.data : null;
  if (offer === null || dismissed === offer.version) return null;
  return (
    <Alert className="mb-4" role="status">
      <Download className="size-4" aria-hidden />
      <AlertTitle>{t("update.available", { version: offer.version })}</AlertTitle>
      <AlertDescription className="space-y-2">
        {offer.notes ? <p className="line-clamp-3 whitespace-pre-line">{cleanDisplayLines(offer.notes)}</p> : null}
        <p className="text-muted-foreground">{t("update.restart_note")}</p>
        <div className="flex gap-2">
          <Button size="sm" disabled={install.isPending} onClick={() => install.mutate()}>
            {install.isPending ? t("update.downloading") : t("update.install")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setDismissed(offer.version)}>
            {t("update.later")}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}
```

`layout/AppShell.tsx`：内容区里 `<Outlet />` 上方放 `<UpdateBanner />`（同在最大宽度与内边距之内）。

`QuitDialog.tsx`：订阅得到的是 `QuitPrompt`，状态从「条数」改存整个 `prompt`；正文 `t("quit.body", { count: prompt.running })`；`prompt.purpose === "update"` 时正文下面加一段 `update.restart_body`，选了等待之后的说明用 `update.waiting` 而不是 `quit.waiting`。按钮与它们调用的 `quit("wait")` / `quit("stop")` 不变。

`pages/Settings.tsx` 的「系统」分组：

- 第一行：`t("update.version", { version })`（`useAppInfo()` 的 `version`）与按钮「检查更新」（`update.check`，`useCheckUpdate`，进行中禁用）。
- 下面一行显示 `const shown = check.data ?? status.data`（本次检查的结果优先，否则是存下的状态）：`up_to_date` → `t("update.up_to_date", { time: formatRelative(shown.checked_at, lang) })`；`available` → `t("update.available", { version })`（安装按钮在页顶的横幅里）；`failed` → 标题 `t(updateFailureKey(shown.code))` 加 `<DetailLine text={shown.detail} />`；`unchecked` 不显示。
- M2 的「自动更新」开关不变，旁注 `settings.auto_update_note` 改为新文案（下表）。

翻译键（`en` / `zh-CN` 如下；`zh-TW`、`ja`、`ko` 同步翻译，`{{…}}` 原样保留）：

| 键 | en | zh-CN |
|---|---|---|
| `update.available` | IssueRelay {{version}} is available | IssueRelay {{version}} 已发布 |
| `update.install` | Install and restart | 安装并重启 |
| `update.downloading` | Downloading and verifying… | 正在下载并校验… |
| `update.later` | Later | 稍后 |
| `update.restart_note` | IssueRelay restarts to install it; if runs are in progress you are asked first. | 安装时 IssueRelay 会重启；有任务在运行时会先问你。 |
| `update.restart_body` | IssueRelay restarts to install the update once these runs are settled. | 这些运行处理完后，IssueRelay 会重启并安装更新。 |
| `update.waiting` | Waiting for the runs to finish, then IssueRelay installs the update and restarts. | 正在等待运行结束，之后 IssueRelay 会安装更新并重启。 |
| `update.version` | Version {{version}} | 版本 {{version}} |
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
| `notify.update_install_failed` | The update could not be installed; IssueRelay restarts with the current version. | 更新没能安装，IssueRelay 用当前版本重启。 |

Run: `pnpm --dir app test && pnpm --dir app lint` 与 `cargo test -p issuerelay-app`
Expected: 全部 PASS（含 `USED_KEYS` 检查、缺键守卫、五种语言对齐）。浏览器里用假后端把 `updateStatus` 临时改成 `available` 看一眼横幅的浅色与深色，看完改回。

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

- [ ] **Step 8: 文档**

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

- 设计第 12 节「自动更新」一条改为：「使用 Tauri updater（`tauri-plugin-updater`）：设置里「自动检查更新」打开时，启动 1 分钟后检查一次、之后每 6 小时一次，也可以在设置里手动检查；清单固定为 `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json`，只接受比当前版本新、用仓库里的公钥验签通过、且签名里记的版本与清单一致的更新包。发现新版本时发一次系统通知、面板顶部显示横幅；安装由用户点「安装并重启」：先下载并验签，有任务在跑时走与退出相同的询问（等待完成 / 立即停止），宿主停下后安装并重启（Windows 交给 NSIS 安装程序）。检查失败、清单缺本平台、验签失败、版本不新只提示，不影响监控。更新包用单独的签名密钥签名，私钥只在维护者本机与 GitHub Actions secrets（`docs/release/updater-key.md`）。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 自动更新（检查、验签、经退出流程安装） | app/src-tauri/src/update.rs |`；第 7 节补：「`quit-requested` 的载荷是 `QuitPrompt { running, purpose }`；`update-status` 的载荷是 `UpdateStatus`，改动时同步 `app/src/lib/types.ts`。」
- `docs/ai-coding/TESTING_SPEC.md` 假实现清单加一行：`| FakeSource | app/src-tauri/src/update.rs 的测试模块 | 更新插件（按脚本给出检查结果与下载结果，并计数下载）；测试不访问网络 |`。
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 5 节登记 `docs/release/`（发布相关说明：更新签名密钥）。

Run: 六条验证命令
Expected: 全部通过。

- [ ] **Step 9: Commit**

```bash
git add app crates/core/src/config.rs scripts/check-secrets.sh docs Cargo.lock app/pnpm-lock.yaml
git commit -m "feat(app): add signed automatic updates from GitHub Releases"
```

提交前确认 `git diff --cached` 里没有 `~/.tauri` 下的任何文件（它们本来就在仓库之外），`bash scripts/check-secrets.sh --strict` 无输出。主智能体在本任务的报告里把 Step 0 的 `gh secret set` 命令交给维护者。

---

### Task 4: 发布工作流

设计第 12 节（推送版本标签时在 macOS 和 Windows 上构建、测试、打包，发布到 GitHub Releases）、16.1 节（维护者发布、升版本号）。调研结论（`tauri-apps/tauri-action` v1.0.0 源码）：

- 用 `@v1`，本计划固定到 v1.0.0 的提交 `1deb371b0cd8bd54025b384f1cd735e725c4060f`。输入 `includeUpdaterJson` 已改名为 `uploadUpdaterJson`（默认 true）；`updaterJsonPreferNsis: true` 让 `latest.json` 的通用键 `windows-x86_64` 指向 NSIS 安装包（默认指向 MSI）；输出 `releaseId`、`artifactPaths`（生成文件的 JSON 数组，不上传时也有）、`appVersion`。
- 没有 `tagName`、`releaseId` 时只构建、不上传；两个矩阵任务各自「先找后建」Release，可能建出两个同标签的草稿；`latest.json` 由每个任务下载、合并、删除旧资产再上传，两个任务交错时可能丢掉一个平台。所以：单独的 `create-release` 任务只建一个草稿并输出 `releaseId`，矩阵任务只经 `releaseId` 上传，并且逐个运行（`max-parallel: 1`），最后的 `verify-release` 再核对一遍。
- `latest.json` 里的 `url` 是 GitHub API 的资产地址（`api.github.com/repos/…/releases/assets/<id>`），更新插件带 `Accept: application/octet-stream` 下载，会被重定向到文件；只有草稿被公开、且它是仓库最新的正式版本时，`…/releases/latest/download/latest.json` 才指向它。
- 通用包 `--target universal-apple-darwin` 写出 `darwin-aarch64`、`darwin-x86_64`、`darwin-universal`（及 `-app` 变体）三组键，都指向同一个 `.app.tar.gz`；需要两个 Apple 目标都装在固定的 1.99.0 工具链上。资产名：`IssueRelay_<版本>_universal.dmg`、`IssueRelay_<版本>_universal.app.tar.gz`（与 `.sig`）、`IssueRelay_<版本>_x64-setup.exe`（与 `.sig`）、`IssueRelay_<版本>_x64_en-US.msi`（与 `.sig`）。
- 上传工作流产物时 `upload-artifact` 的压缩包会丢掉 Unix 权限，所以只传 `.dmg` 与安装包，不传 `.app` 目录。macOS 上 tauri-action 默认设置 `TAURI_BUNDLER_DMG_IGNORE_CI=true`。`macos-latest` 是 macOS 26 arm64，`windows-latest` 是 Windows Server 2025。
- 可选的 secrets 写进 `$GITHUB_ENV` 再交给后面的步骤：只有存在时才设置环境变量（空字符串的 `APPLE_CERTIFICATE` 之类会让打包器去导入一个空证书）。

工作流的形状：

- 触发：推送 `v*` 标签 = 正式发布；`workflow_dispatch` 与改了打包相关文件的 `pull_request` = 试打包（同样的检查、测试和打包，产物存为工作流产物，不建 Release）。GitHub 只在工作流文件已经在默认分支上时才允许手动运行它，所以合入之前的第一次试打包靠 M3 自己的 PR 触发。拉取请求触发的试打包从不使用任何 secret。
- `verify`（ubuntu）：读工作区版本；正式发布时核对标签等于 `v<版本>`、标签提交是 `origin/main` 的祖先；`check-secrets --strict`；记下哪些可选 secrets 齐全（只看是否为空，从不打印值）。
- `test`（macOS、Windows）：六条验证命令。
- `create-release`（ubuntu，只在正式发布时）：同一标签已有 Release 就失败；否则建一个草稿，正文写安装说明链接、签名与自动更新的实际情况，输出 `id`。
- `bundle`（矩阵：macOS 通用包、Windows x64，逐个运行）：装工具链与依赖，`stage-cli`，按 secrets 选配置与签名，`tauri-action` 构建（正式发布时经 `releaseId` 上传并合并 `latest.json`），核对包内容，有更新包时用提交的公钥核对签名，试打包时上传工作流产物。
- `verify-release`（ubuntu，只要草稿建成了就运行）：草稿仍是草稿；资产齐全；有更新包时 `latest.json` 的版本正确、五个平台键都有签名和地址、`windows-x86_64` 指向 NSIS；任何一项不对就失败并把草稿标题改成「incomplete: do not publish」。
- 权限：顶层 `contents: read`；`contents: write` 只给 `create-release`、`bundle`、`verify-release`。

**Files:**
- Create: `.github/workflows/release.yml`
- Create: `scripts/verify-update-signature.mjs`
- Modify: `.github/workflows/ci.yml`（pnpm 与 Node 两步的版本与发布工作流一致）
- Modify: `MAINTAINERS.md`（英文摘要的发布一条、「发布」一节）、`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md`（第 7.1 节）、`docs/ai-coding/AI_CODING_GUIDE.md`（第 10 节）、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（第 6 节）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 12、18 节）、`.github/CODEOWNERS`（`/scripts/stage-cli.*`、`/scripts/verify-update-signature.mjs`）

**Interfaces:**
- Consumes：Task 1 的 `scripts/stage-cli.sh --universal`、`scripts/stage-cli.ps1`、`app/src-tauri/tauri.bundle.conf.json`；Task 3 的 `app/src-tauri/tauri.updater.conf.json`、`plugins.updater.pubkey`；M2 的 `app/package.json`（`packageManager` 字段、`lint`/`test`/`build`/`tauri` 脚本）、`app/pnpm-lock.yaml`、`ci.yml` 的 `test` 任务（M2 Task 1 加的 pnpm、Node 步骤）；`scripts/check-secrets.sh`；`.github/dependabot.yml` 现有的 github-actions 条目（它也更新按 SHA 固定的 action，不用改）。
- Produces：工作流 `release`（任务 `verify`、`test`、`create-release`、`bundle`、`verify-release`；`verify` 的输出 `version`、`updater`、`apple`、`windows`，`create-release` 的输出 `id`）；试打包的工作流产物 `bundles-macos-universal`、`bundles-windows-x64`；`node scripts/verify-update-signature.mjs --version <X.Y.Z> --artifacts '<JSON 数组>' [--pubkey <base64>]`（退出码 0 / 1 / 2）；secrets 名：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（不建）、`APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID`、`WINDOWS_CERTIFICATE`、`WINDOWS_CERTIFICATE_PASSWORD`。

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

- [ ] **Step 2: 写 `.github/workflows/release.yml`**

```yaml
name: release

# A `vX.Y.Z` tag on a commit of main builds the installers and puts them in ONE draft GitHub
# release; a maintainer publishes it by hand (MAINTAINERS.md, "发布"). A manual run and a pull
# request that changes the packaging are dry runs: the same checks, tests and bundles, kept as
# workflow artifacts, and no release. Optional secrets are used only when they are set, and
# never in a pull request.
on:
  push:
    tags: ["v*"]
  pull_request:
    paths:
      - .github/workflows/release.yml
      - scripts/stage-cli.sh
      - scripts/stage-cli.ps1
      - scripts/verify-update-signature.mjs
      - app/src-tauri/tauri.conf.json
      - app/src-tauri/tauri.bundle.conf.json
      - app/src-tauri/tauri.updater.conf.json
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false

env:
  # A release only for a pushed tag; everything else is a dry run.
  RELEASE: ${{ github.event_name == 'push' }}

jobs:
  verify:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.version.outputs.version }}
      updater: ${{ steps.secrets.outputs.updater }}
      apple: ${{ steps.secrets.outputs.apple }}
      windows: ${{ steps.secrets.outputs.windows }}
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
      - name: Read the workspace version
        id: version
        run: |
          version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
          echo "version=$version" >> "$GITHUB_OUTPUT"
          echo "workspace version: $version"
      - name: The tag names the workspace version
        if: env.RELEASE == 'true'
        env:
          VERSION: ${{ steps.version.outputs.version }}
        run: |
          if [[ "$GITHUB_REF_NAME" != "v$VERSION" ]]; then
            echo "::error::tag $GITHUB_REF_NAME does not match the workspace version $VERSION in Cargo.toml"
            exit 1
          fi
      - name: The tagged commit is on main
        if: env.RELEASE == 'true'
        run: |
          git fetch --no-tags origin main
          if ! git merge-base --is-ancestor "$GITHUB_SHA" FETCH_HEAD; then
            echo "::error::$GITHUB_REF_NAME points at a commit that is not on origin/main"
            exit 1
          fi
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
          existing=$(gh api "repos/$GITHUB_REPOSITORY/releases" --paginate --jq ".[] | select(.tag_name == \"$TAG\") | .id")
          if [[ -n "$existing" ]]; then
            echo "::error::a release for $TAG exists already (id $existing); delete that draft first, or release the next version"
            exit 1
          fi
          notes=$(mktemp)
          {
            echo "Install guide / 安装说明: https://github.com/$GITHUB_REPOSITORY/blob/$TAG/docs/install.md"
            echo
            echo "- macOS (Apple silicon and Intel): \`IssueRelay_${VERSION}_universal.dmg\`"
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

  bundle:
    needs: [verify, test, create-release]
    # Dry runs (create-release skipped) and releases (draft created) both bundle.
    if: ${{ !cancelled() && needs.verify.result == 'success' && needs.test.result == 'success' && (needs.create-release.result == 'success' || needs.create-release.result == 'skipped') }}
    permissions:
      contents: write
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
    timeout-minutes: 90
    env:
      VERSION: ${{ needs.verify.outputs.version }}
      UPDATER: ${{ needs.verify.outputs.updater }}
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
      - name: Use the update-signing key
        if: env.UPDATER == 'true'
        shell: bash
        env:
          KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        run: |
          {
            echo "TAURI_SIGNING_PRIVATE_KEY<<ISSUERELAY_EOF"
            echo "$KEY"
            echo "ISSUERELAY_EOF"
            # The key has no password (docs/release/updater-key.md): with CI set the CLI uses an
            # empty one. A password secret, if one is ever set, is passed on.
            if [[ -n "$PASSWORD" ]]; then
              echo "TAURI_SIGNING_PRIVATE_KEY_PASSWORD<<ISSUERELAY_EOF"
              echo "$PASSWORD"
              echo "ISSUERELAY_EOF"
            fi
            echo "ISSUERELAY_TAURI_CONFIG=src-tauri/tauri.updater.conf.json"
          } >> "$GITHUB_ENV"
      - name: Bundle without update payloads
        if: env.UPDATER != 'true'
        shell: bash
        run: echo "ISSUERELAY_TAURI_CONFIG=src-tauri/tauri.bundle.conf.json" >> "$GITHUB_ENV"
      - name: Use the Apple signing certificate
        if: runner.os == 'macOS' && needs.verify.outputs.apple == 'true'
        env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
        # APPLE_SIGNING_IDENTITY overrides the ad hoc "-" in tauri.conf.json; the CLI imports the
        # certificate into a temporary keychain and notarizes with the Apple ID.
        run: |
          for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
            { echo "$name<<ISSUERELAY_EOF"; printenv "$name"; echo "ISSUERELAY_EOF"; } >> "$GITHUB_ENV"
          done
      - name: Use the Windows signing certificate
        if: runner.os == 'Windows' && needs.verify.outputs.windows == 'true'
        shell: pwsh
        env:
          WINDOWS_CERTIFICATE: ${{ secrets.WINDOWS_CERTIFICATE }}
          WINDOWS_CERTIFICATE_PASSWORD: ${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}
        # Tauri merges tauri.windows.conf.json by itself when it builds on Windows.
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
        uses: tauri-apps/tauri-action@1deb371b0cd8bd54025b384f1cd735e725c4060f # v1.0.0
        env:
          GITHUB_TOKEN: ${{ github.token }}
        with:
          projectPath: app
          args: --target ${{ matrix.target }} --config ${{ env.ISSUERELAY_TAURI_CONFIG }}
          # Empty in a dry run: then the action only builds.
          releaseId: ${{ needs.create-release.outputs.id }}
          uploadUpdaterJson: ${{ env.UPDATER == 'true' }}
          updaterJsonPreferNsis: true
          retryAttempts: 1
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
          ARTIFACTS: ${{ steps.tauri.outputs.artifactPaths }}
        run: node scripts/verify-update-signature.mjs --version "$VERSION" --artifacts "$ARTIFACTS"
      - name: List the bundles to keep
        if: env.RELEASE != 'true'
        id: bundles
        shell: bash
        env:
          ARTIFACTS: ${{ steps.tauri.outputs.artifactPaths }}
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

  verify-release:
    needs: [verify, create-release, bundle]
    if: ${{ always() && needs.create-release.result == 'success' }}
    runs-on: ubuntu-latest
    permissions:
      contents: write
    env:
      GH_TOKEN: ${{ github.token }}
      RELEASE_ID: ${{ needs.create-release.outputs.id }}
      VERSION: ${{ needs.verify.outputs.version }}
      UPDATER: ${{ needs.verify.outputs.updater }}
    steps:
      - name: The draft has every asset and latest.json every platform
        run: |
          set -euo pipefail
          draft=$(gh api "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID" --jq .draft)
          if [[ "$draft" != true ]]; then
            echo "::error::release $RELEASE_ID is no longer a draft"
            exit 1
          fi
          assets=$(gh api "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID/assets" --paginate --jq '.[] | "\(.id) \(.name)"')
          echo "$assets"
          want=("IssueRelay_${VERSION}_universal.dmg" "IssueRelay_${VERSION}_x64-setup.exe" "IssueRelay_${VERSION}_x64_en-US.msi")
          if [[ "$UPDATER" == true ]]; then
            want+=("IssueRelay_${VERSION}_universal.app.tar.gz" "IssueRelay_${VERSION}_universal.app.tar.gz.sig"
                   "IssueRelay_${VERSION}_x64-setup.exe.sig" "IssueRelay_${VERSION}_x64_en-US.msi.sig" "latest.json")
          fi
          failed=0
          for name in "${want[@]}"; do
            if ! awk -v n="$name" '$2 == n { found = 1 } END { exit !found }' <<<"$assets"; then
              echo "::error::missing asset $name"
              failed=1
            fi
          done
          id=$(awk '$2 == "latest.json" { print $1 }' <<<"$assets")
          if [[ "$UPDATER" == true && -n "$id" ]]; then
            gh api "repos/$GITHUB_REPOSITORY/releases/assets/$id" -H "Accept: application/octet-stream" > latest.json
            if ! jq -e --arg v "$VERSION" '.version == $v or .version == ("v" + $v)' latest.json > /dev/null; then
              echo "::error::latest.json announces $(jq -r .version latest.json), not $VERSION"
              failed=1
            fi
            for key in darwin-aarch64 darwin-x86_64 windows-x86_64 windows-x86_64-nsis windows-x86_64-msi; do
              if ! jq -e --arg k "$key" '.platforms[$k] | ((.signature // "") | length > 0) and ((.url // "") | length > 0)' latest.json > /dev/null; then
                echo "::error::latest.json has no signed entry for $key"
                failed=1
              fi
            done
            if ! jq -e '.platforms["windows-x86_64"].url == .platforms["windows-x86_64-nsis"].url' latest.json > /dev/null; then
              echo "::error::latest.json's windows-x86_64 entry does not point at the NSIS installer"
              failed=1
            fi
          fi
          exit "$failed"
      - name: Mark an incomplete draft
        if: failure()
        run: |
          gh api -X PATCH "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID" \
            -f name="IssueRelay $GITHUB_REF_NAME (incomplete: do not publish)" > /dev/null
          echo "::error::the draft is incomplete and was renamed; see MAINTAINERS.md, 发布"
```

（几处要在第一次运行时看实际行为、与这里不同就照实改并写进报告：`releaseId` 为空字符串时 tauri-action 是否只构建；`--config` 的相对路径是否按 `projectPath`（`app/`）解析；`7z l` 能否列出 NSIS 安装包里的文件——不能时改为检查 `target/x86_64-pc-windows-msvc/release/` 下的两个 exe，并在报告里说明；`gh api … -H "Accept: application/octet-stream"` 能否跟随重定向取到草稿里的 `latest.json`——不能时改用 `curl -fsSL -H "Authorization: Bearer $GH_TOKEN" -H "Accept: application/octet-stream"`。）

- [ ] **Step 3: 静态检查与固定的 action 版本**

```bash
ruby -ryaml -e 'YAML.load_file(".github/workflows/release.yml"); puts "yaml ok"'
git ls-remote https://github.com/tauri-apps/tauri-action 'refs/tags/v1.0.0' 'refs/tags/v1.0.0^{}'
git ls-remote https://github.com/pnpm/action-setup 'refs/tags/v6.1.0' 'refs/tags/v6.1.0^{}'
git ls-remote https://github.com/Swatinem/rust-cache 'refs/tags/v2.9.2' 'refs/tags/v2.9.2^{}'
```

Expected：`yaml ok`；三个仓库各返回标签（附注标签还会多一行 `^{}`），指向提交的那一行分别是 `1deb371b0cd8bd54025b384f1cd735e725c4060f`、`ea17c68df8912ef543352723c149a84f56e3d413`、`6323deb102c322ba6fcbdcafc7e3dddab59af2b6`。不一致时用 `ls-remote` 给出的提交替换 `release.yml` 里的 SHA，并在报告里写明。

- [ ] **Step 4: `ci.yml` 的 pnpm 与 Node 与发布工作流一致**

`.github/workflows/ci.yml` 的 `test` 任务里，M2 Task 1 加的两步改成下面这样（`pnpm/action-setup` 的浮动标签 `v6` 停在 6.0.10，不支持 pnpm 12，所以写到 `v6.1.0`；Node 24 是当前的长期支持版）：

```yaml
      - uses: pnpm/action-setup@v6.1.0
        with:
          package_json_file: app/package.json
      - uses: actions/setup-node@v7
        with:
          node-version: 24
          cache: pnpm
          cache-dependency-path: app/pnpm-lock.yaml
```

其余步骤不变（`ci.yml` 不放命令行程序也能编译应用 crate：`externalBin` 不在基础配置里）。

- [ ] **Step 5: 文档**

`MAINTAINERS.md`：英文摘要里「Releases start at M3…」一条改为「Releases start at M3: a maintainer bumps the version in a PR, then pushes a `vX.Y.Z` tag on main; the workflow builds the installers into one draft release, and a maintainer publishes it by hand. Contributors never bump the version. The update-signing key lives only on the maintainer's machine and in GitHub Actions secrets.」「## 发布」一节整节换成：

~~~markdown
## 发布

M3 起由维护者发布；外部贡献者的 PR 不改版本号。工作流 `.github/workflows/release.yml`，安装说明 `docs/install.md`，更新签名密钥 `docs/release/updater-key.md`。

1. **升版本号**：开一个 PR，把工作区 `Cargo.toml` 的 `[workspace.package] version` 改成新版本。只用 `X.Y.Z`（MSI 不接受 `-rc.1` 这类非纯数字的预发布后缀），推送前按 `origin/main` 的最新值顺延；合入 `main`。
2. **推标签**（本地会话）：

   ```bash
   git fetch origin
   version=$(git show origin/main:Cargo.toml | sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\(.*\)"$/\1/p')
   git tag -a "v$version" origin/main -m "IssueRelay v$version"
   git push origin "v$version"
   ```

3. **等工作流全绿**：`verify`（标签 = 版本、标签提交在 `origin/main` 上、密钥扫描）→ `test`（macOS、Windows 六条验证命令）→ `create-release`（建一个草稿）→ `bundle`（先 macOS 通用包，后 Windows）→ `verify-release`（资产齐全、`latest.json` 每个平台都有签名）。`verify-release` 失败时，草稿标题会被改成「incomplete: do not publish」。
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

secrets 只由管理员在仓库设置里改；拉取请求触发的试打包从不使用它们。
~~~

- `docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 7.1 节改为：「可以做实机验收、打版本标签、发布安装包。发布只从已推送到 `origin/main` 的提交构建：发布工作流核对标签提交在 `origin/main` 上、标签等于工作区版本，只建草稿 Release；公开由维护者在 GitHub 页面上点。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 10 节最后一条改为：「M3 起推送 `vX.Y.Z` 标签触发发布（`.github/workflows/release.yml`）：标签必须等于工作区版本、标签提交必须在 `origin/main` 上，工作流只建草稿，步骤见 `MAINTAINERS.md`「发布」。」
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 6 节加：`.github/workflows/release.yml`：版本标签触发的打包与草稿 Release，手动运行与改了打包配置的 PR 只试打包；`scripts/verify-update-signature.mjs`：用提交的公钥核对更新包签名（用法见文件头）。
- 设计第 12 节「GitHub Actions」一条改为：「GitHub Actions（`release.yml`）：推送版本标签时先核对标签与版本、标签提交在 `main` 上并扫描密钥，在 macOS 和 Windows 上跑全部验证命令，建一个草稿 Release，逐个平台打包上传，最后核对资产与 `latest.json` 齐全；由维护者确认后公开。手动运行与改了打包配置的 PR 只试打包。」第 18 节第 2 条后补：「现状（M3）：没有证书时 macOS 包做 ad-hoc 签名（不是完全不签：未签名的 arm64 包下载后会被报『已损坏』），Windows 不签名；secrets 里放齐 Apple 或 Windows 证书后，发布工作流自动改用正式签名（macOS 同时公证）。」
- `.github/CODEOWNERS` 加 `/scripts/stage-cli.sh`、`/scripts/stage-cli.ps1`、`/scripts/verify-update-signature.mjs` 三行（发布流水线）。

Run: 六条验证命令
Expected: 全部通过。

- [ ] **Step 6: Commit**

```bash
git add .github scripts/verify-update-signature.mjs MAINTAINERS.md docs
git commit -m "ci: build installers and draft a GitHub release on version tags"
```

- [ ] **Step 7: 试打包（M3 的 PR）**

推送分支并开草稿 PR（已开过就只推送）；改了打包配置，`release` 工作流随 PR 运行：

```bash
git fetch origin && git rebase origin/main   # only if main moved; then run the six commands again
git push -u origin feat/m3-release
# First time only: open a draft PR whose description follows .github/pull_request_template.md.
gh pr create --draft --title "feat: installers, CLI on PATH, signed auto-update and the release workflow (M3)" --body-file "$PR_BODY"
gh run list --workflow release.yml --branch feat/m3-release --limit 1
gh run watch "$(gh run list --workflow release.yml --branch feat/m3-release --limit 1 --json databaseId --jq '.[0].databaseId')" --exit-status
```

（`$PR_BODY` 是按 `.github/pull_request_template.md` 填好的描述文件，放在 `mktemp -d` 的临时目录里。刚推送时运行可能还没出现在列表里，几秒后再查。）

Expected：`verify`、两个 `test`、两个 `bundle` 成功，`create-release` 与 `verify-release` 跳过；`verify` 的输出 `updater=false`、`apple=false`、`windows=false`（拉取请求从不使用 secrets）；两个包内容检查通过。下载产物核对：

```bash
SCRATCH=$(mktemp -d)
run=$(gh run list --workflow release.yml --branch feat/m3-release --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" -D "$SCRATCH"
find "$SCRATCH" -type f | sed "s|$SCRATCH/||" | sort
dmg=$(find "$SCRATCH" -name 'IssueRelay_*_universal.dmg' | head -1)
mkdir "$SCRATCH/mnt" && hdiutil attach -nobrowse -readonly -mountpoint "$SCRATCH/mnt" "$dmg" > /dev/null
lipo -archs "$SCRATCH/mnt/IssueRelay.app/Contents/MacOS/issuerelay" "$SCRATCH/mnt/IssueRelay.app/Contents/MacOS/issuerelay-app"
codesign --verify --deep --strict "$SCRATCH/mnt/IssueRelay.app" && echo "signature ok"
hdiutil detach "$SCRATCH/mnt" > /dev/null
```

Expected：`bundles-macos-universal/…/IssueRelay_<版本>_universal.dmg`、`bundles-windows-x64/…/IssueRelay_<版本>_x64-setup.exe`、`…/IssueRelay_<版本>_x64_en-US.msi`（`<版本>` 是工作区版本；没有 `.sig`：试打包不签更新包）；两个 `lipo` 都是 `x86_64 arm64`；`signature ok`。把 `$SCRATCH` 留给 Task 6 用（Task 6 结束时删除）。任何一处与预期不同（资产名、目录结构、tauri-action 的行为），修好、推送、重跑，并在报告里写明差异与改法。

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

只从上面这个 Releases 页面下载。

## macOS 安装

1. 双击 `.dmg`，把 IssueRelay 拖进「应用程序」。没有管理员权限时，拖进你自己的「应用程序」文件夹（在访达里前往 `~/Applications`，没有就新建）。
2. 推出磁盘映像。不要直接在磁盘映像里或「下载」文件夹里运行 IssueRelay：那样「把命令行加入 PATH」和自动更新都用不了。

## macOS 第一次打开

在 Apple 公证之前，第一次打开会被 macOS 拦下来，只需放行一次：

- **macOS 15 及以后**：双击 IssueRelay，看到提示后点「完成」；打开「系统设置 → 隐私与安全性」，在「安全性」下面点 IssueRelay 旁边的「仍要打开」（被拦后大约一小时内可以点），输入登录密码，再确认一次。之后就能正常打开。
- **macOS 14 及更早**：按住 Control 点按 IssueRelay，选「打开」，再点「打开」。
- **终端里的办法**（只对从上面的 Releases 下载的包这么做）：`xattr -dr com.apple.quarantine /Applications/IssueRelay.app`（装在个人「应用程序」里就把路径换成 `~/Applications/IssueRelay.app`）。它去掉下载标记，macOS 就不再为这个应用做首次检查。
- 如果提示「已损坏，无法打开」：下载的文件不完整或不是从 Releases 下载的，删掉后重新下载。

## Windows 安装

1. 双击 `-setup.exe`。若出现「Windows 已保护你的电脑」，点「更多信息」，再点「仍要运行」（安装包没有代码签名时会这样）。
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

从 [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases/latest) 下载：Mac 用 `IssueRelay_<版本>_universal.dmg`，Windows 用 `IssueRelay_<版本>_x64-setup.exe`（不需要管理员权限）。安装包还没有经过 Apple 公证和 Windows 代码签名，第一次打开要放行一次；步骤、把命令行加入 PATH、自动更新与卸载见 [安装说明](docs/install.md)。

Download from [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases/latest): `IssueRelay_<version>_universal.dmg` for macOS, `IssueRelay_<version>_x64-setup.exe` for Windows (no administrator rights needed). The installers are not yet notarized by Apple or code-signed for Windows, so the first start needs one approval (macOS 15+: System Settings → Privacy & Security → Open Anyway; Windows: More info → Run anyway). See [docs/install.md](docs/install.md) (Chinese) for the steps, putting the CLI on PATH, automatic updates and uninstalling.
```

设计第 12 节「代码签名」一条改为：「代码签名：见第 18 节。未公证 / 未签名时，`docs/install.md` 与 README 写清放行步骤：macOS 15 起在『系统设置 → 隐私与安全性』点『仍要打开』（14 及更早：Control 点按 → 打开），Windows SmartScreen『更多信息 → 仍要运行』。」

- [ ] **Step 3: 只读审阅**

派一个子智能体只读审阅 `docs/install.md` 与 README 新加的一节（不执行安装、不改文件），角度：一位没用过 IssueRelay 的同事在一台干净的 macOS 用户环境里，只看这份说明能否装好、放行、完成首次引导、把命令行加入 PATH、更新与卸载；指出含糊、顺序不对、与 Task 1–4 的实际行为不符（资产名、设置项名、按钮名）的地方。主智能体按意见修订。

- [ ] **Step 4: 提交**

Run: 六条验证命令
Expected: 全部通过（`check-secrets` 不会命中：文档里的路径都以 `~`、`%LOCALAPPDATA%` 或 `/Applications` 开头）。

```bash
git add docs/install.md README.md docs/superpowers/specs/2026-10-10-issuerelay-design.md
git commit -m "docs: add installation instructions"
```

---

### Task 6: M3 实机验收与第一个草稿 Release

由主智能体在本机执行（实机验收只由主智能体做；AGENTS.md）。把调研里「未确认」的几项变成明确的检查：macOS 15 / 26 上 Gatekeeper 的提示与放行流程；ad-hoc 签名的版本更新后会不会再弹钥匙串授权；Windows 上正在运行的 `issuerelay.exe` 会不会挡住 NSIS 更新或卸载（待实机）；`request_restart()` 与单实例插件配合后，更新完程序是否确实在运行。报告写 `docs/gates/<日期>-m3-acceptance.md`（按 `TESTING_SPEC.md` 的模板，真实数据一律脱敏，截图不进仓库）。

**需要用户的地方：** 「仍要打开」要输入登录密码；钥匙串授权框；在浏览器里重新授权 Linear；上传更新签名私钥（`gh secret set`）；合并 PR；公开草稿 Release。这些涉及用户的凭据或对外动作，主智能体只在那一步请用户操作，其余自行完成。

**Files:**
- Create: `docs/gates/<日期>-m3-acceptance.md`
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 17 节 M3 一行的完成情况；按实测更新第 12 节里与实际不符的说法）、`docs/install.md`（按实测改提示文字）

**Interfaces:**
- Consumes：Task 4 Step 7 留下的试打包产物（`$SCRATCH`）；Task 1 的 `scripts/stage-cli.sh`、`tauri.bundle.conf.json`；Task 3 的 `~/.tauri/issuerelay.key`（只以路径交给构建，从不读出）；Task 5 的 `docs/install.md`；M2 Task 13 的沙箱项目与沙箱仓库。
- Produces：验收报告；`v<工作区版本>` 标签与一个通过 `verify-release` 的草稿 Release（由维护者公开）。

- [ ] **Step 1: 前提**

Task 4 Step 7 的试打包全绿；`gh secret list --repo sunkailun1992/issuerelay` 里有 `TAURI_SIGNING_PRIVATE_KEY`（只看名字）。没有时请维护者运行 Task 3 Step 0 的 `gh secret set` 命令；维护者决定先不上传时，第 8 步只核对打包（`updater=false`，没有签名可核对），第 9–10 步照常做，草稿会是「不含自动更新包」的版本，报告里写明。

- [ ] **Step 2: 按说明安装（macOS，不需要管理员）**

模拟浏览器下载（给 dmg 加上浏览器会加的隔离标记；能请用户用 Safari 从 PR 的产物页下载更好）后，按 `docs/install.md` 拖进个人「应用程序」：

```bash
dmg=$(find "$SCRATCH" -name 'IssueRelay_*_universal.dmg' | head -1)
xattr -w com.apple.quarantine "0083;$(printf '%x' "$(date +%s)");Safari;" "$dmg"
open "$dmg"   # drag IssueRelay to ~/Applications in Finder, then eject the disk image
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

用两份本机构建测试真正的更新：A 是当前版本，B 是临时升到下一个补丁版本、用真正的私钥签名的构建；两者的更新地址都临时指向本机。这两份只用于测试，不进仓库、不发布。

```bash
UPDATE_TEST=$(mktemp -d)
bash scripts/stage-cli.sh
# A: the current version, its updater pointed at this machine (test build only).
CI=true pnpm --dir app tauri build --bundles app --config '{"bundle":{"externalBin":["binaries/issuerelay"]},"plugins":{"updater":{"endpoints":["http://127.0.0.1:8765/latest.json"],"dangerousInsecureTransportProtocol":true}}}'
ditto target/release/bundle/macos/IssueRelay.app "$UPDATE_TEST/A/IssueRelay.app"
# B: the next patch version, signed with the real key, which is passed as a path and never read.
current=$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\(.*\)"$/\1/p' Cargo.toml)
next=$(echo "$current" | awk -F. '{printf "%d.%d.%d", $1, $2, $3 + 1}')
sed -i '' "s/^version = \"$current\"$/version = \"$next\"/" Cargo.toml
bash scripts/stage-cli.sh
TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/issuerelay.key" CI=true pnpm --dir app tauri build --bundles app --config '{"bundle":{"externalBin":["binaries/issuerelay"],"createUpdaterArtifacts":true},"plugins":{"updater":{"endpoints":["http://127.0.0.1:8765/latest.json"],"dangerousInsecureTransportProtocol":true}}}'
git checkout Cargo.toml Cargo.lock
mkdir -p "$UPDATE_TEST/serve"
cp target/release/bundle/macos/IssueRelay.app.tar.gz "$UPDATE_TEST/serve/"
node scripts/verify-update-signature.mjs --version "$next" --artifacts "[\"$UPDATE_TEST/serve/IssueRelay.app.tar.gz\",\"$PWD/target/release/bundle/macos/IssueRelay.app.tar.gz.sig\"]"
```

（`--config` 写在一个 JSON 里：一次只传一个。`dangerousInsecureTransportProtocol` 只用于这两份测试构建。最后一行用提交的公钥核对 B 的签名，Expected：`verified IssueRelay.app.tar.gz`。`git checkout` 之后 `git status` 应当干净。）

写本机清单并在另一个终端里起一个只监听本机的服务（测试结束时 Ctrl+C）：

```bash
sig=$(cat target/release/bundle/macos/IssueRelay.app.tar.gz.sig)
cat > "$UPDATE_TEST/serve/latest.json" <<EOF
{
  "version": "$next",
  "notes": "Local update test",
  "pub_date": "2026-10-20T08:00:00Z",
  "platforms": {
    "darwin-aarch64": { "signature": "$sig", "url": "http://127.0.0.1:8765/IssueRelay.app.tar.gz" }
  }
}
EOF
python3 -m http.server 8765 --bind 127.0.0.1 --directory "$UPDATE_TEST/serve"
```

测试：

1. 退出第 2 步装的应用，换上 A：`rm -rf ~/Applications/IssueRelay.app && ditto "$UPDATE_TEST/A/IssueRelay.app" ~/Applications/IssueRelay.app`，从访达打开；登录 Linear（用户操作），确认连上。
2. 在沙箱项目建一条会跑一两分钟的需求，等它开始运行。
3. 「设置 → 检查更新」：显示「IssueRelay <next> 已发布」，顶部出现横幅。点「安装并重启」：出现与退出相同的询问（「1 个运行正在进行…」，并说明会重启安装更新）。选「等待完成」→ 说明变为「等运行结束后安装更新并重启」→ 再改选「立即停止」：运行记为已取消、沙箱里没有残留进程（`ps -axo pid,ppid,pgid,command | grep -E 'claude|codex'`），程序安装并重启。
4. 重启后：面板侧栏版本为 `<next>`；`pgrep -x issuerelay-app | wc -l` 为 `1`（单实例插件没有挡住新实例，调研未确认 4）；托盘与面板正常；再 `open -b com.sunkailun.issuerelay` 只把面板带到前面。
5. 钥匙串（调研未确认 2）：重启后是否弹出「想要使用钥匙串中的机密信息」之类的授权框（原文记录；由用户决定点「始终允许」还是「允许」，记录点了哪个）；Linear 是否仍是已连接；再从托盘退出、重新打开一次，是否还弹。开机自启打开时，`~/Library/LaunchAgents/` 里的条目是否仍指向 `~/Applications/IssueRelay.app`，「登录项与扩展」里显示为什么。
6. 再来一次「点安装并重启 → 继续运行（Keep running）→ 从托盘退出」：程序只退出、不安装（版本仍是装着的那个）。

- [ ] **Step 6: 失败的更新只提示（Review Focus 2 实机）**

让一条沙箱运行在进行中，依次把本机清单改成下面四种，每次点「检查更新」或「安装并重启」，记录面板的标题与细节行，确认什么都没安装、运行继续、托盘状态不变：

1. 签名换成另一把一次性密钥签的签名，版本仍为 `<next>`：「更新包的签名校验没有通过，什么都没安装」。生成那份签名（不碰真正的私钥）：

   ```bash
   pnpm --dir app tauri signer generate --ci -w "$UPDATE_TEST/other.key" > /dev/null
   cp "$UPDATE_TEST/serve/IssueRelay.app.tar.gz" "$UPDATE_TEST/other.tar.gz"
   pnpm --dir app tauri signer sign -f "$UPDATE_TEST/other.key" -p "" --app-version "$next" "$UPDATE_TEST/other.tar.gz" > /dev/null
   cat "$UPDATE_TEST/other.tar.gz.sig"   # a signature, safe to show: paste it into the "signature" field of latest.json
   ```
2. `version` 改成当前版本：「已是最新版本」。
3. 停掉本机服务：「连不上更新服务器」（横幅若已显示，仍保留）。
4. `platforms` 里只留一个 `windows-x86_64`：「新版本还没有适合这台电脑的安装包」。

测试完：停掉服务，删掉 `$UPDATE_TEST` 与 `app/src-tauri/binaries/`（里面是 `<next>` 版的命令行程序），把第 2 步的正式安装（Task 4 的试打包产物）重新装回 `~/Applications`。

- [ ] **Step 7: 报告（合入之前）**

写 `docs/gates/<日期>-m3-acceptance.md`：环境（macOS 版本、芯片、安装位置）、第 2–6 步每项的结果与原文（脱敏：`DEMO-N`、`Demo Project`）、与设计或文档不一致之处及改法。「未闭环」至少列出：

- macOS 15、26 上的 Gatekeeper 提示与「仍要打开」流程（本机不是这两个版本时）；「同事按说明用安装包装好并跑通」（设计第 17 节的完成标志，待同事）。
- Windows 全部待实机（没有 Windows 机器）：NSIS 按用户安装不要管理员与 SmartScreen 的提示原文；智能应用控制开启时的表现；「把命令行加入 PATH」的往返（打开、新终端 `issuerelay --version`、关闭；用户 `Path` 里其他条目逐字不变，`REG_EXPAND_SZ` 不变成 `REG_SZ`）；从终端 `issuerelay start`、关掉终端后托盘程序仍在；**正在运行的 `issuerelay.exe` 是否挡住 NSIS 更新与卸载**（调研未确认 3：在一个终端里跑 `issuerelay logs <编号> --lines 5000` 之类的长命令或让 AI 客户端开着 `issuerelay mcp`，再触发「安装并重启」与卸载；若被挡住，候选改法是 NSIS 的 `NSIS_HOOK_PREINSTALL` / `NSIS_HOOK_PREUNINSTALL` 钩子把 `issuerelay.exe` 也交给重启管理器，或在安装说明里要求先关掉）；MSI 安装要管理员、更新经 `msiexec` 弹管理员确认；NSIS 卸载程序是否提供「删除应用数据」以及它删哪些目录。
- `latest.json` 里的下载地址是 `api.github.com` 的资产地址：同一出口 IP 的未登录 API 限额（每小时 60 次）在多人同时更新时是否够用，待观察。

更新设计第 17 节 M3 一行：「安装包、发布流程、自动更新、把命令行加入 PATH 已在 macOS 实机验收（`docs/gates/<日期>-m3-acceptance.md`）；Windows 与同事安装待实机。」

```bash
git add docs
git commit -m "docs(m3): record the installer acceptance on macOS"
git push
```

然后请维护者按 `MAINTAINERS.md` 合并 M3 的 PR（CI 与 `release` 试打包全绿）。

- [ ] **Step 8: 合入后用私钥试打包一次**

合入后 `release.yml` 已在默认分支上，可以手动运行；这一次用上更新签名私钥，验证签名与核对脚本，仍不建 Release：

```bash
gh workflow run release.yml --ref main
run=$(gh run list --workflow release.yml --branch main --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" --exit-status
gh run download "$run" -D "$(mktemp -d)"
```

Expected：`verify` 的输出 `updater=true`（私钥已上传时）；两个 `bundle` 的「Check the update signatures against the committed public key」步骤输出 `verified IssueRelay.app.tar.gz`、`verified IssueRelay_<版本>_x64-setup.exe`、`verified IssueRelay_<版本>_x64_en-US.msi`；产物里有 `.app.tar.gz` 与三个 `.sig`。失败时修好再来，不推标签。

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

（同样，刚推送时运行可能还没出现，几秒后再查一次 `run`。）Expected：工作流全绿（`verify`、两个 `test`、`create-release`、两个 `bundle`、`verify-release`）；`gh release list` 里有一个 `Draft` 状态的 `IssueRelay v<版本>`，标题不含「incomplete」。从草稿下载 dmg 在本机装一遍（第 2–3 步的简版）。失败时按 `MAINTAINERS.md`「发布」第 6 条删草稿与标签，修好后重推同一个标签。

**不公开草稿**：把草稿链接、资产列表与 `verify-release` 的结果交给维护者，由维护者检查后在 GitHub 页面上点「Publish release」。

- [ ] **Step 10: 补记第一个草稿**

在一个只改文档的小 PR 里，把第 8–9 步的结果（运行编号、资产列表、`latest.json` 的平台键）补进验收报告，设计第 17 节注明「v<版本> 草稿已生成，待维护者公开」：

```bash
git switch -c docs/m3-first-draft origin/main
git add docs
git commit -m "docs(m3): record the first draft release"
git push -u origin docs/m3-first-draft
gh pr create --title "docs(m3): record the first draft release" --body-file "$PR_BODY"   # the description follows .github/pull_request_template.md
```

结束时删掉 Task 4 Step 7 留下的 `$SCRATCH`。

---

## 自查记录（写计划时）

- **设计覆盖**：2.1 安装包、自动更新、命令行小程序——Task 1、3、4；2.2 第一版不做代码签名——默认 ad-hoc（macOS）与不签名（Windows），secrets 齐全时自动改用正式签名（Task 1、4）；4.1 命令行「托盘程序没在运行时给出提示，并可代为启动」——Task 2（`issuerelay start`，MCP 不启动）；10.1 / 10.3 命令行子命令与 MCP 服务——Task 2；12 打包目标、附带程序、把命令行加入 PATH、自动更新、代码签名的放行步骤、版本号——Task 1–5；16 验证命令——每个任务都跑六条，发布工作流的 `test` 任务也跑六条；16.1 维护者发布、只由维护者升版本号——Task 4 的 `MAINTAINERS.md`；17 M3 完成标志（同事按说明装好）——Task 6，同事安装与 Windows 待实机；18.2 代码签名——Task 4 的可选 secrets 与设计第 18 节的现状说明；18.4 Windows 验证环境——Task 6 的待实机清单。
- **占位扫描**：没有「待定」「以后再写」。需要执行时才有的值都由具体命令产生：公钥由 Task 3 Step 0 的 `node -e` 从 `.pub` 文件写入；工作区版本在 CI 里由 Python 的 `tomllib`、在本机由 `sed` 从 `Cargo.toml` 读出；Rust 与 npm 的依赖版本由 `cargo add` / `pnpm add` 解析后锁定；验收报告的日期是执行当天。`zh-TW`、`ja`、`ko` 的文案按 M2 Task 2 的规则由实现者翻译（与 M2 计划的做法相同：计划给出 `en` 与 `zh-CN` 的确切文案）。几处库细节（`tauri_plugin_updater::Error` 的变体、`request_restart`、`winreg` 的 `RegValue`、`windows-sys` 的函数签名、tauri-action 对空 `releaseId` 的处理、`7z` 列 NSIS 内容、`gh api` 跟随重定向）都写明了按解析到的版本核对与不同时的改法。
- **与 M2 计划的名字和类型一致**：主程序名 `mainBinaryName: "issuerelay-app"`（现在由 `paths::APP_EXECUTABLE` 与配置测试共同守住）；`cli_next_to`、`cli_path`、`app_info().cli_path` 不改；退出状态机的 `QuitState`、`QuitEvent::Requested { host_ready, running }`、`QuitStep`、`next`、`handle`、`on_run_event` 不改，`request` 的对外行为不变（多了先丢掉已下载的更新），新增 `request_update`；`QUIT_REQUESTED` 的载荷从 `usize` 改为 `QuitPrompt { running, purpose }`，所有用到它的地方都在 Task 3 里改：`quit.rs` 的 `handle`、`backend.ts` 的 `onQuitRequested`、`mock.ts` 的 `listeners.quit`、`QuitDialog.tsx` 与它的测试（`api.ts` 的 `on<T>` 是泛型，只改类型参数）；命令名表由 M2 的 33 条加到 39 条（Task 2 的 `cli_on_path`、`install_cli_on_path`、`remove_cli_from_path`，Task 3 的 `update_status`、`check_update`、`install_update`），`backend.ts`、`api.ts`、`mock.ts`、`generate_handler!` 同步；事件名 `update-status` 在 `events.rs` 与 `api.ts` 一致；`Settings.auto_update` 沿用 M2 第一部分的字段，只改文档注释；错误代码经 M2 的 `ERROR_CODES` 与 `describeError` 翻译（`errors.update_*`）；变更钩子用 M2 的 `useAction(fn, invalidate, failureKey)`。
- **调研里未确认的事项 → 验收或检查步骤**：

| 未确认事项（调研） | 在哪里核对 |
|---|---|
| macOS 15 / 26 上未签名与 ad-hoc 包的提示原文、「仍要打开」流程 | Task 6 第 2–3 步（本机版本）；15 / 26 记待实机 |
| `xattr -dr com.apple.quarantine` 在 15 / 26 上可用 | Task 6 第 3 步 |
| 经 `~/.local/bin` 链接运行命令行程序要不要另行放行 | Task 6 第 4 步第 2 项 |
| ad-hoc 签名的新版本更新后是否再弹钥匙串授权、Linear 是否仍连接 | Task 6 第 5 步第 5 项 |
| 更新后开机自启条目与「登录项」的显示 | Task 6 第 5 步第 5 项 |
| `request_restart()` 与单实例插件：更新后程序确实在运行、只有一个实例 | Task 6 第 5 步第 4 项 |
| 「安装并重启」走等待 / 停止询问（面板流程，调研建议用 `request_restart` / 退出事件） | Task 3 的测试；Task 6 第 5 步第 3、6 项 |
| 正在运行的 `issuerelay.exe` 挡不挡 NSIS 更新与卸载 | Task 6 第 7 步待实机清单（带测试方法与候选改法） |
| Windows 用户 PATH 的往返（`%VAR%` 条目逐字保留、类型不变） | Task 2 的纯函数测试；Task 6 第 7 步待实机 |
| 智能应用控制与 SmartScreen 的实际表现 | `docs/install.md`；Task 6 第 7 步待实机 |
| 通用包里的命令行程序真是双架构 | `stage-cli.sh --universal` 的自检；Task 4 `bundle` 的包内容检查；Task 4 Step 7 |
| 工作区继承的版本是否进入资产名 | Task 1 Step 5；Task 4 Step 7 与 `verify-release` 的资产名检查 |
| 两个矩阵任务并发合并 `latest.json` 丢平台、重复草稿 | `create-release` 只建一次 + `max-parallel: 1` + `verify-release` |
| 空的 `APPLE_SIGNING_IDENTITY` 等环境变量的处理 | 只在 secrets 齐全时写进 `$GITHUB_ENV`，不出现空值 |
| 私钥与公钥不配对时 `tauri build` 只警告 | `scripts/verify-update-signature.mjs`（Task 4 Step 1 自测、`bundle` 里运行、Task 6 第 8 步） |
| 从磁盘映像或临时副本运行时建链接、更新 | `cli_path::bundle_location` 的测试；Task 6 第 4 步第 6 项 |
| GUI 启动的 AI 客户端看不到 `~/.local/bin` | MCP 注册一直用绝对路径（M2）；`docs/install.md` 写明 |
| `api.github.com` 资产地址的未登录限额 | Task 6 第 7 步「未闭环」 |

- **Review Focus → 测试**：第 1 条——`quit.rs` 的 `the_quit_prompt_tells_the_panel_what_follows` 与 M2 的退出状态机测试、`update.rs` 的 `a_verified_download_waits_for_the_quit_flow_and_is_reused`、`QuitDialog.test.tsx` 的 `installing an update with runs in progress…`、`UpdateBanner.test.tsx` 的第一个测试、Task 6 第 5 步；第 2 条——`update.rs` 的 `only_a_newer_version_is_offered`、`a_failed_check_only_informs`、`a_bad_signature_keeps_nothing_to_install`、`a_failed_check_keeps_the_version_already_offered`、`UpdateBanner.test.tsx` 的第二个测试、`Settings.test.tsx` 的检查更新测试、Task 6 第 6 步；第 3 条——`release.yml` 的 `verify`（标签、`origin/main`、密钥扫描、secrets）、`create-release`（同标签已有 Release）、`bundle`（包内容、签名核对）、`verify-release`（资产、`latest.json`、标记草稿），Task 4 Step 7 与 Task 6 第 8–9 步实际运行；第 4 条——`cli_path.rs` 的全部纯函数测试与 `link` 子模块的四个读写测试、`Settings.test.tsx` 的三个 PATH 测试、Task 6 第 4 步；第 5 条——`main.rs` 的 `start_launches_the_app_only_when_it_is_not_running`、`start_reports_an_app_that_never_answers_and_other_failures`、`a_stopped_app_points_to_issuerelay_start`，`mcp_tools.rs` 的两条新断言，Task 6 第 4 步第 4–5 项。
