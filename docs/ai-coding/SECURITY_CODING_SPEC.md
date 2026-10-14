# 安全编码规范

## 1. 凭据

- Linear 的 OAuth 令牌和 API key 只存在系统钥匙串（`keyring`，服务名 `com.sunkailun.issuerelay`），不写文件，不进日志，不出现在 IPC 响应和错误信息里。
- 同一时刻只留一种登录方式：改用 API key 时先清掉 OAuth 凭据再存 key，OAuth 登录后清掉保存的 key（清不掉只警告，因为启动时 OAuth 优先；`host::SystemConnector`；开发宿主的 `login` / `api-key` 也走它，不得自己存凭据）。启动时先试 OAuth，留着旧的 OAuth 凭据会让重启后仍用旧登录。登录、换 key、登出一次只做一个（`Host` 里的一把锁），钥匙串和宿主才会停在同一个登录上。
- 面板里的个人 API key 输入框（`ApiKeyForm`，设置与首次引导共用）是密码框，不提示记住：key 去掉前后空白后只发给引擎一次，引擎答复后（不论收下与否）立刻清空；面板从不回显它、不写日志、不放进失败提示；浏览器登录在等待时它不可用（core 一次只改一个登录）。
- 代理地址里的用户名和密码：引擎报告环境前先打码（`ExecEnv::proxy_for_display`），面板显示前再打码一次（`app/src/lib/mask.ts`，与 core 的 `env::mask_credentials` 同规则，`mask-cases.json` 两边共读）；面板收不到未打码的值（`Host::env()` 不经任何命令交给面板）。
- Claude Code、Codex、git、CI、测试环境的凭据，程序一概不碰，由各自的工具管理。
- 测试不得读写真实钥匙串；必须碰的测试标 `#[ignore]`，并写明手动运行的方式。

## 2. 执行者权限

- 默认档位「完全放开」意味着 AI 能执行任意命令、使用用户的全部凭据。用户首次选择时必须明确提示（M2 面板）：保存绑定时，生效的档位是完全放开（选了「完全放开」，或选「默认」而设置里的默认档位是完全放开）且设置里的 `full_access_acknowledged` 还是假，面板先弹 `FullAccessWarning`；用户确认后先把 `full_access_acknowledged: true` 写进设置，写成了才保存绑定，取消或写不进去都不保存。
- 三档权限的参数映射（设计 6.1）只能在设计更新后修改；`--permission-prompts none` 必须保留，避免无人值守时卡住。
- 固定规则替代不了仓库自身的保护：README 提醒用户为主分支开启分支保护。

## 3. 不可信输入

- Linear 的标题、描述、评论可能夹带指令（提示注入）。程序不把这些文本放进命令行；提示词里只放需求编号和链接，由 AI 用自己的 Linear 连接去读；固定规则写在提示词最前面，并声明优先于其他任何内容。
- Linear MCP 的返回按不可信数据解析：解析失败就返回错误，不猜测。
- 执行者的输出只用来记录和识别最终结果；成功与否以 Linear 上的真实状态为准，不以 AI 的自述为准。
- 面板显示的外来文本（Linear 的标题与状态名、执行者与 AI 的输出、引擎和系统的错误）同样只当数据：
  - 只作为 React 文本节点渲染（由 React 转义），不用 `dangerouslySetInnerHTML`、`innerHTML`，也不把它当 HTML 或 Markdown 解释。`app/src/lib/api.test.ts` 扫描面板的全部源码，出现 `dangerouslySetInnerHTML`、`innerHTML`、`outerHTML` 或 `insertAdjacentHTML` 就失败。
  - 先清洗再显示。Linear 的文本在 core 生成面板数据时已经过 `text::clean_text`（`engine.rs`）；执行者与 AI 的文本（时间线、原始日志、摘要）和引擎原文的细节行，在面板里再经 `cleanDisplayText` / `cleanDisplayLines`（`app/src/lib/clean.ts`）：去掉 ANSI 转义序列，其余控制字符变空格，去掉 core `text::is_invisible` 去掉的零宽、方向控制等字符，免得一段文字伪装成别的样子。字符规则在 core 里只有 `text::strip_control` 一份实现，TS 的那份与它由 `clean-cases.json` 两边的测试共同核对。
  - 原文从不充当标题：标题由结构化数据（状态、代码、布尔值）翻译得出，原文清洗后作为次要的细节行（`AI_CODING_GUIDE.md` 第 12.4 节）；系统通知不带引擎原文。

