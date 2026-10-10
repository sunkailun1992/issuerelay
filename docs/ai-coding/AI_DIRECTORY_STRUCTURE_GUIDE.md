# AI 目录管理规范

## 1. 总原则

- 按职责放文件，一个文件一个职责；文件明显变大时先拆分再加功能。
- 新增顶层目录或 crate 前，先在本文件登记用途。
- 生成物（`target/`）、本机数据、真实运行的原始输出不进仓库。

## 2. 仓库根目录

| 路径 | 用途 |
|---|---|
| `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | 工作区、锁定的依赖、工具链 |
| `AGENTS.md`、`CLAUDE.md` | AI 入口（`CLAUDE.md` 只导入 `AGENTS.md`） |
| `README.md`、`LICENSE` | 说明与许可证 |
| `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`MAINTAINERS.md` | 贡献指南、行为准则、安全政策、维护者与治理 |
| `crates/` | Rust crate |
| `app/` | （M2）Tauri 外壳与面板前端 |
| `docs/` | 规范、设计、计划、验收报告 |
| `scripts/` | 仓库脚本 |
| `spikes/` | 一次性验证工程（独立 cargo 工程，不进工作区） |
| `.github/` | CI、贡献模板与工具入口 |

## 3. crates/

| crate | 内容 |
|---|---|
| `core`（`issuerelay-core`） | 引擎库；`src/` 每个模块一个文件或目录；`src/bin/fake_agent.rs` 只供测试 |
| `cli`（`issuerelay-cli`） | 库 `src/lib.rs`（`format`、`mcp`）与二进制 `src/main.rs`（`issuerelay`） |
| `devhost`（`issuerelay-devhost`） | 开发用无界面宿主，`publish = false`，不进安装包 |

测试放置：
- 单元测试写在源文件末尾的 `#[cfg(test)] mod tests`，位于所有实现代码之后。
- 集成测试放在 `crates/<crate>/tests/*.rs`；共用的假实现放 `tests/common/`（文件头写 `#![allow(dead_code)]`）。
- 夹具放 `crates/<crate>/tests/fixtures/`，只放脱敏后的内容。

## 4. app/（M2 起）

- `app/src-tauri/`：Tauri 外壳（托盘、窗口、通知、开机自启、更新、IPC 服务端），内嵌 `issuerelay-core`。
- `app/src/`：React + TypeScript 面板；翻译文件放 `app/src/locales/<语言>.json`。

## 5. docs/

| 路径 | 用途 |
|---|---|
| `docs/ai-coding/` | AI 编码规范（本目录） |
| `docs/superpowers/specs/` | 设计文档，命名 `<日期>-<主题>-design.md` |
| `docs/superpowers/plans/` | 实施计划，命名 `<日期>-<主题>.md` |
| `docs/gates/` | 实机验收报告 `<日期>-<主题>.md`；脱敏样本放 `docs/gates/samples/<里程碑>/` |

## 6. scripts/ 与 .github/

- `scripts/check-secrets.sh`：扫描密钥和本机信息。新脚本的文件头写清用途、用法、退出码。
- `.github/workflows/ci.yml`：在 macOS 与 Windows 上跑格式、静态检查、测试，并扫描密钥。
- `.github/copilot-instructions.md`：只转发到 `AGENTS.md`。
- `.github/CODEOWNERS`：自动请求评审的负责人；高风险路径逐条列出。
- `.github/pull_request_template.md`、`.github/ISSUE_TEMPLATE/`：PR 与 issue 模板，带「已脱敏」确认项。
- `.github/dependabot.yml`：每周检查 cargo 与 GitHub Actions 依赖更新；版本更新延迟 7 天（`cooldown`，供应链防护），安全更新不延迟。
- 仓库设置（规则集、合并方式、安全功能）不在仓库文件里，现状记在 `MAINTAINERS.md` 的「分支保护」一节；改设置时同步改那一节。

## 7. 目录变更检查

1. 新文件放在本规范规定的位置了吗？
2. 新增的目录或 crate 在本文件登记了吗？
3. 有没有把生成物、本机数据、原始输出加进仓库？
4. 测试与夹具和被测代码在同一个 crate 吗？
