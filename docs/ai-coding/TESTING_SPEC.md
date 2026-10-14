# 测试分层规范

## 定位

IssueRelay 驱动的是真实的 Linear 和真实的 AI。纯逻辑在单元测试里完整覆盖；与外部的交互用假实现做集成测试；真实环境只在沙箱上验收，并写成报告。「测试通过」和「实机验收通过」永远分开记录。

## 分层

| 层 | 工具 | 对象 | 何时跑 |
|---|---|---|---|
| 单元测试 | `#[test]` / `#[tokio::test]`，写在源文件末尾的 `mod tests` | 配置、触发、调度、提示词、命令拼装、结果识别、解析、格式化 | 每次提交 |
| 集成测试 | `crates/*/tests/`，加上 `tests/common/` 的假实现 | 运行器 + `fake-agent`、Linear MCP + 假服务端、引擎 + 假 Linear + 脚本化运行器、IPC | 每次提交 |
| 平台测试 | `#[cfg(windows)]` / `#[cfg(unix)]` 的测试，CI 两个平台都跑 | `.cmd` 包装、命名管道、残留的套接字文件 | 推 main 与开 PR 时 |
| 前端组件测试 | Vitest + Testing Library（jsdom；公共设置在 `app/src/test-setup.ts`），`pnpm --dir app test` | 面板的组件、页面与数据层，对着内置假后端，不连引擎 | 每次提交；CI 两个平台都跑 |
| 截图比对与版面检查 | Playwright（`app/tests/screens.spec.ts`，用本机装的 Google Chrome），`pnpm --dir app test:screens` | 面板各屏的基准截图（`docs/gates/screenshots/m2/`，只含虚构数据）；最小窗口下的版面（jsdom 没有版面，只能在真浏览器里量） | 改界面时在本机跑；字体因机器而异，CI 不跑 |
| 实机验收 | 开发宿主（M2 起是托盘程序）+ 真实 Linear + 真实 Claude / Codex | 完整流程、耗时、失败与超时 | 每个里程碑收尾，写 `docs/gates/` |

## 单元与集成测试规则

- 不访问网络，不碰真实钥匙串，不依赖本机装的 `claude` / `codex`。
- 与时间有关的上限用毫秒级（例如卡住判定 400 毫秒），不写长时间的 `sleep`。
- 会等异步事件的集成测试给整个测试体套一个总上限（例如 `tests/host.rs` 的 `bounded`，20 秒），启动、停止和等运行的地方再各套较短的上限：CI 没有单个测试的超时，挂住的测试会拖住整个任务。
- 临时文件一律用 `tempfile`，不写本机固定路径。
- 应用 crate（`app/src-tauri`）链接了 Tauri：它的测试可执行文件在 Windows 上要带 Common Controls v6 清单，否则一启动就失败（`STATUS_ENTRYPOINT_NOT_FOUND`，tauri-apps/tauri#13419）。`build.rs` 已把清单嵌进该 crate 的所有可执行文件；这一点只有 CI 的 Windows 任务能验证。
- 每个状态、每个错误分支至少一个测试；修 bug 先写一个能复现它的测试。
- 测试数据只用虚构值：`DEMO-1`、`Demo Project`、`Jane Doe`、`user-1`、`proj-1`。

## 前端测试规则

