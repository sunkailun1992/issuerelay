# AI 目录管理规范

## 1. 总原则

- 按职责放文件，一个文件一个职责；文件明显变大时先拆分再加功能。
- 新增顶层目录或 crate 前，先在本文件登记用途。
- 生成物（`target/`、`app/node_modules/`、`app/dist/`）、本机数据、真实运行的原始输出不进仓库。

## 2. 仓库根目录

| 路径 | 用途 |
|---|---|
| `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | 工作区、锁定的依赖、工具链 |
| `AGENTS.md`、`CLAUDE.md` | AI 入口（`CLAUDE.md` 只导入 `AGENTS.md`） |
| `README.md`、`LICENSE` | 说明与许可证 |
| `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`MAINTAINERS.md` | 贡献指南、行为准则、安全政策、维护者与治理 |
| `crates/` | Rust crate |
| `app/` | （M2）Tauri 外壳（Rust crate `issuerelay-app`）与面板前端（pnpm 工程，包名 `issuerelay-panel`） |
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

`app/` 是 pnpm 前端工程（React + TypeScript + Vite + Tailwind CSS + shadcn/ui），`app/src-tauri/` 是工作区成员 `issuerelay-app`：Tauri 外壳（托盘、窗口、通知、开机自启、更新），内嵌 `issuerelay-core`。下表是 M2 完成后的布局，各文件由 M2 的各任务逐步落地。

| 路径 | 职责 |
|---|---|
| `app/package.json`、`vite.config.ts`、`tsconfig*.json`、`eslint.config.js`、`components.json`、`index.html` | 前端工程与工具链（`vite.config.ts` 只在测试运行时允许读仓库根目录，开发服务器仍只服务 `app/`） |
| `app/src/test-setup.ts`、`app/src/lib/clock.ts` | 测试公共设置（Radix 需要的 jsdom 桩、每个测试后复位固定时钟并收掉没消失的提示 toast、缺翻译键守卫）；假后端与测试共用的固定时钟 |
| `app/src/components/ui/` | shadcn/ui 生成的组件（照原样提交，不手改；不按本仓库的 ESLint 规则检查） |
| `app/src-tauri/Cargo.toml`、`build.rs`、`tauri.conf.json`、`capabilities/default.json` | Tauri crate 与配置（`build.rs` 在应用清单里登记面板的全部命令，Tauri 因此按能力文件检查本程序的命令；能力文件只给主窗口每条命令一个 `allow-<命令>`，加事件的 `listen`、`unlisten`，不给插件权限） |
| `app/src/i18n/`（含 `language-cases.json`）、`app/src-tauri/src/i18n.rs` | 五种语言的翻译文件与语言解析用例表；前端与 Rust 共用 |
| `app/src-tauri/src/{lib,main,state,startup,window,quit,logging,events,notify,tray,test_support}.rs`、`app/src-tauri/icons/tray/` | 宿主接入、启动状态、窗口、退出状态机、日志、事件转发、通知、托盘、测试辅助 |
| `app/src-tauri/src/commands.rs` | 面板调用的全部命令 |
| `app/src/lib/problem-cases.json` | 托盘与面板共用的「需要处理」判定用例表 |
| `app/src/lib/{types,backend,api,mock,mock-fixtures,clean}.ts`、`app/src/lib/clean-cases.json` | 前端数据层：类型、后端接口与错误类型、命令封装、假后端与虚构数据、显示文本清洗与用例表 |
| `app/src/lib/{theme,format,errors,problem,query-client}.ts`、`app/src/lib/backend-context.tsx`、`app/src/hooks/{queries,useEngineEvents,useLinearConnectionRefresh,subscription}.ts` | 主题、时间格式、失败的显示、「需要处理」判定、数据缓存的默认规则（调用都在本机：离线照常读写、不重试、不因焦点或重连重读，应用与测试共用一个）、后端注入、查询、事件桥、Linear 连接变了就重读依赖它的数据、订阅辅助 |
| `app/src/components/{StatusBadge,states,PageHeader,QuitDialog,StartupError,StartupGate,FirstRunGate,EngineEventsBridge,ErrorBoundary}.tsx`、`app/src/components/layout/AppShell.tsx`、`app/src/styles.css`、`app/src/App.tsx`、`app/src/main.tsx` | 面板外框、设计变量（状态色按 4.5:1 对比度定，`styles.test.ts` 核对）、状态标识、空 / 加载 / 错误状态、启动失败页、退出提示、渲染出错时的错误页 |
| `app/src/test/` | 组件测试的辅助（`render.tsx`；`time.ts` 在假时钟下按时间往前推；`colour.ts` 颜色与对比度的计算）；测试文件本身与被测文件放在一起（`*.test.ts`、`*.test.tsx`） |
| `app/src/pages/Overview.tsx`、`Issues.tsx`、`app/src/components/{JobActions,LinearStatus,IssueLink}.tsx`、`app/src/hooks/{useTick,useLang,useTriggerNames,useOpenLinear}.ts`、`app/src/lib/runs.ts`、`app/src/lib/trigger-defaults.json` | 概览、需求、共用的任务操作、Linear 连接卡与「编号即在 Linear 中打开」；按时钟重绘（时长、「几分钟前」）、界面语言、触发用的标签与状态名（设置读到之前用 `trigger-defaults.json` 的默认名，`app/src-tauri/src/commands.rs` 的测试把它对照 core 的 `TriggerNames::default()`）、打开 Linear 链接、哪条运行是需求最新的一次、运行是否还在进行 |
| `app/src/pages/Runs.tsx`、`RunDetail.tsx`、`app/src/components/{RunTimeline,RawLog,RunDuration}.tsx`、`app/src/hooks/useCopyText.ts`、`app/src/lib/timeline.ts`、`RUN_FILTERS`（在 `app/src/lib/runs.ts`） | 运行记录（按状态筛选，状态记在地址的 `?state=`）与运行详情；由运行日志的行解析出的时间线（解析与清洗在 `timeline.ts`，测试直接读 core 的两份执行者输出夹具）、原始日志（跟随末尾、复制全部）、时长、复制到剪贴板并提示结果 |
| `app/src/pages/Bindings.tsx`、`BindingDialog.tsx`、`app/src/components/{FullAccessWarning,MiddleEllipsis}.tsx`、`app/src/lib/{model-name,binding-labels}.ts`、`app/src/lib/model-name-cases.json` | 项目绑定：每个绑定一张卡片（目录过长时中间省略、上一次环境检查的结果），添加与编辑在对话框里，表单主体 `BindingForm` 首次引导也用；首次选「完全放开」时的风险提示；模型名规则（与 core 的 `config::is_valid_model_name` 同规则，`model-name-cases.json` 是 TS 测试与 `app/src-tauri/src/commands.rs` 的测试共读的一张用例表）；绑定里执行者与档位的显示名 |
| `app/src/pages/Doctor.tsx`、`Settings.tsx`、`Help.tsx`、`app/src/components/{ExecutorCheck,WorkspaceCheck}.tsx`、`app/src/lib/mask.ts`、`app/src/lib/mask-cases.json` | 环境检查、设置、使用说明；打码用例表（Rust 与 TS 共用） |
| `app/src/pages/onboarding/` | 首次引导 |
| `app/public/` | 静态资源（`icon.svg`），原样复制进 `app/dist` |
| `app/icons-src/`、`app/scripts/icons.mjs`、`app/src-tauri/icons/` | 图标源文件、生成脚本与产物；`app/icons-src/build/` 是渲染出的中间图，git 忽略 |
| `app/playwright.config.ts`、`app/tests/` | 截图比对（Playwright，只在本机运行）；`app/tests/*.spec.ts` 不是 Vitest 的测试（Vitest 只收 `app/src/**/*.test.{ts,tsx}`），`app/test-results/` 与 `app/playwright-report/` 是输出，git 忽略 |
| `app/dist/`、`app/node_modules/`、`app/src-tauri/gen/`、`app/src-tauri/permissions/autogenerated/` | 前端产物、依赖、Tauri 生成的 schema 与命令权限（每次构建由 `build.rs` 重新生成）；全部 git 忽略。应用 crate 的发布构建内嵌 `app/dist`，先 `pnpm --dir app build` 再跑 cargo |

