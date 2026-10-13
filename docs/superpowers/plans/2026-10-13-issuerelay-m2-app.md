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
- 前端不拿任何插件权限：`capabilities/default.json` 只给 `core:default`；打开链接、选目录、通知、开机自启都由 Rust 侧命令完成。只打开 `https://linear.app/` 开头的链接。
- 来自 Linear 与 AI 的文本只作为 React 文本节点渲染（自动转义），不用 `dangerouslySetInnerHTML`；原始日志显示前去掉 ANSI 转义序列。
- 面板与托盘的每段文案都走翻译键；五种语言（`zh-CN`、`zh-TW`、`en`、`ja`、`ko`）的键必须一一对应（CI 检查）。默认跟随系统语言，设置里随时切换并立即生效（含托盘菜单与通知）。
- 跟随系统浅色 / 深色；颜色、间距、字号、圆角统一用设计变量；任务状态用固定的颜色和图标（见 Task 5）；每个页面都有空状态、加载状态和错误状态，不出现裸露的报错文本。
- 应用标识 `com.sunkailun.issuerelay`，产品名 `IssueRelay`，窗口默认 1100×720、最小 900×600。
- 截图、示例与测试里的数据全部虚构（`DEMO-N`、`Demo Project`、`Jane Doe`、`~/code/demo-app`）；真实验收截图只留在本机，不进仓库。
- 依赖版本：Tauri 及其插件取当前 2.x 最新版，前端依赖取当前最新稳定版；用 `cargo add` / `pnpm add` 解析后锁定，`Cargo.lock` 与 `app/pnpm-lock.yaml` 一并提交。Tauri API 名称以实际解析到的版本文档（docs.rs、v2.tauri.app）为准，与本计划写法不同时照文档写并在报告里说明。
- 分支 `feat/m2-app`（从合入第一部分之后的 `main` 切出），经 PR 合入。

## Review Focus

1. 宿主还没起来（启动要几秒）时面板就发命令：命令要等宿主就绪，宿主启动失败（例如另一个宿主已在运行）时面板显示可读的原因和「退出」按钮（Task 3、4 的测试）。
2. 有任务在跑时退出（托盘「退出」、Cmd+Q、系统要求退出）：必须先问「等待完成 / 立即停止」，选了等待以后仍能改为立即停止，窗口关掉也不能让程序在任务没收尾时退出（Task 3 的退出流程测试）。
3. 引擎事件来得比面板处理快（大量任务同时变化）：事件丢失时面板要整体刷新，不能停在旧状态（Task 3 的 `resync`、Task 5 的事件桥测试）。
4. 第一次用、什么都没配：没有登录、没有绑定、执行者没装时，每个页面都要给出下一步该做什么，而不是空白或报错（Task 5–10 的空状态测试）。
5. 系统语言是五种之外的语言（例如德语）或带地区的写法（`zh-Hant-HK`、`ja-JP`）：解析到正确的界面语言，没有的就用英文（Task 2 的语言解析测试，Rust 与 TypeScript 两边结果一致）。

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `app/package.json`、`vite.config.ts`、`tsconfig*.json`、`eslint.config.js`、`components.json`、`index.html` | 前端工程与工具链 | 1 |
| `app/src-tauri/Cargo.toml`、`build.rs`、`tauri.conf.json`、`capabilities/default.json` | Tauri crate 与配置 | 1 |
| `app/src/i18n/`、`app/src-tauri/src/i18n.rs` | 五种语言的翻译文件；前端与 Rust 共用 | 2 |
| `app/src-tauri/src/{lib,main,state,startup,window,quit,logging,events,notify,tray}.rs` | 宿主接入、窗口与退出、日志、事件转发、通知、托盘 | 3 |
| `app/src-tauri/src/commands.rs` | 面板调用的全部命令 | 4 |
| `app/src/lib/{types,api,mock,format}.ts`、`app/src/hooks/` | 前端数据层：类型、命令封装、假后端、查询与事件桥 | 4、5 |
| `app/src/components/`、`app/src/styles.css`、`app/src/App.tsx` | 面板外框、设计变量、状态标识、空 / 加载 / 错误状态 | 5 |
| `app/src/pages/Overview.tsx`、`Issues.tsx` | 概览、需求 | 6 |
| `app/src/pages/Runs.tsx`、`RunDetail.tsx`、`app/src/lib/timeline.ts` | 运行记录与时间线 | 7 |
| `app/src/pages/Bindings.tsx`、`BindingDialog.tsx` | 项目绑定 | 8 |
| `app/src/pages/Doctor.tsx`、`Settings.tsx`、`Help.tsx` | 环境检查、设置、使用说明 | 9 |
| `app/src/pages/onboarding/` | 首次引导 | 10 |
| `app/icons-src/`、`app/scripts/icons.mjs`、`app/src-tauri/icons/` | 图标源文件、生成脚本与产物 | 11 |
| `app/scripts/screenshots.ts`、`app/tests/screens.spec.ts`、`docs/gates/screenshots/m2/` | 截图与截图比对 | 12 |
| `docs/gates/<日期>-m2-acceptance.md` | M2 实机验收报告 | 13 |

---

### Task 1: 前端工程与 Tauri crate 骨架

**Files:**
- Create: `app/`（由 Vite 的 `react-ts` 模板生成后调整）、`app/src-tauri/`（由 `tauri init` 生成后调整）
- Modify: `Cargo.toml`（工作区成员加 `app/src-tauri`）、`.gitignore`、`.github/workflows/ci.yml`
- Modify: `AGENTS.md`、`CONTRIBUTING.md`、`README.md`、`docs/ai-coding/AI_CODING_GUIDE.md`、`docs/ai-coding/TESTING_SPEC.md`、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md`、`.github/pull_request_template.md`（验证命令加上前端两条）
- Test: `app/src/App.test.tsx`

**Interfaces:**
- Produces: `pnpm --dir app {dev,build,lint,test,tauri}` 五个脚本；Rust crate `issuerelay-app`（lib 名 `issuerelay_app`，入口 `issuerelay_app::run()`）；开发端口 1420；`app/dist` 是前端产物目录（git 忽略）。
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
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    css: false,
  },
});
```

（`test` 字段需要 `/// <reference types="vitest/config" />` 或从 `vitest/config` 导入 `defineConfig`，按 Vitest 当前版本的写法。）`tsconfig.app.json` 加 `"baseUrl": "."` 与 `"paths": { "@/*": ["src/*"] }`，保持 `strict: true`。`app/src/test-setup.ts`：`import "@testing-library/jest-dom/vitest";`。

初始化 shadcn/ui（new-york 风格、neutral 基色、CSS 变量），并加入后续页面要用的组件：

```bash
pnpm --dir app dlx shadcn@latest init
pnpm --dir app dlx shadcn@latest add button card badge dialog alert-dialog input label select switch tabs tooltip scroll-area separator table textarea alert dropdown-menu skeleton sonner
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
  "version": "0.1.0",
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

（窗口启动时隐藏，由 Task 3 决定何时显示；`tauri init` 生成的默认图标先保留，Task 11 替换。）

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

`.gitignore` 加 `app/node_modules/`、`app/dist/`、`app/src-tauri/gen/`（若 `tauri init` 生成的 `.gitignore` 已覆盖就不重复）。

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

- [ ] **Step 5: 文档**

- `AGENTS.md`、`CONTRIBUTING.md`、`docs/ai-coding/AI_CODING_GUIDE.md` 第 2 节、`.github/pull_request_template.md` 的验证命令加 `pnpm --dir app lint` 与 `pnpm --dir app test`；开发环境要求加 Node.js 22 以上与 pnpm（版本见 `app/package.json` 的 `packageManager`）。
- `README.md` 开发一节加：`pnpm --dir app install`、`pnpm --dir app tauri dev`（启动桌面应用开发版）、`pnpm --dir app dev`（只在浏览器里看面板，用假数据）。
- `docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 第 4 节写明 `app/` 的布局（与本计划「文件结构」表一致）；`docs/ai-coding/TESTING_SPEC.md` 分层表加「前端组件测试（Vitest + Testing Library）」一行和「截图比对（Playwright，本机运行）」一行。

Run: 六条验证命令
Expected: 全部通过。

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore .github app AGENTS.md CONTRIBUTING.md README.md docs
git commit -m "feat(app): scaffold the Tauri app and the React panel"
```

---

### Task 2: 五种界面语言（前端与 Rust 共用翻译文件）

设计第 9.4 节。翻译文件是前端与 Rust 的唯一来源：前端经 i18next 读取，Rust 在编译时用 `include_str!` 嵌入同一批文件，供托盘菜单、通知和退出提示使用。

**Files:**
- Create: `app/src/i18n/index.ts`、`app/src/i18n/language.ts`、`app/src/i18n/locales/{zh-CN,zh-TW,en,ja,ko}.json`、`app/src/i18n/i18n.test.ts`
- Create: `app/src-tauri/src/i18n.rs`
- Modify: `app/src-tauri/src/lib.rs`（`mod i18n;`）、`app/src-tauri/Cargo.toml`（`sys-locale`）
- Modify: `app/src/main.tsx`（初始化 i18n）

**Interfaces:**
- Produces:
  - TS：`type Lang = "zh-CN" | "zh-TW" | "en" | "ja" | "ko"`、`LANGUAGES: Lang[]`、`resolveLanguage(setting: string, systemLocale: string | undefined): Lang`、`initI18n(lang: Lang): Promise<void>`、`initI18nOnce(lang: Lang): Promise<void>`（已初始化时只切换语言）、`setLanguage(lang: Lang): Promise<void>`
  - Rust：`pub const i18n::LANGUAGES: [&str; 5]`、`pub fn i18n::resolve_language(setting: &str, system: Option<&str>) -> &'static str`、`pub struct i18n::Translator`：`new(lang: &str) -> Self`、`for_settings(settings: &Settings) -> Self`、`t(&self, key: &str) -> String`、`t_with(&self, key: &str, vars: &[(&str, &str)]) -> String`
  - 翻译键的命名空间：`app`、`nav`、`state`、`common`、`tray`、`notify`、`quit`、`startup`；后续任务在同一批文件里加 `overview`、`issues`、`runs`、`bindings`、`doctor`、`settings`、`help`、`onboarding` 等命名空间。占位符一律写成 `{{name}}`。
- Consumes: 第一部分的 `config::Settings`（`language`）。

- [ ] **Step 1: 语言解析的失败测试（两边同一组用例）**

`app/src/i18n/i18n.test.ts`：

```ts
import { describe, expect, test } from "vitest";
import { LANGUAGES, resolveLanguage } from "./language";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";

/** The same cases as `resolve_language` in app/src-tauri/src/i18n.rs. */
export const LANGUAGE_CASES: [string, string | undefined, string][] = [
  ["system", "zh-CN", "zh-CN"],
  ["system", "zh-Hans-CN", "zh-CN"],
  ["system", "zh-TW", "zh-TW"],
  ["system", "zh-Hant-HK", "zh-TW"],
  ["system", "zh-HK", "zh-TW"],
  ["system", "zh-MO", "zh-TW"],
  ["system", "zh", "zh-CN"],
  ["system", "ja-JP", "ja"],
  ["system", "ko-KR", "ko"],
  ["system", "en-GB", "en"],
  ["system", "de-DE", "en"],
  ["system", undefined, "en"],
  ["ja", "zh-CN", "ja"],
  ["zh-TW", "en-US", "zh-TW"],
  ["fr", "zh-CN", "en"],
];

describe("language resolution", () => {
  test.each(LANGUAGE_CASES)("setting %s with system %s gives %s", (setting, system, expected) => {
    expect(resolveLanguage(setting, system)).toBe(expected);
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

  test("there is one file per language", () => {
    expect(Object.keys(files).sort()).toEqual([...LANGUAGES].sort());
  });

  test.each(Object.entries(files))("%s has exactly the English keys", (_lang, table) => {
    expect(keys(table).sort()).toEqual(keys(en).sort());
  });

  test.each(Object.entries(files))("%s has no empty text and the same placeholders", (_lang, table) => {
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
    use super::*;

    /// The same cases as LANGUAGE_CASES in app/src/i18n/i18n.test.ts.
    const CASES: [(&str, Option<&str>, &str); 15] = [
        ("system", Some("zh-CN"), "zh-CN"),
        ("system", Some("zh-Hans-CN"), "zh-CN"),
        ("system", Some("zh-TW"), "zh-TW"),
        ("system", Some("zh-Hant-HK"), "zh-TW"),
        ("system", Some("zh-HK"), "zh-TW"),
        ("system", Some("zh-MO"), "zh-TW"),
        ("system", Some("zh"), "zh-CN"),
        ("system", Some("ja-JP"), "ja"),
        ("system", Some("ko-KR"), "ko"),
        ("system", Some("en-GB"), "en"),
        ("system", Some("de-DE"), "en"),
        ("system", None, "en"),
        ("ja", Some("zh-CN"), "ja"),
        ("zh-TW", Some("en-US"), "zh-TW"),
        ("fr", Some("zh-CN"), "en"),
    ];

    #[test]
    fn languages_resolve_like_the_panel_does() {
        for (setting, system, expected) in CASES {
            assert_eq!(resolve_language(setting, system), expected, "{setting} / {system:?}");
        }
    }

    #[test]
    fn every_key_rust_uses_exists_in_every_language() {
        for lang in LANGUAGES {
            let t = Translator::new(lang);
            for key in USED_KEYS {
                assert!(lookup(&t.table, key).is_some(), "{lang} lacks {key}");
            }
        }
    }

    #[test]
    fn missing_keys_fall_back_to_english_then_to_the_key() {
        let t = Translator::new("ja");
        assert_eq!(t.t("no.such.key"), "no.such.key");
        assert_eq!(
            Translator::new("en").t_with("tray.status_counts", &[("running", "2"), ("queued", "1")]),
            "Running 2 · Queued 1"
        );
    }
}
```

