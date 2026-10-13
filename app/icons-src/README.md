# 图标源文件

IssueRelay 的应用图标与托盘图标：三套候选方案的 SVG 源文件、设计说明和生成命令（设计第 9.6 节）。图形都是原创，不含商标或徽标，也不模仿 Linear、Claude、OpenAI 的标志。

状态：选定方案 A「接力」（M2 由主智能体代选，用户可以改选，见文末）。`app/src-tauri/icons/` 里的应用图标与托盘图、`app/public/icon.svg` 都是 A 的生成结果。B、C 的源文件留在这里，三套的对比图是 `docs/design/icons/preview.png`。

## 共同的设计

- 主题：把需求交接给 AI 去执行。
- 颜色最多三种：靛蓝渐变（上 `#666af9`，下 `#3633bf`，中间约 `#4e4edc`，与面板主色 `oklch(0.51 0.21 275)` 同色系）、白色、一种浅靛色 `#cdd5ff`。
- 应用图标：1024 画布，居中一个 824 的圆角方形（圆角 185），按 macOS 的网格；Windows 与安装包用同一张图。字形不靠方形底也成立：托盘的彩色版就是没有方形底的同一字形。
- 托盘图标：同一字形的两个版本。macOS 用单色模板图，只有黑色和透明度，系统按菜单栏深浅自动反色；Windows 用彩色版，靛色比应用图标浅一点（`#6265ed`），深色任务栏上也看得清。
- 状态角标：字形留出右下角，脚本在那里叠加角标，先在字形上挖一圈缺口，免得角标贴在字形上。空闲没有角标；运行中是实心圆点；已暂停是两条竖杠；出错是感叹号。Windows 的角标是状态色的圆盘加白色标记（运行中蓝、已暂停灰、出错红）。

## 方案 A「接力」（`option-a/`）

斜放的接力棒加两条向左的运动线。棒是白色胶囊，应用图标里多两道靛色环带；运动线是浅靛色，一长一短。动感最强。托盘字形是胶囊加两条线，16 px 下胶囊读得清，运动线只剩淡淡的痕迹。环带只画在应用图标里：16 px 下它们会糊成一片。短处：单看像一根斜放的圆柱，「接力棒」要靠运动线和名字才读得出。

## 方案 B「交接」（`option-b/`）

左下一个小圆点，右上一个大圆点，一条圆弧带箭头从小点连到大点：需求从人手里交到 AI 手里。最简洁，线条最少。右下角整块是空的，角标压不到字形。短处：「运行中」的实心圆点角标紧挨着字形里的圆点，要靠它周围那圈缺口才看得出是角标。

## 方案 C「工单＋启动」（`option-c/`）

一张工单卡片，里面一个向右的三角：运行这一条。卡片两侧有票根缺口；应用图标里还有标题行和一条虚线，把卡片分成标题和正文两段。托盘字形是实心卡片挖掉一个三角，16 px 下三套里最清楚。短处：卡片加三角也像「视频文件」，工单的意思靠缺口和标题行，缩到 16 px 就看不出了。

## 文件

每套方案一个目录 `option-<a|b|c>/`：

| 文件 | 内容 |
|---|---|
| `app.svg` | 应用图标，1024 × 1024；方形底的投影是名叫 `lift` 的滤镜，`public/icon.svg` 裁掉透明边时一并去掉它 |
| `tray-mono.svg` | 托盘字形，macOS 模板图；32 × 32 网格，墨色用 `currentColor`（根元素的 `color` 是黑色） |
| `tray-color.svg` | 托盘字形，Windows 彩色版；32 × 32 网格 |

角标不在 SVG 里，由 `app/scripts/icons.mjs` 叠加（位置、形状、颜色都在那个脚本里）。改字形时别占右下角。

## 生成

在仓库根目录运行：

```bash
pnpm --dir app icons --preview          # 三套的对比图：docs/design/icons/preview.png
pnpm --dir app icons --option <a|b|c>   # 一套方案的全部文件
pnpm --dir app tauri icon icons-src/build/app-1024.png   # 由 1024 px 图生成桌面应用图标
```

`--option` 写出：

- `app/icons-src/build/app-1024.png`：1024 px 的应用图标，`tauri icon` 的输入；git 忽略，不提交。
- `app/src-tauri/icons/tray/macos/<状态>.png`：macOS 模板图，44 × 44。
- `app/src-tauri/icons/tray/<状态>.png`：Windows 彩色托盘图，32 × 32。
- `app/public/icon.svg`：去掉透明边的应用图标，面板侧栏左上角与首次引导的页眉用（`AppShell.tsx`、`Onboarding.tsx`）。

状态是 `idle`、`running`、`paused`、`problem`。加 `--out <目录>` 把同样的目录树写到那个目录下，不动仓库里的文件，用来试渲染。

`tauri icon`（`@tauri-apps/cli` 2.12）会同时写出 `icons/android/` 和 `icons/ios/`，删掉，不提交：本项目只做桌面。它还会写 `64x64.png`，那是桌面图标，要提交。`tauri.conf.json` 的 `bundle.icon` 列的五个文件（`32x32.png`、`128x128.png`、`128x128@2x.png`、`icon.icns`、`icon.ico`）都在它的输出里，换方案时不用改。

脚本会检查：每个 SVG 的 `viewBox`；macOS 模板图的每个像素都是黑色加透明度；四种状态的图两两不同；应用图标的四角透明、中心不透明。`--option` 的检查不过，就不写任何文件。

## 改选方案

要换成 B 或 C：

1. `pnpm --dir app icons --option <b|c>`，再 `pnpm --dir app tauri icon icons-src/build/app-1024.png`，删掉 `app/src-tauri/icons/android/` 和 `ios/`。
2. 把本文开头的状态行、设计文档 9.6 与 18 的第 1 条、`docs/ai-coding/AI_DIRECTORY_STRUCTURE_GUIDE.md` 里的「选定的是 A」改成新的选择。
3. 一起提交生成的图标（`app/src-tauri/icons/`、`app/public/icon.svg`）和这些文档。托盘图用 `include_bytes!` 编进程序，下次构建就换上了。