约定：
- 依赖版本由 `app/pnpm-lock.yaml` 和 `Cargo.lock` 锁定；pnpm 版本由 `app/package.json` 的 `packageManager` 固定。
- 翻译文件的位置与格式见 `app/src/i18n/`（不是 `locales/`）。

## 5. docs/

| 路径 | 用途 |
|---|---|
| `docs/ai-coding/` | AI 编码规范（本目录） |
| `docs/superpowers/specs/` | 设计文档，命名 `<日期>-<主题>-design.md` |
| `docs/superpowers/plans/` | 实施计划，命名 `<日期>-<主题>.md` |
| `docs/gates/` | 实机验收报告 `<日期>-<主题>.md`；脱敏样本放 `docs/gates/samples/<里程碑>/` |
| `docs/gates/screenshots/` | 面板截图（`docs/gates/screenshots/m2/`，Playwright 比对用的基准图）；只含虚构数据，真实验收截图只留在本机 |
| `docs/design/` | 设计素材的预览（`docs/design/icons/`：图标三套方案的预览） |

## 6. scripts/ 与 .github/

- `scripts/check-secrets.sh`：扫描密钥和本机信息。新脚本的文件头写清用途、用法、退出码。
- `.github/workflows/ci.yml`：在 macOS 与 Windows 上先装前端依赖、跑前端检查与测试并构建面板，再跑 Rust 的格式、静态检查、测试；另在 Ubuntu 上扫描密钥。
- `.github/copilot-instructions.md`：只转发到 `AGENTS.md`。
- `.github/CODEOWNERS`：自动请求评审的负责人；高风险路径逐条列出。
- `.github/pull_request_template.md`、`.github/ISSUE_TEMPLATE/`：PR 与 issue 模板，带「已脱敏」确认项。
- `.github/dependabot.yml`：每周检查 cargo、GitHub Actions 与 `/app` 的 npm 依赖更新；版本更新延迟 7 天（`cooldown`，供应链防护），安全更新不延迟。
- 仓库设置（规则集、合并方式、安全功能）不在仓库文件里，现状记在 `MAINTAINERS.md` 的「分支保护」一节；改设置时同步改那一节。

## 7. 目录变更检查

1. 新文件放在本规范规定的位置了吗？
2. 新增的目录或 crate 在本文件登记了吗？
3. 有没有把生成物、本机数据、原始输出加进仓库？
4. 测试与夹具和被测代码在同一个 crate 吗？