Run: `pnpm --dir app test` 与 `cargo test -p issuerelay-app`
Expected: 失败（文件与函数还不存在）。

- [ ] **Step 2: 实现**

`app/src/i18n/language.ts`：

```ts
export type Lang = "zh-CN" | "zh-TW" | "en" | "ja" | "ko";
export const LANGUAGES: Lang[] = ["zh-CN", "zh-TW", "en", "ja", "ko"];

/**
 * The interface language for a `language` setting ("system" or one of LANGUAGES) and the
 * system's locale. Traditional Chinese regions and scripts map to zh-TW, other Chinese to
 * zh-CN; a language IssueRelay does not offer falls back to English. Keep in step with
 * `resolve_language` in app/src-tauri/src/i18n.rs (the tests share their cases).
 */
export function resolveLanguage(setting: string, systemLocale: string | undefined): Lang {
  if ((LANGUAGES as string[]).includes(setting)) return setting as Lang;
  if (setting !== "system") return "en";
  const locale = (systemLocale ?? "").toLowerCase();
  if (locale.startsWith("zh")) {
    return /hant|-tw|-hk|-mo/.test(locale) ? "zh-TW" : "zh-CN";
  }
  if (locale.startsWith("ja")) return "ja";
  if (locale.startsWith("ko")) return "ko";
  return "en";
}
```

`app/src/i18n/index.ts`：

```ts
import i18n from "i18next";
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

export async function initI18n(lang: Lang): Promise<void> {
  await i18n.use(initReactI18next).init({
    resources,
    lng: lang,
    fallbackLng: "en",
    interpolation: { escapeValue: false }, // React escapes text nodes itself.
    returnNull: false,
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

`app/src/main.tsx` 在渲染前 `await initI18n(resolveLanguage("system", navigator.language))`（Task 5 改为读设置）。

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
    "canceled": "Stopped"
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
    "failed": "The run failed: {{reason}}"
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
    "quit": "Quit"
  }
}
```

（`zh-CN.json` 示例：`"tray": { "status_counts": "运行中 {{running}} 条 · 排队 {{queued}} 条", "status_paused": "已暂停监控", "status_problem": "需要处理", "pause": "暂停监控", "resume": "恢复监控", "poll_now": "立即检查一次", "open_panel": "打开面板", "quit": "退出 IssueRelay", "starting": "正在启动…" }`；状态名与设计 5.2 节一致：等待绑定、排队中、运行中、待验收、待补充信息、已完成、失败、已停止。）

`app/src-tauri/src/i18n.rs`：

```rust
//! The interface languages, read from the panel's translation files so that the tray menu,
//! notifications and quit prompt say the same as the panel.
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
/// system locale. Keep in step with `resolveLanguage` in app/src/i18n/language.ts.
pub fn resolve_language(setting: &str, system: Option<&str>) -> &'static str {
    if let Some(lang) = LANGUAGES.iter().find(|lang| **lang == setting) {
        return lang;
    }
    if setting != "system" {
        return "en";
    }
    let locale = system.unwrap_or_default().to_ascii_lowercase();
    if locale.starts_with("zh") {
        let traditional = ["hant", "-tw", "-hk", "-mo"].iter().any(|mark| locale.contains(mark));
        return if traditional { "zh-TW" } else { "zh-CN" };
    }
    if locale.starts_with("ja") {
        return "ja";
    }
    if locale.starts_with("ko") {
        return "ko";
    }
    "en"
}

fn table(lang: &str) -> Value {
    let source = SOURCES
        .iter()
        .find(|(name, _)| *name == lang)
        .or_else(|| SOURCES.iter().find(|(name, _)| *name == "en"))
        .map(|(_, text)| *text)
        .expect("the English translations are embedded");
    serde_json::from_str(source).expect("translation files are valid JSON (checked by the panel's tests)")
}

fn lookup<'a>(table: &'a Value, key: &str) -> Option<&'a str> {
    key.split('.').try_fold(table, |node, part| node.get(part))?.as_str()
}

/// Looks up interface texts in one language, falling back to English, then to the key itself.
pub struct Translator {
    table: Value,
    fallback: Value,
}

impl Translator {
    pub fn new(lang: &str) -> Self {
        Self { table: table(lang), fallback: table("en") }
    }

    /// The language the settings ask for (`"system"` follows the system locale).
    pub fn for_settings(settings: &Settings) -> Self {
        Self::new(resolve_language(&settings.language, sys_locale::get_locale().as_deref()))
    }

    pub fn t(&self, key: &str) -> String {
        lookup(&self.table, key)
            .or_else(|| lookup(&self.fallback, key))
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

Run: `pnpm --dir app test` 与 `cargo test -p issuerelay-app`
Expected: PASS。

- [ ] **Step 3: 文档与提交**

- 设计第 9.4 节补：「翻译文件在 `app/src/i18n/locales/`，Rust 侧编译时嵌入同一批文件；新增语言要同时加文件、把语言加进两边的 `LANGUAGES`，并补语言解析用例。」

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs
git commit -m "feat(app): add the five interface languages shared by the panel and the tray"
```

---

### Task 3: 宿主接入、窗口与退出、日志、事件转发、通知、托盘

设计第 4.1 节（单实例；关窗口不退出；从托盘退出即停止监控，有任务在跑时先询问）、6.2 节（「等待完成 / 立即停止」）、9.1 节（托盘菜单）、9.5 节（四种通知可分别关闭）。宿主在后台异步启动，面板的命令等它就绪；启动失败（例如另一个宿主已在运行）时显示窗口并告诉面板原因。

**Files:**
- Create: `app/src-tauri/src/{state,startup,window,quit,logging,events,notify,tray}.rs`
- Modify: `app/src-tauri/src/lib.rs`、`app/src-tauri/Cargo.toml`
- Create: `app/src-tauri/icons/tray/{idle,running,paused,problem}.png`（占位：从 `icons/32x32.png` 复制，Task 11 换成正式图标）
- Test: 各模块末尾的 `mod tests`

**Interfaces:**
- Produces:
  - `pub const AUTOSTART_ARG: &str = "--autostart"`
  - `pub struct state::AppState`：`new(autostarted: bool)`、`set_host(&self, result: Result<Host, String>)`、`async host(&self) -> Result<Host, String>`、`try_host(&self) -> Option<Host>`、`pub quitting: AtomicBool`、`pub autostarted: bool`
  - `pub fn window::show(app: &AppHandle)`、`pub fn window::hide(app: &AppHandle)`
  - `pub enum quit::QuitAction { Exit, StopThenExit, Ask }`、`pub fn quit::decide(host_ready: bool, running: usize) -> QuitAction`、`pub fn quit::request(app: &AppHandle)`、`pub async fn quit::finish(app: AppHandle, host: Host, mode: StopMode)`
  - `pub const events::ENGINE_EVENT: &str = "engine-event"`、`events::RESYNC_EVENT = "engine-resync"`、`events::QUIT_REQUESTED = "quit-requested"`、`events::STARTUP_FAILED = "startup-failed"`
  - `pub enum notify::NotificationKind { RunStarted, AwaitingAcceptance, NeedsInfo, Failed }`、`pub fn notify::notification_for(previous: Option<JobState>, job: &Job) -> Option<NotificationKind>`、`pub fn notify::enabled(kind, settings: &NotificationSettings) -> bool`、`pub fn notify::content(kind, job: &Job, t: &Translator) -> (String, String)`
  - `pub enum tray::TrayState { Idle, Running, Paused, Problem }`、`pub struct tray::TrayView { pub state: TrayState, pub status_line: String, pub toggle_label: String }`、`pub fn tray::tray_view(status: &StatusSnapshot, setup: &LinearSetup, t: &Translator) -> TrayView`、`pub fn tray::create(app: &AppHandle) -> tauri::Result<()>`、`pub fn tray::refresh(app: &AppHandle, host: &Host)`
  - `pub fn logging::init(logs_dir: Option<&Path>) -> Option<WorkerGuard>`、`pub fn logging::allowed(target: &str, level: &Level) -> bool`
- Consumes: 第一部分的 `host::{Host, HostParts, SystemConnector, LinearSetup, StopMode}`、`engine::{EngineEvent, StatusSnapshot}`、`config::NotificationSettings`、`model::{Job, JobState}`、`ipc::server::AlreadyRunning`、`paths::AppPaths`；Task 2 的 `i18n::Translator`

- [ ] **Step 1: 依赖**

```bash
cargo add -p issuerelay-app tauri-plugin-single-instance tauri-plugin-notification tauri-plugin-autostart tauri-plugin-opener tauri-plugin-dialog
cargo add -p issuerelay-app tokio --features full
cargo add -p issuerelay-app anyhow tracing tracing-appender
cargo add -p issuerelay-app tracing-subscriber --features env-filter
```

- [ ] **Step 2: 纯逻辑的失败测试**

`notify.rs` 测试（构造 `Job` 用一个本地辅助 `job(state)`，字段取虚构值）：

```rust
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
    fn a_notification_names_the_issue_and_the_reason() {
        let mut failed = job(JobState::Failed);
        failed.reason = Some("run exceeded the time limit".into());
        let (title, body) = content(NotificationKind::Failed, &failed, &Translator::new("en"));
        assert_eq!(title, "DEMO-1 Demo task");
        assert_eq!(body, "The run failed: run exceeded the time limit");
    }
```

`tray.rs` 测试：

```rust
    fn connected() -> LinearSetup {
        LinearSetup::Connected { method: AuthMethod::OAuth }
    }

    fn checked() -> StatusSnapshot {
        StatusSnapshot { linear_ok: true, last_poll_at: Some(chrono::Utc::now()), ..StatusSnapshot::default() }
    }

    #[test]
    fn the_tray_shows_what_the_engine_is_doing() {
        let t = Translator::new("en");
        let idle = tray_view(&checked(), &connected(), &t);
        assert_eq!(idle.state, TrayState::Idle);
        assert_eq!(idle.status_line, "Running 0 · Queued 0");
        assert_eq!(idle.toggle_label, "Pause monitoring");

        let busy = StatusSnapshot { running: 2, queued: 1, ..checked() };
        assert_eq!(tray_view(&busy, &connected(), &t).state, TrayState::Running);
        assert_eq!(tray_view(&busy, &connected(), &t).status_line, "Running 2 · Queued 1");

        let paused = StatusSnapshot { paused: true, running: 1, ..checked() };
        let view = tray_view(&paused, &connected(), &t);
        assert_eq!(view.state, TrayState::Paused);
        assert_eq!(view.status_line, "Monitoring paused");
        assert_eq!(view.toggle_label, "Resume monitoring");
    }

    #[test]
    fn problems_win_over_everything_else() {
        let t = Translator::new("en");
        let not_logged_in = tray_view(&checked(), &LinearSetup::NotConfigured, &t);
        assert_eq!(not_logged_in.state, TrayState::Problem);
        assert_eq!(not_logged_in.status_line, "Running 0 · Queued 0 · Needs attention");
        let failing = StatusSnapshot { linear_ok: false, ..checked() };
        assert_eq!(tray_view(&failing, &connected(), &t).state, TrayState::Problem);
        let not_checked_yet = StatusSnapshot::default();
        assert_eq!(tray_view(&not_checked_yet, &connected(), &t).state, TrayState::Idle, "no check yet is not a failure");
        let start_error = StatusSnapshot { last_error: Some("could not start DEMO-1".into()), ..checked() };
        assert_eq!(tray_view(&start_error, &connected(), &t).state, TrayState::Problem);
    }
```

`quit.rs` 测试：

```rust
    #[test]
    fn quitting_asks_only_while_runs_are_in_progress() {
        assert_eq!(decide(false, 0), QuitAction::Exit, "no host: nothing to stop");
        assert_eq!(decide(true, 0), QuitAction::StopThenExit);
        assert_eq!(decide(true, 2), QuitAction::Ask);
    }
```

`logging.rs` 测试：

```rust
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
```

`state.rs` 测试：

```rust
    #[tokio::test]
    async fn commands_wait_for_the_host_and_see_a_failed_start() {
        let state = Arc::new(AppState::new(false));
        let waiting = tokio::spawn({
            let state = state.clone();
            async move { state.host().await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        state.set_host(Err("another IssueRelay is already running".into()));
        assert_eq!(waiting.await.unwrap().err().unwrap(), "another IssueRelay is already running");
        assert!(state.try_host().is_none());
    }
```

Run: `cargo test -p issuerelay-app`
Expected: 编译失败。

- [ ] **Step 3: 实现各模块**

`state.rs`：

```rust
//! What the app keeps for its whole life: the host, once started, and whether it is quitting.
use std::sync::atomic::AtomicBool;

use issuerelay_core::host::Host;
use tokio::sync::{Notify, OnceCell};

pub struct AppState {
    /// Set once by the startup task: the running host, or why it could not start.
    host: OnceCell<Result<Host, String>>,
    ready: Notify,
    /// Set when the app has decided to quit; window closes and exit requests then go through.
    pub quitting: AtomicBool,
    /// Started by the login item (`AUTOSTART_ARG`): stay in the tray instead of showing the panel.
    pub autostarted: bool,
}

impl AppState {
    pub fn new(autostarted: bool) -> Self {
        Self { host: OnceCell::new(), ready: Notify::new(), quitting: AtomicBool::new(false), autostarted }
    }

    /// Records how the start went and wakes every command waiting for it.
    pub fn set_host(&self, result: Result<Host, String>) {
        let _ = self.host.set(result);
        self.ready.notify_waiters();
    }

    /// The host, waiting while it starts; the reason when it could not start.
    pub async fn host(&self) -> Result<Host, String> {
        loop {
            // Registered before the check, so a `set_host` in between still wakes this wait.
            let notified = self.ready.notified();
            if let Some(result) = self.host.get() {
                return result.clone();
            }
            notified.await;
        }
    }

    /// The host if it has started.
    pub fn try_host(&self) -> Option<Host> {
        self.host.get().and_then(|result| result.as_ref().ok().cloned())
    }
}
```

