# IssueRelay M3：安装与发布 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让同事不用命令行就能装上 IssueRelay：GitHub Actions 在推送版本标签时打出 macOS `.dmg`（通用包）与 Windows `.msi`、`.exe`，附带命令行小程序，发到 GitHub Releases；程序能自动更新；README 与安装说明写清未签名包的放行步骤。

**Architecture:** Tauri 打包。命令行小程序 `issuerelay` 作为 Tauri 的附带程序（`externalBin`）装在主程序旁边；设置里提供「把命令行加入 PATH」；命令行发现托盘程序没在运行时提示并可代为启动。自动更新用 Tauri updater，更新包用单独的签名密钥签名（私钥只放在 GitHub Actions secrets），清单 `latest.json` 从 GitHub Releases 拉取。发布工作流先跑全部测试，再在 macOS 与 Windows 上打包，创建草稿 Release，由维护者确认后公开。

**Tech Stack:** Tauri 2 bundler（dmg、msi、nsis）、`tauri-plugin-updater`、`tauri-plugin-process`、GitHub Actions（`tauri-apps/tauri-action`）、`lipo`（macOS 通用二进制）。

**Spec:** `docs/superpowers/specs/2026-10-10-issuerelay-design.md` 第 4.1、12、16.1、17、18 节；依赖 M2 第一、二部分。

## Global Constraints

- M2 两部分的全部全局约束继续有效（六条验证命令、硬规则、公开仓库红线、分支与 PR、禁止强推）。
- 版本号只有一个来源：工作区 `Cargo.toml` 的 `version`（`tauri.conf.json` 不写 `version`，Tauri 读 crate 版本）；标签为 `vX.Y.Z`，必须与它一致，发布工作流核对，不一致就失败。只由维护者在发布时升（设计 12、16.1）。
- 签名私钥、证书、密码只放在 GitHub Actions secrets；仓库、日志、提交信息里都不能出现。工作流在 secrets 缺失时降级：没有更新签名私钥就不生成更新包并在 Release 说明里写明；没有 Apple / Windows 代码签名证书就发未签名包（设计第 2.2、18 节）。
- 发布工作流只创建草稿 Release；公开由维护者在 GitHub 页面上点（公开发布是对外动作）。
- 自动更新的清单地址固定为 `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json`；只接受用仓库里那把公钥验签通过的更新包。
- 命令行小程序安装在主程序旁边：macOS `IssueRelay.app/Contents/MacOS/issuerelay`，Windows 安装目录下的 `issuerelay.exe`。

## Review Focus

1. 有任务在跑时点「安装更新并重启」：要走与退出相同的询问（等待完成 / 立即停止），不能直接重启丢掉运行（Task 3 的测试）。
2. 更新清单拿不到、验签失败、版本更旧：只提示，不安装，不影响监控（Task 3 的测试）。
3. 标签与 `Cargo.toml` 版本不一致、或 secrets 缺失：工作流明确失败或明确降级，不产出半套资产（Task 4 的检查步骤）。
4. 用户没有管理员权限：「把命令行加入 PATH」不需要管理员（macOS 链接到 `~/.local/bin`，Windows 改用户级 PATH），已在 PATH 里时不重复添加（Task 2 的测试）。
5. 命令行在托盘程序没运行时被调用（例如 Claude 通过 MCP 调用）：给出清楚的提示，`issuerelay start` 能把托盘程序拉起来；MCP 服务不自动拉起（避免 AI 静默启动有完全权限的程序）（Task 2 的测试）。

---

### Task 1: 打包配置与附带的命令行程序

**Files:**
- Modify: `app/src-tauri/tauri.conf.json`（`bundle`：`targets` 为 `["app", "dmg", "msi", "nsis"]`，`externalBin: ["binaries/issuerelay"]`，`macOS.minimumSystemVersion: "12.0"`，`windows.nsis.installMode: "currentUser"`，删掉顶层 `version`）
- Create: `scripts/stage-cli.sh`、`scripts/stage-cli.ps1`（按目标三元组把 `issuerelay` 复制成 `app/src-tauri/binaries/issuerelay-<三元组>[.exe]`；`--universal` 时用 `lipo` 把两个架构合成 `issuerelay-universal-apple-darwin`）
- Modify: `.gitignore`（`app/src-tauri/binaries/`）、`app/src-tauri/src/commands.rs`（`cli_path` 不变：Tauri 把附带程序放到主程序旁边，去掉三元组后缀）
- Test: 本机 `pnpm --dir app tauri build --bundles app,dmg` 成功；`IssueRelay.app/Contents/MacOS/issuerelay --version` 输出版本