- Vitest 只收 `app/src/**/*.test.{ts,tsx}`，测试文件与被测文件放在一起。`vite.config.ts` 开了 `globals: true`（`test`、`expect`、`vi`、`afterEach` 全局可用，Testing Library 也靠它在每个测试后自动清理）和 `restoreMocks: true`；从 `vitest` 显式导入也可以。
- 公共设置 `app/src/test-setup.ts`：Radix 需要的 jsdom 桩和 `matchMedia` 桩；每个测试后复位固定时钟、收掉 sonner 的提示；用英文启动 i18next 并装上缺键守卫（`t()` 用到 `en.json` 没有的键，或用到指向一组文案的键，就抛错）；React 报「not wrapped in act」时让那个测试失败（在测试文件自己的 `afterEach` 与清理之后才检查，清理期间的更新也算）。所以测试要等它引起的读取与更新做完（`findBy…`、`waitFor`、等查询缓存不再读取），再断言或结束。
- 要后端、查询缓存、路由或翻译的组件用 `app/src/test/render.tsx` 的 `renderWithProviders`：它用应用自己的 `createQueryClient()` 和不带延迟的假后端（`createMockBackend("demo", { latencyMs: 0 })`），可以指定 `backend`、`route`（起始地址）和 `lang`。只做展示的组件可以直接 `render`。
- 假后端就是测试替身：场景用 `demo`、`fresh`、`connecting`；要某个调用失败，就把这一个方法换成 `vi.fn().mockRejectedValue(new BackendError("…"))`，要看调用就用 `vi.spyOn`。测试依赖的确切字符串都定义在 `mock-fixtures.ts`。除了检查边界的 `api.test.ts`，测试不 mock `@tauri-apps/api`。
- 等条件，不等固定时间：用 `findBy…`、`waitFor`，或等查询缓存的状态（`client.getQueryState(…)`）；要推进时间就开假定时器，用 `app/src/test/time.ts` 的 `advanceUntil` 推到条件成立为止。不写固定时长的等待。和「几分钟前」、时长有关的断言依赖固定时钟（`MOCK_NOW`，假后端创建时设好，每个测试后复位）。
- 按角色和可读名字找元素（`getByRole`、`getByLabelText`）；`data-testid` 只用于测试自己的探针组件（例如显示当前地址的那个）。
- 测试默认用英文，按英文文案找元素：改英文文案要同步改钉住它的测试；别的语言用 `lang` 参数单独断言。
- 每个读数据的页面至少有一个用失败的假后端驱动的错误状态测试（`AI_CODING_GUIDE.md` 第 12.6 节）。
- `styles.test.ts` 以文本读 `styles.css?raw`（Vitest 只给这个文件保留 CSS 内容），核对状态色在两种主题下的对比度、布局常量与字体。

## 截图比对与版面检查（只在本机）

- 跑法：`pnpm --dir app test:screens`。Playwright 自己执行 `pnpm build && pnpm preview --port 4173 --strictPort` 起面板、跑完自己停掉（不借用已在跑的服务；4173 被占用时直接失败，不会让 vite 悄悄换到别的端口、Playwright 却去测 4173 上别处的页面）；浏览器是本机装的 Google Chrome（`channel: "chrome"`），不下载浏览器。每屏在 1100×720 下拍浅色与深色各一张，语言用 `zh-CN` 与 `en`（地址的 `?lang=`），数据来自假后端（`?mock=` 选场景），时区固定为 UTC，所以截图里的时间不随机器变。
- 基准图在 `docs/gates/screenshots/m2/`，名字是 `<语言>-<主题>-<屏>.png`：八个页面加首次引导的第 1、2、4、6 步，共 48 张。基准图里只有假后端的虚构数据；换了文案或假后端的数据，基准图要重拍。
- 重拍：`pnpm --dir app screenshots`（即 `playwright test --update-snapshots`，不带模式的写法在 Playwright 1.64.0 里等于 `changed`，2026-10-14 核对）只重写与现有基准图不再相符的图，所以改完要看 `git diff --stat` 与新图；它也跑下面的版面检查，版面检查不过它也不过。改动小于下面说的容差时它不会重写，要整套重拍就用 `pnpm --dir app exec playwright test --update-snapshots=all`。
- 容差：一屏最多 30 个像素的颜色差超过 0.05（`maxDiffPixels: 30`、`threshold: 0.05`，Playwright 的颜色距离，0 到 1）。同一台机器上同一页的渲染几乎完全一样：2026-10-14 在零容差下连跑两次，只有 zh-CN 的环境检查页差了 8 到 9 个像素。应用图标（侧栏与首次引导页眉的 `/icon.svg`）遮住不比，Chrome 每次画它的渐变与阴影都差几十个像素；遮罩用图标自己的靛蓝 `#4e4edc`，不用 Playwright 默认的品红，README 引用的截图里才不会有一块品红。当天的证明（改一个设计变量、跑、改回、跑）：浅色 `--primary` 色相 275→250，36 个截图测试里 12 个失败；`--state-running` 色相 255→200（亮度不变），8 个失败；浅色 `--sidebar` 0.985→0.9，18 个失败；`--page-padding` 1.5rem→1.25rem，36 个失败；改回后都是 84 个全过，不改连跑三次也全过。阈值用 0.1 时，上面那个状态色的改动一个也拦不住。拦不住的是几个 RGB 单位的颜色微调，那要靠组件测试和 `styles.test.ts`；窗口尺寸靠同一个文件里的版面检查。
- 只在本机跑，不进 CI：面板用系统字体，同一页在不同机器上像素不同，CI 的机器上比对不稳定；它能防的是同一台机器上页面的意外变化。换了机器或升了 Chrome 的大版本，先整套重拍再比对。
- 版面检查（同一个文件，不拍图）：最小窗口 900×600 与带 Windows 滚动条的 885×600 下，需求与运行记录两页先确认有演示的 9 行数据，再查没有横向溢出、每个按钮都在容器的右边界之内、列名不在一个词中间断行（中文的列名一行放下）；绑定对话框（添加与编辑）装得进窗口、「取消」「保存」看得见；首次引导每一步的「上一步 / 下一步」看得见，前五步顶栏的出口也看得见（首次运行的最后一步，「开始使用」就是出口，顶栏不重复它）；设置里「把命令行加入 PATH」打开之后，开关、shell 那一行与「复制」按钮都在窗口里，「系统」卡片没有横向溢出；概览页顶部的更新横幅（有新版本，以及装不上的更新带一段约 200 个字符、含不能断开的长文件名的原因）的文字与按钮不超出页面，设置里版本与「检查更新」那一行（同样两种状态）的按钮在窗口里、「系统」卡片没有横向溢出。
- `app/tests/*.spec.ts` 不是 Vitest 的测试：Vitest 只收 `app/src/**/*.test.{ts,tsx}`；`app/test-results/` 与 `app/playwright-report/` 是输出，git 忽略。