`logging.rs`：

```rust
//! Logs go to `<data dir>/logs/app.log` (rotated daily, seven files kept) and, in debug builds,
//! to stderr. `RUST_LOG` overrides the default filter, but rmcp is never logged below `info`:
//! at `debug` it writes the raw OAuth authorization code.
use std::path::Path;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, filter::filter_fn, fmt, prelude::*};

const DEFAULT_FILTER: &str = "info,issuerelay_core=debug,issuerelay_app=debug";

/// Whether an event of `level` from `target` may be logged at all.
pub fn allowed(target: &str, level: &Level) -> bool {
    let rmcp = target == "rmcp" || target.starts_with("rmcp::");
    !(rmcp && *level > Level::INFO)
}

/// Starts logging; keep the returned guard alive for as long as the app runs, or the last lines
/// are lost. Without a log directory only stderr is used.
pub fn init(logs_dir: Option<&Path>) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let cap = filter_fn(|meta| allowed(meta.target(), meta.level()));
    let (file_layer, guard) = match logs_dir.and_then(|dir| {
        tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("app")
            .filename_suffix("log")
            .max_log_files(7)
            .build(dir)
            .ok()
    }) {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (Some(fmt::layer().with_ansi(false).with_writer(writer)), Some(guard))
        }
        None => (None, None),
    };
    let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(cap)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    guard
}
```

`window.rs`：

```rust
//! The panel window: shown on demand, hidden instead of closed. On macOS the Dock icon follows
//! the window: a tray app shows no Dock icon while its panel is hidden.
use tauri::{AppHandle, Manager, WindowEvent};

use crate::state::AppState;

pub fn show(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// Closing the panel hides it; monitoring goes on in the tray. Once the app is quitting the
/// window may really close.
pub fn on_event(window: &tauri::Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        let app = window.app_handle();
        let quitting = app.state::<AppState>().quitting.load(std::sync::atomic::Ordering::SeqCst);
        if window.label() == "main" && !quitting {
            api.prevent_close();
            hide(app);
        }
    }
}
```

`quit.rs`：

```rust
//! Quitting (design 4.1, 6.2): with runs in progress the panel asks whether to wait for them or
//! stop them; otherwise the host stops and the app exits. Every way to quit comes here: the
//! tray's Quit, Cmd+Q and the system asking the app to exit.
use std::sync::atomic::Ordering;

use issuerelay_core::host::{Host, StopMode};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::{events, state::AppState, window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitAction {
    /// The host never started: nothing to stop.
    Exit,
    /// Nothing is running: stop the host, then exit.
    StopThenExit,
    /// Runs are in progress: ask the user.
    Ask,
}

pub fn decide(host_ready: bool, running: usize) -> QuitAction {
    match (host_ready, running) {
        (false, _) => QuitAction::Exit,
        (true, 0) => QuitAction::StopThenExit,
        (true, _) => QuitAction::Ask,
    }
}

pub fn request(app: &AppHandle) {
    let host = app.state::<AppState>().try_host();
    let running = host.as_ref().map_or(0, |host| host.engine().status().running);
    match (decide(host.is_some(), running), host) {
        (QuitAction::Ask, _) => {
            window::show(app);
            let _ = app.emit(events::QUIT_REQUESTED, running);
        }
        (QuitAction::StopThenExit, Some(host)) => {
            tauri::async_runtime::spawn(finish(app.clone(), host, StopMode::StopRuns));
        }
        _ => {
            app.state::<AppState>().quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
    }
}

/// Stops the host as `mode` says, then exits. Called again with `StopRuns` while a
/// `WaitForRuns` is waiting, it stops the runs and the first call finishes too.
pub async fn finish(app: AppHandle, host: Host, mode: StopMode) {
    app.state::<AppState>().quitting.store(true, Ordering::SeqCst);
    host.stop(mode).await;
    app.exit(0);
}

/// The app's event loop hook: an exit IssueRelay did not ask for itself goes through `request`.
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
    if let RunEvent::ExitRequested { api, code, .. } = event {
        if code.is_some() {
            return; // `app.exit(..)` from `finish` or `request`.
        }
        api.prevent_exit();
        if !app.state::<AppState>().quitting.load(Ordering::SeqCst) {
            request(app);
        }
    }
}
```

`notify.rs`：

```rust
//! System notifications for the four moments design 9.5 names, each of which can be switched off.
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

/// Title and body. The issue's title and the reason were cleaned when the engine stored them.
pub fn content(kind: NotificationKind, job: &Job, t: &Translator) -> (String, String) {
    let title = format!("{} {}", job.issue_identifier, job.title);
    let body = match kind {
        NotificationKind::RunStarted => t.t("notify.run_started"),
        NotificationKind::AwaitingAcceptance => t.t("notify.awaiting_acceptance"),
        NotificationKind::NeedsInfo => t.t("notify.needs_info"),
        NotificationKind::Failed => {
            t.t_with("notify.failed", &[("reason", job.reason.as_deref().unwrap_or_default())])
        }
    };
    (title, body)
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
//! the tray current.
use std::collections::HashMap;

use issuerelay_core::{engine::EngineEvent, host::Host, model::JobState};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

use crate::{notify, tray};

pub const ENGINE_EVENT: &str = "engine-event";
/// Events were lost: the panel reads everything again.
pub const RESYNC_EVENT: &str = "engine-resync";
pub const QUIT_REQUESTED: &str = "quit-requested";
pub const STARTUP_FAILED: &str = "startup-failed";

/// How many recent jobs seed the known states, so that existing jobs do not notify on start.
const SEED_JOBS: usize = 500;

fn seed(host: &Host) -> HashMap<i64, JobState> {
    host.engine()
        .list_jobs(SEED_JOBS)
        .unwrap_or_default()
        .into_iter()
        .map(|job| (job.id, job.state))
        .collect()
}

pub fn spawn_forwarder(app: AppHandle, host: Host) {
    tauri::async_runtime::spawn(async move {
        let mut events = host.engine().subscribe();
        let mut known = seed(&host);
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let EngineEvent::Job { job } = &event {
                        let previous = known.insert(job.id, job.state);
                        if let Some(kind) = notify::notification_for(previous, job) {
                            notify::show(&app, &host, kind, job);
                        }
                    }
                    let _ = app.emit(ENGINE_EVENT, &event);
                    tray::refresh(&app, &host);
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::debug!(missed, "the panel fell behind the engine's events");
                    known = seed(&host);
                    let _ = app.emit(RESYNC_EVENT, ());
                    tray::refresh(&app, &host);
                }
                Err(RecvError::Closed) => return,
            }
        }
    });
}
```

`tray.rs`（图标字节由 Task 11 换成正式图标；此处先读占位文件）：

```rust
//! The tray icon and its menu (design 9.1): the monitoring status, pause or resume, check now,
//! open the panel, quit. macOS shows a monochrome template image, Windows a colour one.
use issuerelay_core::{engine::StatusSnapshot, host::{Host, LinearSetup}};
use tauri::{
    AppHandle, Manager,
    image::Image,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{i18n::Translator, quit, state::AppState, window};

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
/// check failed, a job could not start) wins over paused, which wins over running.
pub fn tray_view(status: &StatusSnapshot, setup: &LinearSetup, t: &Translator) -> TrayView {
    let problem = !matches!(setup, LinearSetup::Connected { .. })
        || (status.last_poll_at.is_some() && !status.linear_ok)
        || status.last_error.is_some();
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
    let settings = host.engine().settings();
    let t = Translator::for_settings(&settings);
    let view = tray_view(&host.engine().status(), &host.linear_setup(), &t);
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.status.set_text(&view.status_line);
        let _ = items.toggle.set_text(&view.toggle_label);
        let _ = items.toggle.set_enabled(true);
        let _ = items.poll.set_text(t.t("tray.poll_now"));
        let _ = items.poll.set_enabled(true);
        let _ = items.open.set_text(t.t("tray.open_panel"));
        let _ = items.quit.set_text(t.t("tray.quit"));
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon(view.state)));
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos"));
    }
}
```

`startup.rs`：

```rust
//! Starts the host in the background so the window and tray appear at once, then decides
//! whether to show the panel.
use std::sync::Arc;

use issuerelay_core::{
    host::{Host, HostParts, SystemConnector},
    ipc::server::AlreadyRunning,
    paths::AppPaths,
    text::clean_text,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{events, state::AppState, tray, window};

pub async fn start(app: AppHandle) {
    let result = match AppPaths::system() {
        Ok(paths) => Host::start(paths, Arc::new(SystemConnector), HostParts::default())
            .await
            .map_err(describe),
        Err(e) => Err(clean_text(&format!("{e:#}"))),
    };
    let state = app.state::<AppState>();
    match &result {
        Ok(host) => {
            events::spawn_forwarder(app.clone(), host.clone());
            tray::refresh(&app, host);
            // The login item starts IssueRelay in the tray; the first run always shows the guide.
            if !state.autostarted || !host.engine().settings().onboarding_completed {
                window::show(&app);
            }
        }
        Err(message) => {
            tracing::error!(%message, "IssueRelay could not start its engine");
            window::show(&app);
            let _ = app.emit(events::STARTUP_FAILED, message);
        }
    }
    state.set_host(result);
}

fn describe(error: anyhow::Error) -> String {
    if error.downcast_ref::<AlreadyRunning>().is_some() {
        "already_running".to_string()
    } else {
        clean_text(&format!("{error:#}"))
    }
}
```

（`"already_running"` 是给面板的固定代号，面板显示 `startup.already_running` 的译文；其余原因原样显示。）

`lib.rs`：

```rust
//! IssueRelay's desktop app: the tray icon and the panel around the engine.
mod commands;
mod events;
mod i18n;
mod logging;
mod notify;
mod quit;
mod startup;
mod state;
mod tray;
mod window;

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
            tauri::async_runtime::spawn(startup::start(app.handle().clone()));
            Ok(())
        })
        .on_window_event(window::on_event)
        .build(tauri::generate_context!())
        .expect("IssueRelay could not start")
        .run(quit::on_run_event);
}
```

（`commands::handler()` 在 Task 4 实现；本任务先放一个空的 `commands.rs`，其中 `pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static { tauri::generate_handler![] }`——若 `generate_handler!` 不能这样返回，就在 `lib.rs` 里直接写 `.invoke_handler(tauri::generate_handler![...])`，Task 4 在那里补全命令列表。日志写到数据目录的 `logs/app.log*`，与运行日志 `<任务 ID>.jsonl` 同目录但文件名不同，Task 3 的日志清理只删 `<数字>.jsonl`，不会误删。）

Run: `cargo test -p issuerelay-app` 与 `cargo clippy -p issuerelay-app --all-targets -- -D warnings`
Expected: PASS。

- [ ] **Step 4: 手动冒烟**

Run: `pnpm --dir app tauri dev`（本机，开发版）
Expected: 菜单栏出现托盘图标；菜单里有状态行与五个操作；窗口按「首次引导未完成」显示；关闭窗口后程序仍在托盘；托盘「退出」后进程结束（没有任务在跑时不提问）。把观察到的结果写进报告。

- [ ] **Step 5: 文档与提交**

- 设计第 4.1 节补：「托盘程序启动时先出托盘图标，宿主在后台启动（解析环境、连接 Linear 需要几秒）；面板的命令等宿主就绪。开机自启时停在托盘，其余情况显示面板；macOS 上面板隐藏时不显示程序坞图标。」第 9.1 节补托盘图标四种状态的判定顺序（出问题 > 已暂停 > 运行中 > 空闲）。
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 托盘、通知、退出流程 | app/src-tauri/src/tray.rs、notify.rs、quit.rs |`；`.github/CODEOWNERS` 加 `/app/src-tauri/src/`（宿主接入与命令都是高风险）。

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs .github Cargo.lock
git commit -m "feat(app): host the engine in the tray app with notifications and a quit prompt"
```

---

### Task 4: 面板命令与前端数据层

面板调用的全部 Tauri 命令（Rust），以及前端的类型、命令封装和假后端（TypeScript）。假后端让面板在普通浏览器里也能完整运行：开发、组件测试、截图都用它，数据全部虚构。

**Files:**
- Modify: `app/src-tauri/src/commands.rs`、`app/src-tauri/src/lib.rs`（注册命令）
- Create: `app/src/lib/types.ts`、`app/src/lib/api.ts`、`app/src/lib/mock.ts`、`app/src/lib/clock.ts`、`app/src/lib/api.test.ts`、`app/src/lib/mock.test.ts`
- Test: `commands.rs` 末尾 `mod tests`

**Interfaces:**
- Produces（Rust 命令名 = 前端 `invoke` 名；前端参数名用 camelCase，Tauri 自动转成 snake_case）：