## 4. 子进程

- 只用固定参数；提示词经标准输入传入；工作目录用 current_dir 设置；不经 shell；参数里从不含 Linear 的文本。仅有两处例外：① 一键配置命令（`setup.rs`）唯一的可变参数是 IssueRelay 自身命令行程序的绝对路径，放在 `--` 之后；② 读取执行者环境（`env.rs`）时运行用户的登录 shell（`$SHELL -l -i -c` 加一段固定脚本，不含任何外部文本），它在自己的会话里运行，读到结束标记或满 10 秒即被结束。
- 结束运行一律结束整棵进程树：Unix 结束进程组以及所有后代进程，包括已离开该组的（顺着父进程查找，核对启动时间后才发信号，进程号被复用的无关进程不受影响）；Windows 结束 Job Object（不允许脱离）。正常退出后也清理进程组里的残留；执行者已经退出时，离开进程组的后代无从查找（由实机验收测量）。
- 宿主被强杀后留下的执行者：macOS 上每次运行开始时记下执行者的进程号和启动时间，下次启动先清扫，只结束进程号和启动时间都吻合的执行者及其进程树；进程号 0、launchd、宿主自己和它所在的进程组一律不发信号，每次向进程组发信号前都重新核对领头进程的启动时间；记录清扫处理完才清除。执行者已经退出时，它留下的后代找不到。Windows 的 Job Object 随宿主关闭，整棵树自动结束。
- 设有总时长上限和无输出上限。

## 5. 本机通信

- macOS：套接字所在目录权限 0700；启动前检测是否已有宿主在运行，只有残留的套接字文件才删除。
- Windows：命名管道设 `reject_remote_clients(true)`；用 `first_pipe_instance(true)` 保证只有一个宿主；安全描述符按宿主所在用户的 SID 生成（`ipc::pipe_security::pipe_sddl`）：属主是这个用户，只有它和 SYSTEM 有访问权；不用 `OW`（属主权限），因为提权宿主的默认属主常是 Administrators 组，会挡住同一用户未提权的命令行、放进所有提权的管理员进程；客户端连接后核对管道服务端进程的用户，与自己不同就拒绝通信。
- 只提供 `METHODS` 列出的方法；「标记完成」「重试」「取消」只在面板（进程内）提供。
- 桌面面板（网页视图）只能调本程序自己的命令，Tauri 自带的接口只开放事件的 `listen` 与 `unlisten`：`app/src-tauri/build.rs` 在应用清单（`AppManifest`）里登记全部命令，Tauri 因此也按能力文件检查本程序的命令；`capabilities/default.json` 只给主窗口（本地内容）每条命令一个 `allow-<命令>` 和这两条事件权限，不给任何插件权限或其他核心权限。打开链接（只开 Linear 的 `https` 链接，交给系统的是检查时 `url::Url` 解析出的规范化地址，从不是原文：系统的打开方式按自己的规则解析，macOS 的 NSURL 会把 `https://linear.app\@evil.example/` 读成 evil.example）、选目录、通知、开机自启都在 Rust 侧的命令里做。`app/src-tauri/src/lib.rs` 的测试在命令表、应用清单与能力文件不一致，或多出任何权限时失败，运行时也不许加能力（同一处的测试检查本 crate 的源码里没有 `add_capability`）；`app/src/lib/api.test.ts` 在面板绕过 `api.ts` 直接用 Tauri、或调了能力文件没给的命令时失败。
- 链接只在一处检查（`commands.rs` 的 `linear_url`；假后端 `mock.ts` 里的近似版只供浏览器预览，不算检查）：`url::Url` 解析后协议必须是 `https`，主机是 `linear.app` 或它的子域名（OAuth 授权页在 `mcp.linear.app`），不带用户名或密码，主机里没有空标签（例如结尾的点）。`open_linear` 命令与 Linear 登录时打开授权页共用这一个检查和打开方式（`open_linear_link`）；其他链接一律拒绝并记日志，面板收到代码 `not_a_linear_link`。opener 插件关掉了替页面打开点击链接的功能（`open_js_links_on_click(false)`）。
- `Host::env()` 带着真实的代理值（用户名和密码没打码）：它本身和任何代理值都不交给网页视图，命令拿它启动进程；由其中 PATH 得出的不含秘密的事实可以返回（`cli_on_path` 只回答 `~/.local/bin` 在不在登录 shell 的 PATH 里，外加固定的那一行 shell 命令；环境检查的报告本来就带着 PATH）；面板看到的执行环境是 `Engine::doctor` 给的、代理已打码的那份。
- `tauri.conf.json` 的 CSP 只允许本地资源与 IPC（`default-src 'self'`，`connect-src ipc: http://ipc.localhost`），并且不许 `<base>` 改动相对地址、表单向任何地方提交、页面加载插件对象（`base-uri 'none'`、`form-action 'none'`、`object-src 'none'`；面板的两个表单都自己处理提交）；`lib.rs` 的测试逐条核对。不放宽它，面板不加载远程脚本或样式，也不连外部地址。

