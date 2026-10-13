# IssueRelay M2 第二部分：桌面应用 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 做出 IssueRelay 的桌面托盘程序：Tauri 2 外壳内嵌 M2 第一部分的 `core::host`，托盘菜单与状态图标、面板全部页面、首次引导、五种界面语言、系统通知、开机自启、图标，并在 macOS 上用真实 Linear 与 Claude / Codex 实机验收。

**Architecture:** `app/` 是一个 pnpm 前端工程（React + TypeScript + Vite + Tailwind CSS + shadcn/ui），`app/src-tauri` 是工作区里的 Rust crate `issuerelay-app`：启动时异步拉起 `Host`，经 Tauri 命令把引擎接口交给面板，把引擎事件转成前端事件、系统通知和托盘刷新。面板只调本程序自己的命令，所有插件能力（打开链接、选目录、通知、开机自启）都在 Rust 侧调用，前端不直接拿插件权限。浏览器里打开前端时用内置的假后端（虚构数据），开发、组件测试和截图都不碰真实数据。

**Tech Stack:** Tauri 2（插件 single-instance、notification、autostart、opener、dialog）、React 19、TypeScript（strict）、Vite、Tailwind CSS 4、shadcn/ui（Radix）、lucide-react、TanStack Query 5、React Router（HashRouter）、i18next / react-i18next、sonner、Vitest + Testing Library + jsdom、ESLint 9（typescript-eslint、react-hooks）、Playwright（只用于本机截图比对）。

**Spec:** `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 4、4.1、5.6、6.2、8、9、10.3、14、15、16、17、18 节）；M2 第一部分计划 `docs/superpowers/plans/2026-10-13-issuerelay-m2-core.md`（本计划依赖它产出的接口）。

## Global Constraints

- 第一部分的全部全局约束继续有效（硬规则、两平台、测试不碰网络与真实钥匙串、虚构测试数据、公开仓库红线、英文注释中文文档、四条验证命令、英文 Conventional Commits 与 `Co-Authored-By` 写实际模型、分支与 PR、禁止强推）。
- 验证命令扩展为六条（设计第 16 节）：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`pnpm --dir app lint`、`pnpm --dir app test`、`bash scripts/check-secrets.sh --strict`（无输出）。
- 「标记完成」「重新运行」「停止」只经本程序的 Tauri 命令在进程内调用引擎（`Engine::mark_done`、`retry_job`、`cancel_job`）；IPC 方法表、命令行、MCP 控制服务保持不提供。
- 前端不拿任何插件权限：`capabilities/default.json` 只给本程序登记在 app manifest（`app/src-tauri/build.rs`）里的面板命令与面板用到的两条 core 事件权限（`core:event:allow-listen`、`core:event:allow-unlisten`），由 `app/src-tauri/src/lib.rs` 的测试与 `app/src/lib/api.test.ts` 逐条核对（2026-10-13 Task 4 起，取代最初的 `core:default`）；新增命令时同步改 app manifest、能力文件与测试；打开链接、选目录、通知、开机自启都由 Rust 侧命令完成。浏览器只打开 Linear 的链接：用 `url::Url` 解析后，协议必须是 `https`、主机必须是 `linear.app` 或它的子域名（例如 OAuth 授权页所在的 `mcp.linear.app`），且不带用户名或密码、主机里没有空标签（如结尾的点）；交给系统打开的是检查时解析出的规范化地址（`Url::as_str`），从不是传进来的原文（macOS 的 NSURL 与 `url` 对同一段原文可能读出不同的主机，例如 `https://linear.app\@evil.example/`）；`open_linear` 命令与 Linear 登录时打开授权页的回调共用这一个检查与打开方式（Task 4 的 `linear_url`、`open_linear_link`），其他链接一律拒绝并记日志。
- 来自 Linear 与 AI 的文本只作为 React 文本节点渲染（自动转义），不用 `dangerouslySetInnerHTML`。执行者与 AI 的显示文本（时间线、原始日志、摘要）一律经 `cleanDisplayText`（Task 4）：去掉 ANSI 转义序列（CSI、以 BEL 或 ST 结尾的 OSC、两字符 ESC 序列），其余控制字符变空格，去掉 core `text::is_invisible` 去掉的字符；用例表 `app/src/lib/clean-cases.json` 由 Rust（对照 core 的 `clean_text`，字符规则在 core 里只有 `text::strip_control` 一份实现）与 TypeScript 两边的测试共同核对；用例里的不可见字符一律写成转义。
- 引擎给的英文原文（任务原因、`last_error`、环境检查的 `problems`、`LinearSetup` 的说明、命令返回的错误链）从不单独出现：每处都先显示一条由结构化数据（状态、代码、布尔值）得出的已翻译标题，原文清洗后作为次要的细节行放在下面（Task 5 的 `describeError`、`DetailLine`）。系统通知只带已翻译的标题加需求编号与标题，从不带引擎原文。需求页的等待原因按代码翻译（Task 4 的 `WaitReasonView`）。
- 面板与托盘的每段文案都走翻译键；五种语言（`zh-CN`、`zh-TW`、`en`、`ja`、`ko`）的键必须一一对应（CI 检查），组件里用到而 `en` 没有的键让测试失败（Task 2 的缺键守卫）。默认跟随系统语言：系统语言一律取 Rust 侧 `sys_locale` 报告的值（面板经 `AppInfo.system_locale` 拿到），托盘、通知与面板因此总是同一种语言；设置里随时切换并立即生效（含托盘菜单与通知）。
- 跟随系统浅色 / 深色；颜色、间距、字号、圆角统一用设计变量（含布局常量 `--sidebar-width`、`--content-max-width`、`--page-padding` 与状态色 `--state-*`，页面规格里不写死像素）；任务状态用固定的颜色和图标（见 Task 5）；每个页面的规格都写明加载、空、错误三种状态，每个页面至少有一个用失败的假后端调用驱动的错误状态测试；不出现裸露的报错文本。
- 不静默吞错：应用 crate 里每个被忽略的 `Result` 都记日志（托盘、窗口、存储调用 `tracing::warn!`；发给可能已关闭的网页视图的事件 `debug!`）；日志文件与 stderr 都经 core 的逐行清洗（`issuerelay_core::logscrub`，Task 3）。
- 应用标识 `com.sunkailun.issuerelay`，产品名 `IssueRelay`，主程序文件名固定为 `issuerelay-app`（`tauri.conf.json` 的 `mainBinaryName`），窗口默认 1100×720、最小 900×600。
- 截图、示例与测试里的数据全部虚构（`DEMO-N`、`Demo Project`、`Jane Doe`、`~/code/demo-app`）；真实验收截图只留在本机，不进仓库。
- 依赖版本：Tauri 及其插件取当前 2.x 最新版，前端依赖取当前最新稳定版；用 `cargo add` / `pnpm add` 解析后锁定。每个在 `cargo add` / `pnpm add` 之后的提交步骤都显式写出 `Cargo.lock` 与 `app/pnpm-lock.yaml`。`/app` 的 npm 依赖登记进 `.github/dependabot.yml`（与现有条目同样每周检查、版本更新延迟 7 天）。Tauri、shadcn/ui、Radix、i18next、tracing 等库的 API 名称与行为以实际解析到的版本文档（docs.rs、v2.tauri.app 等）为准，与本计划写法不同时照文档写并在报告里说明。
- 本计划只在三处改 core（各自先写 core 的失败测试、守 core 的规则）：Task 3 把 devhost 的日志清洗移进 `issuerelay_core::logscrub`；Task 4 给 `StatusSnapshot` 加 `held`（暂缓任务的重试时间）、给 `IssueStatus::Waiting` 换成结构化的 `WaitReasonView`。命令行照旧打印 `WaitReason::describe()`，IPC 方法表不变。
- 分支 `feat/m2-app`（从合入第一部分之后的 `main` 切出），经 PR 合入。

## Review Focus

1. 宿主还没起来（启动要几秒）时面板就发命令：命令要等宿主就绪（「就绪」指 `Host::start` 已返回：它不再等 Linear，首次连接在后台进行，期间 `LinearSetup` 为 `connecting`，面板显示「正在连接 Linear…」、托盘不算出问题，见第一部分 Task 9 的 AR51）；启动状态可以查询（`startup_state` 命令，`AppState` 里的 `StartupState`），不只靠一次性事件，所以晚打开的面板也知道启动失败了；任何一种启动失败（另一个宿主已在运行、运行记录打不开、其他）面板都显示按代码翻译的标题、清洗过的细节和「退出」按钮（Task 3 的 `state`/`startup` 测试、Task 5 的 `App` 与 `StartupError` 测试）。
2. 有任务在跑时退出（托盘「退出」、Cmd+Q、系统要求退出）：必须先问「等待完成 / 立即停止」，选了等待以后仍能改为立即停止（Rust 侧在等待中的 `Host::stop(WaitForRuns)` 还没返回时调 `Host::stop(StopRuns)`，两者都返回），窗口关掉也不能让程序在任务没收尾时退出（Task 3 的退出状态机测试、Task 13 第 4 步实机）。
3. 引擎事件来得比面板处理快（大量任务同时变化）：事件丢失时面板要整体刷新，不能停在旧状态（Task 3 的 `forward` 测试：落后的接收端只发一次 `engine-resync` 并继续转发；Task 5 的事件桥测试）。
4. 第一次用、什么都没配：没有登录、没有绑定、执行者没装时，每个页面都要给出下一步该做什么，而不是空白或报错；后端调用失败时每个页面都有错误状态（Task 5–10 的空状态与错误状态测试）。
5. 系统语言是五种之外的语言（例如德语）或带地区、文字的写法（`zh-Hant-HK`、`zh-Hans-HK`、`ja-JP`）：先看文字子标签再看地区，解析到正确的界面语言，没有的就用英文；Rust 与 TypeScript 读同一张用例表 `language-cases.json`，面板用 Rust 报告的系统语言（Task 2）。
6. 引擎的英文原文不单独出现：概览提醒、运行原因、环境检查问题、启动失败、操作失败的提示都是「已翻译的标题＋清洗过的细节行」，通知里没有原文（Task 3 的通知测试、Task 5 的 `describeError`、Task 6–9 的页面测试）。

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `app/package.json`、`vite.config.ts`、`tsconfig*.json`、`eslint.config.js`、`components.json`、`index.html` | 前端工程与工具链（`components.json` 的样式路径 Task 5 改成 `src/styles.css`） | 1、5 |
| `app/src/test-setup.ts`、`app/src/lib/clock.ts` | 测试公共设置（Radix 需要的 jsdom 桩、每个测试后复位固定时钟、缺翻译键守卫）；假后端与测试共用的固定时钟 | 1、2 |
| `app/src/components/ui/` | shadcn/ui 生成的组件（照原样提交，不手改） | 1 |
| `app/src-tauri/Cargo.toml`、`build.rs`、`tauri.conf.json`、`capabilities/default.json` | Tauri crate 与配置 | 1 |
| `.github/dependabot.yml` | `/app` 的 npm 依赖检查 | 1 |
| `app/src/i18n/`（含 `language-cases.json`）、`app/src-tauri/src/i18n.rs` | 五种语言的翻译文件与语言解析用例表；前端与 Rust 共用 | 2 |
| `crates/core/src/text.rs`、`crates/core/src/logscrub.rs`、`crates/devhost/src/main.rs` | 字符规则的唯一实现 `text::strip_control`（`clean_text` 建在它上面）与日志逐行清洗：从 devhost 移进 core，devhost 与桌面应用共用 | 3 |
| `app/src-tauri/src/{lib,main,state,startup,window,quit,logging,events,notify,tray,test_support}.rs`、`app/src-tauri/icons/tray/` | 宿主接入、启动状态、窗口、退出状态机、日志、事件转发、通知、托盘、测试辅助 | 3 |
| `app/src/lib/problem-cases.json` | 托盘与面板共用的「需要处理」判定用例表 | 3、5 |
| `app/src-tauri/src/commands.rs` | 面板调用的全部命令（Task 3 先放空壳） | 3、4 |
| `crates/core/src/{engine,trigger}.rs` | 暂缓任务的重试时间（`StatusSnapshot.held`）、结构化等待原因（`WaitReasonView`） | 4 |
| `app/src/lib/{types,backend,api,mock,mock-fixtures,clean}.ts`、`app/src/lib/clean-cases.json` | 前端数据层：类型、后端接口与错误类型、命令封装、假后端与虚构数据、显示文本清洗与用例表 | 4 |
| `app/src/lib/{theme,format,errors,problem}.ts`、`app/src/lib/backend-context.tsx`、`app/src/hooks/{queries,useEngineEvents,subscription}.ts` | 主题、时间格式、失败的显示、「需要处理」判定、后端注入、查询、事件桥、订阅辅助 | 5 |
| `app/src/components/{StatusBadge,states,PageHeader,QuitDialog,StartupError,StartupGate,FirstRunGate,EngineEventsBridge}.tsx`、`app/src/components/layout/AppShell.tsx`、`app/src/styles.css`、`app/src/App.tsx`、`app/src/main.tsx`、`app/src/test/render.tsx` | 面板外框、设计变量、状态标识、空 / 加载 / 错误状态、启动失败页、退出提示、测试辅助 | 5 |
| `app/src/pages/Overview.tsx`、`Issues.tsx`、`app/src/components/{JobActions,LinearStatus}.tsx` | 概览、需求、共用的任务操作与 Linear 连接卡 | 6 |
| `app/src/pages/Runs.tsx`、`RunDetail.tsx`、`app/src/lib/timeline.ts` | 运行记录与时间线 | 7 |
| `app/src/pages/Bindings.tsx`、`BindingDialog.tsx`、`app/src/components/FullAccessWarning.tsx`、`app/src/lib/model-name.ts`、`app/src/lib/model-name-cases.json` | 项目绑定；模型名用例表（Rust 与 TS 共用） | 8 |
| `app/src/pages/Doctor.tsx`、`Settings.tsx`、`Help.tsx`、`app/src/components/{ExecutorCheck,WorkspaceCheck}.tsx`、`app/src/lib/mask.ts`、`app/src/lib/mask-cases.json` | 环境检查、设置、使用说明；打码用例表（Rust 与 TS 共用） | 9 |
| `app/src/pages/onboarding/` | 首次引导 | 10 |
| `app/icons-src/`、`app/scripts/icons.mjs`、`app/src-tauri/icons/`、`app/public/icon.svg`、`docs/design/icons/` | 图标源文件、生成脚本、产物与三套方案的预览 | 11 |
| `app/playwright.config.ts`、`app/tests/screens.spec.ts`、`docs/gates/screenshots/m2/` | 截图与截图比对（只在本机运行） | 12 |
| `docs/gates/<日期>-m2-acceptance.md` | M2 实机验收报告 | 13 |

---

### Task 1: 前端工程与 Tauri crate 骨架

**Files:**
- Create: `app/`（由 Vite 的 `react-ts` 模板生成后调整，含 `package.json`、`vite.config.ts`、`tsconfig.app.json`、`eslint.config.js`、`components.json`、`index.html`）、`app/src-tauri/`（由 `tauri init` 生成后调整）
- Create: `app/src/test-setup.ts`（Vitest 公共设置）、`app/src/lib/clock.ts`（固定时钟）
- Modify: `Cargo.toml`（工作区成员加 `app/src-tauri`）、`.gitignore`、`.github/workflows/ci.yml`、`.github/dependabot.yml`（`/app` 的 npm 条目）
- Modify: `AGENTS.md`、`CONTRIBUTING.md`、`README.md`、`docs/ai-coding/AI_CODING_GUIDE.md`、`docs/ai-coding/TESTING_SPEC.md`、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`、`.github/pull_request_template.md`（验证命令加上前端两条）
- Test: `app/src/App.test.tsx`

**Interfaces:**
- Produces: `pnpm --dir app {dev,build,lint,test,tauri}` 五个脚本；Rust crate `issuerelay-app`（lib 名 `issuerelay_app`，入口 `issuerelay_app::run()`，主程序文件名 `issuerelay-app`）；开发端口 1420；`app/dist` 是前端产物目录（git 忽略）；Vitest 全局 API（`test`、`expect`、`vi` 不必导入）与每个测试后的自动清理；`src/test-setup.ts`；`lib/clock.ts` 的 `MOCK_NOW`、`setFixedClock(at: Date | null)`、`now()`。
- Consumes: 第一部分的 `issuerelay-core`（路径依赖）。

- [ ] **Step 1: 生成前端工程**

```bash
# 在仓库根目录运行
pnpm create vite@latest app --template react-ts
pnpm --dir app install
pnpm --dir app add react-router @tanstack/react-query i18next react-i18next lucide-react sonner clsx tailwind-merge class-variance-authority
pnpm --dir app add -D tailwindcss @tailwindcss/vite @tauri-apps/cli vitest jsdom @testing-library/react @testing-library/user-event @testing-library/jest-dom @types/node
pnpm --dir app add @tauri-apps/api
```

`app/package.json`：`"name": "issuerelay-panel"`、`"version": "0.1.0"`、`"private": true`，加 `"packageManager"`（取本机 `pnpm --version` 的值，例如 `"pnpm@10.28.2"`），`scripts` 为：

```json
{
  "dev": "vite",
  "build": "tsc -b && vite build",
  "lint": "tsc -b --noEmit && eslint .",
  "test": "vitest run",
  "tauri": "tauri"
}
```

（模板的 `preview` 保留。若 `tsc -b --noEmit` 与模板的项目引用冲突，就把 `lint` 写成 `tsc -p tsconfig.app.json --noEmit && tsc -p tsconfig.node.json --noEmit && eslint .`。）

`app/vite.config.ts`：

```ts
/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// Port 1420 is what tauri.conf.json's devUrl points at; Tauri needs it fixed.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "src") } },
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022", sourcemap: false },
  test: {
    // Global test/expect/vi, which also turns on Testing Library's cleanup after each test.
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    // Only the component and unit tests: Task 12's Playwright spec (tests/*.spec.ts) is not Vitest's.
    include: ["src/**/*.test.{ts,tsx}"],
    restoreMocks: true,
  },
});
```

（首行的 `/// <reference types="vitest/config" />` 让 `tsc -b` 认得 `test` 字段；若解析到的 Vitest 版本要求改从 `vitest/config` 导入 `defineConfig`，照它的文档写。）`tsconfig.app.json` 加 `"baseUrl": "."`、`"paths": { "@/*": ["src/*"] }`、`"resolveJsonModule": true`（模板已有就不重复），在 `"types"` 里加上 `"vitest/globals"` 与 `"@testing-library/jest-dom"`（保留模板原有的 `"vite/client"`），保持 `strict: true`。`globals: true` 让 Testing Library 的自动清理生效，公共设置里不必再调 `cleanup`。

`app/src/lib/clock.ts`（假后端与测试共用的时钟；Task 4 的假后端、Task 5 的时间格式都读它）：

```ts
/** The mock backend's fixed "now", so relative times in tests and screenshots do not drift. */
export const MOCK_NOW = new Date("2026-10-12T09:00:00Z");

let fixed: Date | null = null;

/** Fixes `now()` at `at`; `null` goes back to the real clock. The test setup resets it after each test. */
export function setFixedClock(at: Date | null): void {
  fixed = at;
}

export function now(): Date {
  return fixed ?? new Date();
}
```

`app/src/test-setup.ts`（后续任务需要的 jsdom 桩一次放齐；Task 2 再加缺翻译键守卫）：

```ts
import "@testing-library/jest-dom/vitest";
import { setFixedClock } from "@/lib/clock";

// Radix (Select, Switch, ScrollArea, Dialog, RadioGroup) measures elements, captures the pointer
// and scrolls options into view; jsdom implements none of these.
if (!("ResizeObserver" in globalThis)) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}
Element.prototype.hasPointerCapture = () => false;
Element.prototype.setPointerCapture = () => {};
Element.prototype.releasePointerCapture = () => {};
Element.prototype.scrollIntoView = () => {};
// The theme follows `prefers-color-scheme`; jsdom has no matchMedia.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList,
});

afterEach(() => {
  // A test that fixed the clock (the mock backend does) never leaks it into the next one, even
  // when its assertions failed.
  setFixedClock(null);
});
```

（Radix 在 jsdom 里还缺别的桩时，照解析到的版本的报错补进这个文件，并在报告里说明。）

初始化 shadcn/ui（new-york 风格、neutral 基色、CSS 变量），并加入后续页面要用的组件（`collapsible` 给 Task 7 的可展开卡片与 Task 8 的提示词覆盖，`radio-group` 给 Task 10 的语言选择）：

```bash
pnpm --dir app dlx shadcn@latest init
pnpm --dir app dlx shadcn@latest add button card badge dialog alert-dialog input label select switch tabs tooltip scroll-area separator table textarea alert dropdown-menu skeleton sonner collapsible radio-group
```

（交互提问一律选：TypeScript、new-york、neutral、CSS variables、`@/components`、`@/lib/utils`。生成的 `src/components/ui/*` 照原样提交，不手改。）

ESLint：模板自带的 `eslint.config.js` 保留 typescript-eslint 与 react-hooks 规则，加 `ignores: ["dist", "src-tauri", "src/components/ui"]`（生成的 UI 组件不按本仓库的规则改）。

- [ ] **Step 2: 最小页面与第一个测试**

`app/src/App.tsx` 暂时只渲染一个标题（后续任务替换）：

```tsx
export default function App() {
  return (
    <main className="flex h-screen items-center justify-center bg-background text-foreground">
      <h1 className="text-2xl font-semibold">IssueRelay</h1>
    </main>
  );
}
```

`app/src/App.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import App from "./App";

test("the panel renders its title", () => {
  render(<App />);
  expect(screen.getByRole("heading", { name: "IssueRelay" })).toBeInTheDocument();
});
```

删掉模板的示例资源（`src/assets/react.svg`、`public/vite.svg`、计数器样式）。`index.html` 的 `<title>` 改为 `IssueRelay`，`lang` 先写 `en`（Task 2 起由 i18n 设置）。

Run: `pnpm --dir app lint && pnpm --dir app test && pnpm --dir app build`
Expected: 全部通过，测试 1/1。

- [ ] **Step 3: Tauri crate**

```bash
pnpm --dir app tauri init --ci --app-name IssueRelay --window-title IssueRelay --frontend-dist ../dist --dev-url http://localhost:1420 --before-dev-command "pnpm dev" --before-build-command "pnpm build"
```

然后调整生成的文件：

`app/src-tauri/Cargo.toml`：

```toml
[package]
name = "issuerelay-app"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
publish = false
description = "IssueRelay desktop tray app"

[lib]
name = "issuerelay_app"
crate-type = ["rlib"]

[[bin]]
name = "issuerelay-app"
path = "src/main.rs"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
issuerelay-core = { path = "../../crates/core" }
tauri = { version = "2", features = ["tray-icon", "image-png"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

（版本号用 `cargo add` 解析到的 2.x 最新版；后续任务再加插件与其他依赖。只做桌面，不需要移动端的 `staticlib`、`cdylib`。）

根 `Cargo.toml` 的 `members` 改为 `["crates/*", "app/src-tauri"]`。

`app/src-tauri/build.rs`：

```rust
fn main() {
    // `tauri::generate_context!` embeds `../dist`. A plain `cargo build` or `cargo test` of the
    // workspace runs without the panel having been built, so give it a placeholder page; `pnpm
    // build` (and `pnpm tauri build`) replace it with the real panel.
    let dist = std::path::Path::new("../dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("cannot create ../dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>IssueRelay</title><p>Run <code>pnpm --dir app build</code> to build the panel.</p>",
        )
        .expect("cannot write the placeholder panel");
    }
    tauri_build::build()
}
```

`app/src-tauri/src/main.rs`：

```rust
// No console window next to the tray app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    issuerelay_app::run();
}
```

`app/src-tauri/src/lib.rs`（Task 3 扩充）：

```rust
//! IssueRelay's desktop app: the tray icon and the panel around the engine.

/// Starts the app and runs its event loop until it exits.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("IssueRelay could not start");
}
```

`app/src-tauri/tauri.conf.json` 的关键字段：

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "IssueRelay",
  "mainBinaryName": "issuerelay-app",
  "identifier": "com.sunkailun.issuerelay",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "IssueRelay",
        "width": 1100,
        "height": 720,
        "minWidth": 900,
        "minHeight": 600,
        "center": true,
        "visible": false
      }
    ],
    "security": {
      "csp": "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost"
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"]
  }
}
```

（窗口启动时隐藏，由 Task 3 决定何时显示；`tauri init` 生成的默认图标先保留，Task 11 替换。`mainBinaryName` 把安装包里的主程序文件名固定为 `issuerelay-app`：Task 13 与 M3 把命令行程序 `issuerelay` 放在同一目录，不区分大小写的磁盘上主程序若叫 `IssueRelay` 就会与它撞名。键名以解析到的 Tauri 版本的配置文档为准。这里不写 `version`：安装包的版本跟随 Cargo（`version.workspace`），`AppInfo.version` 也读 `CARGO_PKG_VERSION`，M3 的发布流程依赖这一点。`app/package.json` 的 `version` 只是私有前端包的版本，不是应用版本。）

`app/src-tauri/capabilities/default.json`：

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "The panel calls only IssueRelay's own commands and listens to its events.",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

（这是 Task 1 落地时的写法；Task 4 的第二个提交起，权限收窄为 app manifest 里的面板命令加 `core:event:allow-listen`、`core:event:allow-unlisten`，见全局约束。）

`.gitignore` 加 `app/node_modules/`、`app/dist/`、`app/src-tauri/gen/`，以及后续任务的生成物 `app/icons-src/build/`（Task 11 渲染的 1024px PNG）、`app/test-results/` 与 `app/playwright-report/`（Task 12 的 Playwright 输出）（若 `tauri init` 生成的 `.gitignore` 已覆盖就不重复）。

Run: `cargo build -p issuerelay-app`（没有先 `pnpm build` 时也要能编过）、`pnpm --dir app tauri build --debug --no-bundle`
Expected: 两者都成功。

- [ ] **Step 4: CI**

`.github/workflows/ci.yml` 的 `test` 任务在 `rust-cache` 之后、`cargo fmt` 之前加：

```yaml
      - uses: pnpm/action-setup@v4
        with:
          package_json_file: app/package.json
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: app/pnpm-lock.yaml
      - run: pnpm --dir app install --frozen-lockfile
      - run: pnpm --dir app lint
      - run: pnpm --dir app test
      - run: pnpm --dir app build
```

（`pnpm build` 在 cargo 之前跑，CI 里的 Tauri crate 嵌的是真实面板。）

`.github/dependabot.yml` 在现有两个条目后加 `/app` 的 npm 条目，节奏与延迟同现有条目：

```yaml
  - package-ecosystem: npm
    directory: /app
    schedule:
      interval: weekly
    # Delay version updates by 7 days as a supply-chain safeguard; security updates are not delayed.
    cooldown:
      default-days: 7
    open-pull-requests-limit: 5
    commit-message:
      prefix: chore
      include: scope
    groups:
      npm-minor-and-patch:
        update-types: [minor, patch]
```

- [ ] **Step 5: 文档**

- `AGENTS.md`、`CONTRIBUTING.md`、`docs/ai-coding/AI_CODING_GUIDE.md` 第 2 节、`.github/pull_request_template.md` 的验证命令加 `pnpm --dir app lint` 与 `pnpm --dir app test`；开发环境要求加 Node.js 22 以上与 pnpm（版本见 `app/package.json` 的 `packageManager`）。
- `README.md` 开发一节加：`pnpm --dir app install`、`pnpm --dir app tauri dev`（启动桌面应用开发版）、`pnpm --dir app dev`（只在浏览器里看面板，用假数据）。
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 4 节写明 `app/` 的布局，逐行照本计划「文件结构」表（含 `app/public/`、`app/icons-src/`、`app/tests/`、`app/src/test/`、`app/src/test-setup.ts`）；第 5 节登记 `docs/design/`（Task 11 的图标预览）与 `docs/gates/screenshots/`（Task 12 的截图）。`docs/ai-coding/TESTING_SPEC.md` 分层表加「前端组件测试（Vitest + Testing Library，公共设置在 `app/src/test-setup.ts`）」一行和「截图比对（Playwright，只在本机运行）」一行。

Run: 六条验证命令
Expected: 全部通过。

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore .github app app/pnpm-lock.yaml AGENTS.md CONTRIBUTING.md README.md docs
git commit -m "feat(app): scaffold the Tauri app and the React panel"
```

---

### Task 2: 五种界面语言（前端与 Rust 共用翻译文件）

设计第 9.4 节。翻译文件是前端与 Rust 的唯一来源：前端经 i18next 读取，Rust 在编译时用 `include_str!` 嵌入同一批文件，供托盘菜单和通知使用（退出提示与启动失败页是面板文案，Rust 不读）。语言解析的用例只有一份 `language-cases.json`，两边的测试都读它。

**Files:**
- Create: `app/src/i18n/index.ts`、`app/src/i18n/language.ts`、`app/src/i18n/language-cases.json`、`app/src/i18n/locales/{zh-CN,zh-TW,en,ja,ko}.json`、`app/src/i18n/i18n.test.ts`
- Create: `app/src-tauri/src/i18n.rs`
- Modify: `app/src-tauri/src/lib.rs`（`pub mod i18n;`）、`app/src-tauri/Cargo.toml`（`sys-locale`）、`Cargo.lock`
- Modify: `app/src/main.tsx`（初始化 i18n）、`app/src/test-setup.ts`（缺翻译键守卫）
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 9.4 节）

**Interfaces:**
- Produces:
  - TS：`type Lang = "zh-CN" | "zh-TW" | "en" | "ja" | "ko"`、`LANGUAGES: Lang[]`、`resolveLanguage(setting: string, systemLocale: string | undefined): Lang`、`initI18n(lang: Lang, options?: InitOptions): Promise<void>`（`options` 只给测试的缺键守卫用）、`initI18nOnce(lang: Lang): Promise<void>`（已初始化时只切换语言）、`setLanguage(lang: Lang): Promise<void>`
  - Rust（都是 `pub`，模块为 `pub mod i18n`，Task 3 接上之前也不算死代码）：`pub const i18n::LANGUAGES: [&str; 5]`、`pub const i18n::USED_KEYS: &[&str]`、`pub fn i18n::resolve_language(setting: &str, system: Option<&str>) -> &'static str`、`pub struct i18n::Translator<'a>`：`Translator::new(lang: &str) -> Translator<'static>`、`Translator::for_settings(settings: &Settings) -> Translator<'static>`、`t(&self, key: &str) -> String`、`t_with(&self, key: &str, vars: &[(&str, &str)]) -> String`；五张翻译表在第一次用到时解析一次（`LazyLock`），之后只按语言挑选
  - `app/src/i18n/language-cases.json`：`[{ "setting": string, "system": string | null, "expected": Lang }]`
  - 翻译键的命名空间：`app`、`nav`、`state`、`common`、`tray`、`notify`、`quit`、`startup`；后续任务在同一批文件里加 `shell`、`format`、`executor`、`permission`、`actions`、`errors`、`overview`、`issues`、`runs`、`timeline`、`bindings`、`doctor`、`settings`、`help`、`onboarding` 等命名空间。占位符一律写成 `{{name}}`。
- Consumes: 第一部分的 `config::Settings`（`language`）。

- [ ] **Step 1: 语言解析的失败测试（两边读同一张用例表）**

`app/src/i18n/language-cases.json`（Rust 与 TypeScript 的测试都读这个文件，不另抄一份；`system` 为 `null` 表示读不到系统语言）：

```json
[
  { "setting": "system", "system": "zh-CN", "expected": "zh-CN" },
  { "setting": "system", "system": "zh-Hans-CN", "expected": "zh-CN" },
  { "setting": "system", "system": "zh-Hans-HK", "expected": "zh-CN" },
  { "setting": "system", "system": "zh-Hans-MO", "expected": "zh-CN" },
  { "setting": "system", "system": "zh-TW", "expected": "zh-TW" },
  { "setting": "system", "system": "zh_TW", "expected": "zh-TW" },
  { "setting": "system", "system": "zh-Hant", "expected": "zh-TW" },
  { "setting": "system", "system": "zh-Hant-HK", "expected": "zh-TW" },
  { "setting": "system", "system": "zh-HK", "expected": "zh-TW" },
  { "setting": "system", "system": "zh-MO", "expected": "zh-TW" },
  { "setting": "system", "system": "zh", "expected": "zh-CN" },
  { "setting": "system", "system": "ja-JP", "expected": "ja" },
  { "setting": "system", "system": "ko-KR", "expected": "ko" },
  { "setting": "system", "system": "en-GB", "expected": "en" },
  { "setting": "system", "system": "de-DE", "expected": "en" },
  { "setting": "system", "system": null, "expected": "en" },
  { "setting": "ja", "system": "zh-CN", "expected": "ja" },
  { "setting": "zh-TW", "system": "en-US", "expected": "zh-TW" },
  { "setting": "fr", "system": "zh-CN", "expected": "en" }
]
```

`app/src/i18n/i18n.test.ts`：

```ts
import { describe, expect, test } from "vitest";
import cases from "./language-cases.json";
import { LANGUAGES, resolveLanguage } from "./language";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";

describe("language resolution", () => {
  test.each(cases)("setting $setting with system $system gives $expected", ({ setting, system, expected }) => {
    expect(resolveLanguage(setting, system ?? undefined)).toBe(expected);
  });
});

/** Every key of a nested translation object, dotted. */
function keys(table: object, prefix = ""): string[] {
  return Object.entries(table).flatMap(([key, value]) =>
    typeof value === "object" && value !== null ? keys(value, `${prefix}${key}.`) : [`${prefix}${key}`],
  );
}

function lookup(table: object, key: string): unknown {
  return key.split(".").reduce<unknown>((node, part) => (node as Record<string, unknown>)?.[part], table);
}

const placeholders = (text: string) => [...text.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();

describe("translation files", () => {
  const files = { "zh-CN": zhCN, "zh-TW": zhTW, en, ja, ko } as const;
  // English is the reference; comparing it with itself proves nothing.
  const others = Object.entries(files).filter(([lang]) => lang !== "en");

  test("there is one file per language", () => {
    expect(Object.keys(files).sort()).toEqual([...LANGUAGES].sort());
  });

  test("English has no empty text", () => {
    for (const key of keys(en)) {
      expect(String(lookup(en, key)).trim(), key).not.toBe("");
    }
  });

  test.each(others)("%s has exactly the English keys", (_lang, table) => {
    expect(keys(table).sort()).toEqual(keys(en).sort());
  });

  test.each(others)("%s has no empty text and the same placeholders", (_lang, table) => {
    for (const key of keys(en)) {
      const text = lookup(table, key);
      expect(typeof text, key).toBe("string");
      expect((text as string).trim(), key).not.toBe("");
      expect(placeholders(text as string), key).toEqual(placeholders(lookup(en, key) as string));
    }
  });
});
```

`app/src-tauri/src/i18n.rs` 末尾的测试模块：

```rust
#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    /// One row of app/src/i18n/language-cases.json, which the panel's tests read too.
    #[derive(Deserialize)]
    struct Case {
        setting: String,
        system: Option<String>,
        expected: String,
    }

    #[test]
    fn languages_resolve_like_the_panel_does() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("../../src/i18n/language-cases.json"))
            .expect("language-cases.json is valid");
        assert!(cases.len() >= 19, "the shared cases were read");
        for case in cases {
            assert_eq!(
                resolve_language(&case.setting, case.system.as_deref()),
                case.expected,
                "{} / {:?}",
                case.setting,
                case.system
            );
        }
    }

    #[test]
    fn every_key_rust_uses_exists_in_every_language() {
        for lang in LANGUAGES {
            for key in USED_KEYS {
                assert!(lookup(table(lang), key).is_some(), "{lang} lacks {key}");
            }
        }
    }

    #[test]
    fn missing_keys_fall_back_to_english_then_to_the_key() {
        // Small tables of its own, so the English fallback is really taken: the real files have
        // the same keys in every language (the panel's tests check that).
        let en = json!({ "tray": { "quit": "Quit IssueRelay", "pause": "Pause monitoring" } });
        let ja = json!({ "tray": { "quit": "IssueRelay を終了" } });
        let t = Translator { table: &ja, fallback: &en };
        assert_eq!(t.t("tray.quit"), "IssueRelay を終了");
        assert_eq!(t.t("tray.pause"), "Pause monitoring", "missing in Japanese: English");
        assert_eq!(t.t("no.such.key"), "no.such.key", "missing everywhere: the key itself");
    }

    #[test]
    fn placeholders_are_filled_in() {
        assert_eq!(
            Translator::new("en").t_with("tray.status_counts", &[("running", "2"), ("queued", "1")]),
            "Running 2 · Queued 1"
        );
        assert_eq!(Translator::new("xx").t("tray.quit"), "Quit IssueRelay", "an unknown language reads English");
    }
}
```

Run: `pnpm --dir app test`
Expected: 失败（`language.ts` 与五个翻译文件还不存在）。Rust 侧要到 Step 2 在 `lib.rs` 里声明 `pub mod i18n;` 之后才会编译这些测试，此时 `cargo test -p issuerelay-app` 不会多出测试，本步只看 TS 的失败。

- [ ] **Step 2: 实现**

`app/src/i18n/language.ts`：

```ts
export type Lang = "zh-CN" | "zh-TW" | "en" | "ja" | "ko";
export const LANGUAGES: Lang[] = ["zh-CN", "zh-TW", "en", "ja", "ko"];

/**
 * The interface language for a `language` setting ("system" or one of LANGUAGES) and the
 * system's locale (the one Rust reports, `AppInfo.system_locale`). For Chinese the script
 * subtag decides first (Hans → zh-CN, Hant → zh-TW), then the region (TW, HK, MO → zh-TW);
 * a language IssueRelay does not offer falls back to English. Keep in step with
 * `resolve_language` in app/src-tauri/src/i18n.rs: both are tested with language-cases.json.
 */
export function resolveLanguage(setting: string, systemLocale: string | undefined): Lang {
  if ((LANGUAGES as string[]).includes(setting)) return setting as Lang;
  if (setting !== "system") return "en";
  const [primary, ...rest] = (systemLocale ?? "").toLowerCase().split(/[-_]/);
  if (primary === "zh") {
    if (rest.includes("hans")) return "zh-CN";
    if (rest.includes("hant")) return "zh-TW";
    return rest.some((tag) => tag === "tw" || tag === "hk" || tag === "mo") ? "zh-TW" : "zh-CN";
  }
  if (primary === "ja") return "ja";
  if (primary === "ko") return "ko";
  return "en";
}
```

`app/src/i18n/index.ts`：

```ts
import i18n, { type InitOptions } from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import type { Lang } from "./language";

export * from "./language";

const resources = {
  "zh-CN": { translation: zhCN },
  "zh-TW": { translation: zhTW },
  en: { translation: en },
  ja: { translation: ja },
  ko: { translation: ko },
};

/** Starts i18next in `lang`. `options` is for the tests' missing-key guard only. */
export async function initI18n(lang: Lang, options: InitOptions = {}): Promise<void> {
  await i18n.use(initReactI18next).init({
    resources,
    lng: lang,
    fallbackLng: "en",
    interpolation: { escapeValue: false }, // React escapes text nodes itself.
    returnNull: false,
    ...options,
  });
  document.documentElement.lang = lang;
}

/** `initI18n` the first time, `setLanguage` afterwards (tests render many times per file). */
export async function initI18nOnce(lang: Lang): Promise<void> {
  if (i18n.isInitialized) {
    await setLanguage(lang);
  } else {
    await initI18n(lang);
  }
}

export async function setLanguage(lang: Lang): Promise<void> {
  await i18n.changeLanguage(lang);
  document.documentElement.lang = lang;
}

export default i18n;
```

（`returnNull: false`：以解析到的 i18next 版本的类型为准，该选项已不存在就删掉这一行，并在报告里说明。）

`app/src/test-setup.ts` 加缺翻译键守卫（Task 1 的桩与时钟复位之后）：

```ts
import { initI18n } from "@/i18n";

// Every test runs with i18next started once: a `t()` key that English lacks fails the test
// instead of showing the key. (Missing in another language only is caught by i18n.test.ts.)
await initI18n("en", {
  saveMissing: true,
  missingKeyHandler: (_languages, _namespace, key) => {
    throw new Error(`missing translation key: ${key}`);
  },
});
```

（顶层 `await` 在 Vitest 的公共设置文件里可用；`missingKeyHandler` 的参数以解析到的 i18next 类型为准。之后各测试的 `initI18nOnce` 只切换语言，守卫一直生效。）

`app/src/main.tsx` 在渲染前 `await initI18n(resolveLanguage("system", navigator.language))`：这只是本任务的临时写法（还没有后端），Task 5 改为用 Rust 报告的系统语言（`AppInfo.system_locale`）与设置里的语言，托盘、通知与面板从此读同一个系统语言。

`app/src/i18n/locales/en.json`（其余四种语言同结构、全部翻译；`zh-CN` 为主写语言，`zh-TW` 用繁体与台湾用语，`ja`、`ko` 用自然的产品文案，不要逐字直译）：

```json
{
  "app": { "name": "IssueRelay", "tagline": "Hands your Linear issues to Claude Code and Codex" },
  "nav": {
    "overview": "Overview",
    "issues": "Issues",
    "runs": "Runs",
    "bindings": "Bindings",
    "doctor": "Environment",
    "settings": "Settings",
    "help": "Help"
  },
  "state": {
    "waiting_binding": "Waiting for setup",
    "queued": "Queued",
    "running": "Running",
    "awaiting_acceptance": "Awaiting acceptance",
    "needs_info": "Needs information",
    "done": "Done",
    "failed": "Failed",
    "canceled": "Canceled"
  },
  "common": {
    "loading": "Loading…",
    "refresh": "Refresh",
    "retry": "Try again",
    "save": "Save",
    "saved": "Saved",
    "cancel": "Cancel",
    "close": "Close",
    "copy": "Copy",
    "copied": "Copied",
    "error_title": "Something went wrong",
    "unknown": "Unknown",
    "never": "Never"
  },
  "tray": {
    "starting": "Starting…",
    "status_counts": "Running {{running}} · Queued {{queued}}",
    "status_paused": "Monitoring paused",
    "status_problem": "Needs attention",
    "pause": "Pause monitoring",
    "resume": "Resume monitoring",
    "poll_now": "Check now",
    "open_panel": "Open panel",
    "quit": "Quit IssueRelay"
  },
  "notify": {
    "run_started": "The run has started.",
    "awaiting_acceptance": "Ready for you to accept: it is in testing now.",
    "needs_info": "The AI asked a question on the issue; add the details and label it ai-ready again.",
    "failed": "The run failed. The panel shows why."
  },
  "quit": {
    "title": "Runs are still in progress",
    "body": "{{count}} run(s) are in progress. Wait for them to finish, or stop them now?",
    "wait": "Wait for completion",
    "stop": "Stop now",
    "cancel": "Keep running",
    "waiting": "Waiting for the runs to finish, then IssueRelay quits."
  },
  "startup": {
    "failed_title": "IssueRelay could not start",
    "already_running": "Another IssueRelay is already running for this user (the tray app or a development host). Quit it first.",
    "store_unavailable": "IssueRelay could not open its run records",
    "other": "Something stopped IssueRelay from starting",
    "quit": "Quit"
  }
}
```

（通知里不放引擎的原文：失败原因在面板里作为细节行显示，通知只说「失败了，原因见面板」。`quit.*`、`startup.*` 是面板文案。`zh-CN.json` 示例：`"tray": { "status_counts": "运行中 {{running}} 条 · 排队 {{queued}} 条", "status_paused": "已暂停监控", "status_problem": "需要处理", "pause": "暂停监控", "resume": "恢复监控", "poll_now": "立即检查一次", "open_panel": "打开面板", "quit": "退出 IssueRelay", "starting": "正在启动…" }`；状态名照设计 5.2、5.4 节：等待绑定、排队中、运行中、待验收、待补充信息、已完成、失败、已取消（`canceled` 在五种语言里分别是 已取消 / 已取消 / Canceled / キャンセル済み / 취소됨；「停止」只作操作名）。）

`app/src-tauri/src/i18n.rs`：

```rust
//! The interface languages, read from the panel's translation files so that the tray menu and
//! the notifications say the same as the panel. (The quit prompt and the startup error page are
//! the panel's own text.)
use std::sync::LazyLock;

use issuerelay_core::config::Settings;
use serde_json::Value;

/// The interface languages; the panel offers the same.
pub const LANGUAGES: [&str; 5] = ["zh-CN", "zh-TW", "en", "ja", "ko"];

/// The panel's translation files, embedded at build time.
const SOURCES: [(&str, &str); 5] = [
    ("zh-CN", include_str!("../../src/i18n/locales/zh-CN.json")),
    ("zh-TW", include_str!("../../src/i18n/locales/zh-TW.json")),
    ("en", include_str!("../../src/i18n/locales/en.json")),
    ("ja", include_str!("../../src/i18n/locales/ja.json")),
    ("ko", include_str!("../../src/i18n/locales/ko.json")),
];

/// Every key the Rust side looks up; the tests check that each language has them.
pub const USED_KEYS: &[&str] = &[
    "tray.starting",
    "tray.status_counts",
    "tray.status_paused",
    "tray.status_problem",
    "tray.pause",
    "tray.resume",
    "tray.poll_now",
    "tray.open_panel",
    "tray.quit",
    "notify.run_started",
    "notify.awaiting_acceptance",
    "notify.needs_info",
    "notify.failed",
];

/// The interface language for a `language` setting ("system" or one of `LANGUAGES`) and the
/// system locale. For Chinese the script subtag decides first (Hans, Hant), then the region.
/// Keep in step with `resolveLanguage` in app/src/i18n/language.ts: both are tested with
/// app/src/i18n/language-cases.json.
pub fn resolve_language(setting: &str, system: Option<&str>) -> &'static str {
    if let Some(lang) = LANGUAGES.iter().find(|lang| **lang == setting) {
        return lang;
    }
    if setting != "system" {
        return "en";
    }
    let locale = system.unwrap_or_default().to_ascii_lowercase();
    let mut subtags = locale.split(['-', '_']);
    match subtags.next() {
        Some("zh") => {
            let rest: Vec<&str> = subtags.collect();
            // The script wins over the region: zh-Hans-HK is Simplified Chinese.
            let traditional = !rest.contains(&"hans")
                && (rest.contains(&"hant") || rest.iter().any(|tag| matches!(*tag, "tw" | "hk" | "mo")));
            if traditional { "zh-TW" } else { "zh-CN" }
        }
        Some("ja") => "ja",
        Some("ko") => "ko",
        _ => "en",
    }
}

/// Every language's table, parsed once, the first time any text is looked up.
static TABLES: LazyLock<Vec<(&'static str, Value)>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|(lang, text)| {
            let table = serde_json::from_str(text)
                .expect("translation files are valid JSON (checked by the panel's tests)");
            (*lang, table)
        })
        .collect()
});

/// The table of `lang`; English for a language IssueRelay does not offer.
fn table(lang: &str) -> &'static Value {
    let find = |wanted: &str| TABLES.iter().find(|(name, _)| *name == wanted).map(|(_, table)| table);
    find(lang).or_else(|| find("en")).expect("the English translations are embedded")
}

fn lookup<'a>(table: &'a Value, key: &str) -> Option<&'a str> {
    key.split('.').try_fold(table, |node, part| node.get(part))?.as_str()
}

/// Looks up interface texts in one language, falling back to English, then to the key itself.
/// Cheap to make: it only points at tables parsed once (`TABLES`).
pub struct Translator<'a> {
    table: &'a Value,
    fallback: &'a Value,
}

impl Translator<'static> {
    pub fn new(lang: &str) -> Self {
        Self { table: table(lang), fallback: table("en") }
    }

    /// The language the settings ask for (`"system"` follows the system locale).
    pub fn for_settings(settings: &Settings) -> Self {
        Self::new(resolve_language(&settings.language, sys_locale::get_locale().as_deref()))
    }
}

impl Translator<'_> {
    pub fn t(&self, key: &str) -> String {
        lookup(self.table, key)
            .or_else(|| lookup(self.fallback, key))
            .unwrap_or(key)
            .to_string()
    }

    /// `t` with each `{{name}}` replaced by its value.
    pub fn t_with(&self, key: &str, vars: &[(&str, &str)]) -> String {
        vars.iter().fold(self.t(key), |text, (name, value)| {
            text.replace(&format!("{{{{{name}}}}}"), value)
        })
    }
}
```

（`cargo add -p issuerelay-app sys-locale`。）

`app/src-tauri/src/lib.rs` 在 Task 1 的 `run()` 上方加 `pub mod i18n;`：模块与其中的 `Translator`、`resolve_language`、`LANGUAGES`、`USED_KEYS` 都是 `pub`，Task 3 接上托盘之前也不会被 rustc 当成死代码，`clippy -D warnings` 能过。

Run: `pnpm --dir app test` 与 `cargo test -p issuerelay-app`
Expected: PASS。

- [ ] **Step 3: 文档与提交**

- 设计第 9.4 节补：「翻译文件在 `app/src/i18n/locales/`，Rust 侧编译时嵌入同一批文件（托盘菜单与通知）；系统语言一律取 Rust 侧读到的值，面板经 `AppInfo.system_locale` 拿到。新增语言要同时加文件、把语言加进两边的 `LANGUAGES`，并在 `app/src/i18n/language-cases.json` 补语言解析用例（两边的测试都读它）。」

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs Cargo.lock app/pnpm-lock.yaml
git commit -m "feat(app): add the five interface languages shared by the panel and the tray"
```

---

### Task 3: 宿主接入、启动状态、窗口与退出、日志、事件转发、通知、托盘

设计第 4.1 节（单实例；关窗口不退出；从托盘退出即停止监控，有任务在跑时先询问）、6.2 节（「等待完成 / 立即停止」）、9.1 节（托盘菜单）、9.5 节（四种通知可分别关闭）。宿主在后台异步启动，面板的命令等它就绪；启动状态（启动中 / 就绪 / 失败及原因代码）留在 `AppState` 里随时可查，失败时再发一次事件给已经打开的面板。「就绪」指 `Host::start` 已返回：按第一部分 Task 9 的 AR51，它不再等 Linear，引擎先用一个「连接中」的占位连接，首次连接在后台进行（`LinearSetup::Connecting`），连接中不算出问题。退出是一个纯函数的状态机，可以不开窗口测试。日志先经 core 的逐行清洗再写文件和 stderr（清洗代码从 devhost 移进 core，两个宿主共用；字符规则在 core 里只有一份实现 `text::strip_control`，`clean_text` 也建在它上面）。

**Files:**
- Create: `crates/core/src/logscrub.rs`；Modify: `crates/core/src/text.rs`（`strip_control` 移到这里，`clean_text` 改为建在它上面）、`crates/core/src/lib.rs`（`pub mod logscrub;`）、`crates/core/Cargo.toml`（`tracing-subscriber`）、`crates/devhost/src/main.rs`（改用 core 的清洗，测试随代码移走）
- Create: `app/src-tauri/src/{state,startup,window,quit,logging,events,notify,tray,test_support}.rs`
- Create: `app/src-tauri/src/commands.rs`（空壳：只有一个空的 `handler()`，Task 4 写入全部命令并删掉它）
- Create: `app/src/lib/problem-cases.json`（托盘与面板共用的「需要处理」判定用例表，Task 5 的侧栏测试也读它）
- Create: `app/src-tauri/icons/tray/{idle,running,paused,problem}.png`（占位：从 `icons/32x32.png` 复制，Task 11 换成正式图标）
- Modify: `app/src-tauri/src/lib.rs`、`app/src-tauri/Cargo.toml`、`Cargo.lock`
- Modify: `.github/CODEOWNERS`、`docs/ai-coding/AI_CODING_GUIDE.md`（第 5 节）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 4.1、9.1、11 节）
- Test: 各模块末尾的 `mod tests`；`crates/core/src/logscrub.rs` 的测试（从 devhost 移来）

**Interfaces:**
- Produces:
  - core：`pub fn issuerelay_core::text::strip_control(text: &str) -> String`（字符规则的唯一实现：控制字符变空格、`is_invisible` 的字符去掉，不去首尾、不截断；`clean_text` 是它再去首尾、截到 500 字）、`pub struct logscrub::CleanLines<W>(pub W)`（`io::Write`）、`pub struct logscrub::CleanWriter<M>(pub M)`（`MakeWriter`：`CleanWriter(io::stderr)`、`CleanWriter(non_blocking)`）
  - `pub const AUTOSTART_ARG: &str = "--autostart"`
  - `state`：`pub enum StartupCode { AlreadyRunning, StoreUnavailable, Other }`、`pub struct StartupFailure { pub code: StartupCode, pub detail: String }`、`pub enum StartupState { Starting, Ready, Failed(StartupFailure) }`（serde：`{"kind":"starting"}`、`{"kind":"ready"}`、`{"kind":"failed","code":"already_running","detail":"…"}`）；`pub struct AppState`：`new(autostarted: bool)`、`set_host(&self, result: Result<Host, StartupFailure>)`、`async host(&self) -> Result<Host, String>`、`try_host(&self) -> Option<Host>`、`startup_state(&self) -> StartupState`、`pub quit: Mutex<QuitState>`、`pub autostarted: bool`
  - `startup`：`pub async fn start(app: AppHandle)`、`pub fn failure(error: &anyhow::Error, state_db: &Path) -> StartupFailure`
  - `pub fn window::show(app: &AppHandle)`、`pub fn window::hide(app: &AppHandle)`（macOS 上同时切换程序坞图标）、`pub fn window::on_event(window: &tauri::Window, event: &WindowEvent)`
  - `quit`：`pub enum QuitState { Idle, Asking, Waiting, Stopping, Done }`、`pub enum QuitEvent { Requested { host_ready: bool, running: usize }, Wait, StopNow, Stopped, WindowClosed }`、`pub enum QuitStep { Nothing, Ask(usize), ShowPanel, Stop(StopMode), HideWindow, Exit }`、`pub fn next(state: QuitState, event: QuitEvent) -> (QuitState, QuitStep)`（纯函数）、`pub fn handle(app: &AppHandle, event: QuitEvent) -> QuitStep`、`pub fn request(app: &AppHandle)`、`pub fn on_run_event(app: &AppHandle, event: RunEvent)`
  - `events`：`pub const ENGINE_EVENT: &str = "engine-event"`、`RESYNC_EVENT = "engine-resync"`、`QUIT_REQUESTED = "quit-requested"`（载荷：运行中的条数）、`STARTUP_FAILED = "startup-failed"`（载荷：`StartupState::Failed`）；`pub enum Forward { Event(Box<EngineEvent>), Resync, Stop }`（事件装箱：`EngineEvent::Job` 带着约 280 字节的 `Job`，不装箱会触发 `clippy::large_enum_variant`）、`pub fn forward(received: Result<EngineEvent, RecvError>) -> Forward`、`pub fn spawn_forwarder(app: AppHandle, host: Host)`
  - `pub enum notify::NotificationKind { RunStarted, AwaitingAcceptance, NeedsInfo, Failed }`、`pub fn notify::notification_for(previous: Option<JobState>, job: &Job) -> Option<NotificationKind>`、`pub fn notify::enabled(kind, settings: &NotificationSettings) -> bool`、`pub fn notify::content(kind, job: &Job, t: &Translator) -> (String, String)`（标题＝编号＋标题，正文＝已翻译的一句话，不带引擎原文）
  - `pub enum tray::TrayState { Idle, Running, Paused, Problem }`、`pub struct tray::TrayView { pub state: TrayState, pub status_line: String, pub toggle_label: String }`、`pub fn tray::tray_view(status: &StatusSnapshot, setup: &LinearSetup, t: &Translator) -> TrayView`、`pub fn tray::create(app: &AppHandle) -> tauri::Result<()>`、`pub fn tray::refresh(app: &AppHandle, host: &Host)`
  - `pub fn logging::init(logs_dir: Option<&Path>) -> Option<WorkerGuard>`、`pub fn logging::allowed(target: &str, level: &Level) -> bool`、`pub fn logging::warn_failed<T, E: Display>(result: Result<T, E>, what: &str)`、`pub fn logging::debug_failed<T, E: Display>(result: Result<T, E>, what: &str)`
  - 测试辅助（`#[cfg(test)] mod test_support`）：`job(state: JobState) -> Job`（18 个字段全部写出）、`checked() -> StatusSnapshot`（9 个字段全部写出；Task 4 给 `StatusSnapshot` 加 `held` 时同步补上）
  - `app/src/lib/problem-cases.json`：`[{ "name", "linear": "connected" | "not_configured" | "interface_changed" | "connect_failed", "checked", "linear_ok", "last_error", "paused", "running", "expected": "idle" | "running" | "paused" | "problem" }]`
- Consumes: 第一部分的 `host::{Host, HostParts, SystemConnector, LinearSetup, AuthMethod, StopMode}`（Task 9：`stop` 可重复调用，`stop(StopRuns)` 会结束一个仍在等待的 `stop(WaitForRuns)`，两者都返回；宿主最后停 IPC）、`engine::{EngineEvent, StatusSnapshot}`、`config::{NotificationSettings, ExecutorKind}`、`model::{Job, JobState}`、`linear::Viewer`、`ipc::server::AlreadyRunning`、`paths::AppPaths`、`text::clean_text`；`LinearSetup` 含 AR51 新增的 `Connecting`（`{"kind":"connecting"}`）；Task 2 的 `i18n::Translator`

- [ ] **Step 1: 把日志清洗移进 core（先写失败测试）**

devhost 逐行清洗日志（`CleanLines`、`strip_control`、`CleanStderr`，`crates/devhost/src/main.rs`），因为事件字段（`error = %…`）可能带 Linear 的文本；托盘程序跑同一个引擎，日志文件同样要清洗。代码移进 core，两个宿主共用，不复制。`strip_control` 与 `text::clean_text` 原是同一个循环的两份副本：`strip_control` 放进 `text.rs` 作为字符规则的唯一实现，`clean_text` 改为建在它上面，日志清洗与面板的用例表（Task 4）都经它。

`crates/core/src/text.rs` 测试模块加（第一条从 devhost 移来）：

```rust
    #[test]
    fn the_error_line_keeps_the_root_cause_however_long_the_chain() {
        let error = anyhow::anyhow!("root cause\u{1b}[2J")
            .context("x".repeat(600))
            .context("outer\n");
        let line = strip_control(&format!("{error:#}"));
        assert!(line.starts_with("outer : xxx"), "{line}");
        assert!(line.ends_with(": root cause [2J"), "{line}");
        assert_eq!(
            line.chars().count(),
            "outer : ".len() + 600 + ": root cause [2J".len()
        );
    }

    #[test]
    fn clean_text_is_the_character_rule_trimmed_and_capped() {
        // One implementation: `strip_control` keeps the ends and the length, `clean_text` trims
        // and caps what it returns.
        assert_eq!(strip_control(" a\tb\u{200b} "), " a b ");
        assert_eq!(clean_text(" a\tb\u{200b} "), strip_control(" a\tb\u{200b} ").trim());
    }
```

新建 `crates/core/src/logscrub.rs`，先只放从 devhost 移来的另外三条测试（外加一条 `CleanWriter` 的测试）：

```rust
#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::Mutex;

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    /// What `CleanLines` passes on when `text` is written to it in one go.
    fn written(text: &str) -> String {
        let mut out = CleanLines(Vec::new());
        out.write_all(text.as_bytes()).unwrap();
        String::from_utf8(out.0).unwrap()
    }

    #[test]
    fn a_log_line_carries_no_terminal_escapes() {
        // Clear screen, OSC 52 (write the clipboard, ended by BEL), a lone BEL, a carriage return
        // that would overwrite the line, and a right-to-left override.
        let hostile =
            "WARN poll: error=boom\u{1b}[2J\u{1b}]52;c;ZXZpbA==\u{7} more\rrewritten \u{202e}end\n";
        assert_eq!(
            written(hostile),
            "WARN poll: error=boom [2J ]52;c;ZXZpbA==  more rewritten end\n"
        );
    }

    #[test]
    fn only_the_closing_newline_survives() {
        // A newline inside the text would let a field forge a second log line.
        assert_eq!(
            written("INFO a: x=one\nINFO forged: y=two\n"),
            "INFO a: x=one INFO forged: y=two\n"
        );
        assert_eq!(written("\n"), "\n");
        assert_eq!(written("no ending"), "no ending");
        // Text that already ends in a newline before the closing one keeps just one.
        assert_eq!(written("tail\n\n"), "tail \n");
        // Other scripts and punctuation are untouched; so is the C1 range's neighbour U+00A0.
        assert_eq!(written("日本語 é \u{a0}…\n"), "日本語 é \u{a0}…\n");
        // The C1 controls (here NEL and CSI) are controls too.
        assert_eq!(written("a\u{85}b\u{9b}c\n"), "a b c\n");
    }

    /// A writer whose every write fails with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn a_write_reports_the_bytes_it_was_given_and_passes_errors_on() {
        let mut out = CleanLines(Vec::new());
        // `write_all` relies on this: the cleaned text is shorter than what came in.
        assert_eq!(out.write(b"a\x1b[2Jb\n").unwrap(), 7);
        assert_eq!(out.0, b"a [2Jb\n");

        let error = CleanLines(Failing(io::ErrorKind::BrokenPipe))
            .write_all(b"gone\n")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn every_writer_the_subscriber_gets_cleans_its_line() {
        let make = CleanWriter(Mutex::new(Vec::new()));
        make.make_writer()
            .write_all(b"INFO a: x=\x1b]52;c;ZXZpbA==\x07\n")
            .unwrap();
        assert_eq!(*make.0.lock().unwrap(), b"INFO a: x= ]52;c;ZXZpbA== \n");
    }
}
```

`crates/core/src/lib.rs` 加 `pub mod logscrub;`，然后：

```bash
cargo add -p issuerelay-core tracing-subscriber --no-default-features --features fmt
```

（`MakeWriter` 在 `tracing-subscriber` 的 `fmt` feature 下；工作区里 devhost 已在用同一个 crate，锁文件不引入新的包。feature 名以解析到的版本为准。）

Run: `cargo test -p issuerelay-core --lib logscrub` 与 `cargo test -p issuerelay-core --lib text`
Expected: 编译失败（`strip_control`、`CleanLines`、`CleanWriter` 还不存在）。

`crates/core/src/text.rs`：`clean_text` 改为建在 `strip_control` 上（行为不变，`text.rs` 原有测试照旧通过），`strip_control` 从 devhost 移来：

```rust
pub fn clean_text(text: &str) -> String {
    let flat = strip_control(text);
    let flat = flat.trim();
    match flat.char_indices().nth(MAX_TEXT_CHARS) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat.to_string(),
    }
}

/// `text` with every control character (newline and tab included) turned into a space and every
/// invisible character (`is_invisible`) removed; nothing is trimmed and nothing cut. The one
/// implementation of the character rule: `clean_text` trims and caps what it returns, the log
/// scrub (`logscrub::CleanLines`) and the root cause at the end of an error chain need the ends.
pub fn strip_control(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_control() {
                Some(' ')
            } else if is_invisible(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect()
}
```

（`clean_text` 的文档注释保留原文。）然后在 `logscrub.rs` 的测试模块上方写实现（`CleanLines` 照 devhost 原样移来，`CleanStderr` 推广成接受任意写入端的 `CleanWriter`，日志文件也能用）：

```rust
//! Log lines as they are written. A log line can carry Linear's or an executor's text in its
//! fields (`error = %…`: the subscriber escapes only the message and error values), so every
//! line loses its control and invisible characters before it reaches a terminal or a log file,
//! keeping only the newline that ends it. The devhost and the desktop app log through
//! `CleanWriter`.
use std::io::{self, Write};

use tracing_subscriber::fmt::MakeWriter;

use crate::text::strip_control;

/// Passes what is written to `W` with `strip_control` applied, keeping only the line ending that
/// closes the text (the last byte, when it is a newline). A log line can then neither move the
/// cursor, change the terminal's state nor forge a second line. The tracing subscriber hands
/// over each line in a single `write_all`; a character split across two writes would show as
/// U+FFFD.
pub struct CleanLines<W>(pub W);

impl<W: Write> Write for CleanLines<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(bytes);
        let (body, ending) = match text.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (&*text, ""),
        };
        let mut clean = strip_control(body);
        clean.push_str(ending);
        self.0.write_all(clean.as_bytes())?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// A `MakeWriter` whose writers pass every line through `CleanLines`: `CleanWriter(io::stderr)`
/// for stderr, `CleanWriter(non_blocking)` for a log file. Colours must be off
/// (`with_ansi(false)`): their escape sequences would be cleaned into stray text.
pub struct CleanWriter<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for CleanWriter<M> {
    type Writer = CleanLines<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        CleanLines(self.0.make_writer())
    }
}
```

`crates/devhost/src/main.rs`（照第一部分 Task 9 合入后的版本改）：删掉本地的 `strip_control`、`CleanLines`、`CleanStderr` 与 `use tracing_subscriber::fmt::MakeWriter`、`text::is_invisible` 两处导入；改为从 core 导入 `logscrub::CleanWriter` 与 `text::strip_control`（`clean_text` 照旧从 `text` 导入）；`init_logging` 里 `.with_writer(CleanStderr)` 改为 `.with_writer(CleanWriter(io::stderr))`；`main` 里错误行照旧用 `strip_control`。测试模块删掉已移走的四条（`a_log_line_carries_no_terminal_escapes`、`only_the_closing_newline_survives`、`a_write_reports_the_bytes_it_was_given_and_passes_errors_on`、`the_error_line_keeps_the_root_cause_however_long_the_chain`）与 `written`，以及只给移走的那条测试用的 `use anyhow::anyhow;`（Task 9 之后的 devhost 测试模块以它开头；留着就是未使用的导入，`-D warnings` 会拒绝）；`Failing`、`a_closed_pipe_is_not_a_failure_but_other_write_errors_are`、`only_run_logs_below_warnings_by_default` 留下（它们测的是 devhost 自己的 `write_line` 与 `default_filter`；core 的 `logscrub` 测试另有一份同样的 `Failing`，两处各自的测试在用）。`.github/CODEOWNERS` 加 `/crates/core/src/logscrub.rs`（清洗从已登记的 devhost 移来，覆盖不能断）。

Run: `cargo test -p issuerelay-core --lib logscrub`、`cargo test -p issuerelay-core --lib text`、`cargo test -p issuerelay-devhost`、`cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS（`logscrub` 4 条：移来的 3 条加 `CleanWriter` 的 1 条；`text` 原有测试加 2 条新的；devhost 剩下的测试照旧通过）。

```bash
git add crates Cargo.lock .github/CODEOWNERS
git commit -m "refactor(core): move the log line scrub from the devhost into core"
```

- [ ] **Step 2: 依赖**

```bash
cargo add -p issuerelay-app tauri-plugin-single-instance tauri-plugin-notification tauri-plugin-autostart tauri-plugin-opener tauri-plugin-dialog
cargo add -p issuerelay-app tokio --features full
cargo add -p issuerelay-app anyhow tracing tracing-appender
cargo add -p issuerelay-app tracing-subscriber --features env-filter
cargo add -p issuerelay-app --dev chrono
```

（`chrono` 只给测试：`Job` 有三个必填的 `DateTime<Utc>` 字段、`StatusSnapshot.last_poll_at` 也是，测试辅助要构造它们。）

- [ ] **Step 3: 纯逻辑的失败测试**

先在 `lib.rs`（此时还是 Task 1 / Task 2 的版本）`pub mod i18n;` 旁加上 Step 4 的模块声明：`pub mod commands; pub mod events; pub mod logging; pub mod notify; pub mod quit; pub mod startup; pub mod state; pub mod tray; pub mod window;` 与 `#[cfg(test)] mod test_support;`。下面各文件此时只有测试模块（`commands.rs` 先空着），所以测试会编译并失败，而不是被 cargo 跳过。

`app/src-tauri/src/test_support.rs`（`lib.rs` 里 `#[cfg(test)] mod test_support;`；几个模块的测试共用，字段逐个写出，core 加字段时编译器会指出这里）：

```rust
//! Values the unit tests of several modules share: a job with every field filled in, and the
//! status of an engine whose last check worked. All of it is fictional.
use chrono::{TimeZone, Utc};
use issuerelay_core::{
    config::ExecutorKind,
    engine::StatusSnapshot,
    linear::Viewer,
    model::{Job, JobState},
};

/// `DEMO-1` in `state`, with each of `Job`'s 18 fields set.
pub fn job(state: JobState) -> Job {
    let at = Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
    Job {
        id: 1,
        issue_uuid: "issue-1".into(),
        issue_identifier: "DEMO-1".into(),
        title: "Demo task".into(),
        binding_id: Some("proj-1".into()),
        directory: Some("demo-app".into()),
        executor: Some(ExecutorKind::Claude),
        state,
        reason: None,
        priority: 2,
        issue_created_at: at,
        discovered_updated_at: at,
        verified_updated_at: None,
        created_at: at,
        started_at: Some(at),
        finished_at: None,
        log_path: None,
        summary: None,
    }
}

/// An engine that checked Linear just now and has nothing to do, with each of
/// `StatusSnapshot`'s 9 fields set.
pub fn checked() -> StatusSnapshot {
    StatusSnapshot {
        paused: false,
        linear_ok: true,
        viewer: Some(Viewer { id: "user-1".into(), name: "Jane Doe".into() }),
        last_poll_at: Some(Utc::now()),
        last_error: None,
        running: 0,
        queued: 0,
        waiting: 0,
        draining: false,
    }
}
```

`notify.rs` 测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::job;

    #[test]
    fn notifications_follow_state_changes_only() {
        use JobState::*;
        assert_eq!(notification_for(Some(Queued), &job(Running)), Some(NotificationKind::RunStarted));
        assert_eq!(notification_for(None, &job(Running)), Some(NotificationKind::RunStarted), "a job first seen running");
        assert_eq!(notification_for(Some(Running), &job(AwaitingAcceptance)), Some(NotificationKind::AwaitingAcceptance));
        assert_eq!(notification_for(Some(Running), &job(NeedsInfo)), Some(NotificationKind::NeedsInfo));
        assert_eq!(notification_for(Some(Running), &job(Failed)), Some(NotificationKind::Failed));
        assert_eq!(notification_for(Some(Running), &job(Running)), None, "no change, no notification");
        for quiet in [Queued, WaitingBinding, Done, Canceled] {
            assert_eq!(notification_for(Some(Running), &job(quiet)), None, "{quiet:?}");
        }
    }

    #[test]
    fn each_kind_can_be_switched_off() {
        let mut settings = NotificationSettings::default();
        assert!(enabled(NotificationKind::Failed, &settings));
        settings.failed = false;
        assert!(!enabled(NotificationKind::Failed, &settings));
        assert!(enabled(NotificationKind::RunStarted, &settings));
    }

    #[test]
    fn a_notification_names_the_issue_and_never_carries_the_engines_text() {
        let mut failed = job(JobState::Failed);
        failed.reason = Some("run exceeded the time limit".into());
        let (title, body) = content(NotificationKind::Failed, &failed, &Translator::new("en"));
        assert_eq!(title, "DEMO-1 Demo task");
        assert_eq!(body, "The run failed. The panel shows why.");
        assert!(!body.contains("time limit"), "the reason stays in the panel");
    }
}
```

`app/src/lib/problem-cases.json`（「需要处理」的判定只有这一份用例：Rust 的托盘测试读它，Task 5 的面板侧栏测试也读它）：

```json
[
  { "name": "connected, nothing running", "linear": "connected", "checked": true, "linear_ok": true, "last_error": false, "paused": false, "running": 0, "expected": "idle" },
  { "name": "a run in progress", "linear": "connected", "checked": true, "linear_ok": true, "last_error": false, "paused": false, "running": 2, "expected": "running" },
  { "name": "paused wins over running", "linear": "connected", "checked": true, "linear_ok": true, "last_error": false, "paused": true, "running": 1, "expected": "paused" },
  { "name": "no check yet is not a failure", "linear": "connected", "checked": false, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "idle" },
  { "name": "the last check failed", "linear": "connected", "checked": true, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "problem" },
  { "name": "a job could not start", "linear": "connected", "checked": true, "linear_ok": true, "last_error": true, "paused": false, "running": 0, "expected": "problem" },
  { "name": "a problem wins over paused", "linear": "connected", "checked": true, "linear_ok": true, "last_error": true, "paused": true, "running": 0, "expected": "problem" },
  { "name": "not logged in", "linear": "not_configured", "checked": true, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "problem" },
  { "name": "not logged in, before any check", "linear": "not_configured", "checked": false, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "problem" },
  { "name": "Linear's tools changed", "linear": "interface_changed", "checked": false, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "problem" },
  { "name": "connecting failed", "linear": "connect_failed", "checked": false, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "problem" },
  { "name": "the first connection is still under way", "linear": "connecting", "checked": false, "linear_ok": false, "last_error": false, "paused": false, "running": 0, "expected": "idle" },
  { "name": "while connecting, a failed check is no problem yet", "linear": "connecting", "checked": true, "linear_ok": false, "last_error": true, "paused": false, "running": 0, "expected": "idle" },
  { "name": "connecting while paused", "linear": "connecting", "checked": false, "linear_ok": false, "last_error": false, "paused": true, "running": 0, "expected": "paused" }
]
```

`tray.rs` 测试：

```rust
#[cfg(test)]
mod tests {
    use issuerelay_core::host::AuthMethod;
    use serde::Deserialize;

    use super::*;
    use crate::test_support::checked;

    fn connected() -> LinearSetup {
        LinearSetup::Connected { method: AuthMethod::OAuth }
    }

    #[test]
    fn the_tray_shows_what_the_engine_is_doing() {
        let t = Translator::new("en");
        let idle = tray_view(&checked(), &connected(), &t);
        assert_eq!(idle.state, TrayState::Idle);
        assert_eq!(idle.status_line, "Running 0 · Queued 0");
        assert_eq!(idle.toggle_label, "Pause monitoring");

        let busy = StatusSnapshot { running: 2, queued: 1, ..checked() };
        assert_eq!(tray_view(&busy, &connected(), &t).status_line, "Running 2 · Queued 1");

        let paused = StatusSnapshot { paused: true, running: 1, ..checked() };
        let view = tray_view(&paused, &connected(), &t);
        assert_eq!(view.status_line, "Monitoring paused");
        assert_eq!(view.toggle_label, "Resume monitoring");

        let not_logged_in = tray_view(&checked(), &LinearSetup::NotConfigured, &t);
        assert_eq!(not_logged_in.status_line, "Running 0 · Queued 0 · Needs attention");
    }

    /// One row of app/src/lib/problem-cases.json, which the panel's sidebar test reads too.
    #[derive(Deserialize)]
    struct ProblemCase {
        name: String,
        linear: String,
        checked: bool,
        linear_ok: bool,
        last_error: bool,
        paused: bool,
        running: usize,
        expected: String,
    }

    #[test]
    fn the_tray_state_follows_the_cases_the_panel_shares() {
        let cases: Vec<ProblemCase> = serde_json::from_str(include_str!("../../src/lib/problem-cases.json"))
            .expect("problem-cases.json is valid");
        assert!(cases.len() >= 14, "the shared cases were read");
        let t = Translator::new("en");
        for case in cases {
            let setup = match case.linear.as_str() {
                "connected" => connected(),
                "not_configured" => LinearSetup::NotConfigured,
                "interface_changed" => LinearSetup::InterfaceChanged { message: "list_issues changed".into() },
                "connect_failed" => LinearSetup::ConnectFailed { message: "network is down".into() },
                "connecting" => LinearSetup::Connecting,
                other => panic!("unknown Linear state {other} in {:?}", case.name),
            };
            let status = StatusSnapshot {
                paused: case.paused,
                linear_ok: case.linear_ok,
                last_poll_at: case.checked.then(chrono::Utc::now),
                last_error: case.last_error.then(|| "could not start DEMO-1".to_string()),
                running: case.running,
                ..checked()
            };
            let state = match tray_view(&status, &setup, &t).state {
                TrayState::Idle => "idle",
                TrayState::Running => "running",
                TrayState::Paused => "paused",
                TrayState::Problem => "problem",
            };
            assert_eq!(state, case.expected, "{}", case.name);
        }
    }
}
```

`quit.rs` 测试（退出的每个决定都在纯函数 `next` 里，不开窗口就能测）：

```rust
#[cfg(test)]
mod tests {
    use super::QuitEvent::{Requested, StopNow, Stopped, Wait, WindowClosed};
    use super::*;

    #[test]
    fn quitting_asks_only_while_runs_are_in_progress() {
        assert_eq!(
            next(QuitState::Idle, Requested { host_ready: false, running: 0 }),
            (QuitState::Done, QuitStep::Exit),
            "no host: nothing to stop"
        );
        assert_eq!(
            next(QuitState::Idle, Requested { host_ready: true, running: 0 }),
            (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns))
        );
        assert_eq!(next(QuitState::Idle, Requested { host_ready: true, running: 2 }), (QuitState::Asking, QuitStep::Ask(2)));
    }

    #[test]
    fn a_request_with_runs_in_progress_asks_and_a_repeat_is_decided_afresh() {
        // The tray's Quit, and Cmd+Q or the system once `on_run_event` turned them into
        // `Requested`. The hook itself depends on Tauri's `ExitRequested.code` and is checked by
        // hand in Task 13 Step 4.
        let (state, step) = next(QuitState::Idle, Requested { host_ready: true, running: 1 });
        assert_eq!((state, step), (QuitState::Asking, QuitStep::Ask(1)));
        assert_eq!(
            next(state, Requested { host_ready: true, running: 1 }),
            (QuitState::Asking, QuitStep::Ask(1)),
            "asked again while the prompt is open: it shows again, nothing stops"
        );
        assert_eq!(
            next(state, Requested { host_ready: true, running: 0 }),
            (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns)),
            "the runs ended meanwhile: no question"
        );
    }

    #[test]
    fn waiting_can_still_become_stopping_now() {
        let (state, step) = next(QuitState::Asking, Wait);
        assert_eq!((state, step), (QuitState::Waiting, QuitStep::Stop(StopMode::WaitForRuns)));
        // Still waiting: Quit in the tray brings the panel back, which still shows "stop now".
        assert_eq!(next(state, Requested { host_ready: true, running: 1 }), (QuitState::Waiting, QuitStep::ShowPanel));
        // The user switches to "stop now" while `stop(WaitForRuns)` has not returned: a second
        // stop with `StopRuns` starts, which ends the first; both return (core Task 9).
        let (state, step) = next(state, StopNow);
        assert_eq!((state, step), (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns)));
        let (state, step) = next(state, Stopped);
        assert_eq!((state, step), (QuitState::Done, QuitStep::Exit));
        assert_eq!(next(state, Stopped), (QuitState::Done, QuitStep::Nothing), "the other stop returning exits nothing again");
    }

    #[test]
    fn closing_the_window_while_quitting_does_not_exit_early() {
        for state in [QuitState::Idle, QuitState::Asking, QuitState::Waiting, QuitState::Stopping] {
            assert_eq!(next(state, WindowClosed), (state, QuitStep::HideWindow), "{state:?}");
        }
        assert_eq!(next(QuitState::Done, WindowClosed), (QuitState::Done, QuitStep::Nothing), "only once quitting is done may it close");
        assert_eq!(
            next(QuitState::Stopping, Requested { host_ready: true, running: 1 }),
            (QuitState::Stopping, QuitStep::Nothing),
            "a second Cmd+Q while stopping changes nothing"
        );
    }
}
```

`events.rs` 测试：

```rust
#[cfg(test)]
mod tests {
    use tokio::sync::broadcast;

    use super::*;
    use crate::{notify::NotificationKind, test_support::job};

    #[tokio::test]
    async fn a_lagged_panel_gets_one_resync_and_then_the_events_after_it() {
        let (sender, mut receiver) = broadcast::channel(2);
        for _ in 0..4 {
            sender.send(EngineEvent::Status).unwrap();
        }
        sender.send(EngineEvent::Config).unwrap();
        drop(sender);
        let mut seen = Vec::new();
        loop {
            match forward(receiver.recv().await) {
                Forward::Stop => break,
                other => seen.push(other),
            }
        }
        // The channel kept the last two events; the three before them were lost.
        assert_eq!(
            seen,
            [Forward::Resync, Forward::Event(Box::new(EngineEvent::Status)), Forward::Event(Box::new(EngineEvent::Config))]
        );
    }

    #[test]
    fn without_readable_job_records_no_job_notifies() {
        let mut known = None;
        let failed = job(JobState::Failed);
        assert_eq!(notification(&mut known, &failed, || None), None, "still unreadable: nothing notifies");
        assert_eq!(
            notification(&mut known, &failed, || Some(HashMap::new())),
            None,
            "the first readable seed only records the job, it never makes it look new"
        );
        let running = job(JobState::Running);
        assert_eq!(
            notification(&mut known, &running, || unreachable!("seeded already")),
            Some(NotificationKind::RunStarted)
        );
    }

    #[test]
    fn a_seeded_job_notifies_only_when_its_state_changes() {
        let mut known = Some(HashMap::from([(1, JobState::Queued)]));
        assert_eq!(notification(&mut known, &job(JobState::Queued), || None), None);
        assert_eq!(notification(&mut known, &job(JobState::Running), || None), Some(NotificationKind::RunStarted));
    }
}
```

`logging.rs` 测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rmcp_never_logs_below_info() {
        // rmcp logs the raw OAuth authorization code at debug; a user's own RUST_LOG must not
        // bring that into the log file.
        assert!(!allowed("rmcp::transport::auth", &Level::DEBUG));
        assert!(!allowed("rmcp", &Level::TRACE));
        assert!(allowed("rmcp::service", &Level::INFO));
        assert!(allowed("issuerelay_core::engine", &Level::DEBUG));
        assert!(allowed("rmcpx", &Level::DEBUG), "only the rmcp crate itself");
    }
}
```

`state.rs` 测试：

```rust
#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn commands_wait_for_the_host_and_see_a_failed_start() {
        let state = Arc::new(AppState::new(false));
        assert_eq!(state.startup_state(), StartupState::Starting);
        let waiting = tokio::spawn({
            let state = state.clone();
            async move { state.host().await.err() }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        let failure = StartupFailure {
            code: StartupCode::AlreadyRunning,
            detail: "another IssueRelay instance is already running".into(),
        };
        state.set_host(Err(failure.clone()));
        assert_eq!(waiting.await.unwrap().as_deref(), Some("another IssueRelay instance is already running"));
        assert!(state.try_host().is_none());
        // Still known afterwards: a panel that opens later learns why, not only one that listened.
        assert_eq!(state.startup_state(), StartupState::Failed(failure));
    }

    #[test]
    fn the_startup_state_has_the_shape_the_panel_reads() {
        assert_eq!(serde_json::to_value(StartupState::Starting).unwrap(), json!({ "kind": "starting" }));
        assert_eq!(serde_json::to_value(StartupState::Ready).unwrap(), json!({ "kind": "ready" }));
        let failed = StartupState::Failed(StartupFailure {
            code: StartupCode::StoreUnavailable,
            detail: "cannot open state.db".into(),
        });
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            json!({ "kind": "failed", "code": "store_unavailable", "detail": "cannot open state.db" })
        );
    }
}
```

`startup.rs` 测试：

```rust
#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn start_failures_are_told_apart() {
        let db = Path::new("data").join("state.db");
        let second = failure(&anyhow::Error::new(AlreadyRunning), &db);
        assert_eq!(second.code, StartupCode::AlreadyRunning);
        assert_eq!(second.detail, "another IssueRelay instance is already running");
        let store = anyhow!("disk I/O error\n").context(format!("cannot open {}", db.display()));
        let store = failure(&store, &db);
        assert_eq!(store.code, StartupCode::StoreUnavailable);
        assert!(!store.detail.contains('\n'), "the detail is one cleaned line: {:?}", store.detail);
        let folder = anyhow!("permission denied").context(format!("cannot create {}", Path::new("data").display()));
        assert_eq!(failure(&folder, &db).code, StartupCode::StoreUnavailable, "the run records' folder");
        let lock = anyhow!("permission denied").context(format!("cannot lock {}", Path::new("data").join("issuerelay.lock").display()));
        assert_eq!(failure(&lock, &db).code, StartupCode::Other, "the IPC endpoint's lock is not the store");
        let other = failure(&anyhow!("this system has no local data directory"), &db);
        assert_eq!(other.code, StartupCode::Other);
    }
}
```

Run: `cargo test -p issuerelay-app`
Expected: 编译失败。

- [ ] **Step 4: 实现各模块**

`state.rs`：

```rust
//! What the app keeps for its whole life: the host once started (or why it could not start),
//! where quitting stands, and whether the login item started it.
use std::sync::Mutex;

use issuerelay_core::host::Host;
use serde::Serialize;
use tokio::sync::{Notify, OnceCell};

use crate::quit::QuitState;

/// Why the host could not start, as a code the panel translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupCode {
    /// Another host (the tray app or the devhost) serves this user.
    AlreadyRunning,
    /// The run records (`state.db`) cannot be opened.
    StoreUnavailable,
    Other,
}

/// A failed start: its code, and the cleaned error the panel shows as a detail line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupFailure {
    pub code: StartupCode,
    pub detail: String,
}

/// How the start of the host went: the `startup_state` command's answer, and the payload of
/// `startup-failed` for a panel that is already open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartupState {
    Starting,
    /// `Host::start` returned. Linear may still be connecting (`LinearSetup::Connecting`):
    /// the host no longer waits for it.
    Ready,
    Failed(StartupFailure),
}

pub struct AppState {
    /// Set once by the startup task: the running host, or why it could not start.
    host: OnceCell<Result<Host, StartupFailure>>,
    ready: Notify,
    /// Where quitting stands; only `quit::handle` changes it.
    pub quit: Mutex<QuitState>,
    /// Started by the login item (`AUTOSTART_ARG`): stay in the tray instead of showing the panel.
    pub autostarted: bool,
}

impl AppState {
    pub fn new(autostarted: bool) -> Self {
        Self { host: OnceCell::new(), ready: Notify::new(), quit: Mutex::new(QuitState::Idle), autostarted }
    }

    /// Records how the start went and wakes every command waiting for it.
    pub fn set_host(&self, result: Result<Host, StartupFailure>) {
        if self.host.set(result).is_err() {
            tracing::warn!("the start of the host was recorded twice; the first is kept");
        }
        self.ready.notify_waiters();
    }

    /// The host, waiting while it starts; the failure's detail when it could not start.
    pub async fn host(&self) -> Result<Host, String> {
        loop {
            // Registered before the check, so a `set_host` in between still wakes this wait.
            let notified = self.ready.notified();
            if let Some(result) = self.host.get() {
                return result.clone().map_err(|failure| failure.detail);
            }
            notified.await;
        }
    }

    /// The host if it has started.
    pub fn try_host(&self) -> Option<Host> {
        self.host.get().and_then(|result| result.as_ref().ok().cloned())
    }

    /// Starting, ready or failed (with why), at any time.
    pub fn startup_state(&self) -> StartupState {
        match self.host.get() {
            None => StartupState::Starting,
            Some(Ok(_)) => StartupState::Ready,
            Some(Err(failure)) => StartupState::Failed(failure.clone()),
        }
    }
}
```

`logging.rs`：

```rust
//! Logs go to `<data dir>/logs/app.<date>.log` (rotated daily, seven files kept) and, in debug
//! builds, to stderr, every line through core's scrub (`logscrub::CleanWriter`): event fields
//! can hold Linear's or an executor's text. `RUST_LOG` overrides the default filter, but rmcp is
//! never logged below `info`: at `debug` it writes the raw OAuth authorization code.
use std::{fmt::Display, path::Path};

use issuerelay_core::logscrub::{CleanWriter, strip_control};
use tracing::Level;
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{EnvFilter, filter::filter_fn, fmt, prelude::*};

const DEFAULT_FILTER: &str = "info,issuerelay_core=debug,issuerelay_app=debug";

/// Whether an event of `level` from `target` may be logged at all.
pub fn allowed(target: &str, level: &Level) -> bool {
    let rmcp = target == "rmcp" || target.starts_with("rmcp::");
    !(rmcp && *level > Level::INFO)
}

/// Logs a tray, window or store call that failed; the app goes on without it.
pub fn warn_failed<T, E: Display>(result: Result<T, E>, what: &str) {
    if let Err(e) = result {
        tracing::warn!(error = %e, "{what} failed");
    }
}

/// Logs an event the panel could not be sent; its webview may be gone, as while the app quits.
pub fn debug_failed<T, E: Display>(result: Result<T, E>, what: &str) {
    if let Err(e) = result {
        tracing::debug!(error = %e, "{what} failed");
    }
}

/// Starts logging; keep the returned guard alive for as long as the app runs, or the last lines
/// are lost. Without a log directory only stderr is used.
pub fn init(logs_dir: Option<&Path>) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let cap = filter_fn(|meta| allowed(meta.target(), meta.level()));
    let appender = logs_dir.and_then(|dir| {
        let built = tracing_appender::rolling::Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix("app")
            .filename_suffix("log")
            .max_log_files(7)
            .build(dir);
        match built {
            Ok(appender) => Some(appender),
            Err(e) => {
                // Logging is not up yet, so stderr is the only place to say it.
                eprintln!("IssueRelay cannot write its log file: {}", strip_control(&e.to_string()));
                None
            }
        }
    });
    let (file_layer, guard) = match appender {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (Some(fmt::layer().with_ansi(false).with_writer(CleanWriter(writer))), Some(guard))
        }
        None => (None, None),
    };
    // No colours: the scrub would turn their escape sequences into stray text.
    let stderr_layer =
        cfg!(debug_assertions).then(|| fmt::layer().with_ansi(false).with_writer(CleanWriter(std::io::stderr)));
    let started = tracing_subscriber::registry()
        .with(filter)
        .with(cap)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    if let Err(e) = started {
        eprintln!("IssueRelay could not start logging: {e}");
    }
    guard
}
```

（日志文件名是 tracing-appender 按前缀、日期、后缀拼出来的，例如 `app.2026-10-13.log`；以解析到的版本实际写出的名字为准，文档照实写。）

`window.rs`：

```rust
//! The panel window: shown on demand, hidden instead of closed. On macOS the Dock icon follows
//! the window: a tray app shows no Dock icon while its panel is hidden, also after a login start
//! (lib.rs hides through `hide` too).
use tauri::{AppHandle, Manager, WindowEvent};

use crate::{
    logging,
    quit::{self, QuitEvent, QuitStep},
};

// `set_activation_policy` is taken to return a `Result`; if it returns `()` in the resolved Tauri
// version, call it directly instead of through `warn_failed` (and say so in the report).
pub fn show(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    logging::warn_failed(app.set_activation_policy(tauri::ActivationPolicy::Regular), "showing the Dock icon");
    match app.get_webview_window("main") {
        Some(window) => {
            logging::warn_failed(window.show(), "showing the panel");
            logging::warn_failed(window.unminimize(), "restoring the panel");
            logging::warn_failed(window.set_focus(), "focusing the panel");
        }
        None => tracing::warn!("the panel window is missing"),
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        logging::warn_failed(window.hide(), "hiding the panel");
    }
    #[cfg(target_os = "macos")]
    logging::warn_failed(app.set_activation_policy(tauri::ActivationPolicy::Accessory), "hiding the Dock icon");
}

/// Closing the panel hides it; monitoring goes on in the tray. The quit state machine decides:
/// only once quitting is done may the window really close, so closing it while the app waits for
/// runs neither ends the app early nor loses the "stop now" prompt.
pub fn on_event(window: &tauri::Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event
        && window.label() == "main"
        && quit::handle(window.app_handle(), QuitEvent::WindowClosed) == QuitStep::HideWindow
    {
        api.prevent_close();
        hide(window.app_handle());
    }
}
```

`quit.rs`：

```rust
//! Quitting (design 4.1, 6.2). Every way to quit comes here: the tray's Quit, Cmd+Q, the system
//! asking the app to exit, the panel's answer and the window being closed. `next` is the whole
//! decision, a pure function of where quitting stands and what happened; `handle` applies it.
//! With runs in progress the panel asks whether to wait for them or stop them, and a user who
//! chose to wait can still stop them: `Host::stop(StopRuns)` ends a `stop(WaitForRuns)` that is
//! still waiting, and both return.
use std::sync::PoisonError;

use issuerelay_core::host::StopMode;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::{events, logging, state::AppState, window};

/// Where quitting stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitState {
    /// Not quitting.
    Idle,
    /// The panel asks "wait for completion" or "stop now".
    Asking,
    /// Waiting for the runs in progress; nothing new starts.
    Waiting,
    /// Stopping the runs in progress.
    Stopping,
    /// The host has stopped, or never started: the app exits.
    Done,
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitEvent {
    /// The tray's Quit, Cmd+Q or the system asked to quit; the host as it was at that moment.
    Requested { host_ready: bool, running: usize },
    /// The panel's answer: wait for the runs, then quit.
    Wait,
    /// The panel's answer: stop the runs now, also while already waiting.
    StopNow,
    /// A `Host::stop` returned.
    Stopped,
    /// The panel window is being closed.
    WindowClosed,
}

/// What `handle` does after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitStep {
    Nothing,
    /// Show the panel and ask (`quit-requested` with the number of runs in progress).
    Ask(usize),
    /// Show the panel again: it still shows the waiting prompt with "stop now".
    ShowPanel,
    /// Stop the host as the mode says, in the background; `Stopped` follows.
    Stop(StopMode),
    /// Hide the window instead of letting it close.
    HideWindow,
    /// Exit the process.
    Exit,
}

/// The next state and what to do, for `event` in `state`.
pub fn next(state: QuitState, event: QuitEvent) -> (QuitState, QuitStep) {
    use QuitEvent as E;
    use QuitState as S;
    match (state, event) {
        (S::Done, E::WindowClosed) => (S::Done, QuitStep::Nothing),
        (_, E::WindowClosed) => (state, QuitStep::HideWindow),
        (S::Idle | S::Asking, E::Requested { host_ready: false, .. }) => (S::Done, QuitStep::Exit),
        (S::Idle | S::Asking, E::Requested { running: 0, .. }) => (S::Stopping, QuitStep::Stop(StopMode::StopRuns)),
        (S::Idle | S::Asking, E::Requested { running, .. }) => (S::Asking, QuitStep::Ask(running)),
        (S::Waiting, E::Requested { .. }) => (S::Waiting, QuitStep::ShowPanel),
        (S::Idle | S::Asking, E::Wait) => (S::Waiting, QuitStep::Stop(StopMode::WaitForRuns)),
        (S::Idle | S::Asking | S::Waiting, E::StopNow) => (S::Stopping, QuitStep::Stop(StopMode::StopRuns)),
        (S::Waiting | S::Stopping, E::Stopped) => (S::Done, QuitStep::Exit),
        _ => (state, QuitStep::Nothing),
    }
}

/// Applies `event`: moves quitting to its next state and does what the step says. Returns the
/// step, which the window hook reads (`HideWindow`).
pub fn handle(app: &AppHandle, event: QuitEvent) -> QuitStep {
    let state = app.state::<AppState>();
    let step = {
        let mut quit = state.quit.lock().unwrap_or_else(PoisonError::into_inner);
        let (next_state, step) = next(*quit, event);
        *quit = next_state;
        step
    };
    match step {
        QuitStep::Nothing | QuitStep::HideWindow => {}
        QuitStep::Ask(running) => {
            window::show(app);
            logging::debug_failed(app.emit(events::QUIT_REQUESTED, running), "asking the panel about quitting");
        }
        QuitStep::ShowPanel => window::show(app),
        QuitStep::Stop(mode) => match state.try_host() {
            Some(host) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    host.stop(mode).await;
                    handle(&app, QuitEvent::Stopped);
                });
            }
            // Nothing to stop.
            None => {
                handle(app, QuitEvent::Stopped);
            }
        },
        QuitStep::Exit => app.exit(0),
    }
    step
}

/// The tray's Quit, Cmd+Q and the system's exit request, with the host as it is now.
pub fn request(app: &AppHandle) {
    let host = app.state::<AppState>().try_host();
    let running = host.as_ref().map_or(0, |host| host.engine().status().running);
    handle(app, QuitEvent::Requested { host_ready: host.is_some(), running });
}

/// The app's event loop hook: an exit IssueRelay did not ask for itself (Cmd+Q, the system) goes
/// through `request`; the app's own `exit(0)`, and any exit once quitting is done, go through.
/// This relies on `ExitRequested.code` being `Some` only for `AppHandle::exit` and `None` for
/// Cmd+Q, the system's request (log out, shut down) and the last window closing. Verify that
/// against the resolved Tauri version's documentation: a Cmd+Q that arrived with `Some(code)`
/// would exit without asking, and the runs in progress would be recorded as interrupted.
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
    if let RunEvent::ExitRequested { api, code, .. } = event {
        let done = *app.state::<AppState>().quit.lock().unwrap_or_else(PoisonError::into_inner) == QuitState::Done;
        if code.is_some() || done {
            return;
        }
        api.prevent_exit();
        request(app);
    }
}
```

`notify.rs`：

```rust
//! System notifications for the four moments design 9.5 names, each of which can be switched
//! off. A notification names the issue and says what happened in the user's language; the
//! engine's own text (a failure's reason) stays in the panel, under a translated headline.
use issuerelay_core::{config::NotificationSettings, host::Host, model::{Job, JobState}};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::i18n::Translator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    RunStarted,
    AwaitingAcceptance,
    NeedsInfo,
    Failed,
}

/// The notification a job's change calls for: only when its state changed into one of the four.
pub fn notification_for(previous: Option<JobState>, job: &Job) -> Option<NotificationKind> {
    if previous == Some(job.state) {
        return None;
    }
    match job.state {
        JobState::Running => Some(NotificationKind::RunStarted),
        JobState::AwaitingAcceptance => Some(NotificationKind::AwaitingAcceptance),
        JobState::NeedsInfo => Some(NotificationKind::NeedsInfo),
        JobState::Failed => Some(NotificationKind::Failed),
        _ => None,
    }
}

pub fn enabled(kind: NotificationKind, settings: &NotificationSettings) -> bool {
    match kind {
        NotificationKind::RunStarted => settings.run_started,
        NotificationKind::AwaitingAcceptance => settings.awaiting_acceptance,
        NotificationKind::NeedsInfo => settings.needs_info,
        NotificationKind::Failed => settings.failed,
    }
}

/// Title and body: the issue (its title was cleaned when the engine stored it) and a translated
/// sentence. Never the engine's text.
pub fn content(kind: NotificationKind, job: &Job, t: &Translator) -> (String, String) {
    let title = format!("{} {}", job.issue_identifier, job.title);
    let key = match kind {
        NotificationKind::RunStarted => "notify.run_started",
        NotificationKind::AwaitingAcceptance => "notify.awaiting_acceptance",
        NotificationKind::NeedsInfo => "notify.needs_info",
        NotificationKind::Failed => "notify.failed",
    };
    (title, t.t(key))
}

/// Shows the notification unless the settings switched that kind off.
pub fn show(app: &AppHandle, host: &Host, kind: NotificationKind, job: &Job) {
    let settings = host.engine().settings();
    if !enabled(kind, &settings.notifications) {
        return;
    }
    let (title, body) = content(kind, job, &Translator::for_settings(&settings));
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "could not show a notification");
    }
}
```

`events.rs`：

```rust
//! Forwards the engine's events to the panel, turns job changes into notifications and keeps
//! the tray current. A panel that fell behind gets one `engine-resync` and reads everything again.
use std::collections::HashMap;

use issuerelay_core::{
    engine::EngineEvent,
    host::Host,
    model::{Job, JobState},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

use crate::{logging, notify, tray};

pub const ENGINE_EVENT: &str = "engine-event";
/// Events were lost: the panel reads everything again.
pub const RESYNC_EVENT: &str = "engine-resync";
pub const QUIT_REQUESTED: &str = "quit-requested";
pub const STARTUP_FAILED: &str = "startup-failed";

/// How many recent jobs seed the known states, so that existing jobs do not notify on start.
const SEED_JOBS: usize = 500;

/// The recent jobs' states; `None`, with a warning, when the job records cannot be read.
fn seed(host: &Host) -> Option<HashMap<i64, JobState>> {
    match host.engine().list_jobs(SEED_JOBS) {
        Ok(jobs) => Some(jobs.into_iter().map(|job| (job.id, job.state)).collect()),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "could not read the jobs; no job notifies until they can be read");
            None
        }
    }
}

/// The notification a job's change calls for. Without a seed nothing notifies: the seed is tried
/// again (`reseed`) and the job then counts as known, never as new, so a store error cannot make
/// every job look new.
fn notification(
    known: &mut Option<HashMap<i64, JobState>>,
    job: &Job,
    reseed: impl FnOnce() -> Option<HashMap<i64, JobState>>,
) -> Option<notify::NotificationKind> {
    let Some(states) = known.as_mut() else {
        *known = reseed();
        if let Some(states) = known.as_mut() {
            states.insert(job.id, job.state);
        }
        return None;
    };
    let previous = states.insert(job.id, job.state);
    notify::notification_for(previous, job)
}

/// What the forwarder does with one result from the engine's channel.
#[derive(Debug, PartialEq)]
pub enum Forward {
    /// Boxed: a job event carries a whole `Job`, which would make every `Forward` that large.
    Event(Box<EngineEvent>),
    /// Events were lost: one resync, then forwarding goes on.
    Resync,
    /// The engine is gone.
    Stop,
}

pub fn forward(received: Result<EngineEvent, RecvError>) -> Forward {
    match received {
        Ok(event) => Forward::Event(Box::new(event)),
        Err(RecvError::Lagged(_)) => Forward::Resync,
        Err(RecvError::Closed) => Forward::Stop,
    }
}

pub fn spawn_forwarder(app: AppHandle, host: Host) {
    tauri::async_runtime::spawn(async move {
        let mut events = host.engine().subscribe();
        let mut known = seed(&host);
        loop {
            match forward(events.recv().await) {
                Forward::Event(event) => {
                    if let EngineEvent::Job { job } = event.as_ref()
                        && let Some(kind) = notification(&mut known, job, || seed(&host))
                    {
                        notify::show(&app, &host, kind, job);
                    }
                    logging::debug_failed(app.emit(ENGINE_EVENT, event.as_ref()), "forwarding an engine event");
                    tray::refresh(&app, &host);
                }
                Forward::Resync => {
                    tracing::debug!("the panel fell behind the engine's events");
                    known = seed(&host);
                    logging::debug_failed(app.emit(RESYNC_EVENT, ()), "asking the panel to read everything again");
                    tray::refresh(&app, &host);
                }
                Forward::Stop => return,
            }
        }
    });
}
```

`tray.rs`（图标字节由 Task 11 换成正式图标；此处先读占位文件）：

```rust
//! The tray icon and its menu (design 9.1): the monitoring status, pause or resume, check now,
//! open the panel, quit. macOS shows a monochrome template image, Windows a colour one. Which
//! state is a problem is the rule app/src/lib/problem-cases.json pins down; the panel's sidebar
//! follows the same cases.
use issuerelay_core::{engine::StatusSnapshot, host::{Host, LinearSetup}};
use tauri::{
    AppHandle, Manager,
    image::Image,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{i18n::Translator, logging, quit, state::AppState, window};

pub const TRAY_ID: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Running,
    Paused,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    pub state: TrayState,
    pub status_line: String,
    pub toggle_label: String,
}

/// What the tray shows for the engine's status. A problem (not connected to Linear, the last
/// check failed, a job could not start) wins over paused, which wins over running. While the
/// first connection is under way (`Connecting`, the host no longer waits for it) nothing is a
/// problem yet: the checks fail until it is done.
pub fn tray_view(status: &StatusSnapshot, setup: &LinearSetup, t: &Translator) -> TrayView {
    let problem = match setup {
        LinearSetup::Connected { .. } => {
            (status.last_poll_at.is_some() && !status.linear_ok) || status.last_error.is_some()
        }
        LinearSetup::Connecting => false,
        _ => true,
    };
    let state = if problem {
        TrayState::Problem
    } else if status.paused {
        TrayState::Paused
    } else if status.running > 0 {
        TrayState::Running
    } else {
        TrayState::Idle
    };
    let mut status_line = if status.paused {
        t.t("tray.status_paused")
    } else {
        t.t_with(
            "tray.status_counts",
            &[("running", &status.running.to_string()), ("queued", &status.queued.to_string())],
        )
    };
    if problem {
        status_line = format!("{status_line} · {}", t.t("tray.status_problem"));
    }
    let toggle_label = if status.paused { t.t("tray.resume") } else { t.t("tray.pause") };
    TrayView { state, status_line, toggle_label }
}

fn icon(state: TrayState) -> Image<'static> {
    let bytes: &'static [u8] = match state {
        TrayState::Idle => include_bytes!("../icons/tray/idle.png"),
        TrayState::Running => include_bytes!("../icons/tray/running.png"),
        TrayState::Paused => include_bytes!("../icons/tray/paused.png"),
        TrayState::Problem => include_bytes!("../icons/tray/problem.png"),
    };
    Image::from_bytes(bytes).expect("tray icons are valid PNG files")
}

/// The menu items `refresh` updates.
struct TrayItems {
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    poll: MenuItem<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let t = Translator::new(crate::i18n::resolve_language("system", sys_locale::get_locale().as_deref()));
    let status = MenuItem::with_id(app, "status", t.t("tray.starting"), false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", t.t("tray.pause"), false, None::<&str>)?;
    let poll = MenuItem::with_id(app, "poll", t.t("tray.poll_now"), false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", t.t("tray.open_panel"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t.t("tray.quit"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &poll,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(TrayItems { status, toggle, poll, open, quit });
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(TrayState::Idle))
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("IssueRelay")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::show(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn on_menu(app: &AppHandle, event: MenuEvent) {
    let host = app.state::<AppState>().try_host();
    match (event.id().as_ref(), host) {
        ("toggle", Some(host)) => {
            let engine = host.engine();
            let switched = if engine.status().paused { engine.resume() } else { engine.pause() };
            if let Err(e) = switched {
                tracing::warn!(error = %format!("{e:#}"), "could not switch monitoring");
            }
            refresh(app, &host);
        }
        ("poll", Some(host)) => host.engine().poll_now(),
        ("open", _) => window::show(app),
        ("quit", _) => quit::request(app),
        _ => {}
    }
}

/// Brings the tray in line with the engine, in the language the settings ask for.
pub fn refresh(app: &AppHandle, host: &Host) {
    let t = Translator::for_settings(&host.engine().settings());
    let view = tray_view(&host.engine().status(), &host.linear_setup(), &t);
    match app.try_state::<TrayItems>() {
        Some(items) => {
            logging::warn_failed(items.status.set_text(&view.status_line), "updating the tray status");
            logging::warn_failed(items.toggle.set_text(&view.toggle_label), "updating the tray menu");
            logging::warn_failed(items.toggle.set_enabled(true), "updating the tray menu");
            logging::warn_failed(items.poll.set_text(t.t("tray.poll_now")), "updating the tray menu");
            logging::warn_failed(items.poll.set_enabled(true), "updating the tray menu");
            logging::warn_failed(items.open.set_text(t.t("tray.open_panel")), "updating the tray menu");
            logging::warn_failed(items.quit.set_text(t.t("tray.quit")), "updating the tray menu");
        }
        None => tracing::warn!("the tray menu is missing"),
    }
    match app.tray_by_id(TRAY_ID) {
        Some(tray) => {
            logging::warn_failed(tray.set_icon(Some(icon(view.state))), "updating the tray icon");
            logging::warn_failed(tray.set_icon_as_template(cfg!(target_os = "macos")), "updating the tray icon");
        }
        None => tracing::warn!("the tray icon is missing"),
    }
}
```

（`Translator` 只指向解析过一次的翻译表，每个引擎事件 `refresh` 一次也不再解析 JSON。）

`startup.rs`：

```rust
//! Starts the host in the background so the tray appears at once, then decides whether to show
//! the panel (the window is shown once `Host::start` returned or failed; it no longer waits for
//! Linear, which connects in the background). How the start went is kept in `AppState` (`startup_state`), so a
//! panel that opens later still learns it; `startup-failed` tells a panel that is already open.
use std::{path::Path, sync::Arc};

use issuerelay_core::{
    host::{Host, HostParts, SystemConnector},
    ipc::server::AlreadyRunning,
    paths::AppPaths,
    text::clean_text,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    events, logging,
    state::{AppState, StartupCode, StartupFailure, StartupState},
    tray, window,
};

pub async fn start(app: AppHandle) {
    let result = match AppPaths::system() {
        Ok(paths) => {
            let state_db = paths.state_db();
            Host::start(paths, Arc::new(SystemConnector), HostParts::default())
                .await
                .map_err(|error| failure(&error, &state_db))
        }
        Err(error) => Err(StartupFailure { code: StartupCode::Other, detail: clean_text(&format!("{error:#}")) }),
    };
    let state = app.state::<AppState>();
    match &result {
        Ok(host) => {
            events::spawn_forwarder(app.clone(), host.clone());
            tray::refresh(&app, host);
            // A login start stays in the tray (lib.rs hid it already); the first run always shows
            // the guide.
            if !state.autostarted || !host.engine().settings().onboarding_completed {
                window::show(&app);
            }
        }
        Err(failure) => {
            tracing::error!(code = ?failure.code, detail = %failure.detail, "IssueRelay could not start its engine");
            window::show(&app);
        }
    }
    state.set_host(result);
    // Recorded first, then announced: a panel that hears this and asks `startup_state` gets the
    // same answer.
    if let StartupState::Failed(failure) = state.startup_state() {
        logging::debug_failed(
            app.emit(events::STARTUP_FAILED, StartupState::Failed(failure)),
            "telling the panel the start failed",
        );
    }
}

/// A start failure as the panel shows it: a code it translates, and the cleaned error as the
/// detail line.
pub fn failure(error: &anyhow::Error, state_db: &Path) -> StartupFailure {
    let message = format!("{error:#}");
    let folder = state_db.parent().unwrap_or(state_db);
    let code = if error.downcast_ref::<AlreadyRunning>().is_some() {
        StartupCode::AlreadyRunning
    } else if message.contains(&state_db.display().to_string())
        || message.starts_with(&format!("cannot create {}", folder.display()))
    {
        // `Store::open` names the database file, or its folder when it cannot create it (on macOS
        // the IPC endpoint creates that same folder first, in the same words): either way the run
        // records cannot be kept. The other IPC errors name the socket or its lock and stay
        // `Other`.
        StartupCode::StoreUnavailable
    } else {
        StartupCode::Other
    };
    StartupFailure { code, detail: clean_text(&message) }
}
```

`commands.rs`（空壳，Task 4 写入全部命令并删掉这个 `handler()`）：

```rust
//! The commands the panel calls; Task 4 fills them in.

/// No commands yet.
pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![]
}
```

（若 `generate_handler!` 不能这样返回，就在 `lib.rs` 里直接写 `.invoke_handler(tauri::generate_handler![])`，`commands.rs` 只留模块注释，Task 4 在 `lib.rs` 那里补全命令列表。）

`lib.rs`：

```rust
//! IssueRelay's desktop app: the tray icon and the panel around the engine.
// Every module is `pub`: only this crate's own executable uses the library, and what the panel's
// commands (Task 4) use later, such as `AppState::host`, is then not dead code before that.
pub mod commands;
pub mod events;
pub mod i18n;
pub mod logging;
pub mod notify;
pub mod quit;
pub mod startup;
pub mod state;
pub mod tray;
pub mod window;
#[cfg(test)]
mod test_support;

use tauri::Manager;

/// Passed by the login item, so the app starts in the tray (design 9.2: autostart is switched
/// on by the user in the settings).
pub const AUTOSTART_ARG: &str = "--autostart";

/// Starts the app and runs its event loop until it exits.
pub fn run() {
    let autostarted = std::env::args().any(|arg| arg == AUTOSTART_ARG);
    let logs_dir = issuerelay_core::paths::AppPaths::system().ok().map(|paths| paths.logs_dir());
    let _log_guard = logging::init(logs_dir.as_deref());
    tauri::Builder::default()
        // Must come first: a second launch only brings the running instance's panel forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window::show(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::new(autostarted))
        .invoke_handler(commands::handler())
        .setup(|app| {
            tray::create(app.handle())?;
            if app.state::<state::AppState>().autostarted {
                // A login start stays in the tray: through the same `hide` as closing the panel,
                // so macOS shows no Dock icon either.
                window::hide(app.handle());
            }
            tauri::async_runtime::spawn(startup::start(app.handle().clone()));
            Ok(())
        })
        .on_window_event(window::on_event)
        .build(tauri::generate_context!())
        .expect("IssueRelay could not start")
        .run(quit::on_run_event);
}
```

（日志写到数据目录的 `logs/app.<日期>.log`，与运行日志 `<任务 ID>.jsonl` 同目录但文件名不同；core 的日志清理只删 `<数字>.jsonl`，不会误删。）

Run: `cargo test -p issuerelay-app` 与 `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: PASS。

- [ ] **Step 5: 手动冒烟**

Run: `pnpm --dir app tauri dev`（本机，开发版）
Expected: 菜单栏出现托盘图标；菜单里有一行状态与四个操作（暂停 / 恢复、立即检查一次、打开面板、退出）；窗口按「首次引导未完成」显示；关闭窗口后程序仍在托盘、程序坞图标消失；托盘「退出」后进程结束（没有任务在跑时不提问）；`logs/` 下出现当天的 `app.<日期>.log`，内容里没有转义序列。把观察到的结果写进报告。

- [ ] **Step 6: 文档与提交**

- 设计第 4.1 节补：「托盘程序启动时先出托盘图标，宿主在后台启动（解析环境、打开运行记录需要几秒；Linear 的首次连接不在其中，它在后台进行，期间显示『正在连接 Linear…』）；面板的命令等宿主就绪。启动状态（启动中 / 就绪 / 失败及原因代码：另一个宿主已在运行、运行记录打不开、其他）随时可查，面板据此显示启动失败页。开机自启时停在托盘，其余情况显示面板；macOS 上面板隐藏时（含开机自启）不显示程序坞图标。退出由一个状态机决定：有任务在跑时先问，选了等待仍可改为立即停止，等待期间关窗口只隐藏窗口。」第 9.1 节补托盘图标四种状态的判定顺序（出问题 > 已暂停 > 运行中 > 空闲；「出问题」的用例表在 `app/src/lib/problem-cases.json`，托盘与面板共用）。第 11 节补：「桌面应用的程序日志写在 `logs/app.<日期>.log`（每天一个，保留 7 个），每行先去掉控制字符与不可见字符。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 托盘、通知、退出状态机、启动状态 | app/src-tauri/src/tray.rs、notify.rs、quit.rs、state.rs、startup.rs |` 与 `| 日志逐行清洗（devhost 与桌面应用共用） | crates/core/src/logscrub.rs |`；`.github/CODEOWNERS` 加 `/app/src-tauri/src/`（宿主接入与命令都是高风险）。

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs .github Cargo.lock
git commit -m "feat(app): host the engine in the tray app with notifications and a quit prompt"
```

---

### Task 4: 面板命令、两处 core 补充与前端数据层

面板调用的全部 Tauri 命令（Rust），面板需要而 core 还没给的两样数据（暂缓任务的重试时间、结构化的等待原因），以及前端的类型、命令封装、显示文本清洗和假后端（TypeScript）。假后端让面板在普通浏览器里也能完整运行：开发、组件测试、截图都用它，数据全部虚构，并且写成具体的数据表（`mock-fixtures.ts`），后续测试依赖的每个确切字符串都定义在这一处。

**Files:**
- Modify（core）：`crates/core/src/engine.rs`（`StatusSnapshot.held`、`HeldJob`、`IssueStatus::Waiting` 的原因换成 `WaitReasonView`）、`crates/core/src/trigger.rs`（`WaitReasonView`、`WaitReason::view`）、`crates/core/tests/engine.rs`
- Modify: `app/src-tauri/src/commands.rs`（Task 3 的空壳：写入全部命令，删掉空的 `handler()`）、`app/src-tauri/src/lib.rs`（注册命令）、`app/src-tauri/Cargo.toml` 与 `Cargo.lock`（`url`）、`app/src-tauri/src/test_support.rs`（`checked()` 补 `held`，随 Step 1 的 core 提交）
- Create: `app/src/lib/types.ts`、`app/src/lib/backend.ts`、`app/src/lib/api.ts`、`app/src/lib/clean.ts`、`app/src/lib/clean-cases.json`、`app/src/lib/mock-fixtures.ts`、`app/src/lib/mock.ts`
- Test: `commands.rs` 末尾 `mod tests`、`app/src/lib/api.test.ts`、`app/src/lib/mock.test.ts`、`app/src/lib/clean.test.ts`
- Modify: `docs/ai-coding/AI_CODING_GUIDE.md`（第 5、7 节）、`docs/ai-coding/TESTING_SPEC.md`（假实现清单）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 9.2 节）

**Interfaces:**
- Produces（core）：`pub struct engine::HeldJob { pub job_id: i64, pub retry_at: DateTime<Utc> }`；`StatusSnapshot` 加 `#[serde(default)] pub held: Vec<HeldJob>`（因启动失败而暂缓、仍在排队的任务，按 `job_id` 升序；`retry_at` 是暂缓开始时按本机时钟算出的下次尝试时间，只供显示）；`pub enum trigger::WaitReasonView { ProjectNotBound, DirectoryMissing { directory: String }, ExecutorNotReady { executor: ExecutorKind } }`（serde `tag = "code"`：`{"code":"project_not_bound"}`、`{"code":"directory_missing","directory":"…"}`、`{"code":"executor_not_ready","executor":"claude"}`）、`pub fn WaitReason::view(&self) -> WaitReasonView`；`IssueStatus::Waiting { reason: WaitReasonView }`。`WaitReason::describe()` 不变：任务的 `reason` 与命令行照旧用这句英文。
- Produces（Rust 命令名 = 前端 `invoke` 名；前端参数名用 camelCase，Tauri 自动转成 snake_case；共 34 条）：

| 命令 | 参数 | 返回 |
|---|---|---|
| `app_info` | — | `AppInfo { version, cli_path: Option<String>, platform, system_locale: Option<String> }`（不等宿主；`system_locale` 是 Rust 读到的系统语言，面板据此解析「跟随系统」） |
| `startup_state` | — | `StartupState`（Task 3；不等宿主） |
| `status` | — | `StatusView { status: StatusSnapshot, linear: LinearSetup }` |
| `list_jobs` | `limit: usize`（夹在 1..=500） | `Vec<Job>` |
| `job` | `id: i64` | `Option<Job>` |
| `job_log` | `id: i64, lines: usize`（夹在 1..=5000） | `Vec<String>` |
| `issues` | — | `Vec<IssueView>` |
| `pause` / `resume` / `poll_now` | — | `()` |
| `cancel_job` / `retry_job` / `mark_done` | `id: i64` | `()` |
| `bindings` | — | `Vec<Binding>` |
| `save_binding` | `binding: Binding` | `()` |
| `remove_binding` | `id: String` | `bool` |
| `projects` | — | `Vec<Project>` |
| `settings` / `save_settings` | — / `settings: Settings` | `Settings` / `()` |
| `doctor` | — | `DoctorReport` |
| `linear_login` | — | `Viewer`（在系统浏览器里打开授权页；姓名已清洗；被 `linear_cancel_login` 取消时返回错误代码 `login_cancelled`） |
| `linear_cancel_login` | — | `()`（放弃等浏览器的那次登录；没有在等的登录时什么也不做） |
| `linear_use_api_key` | `key: String` | `Viewer`（姓名已清洗） |
| `linear_logout` | — | `()` |
| `linear_reconnect` | — | `LinearSetup` |
| `setup_action` | `action: SetupAction` | `()`（命令的输出不显示：做没做成看随后自动重跑的环境检查；失败时错误里带清洗过的输出） |
| `control_servers` | — | `ControlServers { claude: ControlServer, codex: ControlServer }`，每个执行者一个状态：`registered` / `not_registered` / `not_installed` / `unknown`（检查本身失败——超时、起不来、被信号结束——记一条警告并报 `unknown`，从不当成「没装」） |
| `linear_server_named` | `executor: ExecutorKind` | `bool`：Linear 的 MCP 服务是否以一键授权用的名字注册（Claude `linear-server`、Codex `linear`，名字取自 core 的 `setup::arguments`）；环境检查按网址认 Linear，名字不同时一键授权用不了，面板改为给出手动命令 |
| `open_linear` | `url: String`（`is_linear_url`：`url::Url` 解析后协议为 `https`、主机为 `linear.app` 或其子域名） | `()`；其他链接返回错误代码 `not_a_linear_link`（面板翻译它） |
| `choose_directory` | — | `Option<String>` |
| `autostart_enabled` / `set_autostart` | — / `enabled: bool` | `bool` / `()` |
| `quit` | `mode: QuitMode`（`"wait"` / `"stop"`） | `()`（交给 Task 3 的退出状态机：`QuitEvent::Wait` / `QuitEvent::StopNow`） |
| `request_quit` | — | `()`（与托盘「退出」相同；宿主没起来时直接退出，供启动失败页用） |

  - 命令的两条约定（第一部分最终修复波）：① `Host::env()` 给的是真实的代理值（URL 里的用户名和密码没有打码，见 core 里 `Host::env` 的文档），命令只拿它启动进程（`setup_action`、`control_servers`、`linear_server_named`），从不把它、也不把其中的代理值返回给面板；面板看到的环境只有 `doctor` 返回的、`Engine::doctor` 打过码的那一份。② 等浏览器的登录可以取消：`linear_login` 把这次登录的取消令牌记在 `PendingLogin`（Tauri 托管的状态）里，与 `Host::login` 的 future 竞速；`linear_cancel_login` 一取消，这个 future 就被丢掉（core 写明丢掉它就放弃这次登录，钥匙串只在浏览器回调之后才写），`linear_login` 返回错误代码 `login_cancelled`，面板不把它当失败显示；新的登录开始时，前一次还在等的登录先被取消。core 一次只做一个登录改动，等浏览器的登录最长占着它 10 分钟，所以面板在登录等待期间不让改用 API key（Task 6、9、10）。
  - 前端：`types.ts` 里与上面一一对应的类型（含 `HeldJob`、`WaitReason`、`StartupState`）；叶子模块 `backend.ts`：`interface Backend`（方法名为命令名的 camelCase，外加 `onEngineEvent`、`onResync`、`onQuitRequested`、`onStartupFailed` 四个订阅）、`class BackendError extends Error`（`code: ErrorCode | null`）、`ERROR_CODES`；`api.ts`：`isTauri()`、`tauriBackend`、`loadBackend(): Promise<Backend>`（在 Tauri 里用真实命令，否则用 `import()` 动态载入的假后端）；`api.ts` 与 `mock.ts` 都只从 `backend.ts`、`types.ts` 导入，彼此不互相导入；`createMockBackend(scenario: "demo" | "fresh" | "connecting", options?: { latencyMs?: number }): Backend`（`connecting`：demo 的本地数据，Linear 还在首次连接，`linearReconnect()` 之后连上；`latencyMs` 默认 120，测试传 0；构造时 `setFixedClock(MOCK_NOW)`）；`mock-fixtures.ts` 的 `ago`、`DEMO_*`、`FRESH_DOCTOR`、`DEFAULT_SETTINGS`；`clean.ts`：`cleanDisplayText(text: string): string`、`cleanDisplayLines(text: string): string`。
- Consumes: Task 1 的 `MOCK_NOW`、`setFixedClock`；Task 3 的 `AppState`、`StartupState`、`quit::{handle, QuitEvent}`、`tray::refresh`、`events::*`、`test_support`；第一部分的 `setup::{self, SetupAction}`、`executor::resolve_program_in`、`doctor::{SystemRunner, CommandRunner}`、`host::*`（`LinearSetup` 含 AR51 的 `Connecting`）、`text::clean_text`（Task 3 起建在 `text::strip_control` 上）。

- [ ] **Step 1: core 补充：暂缓任务的重试时间与结构化等待原因（先写失败测试）**

面板要在排队区与运行记录里显示「几点重试」（引擎因启动失败暂缓的任务，原来只在私有表里记着单调时钟），需求页要按代码翻译等待原因（原来只有一句英文）。两样都加在 core，各带测试。

`crates/core/tests/engine.rs` 加（`use` 里补 `issuerelay_core::trigger::WaitReasonView`）：

```rust
#[tokio::test]
async fn a_job_held_back_after_a_failed_start_says_when_it_is_tried_again() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert!(h.engine.status().held.is_empty());
    h.linear.fail_issue_reads("1");
    let before = chrono::Utc::now();
    h.engine.dispatch().await.unwrap();
    let after = chrono::Utc::now();
    let id = h.latest("1").id;
    let held = h.engine.status().held;
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].job_id, id);
    // The first pause is 30 s, given as a wall-clock time the panel can show.
    let pause = chrono::Duration::seconds(30);
    assert!(held[0].retry_at >= before + pause && held[0].retry_at <= after + pause, "{held:?}");
    let json = serde_json::to_value(h.engine.status()).unwrap();
    assert_eq!(json["held"][0]["job_id"], id);
    // A stopped job is no longer listed, and a retry starts afresh.
    h.engine.cancel_job(id).unwrap();
    assert!(h.engine.status().held.is_empty(), "a stopped job is not held");
    h.engine.retry_job(id).unwrap();
    assert!(h.engine.status().held.is_empty(), "a retry starts afresh");
}
```

`the_issue_list_explains_what_each_labeled_issue_is_waiting_for` 里的 `assert!(matches!(status_of(&views, "3"), IssueStatus::Waiting { .. }));` 改为：

```rust
    assert_eq!(
        status_of(&views, "3"),
        IssueStatus::Waiting {
            reason: WaitReasonView::ProjectNotBound
        }
    );
```

`events_and_issue_statuses_have_the_json_shape_the_panel_reads` 里 `Waiting` 那一项换成三项，末尾旧快照的断言后加一行：

```rust
        (
            IssueStatus::Waiting {
                reason: WaitReasonView::ProjectNotBound,
            },
            serde_json::json!({"kind": "waiting", "reason": {"code": "project_not_bound"}}),
        ),
        (
            IssueStatus::Waiting {
                reason: WaitReasonView::DirectoryMissing {
                    directory: "demo-app".into(),
                },
            },
            serde_json::json!({"kind": "waiting", "reason": {"code": "directory_missing", "directory": "demo-app"}}),
        ),
        (
            IssueStatus::Waiting {
                reason: WaitReasonView::ExecutorNotReady {
                    executor: ExecutorKind::Claude,
                },
            },
            serde_json::json!({"kind": "waiting", "reason": {"code": "executor_not_ready", "executor": "claude"}}),
        ),
```

```rust
    assert!(older.held.is_empty(), "a host from before `held` existed sends none");
```

`crates/core/src/trigger.rs` 测试模块加：

```rust
    #[test]
    fn wait_reasons_have_a_code_the_panel_translates() {
        let shapes = [
            (WaitReason::ProjectNotBound, serde_json::json!({"code": "project_not_bound"})),
            (
                WaitReason::DirectoryMissing(PathBuf::from("demo-app")),
                serde_json::json!({"code": "directory_missing", "directory": "demo-app"}),
            ),
            (
                WaitReason::ExecutorNotReady(ExecutorKind::Codex),
                serde_json::json!({"code": "executor_not_ready", "executor": "codex"}),
            ),
        ];
        for (reason, json) in shapes {
            assert_eq!(serde_json::to_value(reason.view()).unwrap(), json);
            assert_eq!(serde_json::from_value::<WaitReasonView>(json).unwrap(), reason.view());
        }
        // The stored job reason and the CLI keep the English sentence.
        assert_eq!(WaitReason::ProjectNotBound.describe(), "project is not bound to a local directory");
    }
```

Run: `cargo test -p issuerelay-core`
Expected: 编译失败（`held`、`WaitReasonView` 还不存在）。

`crates/core/src/trigger.rs`（`WaitReason` 下面；文件头补 `use serde::{Deserialize, Serialize};`）：

```rust
/// `WaitReason` as the panel reads it: a code it translates, with the folder or executor the
/// reason names. `describe` stays the text the job's reason and the CLI show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum WaitReasonView {
    ProjectNotBound,
    DirectoryMissing { directory: String },
    ExecutorNotReady { executor: ExecutorKind },
}

impl WaitReason {
    /// The structured form, for the panel's issue list.
    pub fn view(&self) -> WaitReasonView {
        match self {
            WaitReason::ProjectNotBound => WaitReasonView::ProjectNotBound,
            WaitReason::DirectoryMissing(dir) => WaitReasonView::DirectoryMissing {
                directory: dir.display().to_string(),
            },
            WaitReason::ExecutorNotReady(kind) => WaitReasonView::ExecutorNotReady { executor: *kind },
        }
    }
}
```

`crates/core/src/engine.rs`：

- `use crate::trigger::{self, Eligibility, WaitReason, WaitReasonView, same_name};`。
- `IssueStatus::Waiting` 改为 `Waiting { reason: WaitReasonView }`（文档注释改为「It waits for a binding, a folder or a ready executor; `reason` says which, as a code the panel translates.」），`issues()` 里 `reason: reason.describe()` 改为 `reason: reason.view()`。
- `StatusSnapshot` 末尾加：

```rust
    /// Queued jobs passed over after a failed start, with when each is tried again (this
    /// machine's clock, for display only), by job id. A host from before this field sends none.
    #[serde(default)]
    pub held: Vec<HeldJob>,
```

- `StatusSnapshot` 下面加：

```rust
/// A queued job whose start failed and that waits before it is tried again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldJob {
    pub job_id: i64,
    /// When the job may start again: the wall-clock time worked out when the pause began.
    pub retry_at: DateTime<Utc>,
}
```

- `StartBackoff` 加字段 `retry_at: DateTime<Utc>`（注释：the same moment as `until`, on the wall clock, for the panel）；`hold_back` 里 `or_insert(StartBackoff { failures: 0, until: now, retry_at: Utc::now() })`，并把最后一行换成：

```rust
        let pause = start_pause(entry.failures);
        entry.until = now + pause;
        entry.retry_at = Utc::now() + chrono::Duration::from_std(pause).expect("a start pause is at most five minutes");
```

- `status()` 里把排队数改为先读排队的任务（单独一条语句读完、放掉存储锁，再读等待数），据此筛出仍在排队的暂缓任务：

```rust
        let queued = self.store().list_by_state(&[JobState::Queued]);
        let counts = queued.and_then(|queued| Ok((queued, self.count(JobState::WaitingBinding)?)));
        let (queued, waiting, held, store_error) = match counts {
            Ok((queued, waiting)) => {
                let ids: HashSet<i64> = queued.iter().map(|job| job.id).collect();
                (queued.len(), waiting, self.held(&ids), None)
            }
            Err(e) => (
                0,
                0,
                Vec::new(),
                Some(clean_text(&format!("cannot read the job records: {e:#}"))),
            ),
        };
```

  `StatusSnapshot { … }` 里加 `held,`。
- `retries_of` 上面加：

```rust
    /// The queued jobs passed over after a failed start whose pause has not ended, by job id.
    /// Only jobs in `queued` count: a pause outlives its job's stay in the queue until the next
    /// dispatch prunes it.
    fn held(&self, queued: &HashSet<i64>) -> Vec<HeldJob> {
        let now = tokio::time::Instant::now();
        let backoff = self.inner.start_backoff.lock().expect("backoff lock");
        let mut held: Vec<HeldJob> = backoff
            .held
            .iter()
            .filter(|(id, pause)| pause.until > now && queued.contains(*id))
            .map(|(id, pause)| HeldJob {
                job_id: *id,
                retry_at: pause.retry_at,
            })
            .collect();
        held.sort_by_key(|job| job.job_id);
        held
    }
```

（锁的次序不变：`held` 只在没有持有存储锁时取暂缓表的锁，与模块文档里列的短同步段一致。）

`app/src-tauri/src/test_support.rs` 的 `checked()` 跟上（它逐个写出字段，少了 `held` 应用 crate 的测试就编译不过）：文档改为「each of `StatusSnapshot`'s 10 fields」，`draining: false,` 后加 `held: Vec::new(),`。

Run: `cargo test -p issuerelay-core` 与 `cargo test --workspace`
Expected: PASS（命令行的 `status` 输出不读 `held`，格式测试照旧通过；应用 crate 的托盘测试照旧通过）。

```bash
git add crates app/src-tauri/src/test_support.rs
git commit -m "feat(core): give the panel held jobs' retry time and structured wait reasons"
```

- [ ] **Step 2: Rust 命令的失败测试（纯逻辑部分）**

`commands.rs` 测试模块：

```rust
#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[test]
    fn only_linear_links_are_opened() {
        assert!(is_linear_url("https://linear.app/demo/issue/DEMO-1"));
        // The OAuth authorization page the login opens lives on a subdomain.
        assert!(is_linear_url("https://mcp.linear.app/authorize"));
        assert!(!is_linear_url("http://linear.app/"));
        assert!(!is_linear_url("http://linear.app/demo/issue/DEMO-1"));
        // User info and look-alike hosts: parsed, the host is evil.example and linear.app.evil.example.
        assert!(!is_linear_url("https://linear.app@evil.example/"));
        assert!(!is_linear_url("https://linear.app.evil.example/"));
        assert!(!is_linear_url("https://evil.example/https://linear.app/"));
        assert!(!is_linear_url("file:///etc/passwd"));
        assert!(!is_linear_url("not a link"));
    }

    #[test]
    fn a_failed_registration_check_is_unknown_never_not_installed() {
        assert_eq!(registration(ExecutorKind::Claude, Ok(true)), ControlServer::Registered);
        assert_eq!(registration(ExecutorKind::Claude, Ok(false)), ControlServer::NotRegistered);
        assert_eq!(
            registration(ExecutorKind::Codex, Err(anyhow::anyhow!("`codex mcp get issuerelay` timed out after 60s"))),
            ControlServer::Unknown
        );
        assert_eq!(serde_json::to_value(ControlServer::NotInstalled).unwrap(), "not_installed");
        assert_eq!(serde_json::to_value(ControlServer::Unknown).unwrap(), "unknown");
    }

    #[test]
    fn the_cli_sits_next_to_the_app() {
        let exe = std::path::Path::new("/Applications/IssueRelay.app/Contents/MacOS/issuerelay-app");
        let expected = if cfg!(windows) { "issuerelay.exe" } else { "issuerelay" };
        assert_eq!(cli_next_to(exe).file_name().unwrap(), expected);
        assert_eq!(cli_next_to(exe).parent(), exe.parent());
    }

    #[test]
    fn list_sizes_are_bounded() {
        assert_eq!(clamp_limit(0, 500), 1);
        assert_eq!(clamp_limit(10_000, 500), 500);
        assert_eq!(clamp_limit(40, 500), 40);
    }

    #[test]
    fn setup_actions_name_their_executor() {
        use issuerelay_core::{config::ExecutorKind::*, setup::SetupAction::*};
        assert_eq!(action_executor(AddLinearMcp(Codex)), Codex);
        assert_eq!(action_executor(LoginLinearMcp(Claude)), Claude);
        assert_eq!(action_executor(RegisterControlServer(Codex)), Codex);
        assert_eq!(action_executor(RemoveControlServer(Claude)), Claude);
    }

    #[test]
    fn the_one_click_login_names_the_server_setup_registers() {
        assert_eq!(linear_server_name(ExecutorKind::Claude).unwrap(), "linear-server");
        assert_eq!(linear_server_name(ExecutorKind::Codex).unwrap(), "linear");
    }

    #[test]
    fn a_new_login_cancels_the_one_still_waiting_and_cancel_ends_the_current_one() {
        let pending = PendingLogin::default();
        let first = pending.begin();
        let second = pending.begin();
        assert!(first.is_cancelled(), "one login waits at a time");
        assert!(!second.is_cancelled());
        pending.cancel();
        assert!(second.is_cancelled());
        // Nothing waits any more: a second cancel does nothing.
        pending.cancel();
    }

    #[test]
    fn quit_modes_read_from_the_panel() {
        assert_eq!(serde_json::from_str::<QuitMode>("\"wait\"").unwrap(), QuitMode::Wait);
        assert_eq!(serde_json::from_str::<QuitMode>("\"stop\"").unwrap(), QuitMode::Stop);
        assert!(serde_json::from_str::<QuitMode>("\"later\"").is_err());
    }

    /// One row of app/src/lib/clean-cases.json, which the panel's `cleanDisplayText` test reads.
    #[derive(Deserialize)]
    struct CleanCase {
        input: String,
        expected: String,
        ansi: bool,
    }

    #[test]
    fn display_text_cases_follow_cores_character_rules() {
        let cases: Vec<CleanCase> = serde_json::from_str(include_str!("../../src/lib/clean-cases.json"))
            .expect("clean-cases.json is valid");
        // Escape sequences are the panel's own addition; every other case is core's character
        // rule, checked against `clean_text` itself (it trims what it returns, the panel does not).
        for case in cases.iter().filter(|case| !case.ansi) {
            assert_eq!(clean_text(&case.input), case.expected.trim(), "{:?}", case.input);
        }
        assert!(
            cases.iter().filter(|case| case.ansi).all(|case| case.input.contains(['\u{1b}', '\u{9b}'])),
            "only escape-sequence cases skip the core check"
        );
    }
}
```

Run: `cargo test -p issuerelay-app commands`
Expected: 编译失败。

- [ ] **Step 3: 实现 `commands.rs`**

```bash
cargo add -p issuerelay-app url tokio-util
```

（`is_linear_url` 用 `url` 解析链接，`PendingLogin` 用 `tokio-util` 的 `CancellationToken`；core 已在用这两个 crate，锁文件不引入新的包。）

```rust
//! The commands the panel calls. Each waits for the host, does one thing and returns something
//! the panel can show; an error comes back as one cleaned line, or as a code the panel
//! translates (`not_a_linear_link`, `login_cancelled`). `mark_done`, `retry_job` and
//! `cancel_job` exist only here (design 5.6): IPC, the CLI and MCP never offer them. Plugins
//! (opener, dialog, autostart) are used from here, so the panel itself needs no plugin
//! permission. `Host::env()` holds the real proxy values (user name and password unmasked): the
//! commands use it only to start processes and never return it or anything taken from it; the
//! environment the panel sees is `Engine::doctor`'s, with the proxy values masked.
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::Context as _;
use issuerelay_core::{
    config::{Binding, ExecutorKind, Settings},
    doctor::{CommandRunner as _, DoctorReport, SystemRunner},
    engine::{IssueView, StatusSnapshot},
    executor,
    host::{Host, LinearSetup},
    linear::{Project, Viewer},
    model::Job,
    setup::{self, SetupAction},
    text::clean_text,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;
use tokio_util::sync::CancellationToken;

use crate::{
    quit::{self, QuitEvent},
    state::{AppState, StartupState},
    tray,
};

type CmdResult<T> = Result<T, String>;

/// The error code `linear_login` answers with when `linear_cancel_login` dropped it; the panel
/// shows no failure for it.
const LOGIN_CANCELLED: &str = "login_cancelled";

/// The browser login that is waiting, so that `linear_cancel_login` can drop it (Tauri managed
/// state, next to `AppState`).
#[derive(Default)]
pub struct PendingLogin(Mutex<Option<CancellationToken>>);

impl PendingLogin {
    /// The token of a login that starts now. A login still waiting is cancelled first: the panel
    /// offers one login at a time.
    fn begin(&self) -> CancellationToken {
        let token = CancellationToken::new();
        let earlier = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(token.clone());
        if let Some(earlier) = earlier {
            earlier.cancel();
        }
        token
    }

    /// Cancels the login that is waiting, if there is one.
    fn cancel(&self) {
        let waiting = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(waiting) = waiting {
            waiting.cancel();
        }
    }
}

/// The most jobs one list returns.
const MAX_JOBS: usize = 500;
/// The most log lines one read returns.
const MAX_LOG_LINES: usize = 5000;
/// How long `mcp get` may take.
const MCP_GET_LIMIT: Duration = Duration::from_secs(60);
/// The error code `open_linear` answers with for any other link; the panel translates it.
const NOT_A_LINEAR_LINK: &str = "not_a_linear_link";

fn error(e: impl Into<anyhow::Error>) -> String {
    clean_text(&format!("{:#}", e.into()))
}

fn clamp_limit(limit: usize, max: usize) -> usize {
    limit.clamp(1, max)
}

/// The only links IssueRelay opens in the browser, from the panel (`open_linear`) and for the
/// Linear login (its authorization page): `https` on `linear.app` or one of its subdomains, such
/// as `mcp.linear.app`. Parsed, so user info (`https://linear.app@evil.example/`) and look-alike
/// hosts (`linear.app.evil.example`) are refused.
fn is_linear_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| host == "linear.app" || host.ends_with(".linear.app"))
    })
}

/// Where the command-line program is installed: next to the app's own executable.
fn cli_next_to(exe: &Path) -> PathBuf {
    let name = if cfg!(windows) { "issuerelay.exe" } else { "issuerelay" };
    exe.with_file_name(name)
}

/// The command-line program, when it is installed next to the app (always in the installed
/// app; in development after `cargo build -p issuerelay-cli`).
fn cli_path() -> Option<PathBuf> {
    let cli = cli_next_to(&std::env::current_exe().ok()?);
    cli.is_file().then_some(cli)
}

fn action_executor(action: SetupAction) -> ExecutorKind {
    match action {
        SetupAction::AddLinearMcp(kind)
        | SetupAction::LoginLinearMcp(kind)
        | SetupAction::RegisterControlServer(kind)
        | SetupAction::RemoveControlServer(kind) => kind,
    }
}

/// The server name the one-click login passes to `mcp login`, read from core's own arguments so
/// the two never differ.
fn linear_server_name(kind: ExecutorKind) -> anyhow::Result<String> {
    setup::arguments(SetupAction::LoginLinearMcp(kind), None)?
        .last()
        .cloned()
        .context("the login command names no server")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuitMode {
    Wait,
    Stop,
}

#[derive(Serialize)]
pub struct StatusView {
    pub status: StatusSnapshot,
    pub linear: LinearSetup,
}

#[derive(Serialize)]
pub struct AppInfo {
    pub version: &'static str,
    pub cli_path: Option<String>,
    pub platform: &'static str,
    /// The system language as Rust reads it (`sys_locale`). The panel resolves "system" from it,
    /// so the panel, the tray and the notifications always agree.
    pub system_locale: Option<String>,
}

/// Whether IssueRelay's control server is registered with one executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlServer {
    Registered,
    NotRegistered,
    NotInstalled,
    /// The check itself failed (timed out, did not start, was killed); it was logged and the
    /// panel offers to check again.
    Unknown,
}

#[derive(Serialize)]
pub struct ControlServers {
    pub claude: ControlServer,
    pub codex: ControlServer,
}

/// What a registration check of an installed executor came to; a failed check is logged.
fn registration(kind: ExecutorKind, checked: anyhow::Result<bool>) -> ControlServer {
    match checked {
        Ok(true) => ControlServer::Registered,
        Ok(false) => ControlServer::NotRegistered,
        Err(e) => {
            tracing::warn!(executor = kind.as_str(), error = %format!("{e:#}"), "could not check the control server registration");
            ControlServer::Unknown
        }
    }
}

async fn host(state: &State<'_, AppState>) -> CmdResult<Host> {
    state.host().await
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        cli_path: cli_path().map(|p| p.display().to_string()),
        platform: std::env::consts::OS,
        system_locale: sys_locale::get_locale(),
    }
}

/// Starting, ready, or failed with a code and a cleaned detail; answers at once.
#[tauri::command]
pub fn startup_state(state: State<'_, AppState>) -> StartupState {
    state.startup_state()
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> CmdResult<StatusView> {
    let host = host(&state).await?;
    Ok(StatusView { status: host.engine().status(), linear: host.linear_setup() })
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>, limit: usize) -> CmdResult<Vec<Job>> {
    host(&state).await?.engine().list_jobs(clamp_limit(limit, MAX_JOBS)).map_err(error)
}

#[tauri::command]
pub async fn job(state: State<'_, AppState>, id: i64) -> CmdResult<Option<Job>> {
    host(&state).await?.engine().job(id).map_err(error)
}

#[tauri::command]
pub async fn job_log(state: State<'_, AppState>, id: i64, lines: usize) -> CmdResult<Vec<String>> {
    host(&state).await?.engine().log_tail(id, clamp_limit(lines, MAX_LOG_LINES)).map_err(error)
}

#[tauri::command]
pub async fn issues(state: State<'_, AppState>) -> CmdResult<Vec<IssueView>> {
    host(&state).await?.engine().issues().await.map_err(error)
}

#[tauri::command]
pub async fn pause(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let host = host(&state).await?;
    host.engine().pause().map_err(error)?;
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn resume(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let host = host(&state).await?;
    host.engine().resume().map_err(error)?;
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn poll_now(state: State<'_, AppState>) -> CmdResult<()> {
    host(&state).await?.engine().poll_now();
    Ok(())
}

#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    host(&state).await?.engine().cancel_job(id).map_err(error)
}

#[tauri::command]
pub async fn retry_job(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    host(&state).await?.engine().retry_job(id).map_err(error)
}

/// The panel's "mark done": IssueRelay's only write to Linear (design 5.6).
#[tauri::command]
pub async fn mark_done(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    host(&state).await?.engine().mark_done(id).await.map_err(error)
}

#[tauri::command]
pub async fn bindings(state: State<'_, AppState>) -> CmdResult<Vec<Binding>> {
    Ok(host(&state).await?.engine().bindings())
}

#[tauri::command]
pub async fn save_binding(state: State<'_, AppState>, binding: Binding) -> CmdResult<()> {
    host(&state).await?.engine().add_binding(binding).map_err(error)
}

#[tauri::command]
pub async fn remove_binding(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    host(&state).await?.engine().remove_binding(&id).map_err(error)
}

#[tauri::command]
pub async fn projects(state: State<'_, AppState>) -> CmdResult<Vec<Project>> {
    host(&state).await?.engine().projects().await.map_err(error)
}

#[tauri::command]
pub async fn settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(host(&state).await?.engine().settings())
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    let host = host(&state).await?;
    host.engine().update_settings(settings).map_err(error)?;
    // The language may have changed: the tray menu follows at once.
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn doctor(state: State<'_, AppState>) -> CmdResult<DoctorReport> {
    Ok(host(&state).await?.engine().doctor().await)
}

#[tauri::command]
pub async fn linear_login(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingLogin>,
) -> CmdResult<Viewer> {
    let host = host(&state).await?;
    let opener = app.clone();
    // `linear_cancel_login` drops the login while it waits for the browser, which abandons it:
    // core writes the keychain only after the browser has answered (`Host::login`).
    let cancelled = pending.begin();
    let login = host.login(move |url| {
        // The same check as `open_linear`: the browser opens nothing but Linear.
        if !is_linear_url(url) {
            tracing::warn!(url = %clean_text(url), "refused to open an authorization page outside linear.app");
            return;
        }
        if let Err(e) = opener.opener().open_url(url, None::<&str>) {
            tracing::warn!(error = %e, "could not open the authorization page");
        }
    });
    let viewer = tokio::select! {
        viewer = login => viewer.map_err(error)?,
        () = cancelled.cancelled() => return Err(LOGIN_CANCELLED.into()),
    };
    tray::refresh(&app, &host);
    Ok(viewer)
}

/// Gives up the browser login that is waiting (the panel's Cancel); nothing happens when none is.
#[tauri::command]
pub fn linear_cancel_login(pending: State<'_, PendingLogin>) {
    pending.cancel();
}

#[tauri::command]
pub async fn linear_use_api_key(app: AppHandle, state: State<'_, AppState>, key: String) -> CmdResult<Viewer> {
    let host = host(&state).await?;
    let viewer = host.use_api_key(key.trim()).await.map_err(error)?;
    tray::refresh(&app, &host);
    Ok(viewer)
}

#[tauri::command]
pub async fn linear_logout(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let host = host(&state).await?;
    host.logout().await.map_err(error)?;
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn linear_reconnect(app: AppHandle, state: State<'_, AppState>) -> CmdResult<LinearSetup> {
    let host = host(&state).await?;
    let setup = host.reconnect().await;
    tray::refresh(&app, &host);
    Ok(setup)
}

/// Runs a one-click setup command with the executor found on the resolved PATH. Its output is not
/// shown: the panel checks again afterwards; a failure carries the cleaned output in its error.
#[tauri::command]
pub async fn setup_action(state: State<'_, AppState>, action: SetupAction) -> CmdResult<()> {
    let host = host(&state).await?;
    let env = host.env();
    let program = executor::resolve_program_in(action_executor(action), env.path.as_deref()).map_err(error)?;
    let runner = SystemRunner::with_env(env.shared());
    setup::run(action, &program, cli_path().as_deref(), &runner).await.map_err(error)?;
    Ok(())
}

#[tauri::command]
pub async fn control_servers(state: State<'_, AppState>) -> CmdResult<ControlServers> {
    let host = host(&state).await?;
    let env = host.env();
    let runner = SystemRunner::with_env(env.clone().shared());
    let mut found = [ControlServer::NotInstalled; 2];
    for (slot, kind) in found.iter_mut().zip([ExecutorKind::Claude, ExecutorKind::Codex]) {
        // Not found on the resolved PATH: not installed. Found: the check decides, and a check
        // that fails is `Unknown`, never "not installed".
        if let Ok(program) = executor::resolve_program_in(kind, env.path.as_deref()) {
            *slot = registration(kind, setup::control_server_registered(&program, &runner).await);
        }
    }
    Ok(ControlServers { claude: found[0], codex: found[1] })
}

/// Whether Linear's MCP server is registered under the name the one-click login uses. The
/// environment check finds Linear by its URL, whatever the name; under another name the
/// one-click login cannot work and the panel shows the manual command instead.
#[tauri::command]
pub async fn linear_server_named(state: State<'_, AppState>, executor: ExecutorKind) -> CmdResult<bool> {
    let host = host(&state).await?;
    let env = host.env();
    let program = executor::resolve_program_in(executor, env.path.as_deref()).map_err(error)?;
    let name = linear_server_name(executor).map_err(error)?;
    let runner = SystemRunner::with_env(env.shared());
    let output = runner.run_limited(&program, &["mcp", "get", &name], MCP_GET_LIMIT).await.map_err(error)?;
    Ok(output.code == Some(0))
}

#[tauri::command]
pub async fn open_linear(app: AppHandle, url: String) -> CmdResult<()> {
    if !is_linear_url(&url) {
        return Err(NOT_A_LINEAR_LINK.into());
    }
    app.opener().open_url(&url, None::<&str>).map_err(error)
}

#[tauri::command]
pub async fn choose_directory(app: AppHandle) -> CmdResult<Option<String>> {
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(error)?;
    Ok(picked
        .and_then(|path| path.into_path().ok())
        .map(|path| path.display().to_string()))
}

#[tauri::command]
pub async fn autostart_enabled(app: AppHandle) -> CmdResult<bool> {
    app.autolaunch().is_enabled().map_err(error)
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let launcher = app.autolaunch();
    if enabled { launcher.enable() } else { launcher.disable() }.map_err(error)
}

/// The quit prompt's answer: wait for the runs (still switchable to "stop now"), or stop them.
/// The quit state machine (Task 3) decides what follows.
#[tauri::command]
pub fn quit(app: AppHandle, mode: QuitMode) {
    let event = match mode {
        QuitMode::Wait => QuitEvent::Wait,
        QuitMode::Stop => QuitEvent::StopNow,
    };
    quit::handle(&app, event);
}

/// The same as the tray's Quit; the startup error page uses it when the host never started.
#[tauri::command]
pub fn request_quit(app: AppHandle) {
    quit::request(&app);
}
```

（`quit::handle` 的返回值只有窗口钩子用得上，这里不需要。）`lib.rs` 在 `.manage(state::AppState::new(autostarted))` 之后加 `.manage(commands::PendingLogin::default())`，把 `.invoke_handler(commands::handler())` 换成全部命令的列表，并删掉 `commands.rs` 里 Task 3 的空 `handler()`：

```rust
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::startup_state,
            commands::status,
            commands::list_jobs,
            commands::job,
            commands::job_log,
            commands::issues,
            commands::pause,
            commands::resume,
            commands::poll_now,
            commands::cancel_job,
            commands::retry_job,
            commands::mark_done,
            commands::bindings,
            commands::save_binding,
            commands::remove_binding,
            commands::projects,
            commands::settings,
            commands::save_settings,
            commands::doctor,
            commands::linear_login,
            commands::linear_cancel_login,
            commands::linear_use_api_key,
            commands::linear_logout,
            commands::linear_reconnect,
            commands::setup_action,
            commands::control_servers,
            commands::linear_server_named,
            commands::open_linear,
            commands::choose_directory,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::quit,
            commands::request_quit,
        ])
```

Run: `cargo test -p issuerelay-app` 与 `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: PASS。

- [ ] **Step 4: 前端数据层的失败测试**

`app/src/lib/api.test.ts`：

```ts
import { afterEach, describe, expect, test, vi } from "vitest";

afterEach(() => {
  // Also after a failed assertion: the next test must not think it runs inside Tauri.
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.doUnmock("@tauri-apps/api/core");
});

describe("backend selection", () => {
  test("outside Tauri the panel loads the mock backend", async () => {
    vi.resetModules();
    const { isTauri, loadBackend } = await import("./api");
    expect(isTauri()).toBe(false);
    const backend = await loadBackend();
    expect((await backend.status()).status.viewer?.name).toBe("Jane Doe");
  });

  test("command errors become BackendError with the backend's text, and codes are recognised", async () => {
    vi.resetModules();
    const invoke = vi
      .fn()
      .mockRejectedValueOnce("job 7 not found")
      .mockRejectedValueOnce("not_a_linear_link")
      .mockRejectedValueOnce("login_cancelled");
    vi.doMock("@tauri-apps/api/core", () => ({ invoke }));
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const { loadBackend } = await import("./api");
    const { BackendError } = await import("./backend");
    const backend = await loadBackend();
    const failed = await backend.job(7).catch((error: unknown) => error);
    expect(failed).toBeInstanceOf(BackendError);
    expect(failed).toMatchObject({ message: "job 7 not found", code: null });
    const refused = await backend.openLinear("https://evil.example/").catch((error: unknown) => error);
    expect(refused).toMatchObject({ code: "not_a_linear_link" });
    const cancelled = await backend.linearLogin().catch((error: unknown) => error);
    expect(cancelled).toMatchObject({ code: "login_cancelled" });
  });
});
```

`app/src/lib/mock.test.ts`：

```ts
import { describe, expect, test } from "vitest";
import { createMockBackend } from "./mock";
import type { EngineEvent } from "./types";

describe("mock backend", () => {
  test("the demo scenario has a job in every state", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const states = new Set((await backend.listJobs(100)).map((job) => job.state));
    for (const state of ["waiting_binding", "queued", "running", "awaiting_acceptance", "needs_info", "done", "failed", "canceled"]) {
      expect(states.has(state as never), state).toBe(true);
    }
  });

  test("marking a job done changes it and announces the change", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const events: EngineEvent[] = [];
    await backend.onEngineEvent((event) => events.push(event));
    const waiting = (await backend.listJobs(100)).find((job) => job.state === "awaiting_acceptance")!;
    await backend.markDone(waiting.id);
    expect((await backend.job(waiting.id))?.state).toBe("done");
    expect(events).toContainEqual({ kind: "job", job: expect.objectContaining({ id: waiting.id, state: "done" }) });
  });

  test("marking a job that is not awaiting acceptance fails like the engine does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const running = (await backend.listJobs(100)).find((job) => job.state === "running")!;
    await expect(backend.markDone(running.id)).rejects.toThrow(`job ${running.id} is not awaiting acceptance`);
  });

  test("only the newest run of an issue can run again, as the engine says", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await expect(backend.retryJob(2)).rejects.toThrow("job 2 is not the latest run of DEMO-12; retry that one instead");
    await backend.retryJob(8);
    expect((await backend.job(8))?.state).toBe("queued");
  });

  test("a held job says when it is tried again", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    expect((await backend.status()).status.held).toEqual([{ job_id: 11, retry_at: "2026-10-12T09:02:00Z" }]);
  });

  test("waiting issues carry a reason code, not a sentence", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const waiting = (await backend.issues()).find((view) => view.issue.identifier === "DEMO-5")!;
    expect(waiting.status).toEqual({ kind: "waiting", reason: { code: "project_not_bound" } });
    expect(waiting.job?.state).toBe("waiting_binding");
  });

  test("a binding whose folder is not absolute is refused with the engine's message", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const binding = { id: "proj-3", project_id: "proj-3", project_name: "Docs Site", directory: "code/docs" };
    await expect(backend.saveBinding(binding)).rejects.toThrow('bindings[2].directory must be an absolute path: "code/docs"');
    await backend.saveBinding({ ...binding, directory: "~/code/docs" });
    expect((await backend.bindings()).map((b) => b.id)).toEqual(["proj-1", "proj-2", "proj-3"]);
  });

  test("the fresh scenario is a first run", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    expect((await backend.status()).linear.kind).toBe("not_configured");
    expect(await backend.bindings()).toEqual([]);
    expect((await backend.settings()).onboarding_completed).toBe(false);
    await expect(backend.issues()).rejects.toThrow("not connected to Linear: log in first");
    await backend.linearLogin();
    expect((await backend.status()).linear).toEqual({ kind: "connected", method: "oauth" });
    expect((await backend.doctor()).viewer?.name).toBe("Jane Doe");
  });

  test("settings keep the pause switch, as the engine does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await backend.pause();
    const settings = await backend.settings();
    await backend.saveSettings({ ...settings, paused: false, poll_interval_secs: 60 });
    expect((await backend.settings()).paused).toBe(true);
    expect((await backend.settings()).poll_interval_secs).toBe(60);
  });

  test("the app info reports a system language, as Rust does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    expect((await backend.appInfo()).system_locale).toBe("en-US");
    expect(await backend.startupState()).toEqual({ kind: "ready" });
  });

  test("a start still connecting to Linear is ready, and connects on reconnect", async () => {
    const backend = createMockBackend("connecting", { latencyMs: 0 });
    expect(await backend.startupState()).toEqual({ kind: "ready" });
    expect((await backend.status()).linear).toEqual({ kind: "connecting" });
    await expect(backend.issues()).rejects.toThrow("still connecting to Linear");
    const events: EngineEvent[] = [];
    await backend.onEngineEvent((event) => events.push(event));
    await backend.linearReconnect();
    expect((await backend.status()).linear).toEqual({ kind: "connected", method: "oauth" });
    expect(events).toContainEqual({ kind: "status" });
  });

  test("links outside linear.app are refused, as the app refuses them", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await backend.openLinear("https://mcp.linear.app/authorize");
    for (const url of ["https://linear.app@evil.example/", "https://linear.app.evil.example/", "http://linear.app/"]) {
      await expect(backend.openLinear(url), url).rejects.toMatchObject({ code: "not_a_linear_link" });
    }
  });

  test("control servers report one state per executor", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    expect(await backend.controlServers()).toEqual({ claude: "not_registered", codex: "not_registered" });
    await backend.setupAction({ kind: "register_control_server", executor: "claude" });
    expect((await backend.controlServers()).claude).toBe("registered");
  });
});
```

`app/src/lib/clean-cases.json`（`ansi: false` 的用例同时由 Rust 测试对照 core 的 `clean_text` 核对：期望值去掉首尾空白后比较，因为 `clean_text` 会去首尾而面板不去；`ansi: true` 的是面板额外去掉的转义序列。每行的 `note` 写明它用到的码位；不可见与方向控制字符一律写成 `\uXXXX` 转义（U+E0041 写成代理对），文件里不出现原样的不可见字符，免得被编辑器悄悄吞掉后用例照样通过）：

```json
[
  { "note": "plain text stays", "input": "plain text", "expected": "plain text", "ansi": false },
  { "note": "leading spaces stay (nothing is trimmed)", "input": "  indented, not trimmed", "expected": "  indented, not trimmed", "ansi": false },
  { "note": "U+0009 tab becomes a space", "input": "tab\there", "expected": "tab here", "ansi": false },
  { "note": "U+000A line feed becomes a space", "input": "line one\nline two", "expected": "line one line two", "ansi": false },
  { "note": "U+000D carriage return becomes a space", "input": "done\rrewritten", "expected": "done rewritten", "ansi": false },
  { "note": "U+0007 bell and U+007F delete become spaces", "input": "bell\u0007 and del\u007f", "expected": "bell  and del ", "ansi": false },
  { "note": "U+0085 next line, a C1 control, becomes a space", "input": "next line\u0085here", "expected": "next line here", "ansi": false },
  { "note": "U+001B escape with no sequence after it becomes a space", "input": "lone escape\u001b", "expected": "lone escape ", "ansi": false },
  { "note": "U+200B zero width space is removed", "input": "zero\u200bwidth", "expected": "zerowidth", "ansi": false },
  { "note": "U+202E right-to-left override is removed", "input": "abc\u202edef", "expected": "abcdef", "ansi": false },
  { "note": "U+00AD soft hyphen and U+FEFF byte order mark are removed", "input": "soft\u00adhyphen and bom\ufeff", "expected": "softhyphen and bom", "ansi": false },
  { "note": "U+E0041 tag latin capital letter A, written as the surrogate pair DB40 DC41, is removed", "input": "hidden\udb40\udc41tag", "expected": "hiddentag", "ansi": false },
  { "note": "U+200D zero width joiner stays: emoji sequences need it", "input": "family 👨\u200d👩\u200d👧", "expected": "family 👨\u200d👩\u200d👧", "ansi": false },
  { "note": "other scripts and accents stay", "input": "日本語 é", "expected": "日本語 é", "ansi": false },
  { "note": "CSI colour sequences (U+001B [) are removed", "input": "\u001b[1;32mok\u001b[0m", "expected": "ok", "ansi": true },
  { "note": "a CSI that clears the screen is removed", "input": "\u001b[2Jclear", "expected": "clear", "ansi": true },
  { "note": "a one-byte CSI (U+009B) sequence is removed", "input": "\u009b31mred", "expected": "red", "ansi": true },
  { "note": "an OSC ended by BEL (U+0007) is removed", "input": "\u001b]0;title\u0007text", "expected": "text", "ansi": true },
  { "note": "OSC 8 hyperlinks ended by ST (U+001B \\) are removed", "input": "\u001b]8;;https://example.com\u001b\\link\u001b]8;;\u001b\\", "expected": "link", "ansi": true },
  { "note": "a two-character ESC sequence (ESC c) is removed", "input": "\u001bcreset", "expected": "reset", "ansi": true }
]
```

`app/src/lib/clean.test.ts`：

```ts
import { expect, test } from "vitest";
import cases from "./clean-cases.json";
import { cleanDisplayLines, cleanDisplayText } from "./clean";

// Each row names the code points it uses in `note`; the file writes them as escapes.
test.each(cases)("$note", ({ input, expected }) => {
  expect(cleanDisplayText(input)).toBe(expected);
});

test("multi-line output keeps its lines, each one cleaned", () => {
  // U+001B [0m resets a colour; U+202E (right-to-left override) is removed.
  expect(cleanDisplayLines("ok\u001b[0m\r\n\u202efailed\tonce")).toBe("ok\nfailed once");
});
```

Run: `pnpm --dir app test`
Expected: 失败（模块不存在）。

- [ ] **Step 5: 实现 `types.ts`、`backend.ts`、`api.ts`、`clean.ts`、`mock-fixtures.ts`、`mock.ts`**

`app/src/lib/types.ts`（与 Rust 的 serde 输出一一对应；字段名照 Rust）：

```ts
export type JobState =
  | "waiting_binding"
  | "queued"
  | "running"
  | "awaiting_acceptance"
  | "needs_info"
  | "done"
  | "failed"
  | "canceled";
export const JOB_STATES: JobState[] = [
  "waiting_binding",
  "queued",
  "running",
  "awaiting_acceptance",
  "needs_info",
  "done",
  "failed",
  "canceled",
];
export type ExecutorKind = "claude" | "codex";
export type PermissionProfile = "full" | "auto" | "personal";

export interface Viewer { id: string; name: string }

export interface Issue {
  uuid: string;
  identifier: string;
  title: string;
  url: string;
  state: string;
  labels: string[];
  project_id: string | null;
  project_name: string | null;
  assignee_id: string | null;
  parent_identifier: string | null;
  priority: number;
  created_at: string;
  updated_at: string;
}

export interface Job {
  id: number;
  issue_uuid: string;
  issue_identifier: string;
  title: string;
  binding_id: string | null;
  directory: string | null;
  executor: ExecutorKind | null;
  state: JobState;
  reason: string | null;
  priority: number;
  issue_created_at: string;
  discovered_updated_at: string;
  verified_updated_at: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  log_path: string | null;
  summary: string | null;
}

/** A queued job passed over after a failed start, and when it is tried again. */
export interface HeldJob { job_id: number; retry_at: string }

export interface StatusSnapshot {
  paused: boolean;
  linear_ok: boolean;
  viewer: Viewer | null;
  last_poll_at: string | null;
  last_error: string | null;
  running: number;
  queued: number;
  waiting: number;
  draining: boolean;
  held: HeldJob[];
}

export type AuthMethod = "oauth" | "api_key";
export type LinearSetup =
  | { kind: "connected"; method: AuthMethod }
  | { kind: "not_configured" }
  | { kind: "interface_changed"; message: string }
  | { kind: "connect_failed"; message: string }
  /** The first connection is still under way (the host no longer waits for it). */
  | { kind: "connecting" };
export interface StatusView { status: StatusSnapshot; linear: LinearSetup }

/** Why an eligible-looking issue waits, as a code the panel translates. */
export type WaitReason =
  | { code: "project_not_bound" }
  | { code: "directory_missing"; directory: string }
  | { code: "executor_not_ready"; executor: ExecutorKind };
export type IssueStatus =
  | { kind: "eligible" }
  | { kind: "not_todo"; state: string }
  | { kind: "not_candidate" }
  | { kind: "waiting"; reason: WaitReason }
  | { kind: "paused" };
export interface IssueView { issue: Issue; job: Job | null; status: IssueStatus }

export interface TriggerNames { label: string; todo: string; in_progress: string; in_testing: string; done: string }
export interface NotificationSettings { run_started: boolean; awaiting_acceptance: boolean; needs_info: boolean; failed: boolean }
export interface Settings {
  language: string;
  poll_interval_secs: number;
  max_concurrent: number;
  run_timeout_mins: number;
  stall_timeout_mins: number;
  log_retention_days: number;
  log_max_total_mb: number;
  default_executor: ExecutorKind;
  default_permission: PermissionProfile;
  prompt_template: string;
  /** "follow-ui" or a language code. */
  comment_language: string;
  paused: boolean;
  trigger: TriggerNames;
  notifications: NotificationSettings;
  auto_update: boolean;
  onboarding_completed: boolean;
  full_access_acknowledged: boolean;
}

export interface Binding {
  id: string;
  project_id: string;
  project_name: string;
  directory: string;
  executor?: ExecutorKind;
  permission?: PermissionProfile;
  model?: string;
  prompt_override?: string;
}
export interface Project { id: string; name: string; team_ids: string[] }

export type EnvSource = "host" | "login-shell" | "system-settings" | "none";
export interface ExecEnv {
  path: string | null;
  path_source: EnvSource;
  proxy: [string, string][];
  proxy_source: EnvSource;
  problems: string[];
}
export interface ExecutorReport {
  kind: ExecutorKind;
  program: string | null;
  version: string | null;
  logged_in: boolean;
  linear_mcp: boolean;
  problems: string[];
}
export interface WorkspaceReport { label_ok: boolean; missing_states: string[]; teams_checked: string[] }
export interface BindingReport {
  binding_id: string;
  project_name: string;
  directory_exists: boolean;
  executor: ExecutorKind;
  executor_ready: boolean;
}
export interface DoctorReport {
  viewer: Viewer | null;
  linear_error: string | null;
  executors: ExecutorReport[];
  workspace: WorkspaceReport | null;
  bindings: BindingReport[];
  env: ExecEnv | null;
}

export type SetupAction =
  | { kind: "add_linear_mcp"; executor: ExecutorKind }
  | { kind: "login_linear_mcp"; executor: ExecutorKind }
  | { kind: "register_control_server"; executor: ExecutorKind }
  | { kind: "remove_control_server"; executor: ExecutorKind };
/** One executor's control-server registration; `unknown` when the check itself failed. */
export type ControlServer = "registered" | "not_registered" | "not_installed" | "unknown";
export interface ControlServers { claude: ControlServer; codex: ControlServer }
export interface AppInfo { version: string; cli_path: string | null; platform: string; system_locale: string | null }
export type StartupCode = "already_running" | "store_unavailable" | "other";
export type StartupState =
  | { kind: "starting" }
  | { kind: "ready" }
  | { kind: "failed"; code: StartupCode; detail: string };
export type EngineEvent = { kind: "status" } | { kind: "job"; job: Job } | { kind: "config" };
export type QuitMode = "wait" | "stop";
```

（`DoctorReport.env.proxy` 在引擎里已经打过码（M2 第一部分 Task 1 修复：`Engine::doctor` 用 `proxy_for_display()`），面板拿不到原始凭据；Task 9 的 `maskCredentials` 再打一次码作为纵深防护，规则与 Rust 的 `env::mask_credentials` 一致。）

`app/src/lib/backend.ts`（叶子模块：只导入类型，`api.ts` 与 `mock.ts` 都从这里取，二者之间没有导入环）：

```ts
import type {
  AppInfo, Binding, ControlServers, DoctorReport, EngineEvent, ExecutorKind, IssueView, Job, LinearSetup,
  Project, QuitMode, Settings, SetupAction, StartupState, StatusView, Viewer,
} from "./types";

/** Errors a command answers with as a code rather than a sentence; the panel translates them (`errors.*`). */
export const ERROR_CODES = ["not_a_linear_link", "login_cancelled"] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

/** A command failed. `message` is the backend's one cleaned line; `code` is set when it is one of `ERROR_CODES`. */
export class BackendError extends Error {
  readonly code: ErrorCode | null;

  constructor(message: string) {
    super(message);
    this.name = "BackendError";
    this.code = (ERROR_CODES as readonly string[]).includes(message) ? (message as ErrorCode) : null;
  }
}

export interface Backend {
  appInfo(): Promise<AppInfo>;
  startupState(): Promise<StartupState>;
  status(): Promise<StatusView>;
  listJobs(limit: number): Promise<Job[]>;
  job(id: number): Promise<Job | null>;
  jobLog(id: number, lines: number): Promise<string[]>;
  issues(): Promise<IssueView[]>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  pollNow(): Promise<void>;
  cancelJob(id: number): Promise<void>;
  retryJob(id: number): Promise<void>;
  markDone(id: number): Promise<void>;
  bindings(): Promise<Binding[]>;
  saveBinding(binding: Binding): Promise<void>;
  removeBinding(id: string): Promise<boolean>;
  projects(): Promise<Project[]>;
  settings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<void>;
  doctor(): Promise<DoctorReport>;
  linearLogin(): Promise<Viewer>;
  linearCancelLogin(): Promise<void>;
  linearUseApiKey(key: string): Promise<Viewer>;
  linearLogout(): Promise<void>;
  linearReconnect(): Promise<LinearSetup>;
  setupAction(action: SetupAction): Promise<void>;
  controlServers(): Promise<ControlServers>;
  linearServerNamed(executor: ExecutorKind): Promise<boolean>;
  openLinear(url: string): Promise<void>;
  chooseDirectory(): Promise<string | null>;
  autostartEnabled(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<void>;
  quit(mode: QuitMode): Promise<void>;
  requestQuit(): Promise<void>;
  onEngineEvent(listener: (event: EngineEvent) => void): Promise<() => void>;
  onResync(listener: () => void): Promise<() => void>;
  onQuitRequested(listener: (running: number) => void): Promise<() => void>;
  onStartupFailed(listener: (state: StartupState) => void): Promise<() => void>;
}
```

`app/src/lib/api.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type Backend, BackendError } from "./backend";
import type { StartupState } from "./types";

export const isTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new BackendError(typeof error === "string" ? error : String(error));
  }
}

async function on<T>(event: string, listener: (payload: T) => void): Promise<() => void> {
  return listen<T>(event, (e) => listener(e.payload));
}

export const tauriBackend: Backend = {
  appInfo: () => call("app_info"),
  startupState: () => call("startup_state"),
  status: () => call("status"),
  listJobs: (limit) => call("list_jobs", { limit }),
  job: (id) => call("job", { id }),
  jobLog: (id, lines) => call("job_log", { id, lines }),
  issues: () => call("issues"),
  pause: () => call("pause"),
  resume: () => call("resume"),
  pollNow: () => call("poll_now"),
  cancelJob: (id) => call("cancel_job", { id }),
  retryJob: (id) => call("retry_job", { id }),
  markDone: (id) => call("mark_done", { id }),
  bindings: () => call("bindings"),
  saveBinding: (binding) => call("save_binding", { binding }),
  removeBinding: (id) => call("remove_binding", { id }),
  projects: () => call("projects"),
  settings: () => call("settings"),
  saveSettings: (settings) => call("save_settings", { settings }),
  doctor: () => call("doctor"),
  linearLogin: () => call("linear_login"),
  linearCancelLogin: () => call("linear_cancel_login"),
  linearUseApiKey: (key) => call("linear_use_api_key", { key }),
  linearLogout: () => call("linear_logout"),
  linearReconnect: () => call("linear_reconnect"),
  setupAction: (action) => call("setup_action", { action }),
  controlServers: () => call("control_servers"),
  linearServerNamed: (executor) => call("linear_server_named", { executor }),
  openLinear: (url) => call("open_linear", { url }),
  chooseDirectory: () => call("choose_directory"),
  autostartEnabled: () => call("autostart_enabled"),
  setAutostart: (enabled) => call("set_autostart", { enabled }),
  quit: (mode) => call("quit", { mode }),
  requestQuit: () => call("request_quit"),
  onEngineEvent: (listener) => on("engine-event", listener),
  onResync: (listener) => on("engine-resync", () => listener()),
  onQuitRequested: (listener) => on("quit-requested", listener),
  onStartupFailed: (listener) => on<StartupState>("startup-failed", listener),
};

/**
 * `?mock=fresh` in a browser starts the mock as a first run (no login, no bindings);
 * `?mock=connecting` as a start whose first connection to Linear is still under way.
 */
function mockScenario(): "demo" | "fresh" | "connecting" {
  const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  const scenario = params.get("mock");
  return scenario === "fresh" || scenario === "connecting" ? scenario : "demo";
}

/**
 * The backend the panel uses: the real commands inside Tauri; in a plain browser the mock,
 * loaded with `import()` only then, so the demo data stays out of the app's own start-up code.
 */
export async function loadBackend(): Promise<Backend> {
  if (isTauri()) return tauriBackend;
  const { createMockBackend } = await import("./mock");
  return createMockBackend(mockScenario());
}
```

`app/src/lib/clean.ts`：

```ts
// The ANSI escape sequences the panel removes: CSI (ESC [ or the one-byte U+009B, parameters,
// one final byte), OSC ended by BEL or ST (ESC \), and the two-character ESC sequences.
// eslint-disable-next-line no-control-regex -- matching terminal escape sequences is the point.
const ANSI = /(?:\u001b\[|\u009b)[0-?]*[ -/]*[@-~]|\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)|\u001b[^[\]]/g;

/** The characters core's `text::is_invisible` removes; keep in step with crates/core/src/text.rs. */
function isInvisible(code: number): boolean {
  return (
    code === 0x00ad ||
    code === 0x061c ||
    code === 0x115f ||
    code === 0x1160 ||
    code === 0x200b ||
    code === 0x200e ||
    code === 0x200f ||
    (code >= 0x2028 && code <= 0x202e) ||
    (code >= 0x2060 && code <= 0x206f) ||
    code === 0x3164 ||
    code === 0xfeff ||
    code === 0xffa0 ||
    (code >= 0xe0000 && code <= 0xe007f)
  );
}

/** Rust's `char::is_control`: the C0 controls, DEL and the C1 controls. */
function isControl(code: number): boolean {
  return code <= 0x1f || (code >= 0x7f && code <= 0x9f);
}

/**
 * Text from executors, the AI or the engine as the panel shows it: escape sequences removed,
 * every other control character turned into a space, invisible characters dropped. Unlike core's
 * `clean_text` it neither trims nor shortens, so a log line keeps its indentation. The cases in
 * clean-cases.json are checked here and, for the character rules, against core's `clean_text`
 * in commands.rs (which trims what it returns).
 */
export function cleanDisplayText(text: string): string {
  let out = "";
  for (const char of text.replace(ANSI, "")) {
    const code = char.codePointAt(0) ?? 0;
    if (isControl(code)) out += " ";
    else if (!isInvisible(code)) out += char;
  }
  return out;
}

/** `cleanDisplayText` for each line of a multi-line text (command output, tool results). */
export function cleanDisplayLines(text: string): string {
  return text.split(/\r?\n/).map(cleanDisplayText).join("\n");
}
```

`app/src/lib/mock-fixtures.ts`（假后端的全部虚构数据；后续测试用到的确切字符串都在这里）：

```ts
import { MOCK_NOW } from "./clock";
import type { AppInfo, Binding, DoctorReport, HeldJob, Issue, IssueStatus, Job, Project, Settings, Viewer } from "./types";

/** `MOCK_NOW` minus `minutes`, as the engine sends times. */
export function ago(minutes: number): string {
  return new Date(MOCK_NOW.getTime() - minutes * 60_000).toISOString();
}

export const DEMO_VIEWER: Viewer = { id: "user-1", name: "Jane Doe" };

export const DEMO_APP_INFO: AppInfo = {
  version: "0.1.0",
  cli_path: "/Applications/IssueRelay.app/Contents/MacOS/issuerelay",
  platform: "macos",
  system_locale: "en-US",
};

export const DEMO_PROJECTS: Project[] = [
  { id: "proj-1", name: "Demo Project", team_ids: ["team-1"] },
  { id: "proj-2", name: "Demo Mobile", team_ids: ["team-1"] },
  { id: "proj-3", name: "Docs Site", team_ids: ["team-2"] },
];

export const DEMO_BINDINGS: Binding[] = [
  { id: "proj-1", project_id: "proj-1", project_name: "Demo Project", directory: "~/code/demo-app", permission: "full" },
  {
    id: "proj-2",
    project_id: "proj-2",
    project_name: "Demo Mobile",
    directory: "~/code/demo-mobile",
    executor: "codex",
    permission: "auto",
    model: "gpt-5-codex",
  },
];

/** `config::Settings::default()`. */
export const DEFAULT_SETTINGS: Settings = {
  language: "system",
  poll_interval_secs: 30,
  max_concurrent: 2,
  run_timeout_mins: 60,
  stall_timeout_mins: 15,
  log_retention_days: 30,
  log_max_total_mb: 1024,
  default_executor: "claude",
  default_permission: "full",
  prompt_template: "",
  comment_language: "follow-ui",
  paused: false,
  trigger: { label: "ai-ready", todo: "Todo", in_progress: "In Progress", in_testing: "In Testing", done: "Done" },
  notifications: { run_started: true, awaiting_acceptance: true, needs_info: true, failed: true },
  auto_update: true,
  onboarding_completed: false,
  full_access_acknowledged: false,
};

/** A demo job: what every demo job shares, then `fields`. */
function job(fields: Pick<Job, "id" | "issue_identifier" | "title" | "state"> & Partial<Job>): Job {
  return {
    issue_uuid: `issue-${fields.issue_identifier.replace("DEMO-", "")}`,
    binding_id: "proj-1",
    directory: "~/code/demo-app",
    executor: "claude",
    reason: null,
    priority: 2,
    issue_created_at: ago(24 * 60),
    discovered_updated_at: ago(12 * 60),
    verified_updated_at: null,
    created_at: ago(12 * 60),
    started_at: null,
    finished_at: null,
    log_path: null,
    summary: null,
    ...fields,
  };
}

/**
 * One job in every state, newest first, plus an older failed run of DEMO-12 (id 2): only the
 * newest run of an issue may run again, and an old failure does not raise the overview's alert.
 */
export const DEMO_JOBS: Job[] = [
  job({ id: 12, issue_identifier: "DEMO-12", title: "Add a health check endpoint", state: "running", started_at: "2026-10-12T08:57:37Z", log_path: "logs/12.jsonl" }),
  job({ id: 11, issue_identifier: "DEMO-11", title: "Translate the settings page", state: "queued", binding_id: "proj-2", directory: "~/code/demo-mobile", executor: "codex" }),
  job({ id: 10, issue_identifier: "DEMO-10", title: "Add hello.txt", state: "awaiting_acceptance", started_at: ago(50), finished_at: ago(20), verified_updated_at: ago(20), log_path: "logs/10.jsonl", summary: "新增 hello.txt，已推送到 test 分支并读回部署" }),
  job({ id: 9, issue_identifier: "DEMO-9", title: "Clarify the export format", state: "needs_info", started_at: ago(70), finished_at: ago(60), summary: "Asked on the issue which date format the export should use" }),
  job({ id: 8, issue_identifier: "DEMO-8", title: "Speed up the search index", state: "failed", reason: "run exceeded the time limit", started_at: ago(130), finished_at: ago(70) }),
  job({ id: 7, issue_identifier: "DEMO-7", title: "Fix the login redirect", state: "done", started_at: ago(240), finished_at: ago(200), summary: "Fixed the redirect after login and added a test" }),
  job({ id: 6, issue_identifier: "DEMO-6", title: "Remove the old banner", state: "canceled", reason: "stopped by the user", started_at: ago(300), finished_at: ago(290) }),
  job({ id: 5, issue_identifier: "DEMO-5", title: "Write the docs landing page", state: "waiting_binding", reason: "project is not bound to a local directory", binding_id: null, directory: null, executor: null }),
  job({ id: 2, issue_identifier: "DEMO-12", title: "Add a health check endpoint", state: "failed", reason: "executor exited with code 1", started_at: ago(2 * 24 * 60), finished_at: ago(2 * 24 * 60 - 5) }),
];

/** DEMO-11's start failed once; it is tried again two minutes after `MOCK_NOW`. */
export const DEMO_HELD: HeldJob[] = [{ job_id: 11, retry_at: "2026-10-12T09:02:00Z" }];

function issue(n: number, title: string, state: string, project: Project): Issue {
  return {
    uuid: `issue-${n}`,
    identifier: `DEMO-${n}`,
    title,
    url: `https://linear.app/demo/issue/DEMO-${n}`,
    state,
    labels: ["ai-ready"],
    project_id: project.id,
    project_name: project.name,
    assignee_id: "user-1",
    parent_identifier: null,
    priority: 2,
    created_at: ago(24 * 60),
    updated_at: ago(30),
  };
}

const [demoProject, demoMobile, docsSite] = DEMO_PROJECTS;

/** The issue list (the issues with the label; DEMO-9 lost it when the AI asked back) and the engine's verdicts. */
export const DEMO_ISSUES: { issue: Issue; status: IssueStatus }[] = [
  { issue: issue(12, "Add a health check endpoint", "In Progress", demoProject), status: { kind: "not_todo", state: "In Progress" } },
  { issue: issue(11, "Translate the settings page", "Todo", demoMobile), status: { kind: "eligible" } },
  { issue: issue(10, "Add hello.txt", "In Testing", demoProject), status: { kind: "not_todo", state: "In Testing" } },
  { issue: issue(8, "Speed up the search index", "In Progress", demoProject), status: { kind: "not_todo", state: "In Progress" } },
  { issue: issue(7, "Fix the login redirect", "Done", demoProject), status: { kind: "not_todo", state: "Done" } },
  { issue: issue(6, "Remove the old banner", "Todo", demoProject), status: { kind: "eligible" } },
  { issue: issue(5, "Write the docs landing page", "Todo", docsSite), status: { kind: "waiting", reason: { code: "project_not_bound" } } },
  { issue: issue(4, "Migrate the build to the new runner", "Backlog", demoProject), status: { kind: "not_todo", state: "Backlog" } },
  { issue: issue(3, "Update the README badges", "Todo", demoProject), status: { kind: "eligible" } },
];

export const DEMO_DOCTOR: DoctorReport = {
  viewer: DEMO_VIEWER,
  linear_error: null,
  executors: [
    { kind: "claude", program: "~/.local/bin/claude", version: "2.1.285 (Claude Code)", logged_in: true, linear_mcp: true, problems: [] },
    { kind: "codex", program: "~/.local/bin/codex", version: "codex-cli 0.159.2", logged_in: false, linear_mcp: true, problems: ["codex is not logged in"] },
  ],
  workspace: { label_ok: true, missing_states: [], teams_checked: ["Demo"] },
  bindings: [
    { binding_id: "proj-1", project_name: "Demo Project", directory_exists: true, executor: "claude", executor_ready: true },
    { binding_id: "proj-2", project_name: "Demo Mobile", directory_exists: true, executor: "codex", executor_ready: false },
  ],
  env: {
    path: "~/.local/bin:/opt/homebrew/bin:/usr/bin:/bin",
    path_source: "login-shell",
    proxy: [["HTTPS_PROXY", "http://127.0.0.1:7890"]],
    proxy_source: "system-settings",
    problems: [],
  },
};

/** Before the first login: no user, no workspace check, no bindings. */
export const FRESH_DOCTOR: DoctorReport = {
  ...DEMO_DOCTOR,
  viewer: null,
  linear_error: "not connected to Linear: log in first",
  workspace: null,
  bindings: [],
};

const CLAUDE_SESSION = [
  JSON.stringify({ type: "system", subtype: "init", model: "claude-sonnet-5-5", session_id: "demo-session" }),
  JSON.stringify({ type: "assistant", message: { content: [{ type: "text", text: "I will add the endpoint and a test for it." }] } }),
  JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "tool-1", name: "Bash", input: { command: "cargo test" } }] } }),
  JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "tool-1", content: "test result: ok. 12 passed", is_error: false }] } }),
];

/** Run logs by job id; the other jobs' logs are gone (removed by the cleanup) and read as empty. */
export const DEMO_LOGS: Record<number, string[]> = {
  12: [...CLAUDE_SESSION, JSON.stringify({ stream: "stderr", text: "warning: unused import" })],
  10: [...CLAUDE_SESSION, JSON.stringify({ type: "result", result: "Added hello.txt and pushed it to the test branch.", is_error: false })],
};
```

`app/src/lib/mock.ts`：

```ts
import { type Backend, BackendError } from "./backend";
import { MOCK_NOW, now, setFixedClock } from "./clock";
import {
  DEFAULT_SETTINGS, DEMO_APP_INFO, DEMO_BINDINGS, DEMO_DOCTOR, DEMO_HELD, DEMO_ISSUES, DEMO_JOBS, DEMO_LOGS,
  DEMO_PROJECTS, DEMO_VIEWER, FRESH_DOCTOR, ago,
} from "./mock-fixtures";
import type {
  Binding, ControlServers, DoctorReport, EngineEvent, HeldJob, IssueView, Job, LinearSetup, Settings,
  StartupState, StatusView, Viewer,
} from "./types";

export interface MockOptions {
  /** The delay of every call: 120 ms by default so loading states show in a browser; tests pass 0. */
  latencyMs?: number;
}

const NOT_CONNECTED = "not connected to Linear: log in first";
const CONNECTING = "still connecting to Linear";

/** `https` on linear.app or one of its subdomains: the rule of `is_linear_url` in commands.rs. */
function isLinearUrl(text: string): boolean {
  try {
    const url = new URL(text);
    return url.protocol === "https:" && (url.hostname === "linear.app" || url.hostname.endsWith(".linear.app"));
  } catch {
    return false;
  }
}

/**
 * The panel's backend without Tauri: fictional data in memory that behaves like the engine as far
 * as the panel can see — the same rules and messages for marking done, running again, stopping,
 * saving settings and bindings, and the same events. Development, component tests and screenshots
 * use it; it never touches real data.
 */
export function createMockBackend(scenario: "demo" | "fresh" | "connecting", options: MockOptions = {}): Backend {
  const latencyMs = options.latencyMs ?? 120;
  // Relative times and run durations in tests and screenshots are computed from this moment.
  setFixedClock(MOCK_NOW);
  // "connecting" is the demo install starting up: its local data, Linear not connected yet.
  const demo = scenario !== "fresh";
  let jobs: Job[] = demo ? structuredClone(DEMO_JOBS) : [];
  let held: HeldJob[] = demo ? structuredClone(DEMO_HELD) : [];
  let bindings: Binding[] = demo ? structuredClone(DEMO_BINDINGS) : [];
  let settings: Settings = { ...DEFAULT_SETTINGS, onboarding_completed: demo, full_access_acknowledged: demo };
  let linear: LinearSetup =
    scenario === "demo" ? { kind: "connected", method: "oauth" } : scenario === "connecting" ? { kind: "connecting" } : { kind: "not_configured" };
  let viewer: Viewer | null = scenario === "demo" ? DEMO_VIEWER : null;
  let doctor: DoctorReport = structuredClone(
    scenario === "demo" ? DEMO_DOCTOR : scenario === "connecting" ? { ...DEMO_DOCTOR, viewer: null, linear_error: CONNECTING, workspace: null } : FRESH_DOCTOR,
  );
  const controlServers: ControlServers = { claude: "not_registered", codex: "not_registered" };
  let autostart = false;
  const listeners = {
    event: new Set<(event: EngineEvent) => void>(),
    resync: new Set<() => void>(),
    quit: new Set<(running: number) => void>(),
    startup: new Set<(state: StartupState) => void>(),
  };

  const wait = () => new Promise<void>((resolve) => setTimeout(resolve, latencyMs));
  /** Answers after the delay, reading the state only then. */
  const reply = async <T>(value: () => T): Promise<T> => {
    await wait();
    return value();
  };
  const emit = (event: EngineEvent) => listeners.event.forEach((listener) => listener(event));
  const fail = (message: string): never => {
    throw new BackendError(message);
  };
  const find = (id: number): Job => jobs.find((job) => job.id === id) ?? fail(`job ${id} not found`);
  const connected = () => linear.kind === "connected";
  const connecting = () => linear.kind === "connecting";
  /** What a call that needs Linear fails with while it is not connected. */
  const offline = () => fail(connecting() ? CONNECTING : NOT_CONNECTED);
  const update = (id: number, change: Partial<Job>) => {
    jobs = jobs.map((job) => (job.id === id ? { ...job, ...change } : job));
    emit({ kind: "job", job: find(id) });
    emit({ kind: "status" });
  };
  const subscribe = async <T>(set: Set<T>, listener: T): Promise<() => void> => {
    set.add(listener);
    return () => {
      set.delete(listener);
    };
  };
  const status = (): StatusView => ({
    status: {
      paused: settings.paused,
      linear_ok: connected(),
      viewer,
      // While connecting no check has finished yet.
      last_poll_at: connecting() ? null : ago(0.5),
      last_error: connected() || connecting() ? null : NOT_CONNECTED,
      running: jobs.filter((job) => job.state === "running").length,
      queued: jobs.filter((job) => job.state === "queued").length,
      waiting: jobs.filter((job) => job.state === "waiting_binding").length,
      draining: false,
      held: held.filter((pause) => jobs.some((job) => job.id === pause.job_id && job.state === "queued")),
    },
    linear,
  });
  const newestRun = (issueUuid: string): Job | undefined =>
    jobs.filter((job) => job.issue_uuid === issueUuid).sort((a, b) => b.id - a.id)[0];
  const issueViews = (): IssueView[] =>
    (demo ? DEMO_ISSUES : []).map(({ issue, status }) => ({ issue, job: newestRun(issue.uuid) ?? null, status }));
  const connect = (method: "oauth" | "api_key"): Viewer => {
    linear = { kind: "connected", method };
    viewer = DEMO_VIEWER;
    doctor = { ...doctor, viewer: DEMO_VIEWER, linear_error: null, workspace: DEMO_DOCTOR.workspace };
    emit({ kind: "status" });
    return DEMO_VIEWER;
  };

  return {
    appInfo: () => reply(() => DEMO_APP_INFO),
    startupState: () => reply((): StartupState => ({ kind: "ready" })),
    status: () => reply(status),
    listJobs: (limit) => reply(() => [...jobs].sort((a, b) => b.id - a.id).slice(0, Math.min(Math.max(limit, 1), 500))),
    job: (id) => reply(() => jobs.find((job) => job.id === id) ?? null),
    jobLog: (id, lines) =>
      reply(() => {
        find(id);
        return (DEMO_LOGS[id] ?? []).slice(-lines);
      }),
    issues: () => reply(() => (connected() ? issueViews() : offline())),
    pause: () =>
      reply(() => {
        settings = { ...settings, paused: true };
        emit({ kind: "status" });
      }),
    resume: () =>
      reply(() => {
        settings = { ...settings, paused: false };
        emit({ kind: "status" });
      }),
    pollNow: () => reply(() => emit({ kind: "status" })),
    cancelJob: (id) =>
      reply(() => {
        const job = find(id);
        if (!["queued", "waiting_binding", "running"].includes(job.state)) {
          fail(`job ${id} is ${job.state} and cannot be stopped`);
        }
        // A running job's process tree ends and the run is recorded at once here.
        update(id, { state: "canceled", reason: "stopped by the user", finished_at: now().toISOString() });
      }),
    retryJob: (id) =>
      reply(() => {
        const job = find(id);
        if (!["failed", "canceled", "needs_info"].includes(job.state)) {
          fail(`job ${id} is ${job.state}; only failed, stopped or needs-info runs can be retried`);
        }
        if (newestRun(job.issue_uuid)?.id !== id) {
          fail(`job ${id} is not the latest run of ${job.issue_identifier}; retry that one instead`);
        }
        held = held.filter((pause) => pause.job_id !== id);
        update(id, { state: "queued", reason: null, started_at: null, finished_at: null });
      }),
    markDone: (id) =>
      reply(() => {
        if (find(id).state !== "awaiting_acceptance") fail(`job ${id} is not awaiting acceptance`);
        update(id, { state: "done" });
      }),
    bindings: () => reply(() => structuredClone(bindings)),
    saveBinding: (binding) =>
      reply(() => {
        const others = bindings.filter((b) => b.project_id !== binding.project_id && b.id !== binding.id);
        // The engine refuses a folder that is not absolute; the demo's fictional folders start with ~/.
        if (!binding.directory.startsWith("/") && !binding.directory.startsWith("~/")) {
          fail(`bindings[${others.length}].directory must be an absolute path: ${JSON.stringify(binding.directory)}`);
        }
        bindings = [...others, binding];
        emit({ kind: "config" });
      }),
    removeBinding: (id) =>
      reply(() => {
        const before = bindings.length;
        bindings = bindings.filter((b) => b.id !== id);
        emit({ kind: "config" });
        return bindings.length !== before;
      }),
    projects: () => reply(() => (connected() ? DEMO_PROJECTS : offline())),
    settings: () => reply(() => structuredClone(settings)),
    saveSettings: (next) =>
      reply(() => {
        // Monitoring is switched with pause and resume, never by saving the settings page.
        settings = { ...next, paused: settings.paused };
        emit({ kind: "config" });
      }),
    doctor: () => reply(() => structuredClone(doctor)),
    linearLogin: () => reply(() => connect("oauth")),
    // The fake's logins answer at once, so nothing is ever waiting to be cancelled; tests that
    // need a waiting login replace `linearLogin` and `linearCancelLogin`.
    linearCancelLogin: () => reply(() => undefined),
    linearUseApiKey: (key) => reply(() => (key === "lin_api_bad" ? fail("Linear did not accept the API key") : connect("api_key"))),
    linearLogout: () =>
      reply(() => {
        linear = { kind: "not_configured" };
        viewer = null;
        emit({ kind: "status" });
      }),
    // A start still connecting gets its connection here; otherwise the setup stays as it is.
    linearReconnect: () =>
      reply(() => {
        if (connecting()) connect("oauth");
        return linear;
      }),
    setupAction: (action) =>
      reply(() => {
        doctor = {
          ...doctor,
          executors: doctor.executors.map((report) =>
            report.kind === action.executor && (action.kind === "add_linear_mcp" || action.kind === "login_linear_mcp")
              ? { ...report, linear_mcp: true, problems: report.problems.filter((p) => !p.startsWith("Linear MCP")) }
              : report,
          ),
        };
        if (action.kind === "register_control_server") controlServers[action.executor] = "registered";
        if (action.kind === "remove_control_server") controlServers[action.executor] = "not_registered";
      }),
    controlServers: () => reply(() => ({ ...controlServers })),
    // The demo registered Linear under the names the one-click login uses.
    linearServerNamed: () => reply(() => true),
    openLinear: (url) =>
      reply(() => {
        if (!isLinearUrl(url)) fail("not_a_linear_link");
      }),
    chooseDirectory: () => reply(() => "~/code/new-project"),
    autostartEnabled: () => reply(() => autostart),
    setAutostart: (enabled) =>
      reply(() => {
        autostart = enabled;
      }),
    quit: () => reply(() => undefined),
    requestQuit: () => reply(() => undefined),
    onEngineEvent: (listener) => subscribe(listeners.event, listener),
    onResync: (listener) => subscribe(listeners.resync, listener),
    onQuitRequested: (listener) => subscribe(listeners.quit, listener),
    onStartupFailed: (listener) => subscribe(listeners.startup, listener),
  };
}
```

（控制服务两个执行者都是 `not_registered`（已安装、未注册），与环境检查里两者都已安装一致。`listeners.resync`、`quit`、`startup` 只在测试替换订阅时用到，假后端自己不发这三种事件。）

Run: `pnpm --dir app test` 与 `cargo test -p issuerelay-app`
Expected: PASS。

- [ ] **Step 6: 文档与提交**

- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 面板命令（含仅面板可用的标记完成、重试、停止） | app/src-tauri/src/commands.rs |` 与 `| 前端数据层、假后端与虚构数据 | app/src/lib/ |`；第 7 节「外部接口变更规则」补一句：Rust 侧给面板的类型变了（含 `StatusSnapshot`、`IssueStatus`、`StartupState`），同步改 `app/src/lib/types.ts`、`mock-fixtures.ts` 和假后端。
- `docs/ai-coding/TESTING_SPEC.md` 的「假实现清单」登记「面板的假后端 `createMockBackend`（`app/src/lib/mock.ts`，数据在 `mock-fixtures.ts`）：组件测试、浏览器开发与截图共用，规则与报错文本照引擎」。
- 设计第 9.2 节补：「需求页按原因代码显示等待原因；排队中因启动失败暂缓的任务显示下次重试的时间。」

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs Cargo.lock app/pnpm-lock.yaml
git commit -m "feat(app): add the panel commands and the typed front-end backend with a mock"
```

---

### Task 5: 面板外框、设计变量与数据钩子

设计第 9.2、9.3 节。这一任务定下所有页面共用的东西：外框（侧边导航、页头）、设计变量（浅色 / 深色、布局常量、状态颜色）、状态标识、空 / 加载 / 错误三种状态、失败的显示方式（已翻译的标题＋清洗过的细节行）、数据查询与引擎事件桥、首次运行跳转、启动失败页、退出提示。之后的页面任务只在这个框架里加页面。

**Files:**
- Create: `app/src/styles.css`（替换模板的 `index.css`）、`app/src/lib/theme.ts`、`app/src/lib/format.ts`、`app/src/lib/errors.ts`、`app/src/lib/problem.ts`、`app/src/lib/backend-context.tsx`
- Create: `app/src/components/StatusBadge.tsx`、`app/src/components/states.tsx`、`app/src/components/PageHeader.tsx`、`app/src/components/layout/AppShell.tsx`、`app/src/components/QuitDialog.tsx`、`app/src/components/StartupError.tsx`、`app/src/components/StartupGate.tsx`、`app/src/components/FirstRunGate.tsx`、`app/src/components/EngineEventsBridge.tsx`
- Create: `app/src/hooks/queries.ts`、`app/src/hooks/useEngineEvents.ts`、`app/src/hooks/subscription.ts`
- Create: `app/src/pages/{Overview,Issues,Runs,RunDetail,Bindings,Doctor,Settings,Help}.tsx`（本任务只放页头占位，后续任务填内容）、`app/src/pages/onboarding/Onboarding.tsx`（占位）
- Create: `app/src/test/render.tsx`（测试辅助）
- Modify: `app/src/App.tsx`、`app/src/main.tsx`（样式改为导入 `./styles.css`、先载入后端再初始化语言）、`app/components.json`（样式文件路径改为 `src/styles.css`）、五个翻译文件（建立后续页面共用的 `shell`、`format`、`executor`、`permission`、`actions.failed`、`errors` 命名空间，补 `state`、`common`）
- Replace: `app/src/App.test.tsx`（Task 1 的标题测试换成新外框在假后端下的测试）
- Test: `app/src/App.test.tsx`、`app/src/components/*.test.tsx`、`app/src/components/layout/*.test.tsx`、`app/src/hooks/*.test.tsx`、`app/src/lib/{format,problem}.test.ts`
- Modify: `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 9.3 节）

**Interfaces:**
- Produces:
  - `BackendProvider({ backend, children })`、`useBackend(): Backend`（没有 `BackendProvider` 时抛错）
  - 查询键常量 `queryKeys`：`startup`、`status`、`jobs`、`job(id)`、`log(id)`、`issues`、`bindings`、`projects`、`settings`、`doctor`、`controlServers`、`linearServerNamed(executor)`、`autostart`、`appInfo`
  - 查询钩子：`useStartupState()`（启动中每秒再问一次）、`useStatus()`、`useJobs(limit = 200)`、`useJob(id)`、`useJobLog(id, { live })`（运行中每 1.5 秒刷新）、`useIssues()`、`useBindings()`、`useProjects(enabled)`、`useSettings()`、`useDoctor(enabled)`、`useControlServers()`、`useLinearServerNamed(executor, enabled)`、`useAutostart()`、`useAppInfo()`
  - 变更钩子（成功后按需失效相关查询；失败时 sonner 提示：标题是该操作的 `actions.failed.*` 译文，说明是 `describeError` 给出的细节）：`usePause`、`useResume`、`usePollNow`、`useCancelJob`、`useRetryJob`、`useMarkDone`、`useSaveBinding`、`useRemoveBinding`、`useSaveSettings`、`useLinearLogin`、`useLinearApiKey`、`useLinearLogout`、`useLinearReconnect`、`useSetupAction`、`useSetAutostart`。`useLinearLogin` 的 `mutationKey` 为 `["linear_login"]`，返回值另带 `cancel()`（调 `linearCancelLogin()`，Task 4）；它以 `login_cancelled` 失败时是用户自己取消的，不提示。`useLinearLoginWaiting(): boolean`（`useIsMutating({ mutationKey: ["linear_login"] }) > 0`）：浏览器登录正在等待，凡是提供「改用 API key」的地方据此让它不可用（core 一次只做一个登录改动）
  - `useEngineEvents()`：`status` → 失效 `status`（`StatusView` 里带着 Linear 的连接状态 `linear`：宿主在首次连接结束、重新连接、登录或登出时用 `Engine::set_linear` 换连接，它发出 `Status`，所以面板不用刷新页面就能看到 `connecting` 变成 `connected` 或 `connect_failed`，AR51）；`job` → 失效 `status`、`jobs`、`job(id)`、`issues`；`config` → 失效 `settings`、`bindings`、`status`；`resync` → 失效全部。由 `<EngineEventsBridge/>` 挂在 `QueryClientProvider` 之内。
  - `subscription(start: Promise<() => void>): () => void`（在 `useEffect` 里订阅后端事件，卸载早于订阅完成时也会退订）
  - `StatusBadge({ state })`、`STATE_STYLE: Record<JobState, { icon, className }>`
  - `EmptyState({ icon, title, description, action? })`、`LoadingState({ rows? })`、`ErrorState({ error, headline?, onRetry? })`（`headline` 是翻译键，默认 `common.error_title`）、`DetailLine({ text })`（清洗过的、次要样式的一行原文）
  - `describeError(error: unknown, t: TFunction, headlineKey: string): { headline: string; detail: string | null }`
  - `trayState(status, linear): "idle" | "running" | "paused" | "problem"`、`problemCause(status, linear): ProblemCause | null`（`"not_configured" | "interface_changed" | "connect_failed" | "check_failed" | "start_failed"`），与托盘同一规则、同一张用例表
  - `PageHeader({ title, description?, actions? })`
  - `formatRelative(iso, lang)`、`formatDateTime(iso, lang)`、`formatTime(iso, lang)`（时:分，给「几点重试」用）、`formatDuration(fromIso, toIso | null, lang)`；相对时间与时长用本项目自己的翻译键（i18next 复数），不用 `Intl.RelativeTimeFormat`
  - `StartupError({ code, detail })`、`StartupGate({ children })`、`FirstRunGate()`、`QuitDialog()`、`EngineEventsBridge()`、`App({ backend })`
  - `renderWithProviders(ui, { backend?, route?, lang? })`（测试用；默认 `createMockBackend("demo", { latencyMs: 0 })`）
  - 翻译命名空间：`shell`、`format`、`executor`（`claude`、`codex`、`default`）、`permission`（`full`、`auto`、`personal`、`default`）、`actions.failed.*`、`errors.*`、`state.retrying_at`（Task 6–10 直接用，不再各自新建）
- Consumes: Task 1 的 `now()`、`setFixedClock`；Task 2 的 i18n；Task 3 的 `app/src/lib/problem-cases.json`；Task 4 的 `Backend`、`BackendError`、`loadBackend`、`createMockBackend`（含 `connecting` 场景）、`types`（`LinearSetup` 含 `connecting`、`ControlServer`）、`cleanDisplayText`

- [ ] **Step 1: 设计变量**

`app/src/styles.css`：保留 shadcn 初始化生成的颜色变量结构（浅色 `:root`、深色 `.dark`），并做以下约定：

- 主色（`--primary`）用靛蓝：浅色 `oklch(0.51 0.21 275)`，深色 `oklch(0.68 0.17 275)`；其余中性色用 shadcn 的 neutral。
- 字体：`--font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", "PingFang SC", "Hiragino Sans", "Microsoft YaHei UI", "Malgun Gothic", sans-serif;`，`--font-mono: "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;`；基础字号 14px，行高 1.5。
- 圆角 `--radius: 0.625rem`；间距只用 Tailwind 的 4px 刻度。
- 布局常量（页面规格里只引用变量，不写像素）：`--sidebar-width: 232px`、`--content-max-width: 1080px`、`--page-padding: 1.5rem`。
- 八种任务状态各一个变量（浅色 / 深色两套），并经 `@theme inline` 暴露为 `text-state-*`、`border-state-*/30`、`bg-state-*/10` 等工具类（`StatusBadge` 用的就是这三种）：

| 状态 | 变量 | 浅色 | 深色 | 图标（lucide） |
|---|---|---|---|---|
| 等待绑定 `waiting_binding` | `--state-waiting` | `oklch(0.70 0.15 75)` | `oklch(0.80 0.14 80)` | `Unplug` |
| 排队中 `queued` | `--state-queued` | `oklch(0.55 0.03 260)` | `oklch(0.72 0.03 260)` | `Clock` |
| 运行中 `running` | `--state-running` | `oklch(0.58 0.19 255)` | `oklch(0.72 0.15 250)` | `LoaderCircle`（旋转） |
| 待验收 `awaiting_acceptance` | `--state-awaiting` | `oklch(0.55 0.20 295)` | `oklch(0.72 0.16 295)` | `ClipboardCheck` |
| 待补充信息 `needs_info` | `--state-needs-info` | `oklch(0.66 0.17 50)` | `oklch(0.78 0.14 55)` | `MessageCircleQuestion` |
| 已完成 `done` | `--state-done` | `oklch(0.60 0.15 150)` | `oklch(0.75 0.14 150)` | `CircleCheck` |
| 失败 `failed` | `--state-failed` | `oklch(0.58 0.22 25)` | `oklch(0.72 0.18 25)` | `CircleX` |
| 已取消 `canceled` | `--state-canceled` | `oklch(0.58 0.01 260)` | `oklch(0.70 0.01 260)` | `CircleSlash` |

（失败、出错的提示与「标红」一律用 `--state-failed` 或 shadcn 的 `destructive`，不另写颜色。）

`app/src/lib/theme.ts`：`useSystemTheme()` 读 `matchMedia("(prefers-color-scheme: dark)")`，给 `<html>` 加或去 `dark` 类，并监听变化。窗口背景用 `bg-background`，侧栏用 `bg-sidebar`（shadcn 变量），内容区卡片用 `bg-card`。

- [ ] **Step 2: 失败测试**

`app/src/test/render.tsx`：

```tsx
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { MemoryRouter } from "react-router";
import { Toaster } from "@/components/ui/sonner";
import { initI18nOnce, type Lang } from "@/i18n";
import type { Backend } from "@/lib/backend";
import { BackendProvider } from "@/lib/backend-context";
import { createMockBackend } from "@/lib/mock";

export async function renderWithProviders(
  ui: ReactElement,
  { backend = createMockBackend("demo", { latencyMs: 0 }), route = "/", lang = "en" as Lang } = {},
) {
  // i18next was started by the test setup (with the missing-key guard); this only switches
  // the language.
  await initI18nOnce(lang);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const result = render(
    <BackendProvider backend={backend}>
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[route]}>{ui}</MemoryRouter>
        <Toaster />
      </QueryClientProvider>
    </BackendProvider>,
  );
  return { ...result, backend, client };
}
```

`app/src/components/StatusBadge.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import { JOB_STATES } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { StatusBadge, STATE_STYLE } from "./StatusBadge";

test("every state has its own label, colour and icon", async () => {
  await renderWithProviders(<>{JOB_STATES.map((s) => <StatusBadge key={s} state={s} />)}</>, { lang: "zh-CN" });
  for (const label of ["等待绑定", "排队中", "运行中", "待验收", "待补充信息", "已完成", "失败", "已取消"]) {
    expect(screen.getByText(label)).toBeInTheDocument();
  }
  const classes = new Set(JOB_STATES.map((s) => STATE_STYLE[s].className));
  const icons = new Set(JOB_STATES.map((s) => STATE_STYLE[s].icon));
  expect(classes.size).toBe(JOB_STATES.length);
  expect(icons.size).toBe(JOB_STATES.length);
});
```

`app/src/hooks/useEngineEvents.test.tsx`：

```tsx
import { waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { createMockBackend } from "@/lib/mock";
import type { EngineEvent } from "@/lib/types";
import { renderWithProviders } from "@/test/render";

test("engine events refresh exactly the data they touch", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let emit: (event: EngineEvent) => void = () => {};
  let resync: () => void = () => {};
  const listen = vi.fn(async (listener: (event: EngineEvent) => void) => {
    emit = listener;
    return () => {};
  });
  backend.onEngineEvent = listen;
  backend.onResync = async (listener) => {
    resync = listener;
    return () => {};
  };
  const { client } = await renderWithProviders(<EngineEventsBridge />, { backend });
  const spy = vi.spyOn(client, "invalidateQueries");
  // Wait until the bridge has really subscribed (`emit` exists from the start, so waiting on it
  // would prove nothing).
  await waitFor(() => expect(listen).toHaveBeenCalledWith(expect.any(Function)));
  const job = (await backend.listJobs(1))[0];
  emit({ kind: "job", job });
  const keys = spy.mock.calls.map(([filters]) => JSON.stringify(filters?.queryKey));
  for (const key of [["status"], ["jobs"], ["job", job.id], ["issues"]]) {
    expect(keys).toContain(JSON.stringify(key));
  }
  spy.mockClear();
  emit({ kind: "config" });
  expect(spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey))).toEqual(
    expect.arrayContaining([JSON.stringify(["settings"]), JSON.stringify(["bindings"])]),
  );
  spy.mockClear();
  // A Status event re-reads the status, which carries the Linear setup: "connecting" turns into
  // "connected" or "connect_failed" without a reload.
  emit({ kind: "status" });
  expect(spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey))).toContain(JSON.stringify(["status"]));
  spy.mockClear();
  resync();
  expect(spy).toHaveBeenCalledWith();
});
```

（假后端的 `onEngineEvent` 只收监听函数，不带事件名；桥接走 `Backend` 接口，所以测试等的是这次订阅调用。）

`app/src/components/QuitDialog.test.tsx`：

```tsx
import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { QuitDialog } from "./QuitDialog";

test("quitting with runs in progress asks, and waiting can still become stopping", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (running: number) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn().mockResolvedValue(undefined);
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask(2));
  expect(screen.getByText(/2 run\(s\) are in progress/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(backend.quit).toHaveBeenCalledWith("wait");
  expect(screen.getByText(/Waiting for the runs to finish/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenLastCalledWith("stop");
});

test("keep running closes the prompt without quitting", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (running: number) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn();
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask(1));
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  expect(backend.quit).not.toHaveBeenCalled();
  expect(screen.queryByText(/run\(s\) are in progress/)).not.toBeInTheDocument();
});
```

`app/src/components/FirstRunGate.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import { Route, Routes } from "react-router";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { FirstRunGate } from "./FirstRunGate";

const routes = (
  <Routes>
    <Route element={<FirstRunGate />}>
      <Route path="/" element={<p>overview page</p>} />
    </Route>
    <Route path="/onboarding" element={<p>guide page</p>} />
  </Routes>
);

test("a first run goes to the guide", async () => {
  await renderWithProviders(routes, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("guide page")).toBeInTheDocument();
});

test("a configured install shows the page it was asked for", async () => {
  await renderWithProviders(routes);
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});
```

`app/src/components/StartupError.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { StartupError } from "./StartupError";

test("a second instance explains itself and can quit", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.requestQuit = vi.fn().mockResolvedValue(undefined);
  await renderWithProviders(
    <StartupError code="already_running" detail="another IssueRelay instance is already running" />,
    { backend },
  );
  expect(screen.getByText(/Another IssueRelay is already running/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Quit" }));
  expect(backend.requestQuit).toHaveBeenCalled();
});

test.each([
  ["store_unavailable", "IssueRelay could not open its run records"],
  ["other", "Something stopped IssueRelay from starting"],
] as const)("a %s failure has a translated headline, the cleaned detail and Quit", async (code, headline) => {
  await renderWithProviders(<StartupError code={code} detail={"cannot open state.db: disk full\u001b[2J"} />);
  expect(screen.getByText(headline)).toBeInTheDocument();
  expect(screen.getByText("cannot open state.db: disk full")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Quit" })).toBeInTheDocument();
});
```

`app/src/components/states.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { renderWithProviders } from "@/test/render";
import { ErrorState } from "./states";

test("errors show a translated headline, the backend's line as a cleaned detail, and a retry", async () => {
  const retry = vi.fn();
  await renderWithProviders(
    <ErrorState error={new BackendError("Linear did not answer `list_issues` within 30s\u001b[0m")} onRetry={retry} />,
  );
  expect(screen.getByText("Something went wrong")).toBeInTheDocument();
  expect(screen.getByText("Linear did not answer `list_issues` within 30s")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(retry).toHaveBeenCalled();
});

test("an error the backend answers with as a code is translated and shows no raw text", async () => {
  await renderWithProviders(<ErrorState error={new BackendError("not_a_linear_link")} />);
  expect(screen.getByText("IssueRelay only opens Linear's own pages")).toBeInTheDocument();
  expect(screen.queryByText("not_a_linear_link")).not.toBeInTheDocument();
});
```

`app/src/lib/format.test.ts`：

```ts
import { expect, test } from "vitest";
import { initI18nOnce } from "@/i18n";
import { setFixedClock } from "./clock";
import { formatDuration, formatRelative } from "./format";

test("relative times and durations read naturally in each language", async () => {
  // Explicitly started here: the strings come from our own translation keys.
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("2026-10-12T08:57:00Z", "en")).toBe("3 minutes ago");
  expect(formatRelative("2026-10-12T08:59:00Z", "en")).toBe("1 minute ago");
  expect(formatRelative("2026-10-12T08:59:50Z", "en")).toBe("Just now");
  expect(formatRelative("2026-10-12T06:00:00Z", "en")).toBe("3 hours ago");
  expect(formatRelative("2026-10-12T08:57:00Z", "zh-CN")).toBe("3 分钟前");
  expect(formatDuration("2026-10-12T08:57:37Z", "2026-10-12T09:00:00Z", "en")).toBe("2 min 23 s");
  expect(formatDuration("2026-10-12T08:57:37Z", "2026-10-12T09:00:00Z", "zh-CN")).toBe("2 分 23 秒");
  expect(formatDuration("2026-10-12T08:59:50Z", "2026-10-12T09:00:00Z", "en")).toBe("10 s");
  expect(formatDuration("2026-10-12T07:00:00Z", null, "en")).toBe("2 h 0 min");
});
```

（固定时钟由公共设置在每个测试后复位，测试里不再手动复位。）

`app/src/lib/problem.test.ts`（与 Task 3 的托盘测试读同一张用例表）：

```ts
import { expect, test } from "vitest";
import cases from "./problem-cases.json";
import { problemCause, trayState } from "./problem";
import type { LinearSetup, StatusSnapshot } from "./types";

type Row = (typeof cases)[number];

function inputs(row: Row): [StatusSnapshot, LinearSetup] {
  const setups: Record<string, LinearSetup> = {
    connected: { kind: "connected", method: "oauth" },
    not_configured: { kind: "not_configured" },
    interface_changed: { kind: "interface_changed", message: "list_issues changed" },
    connect_failed: { kind: "connect_failed", message: "network is down" },
    connecting: { kind: "connecting" },
  };
  const status: StatusSnapshot = {
    paused: row.paused,
    linear_ok: row.linear_ok,
    viewer: null,
    last_poll_at: row.checked ? "2026-10-12T08:59:30Z" : null,
    last_error: row.last_error ? "could not start DEMO-1" : null,
    running: row.running,
    queued: 0,
    waiting: 0,
    draining: false,
    held: [],
  };
  return [status, setups[row.linear]];
}

test.each(cases)("$name: the sidebar shows $expected, as the tray does", (row) => {
  expect(trayState(...inputs(row))).toBe(row.expected);
});

test("every problem has a cause the overview can name", () => {
  for (const row of cases) {
    expect(problemCause(...inputs(row)) !== null, row.name).toBe(row.expected === "problem");
  }
});
```

`app/src/components/layout/AppShell.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import { renderWithProviders } from "@/test/render";
import { AppShell } from "./AppShell";

test("the sidebar lists every page and shows the monitoring state", async () => {
  await renderWithProviders(<AppShell />, { lang: "zh-CN" });
  for (const name of ["概览", "需求", "运行记录", "项目绑定", "环境检查", "设置", "使用说明"]) {
    expect(screen.getByRole("link", { name })).toBeInTheDocument();
  }
  expect(await screen.findByText("监控中")).toBeInTheDocument();
});
```

`app/src/App.test.tsx`（替换 Task 1 的标题测试：`App` 自带 `HashRouter` 与查询客户端，所以直接 `render`，不套 `renderWithProviders`）：

```tsx
import { render, screen } from "@testing-library/react";
import { initI18nOnce } from "@/i18n";
import { createMockBackend } from "@/lib/mock";
import App from "./App";

test("the panel opens on the overview inside the shell", async () => {
  await initI18nOnce("en");
  render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Runs" })).toBeInTheDocument();
});

test("any start failure replaces the panel with a readable reason and a Quit button", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.startupState = async () => ({ kind: "failed", code: "store_unavailable", detail: "cannot open state.db" });
  render(<App backend={backend} />);
  expect(await screen.findByText("IssueRelay could not open its run records")).toBeInTheDocument();
  expect(screen.getByText("cannot open state.db")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Quit" })).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败（组件尚不存在）。

- [ ] **Step 3: 实现**

`app/src/lib/problem.ts`：

```ts
import type { LinearSetup, StatusSnapshot } from "./types";

export type TrayState = "idle" | "running" | "paused" | "problem";

/** Why IssueRelay needs attention; the overview names it in a translated headline. */
export type ProblemCause = "not_configured" | "interface_changed" | "connect_failed" | "check_failed" | "start_failed";

/**
 * The problem rule of the tray (`tray::tray_view` in app/src-tauri/src/tray.rs): not connected
 * to Linear, the last check failed, or a job could not start. While the first connection is
 * still under way (`connecting`) nothing is a problem yet. Both sides are tested with
 * app/src/lib/problem-cases.json.
 */
export function problemCause(status: StatusSnapshot, linear: LinearSetup): ProblemCause | null {
  if (linear.kind === "connecting") return null;
  if (linear.kind !== "connected") return linear.kind;
  if (status.last_poll_at !== null && !status.linear_ok) return "check_failed";
  if (status.last_error !== null) return "start_failed";
  return null;
}

/** A problem wins over paused, which wins over running. */
export function trayState(status: StatusSnapshot, linear: LinearSetup): TrayState {
  if (problemCause(status, linear) !== null) return "problem";
  if (status.paused) return "paused";
  return status.running > 0 ? "running" : "idle";
}
```

`app/src/lib/errors.ts`：

```ts
import type { TFunction } from "i18next";
import { BackendError } from "./backend";
import { cleanDisplayText } from "./clean";

/**
 * A failure as the panel shows it (design 9.3: no bare error text): a translated headline, and
 * the backend's own line, cleaned, as a secondary detail. An error the backend answers with as a
 * code gets that code's text as the headline and no detail.
 */
export function describeError(error: unknown, t: TFunction, headlineKey: string): { headline: string; detail: string | null } {
  if (error instanceof BackendError && error.code !== null) {
    return { headline: t(`errors.${error.code}`), detail: null };
  }
  const detail = cleanDisplayText(error instanceof Error ? error.message : String(error)).trim();
  return { headline: t(headlineKey), detail: detail === "" ? null : detail };
}
```

`app/src/hooks/subscription.ts`：

```ts
/**
 * An async subscription inside `useEffect`: `useEffect(() => subscription(backend.onResync(f)), [...])`.
 * The returned cleanup also unsubscribes when the component unmounts before the subscription
 * resolved.
 */
export function subscription(start: Promise<() => void>): () => void {
  let stop: (() => void) | undefined;
  let ended = false;
  void start.then((unsubscribe) => {
    if (ended) unsubscribe();
    else stop = unsubscribe;
  });
  return () => {
    ended = true;
    stop?.();
  };
}
```

`app/src/components/StartupGate.tsx`：

```tsx
import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useEffect } from "react";
import { queryKeys, useStartupState } from "@/hooks/queries";
import { subscription } from "@/hooks/subscription";
import { useBackend } from "@/lib/backend-context";
import { StartupError } from "./StartupError";

/**
 * The startup error page instead of the panel, for any failed start of the host. The state is
 * asked for (`startup_state`, again every second while starting), so a panel opened after the
 * failure sees it too; `startup-failed` updates a panel that was already open. "Ready" means the
 * host started; Linear may still be connecting, which `LinearStatus` shows.
 */
export function StartupGate({ children }: { children: ReactNode }) {
  const backend = useBackend();
  const client = useQueryClient();
  const startup = useStartupState();
  useEffect(
    () => subscription(backend.onStartupFailed((state) => client.setQueryData(queryKeys.startup, state))),
    [backend, client],
  );
  if (startup.data?.kind === "failed") {
    return <StartupError code={startup.data.code} detail={startup.data.detail} />;
  }
  return children;
}
```

`app/src/components/EngineEventsBridge.tsx`：

```tsx
import { useEngineEvents } from "@/hooks/useEngineEvents";

/** Mounts the engine event subscription; it must sit inside `QueryClientProvider`. */
export function EngineEventsBridge() {
  useEngineEvents();
  return null;
}
```

其余按下面实现：

- `backend-context.tsx`：`createContext<Backend | null>(null)`、`BackendProvider`、`useBackend`（拿不到时抛 `Error("BackendProvider is missing")`；面板与测试都显式提供后端，不再有全局默认值）。
- `queries.ts`：每个钩子用 `useBackend()` 取后端；`useStartupState()` 用 `refetchInterval: (query) => (query.state.data?.kind === "starting" ? 1000 : false)`；`useJobLog(id, { live })` 在 `live` 时 `refetchInterval: 1500`；`useLinearServerNamed(executor, enabled)` 的键是 `queryKeys.linearServerNamed(executor)`。变更钩子统一用一个 `useAction(fn, invalidate: QueryKey[], failureKey: string)` 辅助：成功后失效给定的键；失败时 `const { headline, detail } = describeError(error, t, failureKey)`，`toast.error(headline, { description: detail ?? undefined })`（标题已翻译，原文只作说明，纯文本显示）。每个变更钩子的 `failureKey` 是 `actions.failed.<操作>`。
- `useEngineEvents.ts`：在 `useEffect` 里用 `subscription(...)` 订阅 `onEngineEvent` 与 `onResync`，按 Interfaces 里的规则失效查询。
- `StatusBadge.tsx`：shadcn `Badge`（`variant="outline"`），左侧图标，文字为 `t("state.<state>")`；`running` 的图标加 `animate-spin`；颜色用 `text-state-*`、`border-state-*/30`、`bg-state-*/10`。
- `states.tsx`：`EmptyState` 居中、图标 `size-10`、标题 + 说明 + 可选操作按钮；`LoadingState` 用 `Skeleton` 画 `rows` 行（默认 3）；`DetailLine` 是一行 `cleanDisplayText` 之后的原文（`text-muted-foreground`、等宽、小一号、可选中、可折行），用在每个显示引擎原文的地方；`ErrorState` 用 `Alert`（destructive）：标题是 `describeError(error, t, headline ?? "common.error_title")` 的 `headline`，下面是 `DetailLine`（有细节时），有 `onRetry` 时显示 `common.retry` 按钮。
- `PageHeader.tsx`：标题（`text-xl font-semibold`，`h1`）、可选说明（`text-muted-foreground`）、右侧操作区。
- `AppShell.tsx`：左侧侧栏宽 `var(--sidebar-width)`（应用名与图标、七个 `NavLink`：`/` 概览 `LayoutDashboard`、`/issues` 需求 `ListTodo`、`/runs` 运行记录 `History`、`/bindings` 项目绑定 `FolderGit2`、`/doctor` 环境检查 `Stethoscope`、`/settings` 设置 `Settings`、`/help` 使用说明 `CircleHelp`；当前页高亮），侧栏底部是监控状态和版本号：`status.draining` 为真时 `shell.draining`，`linear.kind` 为 `connecting` 时 `shell.connecting`，否则按 `trayState(status, linear)`：`problem` → `shell.problem`（`text-state-failed`）、`paused` → `shell.paused`、`running` / `idle` → `shell.monitoring`；状态读不到时只显示版本号。右侧 `<Outlet />`，内容区最大宽度 `var(--content-max-width)`、左右内边距 `var(--page-padding)`、可滚动。
- `QuitDialog.tsx`：用 `subscription` 订阅 `onQuitRequested`；shadcn `AlertDialog`：标题 `quit.title`，正文 `quit.body`（`count`），三个按钮 `quit.wait`、`quit.stop`、`quit.cancel`；选了等待后换成等待中的说明（`quit.waiting`）和一个 `quit.stop` 按钮（Rust 侧的退出状态机允许从等待改为立即停止）。
- `StartupError.tsx`：整页居中卡片，标题 `startup.failed_title`；正文是按代码翻译的标题 `startup.<code>`（`already_running`、`store_unavailable`、`other`），下面 `DetailLine` 显示 `detail`；按钮 `startup.quit` 调 `requestQuit()`。
- `FirstRunGate.tsx`：读 `useSettings()`；加载中显示 `LoadingState`；读取失败显示 `ErrorState`（带重试）；`onboarding_completed` 为假时 `<Navigate to="/onboarding" replace />`，否则 `<Outlet />`。
- `App.tsx`：`App({ backend }: { backend: Backend })`：`BackendProvider` → `QueryClientProvider`（`useState` 里建一个 `QueryClient`）→ `<EngineEventsBridge/>`、`<ThemeSync/>`（调 `useSystemTheme()`）、`<LanguageSync/>`、`<StartupGate>` 包着 `HashRouter`；路由：`/onboarding` 引导页（不带外框），其余页面在 `FirstRunGate` → `AppShell` 之下；`QuitDialog`、`Toaster` 挂在 `StartupGate` 之外。`LanguageSync` 读 `useAppInfo()` 与 `useSettings()`，`resolveLanguage(settings.language, appInfo.system_locale ?? undefined)` 与当前语言不同就 `setLanguage`（用户在设置里选了非系统语言时，第一屏之后切换一次）。
- 页面占位：每个页面先只渲染 `PageHeader`（标题为导航名）。
- `main.tsx`：`import "./styles.css"`；`const backend = await loadBackend(); const info = await backend.appInfo(); await initI18n(resolveLanguage("system", info.system_locale ?? undefined));` 后渲染 `<App backend={backend} />`（`app_info` 不等宿主，立即返回）。`components.json` 的样式路径改为 `src/styles.css`，删除模板的 `index.css`。
- `format.ts`：`formatRelative` 不到 1 分钟为 `format.just_now`，其后依次用 `format.minutes_ago`、`format.hours_ago`、`format.days_ago`（`count` 复数），超过 7 天用 `formatDateTime`；`formatDuration` 用 `format.duration_s`、`format.duration_ms`、`format.duration_hm`；都用 `i18n.getFixedT(lang)`，不依赖当前语言；`formatDateTime`、`formatTime` 用 `Intl.DateTimeFormat`（只用于显示，测试不比对其确切输出）。
- 翻译键（`en` 的确切文案如下；五种语言同步补齐，`_one` / `_other` 两个键每种语言都写，中日韩两者内容相同）：

```json
{
  "state": { "retrying_at": "Retrying at {{time}}" },
  "shell": {
    "monitoring": "Monitoring",
    "paused": "Paused",
    "draining": "Waiting for runs to finish",
    "problem": "Needs attention",
    "connecting": "Connecting to Linear…",
    "version": "Version {{version}}"
  },
  "format": {
    "just_now": "Just now",
    "minutes_ago_one": "{{count}} minute ago",
    "minutes_ago_other": "{{count}} minutes ago",
    "hours_ago_one": "{{count}} hour ago",
    "hours_ago_other": "{{count}} hours ago",
    "days_ago_one": "{{count}} day ago",
    "days_ago_other": "{{count}} days ago",
    "duration_s": "{{s}} s",
    "duration_ms": "{{m}} min {{s}} s",
    "duration_hm": "{{h}} h {{m}} min"
  },
  "executor": { "claude": "Claude Code", "codex": "Codex", "default": "Default ({{name}})" },
  "permission": { "full": "Full access", "auto": "Automatic review", "personal": "Personal settings", "default": "Default ({{name}})" },
  "actions": {
    "failed": {
      "pause": "Could not pause monitoring",
      "resume": "Could not resume monitoring",
      "poll_now": "Could not start a check",
      "cancel": "Could not stop the run",
      "retry": "Could not run it again",
      "mark_done": "Could not mark it done",
      "save_binding": "Could not save the binding",
      "remove_binding": "Could not remove the binding",
      "save_settings": "Could not save the settings",
      "linear_login": "Could not log in to Linear",
      "linear_api_key": "Could not use the API key",
      "linear_logout": "Could not log out of Linear",
      "linear_reconnect": "Could not reconnect to Linear",
      "setup_action": "The setup command failed",
      "set_autostart": "Could not change start at login"
    }
  },
  "errors": {
    "not_a_linear_link": "IssueRelay only opens Linear's own pages",
    "login_cancelled": "The browser login was cancelled"
  }
}
```

（`errors.login_cancelled` 的 `zh-CN` 为「已取消浏览器登录」；`useLinearLogin` 不为它提示，`describeError` 遇到时照样有译文。）

（`zh-CN` 里本任务测试用到的：`shell.monitoring` 为「监控中」，`format.minutes_ago_one` / `_other` 为「{{count}} 分钟前」，`format.duration_ms` 为「{{m}} 分 {{s}} 秒」，`format.duration_hm` 为「{{h}} 小时 {{m}} 分」，`format.duration_s` 为「{{s}} 秒」。）

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS。再 `pnpm --dir app dev` 在浏览器里打开 `http://localhost:1420/`，确认外框、导航、深浅色切换（改系统外观）正常，截一张图附在报告里（只用假数据）。

- [ ] **Step 4: 文档与提交**

- 设计第 9.3 节补：「主色靛蓝；系统字体栈（含中日韩字体）；布局常量与八种状态的颜色、图标见 `app/src/styles.css` 与 `StatusBadge.tsx`。失败一律显示为『已翻译的标题＋清洗过的细节行』，引擎原文不单独出现。」

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs app/pnpm-lock.yaml
git commit -m "feat(app): add the panel shell, design tokens and data hooks"
```

---

### Task 6: 概览页与需求页

设计第 9.2 节：概览（监控开关、上次检查时间、运行中与排队中的任务、最近结果、需要处理的提醒：失败、待验收、环境问题）；需求（当前用户所有带 `ai-ready` 的需求及其本地状态；操作：在 Linear 中打开、停止、重新运行、标记完成）。「标记完成」是程序唯一的 Linear 写操作，点之前要确认。

**Files:**
- Modify: `app/src/pages/Overview.tsx`、`app/src/pages/Issues.tsx`
- Create: `app/src/components/JobActions.tsx`（停止 / 重新运行 / 标记完成 / 在 Linear 中打开，概览、需求、运行详情共用）、`app/src/components/LinearStatus.tsx`（概览与 Task 9 的环境检查页共用）
- Modify: 五个翻译文件（新增 `overview`、`issues`、`linear` 命名空间；在 Task 5 建好的 `actions` 里补操作名与确认文案）
- Test: `app/src/pages/Overview.test.tsx`、`app/src/pages/Issues.test.tsx`、`app/src/components/JobActions.test.tsx`、`app/src/components/LinearStatus.test.tsx`

**Interfaces:**
- Produces: `JobActions({ job?: Job | null, url?: string, compact?: boolean })`：没有任务时只有「在 Linear 中打开」（有 `url` 时）；有任务时按状态只显示能用的操作——排队 / 等待 / 运行中：停止；失败 / 已取消 / 待补充信息：重新运行，且只在它是面板手上这条需求最新的一次运行时（`useJobs()` 里同一 `issue_uuid` 没有更大的 `id`；列表还没读到时照常显示，引擎也会拒绝旧的运行）；待验收：标记完成（确认对话框，按钮 `actions.confirm`）；有 `url` 时：在 Linear 中打开。`LinearStatus({ manage?: boolean })`：按 `StatusView.linear` 与 `status` 显示连接状态与对应操作，标题都由结构化数据得出：首次连接还在进行（`connecting`，AR51）→ `linear.connecting`「Connecting to Linear…」，没有任何操作按钮；未登录 → `linear.not_configured` ＋「Log in to Linear」；点了登录、浏览器还没回来时（`useLinearLogin` 进行中）→ `linear.waiting_browser`「Finish the authorization in your browser…」＋「Cancel」（`linear.cancel_login`），没有登录按钮；「Cancel」调 `useLinearLogin` 的 `cancel()` 丢掉这次登录（core 写明丢掉 `Host::login` 就放弃登录，钥匙串只在浏览器回调之后才写），卡片回到原来的样子，不提示失败；连接失败 → `linear.connect_failed` ＋ `DetailLine`（说明原文）＋重试；接口已变化 → `linear.interface_changed` ＋ `DetailLine`；已连接 → 「已连接」与授权人姓名（姓名单独一个元素）和上次检查时间；`manage` 为真时（环境检查页）已连接状态另有「重新授权」与「退出登录」，「重新授权」等浏览器时同样显示上面的等待状态与「Cancel」。
- Consumes: Task 5 的钩子、`StatusBadge`、三种状态组件与 `DetailLine`、`PageHeader`、`trayState`/`problemCause`、`formatTime`、`state.retrying_at`；Task 4 的 `BackendError`、`StatusSnapshot.held`、`WaitReason`。

页面规格：

**概览**（路由 `/`）

| 区块 | 内容 | 数据 |
|---|---|---|
| 页头 | 标题 `nav.overview`；右侧「立即检查」按钮（`actions.poll_now`）与监控开关（`Switch`，名称 `overview.monitoring`，开 = 监控中；关 = 已暂停，调 `pause` / `resume`） | `useStatus` |
| 连接卡 | `LinearStatus` | `useStatus` |
| 提醒 | 按需显示的 `Alert`：环境问题（`problemCause(status, linear)` 不为空时，标题为 `overview.problem.<原因>`，有说明原文时（`linear.message` 或 `status.last_error`）下面一行 `DetailLine`，并带链接「Open the environment check」跳 `/doctor`；判定与托盘同一规则）；有失败（`overview.alert_failed`：只数最新一次运行是失败的需求，被后来的运行取代的旧失败不算，带「查看」跳 `/runs?state=failed`）；有待验收（`overview.alert_awaiting`，跳 `/issues`）；正在等待运行结束（`status.draining`，`overview.draining`） | `useStatus`、`useJobs` |
| 运行中 | 每条：编号、标题、执行者（`executor.*`）、已运行时长（每秒刷新）、`JobActions`（停止）；没有时 `EmptyState`（`overview.no_running`） | `useJobs` 过滤 `running` |
| 排队 | 排队中与等待绑定的任务：排队中的若在 `status.held` 里，显示 `state.retrying_at`（`formatTime(retry_at)`）；等待绑定的以 `StatusBadge` 作标题、任务原因作 `DetailLine`；没有时一行淡色文字 `overview.queue_empty` | `useJobs`、`useStatus` |
| 最近结果 | 最近 8 条已结束的任务（待验收 / 待补充信息 / 已完成 / 失败 / 已取消）：状态、编号、标题、结束时间（相对时间）、摘要（`cleanDisplayText`）或原因（`DetailLine`）的第一行；点击进运行详情；没有时 `overview.recent_empty` | `useJobs` |
| 加载 / 错误 | `status` 读取中时连接卡与提醒位置显示 `LoadingState`；读取失败时整页换成 `ErrorState`（标题 `overview.status_failed`，带重试）；`jobs` 读取失败时三个任务区块换成一个 `ErrorState`（标题 `overview.jobs_failed`） | |

**需求**（路由 `/issues`）

| 区块 | 内容 |
|---|---|
| 页头 | 标题 `nav.issues`，说明 `issues.description`（「指派给你、带 {{label}} 标签的需求，最近更新的 50 条」）；右侧刷新按钮 |
| 列表 | `Table`：编号（等宽）、标题、Linear 状态、本地状态（有任务时 `StatusBadge`，没有时淡色「—」）、原因（`IssueStatus`：`eligible` → `issues.eligible`「下一次检查会接走」；`not_todo` → `issues.not_todo`「状态是 {{state}}，不是 {{todo}}」；`not_candidate` → `issues.not_candidate`；`waiting` → 按代码翻译：`issues.waiting.project_not_bound`、`issues.waiting.directory_missing`（`{{directory}}`）、`issues.waiting.executor_not_ready`（`{{executor}}` 用 `executor.*` 的译名）；`paused` → `issues.paused`）、操作（`JobActions`，`job` 可为空，`url` 为需求链接） |
| 加载 | `LoadingState`（5 行） |
| 空状态 | 没有需求时 `EmptyState`：`issues.empty_title`「还没有带 {{label}} 标签的需求」、说明触发条件、按钮跳 `/help` |
| 错误状态 | `ErrorState`（标题 `issues.load_failed`；例如未登录时细节行是后端的「not connected to Linear: log in first」）＋ 按钮「Open the guide」跳 `/onboarding`、「Open the environment check」跳 `/doctor` |

- [ ] **Step 1: 失败测试**

`app/src/components/JobActions.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { JobState } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { JobActions } from "./JobActions";

async function jobIn(state: JobState) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.listJobs(100)).find((j) => j.state === state)!;
  return { backend, job };
}

test.each([
  ["running", ["Stop"]],
  ["queued", ["Stop"]],
  ["waiting_binding", ["Stop"]],
  ["failed", ["Run again"]],
  ["canceled", ["Run again"]],
  ["needs_info", ["Run again"]],
  ["awaiting_acceptance", ["Mark done"]],
  ["done", []],
] as [JobState, string[]][])("a %s job offers %j", async (state, expected) => {
  const { backend, job } = await jobIn(state);
  await renderWithProviders(<JobActions job={job} />, { backend });
  for (const name of ["Stop", "Run again", "Mark done"]) {
    const button = screen.queryByRole("button", { name });
    expect(Boolean(button), name).toBe(expected.includes(name));
  }
});

test("only the newest run of an issue offers Run again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const older = (await backend.job(2))!; // DEMO-12's earlier failed run; DEMO-12 runs again now
  await renderWithProviders(<JobActions job={older} />, { backend });
  await waitFor(() => expect(screen.queryByRole("button", { name: "Run again" })).not.toBeInTheDocument());
});

test("an issue without a run only offers to open it in Linear", async () => {
  await renderWithProviders(<JobActions job={null} url="https://linear.app/demo/issue/DEMO-4" />);
  expect(screen.getByRole("button", { name: "Open in Linear" })).toBeInTheDocument();
  for (const name of ["Stop", "Run again", "Mark done"]) {
    expect(screen.queryByRole("button", { name })).not.toBeInTheDocument();
  }
});

test("mark done asks first and names the only write to Linear", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  const markDone = vi.spyOn(backend, "markDone");
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  expect(screen.getByText(new RegExp(`${job.issue_identifier}.*Done`))).toBeInTheDocument();
  expect(markDone).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(markDone).toHaveBeenCalledWith(job.id);
});

test("a failed action shows a translated headline and the backend's reason", async () => {
  const { backend, job } = await jobIn("failed");
  backend.retryJob = vi.fn().mockRejectedValue(new BackendError("job 8 is not the latest run of DEMO-8; retry that one instead"));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Run again" }));
  expect(await screen.findByText("Could not run it again")).toBeInTheDocument();
  expect(screen.getByText(/not the latest run of DEMO-8/)).toBeInTheDocument();
});

test("only Linear links are offered for opening", async () => {
  const { backend, job } = await jobIn("done");
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<JobActions job={job} url="https://linear.app/demo/issue/DEMO-7" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open in Linear" }));
  expect(open).toHaveBeenCalledWith("https://linear.app/demo/issue/DEMO-7");
});
```

`app/src/pages/Overview.test.tsx`：

```tsx
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { BackendError } from "@/lib/backend";
import { formatTime } from "@/lib/format";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Overview from "./Overview";

test("the overview shows what runs, what waits and what needs attention", async () => {
  await renderWithProviders(<Overview />);
  const running = await screen.findByRole("region", { name: "Running" });
  expect(within(running).getByText("DEMO-12")).toBeInTheDocument();
  const queued = screen.getByRole("region", { name: "Queued" });
  expect(queued).toHaveTextContent("DEMO-11");
  expect(queued).toHaveTextContent(`Retrying at ${formatTime("2026-10-12T09:02:00Z", "en")}`);
  // DEMO-8's newest run failed; DEMO-12's old failure was followed by a new run and is not counted.
  expect(await screen.findByText(/1 run failed/)).toBeInTheDocument();
  expect(screen.getByText(/1 issue awaits your acceptance/)).toBeInTheDocument();
  expect(screen.getByText("Jane Doe")).toBeInTheDocument();
});

test("the monitoring switch pauses and resumes", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const pause = vi.spyOn(backend, "pause");
  await renderWithProviders(<Overview />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Monitoring" }));
  expect(pause).toHaveBeenCalled();
});

test("a first run without a login offers the login and the environment check", async () => {
  await renderWithProviders(<Overview />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
  expect(screen.getByText("Nothing is running")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Open the environment check" })).toBeInTheDocument();
});

test("while the first connection is under way the overview says so, then follows it", async () => {
  const backend = createMockBackend("connecting", { latencyMs: 0 });
  await renderWithProviders(
    <>
      <EngineEventsBridge />
      <Overview />
    </>,
    { backend },
  );
  expect(await screen.findByText("Connecting to Linear…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log in to Linear" })).not.toBeInTheDocument();
  // Connecting is no problem yet: no reminder.
  expect(screen.queryByRole("link", { name: "Open the environment check" })).not.toBeInTheDocument();
  // The connection finishes; the Status event refreshes the card without a reload.
  await act(() => backend.linearReconnect());
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
});

test("an environment problem is named, with the engine's line as a detail", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = vi.fn().mockResolvedValue({
    ...view,
    status: { ...view.status, last_error: "could not start DEMO-11: Linear did not answer" },
  });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("A run could not be started")).toBeInTheDocument();
  expect(screen.getByText("could not start DEMO-11: Linear did not answer")).toBeInTheDocument();
});

test("when the status cannot be read the page says so and offers a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.status = vi.fn().mockRejectedValue(new BackendError("the engine did not answer"));
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Could not read IssueRelay's status")).toBeInTheDocument();
  expect(screen.getByText("the engine did not answer")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});
```

`app/src/pages/Issues.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Issues from "./Issues";

test("each labeled issue says whether it will be picked up", async () => {
  await renderWithProviders(<Issues />);
  expect(await screen.findByText("DEMO-4")).toBeInTheDocument();
  expect(screen.getByText(/status is Backlog, not Todo/)).toBeInTheDocument();
  // Translated from the code `project_not_bound`, not the engine's English sentence.
  expect(screen.getByText(/not bound to a local folder/)).toBeInTheDocument();
});

test("without a Linear connection the list explains what to do", async () => {
  await renderWithProviders(<Issues />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("Could not read the issues from Linear")).toBeInTheDocument();
  expect(screen.getByText(/not connected to Linear/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open the guide" })).toBeInTheDocument();
});
```

`app/src/components/LinearStatus.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { LinearStatus } from "./LinearStatus";

// core takes one change of the login at a time, and a login waits for the browser for up to ten
// minutes: the user must be able to give it up.
test("a login that waits for the browser can be cancelled", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  let cancel = () => {};
  backend.linearLogin = vi.fn(
    () =>
      new Promise<never>((_, reject) => {
        cancel = () => reject(new BackendError("login_cancelled"));
      }),
  );
  backend.linearCancelLogin = vi.fn(async () => cancel());
  await renderWithProviders(<LinearStatus />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log in to Linear" })).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(backend.linearCancelLogin).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
  // The user gave it up: no failure is shown.
  expect(screen.queryByText("Could not log in to Linear")).not.toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按上面的页面规格实现。区块用 `<section aria-labelledby>` 包起来，标题即区块名（测试按 `role="region"` 找）。已运行时长用 `formatDuration(started_at, null, lang)`，每秒刷新一个本地计时器（组件卸载时清掉）。失败提醒的计数：把 `useJobs()` 的任务按 `issue_uuid` 分组、取每组 `id` 最大的一条，数其中 `failed` 的个数。英文文案：

- `overview`：`monitoring` = `Monitoring`；`running` = `Running`、`queued` = `Queued`、`recent` = `Recent results`（区块名）；`no_running` = `Nothing is running`；`queue_empty` = `Nothing is waiting`；`recent_empty` = `No finished runs yet`；`alert_failed_one` = `{{count}} run failed`、`alert_failed_other` = `{{count}} runs failed`；`alert_awaiting_one` = `{{count}} issue awaits your acceptance`、`alert_awaiting_other` = `{{count}} issues await your acceptance`；`view` = `View`；`draining` = `Waiting for the runs in progress to finish; nothing new starts`；`open_doctor` = `Open the environment check`；`problem.not_configured` = `Not logged in to Linear`、`problem.interface_changed` = `Linear's tools changed: update IssueRelay`、`problem.connect_failed` = `Could not connect to Linear; IssueRelay tries again every minute`、`problem.check_failed` = `The last check of Linear failed`、`problem.start_failed` = `A run could not be started`；`status_failed` = `Could not read IssueRelay's status`；`jobs_failed` = `Could not read the runs`。复数用 i18next 的 `_one` / `_other`，五个文件都写两个键（中日韩两者内容相同）。
- `issues`：`not_todo` = `Its status is {{state}}, not {{todo}}`；`eligible` = `The next check picks it up`；`waiting.project_not_bound` = `Its project is not bound to a local folder`、`waiting.directory_missing` = `The bound folder {{directory}} does not exist`、`waiting.executor_not_ready` = `{{executor}} is not ready; see the environment check`；`load_failed` = `Could not read the issues from Linear`；`open_guide` = `Open the guide`。
- `actions`：`stop` = `Stop`、`run_again` = `Run again`、`mark_done` = `Mark done`、`open_in_linear` = `Open in Linear`、`poll_now` = `Check now`、`confirm` = `Confirm`；确认对话框文案 `actions.mark_done_confirm`：「把 {{identifier}} 设为 {{done}}？这是 IssueRelay 唯一会改动 Linear 的操作。」（英文 `Set {{identifier}} to {{done}}? This is the only change IssueRelay makes in Linear.`）。Task 8 的删除确认复用 `actions.confirm`。
- `linear`：`connecting` = `Connecting to Linear…`、`not_configured` = `Not logged in to Linear`、`log_in` = `Log in to Linear`、`connect_failed` = `Could not connect to Linear`、`interface_changed` = `Linear's tools changed: update IssueRelay`、`retry` = `Try again`、`connected` = `Connected as`、`last_check` = `Last check {{time}}`、`log_in_again` = `Authorize again`、`log_out` = `Log out`、`waiting_browser` = `Finish the authorization in your browser…`、`cancel_login` = `Cancel`。

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS。在浏览器（`pnpm --dir app dev`）里看一遍两个页面的浅色与深色，截图附在报告里。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app
git commit -m "feat(app): add the overview and issues pages"
```

---

### Task 7: 运行记录与运行详情（时间线、原始日志）

设计第 9.2 节「运行记录：历史运行列表；详情页显示可读的时间线和原始日志，支持复制」、6.1 节「输出逐行读取：原样写进日志文件，同时解析出面板显示用的时间线（AI 消息、工具调用、命令、错误）」。执行者与 AI 的文本显示前都经 Task 4 的 `cleanDisplayText` / `cleanDisplayLines`。

**Files:**
- Create: `app/src/lib/timeline.ts`、`app/src/lib/timeline.test.ts`（直接读 core 的夹具 `crates/core/tests/fixtures/{claude-stream,codex-exec}.jsonl`，不复制）
- Modify: `app/src/pages/Runs.tsx`、`app/src/pages/RunDetail.tsx`、五个翻译文件（`runs`、`timeline` 命名空间；执行者名用 Task 5 的 `executor` 命名空间）
- Test: `app/src/pages/Runs.test.tsx`、`app/src/pages/RunDetail.test.tsx`

**Interfaces:**
- Produces:
  - `type TimelineEntry = { kind: "message" | "tool" | "command" | "result" | "error" | "stderr" | "note" | "system"; text: string; detail?: string; failed?: boolean }`（`note` 是 IssueRelay 自己的运行记录行，例如启动失败）
  - `parseTimeline(lines: string[], executor: ExecutorKind | null): TimelineEntry[]`（产出的 `text`、`detail` 都已清洗）
  - `RUN_FILTERS = ["all", "waiting_binding", "queued", "running", "awaiting_acceptance", "needs_info", "done", "failed", "canceled"] as const`：`/runs?state=` 接受的值，`all` 加上 `JobState` 的八个名字；其他值按 `all` 处理。Task 6 的提醒链接 `/runs?state=failed` 用其中之一。
- Consumes: Task 4 的 `cleanDisplayText`、`cleanDisplayLines`、`StatusSnapshot.held`；Task 5 的 `useJobs`、`useJob`、`useJobLog`、`useIssues`、`useStatus`、`StatusBadge`、三种状态组件与 `DetailLine`、`formatTime`；Task 6 的 `JobActions`。

解析规则（每行一个 JSON；解析不了的行算 `stderr`，原文清洗后保留）：

| 来源 | 事件 | 时间线条目 |
|---|---|---|
| Claude | `{"type":"system","subtype":"init",…}` | `system`：`timeline.session_started`（带模型名，若有） |
| Claude | `{"type":"assistant","message":{"content":[{"type":"text","text":…}]}}` | 每段文本一条 `message`（`cleanDisplayLines`） |
| Claude | 同上，`{"type":"tool_use","name":…,"input":…}` | `tool`：`text` = 工具名，`detail` = `JSON.stringify(input)`（截到 2000 字符）；`Bash` 时为 `command`：`text` = `input.command`，先不带 `detail` |
| Claude | `{"type":"user","message":{"content":[{"type":"tool_result","content":…,"is_error":…}]}}` | 按 `tool_use_id` 并入对应的那一条（找不到就并入上一条 `tool` / `command`）：`command` 的结果**取代** `detail`（命令行本身已在 `text` 里）；其他工具的结果**接在**输入之后（`detail = 输入 + "\n\n" + 结果`）；结果截到 2000 字符，`is_error` 为真时 `failed: true` |
| Claude | `{"type":"result",…,"result":…,"is_error":…}` | `result`：`text` = `result`；`is_error` 为真时 `failed: true` |
| Codex | `{"type":"item.completed","item":{"type":"agent_message","text":…}}` | `message` |
| Codex | `{"type":"item.completed","item":{"type":"command_execution","command":…,"aggregated_output":…,"exit_code":…}}` | `command`：`text` = 命令，`detail` = 输出（截断），`exit_code` 非 0 时 `failed: true` |
| Codex | `{"type":"item.completed","item":{"type":"mcp_tool_call","server":…,"tool":…}}` | `tool`：`text` = `server.tool` |
| Codex | `{"type":"error","message":…}` 或 `{"type":"turn.failed",…}` | `error` |
| 两者 | IssueRelay 自己的 `{"stream":"stderr","text":…}` | `stderr` |
| 两者 | IssueRelay 自己的 `{"stream":"runner","text":…}`（例如 `spawn failed: …`） | `note`，样式同 `stderr` 行，标签 `timeline.note`（「IssueRelay」） |
| 两者 | 其余事件（`thread.started`、`turn.started`、`item.started`、用量统计等） | 跳过 |

每条的 `text` 经 `cleanDisplayText`（多行的消息与 `detail` 经 `cleanDisplayLines`），截断按清洗后的长度算。

页面规格：

**运行记录**（`/runs`）：页头＋状态筛选（`Tabs`：「全部」加八种状态，标签用 `runs.all` 与 `state.*`；当前值同步到地址栏 `?state=`，取值见 `RUN_FILTERS`）；列表（`Table`）：状态（`StatusBadge`）、编号、标题、执行者（`executor.*`）、开始时间（相对时间）、时长、说明：排队中且在 `status.held` 里的显示 `state.retrying_at`，否则摘要（`cleanDisplayText`）或原因（`DetailLine` 样式）的第一行；行可点击进详情。加载：`LoadingState`；空：`EmptyState`（`runs.empty`，筛选后为空时 `runs.empty_filtered`）；错误：`ErrorState`（标题 `runs.load_failed`，带重试）。

**运行详情**（`/runs/:id`）：页头为编号＋标题，右侧 `JobActions`（`url` 从 `useIssues()` 里按 `issue_uuid` 找；`Job` 没有链接字段；需求列表读取失败或找不到时只是不显示「在 Linear 中打开」，页面其余部分照常）；信息卡：状态、执行者、目录（等宽、可复制）、创建 / 开始 / 结束时间、时长、摘要（`cleanDisplayText`）；有原因时显示由状态得出的标题 `runs.reason_title.<failed|canceled|waiting_binding|other>` 与原因的 `DetailLine`；`Tabs` 两页：时间线（按上表渲染：消息用正文样式，工具与命令为 `Collapsible` 可展开的卡片，失败的用 `--state-failed` 标出，结果卡片放在最后）与原始日志（`ScrollArea` 内等宽显示 `cleanDisplayText` 之后的每一行，右上角「复制全部」）；运行中时日志每 1.5 秒刷新并自动滚到底（用户上翻后暂停自动滚动）；日志被清理（返回空）且任务已结束时显示 `runs.log_removed`。加载：`LoadingState`；找不到（`job(id)` 为 `null`）：`EmptyState`（`runs.not_found`，按钮回 `/runs`）；错误：`ErrorState`（标题 `runs.detail_failed`，带重试）；日志读取失败只在原始日志页显示 `ErrorState`。

- [ ] **Step 1: 失败测试**

`app/src/lib/timeline.test.ts`（直接读 core 的夹具：以 `?raw` 导入，Vite 把文件内容当字符串；若解析到的 Vitest 不允许导入项目根目录之外的文件，改为在 `app/src/lib/__fixtures__/` 放副本，并加一条测试用 `node:fs` 逐字节比对副本与 core 的原件）：

```ts
import { expect, test } from "vitest";
import claudeStream from "../../../crates/core/tests/fixtures/claude-stream.jsonl?raw";
import codexExec from "../../../crates/core/tests/fixtures/codex-exec.jsonl?raw";
import { parseTimeline } from "./timeline";

const linesOf = (text: string) => text.split("\n").filter(Boolean);

test("a real Claude stream becomes a readable timeline", () => {
  const entries = parseTimeline(linesOf(claudeStream), "claude");
  expect(entries[0].kind).toBe("system");
  expect(entries.some((e) => e.kind === "tool" || e.kind === "command")).toBe(true);
  expect(entries.at(-1)?.kind).toBe("result");
  expect(entries.filter((e) => e.kind === "message").length).toBeGreaterThan(0);
});

test("a real Codex stream becomes a readable timeline", () => {
  const entries = parseTimeline(linesOf(codexExec), "codex");
  expect(entries.some((e) => e.kind === "message")).toBe(true);
  expect(entries.every((e) => e.text.length > 0)).toBe(true);
});

test("a command's result replaces its detail and failures are marked", () => {
  const lines = [
    JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "t1", name: "Bash", input: { command: "cargo test" } }] } }),
    JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t1", content: "1 failed", is_error: true }] } }),
    JSON.stringify({ type: "result", result: "could not fix the test", is_error: true }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "command", text: "cargo test", detail: "1 failed", failed: true },
    { kind: "result", text: "could not fix the test", failed: true },
  ]);
});

test("another tool's result is added below its input", () => {
  const lines = [
    JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "t2", name: "Read", input: { file_path: "src/lib.rs" } }] } }),
    JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t2", content: "fn main() {}", is_error: false }] } }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "tool", text: "Read", detail: '{"file_path":"src/lib.rs"}\n\nfn main() {}' },
  ]);
});

test("Codex commands carry their output and exit code", () => {
  const lines = [
    JSON.stringify({ type: "item.started", item: { type: "command_execution", command: "git push" } }),
    JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: "git push", aggregated_output: "rejected", exit_code: 1 } }),
    JSON.stringify({ type: "error", message: "stream disconnected" }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    { kind: "command", text: "git push", detail: "rejected", failed: true },
    { kind: "error", text: "stream disconnected" },
  ]);
});

test("unreadable lines and IssueRelay's own notes stay visible, cleaned", () => {
  const lines = [
    "not json at all",
    JSON.stringify({ stream: "stderr", text: "\u001b[31mwarning\u001b[0m: slow" }),
    JSON.stringify({ stream: "runner", text: "spawn failed: No such file or directory" }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "stderr", text: "not json at all" },
    { kind: "stderr", text: "warning: slow" },
    { kind: "note", text: "spawn failed: No such file or directory" },
  ]);
});

test("text from the AI is cleaned before it is shown", () => {
  // U+202E (right-to-left override) is removed, U+001B [2J (clear the screen) too.
  const lines = [JSON.stringify({ type: "assistant", message: { content: [{ type: "text", text: "done\u202e\u001b[2J" }] } })];
  expect(parseTimeline(lines, "claude")).toEqual([{ kind: "message", text: "done" }]);
});

test("long details are cut", () => {
  const lines = [
    JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: "cat big", aggregated_output: "x".repeat(5000), exit_code: 0 } }),
  ];
  const [entry] = parseTimeline(lines, "codex");
  expect(entry.detail!.length).toBeLessThanOrEqual(2001);
});
```

`app/src/pages/RunDetail.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import RunDetail from "./RunDetail";

const page = <Routes><Route path="/runs/:id" element={<RunDetail />} /></Routes>;

async function open(state: string, backend = createMockBackend("demo", { latencyMs: 0 })) {
  const job = (await backend.listJobs(100)).find((j) => j.state === state)!;
  await renderWithProviders(page, { backend, route: `/runs/${job.id}` });
  return { backend, job };
}

test("a running job shows its timeline, its raw log and its Linear link", async () => {
  const { job } = await open("running");
  expect(await screen.findByRole("heading", { name: new RegExp(job.issue_identifier) })).toBeInTheDocument();
  expect(await screen.findByText(/Session started/)).toBeInTheDocument();
  expect(await screen.findByRole("button", { name: "Open in Linear" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByRole("button", { name: "Copy all" })).toBeInTheDocument();
});

test("a failed job says why under a translated headline", async () => {
  await open("failed");
  expect(await screen.findByText("The run failed")).toBeInTheDocument();
  expect(screen.getByText("run exceeded the time limit")).toBeInTheDocument();
});

test("the page works when the issue list cannot be read, just without the link", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.issues = vi.fn().mockRejectedValue(new BackendError("not connected to Linear: log in first"));
  const { job } = await open("running", backend);
  expect(await screen.findByRole("heading", { name: new RegExp(job.issue_identifier) })).toBeInTheDocument();
  expect(await screen.findByText(/Session started/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Open in Linear" })).not.toBeInTheDocument();
});

test("a run that does not exist says so", async () => {
  await renderWithProviders(page, { route: "/runs/999" });
  expect(await screen.findByText("This run does not exist")).toBeInTheDocument();
});

test("when the run cannot be read the page says so and offers a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.job = vi.fn().mockRejectedValue(new BackendError("cannot read the job records"));
  await renderWithProviders(page, { backend, route: "/runs/12" });
  expect(await screen.findByText("Could not read the run")).toBeInTheDocument();
  expect(screen.getByText("cannot read the job records")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});
```

`app/src/pages/Runs.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { formatTime } from "@/lib/format";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Runs from "./Runs";

test("runs can be filtered by state", async () => {
  await renderWithProviders(<Runs />);
  expect(await screen.findByText("DEMO-7")).toBeInTheDocument();
  expect(await screen.findByText(`Retrying at ${formatTime("2026-10-12T09:02:00Z", "en")}`)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Failed" }));
  expect(screen.getByText("DEMO-8")).toBeInTheDocument();
  expect(screen.queryByText("DEMO-7")).not.toBeInTheDocument();
});

test("the state in the address selects its tab", async () => {
  await renderWithProviders(<Runs />, { route: "/runs?state=failed" });
  expect(await screen.findByText("DEMO-8")).toBeInTheDocument();
  expect(screen.queryByText("DEMO-7")).not.toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Failed" })).toHaveAttribute("aria-selected", "true");
});

test("when the runs cannot be read the page says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.listJobs = vi.fn().mockRejectedValue(new BackendError("cannot read the job records"));
  await renderWithProviders(<Runs />, { backend });
  expect(await screen.findByText("Could not read the runs")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

`timeline.ts` 按上表实现（清洗用 Task 4 的 `cleanDisplayText` / `cleanDisplayLines`，不再另写 `stripAnsi`）。复制用 `navigator.clipboard.writeText`（复制的是清洗后的行），成功后 toast `common.copied`。页面按规格实现。英文文案：`runs.all` = `All`、`runs.empty` = `No runs yet`、`runs.empty_filtered` = `No runs in this state`、`runs.load_failed` = `Could not read the runs`、`runs.detail_failed` = `Could not read the run`、`runs.not_found` = `This run does not exist`、`runs.reason_title.failed` = `The run failed`、`runs.reason_title.canceled` = `The run was canceled`、`runs.reason_title.waiting_binding` = `The run waits for setup`、`runs.reason_title.other` = `A note from IssueRelay`、`runs.timeline` = `Timeline`、`runs.raw_log` = `Raw log`、`runs.copy_all` = `Copy all`、`runs.log_removed` = `The log of this run was removed by the log cleanup`、`timeline.session_started` = `Session started`（有模型名时后面跟一个淡色的模型名）、`timeline.note` = `IssueRelay`。

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS。浏览器里看运行中、失败、待验收三种详情页，截图附在报告里。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app
git commit -m "feat(app): add the run history with a readable timeline and the raw log"
```

---

### Task 8: 项目绑定

设计第 9.2 节「项目绑定：每个绑定的目录、执行者、权限档位、提示词覆盖和检查结果」、第 8 节第 5 步、第 14 节「首次选择『完全放开』时明确提示风险」。

**Files:**
- Modify: `app/src/pages/Bindings.tsx`
- Create: `app/src/pages/BindingDialog.tsx`、`app/src/components/FullAccessWarning.tsx`、`app/src/lib/model-name.ts`、`app/src/lib/model-name-cases.json`（模型名用例表：TS 测试与 Rust 测试都读）
- Modify: `app/src-tauri/src/commands.rs`（测试模块加一条：用例表对照 core 的 `config::is_valid_model_name`）
- Modify: 五个翻译文件（新增 `bindings` 命名空间；在 Task 5 建好的 `permission` 里补每档的说明 `permission.full_hint` 等；执行者名沿用 `executor`）
- Test: `app/src/pages/Bindings.test.tsx`、`app/src/pages/BindingDialog.test.tsx`、`app/src/lib/model-name.test.ts`

**Interfaces:**
- Produces: `BindingForm({ initial?, onSaved })`（对话框与首次引导共用的表单主体）；`isValidModelName(text: string): boolean`（与 Rust 的 `config::is_valid_model_name` 同规则：1–100 个 ASCII 字母、数字、`.`、`_`、`:`、`-`，不以 `-` 开头；两边读同一张用例表 `model-name-cases.json`）。
- Consumes: Task 5 的钩子；`useBindings`、`useProjects`、`useSaveBinding`、`useRemoveBinding`、`useDoctor`（检查结果）、`useSettings`（默认执行者与档位、`full_access_acknowledged`）、`useSaveSettings`；三种状态组件、`DetailLine`、`describeError`；Task 6 的 `actions.confirm`；Task 4 假后端对非绝对目录的拒绝。

页面与表单规格：

- 列表：每个绑定一张卡片（`<section aria-labelledby>`，名称为项目名，测试用 `within` 在卡片里找）：项目名、目录（等宽，过长时中间省略，悬停看全文）、执行者（`executor.default` 这类写法：「默认（Claude Code）」）、权限档位（`permission.*`）、模型（没有时「沿用执行者的配置」）、提示词覆盖（有时显示「有」，用 `Collapsible` 展开）、检查结果（来自最近一次环境检查：目录存在、执行者就绪；没有检查过时显示「重新检查」按钮）、编辑与删除（删除要确认，确认按钮复用 `actions.confirm`）。右上角「添加绑定」。加载：`LoadingState`；没有绑定时 `EmptyState`（`bindings.empty`，按钮「添加第一个绑定」）；`bindings()` 读取失败时 `ErrorState`（标题 `bindings.load_failed`，带重试）。
- 表单字段：
  1. Linear 项目（`Select`，选项来自 `projects()`；已绑定的项目在新增时禁用并注明；加载失败时显示错误与重试）；
  2. 本地目录（只读输入框＋「选择…」按钮调 `chooseDirectory()`；目录必须是绝对路径，由引擎校验：面板不另做判断，引擎拒绝时按下面「保存失败」显示）；
  3. 执行者（`Select`：默认 / Claude Code / Codex）；
  4. 权限档位（`Select`：默认 / 完全放开 / 自动审查 / 按个人设置；每档下方一行说明，取自设计 6.1：完全放开 = AI 可以在本机执行任何命令；自动审查 = 执行者自动审查每一步，Codex 明显更慢；按个人设置 = Claude 只做设置里预先允许的事，Linear 工具和 git 要预先允许，否则会失败；Codex 沿用 `~/.codex/config.toml`）；
  5. 模型（输入框，可空；非法时字段下方提示 `bindings.model_invalid`：「只能用字母、数字和 . _ : -，例如 sonnet 或 gpt-5-codex；带方括号的名字（如 sonnet[1m]）不支持」）；
  6. 提示词覆盖（多行，可空；说明「留空用设置里的默认模板；固定规则始终生效」）。
- 保存：`id` 新建时取项目 `id`；生效的档位是「完全放开」（选中完全放开，或选「默认」而默认档位是完全放开）且 `settings.full_access_acknowledged` 为假时，先弹 `FullAccessWarning`（第 14 节的风险说明：AI 可以在本机执行任意命令、使用你的全部凭据，约束只有提示词固定规则和仓库的 AGENTS.md；可以改用较低档位），确认后先 `saveSettings({...settings, full_access_acknowledged: true})` 再保存绑定；取消则不保存。保存失败（例如引擎校验报目录不是绝对路径）在表单顶部显示 `Alert`：标题 `actions.failed.save_binding`（「Could not save the binding」），下面 `DetailLine` 是引擎的原文。

- [ ] **Step 1: 失败测试**

`app/src/lib/model-name-cases.json`（100 个 `x` 合法，101 个不合法）：

```json
[
  {"name": "sonnet", "valid": true},
  {"name": "claude-sonnet-5-5", "valid": true},
  {"name": "gpt-5-codex", "valid": true},
  {"name": "o3:high", "valid": true},
  {"name": "a.b_c", "valid": true},
  {"name": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "valid": true},
  {"name": "", "valid": false},
  {"name": "-rf", "valid": false},
  {"name": "sonnet[1m]", "valid": false},
  {"name": "a b", "valid": false},
  {"name": "模型", "valid": false},
  {"name": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "valid": false}
]
```

`app/src/lib/model-name.test.ts`：

```ts
import { expect, test } from "vitest";
import cases from "./model-name-cases.json";
import { isValidModelName } from "./model-name";

test.each(cases)("$name: valid $valid", ({ name, valid }) => {
  expect(isValidModelName(name)).toBe(valid);
});
```

`app/src-tauri/src/commands.rs` 测试模块加（同一张表对照 core 的规则）：

```rust
    /// One row of app/src/lib/model-name-cases.json, which the panel's `isValidModelName` test reads.
    #[derive(Deserialize)]
    struct ModelNameCase {
        name: String,
        valid: bool,
    }

    #[test]
    fn model_name_cases_follow_core() {
        let cases: Vec<ModelNameCase> = serde_json::from_str(include_str!("../../src/lib/model-name-cases.json"))
            .expect("model-name-cases.json is valid");
        for case in cases {
            assert_eq!(issuerelay_core::config::is_valid_model_name(&case.name), case.valid, "{:?}", case.name);
        }
    }
```

`app/src/pages/BindingDialog.test.tsx`：

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { BindingForm } from "./BindingDialog";

test("binding a project the first time with full access shows the warning once", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await userEvent.click(await screen.findByRole("combobox", { name: "Linear project" }));
  await userEvent.click(await screen.findByRole("option", { name: "Demo Project" }));
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText(/run any command on this computer/)).toBeInTheDocument();
  expect(saveBinding).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "I understand" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledWith(expect.objectContaining({ project_id: "proj-1", directory: "~/code/new-project" })));
  expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({ full_access_acknowledged: true }));
});

test("an invalid model name blocks saving with a hint", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  // user-event reads "[" as the start of a key name; "[[" types one "[".
  await userEvent.type(await screen.findByRole("textbox", { name: "Model" }), "sonnet[[1m]");
  expect(screen.getByRole("textbox", { name: "Model" })).toHaveValue("sonnet[1m]");
  expect(screen.getByText(/names with brackets/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("a folder the engine refuses is explained at the top of the form", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.chooseDirectory = vi.fn().mockResolvedValue("code/new-project");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await userEvent.click(await screen.findByRole("combobox", { name: "Linear project" }));
  await userEvent.click(await screen.findByRole("option", { name: "Docs Site" }));
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText("Could not save the binding")).toBeInTheDocument();
  expect(within(alert).getByText('bindings[2].directory must be an absolute path: "code/new-project"')).toBeInTheDocument();
  expect(onSaved).not.toHaveBeenCalled();
});
```

`app/src/pages/Bindings.test.tsx`：

```tsx
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Bindings from "./Bindings";

test("bindings list their folder, executor and permission", async () => {
  await renderWithProviders(<Bindings />);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(demo).getByText("~/code/demo-app")).toBeInTheDocument();
  expect(within(demo).getByText("Full access")).toBeInTheDocument();
  expect(within(demo).getByText("Default (Claude Code)")).toBeInTheDocument();
  const mobile = screen.getByRole("region", { name: "Demo Mobile" });
  expect(within(mobile).getByText("Automatic review")).toBeInTheDocument();
  expect(within(mobile).getByText("gpt-5-codex")).toBeInTheDocument();
});

test("removing a binding asks first", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const remove = vi.spyOn(backend, "removeBinding");
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  expect(remove).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(remove).toHaveBeenCalledWith("proj-1");
});

test("when the bindings cannot be read the page says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn().mockRejectedValue(new BackendError("config.toml was changed while IssueRelay was running"));
  await renderWithProviders(<Bindings />, { backend });
  expect(await screen.findByText("Could not read the bindings")).toBeInTheDocument();
  expect(screen.getByText("config.toml was changed while IssueRelay was running")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

test("no bindings yet invites the first one", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  await renderWithProviders(<Bindings />, { backend });
  expect(await screen.findByRole("button", { name: "Add your first binding" })).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按规格实现（Radix 在 jsdom 里需要的桩 Task 1 已放进 `src/test-setup.ts`，本任务不再改它）。英文文案：`bindings.load_failed` = `Could not read the bindings`、`bindings.empty` = `No bindings yet`、`bindings.add_first` = `Add your first binding`、`bindings.project` = `Linear project`、`bindings.choose` = `Choose…`、`bindings.model` = `Model`、`bindings.remove` = `Remove`、`bindings.model_invalid` = `Only letters, digits and . _ : - are allowed, for example sonnet or gpt-5-codex; names with brackets (such as sonnet[1m]) are not supported`；`FullAccessWarning` 的确认按钮 `bindings.full_access_ok` = `I understand`，说明里含 `run any command on this computer`。

Run: `pnpm --dir app test && pnpm --dir app lint` 与 `cargo test -p issuerelay-app commands`
Expected: PASS。浏览器里看列表、新建、编辑、风险提示，截图附在报告里。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app
git commit -m "feat(app): add project bindings with the full-access warning"
```

---

### Task 9: 环境检查、设置与使用说明

设计第 8 节（环境检查与修复按钮）、9.2 节（设置、使用说明两页的内容）、10.3 节（在设置里一键注册或移除 MCP 控制服务）、第 7 节（高级设置里改用个人 API key）。

**Files:**
- Modify: `app/src/pages/Doctor.tsx`、`app/src/pages/Settings.tsx`、`app/src/pages/Help.tsx`
- Create: `app/src/lib/mask.ts`（与 Rust `env::mask_credentials` 同规则：`://` 之前不含 `@` 和 `/` 的部分算协议头，其后到第一个 `/` 为止、最后一个 `@` 之前的用户信息换成 `***`；没有协议头时从开头算）、`app/src/lib/mask-cases.json`（打码用例表：TS 测试与 Rust 测试都读）、`app/src/components/ExecutorCheck.tsx`（引导页复用）、`app/src/components/WorkspaceCheck.tsx`（引导页复用）
- Modify: `app/src-tauri/src/commands.rs`（测试模块加一条：打码用例表对照 core 的 `env::mask_credentials`）
- Modify: 五个翻译文件（`doctor`、`settings`、`help` 命名空间；`help.example.*` 的键 Task 10 也读）
- Test: `app/src/pages/Doctor.test.tsx`、`app/src/pages/Settings.test.tsx`、`app/src/pages/Help.test.tsx`、`app/src/lib/mask.test.ts`

**Interfaces:**
- Produces: `ExecutorCheck({ report, onFixed })`、`WorkspaceCheck({ report, labelName })`、`maskCredentials(url: string): string`（与 core 读同一张用例表 `mask-cases.json`）；翻译键 `help.example.good_title`、`help.example.good_body`、`help.example.vague_title`、`help.example.vague_body`（Task 10 第 6 步读前两个）；环境检查页的「重新走一遍引导」链接（`doctor.rerun_guide`，到 `/onboarding`）。
- Consumes: Task 5 的钩子（`useDoctor`、`useSetupAction`、`useControlServers`、`useLinearServerNamed`、`useSettings`、`useSaveSettings`、`useAutostart`、`useSetAutostart`、`useLinearLogin`、`useLinearApiKey`、`useLinearLogout`、`useLinearReconnect`、`useAppInfo`）、三种状态组件、`DetailLine`、`StatusBadge`；Task 6 的 `LinearStatus`；Task 8 的 `FullAccessWarning`。

页面规格：

**环境检查**（`/doctor`）——打开页面时自动跑一次 `doctor()`（它会现查，最多约一分钟），页头有「重新检查」；检查中显示进度说明 `doctor.checking`（「正在检查 Linear、Claude Code 和 Codex，最多约一分钟」）；`doctor()` 失败时整页 `ErrorState`（标题 `doctor.failed`，带重试）。页头右侧另有链接「重新走一遍引导」（`doctor.rerun_guide`，到 `/onboarding`，Task 10 从第 1 步开始并显示各步的真实状态）。五个区块，各是一个 `<section aria-labelledby>`：

1. Linear：复用 Task 6 的 `LinearStatus manage`（未登录 → 登录；连接失败 → 重试连接；接口已变化 → 说明需要升级；已连接 → 授权人，另有重新授权与退出登录），不另写一套状态判断；环境检查报了 `linear_error` 时，在下面加标题 `doctor.linear_check_failed` 与它的 `DetailLine`。
2. 工作区：标签 `{{label}}` 是否存在；已检查的团队（`teams_checked`）与缺失的状态（`missing_states`）；缺失时说明在 Linear 的「设置 → 团队 → 工作流」里添加，并提供打开 `https://linear.app/` 的按钮（程序不代为创建）。没有工作区报告（`workspace` 为 `null`：还没连上 Linear）时，这一块是空状态 `doctor.workspace_unchecked`（「Connect to Linear first to check the workspace」）。
3. 执行者：Claude Code、Codex 各一张 `ExecutorCheck` 卡：已安装（版本）、已登录、Linear MCP 已连通三项，每项通过 / 不通过的图标与由布尔值得出的已翻译标题；`problems` 的原文作为 `DetailLine` 列在下面；修复按钮——没装：显示官方安装说明链接（`https://docs.anthropic.com/en/docs/claude-code/setup`、`https://developers.openai.com/codex/cli`，经 `openLinear` 打不开的外链，改为显示可复制的地址，不直接打开）；没登录：说明在终端运行 `claude` 并登录 / `codex login`；Linear MCP 没连通时先问 `useLinearServerNamed(executor)`：Linear 以一键授权用的名字注册（Claude `linear-server`、Codex `linear`）→ 显示「在浏览器授权」（`setupAction({ kind: "login_linear_mcp", executor })`，提示最多等 5 分钟）；否则显示「一键添加」（`setupAction({ kind: "add_linear_mcp", executor })`，按钮名 `Add Linear to Claude Code` / `Add Linear to Codex`），并说明「已用别的名字添加过的，在终端运行 `claude mcp list` 找到名字后运行 `claude mcp login <名字>`」（Codex 同理）；`setupAction` 调用一律带 `executor`；每次修复完成后自动重新检查。
4. 运行环境：PATH 来源、代理来源与代理值（`maskCredentials` 打码）；读取时的问题（`env.problems`）放在标题 `doctor.env_problems` 下作为 `DetailLine`；说明为什么需要它们（第 8 节最后一段的意思）。
5. 绑定：每个绑定的目录是否存在、执行者是否就绪（由布尔值得出的已翻译文字）；一个绑定都没有时是空状态 `doctor.no_bindings`（「No bindings yet」，链接到 `/bindings`）。

**设置**（`/settings`）——一张表单，分组卡片，底部固定「保存」；保存成功 toast，失败时 toast 标题 `actions.failed.save_settings`、说明为原文；语言切换保存后立即生效（含托盘）。加载：`LoadingState`；`settings()` 读取失败：`ErrorState`（标题 `settings.load_failed`，带重试）。

| 分组 | 字段 |
|---|---|
| 通用 | 界面语言（跟随系统 / 简体中文 / 繁體中文 / English / 日本語 / 한국어，存 `"system"` 或语言代码）；AI 评论语言（`Select`，名称 `AI comment language`：跟随界面语言，存 `"follow-ui"`（与 core `config::Settings` 的默认值一致）；或上述五种之一，存语言代码） |
| 监控 | 检查间隔（秒，≥ 5）、总并发上限（≥ 1）、单次运行时长上限（分钟，≥ 1）、无输出判定卡住（分钟，≥ 1） |
| 默认值 | 默认执行者、默认权限档位（选「完全放开」且未确认过时弹 Task 8 的 `FullAccessWarning`） |
| 提示词 | 默认模板（多行；说明固定规则始终生效、模板写团队流程，例如测试分支名、部署读回方式） |
| 通知 | 四个开关：开始运行、进入待验收、需要补充信息、失败 |
| 系统 | 开机自启（`setAutostart`，立即生效，不经「保存」）；自动更新（开关存进设置，M3 起生效，旁注 `settings.auto_update_note`） |
| MCP 控制服务 | Claude Code、Codex 各一行，按 `ControlServer`：`registered` 已注册 /`not_registered` 未注册 / `not_installed` 执行者未安装 / `unknown` 查不出（「Could not check」＋按钮「Check again」重查 `controlServers`，失败原因已在日志里）；按钮「注册」/「移除」（`setupAction({ kind: "register_control_server" / "remove_control_server", executor })`）；说明 `settings.control_server_note`：「按用户范围注册，无人值守的运行也会加载它；它的工具里有『暂停监控』」；命令行程序没找到时说明（开发版需先 `cargo build -p issuerelay-cli`） |
| 高级 | 触发标签名与四个状态名；个人 API key（密码输入框＋「使用这个 key」，成功后显示「已改用 API key」；浏览器登录正在等待时（`useLinearLoginWaiting()`）「使用这个 key」不可用，旁边一行 `settings.api_key_after_login`「Finish or cancel the browser login first」——core 一次只做一个登录改动，这时点它只会排在登录后面最长 10 分钟）；日志保留天数与总大小（MB） |

字段校验与后端一致（数值下限、名字非空且不含引号与控制字符），不合法时字段下方提示，「保存」禁用。

**使用说明**（`/help`）——内容全部走翻译键，标签名与状态名取自 `useSettings()`（读取中先用默认名；读取失败时照样显示默认名，并在页首加一行淡色说明 `help.defaults_note`，这就是本页的错误状态）：触发条件（区块名 `help.trigger_title`，一个四项的列表：`help.conditions.assigned`「指派给自己」、`help.conditions.label`「带 {{label}} 标签」、`help.conditions.todo`「状态为 {{todo}}」、`help.conditions.bound`「所属项目已绑定本地目录，且执行者已就绪」）；状态流转（Todo → In Progress → In Testing → 面板上标记完成 → Done，配八种本地状态的图例，用 `StatusBadge`）；需求的写法示例（`help.example.good_title` / `good_body` 一条清楚的需求，`help.example.vague_title` / `vague_body` 一条会被问回来的含糊需求）；常见问题（为什么没被接走、三档权限的区别、需要代理时怎么办、怎样让 Claude / Codex 调用 IssueRelay 的 MCP 控制服务（按用户范围注册、无人值守的运行也加载、工具里有暂停）、日志在哪里）。

- [ ] **Step 1: 失败测试**

`app/src/lib/mask-cases.json`（core `env::mask_credentials` 测试里的各种写法）：

```json
[
  {"value": "http://jane:s3cret@proxy.example:8080", "masked": "http://***@proxy.example:8080"},
  {"value": "jane:s3cret@proxy.example:8080", "masked": "***@proxy.example:8080"},
  {"value": " http://jane:s3cret@proxy.example:8080", "masked": " http://***@proxy.example:8080"},
  {"value": "HTTPS_PROXY=http://jane:s3cret@proxy.example:8080", "masked": "HTTPS_PROXY=http://***@proxy.example:8080"},
  {"value": "http://jane:p@ss@proxy.example:8080", "masked": "http://***@proxy.example:8080"},
  {"value": "http://token@proxy.example:8080/path@x", "masked": "http://***@proxy.example:8080/path@x"},
  {"value": "http://proxy.example:8080", "masked": "http://proxy.example:8080"},
  {"value": "localhost,127.0.0.1", "masked": "localhost,127.0.0.1"}
]
```

`app/src/lib/mask.test.ts`：

```ts
import { expect, test } from "vitest";
import cases from "./mask-cases.json";
import { maskCredentials } from "./mask";

test.each(cases)("$value is shown as $masked, as the engine masks it", ({ value, masked }) => {
  expect(maskCredentials(value)).toBe(masked);
});
```

`app/src-tauri/src/commands.rs` 测试模块加：

```rust
    /// One row of app/src/lib/mask-cases.json, which the panel's `maskCredentials` test reads.
    #[derive(Deserialize)]
    struct MaskCase {
        value: String,
        masked: String,
    }

    #[test]
    fn mask_cases_follow_core() {
        let cases: Vec<MaskCase> = serde_json::from_str(include_str!("../../src/lib/mask-cases.json"))
            .expect("mask-cases.json is valid");
        for case in cases {
            assert_eq!(issuerelay_core::env::mask_credentials(&case.value), case.masked, "{:?}", case.value);
        }
    }
```

`app/src/pages/Doctor.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Doctor from "./Doctor";

test("the check shows each executor and offers the fix it needs", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText(/2\.1\.285/)).toBeInTheDocument();
  // The engine's problem text is a detail line under a translated check.
  expect(screen.getByText("codex is not logged in")).toBeInTheDocument();
  expect(screen.getByText(/codex login/)).toBeInTheDocument();
  expect(screen.getByText("HTTPS_PROXY=http://127.0.0.1:7890")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Run the first-run guide again" })).toBeInTheDocument();
  expect(setup).not.toHaveBeenCalled();
});

test("Linear registered under another name gets the add button and the manual login", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  report.executors[0] = { ...report.executors[0], linear_mcp: false, problems: ["Linear MCP is not configured or not authorized in claude"] };
  const doctor = vi.spyOn(backend, "doctor").mockResolvedValueOnce(report);
  backend.linearServerNamed = vi.fn().mockResolvedValue(false);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Add Linear to Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "add_linear_mcp", executor: "claude" });
  expect(screen.queryByRole("button", { name: "Authorize Linear in Claude Code" })).not.toBeInTheDocument();
  expect(screen.getByText(/claude mcp list/)).toBeInTheDocument();
  await waitFor(() => expect(doctor).toHaveBeenCalledTimes(2));
});

test("Linear registered under the default name gets the one-click browser login", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  report.executors[0] = { ...report.executors[0], linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] };
  vi.spyOn(backend, "doctor").mockResolvedValueOnce(report);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "login_linear_mcp", executor: "claude" });
});

test("when the check cannot run the page says so and offers a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.doctor = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});
```

`app/src/pages/Settings.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Settings from "./Settings";

test("settings save what was changed and keep the rest", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await userEvent.clear(interval);
  await userEvent.type(interval, "60");
  await userEvent.click(screen.getByRole("switch", { name: "Run failed" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        poll_interval_secs: 60,
        comment_language: "follow-ui",
        notifications: expect.objectContaining({ failed: false, run_started: true }),
      }),
    ),
  );
  expect(screen.getByRole("combobox", { name: "AI comment language" })).toHaveTextContent("Follow the interface language");
});

test("values the engine would refuse block saving", async () => {
  await renderWithProviders(<Settings />);
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await userEvent.clear(interval);
  await userEvent.type(interval, "2");
  expect(screen.getByText(/at least 5/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("autostart switches at once and the MCP control server can be registered", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const autostart = vi.spyOn(backend, "setAutostart");
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Start at login" }));
  expect(autostart).toHaveBeenCalledWith(true);
  await userEvent.click(screen.getByRole("button", { name: "Register with Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "register_control_server", executor: "claude" });
});

test("a registration that could not be checked says so and can be checked again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const check = vi.spyOn(backend, "controlServers").mockResolvedValueOnce({ claude: "unknown", codex: "not_registered" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not check")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Check again" }));
  await waitFor(() => expect(check).toHaveBeenCalledTimes(2));
  expect(await screen.findByRole("button", { name: "Register with Claude Code" })).toBeInTheDocument();
});

test("a rejected API key says so and changes nothing", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.type(await screen.findByLabelText("Personal API key"), "lin_api_bad");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Could not use the API key")).toBeInTheDocument();
  expect(screen.getByText(/did not accept the API key/)).toBeInTheDocument();
});

test("the control server note says unattended runs load it too", async () => {
  await renderWithProviders(<Settings />);
  expect(await screen.findByText(/unattended runs load it too/)).toBeInTheDocument();
});

test("when the settings cannot be read the page says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("cannot read config.toml"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});
```

`app/src/pages/Help.test.tsx`：

```tsx
import { screen, within } from "@testing-library/react";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Help from "./Help";

test("the help page states the trigger conditions in full", async () => {
  await renderWithProviders(<Help />, { lang: "zh-CN" });
  // The page repeats ai-ready and Todo in its examples and FAQ, so the four conditions are read
  // from their own list.
  const conditions = await screen.findByRole("region", { name: "触发条件" });
  const items = within(conditions).getAllByRole("listitem");
  expect(items).toHaveLength(4);
  expect(items[0]).toHaveTextContent("指派给自己");
  expect(items[1]).toHaveTextContent("ai-ready");
  expect(items[2]).toHaveTextContent("Todo");
  expect(items[3]).toHaveTextContent("已绑定");
});

test("without the settings the page still helps, with the default names", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Help />, { backend });
  expect(await screen.findByText("Showing the default names: the settings could not be read.")).toBeInTheDocument();
  const conditions = screen.getByRole("region", { name: "Trigger conditions" });
  expect(within(conditions).getAllByRole("listitem")[1]).toHaveTextContent("ai-ready");
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按规格实现。外部安装说明链接只作为可复制文本显示（`open_linear` 只开 Linear 的链接，这是有意的限制）。英文文案：`doctor.failed` = `Could not run the environment check`、`doctor.rerun_guide` = `Run the first-run guide again`、`doctor.linear_check_failed` = `The check of Linear failed`、`doctor.add_linear` = `Add Linear to {{executor}}`、`doctor.login_linear` = `Authorize Linear in {{executor}}`、`doctor.other_name` = `Added Linear under another name? Run {{list}} in a terminal to find it, then {{login}}.`（`list` 为 `claude mcp list` / `codex mcp list`，`login` 为 `claude mcp login <name>` / `codex mcp login <name>`）、`doctor.login_hint` = `Run {{command}} in a terminal and log in`（`command` 为 `claude` / `codex login`）、`doctor.workspace_unchecked` = `Connect to Linear first to check the workspace`、`doctor.no_bindings` = `No bindings yet`、`doctor.env_problems` = `Reading the environment had problems`、`settings.load_failed` = `Could not read the settings`、`settings.language` = `Interface language`、`settings.follow_system` = `Follow the system`、`settings.comment_language` = `AI comment language`、`settings.follow_ui` = `Follow the interface language`、`settings.poll_interval` = `Check every (seconds)`、`settings.min_value` = `Must be at least {{min}}`、`settings.notify_run_started` = `Run started`、`settings.notify_awaiting_acceptance` = `Ready for acceptance`、`settings.notify_needs_info` = `Needs information`、`settings.notify_failed` = `Run failed`、`settings.autostart` = `Start at login`、`settings.register` = `Register with {{executor}}`、`settings.remove_server` = `Remove from {{executor}}`、`settings.control_unknown` = `Could not check`、`settings.check_again` = `Check again`、`settings.api_key` = `Personal API key`、`settings.use_key` = `Use this key`、`settings.api_key_used` = `Using the API key now`、`settings.api_key_after_login` = `Finish or cancel the browser login first`、语言名 `languages.zh-CN` = `简体中文`、`languages.zh-TW` = `繁體中文`、`languages.en` = `English`、`languages.ja` = `日本語`、`languages.ko` = `한국어`（各语言用自己的名字，五个文件里相同；Task 10 的语言步骤也用）、`settings.control_server_note` = `It is registered for your user, so unattended runs load it too; its tools can pause monitoring.`、`help.trigger_title` = `Trigger conditions`、`help.conditions.assigned` = `The issue is assigned to you`、`help.conditions.label` = `It has the {{label}} label`、`help.conditions.todo` = `Its status is {{todo}}`、`help.conditions.bound` = `Its project is bound to a local folder and the executor is ready`、`help.defaults_note` = `Showing the default names: the settings could not be read.`（示例需求的英文里不要再出现 `assigned to you`，Task 10 按它找第 6 步）；`zh-CN` 的 `help.trigger_title` 为「触发条件」，四个条件的文案见上面的页面规格。

Run: `pnpm --dir app test && pnpm --dir app lint` 与 `cargo test -p issuerelay-app commands`
Expected: PASS。浏览器里看三页的浅色与深色，截图附在报告里。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app
git commit -m "feat(app): add the environment check, settings and help pages"
```

---

### Task 10: 首次引导

设计第 8 节：首次打开面板时进入引导，六步：选择界面语言；连接 Linear（浏览器授权），显示授权人；检查工作区；检查执行者（含一键添加 Linear MCP 与浏览器授权）；绑定第一个项目（首次选完全放开时提示风险）；使用说明（明确写出四个触发条件并附示例）。之后可在「环境检查」页随时重跑检查。

**Files:**
- Modify: `app/src/pages/onboarding/Onboarding.tsx`
- Create: `app/src/pages/onboarding/{LanguageStep,LinearStep,WorkspaceStep,ExecutorsStep,BindingStep,FinishStep}.tsx`
- Modify: 五个翻译文件（`onboarding` 命名空间）
- Test: `app/src/pages/onboarding/Onboarding.test.tsx`

**Interfaces:**
- Consumes: Task 8 的 `BindingForm`、Task 9 的 `ExecutorCheck`、`WorkspaceCheck`、`help.example.good_title` / `good_body`、环境检查页的「重新走一遍引导」链接；Task 5 的钩子、三种状态组件；Task 1 的 `radio-group` 组件。

规格：

- 整页布局（不带侧栏）：左侧竖排步骤列表（当前步高亮，已完成打勾，可点回已完成的步骤），右侧当前步内容，底部「上一步 / 下一步」。顶部应用名与图标。
- 第 1 步：语言单选（shadcn `RadioGroup`：跟随系统（`settings.follow_system`）＋五种语言（`languages.*`，各用自己的名字），选中立即切换界面并保存设置。
- 第 2 步：首次连接还在进行（`linear.kind` 为 `connecting`，AR51）时显示「正在检查与 Linear 的连接…」（`onboarding.checking`），没有授权按钮，下一步不可用，连接结束（`Status` 事件刷新状态）后按结果显示；未连接时「在浏览器里授权 Linear」（`useLinearLogin`）；等浏览器期间显示 Task 6 的等待文案 `linear.waiting_browser` 与「Cancel」（`linear.cancel_login`，调 `useLinearLogin` 的 `cancel()` 丢掉这次登录，回到授权按钮，不提示失败）；已连接时显示授权人；高级选项里可改用个人 API key（按钮 `onboarding.use_api_key`「Use a personal API key」，点开后是与设置页相同的输入框），浏览器登录等待期间（`useLinearLoginWaiting()`）这个按钮不可用。连上之前不能进下一步。
- 第 3 步：`WorkspaceCheck`（来自 `doctor()` 的工作区部分）；检查中 `LoadingState` 并说明最多约一分钟；`doctor()` 失败时 `ErrorState`（标题 `doctor.failed`，带重试），仍可「我稍后再加」继续；缺标签或状态时显示添加方法，但允许继续（「我稍后再加」）。
- 第 4 步：两个 `ExecutorCheck`（一键授权只在 Linear 以默认名字注册时出现，见 Task 9）；至少一个执行者就绪才能进下一步；都没就绪时说明需要先装好并登录 Claude Code 或 Codex；`doctor()` 失败时同第 3 步的错误状态，不能进下一步。
- 第 5 步：嵌入 `BindingForm`；保存成功后可「再加一个」或进下一步；也允许「稍后再绑定」。
- 第 6 步：四个触发条件、一条示例需求（`help.example.good_title` 与 `help.example.good_body`）、状态流转图例；一段说明 `onboarding.control_server_note`：「可以在设置里让 Claude Code / Codex 调用 IssueRelay 的 MCP 控制服务；它按用户范围注册，无人值守的运行也会加载它，它的工具里有『暂停监控』。」；按钮「开始使用」：`saveSettings({...settings, onboarding_completed: true})` 后跳到概览；保存失败时 toast（`actions.failed.save_settings` ＋原文），停在本步。
- 引导完成后再进入 `/onboarding`（例如从环境检查页的「重新走一遍引导」）时，从第 1 步开始，但各步显示当前真实状态。

- [ ] **Step 1: 失败测试**

`app/src/pages/onboarding/Onboarding.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { vi } from "vitest";
import { type Backend, BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Onboarding from "./Onboarding";

/** The guide (fresh scenario unless told otherwise); `prepare` changes the mock before the first render. */
async function start(prepare: (backend: Backend) => Promise<void> = async () => {}, scenario: "fresh" | "connecting" = "fresh") {
  const backend = createMockBackend(scenario, { latencyMs: 0 });
  await prepare(backend);
  const result = await renderWithProviders(
    <Routes>
      <Route path="/onboarding" element={<Onboarding />} />
      <Route path="/" element={<p>overview page</p>} />
    </Routes>,
    { backend, route: "/onboarding" },
  );
  return { ...result, backend };
}

test("the guide walks from language to the first binding and finishes", async () => {
  const { backend } = await start();
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await userEvent.click(await screen.findByRole("radio", { name: "English" }));
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Authorize Linear in the browser" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  // The workspace step may name the label more than once (the check and how to add it).
  expect((await screen.findAllByText(/ai-ready/)).length).toBeGreaterThan(0);
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  expect(await screen.findByText("Claude Code")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  await userEvent.click(await screen.findByRole("button", { name: "Bind later" }));
  expect(await screen.findByText(/assigned to you/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Start using IssueRelay" }));
  await waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ onboarding_completed: true })));
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});

test("without a ready executor the guide does not move on", async () => {
  await start(async (backend) => {
    await backend.linearLogin();
    const report = await backend.doctor();
    vi.spyOn(backend, "doctor").mockResolvedValue({
      ...report,
      executors: report.executors.map((e) => ({ ...e, logged_in: false, problems: ["not logged in"] })),
    });
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" })); // language → Linear
  // A disabled Next ignores the click: wait until the Linear step shows the connection.
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" })); // Linear → workspace
  await userEvent.click(await screen.findByRole("button", { name: "Next" })); // workspace → executors
  expect(await screen.findByText(/Install and log in to Claude Code or Codex first/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

// core takes one change of the login at a time: while the browser login waits, the key waits too,
// and the login can be given up.
test("a login that waits for the browser can be cancelled, and the API key waits for it", async () => {
  let cancel = () => {};
  const { backend } = await start(async (backend) => {
    backend.linearLogin = vi.fn(
      () =>
        new Promise<never>((_, reject) => {
          cancel = () => reject(new BackendError("login_cancelled"));
        }),
    );
    backend.linearCancelLogin = vi.fn(async () => cancel());
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in the browser" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use a personal API key" })).toBeDisabled();

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(backend.linearCancelLogin).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("button", { name: "Authorize Linear in the browser" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use a personal API key" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("while the first connection is under way the Linear step is checking", async () => {
  await start(async () => {}, "connecting");
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  expect(await screen.findByText("Checking the connection to Linear…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Authorize Linear in the browser" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("a check that fails is shown with a retry, and the workspace step can be skipped", async () => {
  await start(async (backend) => {
    await backend.linearLogin();
    backend.doctor = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "I'll add them later" })).toBeEnabled();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按规格实现。英文文案：`onboarding.next` = `Next`、`onboarding.checking` = `Checking the connection to Linear…`、`onboarding.authorize` = `Authorize Linear in the browser`、`onboarding.use_api_key` = `Use a personal API key`、`onboarding.later` = `I'll add them later`、`onboarding.bind_later` = `Bind later`、`onboarding.no_executor` = `Install and log in to Claude Code or Codex first`、`onboarding.start` = `Start using IssueRelay`；第 6 步的触发条件沿用 Task 9 的 `help.conditions.*`（英文含 `assigned to you`）；语言名用 Task 9 的 `languages.*`（测试里的 `English`）。

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS。浏览器里用 `http://localhost:1420/?mock=fresh#/onboarding` 走一遍，每一步截图附在报告里。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app
git commit -m "feat(app): add the first-run guide"
```

---

### Task 11: 图标（三套方案、选定、全部尺寸）

设计第 9.6 节：应用图标（安装包、程序坞 / 任务栏、面板窗口、关于页；macOS 图标集、Windows 多尺寸 `.ico`）；托盘图标（macOS 单色模板图、Windows 彩色小图；空闲、运行中、已暂停、出错四种状态）；先出两到三套方案选定，再出全部尺寸；源文件（SVG）放仓库，各平台尺寸由脚本生成。用户 2026-10-13 要求自行完成：由主智能体在三套方案里选定一套（记为裁决），方案预览图一并给用户，用户可以之后改选。

**Files:**
- Create: `app/icons-src/option-a/`、`option-b/`、`option-c/`（每套：`app.svg`，`tray-mono.svg`，`tray-color.svg`，各状态的角标在脚本里叠加）、`app/icons-src/README.md`（三套的设计说明）
- Create: `app/scripts/icons.mjs`（用 `@resvg/resvg-js` 渲染 PNG；`--preview` 出三套对比图；`--option <a|b|c>` 出选定方案的全部文件）
- Create（脚本生成后提交）：`app/src-tauri/icons/*`（`pnpm tauri icon` 由 1024px PNG 生成的桌面应用图标；它顺带写出的移动端目录删掉、不提交）、`app/src-tauri/icons/tray/macos/{idle,running,paused,problem}.png`（macOS 模板图 44×44，纯黑加透明度）、`app/public/icon.svg`（面板侧栏与关于页用）、`docs/design/icons/preview.png`（三套对比）
- Modify（脚本生成后提交）：`app/src-tauri/icons/tray/{idle,running,paused,problem}.png`（Task 3 放的占位图，换成 Windows 彩色 32×32）
- Modify: `app/src-tauri/src/tray.rs`（macOS 用 `icons/tray/macos/*`，Windows 用 `icons/tray/*`）、`app/package.json`（`"icons": "node scripts/icons.mjs"`）、`app/pnpm-lock.yaml`、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`（登记 `docs/design/icons/` 与 `app/icons-src/`）、`docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 9.6 节与第 18 节第 1 条）
- 渲染出的 1024px PNG 在 `app/icons-src/build/`，Task 1 已把它加进 `.gitignore`。

**Interfaces:**
- Produces: `pnpm --dir app icons --preview`、`pnpm --dir app icons --option a`。
- Consumes: Task 3 的 `tray::icon`。

设计约束：

- 三套方案主题都是「把需求交接给 AI 执行」：A「接力」——圆角方块上一枚斜放的接力棒与运动线；B「交接」——两个圆点由一条带箭头的弧线相连；C「工单＋启动」——一张工单卡片里一个向右的三角（运行）。主色与面板一致（靛蓝 `oklch(0.51 0.21 275)` 附近的渐变），最多三种颜色，形状简单，16px 时仍可辨认。
- macOS 应用图标遵循系统圆角方形网格（1024 画布、824 圆角方形居中、圆角约 185），Windows 用同一图形，不加圆角方形底时也要好看。
- 托盘：同一字形的单色版；四种状态用右下角小角标区分——空闲（无角标）、运行中（实心圆点）、已暂停（两条竖杠）、出错（感叹号）；macOS 模板图只用黑色加透明度；Windows 彩色版角标用状态色（运行中蓝、已暂停灰、出错红）。
- 预览图 `docs/design/icons/preview.png`：三套各一行，依次是 1024、128、32、16 的应用图标，以及四种托盘状态在浅色、深色菜单栏背景上的样子。

- [ ] **Step 1: 画三套 SVG、写脚本、出预览**

```bash
pnpm --dir app add -D @resvg/resvg-js
pnpm --dir app icons --preview
```

Expected: `docs/design/icons/preview.png` 生成。实现者把预览图路径写进报告，然后**停下来报告 DONE_WITH_CONCERNS**（不要自己选方案）：主智能体看过预览、记下裁决后，用 `SendMessage` 告诉实现者选哪一套，再继续 Step 2。

- [ ] **Step 2: 生成选定方案的全部文件**

```bash
pnpm --dir app icons --option <选定的方案>
pnpm --dir app tauri icon icons-src/build/app-1024.png
```

（`pnpm --dir app` 在 `app/` 里运行命令，输入路径相对 `app/`。`tauri icon` 生成 `icons/` 下的 `icon.icns`、`icon.ico`、各尺寸 PNG 与 Windows Store 图标；它若还写出 `icons/android/`、`icons/ios/` 这类移动端目录，删掉、不提交（本项目只做桌面）。脚本把渲染出的 1024px PNG 放在 git 忽略的 `app/icons-src/build/`；提交前 `git status` 确认没有它。）`tray.rs` 的 `icon()` 按平台读对应目录。面板侧栏左上角与关于信息用 `public/icon.svg`。

- [ ] **Step 3: 测试**

`tray.rs` 测试模块加：

```rust
    #[test]
    fn every_tray_state_has_a_distinct_valid_icon() {
        let icons: Vec<Vec<u8>> = [TrayState::Idle, TrayState::Running, TrayState::Paused, TrayState::Problem]
            .into_iter()
            .map(|state| icon(state).rgba().to_vec())
            .collect();
        for (i, a) in icons.iter().enumerate() {
            for b in &icons[i + 1..] {
                assert_ne!(a, b, "two states share a tray icon");
            }
        }
    }
```

Run: `cargo test -p issuerelay-app` 与 `pnpm --dir app tauri dev`（看菜单栏图标在浅色与深色菜单栏下都清楚，四种状态可分；把截图附在报告里）
Expected: PASS。

- [ ] **Step 4: 文档与提交**

- 设计第 9.6 节与第 18 节第 1 条补：「图标选定方案 <A/B/C>（M2 由主智能体代选，三套预览见 `docs/design/icons/preview.png`，用户可改选：改 `pnpm --dir app icons --option <x>` 后重新生成）。」`app/icons-src/README.md` 写明三套方案的含义与生成命令。`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 登记 `app/icons-src/`（图标源文件与说明）与 `docs/design/icons/`（方案预览）。

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs app/pnpm-lock.yaml
git commit -m "feat(app): add the app and tray icons"
```

---

### Task 12: 关键页面截图与截图比对

设计第 15 节「关键页面截图比对」、第 17 节 M2 完成标志「页面截图经用户确认」。截图只用假后端的虚构数据，可以进仓库；真实验收截图留在本机。

**Files:**
- Create: `app/playwright.config.ts`、`app/tests/screens.spec.ts`
- Modify: `app/src/lib/mock.ts`（`language` 选项）、`app/src/lib/api.ts`（读地址里的 `?lang=`）、`app/tsconfig.node.json`（`include` 加上 Playwright 的文件）、`app/pnpm-lock.yaml`
- Create（生成后提交）：`docs/gates/screenshots/m2/{zh-CN,en}-{light,dark}-{overview,issues,runs,run-detail,bindings,doctor,settings,help,onboarding-1,onboarding-2,onboarding-4,onboarding-6}.png`
- Modify: `app/package.json`（`"screenshots": "playwright test --update-snapshots"`、`"test:screens": "playwright test"`）、`README.md`（中英文各放两张截图：概览、运行详情）

**Interfaces:**
- Produces: `pnpm --dir app screenshots`（重新生成基准截图）、`pnpm --dir app test:screens`（与基准比对，允许 `maxDiffPixelRatio: 0.01`）。比对只在本机运行，不进 CI：字体随系统不同，CI 的机器上比对不稳定；它能防的是同一台机器上页面的意外变化，回归要靠实现者在本机跑。`createMockBackend(scenario, { latencyMs?, language? })`（`language` 写进假后端设置的 `language`）；浏览器里的 `?lang=zh-CN` 等参数传给它。Vitest 的 `include` 只认 `src/**/*.test.*`（Task 1），`tests/*.spec.ts` 不会被 `pnpm test` 载入。

- [ ] **Step 1: Playwright**

```bash
pnpm --dir app add -D @playwright/test
pnpm --dir app exec playwright install chromium
```

`app/tsconfig.node.json` 的 `include` 加上 `playwright.config.ts` 与 `tests/**/*.ts`（否则 `pnpm lint` 不检查它们的类型）。

`app/playwright.config.ts`：`webServer` 用 `pnpm build && pnpm preview --port 4173`，`use.viewport` 为 1100×720，`snapshotPathTemplate` 指向 `../docs/gates/screenshots/m2/{arg}{ext}`，两个项目：`light`（`colorScheme: "light"`）与 `dark`（`colorScheme: "dark"`）。

假后端支持语言参数。`mock.ts` 的 `MockOptions` 加 `language?: string`，构造时：

```ts
  let settings: Settings = {
    ...DEFAULT_SETTINGS,
    language: options.language ?? DEFAULT_SETTINGS.language,
    onboarding_completed: demo,
    full_access_acknowledged: demo,
  };
```

`api.ts` 的 `loadBackend()` 读地址参数：

```ts
/** `?lang=zh-CN` in a browser makes the mock's settings ask for that language (screenshots). */
function mockLanguage(): string | undefined {
  const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  return params.get("lang") ?? undefined;
}

export async function loadBackend(): Promise<Backend> {
  if (isTauri()) return tauriBackend;
  const { createMockBackend } = await import("./mock");
  return createMockBackend(mockScenario(), { language: mockLanguage() });
}
```

`app/tests/screens.spec.ts`：对每个语言（`zh-CN`、`en`，地址带 `?lang=<语言>`）、每个页面（地址用 `#/`、`#/issues`、`#/runs`、`#/runs/12`（假后端里运行中的 DEMO-12）、`#/bindings`、`#/doctor`、`#/settings`、`#/help`；引导页用 `?mock=fresh&lang=<语言>#/onboarding`：第 1 步截图；点「下一步」到第 2 步截图；点「在浏览器里授权 Linear」，等授权人出现（「下一步」此时才可用）后点「下一步」到第 3 步，再点「下一步」到第 4 步截图；点「下一步」到第 5 步，点「稍后再绑定」到第 6 步截图）等待加载状态消失后 `expect(page).toHaveScreenshot(\`${lang}-${theme}-${name}.png\`, { maxDiffPixelRatio: 0.01, animations: "disabled" })`。运行中任务的计时器在假后端里用固定时钟，截图稳定。

- [ ] **Step 2: 生成并检查**

Run: `pnpm --dir app screenshots`，再 `pnpm --dir app test:screens`
Expected: 生成 48 张基准图（2 语言 × 2 主题 × 12 屏），第二条命令全部通过。

实现者逐张检查：没有截断的文字、没有溢出、没有裸露的报错文本、深色下对比度足够；问题当场修页面并重新生成。报告里列出检查结果。

然后证明比对能失败（刚生成完基准就比对，本身不可能失败）：把 `styles.css` 里的一个设计变量（例如浅色的 `--primary`）临时改掉，跑 `pnpm --dir app test:screens`，Expected: 失败并列出差异图；改回后再跑，Expected: 全部通过。两次的结果写进报告。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过；`check-secrets` 不报截图（PNG 是二进制，`git grep -I` 跳过——截图里只有虚构数据）；`git status` 里没有 `app/test-results/`、`app/playwright-report/`（Task 1 已忽略）。

```bash
git add app docs README.md app/pnpm-lock.yaml
git commit -m "test(app): add key page screenshots and a local screenshot comparison"
```

---

### Task 13: M2 实机验收（macOS）

由主智能体在本机执行（AGENTS.md：实机验收只由主智能体在沙箱项目与沙箱仓库上做）。设计第 15 节：完整流程（发现 → 运行 → In Testing → 标记完成），另外制造一次失败和一次超时；结果写 `docs/gates/<日期>-m2-acceptance.md`。Windows 待实机（设计第 18 节第 4 条），待实机清单至少包括：Windows 上整个流程与托盘、通知、开机自启；第一部分 Task 8 的两条——登录命令（`mcp login`）在没有已开浏览器时启动的浏览器，可能随该命令的 Job Object 结束而被关掉；登录超时后，打印出的授权链接作废；第一部分最终修复波的一条——托盘程序运行期间安装执行者，再在「环境检查」页重查：PATH 从注册表重新读取，执行者被找到；以及第一部分计划末尾「评审遗留的小项」里其余待 Windows 实机的各条（提权运行的托盘程序与未提权的命令行）。

**需要用户的地方：** 桌面应用第一次读钥匙串里的 Linear 凭据时 macOS 可能弹出授权框，或需要在浏览器里重新授权 Linear——这两件事涉及用户的凭据，由用户点；主智能体只在那一步请用户操作，其余自行完成。

- [ ] **Step 1: 构建**：`pnpm --dir app tauri build --bundles app`（得到 `IssueRelay.app`；确认 `Contents/MacOS/` 里的主程序叫 `issuerelay-app`，即 `mainBinaryName` 生效）、`cargo build -p issuerelay-cli --release`，把命令行程序复制到 `IssueRelay.app/Contents/MacOS/issuerelay`（M3 由打包流程完成），复制后两个文件都在。
- [ ] **Step 2: 从访达启动**（不经终端，验证登录 shell 的 PATH 与代理）：首次引导走一遍；环境检查页确认 PATH 来源为登录 shell、代理来源与值（打码）；Claude、Codex 都就绪。另外退出托盘程序后在终端里运行一次 `cargo run -p issuerelay-devhost -- run`（Step 1 不构建 devhost），用 `issuerelay doctor` 确认 PATH 来源同样是登录 shell、读取没有卡到 10 秒上限（登录 shell 在自己的会话里运行，第一部分 `env::shell_vars` 的注释记着这项测量），再 Ctrl+C 停掉。
- [ ] **Step 3: 沙箱流程**：绑定沙箱项目（目录 `~/issuerelay-sandbox/demo`）；在沙箱项目建一条需求（Claude）→ 被接走（记时）→ 待验收 → 面板「标记完成」→ Linear 为 Done；换 Codex 再跑一条；四种通知各出现一次（开始运行、待验收、需要补充信息、失败）；托盘四种图标都出现过；暂停 / 恢复（托盘与面板）；设置里把运行时长上限改为 1 分钟制造一次超时（改完恢复 60）；一条故意做不到的需求制造失败；运行中停止一次；「重新运行」一次。
- [ ] **Step 4: 退出流程**：有任务在跑时托盘「退出」→ 提示 → 选「等待完成」→ 结束后自动退出；再来一次选「立即停止」→ 运行记为已取消、沙箱里没有残留进程（`ps -axo pid,ppid,pgid,command | grep -E 'claude|codex'`）、没有 `.git/index.lock`；第三次：选「等待完成」，在运行还没结束时关掉面板窗口（程序不退出，托盘还在），从托盘「退出」或「打开面板」把面板带回来，等待中的提示还在，改选「立即停止」→ 运行记为已取消、程序随即退出、没有残留进程（Review Focus 2）；第四次：有任务在跑时在面板里按 Cmd+Q → 同样先问，选「保持运行」后程序与运行都还在（这一条检验退出钩子对 `ExitRequested.code` 的假设，Task 3 的单元测试测不到它）；可行的话再试有任务在跑时注销或关机：系统发来退出请求 → 程序先问（macOS 可能提示某个应用阻止了注销），选「立即停止」后运行记为已取消、注销继续；做不到或结果不同就把原因写进报告。
- [ ] **Step 4a: 离线启动**（AR51）：退出程序，断开网络（或让 `mcp.linear.app` 不可达），再启动：面板立刻可用、终端里 `issuerelay status` 立刻有回应（宿主不再等 Linear）；面板与侧栏显示「正在连接 Linear…」，托盘不显示出问题；连接失败后面板显示连接失败与「重试」，托盘转为出问题；恢复网络后点「重试」→ 连上，面板不用刷新就显示授权人。
- [ ] **Step 5a: Linear 口径**：建一条中文标题的沙箱需求，确认提示词里的链接只保留到 `/issue/<编号>`，且这个链接在浏览器里能打开同一条需求（M1 验收 H2，M2 第一部分 Task 6）；若工作区有多个团队，确认环境检查只检查已绑定项目所属的团队，`teams_checked` 与实际一致（截图不进仓库）。
- [ ] **Step 5: 其他**：开机自启开关（打开后 `~/Library/LaunchAgents/` 出现条目，关闭后消失，不真的注销重登）；模拟登录项启动：退出程序后 `open IssueRelay.app --args --autostart`（与登录项传的参数相同），只出托盘图标、不出窗口、程序坞里没有图标，从托盘打开面板后程序坞图标出现，关掉面板后消失；MCP 控制服务注册到 Claude Code（`claude mcp get issuerelay` 退出码 0），再移除；切换五种语言，托盘菜单与通知跟着变；关窗口后程序留在托盘、程序坞图标消失；第二次打开应用只把面板带到前面。
- [ ] **Step 6: 精简运行评估**（设计第 18 节第 6 条）：在沙箱目录手动对比 `claude -p … --strict-mcp-config --mcp-config '{"mcpServers":{"linear-server":{"type":"http","url":"https://mcp.linear.app/mcp"}}}'` 与现有参数的启动时间，以及 Linear 工具是否可用（是否复用已有授权）；Codex 对比 `-c` 覆盖 `mcp_servers` 的可行性。结论写进验收报告与设计第 18 节；是否做成按绑定的选项由结论决定，若做，另起任务（执行者参数属于极高风险，单独提交）。
- [ ] **Step 7: 报告**：`docs/gates/<日期>-m2-acceptance.md`，按 `TESTING_SPEC.md` 的模板；真实数据一律脱敏（`DEMO-N`、`Demo Project`），截图不进仓库；未闭环写明（Windows 待实机、页面截图待用户确认等）。设计第 17 节 M2 一行的完成情况同步。

```bash
git add docs
git commit -m "docs(m2): record the desktop app acceptance on macOS"
```

---

## 自查记录（写计划时）

- 设计覆盖：4.1（单实例、关窗不退出、退出询问）Task 3；5.6（标记完成只在面板）Task 4、6；6.2（等待完成 / 立即停止）Task 3、5；8（首次引导、环境检查、一键修复）Task 9、10；9.1 托盘 Task 3；9.2 七个页面 Task 6–9；9.3 外观 Task 5；9.4 多语言 Task 2；9.5 通知 Task 3、9；9.6 图标 Task 11；10.3 MCP 注册 Task 9；14 完全放开提示 Task 8；15 前端测试与截图比对 Task 2–12；16 验证命令 Task 1；17 M2 完成标志 Task 12、13；18.6 精简运行评估 Task 13。开机自启 Task 3、9。自动更新只放开关（M3 实现）。
- 依赖第一部分：`host`、`EngineEvent`、`IssueView`、`Settings` 新字段（`notifications`、`auto_update`、`onboarding_completed`、`full_access_acknowledged`、`log_*`）、`setup`、`SystemRunner::with_env`、`AuthMethod` 的 serde 名 `oauth` / `api_key`。
- 依赖第一部分 Task 9（`core::host`）的三条约定：宿主各部分用各自的停止令牌、IPC 最后停；`Host::login` 与 `Host::use_api_key` 返回清洗过姓名的授权人；`stop(StopRuns)` 会结束一个仍在等待的 `stop(WaitForRuns)`，两者都返回（Task 3 的退出状态机靠这一条从「等待」改为「立即停止」）。
- 类型一致：前端 `types.ts` 与 Rust serde 输出逐字段对应（Task 4，含 `StatusSnapshot.held`、`WaitReasonView`、`StartupState`、`AppInfo.system_locale`）；命令名表（34 条，Task 4）与 `backend.ts`、`api.ts`、`generate_handler!` 一致；事件名常量（Task 3）与 `api.ts` 一致；三张共用用例表（`language-cases.json`、`problem-cases.json`、`clean-cases.json`）各由 Rust 与 TypeScript 两边的测试读取。

### 2026-10-13 预检后修订

计划在执行前做了一次只读的预检扫描（任务两两之间的共享文件与接口、各任务自身的一致性、对照已合入的 core 与第一部分 Task 9 的约定、评审规则、本计划的全局约束），主智能体对每条发现作了裁决（AR1–AR50），本次修订把它们全部写进了计划：测试基础设施（Vitest 全局 API 与只收 `src/**/*.test.*`、公共设置里的 jsdom 桩与固定时钟复位、`.gitignore`、shadcn 组件、Dependabot 的 npm 条目、文件结构表，AR1–AR8）；多语言（缺键守卫、`pub mod i18n`、一张语言用例表且先看文字子标签、真正走到英文回退的测试、自己的相对时间文案、「已取消」、系统语言取 Rust 的 `sys_locale`、锁文件随提交，AR9–AR16）；宿主接入（`chrono` 与写全字段的测试辅助、空壳 `commands.rs`、可查询的启动状态、纯函数的退出状态机、事件转发的落后路径、开机自启不出程序坞图标、日志清洗移进 core、不静默吞错、托盘与面板共用的问题用例表、翻译表只解析一次、`mainBinaryName`，AR17–AR28）；命令与数据层（叶子模块 `backend.ts` 与动态载入的假后端、core 加 `StatusSnapshot.held` 与 `WaitReasonView`、引擎原文只作细节行、`JobActions` 的可选任务与「只重跑最新一次」、写成数据表的假后端、TESTING_SPEC 登记假对象、`setupAction` 一律带执行者，AR29–AR36）；外框与页面（新的 `App` 测试与事件桥组件、等订阅而不是等变量、每页的加载 / 空 / 错误状态与错误测试、布局与状态色变量、共用命名空间与 `?state=` 取值、概览的环境提醒与只数最新一次失败、`cleanDisplayText`、时间线的运行记录行与结果合并规则并直接读 core 夹具、表单测试的输入转义与按区块查询、假后端拒绝非绝对目录、环境检查的五个区块与 `follow-ui` 与按名字决定的一键授权与控制服务的用户范围说明、引导测试等连接状态，AR37–AR47）；图标、截图与实机验收（`tauri icon` 的路径与移动端目录、截图的 `?lang=` 与证明比对会失败、等待改立即停止与登录启动无程序坞图标的实机步骤、Windows 待实机清单，AR48–AR50）。照原样接受、没有改计划的：R14（测试里的 `/Applications/IssueRelay.app/...` 与 `file://` 字面量是通用写法，不是本机路径）；G7（提交命令示例不写 `Co-Authored-By`，由派发时补上）；A14 余下的面板与 Rust 输入不一致一项（已由 AR15 关闭）；扫描第 6 节列的 Tauri、shadcn、Radix 等 API 名称不逐个核对，照全局约束以解析到的版本文档为准、不同处在报告里说明。同日的复扫（核对上述修订）又提出 4 条 H、5 条 M、17 条 L，连同新裁决 AR51（`Host::start` 不再等 Linear，`LinearSetup` 新增 `connecting`）一并写进了计划：`held()` 的类型错误、devhost 测试里遗留的导入、只给测试用的导入、事件转发的大枚举改为装箱；字符规则在 core 里合为 `text::strip_control` 一份实现、用例表对照 `clean_text`；退出钩子对 `ExitRequested.code` 的假设写明待核对并在 Task 13 实机检查 Cmd+Q；链接检查改为 `url::Url` 解析、登录回调同样只开 Linear；用例里的不可见字符改为转义；控制服务检查不再吞错（`unknown` 状态）；以及各条 L（版本跟随 Cargo、测试钉住的英文文案列出、模型名与打码各有共用用例表等）。另外修正了两处扫描没列的不一致：假后端的控制服务原来说 Codex「未安装」而环境检查说它已安装（现在两边都是已安装）；Task 3 的模块原为私有 `mod`，`AppState::host` 等在 Task 4 接上命令之前会被当成死代码（现在都是 `pub mod`）。