| 命令 | 参数 | 返回 |
|---|---|---|
| `app_info` | — | `AppInfo { version, cli_path: Option<String>, platform }` |
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
| `linear_login` | — | `Viewer`（在系统浏览器里打开授权页） |
| `linear_use_api_key` | `key: String` | `Viewer` |
| `linear_logout` | — | `()` |
| `linear_reconnect` | — | `LinearSetup` |
| `setup_action` | `action: SetupAction` | `String`（命令输出，清洗过） |
| `control_servers` | — | `ControlServers { claude: Option<bool>, codex: Option<bool> }`（执行者没装为 `None`） |
| `open_linear` | `url: String`（必须以 `https://linear.app/` 开头） | `()` |
| `choose_directory` | — | `Option<String>` |
| `autostart_enabled` / `set_autostart` | — / `enabled: bool` | `bool` / `()` |
| `quit` | `mode: QuitMode`（`"wait"` / `"stop"`） | `()` |
| `request_quit` | — | `()`（与托盘「退出」相同；宿主没起来时直接退出，供启动失败页用） |

  - 前端：`types.ts` 里与上面一一对应的类型；`interface Backend`（方法名为命令名的 camelCase，外加 `onEngineEvent`、`onResync`、`onQuitRequested`、`onStartupFailed` 四个订阅）；`const backend: Backend`（在 Tauri 里用真实命令，否则用假后端）；`class BackendError extends Error`；`createMockBackend(scenario: "demo" | "fresh", options?: { latencyMs?: number }): Backend`（`latencyMs` 默认 120，测试传 0）；`now(): Date`（假后端里固定为 `2026-10-12T09:00:00Z`，截图稳定）。
- Consumes: Task 3 的 `AppState`、`quit::finish`、`tray::refresh`、`events::*`；第一部分的 `setup`、`executor::resolve_program_in`、`doctor::SystemRunner::with_env`、`host::*`。

- [ ] **Step 1: Rust 命令的失败测试（纯逻辑部分）**

`commands.rs` 测试模块：

```rust
    #[test]
    fn only_linear_links_are_opened() {
        assert!(is_linear_url("https://linear.app/demo/issue/DEMO-1"));
        assert!(!is_linear_url("http://linear.app/demo/issue/DEMO-1"));
        assert!(!is_linear_url("https://linear.app.evil.example/x"));
        assert!(!is_linear_url("https://evil.example/https://linear.app/"));
        assert!(!is_linear_url("file:///etc/passwd"));
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
        assert_eq!(action_executor(RemoveControlServer(Claude)), Claude);
    }

    #[test]
    fn quit_modes_read_from_the_panel() {
        assert_eq!(serde_json::from_str::<QuitMode>("\"wait\"").unwrap(), QuitMode::Wait);
        assert_eq!(serde_json::from_str::<QuitMode>("\"stop\"").unwrap(), QuitMode::Stop);
        assert!(serde_json::from_str::<QuitMode>("\"later\"").is_err());
    }
```

Run: `cargo test -p issuerelay-app commands`
Expected: 编译失败。

- [ ] **Step 2: 实现 `commands.rs`**

```rust
//! The commands the panel calls. Each waits for the host, does one thing and returns something
//! the panel can show; an error comes back as one cleaned line. `mark_done`, `retry_job` and
//! `cancel_job` exist only here (design 5.6): IPC, the CLI and MCP never offer them. Plugins
//! (opener, dialog, autostart) are used from here, so the panel itself needs no plugin
//! permission.
use std::path::{Path, PathBuf};

use issuerelay_core::{
    config::{Binding, ExecutorKind, Settings},
    doctor::{DoctorReport, SystemRunner},
    engine::{IssueView, StatusSnapshot},
    executor,
    host::{Host, LinearSetup, StopMode},
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

use crate::{quit, state::AppState, tray};

type CmdResult<T> = Result<T, String>;

/// The most jobs one list returns.
const MAX_JOBS: usize = 500;
/// The most log lines one read returns.
const MAX_LOG_LINES: usize = 5000;

fn error(e: impl Into<anyhow::Error>) -> String {
    clean_text(&format!("{:#}", e.into()))
}

fn clamp_limit(limit: usize, max: usize) -> usize {
    limit.clamp(1, max)
}

/// Only Linear's own pages are opened from the panel; issue URLs come from Linear and are
/// shown, but a link elsewhere is never opened.
fn is_linear_url(url: &str) -> bool {
    url.starts_with("https://linear.app/")
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
}

#[derive(Serialize)]
pub struct ControlServers {
    pub claude: Option<bool>,
    pub codex: Option<bool>,
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
    }
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
pub async fn linear_login(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Viewer> {
    let host = host(&state).await?;
    let opener = app.clone();
    let viewer = host
        .login(move |url| {
            if let Err(e) = opener.opener().open_url(url, None::<&str>) {
                tracing::warn!(error = %e, "could not open the authorization page");
            }
        })
        .await
        .map_err(error)?;
    tray::refresh(&app, &host);
    Ok(viewer)
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

/// Runs a one-click setup command with the executor found on the resolved PATH.
#[tauri::command]
pub async fn setup_action(state: State<'_, AppState>, action: SetupAction) -> CmdResult<String> {
    let host = host(&state).await?;
    let env = host.env();
    let program = executor::resolve_program_in(action_executor(action), env.path.as_deref()).map_err(error)?;
    let runner = SystemRunner::with_env(env.shared());
    let output = setup::run(action, &program, cli_path().as_deref(), &runner).await.map_err(error)?;
    Ok(clean_text(&output.stdout))
}

#[tauri::command]
pub async fn control_servers(state: State<'_, AppState>) -> CmdResult<ControlServers> {
    let host = host(&state).await?;
    let env = host.env();
    let runner = SystemRunner::with_env(env.clone().shared());
    let mut found = [None, None];
    for (slot, kind) in found.iter_mut().zip([ExecutorKind::Claude, ExecutorKind::Codex]) {
        if let Ok(program) = executor::resolve_program_in(kind, env.path.as_deref()) {
            *slot = setup::control_server_registered(&program, &runner).await.ok();
        }
    }
    Ok(ControlServers { claude: found[0], codex: found[1] })
}

#[tauri::command]
pub async fn open_linear(app: AppHandle, url: String) -> CmdResult<()> {
    if !is_linear_url(&url) {
        return Err("only Linear links are opened".into());
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

/// The quit prompt's answer: wait for the runs, or stop them, then exit.
#[tauri::command]
pub async fn quit(app: AppHandle, state: State<'_, AppState>, mode: QuitMode) -> CmdResult<()> {
    let host = host(&state).await?;
    let mode = match mode {
        QuitMode::Wait => StopMode::WaitForRuns,
        QuitMode::Stop => StopMode::StopRuns,
    };
    tauri::async_runtime::spawn(quit::finish(app, host, mode));
    Ok(())
}

/// The same as the tray's Quit; the startup error page uses it when the host never started.
#[tauri::command]
pub fn request_quit(app: AppHandle) {
    quit::request(&app);
}
```

`lib.rs` 的 `.invoke_handler(...)` 写全命令列表：

```rust
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
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
            commands::linear_use_api_key,
            commands::linear_logout,
            commands::linear_reconnect,
            commands::setup_action,
            commands::control_servers,
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

- [ ] **Step 3: 前端类型与命令封装（先写失败测试）**

`app/src/lib/api.test.ts`：

```ts
import { describe, expect, test, vi } from "vitest";

describe("backend selection", () => {
  test("outside Tauri the panel uses the mock backend", async () => {
    vi.resetModules();
    const { backend, isTauri } = await import("./api");
    expect(isTauri()).toBe(false);
    const status = await backend.status();
    expect(status.status.viewer?.name).toBe("Jane Doe");
  });

  test("command errors become BackendError with the backend's text", async () => {
    vi.resetModules();
    vi.doMock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockRejectedValue("job 7 not found") }));
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const { backend, BackendError } = await import("./api");
    await expect(backend.job(7)).rejects.toBeInstanceOf(BackendError);
    await expect(backend.job(7)).rejects.toThrow("job 7 not found");
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    vi.doUnmock("@tauri-apps/api/core");
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
    await expect(backend.markDone(running.id)).rejects.toThrow("not awaiting acceptance");
  });

  test("the fresh scenario is a first run", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    expect((await backend.status()).linear.kind).toBe("not_configured");
    expect(await backend.bindings()).toEqual([]);
    expect((await backend.settings()).onboarding_completed).toBe(false);
    await backend.linearLogin();
    expect((await backend.status()).linear).toEqual({ kind: "connected", method: "oauth" });
  });

  test("settings keep the pause switch, as the engine does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await backend.pause();
    const settings = await backend.settings();
    await backend.saveSettings({ ...settings, paused: false, poll_interval_secs: 60 });
    expect((await backend.settings()).paused).toBe(true);
    expect((await backend.settings()).poll_interval_secs).toBe(60);
  });
});
```

Run: `pnpm --dir app test`
Expected: 失败（模块不存在）。

- [ ] **Step 4: 实现 `types.ts`、`api.ts`、`clock.ts`、`mock.ts`**

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
}

export type AuthMethod = "oauth" | "api_key";
export type LinearSetup =
  | { kind: "connected"; method: AuthMethod }
  | { kind: "not_configured" }
  | { kind: "interface_changed"; message: string }
  | { kind: "connect_failed"; message: string };
export interface StatusView { status: StatusSnapshot; linear: LinearSetup }

export type IssueStatus =
  | { kind: "eligible" }
  | { kind: "not_todo"; state: string }
  | { kind: "not_candidate" }
  | { kind: "waiting"; reason: string }
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
export interface ControlServers { claude: boolean | null; codex: boolean | null }
export interface AppInfo { version: string; cli_path: string | null; platform: string }
export type EngineEvent = { kind: "status" } | { kind: "job"; job: Job } | { kind: "config" };
export type QuitMode = "wait" | "stop";
```

（`DoctorReport.env.proxy` 在引擎里已经打过码（M2 第一部分 Task 1 修复：`Engine::doctor` 用 `proxy_for_display()`），面板拿不到原始凭据；Task 9 的 `maskCredentials` 再打一次码作为纵深防护，规则与 Rust 的 `env::mask_credentials` 一致。）

`app/src/lib/clock.ts`：

```ts
/** The mock backend's fixed "now", so relative times in screenshots do not drift. */
export const MOCK_NOW = new Date("2026-10-12T09:00:00Z");
let fixed: Date | null = null;

export function useFixedClock(at: Date | null): void {
  fixed = at;
}

export function now(): Date {
  return fixed ?? new Date();
}
```

`app/src/lib/api.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createMockBackend } from "./mock";
import type {
  AppInfo, Binding, ControlServers, DoctorReport, EngineEvent, IssueView, Job, LinearSetup,
  Project, QuitMode, Settings, SetupAction, StatusView, Viewer,
} from "./types";

export class BackendError extends Error {}

export interface Backend {
  appInfo(): Promise<AppInfo>;
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
  linearUseApiKey(key: string): Promise<Viewer>;
  linearLogout(): Promise<void>;
  linearReconnect(): Promise<LinearSetup>;
  setupAction(action: SetupAction): Promise<string>;
  controlServers(): Promise<ControlServers>;
  openLinear(url: string): Promise<void>;
  chooseDirectory(): Promise<string | null>;
  autostartEnabled(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<void>;
  quit(mode: QuitMode): Promise<void>;
  requestQuit(): Promise<void>;
  onEngineEvent(listener: (event: EngineEvent) => void): Promise<() => void>;
  onResync(listener: () => void): Promise<() => void>;
  onQuitRequested(listener: (running: number) => void): Promise<() => void>;
  onStartupFailed(listener: (message: string) => void): Promise<() => void>;
}

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

const tauriBackend: Backend = {
  appInfo: () => call("app_info"),
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
  linearUseApiKey: (key) => call("linear_use_api_key", { key }),
  linearLogout: () => call("linear_logout"),
  linearReconnect: () => call("linear_reconnect"),
  setupAction: (action) => call("setup_action", { action }),
  controlServers: () => call("control_servers"),
  openLinear: (url) => call("open_linear", { url }),
  chooseDirectory: () => call("choose_directory"),
  autostartEnabled: () => call("autostart_enabled"),
  setAutostart: (enabled) => call("set_autostart", { enabled }),
  quit: (mode) => call("quit", { mode }),
  requestQuit: () => call("request_quit"),
  onEngineEvent: (listener) => on("engine-event", listener),
  onResync: (listener) => on("engine-resync", () => listener()),
  onQuitRequested: (listener) => on("quit-requested", listener),
  onStartupFailed: (listener) => on("startup-failed", listener),
};

/** `?mock=fresh` in a browser starts the mock as a first run (no login, no bindings). */
function mockScenario(): "demo" | "fresh" {
  const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  return params.get("mock") === "fresh" ? "fresh" : "demo";
}

export const backend: Backend = isTauri() ? tauriBackend : createMockBackend(mockScenario());
```

`app/src/lib/mock.ts`：实现 `createMockBackend(scenario)`，内存里保存状态，行为与引擎一致到面板能观察的程度：