## 面板与 core 共用的用例表

面板重复 core 的规则时，规则写成一张 JSON 用例表，两边的测试读同一个文件：TS 直接导入 JSON，Rust 用 `include_str!("../../src/…")` 读，不复制成第二份。改规则时先改表，两边的测试都要过。

| 用例表 | TS 一侧 | Rust 一侧 |
|---|---|---|
| `app/src/lib/clean-cases.json` | `clean.test.ts`（`cleanDisplayText`） | `app/src-tauri/src/commands.rs` 的测试，对照 core 的 `clean_text`（`ansi: true` 的转义序列用例只在面板） |
| `app/src/lib/model-name-cases.json` | `model-name.test.ts`（`isValidModelName`） | `commands.rs` 的测试，对照 `config::is_valid_model_name` |
| `app/src/lib/mask-cases.json` | `mask.test.ts`（`maskCredentials`） | `commands.rs` 的测试，对照 `env::mask_credentials` |
| `app/src/lib/problem-cases.json` | `problem.test.ts`（侧栏的「需要处理」） | `app/src-tauri/src/tray.rs` 的测试（托盘图标） |
| `app/src/i18n/language-cases.json` | `i18n.test.ts`（`resolveLanguage`） | `app/src-tauri/src/i18n.rs` 的测试（`resolve_language`） |
| `app/src/lib/trigger-defaults.json` | `useTriggerNames`、假后端的默认设置 | `commands.rs` 的测试，对照 `TriggerNames::default()` |
| `app/src/lib/shell-line.json` | `mock-fixtures.ts` 的 `DEMO_SHELL_LINE`（Settings 的测试按它找那一行） | `app/src-tauri/src/commands.rs` 的测试，对照 core 的 `cli_path::SHELL_LINE` |
| `app/src/lib/update-failure-cases.json` | `errors.test.ts`（`UPDATE_FAILURE_CODES`、`ERROR_CODES`、`updateFailureKey` 与 `errors.*` 文案） | `app/src-tauri/src/update.rs` 的测试（`FailureCode` 的 serde 名与 `error_code`） |

前端测试要用 core 的样本时直接读 `crates/core/tests/fixtures/`（`timeline.test.ts` 以 `?raw` 导入 `claude-stream.jsonl`、`codex-exec.jsonl`），`vite.config.ts` 只在测试运行时允许读仓库根目录。用例里的不可见字符写成转义（`SECURITY_CODING_SPEC.md` 第 7 节）。

## 假实现清单