- [ ] **Step 1**: 写 `stage-cli.sh`（`set -euo pipefail`；参数 `--target <三元组>` 或 `--universal`；先 `cargo build -p issuerelay-cli --release --target …`，再复制；目标目录不存在就建；打印生成的文件名）与等价的 `stage-cli.ps1`（Windows 只有 `x86_64-pc-windows-msvc`）。
- [ ] **Step 2**: 改 `tauri.conf.json`；本机 `bash scripts/stage-cli.sh --target aarch64-apple-darwin && pnpm --dir app tauri build --bundles app,dmg`。
- [ ] **Step 3**: 核对 `.app` 里的附带程序：`IssueRelay.app/Contents/MacOS/issuerelay --version`、`status`（托盘程序没运行时的提示）。
- [ ] **Step 4**: 文档：设计第 12 节补具体的打包目标与附带程序位置；`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 写明 `binaries/` 是生成目录。
- [ ] **Step 5**: 提交 `feat(app): bundle the CLI next to the tray app`。

### Task 2: 「把命令行加入 PATH」与 `issuerelay start`

**Files:**
- Create: `crates/core/src/cli_path.rs`（纯逻辑：`fn path_has(dir, path_var) -> bool`、`fn windows_path_with(dir, current) -> Option<String>`）
- Modify: `app/src-tauri/src/commands.rs`（`cli_on_path() -> CliPathStatus { installed: bool, location: Option<String>, on_path: bool }`、`install_cli_on_path()`、`remove_cli_from_path()`）、`app/src/pages/Settings.tsx`（系统一组加开关与说明）
- Modify: `crates/cli/src/main.rs`（新子命令 `start`；`NotRunning` 的提示改为「IssueRelay is not running; start it with `issuerelay start`」；MCP 服务模式下只提示不启动）
- Test: `cli_path.rs` 的单元测试（已在 PATH、不在、Windows 的分号拼接与去重、大小写不敏感的比较）；命令行 `start` 的测试用一个可注入的启动器（不真的启动应用）

规则：

- macOS：在 `~/.local/bin/issuerelay` 建指向 `IssueRelay.app/Contents/MacOS/issuerelay` 的符号链接（已有同名非链接文件时不覆盖，提示用户）；用 Task 1（M2 第一部分）解析出的登录 shell PATH 判断 `~/.local/bin` 在不在 PATH，不在就在面板里给出要加到 shell 配置文件的那一行（不替用户改 shell 配置文件）。
- Windows：把安装目录加到 `HKCU\Environment` 的 `Path`（已存在就不加），然后广播 `WM_SETTINGCHANGE`；移除时只删这一项。
- `issuerelay start`：macOS 用 `open -b com.sunkailun.issuerelay`；Windows 启动命令行程序同目录下的 `IssueRelay.exe`（找不到时提示从开始菜单启动）。已在运行时只说明已在运行。

- [ ] Steps: 失败测试 → 实现 → 设置页 → 文档（设计第 4.1 节「代为启动」与第 12 节「把命令行加入 PATH」写实际行为）→ 提交 `feat: put the CLI on PATH from the settings and start the tray app from the CLI`。

### Task 3: 自动更新

**Files:**
- Modify: `app/src-tauri/Cargo.toml`（`tauri-plugin-updater`、`tauri-plugin-process`）、`app/src-tauri/tauri.conf.json`（`plugins.updater.pubkey`、`plugins.updater.endpoints`；`bundle.createUpdaterArtifacts: false`，发布工作流用 `--config` 打开）
- Create: `app/src-tauri/src/update.rs`（`check() -> Option<UpdateInfo { version, notes, date }>`、`install_and_restart()`：先走退出流程的判断——有任务在跑时发 `quit-requested` 让面板问「等待完成 / 立即停止」，用户选了之后安装并重启；`settings.auto_update` 为真时启动 1 分钟后检查一次、之后每 6 小时一次，发现新版本发系统通知并在面板顶部显示横幅）
- Modify: `app/src-tauri/src/commands.rs`（`check_update`、`install_update`）、`app/src/components/UpdateBanner.tsx`、`app/src/pages/Settings.tsx`（「检查更新」按钮与当前版本）
- Create: `docs/release/updater-key.md`（维护者如何生成与保存更新签名密钥：`pnpm --dir app tauri signer generate -w ~/.tauri/issuerelay.key`，再 `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/issuerelay.key` 与密码；公钥写进 `tauri.conf.json`；私钥不进仓库、不进聊天）
- Test: `update.rs` 的纯逻辑（版本比较只接受更新的版本、检查间隔、`auto_update` 关闭时不检查）；前端横幅与按钮的组件测试（假后端加 `checkUpdate`、`installUpdate`）

密钥：主智能体在本机生成密钥对（私钥文件权限 600，存 `~/.tauri/issuerelay.key`，不读出内容），公钥提交进 `tauri.conf.json`；把私钥上传到 GitHub secrets 是用户的事（凭据），在最终报告里给出一条命令。

- [ ] Steps: 失败测试 → 实现 → 生成密钥对并提交公钥 → 文档 → 提交 `feat(app): add signed automatic updates from GitHub Releases`。

### Task 4: 发布工作流

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `MAINTAINERS.md`（发布一节：升版本号、推标签、检查草稿、公开）、`docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md` 第 7.1 节（发布只从已推送到 `origin/main` 的提交构建，工作流核对标签提交在 `main` 上）

工作流：

- 触发：推送 `v*` 标签；`workflow_dispatch`（输入 `dry_run`，默认 true：只打包并上传为工作流产物，不建 Release）。
- `verify` 任务（ubuntu）：标签去掉 `v` 后等于工作区版本；标签提交是 `origin/main` 的祖先；`check-secrets --strict`。
- `test` 任务（macOS、Windows）：六条验证命令。
- `bundle` 任务（矩阵：`macos-latest` 目标 `universal-apple-darwin`；`windows-latest` 目标 `x86_64-pc-windows-msvc`），`needs: [verify, test]`：装 Rust 工具链与两个 Apple 目标（`rustup target add aarch64-apple-darwin x86_64-apple-darwin`）、pnpm 与 Node、`pnpm --dir app install --frozen-lockfile`；`scripts/stage-cli.*`；`tauri-apps/tauri-action`：`args: --target <目标>`，`tagName: ${{ github.ref_name }}`，`releaseName: IssueRelay ${{ github.ref_name }}`，`releaseDraft: true`，`releaseBody` 写安装说明链接；`TAURI_SIGNING_PRIVATE_KEY` 与密码有值时加 `--config '{"bundle":{"createUpdaterArtifacts":true}}'`，没有时 Release 说明里加一句「本版本不含自动更新包」；Apple 签名与公证（`APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID`）与 Windows 签名的 secrets 只在存在时传入。`dry_run` 时不传 `tagName`，改用 `actions/upload-artifact` 上传 `bundle/` 产物。
- 权限：`contents: write` 只给 `bundle` 任务。

- [ ] Steps: 写工作流 → 本地用 `act` 不现实，改为推分支后用 `workflow_dispatch`（`dry_run: true`）真跑一次 → 看产物（dmg、msi、exe 都在）→ 提交 `ci: build installers and draft a GitHub release on version tags`。

### Task 5: 安装说明

**Files:**
- Create: `docs/install.md`（中文）；Modify: `README.md`（中英文「安装」一节，链接 `docs/install.md` 与 Releases）
- 内容：下载哪个文件；macOS 打开 dmg、拖进「应用程序」；未签名时第一次打开会被拦：macOS 15 起在「系统设置 → 隐私与安全性」点「仍要打开」，或终端运行 `xattr -dr com.apple.quarantine /Applications/IssueRelay.app`；Windows SmartScreen「更多信息 → 仍要运行」；首次引导；把命令行加入 PATH；自动更新；卸载（删除应用、关掉开机自启、可选删除 `~/Library/Application Support/com.sunkailun.issuerelay` 等数据目录，Windows 对应 `%APPDATA%` 与 `%LOCALAPPDATA%` 下的目录，并在「凭据管理器」里删 `com.sunkailun.issuerelay` 的条目）；常见问题（找不到 claude / codex、需要代理、Linear 授权）。
- [ ] Steps: 写文档 → 让一个子智能体只读文档、按文档在干净的 macOS 用户环境里能否完成安装的角度审一遍（不执行安装）→ 修订 → 提交 `docs: add installation instructions`。

### Task 6: M3 实机验收

由主智能体执行：`workflow_dispatch`（`dry_run: true`）产出的 dmg 下载到本机，按 `docs/install.md` 安装到 `~/Applications`（不需要管理员权限），走一遍首次引导与一条沙箱需求；「把命令行加入 PATH」后在新终端里 `issuerelay status`；退出托盘程序后 `issuerelay start` 拉起；若用户已上传更新签名私钥：把版本临时升到 0.1.1 跑一次 dry run 验证更新包与清单（不公开）。之后升版本号到 `0.1.0` 以外的值只在维护者发布时做：本计划在验收通过后推 `v0.1.0` 标签，产出草稿 Release，由用户检查后公开。报告写 `docs/gates/<日期>-m3-acceptance.md`；「同事按说明装好并跑通」与 Windows 安装待实机。

- [ ] Steps: 下载产物 → 安装 → 验证 → 报告 → 推标签产出草稿 → 提交 `docs(m3): record the installer acceptance on macOS`。