- 「demo」场景：查看者 `Jane Doe`（`user-1`）；项目 `Demo Project`（`proj-1`，团队 `team-1`）、`Demo Mobile`（`proj-2`）、`Docs Site`（`proj-3`）；绑定 `proj-1 → ~/code/demo-app`（Claude，完全放开）与 `proj-2 → ~/code/demo-mobile`（Codex，自动审查）；八条任务各占一个状态：`DEMO-12` 运行中、`DEMO-11` 排队中、`DEMO-10` 待验收（摘要「新增 hello.txt，已推送到 test 分支并读回部署」）、`DEMO-9` 待补充信息、`DEMO-8` 失败（原因 `run exceeded the time limit`）、`DEMO-7` 已完成、`DEMO-6` 已停止、`DEMO-5` 等待绑定（原因 `project Docs Site is not bound`）；需求列表含这些需求和一条 `DEMO-4`（状态 Backlog，`not_todo`）；时间都以 `MOCK_NOW` 为基准往前推；`DEMO-12` 的日志是几行 Claude 流式事件（`system/init`、一条助手文本、一次 `Bash` 工具调用和结果）；环境检查：Claude 就绪（2.1.285），Codex 已安装但未登录（问题「not logged in」），工作区 `label_ok: true`、`teams_checked: ["Demo"]`，环境 `path_source: "login-shell"`、`proxy_source: "system-settings"`、代理 `HTTPS_PROXY=http://127.0.0.1:7890`；设置为默认值且 `onboarding_completed: true`、`full_access_acknowledged: true`；控制服务：Claude 未注册、Codex 未安装（`null`）；开机自启关。
- 「fresh」场景：`linear` 为 `not_configured`、没有查看者、没有绑定和任务、需求列表报错「not connected to Linear: log in first」、`onboarding_completed: false`、`full_access_acknowledged: false`；`linearLogin()` 之后变成 demo 场景的连接状态（但仍没有绑定）。
- 写操作与引擎同规则：`markDone` 只接受待验收、`retryJob` 只接受失败 / 已停止 / 待补充信息、`cancelJob` 只接受排队 / 等待 / 运行中（运行中直接变已停止）、`saveSettings` 保留当前 `paused`、`saveBinding` 按 `project_id` 或 `id` 替换。每次改动后按引擎的规则发 `{kind: "job", job}`、`{kind: "status"}` 或 `{kind: "config"}` 事件。
- `linearUseApiKey("lin_api_bad")` 失败（「Linear did not accept the API key」），其他值成功；`setupAction` 返回一行假输出并把对应执行者的状态改成已配置；`chooseDirectory` 返回 `~/code/new-project`；`quit` 与 `requestQuit` 什么都不做。
- 每个方法 `await` 一个 0–150 ms 的小延迟（测试里为 0，用参数 `latencyMs` 控制，默认浏览器里 120 ms），让加载状态在浏览器里看得见。
- 构造时调用 `useFixedClock(MOCK_NOW)`。

Run: `pnpm --dir app test`
Expected: PASS。

- [ ] **Step 5: 文档与提交**

- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 面板命令（含仅面板可用的标记完成、重试、停止） | app/src-tauri/src/commands.rs |` 与 `| 前端数据层与假后端 | app/src/lib/ |`；第 7 节「外部接口变更规则」补一句：Rust 侧给面板的类型变了，同步改 `app/src/lib/types.ts` 和假后端。

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs
git commit -m "feat(app): add the panel commands and the typed front-end backend with a mock"
```

---

### Task 5: 面板外框、设计变量与数据钩子

设计第 9.2、9.3 节。这一任务定下所有页面共用的东西：外框（侧边导航、页头）、设计变量（浅色 / 深色、状态颜色）、状态标识、空 / 加载 / 错误三种状态、数据查询与引擎事件桥、首次运行跳转、启动失败页、退出提示。之后的页面任务只在这个框架里加页面。

**Files:**
- Create: `app/src/styles.css`（替换模板的 `index.css`）、`app/src/lib/theme.ts`、`app/src/lib/format.ts`、`app/src/lib/backend-context.tsx`
- Create: `app/src/components/StatusBadge.tsx`、`app/src/components/states.tsx`、`app/src/components/PageHeader.tsx`、`app/src/components/layout/AppShell.tsx`、`app/src/components/QuitDialog.tsx`、`app/src/components/StartupError.tsx`、`app/src/components/FirstRunGate.tsx`
- Create: `app/src/hooks/queries.ts`、`app/src/hooks/useEngineEvents.ts`
- Create: `app/src/pages/{Overview,Issues,Runs,RunDetail,Bindings,Doctor,Settings,Help}.tsx`（本任务只放页头占位，后续任务填内容）、`app/src/pages/onboarding/Onboarding.tsx`（占位）
- Create: `app/src/test/render.tsx`（测试辅助）
- Modify: `app/src/App.tsx`、`app/src/main.tsx`、五个翻译文件（`state`、`common`、`shell`、`quit`、`startup`、`format` 命名空间补齐）
- Test: `app/src/components/*.test.tsx`、`app/src/hooks/*.test.tsx`、`app/src/lib/format.test.ts`

**Interfaces:**
- Produces:
  - `BackendProvider({ backend, children })`、`useBackend(): Backend`
  - 查询键常量 `queryKeys`：`status`、`jobs`、`job(id)`、`log(id)`、`issues`、`bindings`、`projects`、`settings`、`doctor`、`controlServers`、`autostart`、`appInfo`
  - 查询钩子：`useStatus()`、`useJobs(limit = 200)`、`useJob(id)`、`useJobLog(id, { live })`（运行中每 1.5 秒刷新）、`useIssues()`、`useBindings()`、`useProjects(enabled)`、`useSettings()`、`useDoctor(enabled)`、`useControlServers()`、`useAutostart()`、`useAppInfo()`
  - 变更钩子（成功后按需失效相关查询，失败时用 sonner 显示错误的一行原文）：`usePause`、`useResume`、`usePollNow`、`useCancelJob`、`useRetryJob`、`useMarkDone`、`useSaveBinding`、`useRemoveBinding`、`useSaveSettings`、`useLinearLogin`、`useLinearApiKey`、`useLinearLogout`、`useLinearReconnect`、`useSetupAction`、`useSetAutostart`
  - `useEngineEvents()`：`status` → 失效 `status`；`job` → 失效 `status`、`jobs`、`job(id)`、`issues`；`config` → 失效 `settings`、`bindings`、`status`；`resync` → 失效全部
  - `StatusBadge({ state })`、`STATE_STYLE: Record<JobState, { icon, className }>`
  - `EmptyState({ icon, title, description, action? })`、`LoadingState({ rows? })`、`ErrorState({ error, onRetry? })`
  - `PageHeader({ title, description?, actions? })`
  - `formatRelative(iso, lang)`、`formatDateTime(iso, lang)`、`formatDuration(fromIso, toIso | null, lang)`
  - `renderWithProviders(ui, { backend?, route?, lang? })`（测试用；默认 `createMockBackend("demo", { latencyMs: 0 })`）
- Consumes: Task 2 的 i18n、Task 4 的 `Backend`、`createMockBackend`、`types`、`now()`

- [ ] **Step 1: 设计变量**

`app/src/styles.css`：保留 shadcn 初始化生成的颜色变量结构（浅色 `:root`、深色 `.dark`），并做以下约定：

- 主色（`--primary`）用靛蓝：浅色 `oklch(0.51 0.21 275)`，深色 `oklch(0.68 0.17 275)`；其余中性色用 shadcn 的 neutral。
- 字体：`--font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", "PingFang SC", "Hiragino Sans", "Microsoft YaHei UI", "Malgun Gothic", sans-serif;`，`--font-mono: "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;`；基础字号 14px，行高 1.5。
- 圆角 `--radius: 0.625rem`；间距只用 Tailwind 的 4px 刻度。
- 八种任务状态各一个变量（浅色 / 深色两套），并经 `@theme inline` 暴露为 `text-state-*`、`bg-state-*/15` 等工具类：

| 状态 | 变量 | 浅色 | 深色 | 图标（lucide） |
|---|---|---|---|---|
| 等待绑定 `waiting_binding` | `--state-waiting` | `oklch(0.70 0.15 75)` | `oklch(0.80 0.14 80)` | `Unplug` |
| 排队中 `queued` | `--state-queued` | `oklch(0.55 0.03 260)` | `oklch(0.72 0.03 260)` | `Clock` |
| 运行中 `running` | `--state-running` | `oklch(0.58 0.19 255)` | `oklch(0.72 0.15 250)` | `LoaderCircle`（旋转） |
| 待验收 `awaiting_acceptance` | `--state-awaiting` | `oklch(0.55 0.20 295)` | `oklch(0.72 0.16 295)` | `ClipboardCheck` |
| 待补充信息 `needs_info` | `--state-needs-info` | `oklch(0.66 0.17 50)` | `oklch(0.78 0.14 55)` | `MessageCircleQuestion` |
| 已完成 `done` | `--state-done` | `oklch(0.60 0.15 150)` | `oklch(0.75 0.14 150)` | `CircleCheck` |
| 失败 `failed` | `--state-failed` | `oklch(0.58 0.22 25)` | `oklch(0.72 0.18 25)` | `CircleX` |
| 已停止 `canceled` | `--state-canceled` | `oklch(0.58 0.01 260)` | `oklch(0.70 0.01 260)` | `CircleSlash` |

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
import type { Backend } from "@/lib/api";
import { BackendProvider } from "@/lib/backend-context";
import { createMockBackend } from "@/lib/mock";