| 假实现 | 位置 | 替代什么 |
|---|---|---|
| `fake-agent` | `crates/core/src/bin/fake_agent.rs` | Claude Code / Codex 进程（按脚本输出、退出码、启动孙进程，包括离开运行进程组的孙进程） |
| `FakeLinearServer`、`PartialServer` | `crates/core/tests/common/fake_server.rs` | Linear MCP 服务端（经内存管道连接）：按 `tests/fixtures/linear-tools.json` 校验每次调用的参数，只返回请求的 `fields`，`tools/list` 里 `fields` 参数的取值取自同一份记录（连接时的检查才有东西可查），可编程返回文本（也可以逐页给不同的文本）、工具错误或永不应答 |
| `FakeLinear` | `crates/core/tests/common/fake_linear.rs` | `LinearApi` 的内存版；`fail_next` 让下一次任意调用失败，`fail_calls` 让接下来若干次调用失败，`fail_issue_reads` 让某条需求的单条读取一直失败（直到 `clear_issue_failures`）；`set_teams`、`set_projects`（含项目所属团队）、`set_team_statuses` 设置团队、项目和按团队的状态，不设置时是默认的 `team-1` 与 `proj-1`；`set_viewer_name` 改查看者的名字（默认 `Jane Doe`） |
| 内存钥匙串 | `crates/core/src/linear/auth.rs` 的测试模块 | 系统钥匙串（测试不碰真实钥匙串，每个测试用自己的服务名） |
| `ScriptedRunner`、`FakeReadiness` | `crates/core/tests/common/engine_kit.rs` | 子进程运行与环境检查（`ScriptedRunner` 报告的进程号是虚构的，只进存储，不能让宿主去清扫） |
| `FakeConnector`、`LingeringRunner`、`PanickingReadiness`、`PanicsOnMessage` | `crates/core/tests/host.rs` | 宿主的 Linear 连接（按次序给出已存凭据的结果，可让 `stored()` 等到放行；记一份内存钥匙串，可让 `login` 存完后等放行、让 `logout` 失败）、被告知停止后仍不结束的运行、会在引擎循环里 panic 的环境检查、在某条日志处 panic 的 tracing 订阅者（引擎在持锁的段落里出 bug，锁随之中毒；用它的测试另外登记一个空闲的订阅者，否则 tracing 会按并行测试线程的默认订阅者把这条日志缓存成「不要」） |
| `Orphan` | `crates/core/tests/common/orphan.rs` | 宿主崩溃后留下的执行者（macOS：领着自己进程组的 `fake-agent`）；测试结束时没被回收就整组结束并回收 |
| `FakeRunner` | `crates/core/src/doctor.rs` 的测试模块 | 检查命令的输出（计数调用）；`SystemRunner` 本身用 `fake-agent` 测超时 |
| `FakeSource` | `app/src-tauri/src/update.rs` 的测试模块 | 更新插件（按脚本给出检查结果与下载结果，并计数下载）；测试不访问网络 |
| `Recorder` | `crates/core/src/setup.rs` 的测试模块 | 设置命令的运行：记录每次调用的程序、参数与时间上限，按设定答复输出，或报告无法运行 |
| `BarrierRunner` | `crates/core/src/doctor.rs` 的测试模块 | 检查命令：要求几条命令同时在运行，检查才能结束（证明它们真的并行） |
| `createMockBackend`（面板的假后端） | `app/src/lib/mock.ts`，数据在 `app/src/lib/mock-fixtures.ts` | 面板的 Tauri 命令与事件（`Backend`）：组件测试、浏览器开发与截图共用，规则与报错文本照引擎；数据全部虚构，后续测试依赖的确切字符串都定义在 `mock-fixtures.ts`；场景 `demo`、`fresh`（首次运行）、`connecting`（首次连接 Linear 还没结束）；选项 `latencyMs`（每个调用的延迟）、`language`（写进设置的 `language`，浏览器里地址的 `?lang=` 传给它，截图用）与 `update`（应用上次检查更新的结果：`available` 有新版本、`install-failed` 上次装不上，别的名字是已是最新；地址的 `?update=` 传给它，版面检查用） |

和时间有关的缓存测试用 tokio 的暂停时钟（`#[tokio::test(start_paused = true)]` 加 `tokio::time::advance`），不真的等待。

## 夹具与脱敏

- 真实样本放在 `docs/gates/samples/<里程碑>/`，测试需要时复制到 `crates/<crate>/tests/fixtures/`。
- 提交前替换：姓名改为 `Jane Doe`，邮箱改为 `jane@example.com`，需求标识改为 `DEMO-N`，UUID 改为 `00000000-0000-0000-0000-00000000000N`，项目改为 `Demo Project`，团队改为 `Demo`，URL 改为 `https://linear.app/demo/issue/DEMO-N`，路径改为 `~/…`，会话 ID 改为全零。字段名与结构保持原样。
- 不为了让测试通过去手改真实样本；需要别的形态就另写一份虚构夹具。
- 面板与 core（或托盘）必须同规则的函数，规则写进上文「面板与 core 共用的用例表」那张表里的共读文件，不复制；新增这类规则时在表里加一行。

## 实机验收规则

- 只在 Linear 沙箱项目和本机沙箱仓库（用本地裸仓库充当远端）上进行，不碰真实业务仓库。
- 记录环境与版本、每一步的命令和结果、从建需求到被接走的耗时、Linear 状态与评论、推送结果、异常与未验证项。
- 没做的项写「待实机」，不能用测试结果代替。

## 验收报告模板

````markdown
# <里程碑>：<主题>（<日期>）— <通过 | 部分通过 | 待实机>

## 环境
- 系统与版本；Claude Code / Codex 版本；IssueRelay 提交

## 步骤与结果
| 步骤 | 命令 | 结果 |
|---|---|---|

## 与设计不一致之处

## 未闭环
````