## 6. 日志与数据

- 运行日志可能包含代码和 AI 打印出来的敏感内容：只存在本机数据目录，按保留天数和总大小清理，不上传。
- 程序日志（开发宿主的 stderr、桌面应用的 `logs/app.<日期>.log`）的字段可能带 Linear 或执行者的文本：每行先经 core 的 `logscrub` 去掉控制字符与不可见字符，只留结尾的换行。桌面应用里 rmcp 的日志不低于 info（`logging::allowed`，`RUST_LOG` 也改不了），因为它在 debug 级写出 OAuth 授权码。
- 没有遥测，不向任何第三方发送数据（Linear 除外，且只经用户授权的 MCP 连接）。

## 7. 开源卫生

- 仓库是公开的。代码、测试、夹具、文档、提交信息里不得出现令牌、API key、公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径。
- 每次提交前跑 `bash scripts/check-secrets.sh --strict`，必须无输出、退出码为 0（CI 同样用 `--strict`；退出码 2 表示扫描本身出错，要先修好）；GitHub 已开启密钥扫描与推送拦截。
- 来自真实运行的样本先脱敏再提交（见 `TESTING_SPEC.md`）。
- 发现疑似泄露只报告文件和行号；是否改写历史由用户决定。
- 被跟踪的文本文件里不放原样的不可见字符、方向控制字符和控制字符（制表、换行、回车除外），一律写成转义：TS 与 JSON 写 `\u200b`（补充平面的字符写成代理对），Rust 写 `\u{200b}`。这类字符在编辑器和 diff 里看不见，能让评审看到的代码与实际执行的不同，也可能被编辑器悄悄吞掉而用例照样通过。`bash scripts/check-secrets.sh --strict` 会跑下面这条扫描（需要带 PCRE2 的 git；没有时扫描本身失败，退出码 2），命中时报文件和行号并以 1 退出，所以本地检查与 CI 都会拦住；要单独查时也可以直接跑，应无输出：

  ```bash
  git grep -nIP '(*UTF)[\x{01}-\x{08}\x{0B}\x{0C}\x{0E}-\x{1F}\x{7F}-\x{9F}\x{AD}\x{61C}\x{115F}\x{1160}\x{200B}-\x{200F}\x{2028}-\x{202E}\x{2060}-\x{206F}\x{3164}\x{FEFF}\x{FFA0}\x{E0000}-\x{E007F}]' -- . | cut -d: -f1,2
  ```

## 8. 供应链

- 新依赖用 `cargo add` 引入，并说明用途；`Cargo.lock` 入库。
- 前端依赖（`app/`）用 `pnpm add` 引入，并说明用途；`app/pnpm-lock.yaml` 入库，CI 用 `pnpm install --frozen-lockfile` 安装，版本更新由 Dependabot 每周检查并延迟 7 天。
- GitHub Actions 只用官方或广泛使用的 action，并固定主版本。M3 起的发布工作流（`release.yml`）里，第三方 action 按提交 SHA 固定并在行尾注明版本。
- 不在构建脚本或 CI 里下载并执行来历不明的脚本。

## 9. AI 边界

AI 不替用户完成任何授权（Linear、Claude、Codex、GitHub），不改系统设置，不安装或卸载开机项，不改写已推送的历史。