export async function renderWithProviders(
  ui: ReactElement,
  { backend = createMockBackend("demo", { latencyMs: 0 }), route = "/", lang = "en" as Lang } = {},
) {
  // initI18n runs once per test file; later calls only switch the language, so tests print no
  // "already initialized" warning.
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
  for (const label of ["等待绑定", "排队中", "运行中", "待验收", "待补充信息", "已完成", "失败", "已停止"]) {
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
import { createMockBackend } from "@/lib/mock";
import type { EngineEvent } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { useEngineEvents } from "./useEngineEvents";

function Bridge() {
  useEngineEvents();
  return null;
}

test("engine events refresh exactly the data they touch", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let emit: (event: EngineEvent) => void = () => {};
  let resync: () => void = () => {};
  backend.onEngineEvent = async (listener) => ((emit = listener), () => {});
  backend.onResync = async (listener) => ((resync = listener), () => {});
  const { client } = await renderWithProviders(<Bridge />, { backend });
  const spy = vi.spyOn(client, "invalidateQueries");
  await waitFor(() => expect(emit).not.toBe(undefined));
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
  resync();
  expect(spy).toHaveBeenCalledWith();
});
```

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
  backend.onQuitRequested = async (listener) => ((ask = listener), () => {});
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
  backend.onQuitRequested = async (listener) => ((ask = listener), () => {});
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

test("a first run goes to the guide, later runs stay where they are", async () => {
  const routes = (
    <Routes>
      <Route element={<FirstRunGate />}>
        <Route path="/" element={<p>overview page</p>} />
      </Route>
      <Route path="/onboarding" element={<p>guide page</p>} />
    </Routes>
  );
  await renderWithProviders(routes, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("guide page")).toBeInTheDocument();
});

test("a configured install shows the page it was asked for", async () => {
  await renderWithProviders(
    <Routes>
      <Route element={<FirstRunGate />}>
        <Route path="/" element={<p>overview page</p>} />
      </Route>
      <Route path="/onboarding" element={<p>guide page</p>} />
    </Routes>,
  );
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
  await renderWithProviders(<StartupError message="already_running" />, { backend });
  expect(screen.getByText(/Another IssueRelay is already running/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Quit" }));
  expect(backend.requestQuit).toHaveBeenCalled();
});

test("other start failures show their own text", async () => {
  await renderWithProviders(<StartupError message="cannot open state.db: disk full" />);
  expect(screen.getByText("cannot open state.db: disk full")).toBeInTheDocument();
});
```

`app/src/components/states.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/api";
import { renderWithProviders } from "@/test/render";
import { ErrorState } from "./states";

test("errors show a friendly title, the backend's line and a retry", async () => {
  const retry = vi.fn();
  await renderWithProviders(<ErrorState error={new BackendError("Linear did not answer `list_issues` within 30s")} onRetry={retry} />);
  expect(screen.getByText("Something went wrong")).toBeInTheDocument();
  expect(screen.getByText("Linear did not answer `list_issues` within 30s")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(retry).toHaveBeenCalled();
});
```

`app/src/lib/format.test.ts`：

```ts
import { expect, test } from "vitest";
import { useFixedClock } from "./clock";
import { formatDuration, formatRelative } from "./format";

test("relative times and durations read naturally in each language", () => {
  useFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("2026-10-12T08:57:00Z", "en")).toBe("3 minutes ago");
  expect(formatRelative("2026-10-12T08:57:00Z", "zh-CN")).toBe("3分钟前");
  expect(formatDuration("2026-10-12T08:57:37Z", "2026-10-12T09:00:00Z", "en")).toBe("2 min 23 s");
  expect(formatDuration("2026-10-12T08:57:37Z", "2026-10-12T09:00:00Z", "zh-CN")).toBe("2 分 23 秒");
  expect(formatDuration("2026-10-12T07:00:00Z", null, "en")).toBe("2 h 0 min");
  useFixedClock(null);
});
```

（`formatRelative` 用 `Intl.RelativeTimeFormat`（`numeric: "auto"`），时长用翻译键 `format.duration_hm`、`format.duration_ms`、`format.duration_s`；上面期望值按 jsdom 的 ICU 输出核对，若 Node 的 ICU 给出的中文写法不同（例如带空格），以实际输出为准修改期望值并在报告里说明。）

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

Run: `pnpm --dir app test`
Expected: 失败（组件尚不存在）。

- [ ] **Step 3: 实现**

- `backend-context.tsx`：`createContext<Backend>(backend)`（默认值是 `api.ts` 的全局 `backend`）、`BackendProvider`、`useBackend`。
- `queries.ts`：每个钩子用 `useBackend()` 取后端；`useJobLog(id, { live })` 在 `live` 时 `refetchInterval: 1500`；变更钩子统一用一个 `useAction(fn, invalidate: QueryKey[])` 辅助：成功后失效给定的键，失败时 `toast.error(error.message)`（错误文本是后端清洗过的一行，作为纯文本显示）。
- `useEngineEvents.ts`：在 `useEffect` 里订阅 `onEngineEvent` 与 `onResync`，按 Interfaces 里的规则失效查询；卸载时取消订阅。
- `StatusBadge.tsx`：shadcn `Badge`（`variant="outline"`），左侧图标，文字为 `t("state.<state>")`；`running` 的图标加 `animate-spin`；颜色用 `text-state-*`、`border-state-*/30`、`bg-state-*/10`。
- `states.tsx`：`EmptyState` 居中、图标 40px、标题 + 说明 + 可选操作按钮；`LoadingState` 用 `Skeleton` 画 `rows` 行（默认 3）；`ErrorState` 用 `Alert`（destructive）：标题 `common.error_title`，正文是错误的一行原文（等宽字体、可选中），有 `onRetry` 时显示 `common.retry` 按钮。
- `PageHeader.tsx`：标题（`text-xl font-semibold`）、可选说明（`text-muted-foreground`）、右侧操作区。
- `AppShell.tsx`：左侧 232px 侧栏（应用名与图标、七个 `NavLink`：`/` 概览 `LayoutDashboard`、`/issues` 需求 `ListTodo`、`/runs` 运行记录 `History`、`/bindings` 项目绑定 `FolderGit2`、`/doctor` 环境检查 `Stethoscope`、`/settings` 设置 `Settings`、`/help` 使用说明 `CircleHelp`；当前页高亮），侧栏底部是监控状态（`shell.monitoring` 监控中 / `shell.paused` 已暂停 / `shell.draining` 正在等待运行结束 / `shell.problem` 需要处理，颜色同托盘的判定）和版本号；右侧 `<Outlet />`，内容区最大宽度 1080px、左右内边距 24px、可滚动。
- `QuitDialog.tsx`：订阅 `onQuitRequested`；shadcn `AlertDialog`：标题 `quit.title`，正文 `quit.body`（`count`），三个按钮 `quit.wait`、`quit.stop`、`quit.cancel`；选了等待后换成等待中的说明（`quit.waiting`）和一个 `quit.stop` 按钮。
- `StartupError.tsx`：整页居中卡片，标题 `startup.failed_title`；`message === "already_running"` 时正文为 `startup.already_running`，否则原样显示 `message`；按钮 `startup.quit` 调 `requestQuit()`。
- `FirstRunGate.tsx`：读 `useSettings()`；加载中显示 `LoadingState`；`onboarding_completed` 为假时 `<Navigate to="/onboarding" replace />`，否则 `<Outlet />`。
- `App.tsx`：`BackendProvider` → `QueryClientProvider` → `HashRouter`；路由：`/onboarding` 引导页（不带外框），其余页面在 `FirstRunGate` → `AppShell` 之下；全局挂 `QuitDialog`、`Toaster`、`useEngineEvents()`、`useSystemTheme()`；订阅 `onStartupFailed`，以及 `useStatus()` 报错且错误文本是 `already_running` 时，整页换成 `StartupError`；设置加载后若 `resolveLanguage(settings.language, navigator.language)` 与当前语言不同就 `setLanguage`。
- 页面占位：每个页面先只渲染 `PageHeader`（标题为导航名）。
- `main.tsx`：`await initI18n(resolveLanguage("system", navigator.language))` 后渲染 `<App />`。
- 翻译键：`shell.monitoring`、`shell.paused`、`shell.draining`、`shell.problem`、`shell.version`（`版本 {{version}}`）、`format.duration_hm`（`{{h}} 小时 {{m}} 分` / `{{h}} h {{m}} min`）、`format.duration_ms`、`format.duration_s`。五种语言同步补齐。

Run: `pnpm --dir app test && pnpm --dir app lint`
Expected: PASS。再 `pnpm --dir app dev` 在浏览器里打开 `http://localhost:1420/`，确认外框、导航、深浅色切换（改系统外观）正常，截一张图附在报告里（只用假数据）。

- [ ] **Step 4: 文档与提交**

- 设计第 9.3 节补：「主色靛蓝；系统字体栈（含中日韩字体）；八种状态的颜色与图标见 `app/src/styles.css` 与 `StatusBadge.tsx`。」

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs
git commit -m "feat(app): add the panel shell, design tokens and data hooks"
```

---

### Task 6: 概览页与需求页

设计第 9.2 节：概览（监控开关、上次检查时间、运行中与排队中的任务、最近结果、需要处理的提醒）；需求（当前用户所有带 `ai-ready` 的需求及其本地状态；操作：在 Linear 中打开、停止、重新运行、标记完成）。「标记完成」是程序唯一的 Linear 写操作，点之前要确认。

**Files:**
- Modify: `app/src/pages/Overview.tsx`、`app/src/pages/Issues.tsx`
- Create: `app/src/components/JobActions.tsx`（停止 / 重新运行 / 标记完成 / 在 Linear 中打开，概览、需求、运行详情共用）、`app/src/components/LinearStatus.tsx`
- Modify: 五个翻译文件（`overview`、`issues`、`actions`、`linear` 命名空间）
- Test: `app/src/pages/Overview.test.tsx`、`app/src/pages/Issues.test.tsx`、`app/src/components/JobActions.test.tsx`

**Interfaces:**
- Produces: `JobActions({ job, url? , compact? })`：按任务状态只显示能用的操作——排队 / 等待 / 运行中：停止；失败 / 已停止 / 待补充信息：重新运行；待验收：标记完成（确认对话框）；有 `url` 时：在 Linear 中打开。`LinearStatus()`：按 `StatusView.linear` 与 `status` 显示连接状态与对应操作（未登录 → 去登录；连接失败 → 重试；接口已变化 → 说明需要升级；已连接 → 授权人姓名与上次检查时间）。
- Consumes: Task 5 的钩子、`StatusBadge`、三种状态组件、`PageHeader`。

页面规格：

**概览**（路由 `/`）

| 区块 | 内容 | 数据 |
|---|---|---|
| 页头 | 标题 `nav.overview`；右侧「立即检查」按钮（`actions.poll_now`）与监控开关（`Switch`，开 = 监控中；关 = 已暂停，调 `pause` / `resume`） | `useStatus` |
| 连接卡 | `LinearStatus` | `useStatus` |
| 提醒 | 按需显示的 `Alert`：有失败任务（`overview.alert_failed`，带「查看」跳 `/runs?state=failed`）、有待验收（`overview.alert_awaiting`，跳 `/issues`）、Linear 接口已变化（`overview.alert_interface`）、最近一次检查报错（`status.last_error` 原文）、正在等待运行结束（`status.draining`） | `useStatus`、`useJobs` |
| 运行中 | 每条：编号、标题、执行者、已运行时长（每秒刷新）、`JobActions`（停止）；没有时 `EmptyState`（`overview.no_running`） | `useJobs` 过滤 `running` |
| 排队 | 排队中与等待绑定的任务（等待绑定的显示原因）；没有时一行淡色文字 | `useJobs` |
| 最近结果 | 最近 8 条已结束的任务（待验收 / 待补充信息 / 已完成 / 失败 / 已停止）：状态、编号、标题、结束时间（相对时间）、摘要或原因的第一行；点击进运行详情 | `useJobs` |

**需求**（路由 `/issues`）

| 区块 | 内容 |
|---|---|
| 页头 | 标题 `nav.issues`，说明 `issues.description`（「指派给你、带 {{label}} 标签的需求，最近更新的 50 条」）；右侧刷新按钮 |
| 列表 | `Table`：编号（等宽）、标题、Linear 状态、本地状态（有任务时 `StatusBadge`，没有时淡色「—」）、原因（`IssueStatus`：`eligible` → `issues.eligible`「下一次检查会接走」；`not_todo` → `issues.not_todo`「状态是 {{state}}，不是 {{todo}}」；`not_candidate` → `issues.not_candidate`；`waiting` → 原因原文；`paused` → `issues.paused`）、操作（`JobActions`，`url` 为需求链接） |
| 空状态 | 没有需求时 `EmptyState`：`issues.empty_title`「还没有带 {{label}} 标签的需求」、说明触发条件、按钮跳 `/help` |
| 错误状态 | `ErrorState`（例如未登录：后端返回「not connected to Linear: log in first」）＋ 按钮跳 `/onboarding` 或 `/doctor` |

- [ ] **Step 1: 失败测试**

`app/src/components/JobActions.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
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

test("a failed action shows the backend's reason", async () => {
  const { backend, job } = await jobIn("failed");
  backend.retryJob = vi.fn().mockRejectedValue(new Error("job 8 is not the latest run of DEMO-8; retry that one instead"));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Run again" }));
  expect(await screen.findByText(/not the latest run of DEMO-8/)).toBeInTheDocument();
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
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Overview from "./Overview";

test("the overview shows what runs, what waits and what needs attention", async () => {
  await renderWithProviders(<Overview />);
  const running = await screen.findByRole("region", { name: "Running" });
  expect(within(running).getByText("DEMO-12")).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Queued" })).toHaveTextContent("DEMO-11");
  expect(screen.getByText(/1 run failed/)).toBeInTheDocument();
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

test("a first run without a login points to the guide", async () => {
  await renderWithProviders(<Overview />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
  expect(screen.getByText("Nothing is running")).toBeInTheDocument();
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
  expect(screen.getByText(/project Docs Site is not bound/)).toBeInTheDocument();
});

test("without a Linear connection the list explains what to do", async () => {
  await renderWithProviders(<Issues />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText(/not connected to Linear/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open the guide" })).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按上面的页面规格实现。区块用 `<section aria-labelledby>` 包起来，标题即区块名（测试按 `role="region"` 找）。已运行时长用 `formatDuration(started_at, null, lang)`，每秒刷新一个本地计时器（组件卸载时清掉）。英文文案示例：`overview.alert_failed` = `{{count}} run failed` / 复数用 i18next 的 `_other`（例如 `"alert_failed_one": "{{count}} run failed"`, `"alert_failed_other": "{{count}} runs failed"`；中文只需 `_other`；五种语言的键要一致——i18next 在 `zh`、`ja`、`ko` 只用 `_other`，为了键集一致，五个文件都写 `_one` 与 `_other` 两个键，中日韩两者内容相同）。确认对话框文案 `actions.mark_done_confirm`：「把 {{identifier}} 设为 {{done}}？这是 IssueRelay 唯一会改动 Linear 的操作。」按钮 `actions.confirm`。

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

设计第 9.2 节「运行记录：历史运行列表；详情页显示可读的时间线和原始日志，支持复制」、6.1 节「输出逐行读取：原样写进日志文件，同时解析出面板显示用的时间线（AI 消息、工具调用、命令、错误）」。

**Files:**
- Create: `app/src/lib/timeline.ts`、`app/src/lib/timeline.test.ts`、`app/src/lib/__fixtures__/claude-stream.jsonl`、`app/src/lib/__fixtures__/codex-exec.jsonl`（从 `crates/core/tests/fixtures/` 复制，已脱敏）
- Modify: `app/src/pages/Runs.tsx`、`app/src/pages/RunDetail.tsx`、五个翻译文件（`runs`、`timeline`）
- Test: `app/src/pages/Runs.test.tsx`、`app/src/pages/RunDetail.test.tsx`

**Interfaces:**
- Produces:
  - `type TimelineEntry = { kind: "message" | "tool" | "command" | "result" | "error" | "stderr" | "system"; text: string; detail?: string; failed?: boolean }`
  - `parseTimeline(lines: string[], executor: ExecutorKind | null): TimelineEntry[]`
  - `stripAnsi(text: string): string`
- Consumes: Task 5 的 `useJobs`、`useJob`、`useJobLog`、`StatusBadge`；Task 6 的 `JobActions`。

解析规则（每行一个 JSON；解析不了的行算 `stderr`，原文保留）：

| 来源 | 事件 | 时间线条目 |
|---|---|---|
| Claude | `{"type":"system","subtype":"init",…}` | `system`：`timeline.session_started`（带模型名，若有） |
| Claude | `{"type":"assistant","message":{"content":[{"type":"text","text":…}]}}` | 每段文本一条 `message` |
| Claude | 同上，`{"type":"tool_use","name":…,"input":…}` | `tool`：`text` = 工具名（`Bash` 时为 `command`，`text` = `input.command`），`detail` = 输入的 JSON（截到 2000 字符） |
| Claude | `{"type":"user","message":{"content":[{"type":"tool_result","content":…,"is_error":…}]}}` | 并入上一条 `tool` / `command` 的 `detail`（结果截到 2000 字符），`is_error` 为真时 `failed: true` |
| Claude | `{"type":"result",…,"result":…,"is_error":…}` | `result`：`text` = `result`；`is_error` 为真时 `failed: true` |
| Codex | `{"type":"item.completed","item":{"type":"agent_message","text":…}}` | `message` |
| Codex | `{"type":"item.completed","item":{"type":"command_execution","command":…,"aggregated_output":…,"exit_code":…}}` | `command`：`text` = 命令，`detail` = 输出（截断），`exit_code` 非 0 时 `failed: true` |
| Codex | `{"type":"item.completed","item":{"type":"mcp_tool_call","server":…,"tool":…}}` | `tool`：`text` = `server.tool` |
| Codex | `{"type":"error","message":…}` 或 `{"type":"turn.failed",…}` | `error` |
| 两者 | IssueRelay 自己的 `{"stream":"stderr","text":…}` | `stderr`（经 `stripAnsi`） |
| 两者 | 其余事件（`thread.started`、`turn.started`、`item.started`、用量统计等） | 跳过 |

页面规格：

**运行记录**（`/runs`）：页头＋状态筛选（`Tabs`：全部、运行中、待验收、失败、其他；同步到地址栏 `?state=`）；列表（`Table`）：状态、编号、标题、执行者、开始时间（相对时间）、时长、摘要或原因的第一行；行可点击进详情；空、加载、错误三种状态。

**运行详情**（`/runs/:id`）：页头为编号＋标题，右侧 `JobActions`（`url` 由需求编号无法得知时不显示「在 Linear 中打开」；`Job` 没有链接字段，详情页从 `useIssues()` 里按 `issue_uuid` 找链接，找不到就不显示）；信息卡：状态、执行者、目录（等宽、可复制）、创建 / 开始 / 结束时间、时长、原因、摘要；`Tabs` 两页：时间线（按上表渲染：消息用正文样式，工具与命令为可展开的卡片，失败的标红，结果卡片放在最后）与原始日志（`ScrollArea` 内等宽显示 `stripAnsi` 之后的每一行，右上角「复制全部」）；运行中时日志每 1.5 秒刷新并自动滚到底（用户上翻后暂停自动滚动）；日志被清理（返回空）且任务已结束时显示 `runs.log_removed`。

- [ ] **Step 1: 失败测试**

`app/src/lib/timeline.test.ts`：

```ts
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";
import { parseTimeline, stripAnsi } from "./timeline";

const fixture = (name: string) =>
  readFileSync(join(__dirname, "__fixtures__", name), "utf8").split("\n").filter(Boolean);

test("a real Claude stream becomes a readable timeline", () => {
  const entries = parseTimeline(fixture("claude-stream.jsonl"), "claude");
  expect(entries[0].kind).toBe("system");
  expect(entries.some((e) => e.kind === "tool")).toBe(true);
  expect(entries.at(-1)?.kind).toBe("result");
  expect(entries.filter((e) => e.kind === "message").length).toBeGreaterThan(0);
});

test("a real Codex stream becomes a readable timeline", () => {
  const entries = parseTimeline(fixture("codex-exec.jsonl"), "codex");
  expect(entries.some((e) => e.kind === "message")).toBe(true);
  expect(entries.every((e) => e.text.length > 0)).toBe(true);
});

test("tool results join their call and failures are marked", () => {
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

test("unreadable lines and IssueRelay's own notes stay visible", () => {
  const lines = ["not json at all", JSON.stringify({ stream: "stderr", text: "\u001b[31mwarning\u001b[0m: slow" })];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "stderr", text: "not json at all" },
    { kind: "stderr", text: "warning: slow" },
  ]);
  expect(stripAnsi("\u001b[1;32mok\u001b[0m")).toBe("ok");
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
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import RunDetail from "./RunDetail";

async function open(state: string) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.listJobs(100)).find((j) => j.state === state)!;
  await renderWithProviders(
    <Routes><Route path="/runs/:id" element={<RunDetail />} /></Routes>,
    { backend, route: `/runs/${job.id}` },
  );
  return { backend, job };
}

test("a running job shows its timeline and raw log", async () => {
  const { job } = await open("running");
  expect(await screen.findByRole("heading", { name: new RegExp(job.issue_identifier) })).toBeInTheDocument();
  expect(await screen.findByText(/Session started/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByRole("button", { name: "Copy all" })).toBeInTheDocument();
});

test("a failed job shows why it failed", async () => {
  await open("failed");
  expect(await screen.findByText("run exceeded the time limit")).toBeInTheDocument();
});
```

`app/src/pages/Runs.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithProviders } from "@/test/render";
import Runs from "./Runs";

test("runs can be filtered by state", async () => {
  await renderWithProviders(<Runs />);
  expect(await screen.findByText("DEMO-7")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Failed" }));
  expect(screen.getByText("DEMO-8")).toBeInTheDocument();
  expect(screen.queryByText("DEMO-7")).not.toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

`timeline.ts` 按上表实现；`stripAnsi` 去掉 `ESC [ … 字母` 与 `ESC ] … BEL` 两类序列。复制用 `navigator.clipboard.writeText`，成功后 toast `common.copied`。页面按规格实现。

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
- Create: `app/src/pages/BindingDialog.tsx`、`app/src/components/FullAccessWarning.tsx`、`app/src/lib/model-name.ts`
- Modify: 五个翻译文件（`bindings`、`permission`、`executor`）
- Test: `app/src/pages/Bindings.test.tsx`、`app/src/pages/BindingDialog.test.tsx`、`app/src/lib/model-name.test.ts`

**Interfaces:**
- Produces: `BindingForm({ initial?, onSaved })`（对话框与首次引导共用的表单主体）；`isValidModelName(text: string): boolean`（与 Rust 的 `config::is_valid_model_name` 同规则：1–100 个 ASCII 字母、数字、`.`、`_`、`:`、`-`，不以 `-` 开头）。
- Consumes: Task 5 的钩子；`useProjects`、`useSaveBinding`、`useRemoveBinding`、`useDoctor`（检查结果）、`useSettings`（默认执行者与档位、`full_access_acknowledged`）、`useSaveSettings`。

页面与表单规格：

- 列表：每个绑定一张卡片：项目名、目录（等宽，过长时中间省略，悬停看全文）、执行者（「默认（Claude Code）」这类写法）、权限档位、模型（没有时「沿用执行者的配置」）、提示词覆盖（有时显示「有」并可展开）、检查结果（来自最近一次环境检查：目录存在、执行者就绪；没有检查过时显示「重新检查」按钮）、编辑与删除（删除要确认）。右上角「添加绑定」。没有绑定时 `EmptyState`（`bindings.empty`，按钮「添加第一个绑定」）。
- 表单字段：
  1. Linear 项目（`Select`，选项来自 `projects()`；已绑定的项目在新增时禁用并注明；加载失败时显示错误与重试）；
  2. 本地目录（只读输入框＋「选择…」按钮调 `chooseDirectory()`；必须是绝对路径）；
  3. 执行者（`Select`：默认 / Claude Code / Codex）；
  4. 权限档位（`Select`：默认 / 完全放开 / 自动审查 / 按个人设置；每档下方一行说明，取自设计 6.1：完全放开 = AI 可以在本机执行任何命令；自动审查 = 执行者自动审查每一步，Codex 明显更慢；按个人设置 = Claude 只做设置里预先允许的事，Linear 工具和 git 要预先允许，否则会失败；Codex 沿用 `~/.codex/config.toml`）；
  5. 模型（输入框，可空；非法时字段下方提示 `bindings.model_invalid`：「只能用字母、数字和 . _ : -，例如 sonnet 或 gpt-5-codex；带方括号的名字（如 sonnet[1m]）不支持」）；
  6. 提示词覆盖（多行，可空；说明「留空用设置里的默认模板；固定规则始终生效」）。
- 保存：`id` 新建时取项目 `id`；生效的档位是「完全放开」（选中完全放开，或选「默认」而默认档位是完全放开）且 `settings.full_access_acknowledged` 为假时，先弹 `FullAccessWarning`（第 14 节的风险说明：AI 可以在本机执行任意命令、使用你的全部凭据，约束只有提示词固定规则和仓库的 AGENTS.md；可以改用较低档位），确认后先 `saveSettings({...settings, full_access_acknowledged: true})` 再保存绑定；取消则不保存。保存失败（例如后端校验报目录不是绝对路径）在表单顶部显示错误原文。

- [ ] **Step 1: 失败测试**

`app/src/lib/model-name.test.ts`：

```ts
import { expect, test } from "vitest";
import { isValidModelName } from "./model-name";

test("model names follow the engine's rule", () => {
  for (const ok of ["sonnet", "claude-sonnet-5-5", "gpt-5-codex", "o3:high", "a.b_c"]) expect(isValidModelName(ok), ok).toBe(true);
  for (const bad of ["", "-rf", "sonnet[1m]", "a b", "模型", "x".repeat(101)]) expect(isValidModelName(bad), bad).toBe(false);
});
```

`app/src/pages/BindingDialog.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
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
  await userEvent.type(await screen.findByRole("textbox", { name: "Model" }), "sonnet[1m]");
  expect(screen.getByText(/names with brackets/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});
```

`app/src/pages/Bindings.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Bindings from "./Bindings";

test("bindings list their folder, executor and permission", async () => {
  await renderWithProviders(<Bindings />);
  expect(await screen.findByText("~/code/demo-app")).toBeInTheDocument();
  expect(screen.getByText("Full access")).toBeInTheDocument();
  expect(screen.getByText("Automatic review")).toBeInTheDocument();
});

test("removing a binding asks first", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const remove = vi.spyOn(backend, "removeBinding");
  await renderWithProviders(<Bindings />, { backend });
  await userEvent.click((await screen.findAllByRole("button", { name: "Remove" }))[0]);
  expect(remove).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(remove).toHaveBeenCalledWith("proj-1");
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

按规格实现。Radix `Select` 在 jsdom 里需要 `pointer` 事件的补丁：在 `src/test-setup.ts` 里补 `Element.prototype.hasPointerCapture`、`scrollIntoView` 等桩（Radix 文档的测试建议）。

Run: `pnpm --dir app test && pnpm --dir app lint`
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
- Create: `app/src/lib/mask.ts`（与 Rust `env::mask_credentials` 同规则：`://` 之前不含 `@` 和 `/` 的部分算协议头，其后到第一个 `/` 为止、最后一个 `@` 之前的用户信息换成 `***`；没有协议头时从开头算）、`app/src/components/ExecutorCheck.tsx`（引导页复用）、`app/src/components/WorkspaceCheck.tsx`（引导页复用）
- Modify: 五个翻译文件（`doctor`、`settings`、`help` 命名空间）
- Test: `app/src/pages/Doctor.test.tsx`、`app/src/pages/Settings.test.tsx`、`app/src/pages/Help.test.tsx`、`app/src/lib/mask.test.ts`

**Interfaces:**
- Produces: `ExecutorCheck({ report, onFixed })`、`WorkspaceCheck({ report, labelName })`、`maskCredentials(url: string): string`。
- Consumes: Task 5 的钩子（`useDoctor`、`useSetupAction`、`useControlServers`、`useSettings`、`useSaveSettings`、`useAutostart`、`useSetAutostart`、`useLinearLogin`、`useLinearApiKey`、`useLinearLogout`、`useLinearReconnect`、`useAppInfo`）。

页面规格：

**环境检查**（`/doctor`）——打开页面时自动跑一次 `doctor()`（它会现查，最多约一分钟），页头有「重新检查」；检查中显示进度说明 `doctor.checking`（「正在检查 Linear、Claude Code 和 Codex，最多约一分钟」）。四个区块：

1. Linear：授权人或错误原文；按钮按状态给出（登录 / 重新授权 / 重试连接 / 退出登录）。
2. 工作区：标签 `{{label}}` 是否存在；已检查的团队（`teams_checked`）与缺失的状态（`missing_states`）；缺失时说明在 Linear 的「设置 → 团队 → 工作流」里添加，并提供打开 `https://linear.app/` 的按钮（程序不代为创建）。
3. 执行者：Claude Code、Codex 各一张 `ExecutorCheck` 卡：已安装（版本）、已登录、Linear MCP 已连通三项，每项通过 / 不通过的图标；问题原文；修复按钮——没装：显示官方安装说明链接（`https://docs.anthropic.com/en/docs/claude-code/setup`、`https://developers.openai.com/codex/cli`，经 `openLinear` 打不开的外链，改为显示可复制的地址，不直接打开）；没登录：说明在终端运行 `claude` 并登录 / `codex login`；没配 Linear MCP：「一键添加」（`setupAction({kind: "add_linear_mcp"})`）然后「在浏览器授权」（`login_linear_mcp`，提示最多等 5 分钟）；每次修复完成后自动重新检查。
4. 运行环境：PATH 来源、代理来源与代理值（`maskCredentials` 打码）、读取问题原文；说明为什么需要它们（第 8 节最后一段的意思）。
5. 绑定：每个绑定的目录是否存在、执行者是否就绪。

**设置**（`/settings`）——一张表单，分组卡片，底部固定「保存」；保存成功 toast，失败显示原文；语言切换保存后立即生效（含托盘）。

| 分组 | 字段 |
|---|---|
| 通用 | 界面语言（跟随系统 / 简体中文 / 繁體中文 / English / 日本語 / 한국어）；AI 评论语言（跟随界面语言，或上述五种之一，存语言代码） |
| 监控 | 检查间隔（秒，≥ 5）、总并发上限（≥ 1）、单次运行时长上限（分钟，≥ 1）、无输出判定卡住（分钟，≥ 1） |
| 默认值 | 默认执行者、默认权限档位（选「完全放开」且未确认过时弹 Task 8 的 `FullAccessWarning`） |
| 提示词 | 默认模板（多行；说明固定规则始终生效、模板写团队流程，例如测试分支名、部署读回方式） |
| 通知 | 四个开关：开始运行、进入待验收、需要补充信息、失败 |
| 系统 | 开机自启（`setAutostart`，立即生效，不经「保存」）；自动更新（开关存进设置，M3 起生效，旁注 `settings.auto_update_note`） |
| MCP 控制服务 | Claude Code、Codex 各一行：已注册 / 未注册 / 执行者未安装；按钮「注册」/「移除」（`register_control_server` / `remove_control_server`）；命令行程序没找到时说明（开发版需先 `cargo build -p issuerelay-cli`） |
| 高级 | 触发标签名与四个状态名；个人 API key（密码输入框＋「使用这个 key」，成功后显示「已改用 API key」）；日志保留天数与总大小（MB） |

字段校验与后端一致（数值下限、名字非空且不含引号与控制字符），不合法时字段下方提示，「保存」禁用。

**使用说明**（`/help`）——静态内容，全部走翻译键：触发条件（指派给自己、`{{label}}` 标签、Todo、所属项目已绑定）；状态流转（Todo → In Progress → In Testing → 面板上标记完成 → Done，配八种本地状态的图例，用 `StatusBadge`）；需求的写法示例（一条清楚的需求和一条会被问回来的含糊需求）；常见问题（为什么没被接走、三档权限的区别、需要代理时怎么办、怎样让 Claude / Codex 调用 IssueRelay 的 MCP 控制服务、日志在哪里）。

- [ ] **Step 1: 失败测试**

`app/src/lib/mask.test.ts`：

```ts
import { expect, test } from "vitest";
import { maskCredentials } from "./mask";

test("credentials in proxy URLs are masked like the engine does", () => {
  expect(maskCredentials("http://jane:s3cret@proxy.example:8080")).toBe("http://***@proxy.example:8080");
  expect(maskCredentials("jane:s3cret@proxy.example:8080")).toBe("***@proxy.example:8080");
  expect(maskCredentials(" http://jane:s3cret@proxy.example:8080")).toBe(" http://***@proxy.example:8080");
  expect(maskCredentials("HTTPS_PROXY=http://jane:s3cret@proxy.example:8080")).toBe("HTTPS_PROXY=http://***@proxy.example:8080");
  expect(maskCredentials("http://jane:p@ss@proxy.example:8080")).toBe("http://***@proxy.example:8080");
  expect(maskCredentials("http://proxy.example:8080")).toBe("http://proxy.example:8080");
  expect(maskCredentials("localhost,127.0.0.1")).toBe("localhost,127.0.0.1");
});
```

`app/src/pages/Doctor.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Doctor from "./Doctor";

test("the check shows each executor and offers the fix it needs", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText("2.1.285")).toBeInTheDocument();
  expect(screen.getByText(/not logged in/)).toBeInTheDocument();
  expect(screen.getByText(/codex login/)).toBeInTheDocument();
  expect(screen.getByText("HTTPS_PROXY=http://127.0.0.1:7890")).toBeInTheDocument();
  expect(setup).not.toHaveBeenCalled();
});

test("adding Linear's MCP server runs the official command and checks again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  report.executors[0] = { ...report.executors[0], linear_mcp: false, problems: ["Linear MCP is not configured"] };
  const doctor = vi.spyOn(backend, "doctor").mockResolvedValueOnce(report);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Add Linear to Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "add_linear_mcp", executor: "claude" });
  await waitFor(() => expect(doctor).toHaveBeenCalledTimes(2));
});
```

`app/src/pages/Settings.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
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
      expect.objectContaining({ poll_interval_secs: 60, notifications: expect.objectContaining({ failed: false, run_started: true }) }),
    ),
  );
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

test("a rejected API key says so and changes nothing", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.type(await screen.findByLabelText("Personal API key"), "lin_api_bad");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText(/did not accept the API key/)).toBeInTheDocument();
});
```

`app/src/pages/Help.test.tsx`：

```tsx
import { screen } from "@testing-library/react";
import { renderWithProviders } from "@/test/render";
import Help from "./Help";

test("the help page states the trigger conditions in full", async () => {
  await renderWithProviders(<Help />, { lang: "zh-CN" });
  expect(await screen.findByText(/指派给自己/)).toBeInTheDocument();
  expect(screen.getByText(/ai-ready/)).toBeInTheDocument();
  expect(screen.getByText(/Todo/)).toBeInTheDocument();
  expect(screen.getByText(/已绑定/)).toBeInTheDocument();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按规格实现。外部安装说明链接只作为可复制文本显示（`open_linear` 只开 Linear 的链接，这是有意的限制）。

Run: `pnpm --dir app test && pnpm --dir app lint`
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
- Consumes: Task 8 的 `BindingForm`、Task 9 的 `ExecutorCheck`、`WorkspaceCheck`、Task 5 的钩子。

规格：

- 整页布局（不带侧栏）：左侧竖排步骤列表（当前步高亮，已完成打勾，可点回已完成的步骤），右侧当前步内容，底部「上一步 / 下一步」。顶部应用名与图标。
- 第 1 步：语言单选（跟随系统＋五种语言），选中立即切换界面并保存设置。
- 第 2 步：未连接时「在浏览器里授权 Linear」（`linearLogin()`，等待期间显示「请在浏览器里完成授权」与取消提示）；已连接时显示授权人；高级选项里可改用个人 API key。连上之前不能进下一步。
- 第 3 步：`WorkspaceCheck`（来自 `doctor()` 的工作区部分）；缺标签或状态时显示添加方法，但允许继续（「我稍后再加」）。
- 第 4 步：两个 `ExecutorCheck`；至少一个执行者就绪才能进下一步；都没就绪时说明需要先装好并登录 Claude Code 或 Codex。
- 第 5 步：嵌入 `BindingForm`；保存成功后可「再加一个」或进下一步；也允许「稍后再绑定」。
- 第 6 步：四个触发条件、一条示例需求（标题与描述各一句，取 `help` 命名空间的示例）、状态流转图例；按钮「开始使用」：`saveSettings({...settings, onboarding_completed: true})` 后跳到概览。
- 引导完成后再进入 `/onboarding`（例如从环境检查页的「重新走一遍引导」）时，从第 1 步开始，但各步显示当前真实状态。

- [ ] **Step 1: 失败测试**

`app/src/pages/onboarding/Onboarding.test.tsx`：

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import Onboarding from "./Onboarding";

async function start() {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
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

  expect(await screen.findByText(/ai-ready/)).toBeInTheDocument();
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
  const { backend } = await start();
  await backend.linearLogin();
  const report = await backend.doctor();
  vi.spyOn(backend, "doctor").mockResolvedValue({
    ...report,
    executors: report.executors.map((e) => ({ ...e, logged_in: false, problems: ["not logged in"] })),
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  await userEvent.click(screen.getByRole("button", { name: "Next" }));
  await userEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(await screen.findByText(/Install and log in to Claude Code or Codex first/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});
```

Run: `pnpm --dir app test`
Expected: 失败。

- [ ] **Step 2: 实现**

按规格实现。

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
- Create（脚本生成后提交）：`app/src-tauri/icons/*`（`pnpm tauri icon` 由 1024px PNG 生成的整套应用图标）、`app/src-tauri/icons/tray/{idle,running,paused,problem}.png`（Windows 彩色 32×32）、`app/src-tauri/icons/tray/macos/{idle,running,paused,problem}.png`（macOS 模板图 44×44，纯黑加透明度）、`app/public/icon.svg`（面板侧栏与关于页用）、`docs/design/icons/preview.png`（三套对比）
- Modify: `app/src-tauri/src/tray.rs`（macOS 用 `icons/tray/macos/*`，Windows 用 `icons/tray/*`）、`app/package.json`（`"icons": "node scripts/icons.mjs"`）

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
pnpm --dir app tauri icon app/icons-src/build/app-1024.png
```

（`tauri icon` 生成 `icons/` 下的 `icon.icns`、`icon.ico`、各尺寸 PNG 与 Windows Store 图标；脚本把渲染出的 1024px PNG 放在 git 忽略的 `app/icons-src/build/`。）`tray.rs` 的 `icon()` 按平台读对应目录。面板侧栏左上角与关于信息用 `public/icon.svg`。

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

- 设计第 9.6 节与第 18 节第 1 条补：「图标选定方案 <A/B/C>（M2 由主智能体代选，三套预览见 `docs/design/icons/preview.png`，用户可改选：改 `pnpm --dir app icons --option <x>` 后重新生成）。」`app/icons-src/README.md` 写明三套方案的含义与生成命令。

Run: 六条验证命令
Expected: 全部通过。

```bash
git add app docs
git commit -m "feat(app): add the app and tray icons"
```

---

### Task 12: 关键页面截图与截图比对

设计第 15 节「关键页面截图比对」、第 17 节 M2 完成标志「页面截图经用户确认」。截图只用假后端的虚构数据，可以进仓库；真实验收截图留在本机。

**Files:**
- Create: `app/playwright.config.ts`、`app/tests/screens.spec.ts`、`app/scripts/screenshots.ts`
- Create（生成后提交）：`docs/gates/screenshots/m2/{zh-CN,en}-{light,dark}-{overview,issues,runs,run-detail,bindings,doctor,settings,help,onboarding-1,onboarding-2,onboarding-4,onboarding-6}.png`
- Modify: `app/package.json`（`"screenshots": "playwright test --update-snapshots"`、`"test:screens": "playwright test"`）、`README.md`（中英文各放两张截图：概览、运行详情）

**Interfaces:**
- Produces: `pnpm --dir app screenshots`（重新生成基准截图）、`pnpm --dir app test:screens`（与基准比对，允许 `maxDiffPixelRatio: 0.01`）。只在本机运行，不进 CI（字体随系统不同，CI 上比对不稳定）。

- [ ] **Step 1: Playwright**

```bash
pnpm --dir app add -D @playwright/test
pnpm --dir app exec playwright install chromium
```

`app/playwright.config.ts`：`webServer` 用 `pnpm build && pnpm preview --port 4173`，`use.viewport` 为 1100×720，`snapshotPathTemplate` 指向 `../docs/gates/screenshots/m2/{arg}{ext}`，两个项目：`light`（`colorScheme: "light"`）与 `dark`（`colorScheme: "dark"`）。

`app/tests/screens.spec.ts`：对每个语言（`zh-CN`、`en`，经 `localStorage` 或地址参数让假后端把 `settings.language` 设成该语言——在 `mock.ts` 里支持 `?lang=` 参数）、每个页面（地址用 `#/`、`#/issues`、`#/runs`、`#/runs/<运行中任务的 id>`、`#/bindings`、`#/doctor`、`#/settings`、`#/help`；引导页用 `?mock=fresh#/onboarding` 并点到第 1、2、4、6 步）等待加载状态消失后 `expect(page).toHaveScreenshot(\`${lang}-${theme}-${name}.png\`, { maxDiffPixelRatio: 0.01, animations: "disabled" })`。运行中任务的计时器在假后端里用固定时钟，截图稳定。

- [ ] **Step 2: 生成并检查**

Run: `pnpm --dir app screenshots`，再 `pnpm --dir app test:screens`
Expected: 生成 48 张基准图（2 语言 × 2 主题 × 12 屏），第二条命令全部通过。实现者逐张检查：没有截断的文字、没有溢出、没有裸露的报错文本、深色下对比度足够；问题当场修页面并重新生成。报告里列出检查结果。

- [ ] **Step 3: 提交**

Run: 六条验证命令
Expected: 全部通过；`check-secrets` 不报截图（PNG 是二进制，`git grep -I` 跳过——截图里只有虚构数据）。

```bash
git add app docs README.md
git commit -m "test(app): add key page screenshots and a local screenshot comparison"
```

---

### Task 13: M2 实机验收（macOS）

由主智能体在本机执行（AGENTS.md：实机验收只由主智能体在沙箱项目与沙箱仓库上做）。设计第 15 节：完整流程（发现 → 运行 → In Testing → 标记完成），另外制造一次失败和一次超时；结果写 `docs/gates/<日期>-m2-acceptance.md`。Windows 待实机（设计第 18 节第 4 条）。

**需要用户的地方：** 桌面应用第一次读钥匙串里的 Linear 凭据时 macOS 可能弹出授权框，或需要在浏览器里重新授权 Linear——这两件事涉及用户的凭据，由用户点；主智能体只在那一步请用户操作，其余自行完成。

- [ ] **Step 1: 构建**：`pnpm --dir app tauri build --bundles app`（得到 `IssueRelay.app`）、`cargo build -p issuerelay-cli --release`，把命令行程序复制到 `IssueRelay.app/Contents/MacOS/issuerelay`（M3 由打包流程完成）。
- [ ] **Step 2: 从访达启动**（不经终端，验证登录 shell 的 PATH 与代理）：首次引导走一遍；环境检查页确认 PATH 来源为登录 shell、代理来源与值（打码）；Claude、Codex 都就绪。
- [ ] **Step 3: 沙箱流程**：绑定沙箱项目（目录 `~/issuerelay-sandbox/demo`）；在沙箱项目建一条需求（Claude）→ 被接走（记时）→ 待验收 → 面板「标记完成」→ Linear 为 Done；换 Codex 再跑一条；四种通知各出现一次（开始运行、待验收、需要补充信息、失败）；托盘四种图标都出现过；暂停 / 恢复（托盘与面板）；设置里把运行时长上限改为 1 分钟制造一次超时（改完恢复 60）；一条故意做不到的需求制造失败；运行中停止一次；「重新运行」一次。
- [ ] **Step 4: 退出流程**：有任务在跑时托盘「退出」→ 提示 → 选「等待完成」→ 结束后自动退出；再来一次选「立即停止」→ 运行记为已停止、沙箱里没有残留进程（`ps -axo pid,ppid,pgid,command | grep -E 'claude|codex'`）、没有 `.git/index.lock`。
- [ ] **Step 5a: Linear 口径**：建一条中文标题的沙箱需求，确认提示词里的链接只保留到 `/issue/<编号>`，且这个链接在浏览器里能打开同一条需求（M1 验收 H2，M2 第一部分 Task 6）；若工作区有多个团队，确认环境检查只检查已绑定项目所属的团队，`teams_checked` 与实际一致（截图不进仓库）。
- [ ] **Step 5: 其他**：开机自启开关（打开后 `~/Library/LaunchAgents/` 出现条目，关闭后消失，不真的注销重登）；MCP 控制服务注册到 Claude Code（`claude mcp get issuerelay` 退出码 0），再移除；切换五种语言，托盘菜单与通知跟着变；关窗口后程序留在托盘、程序坞图标消失；第二次打开应用只把面板带到前面。
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
- 类型一致：前端 `types.ts` 与 Rust serde 输出逐字段对应（Task 4）；命令名表（Task 4）与 `api.ts`、`generate_handler!` 一致；事件名常量（Task 3）与 `api.ts` 一致。
