# M3：安装包与发布实机验收（2026-10-14）— 部分通过（macOS 通过，Windows 与同事安装待实机）

## 结论

- 安装：在 macOS 上按 `docs/install.md` 安装通过。dmg 先加上浏览器会加的下载标记，再用访达拷进个人「应用程序」。Gatekeeper 拦下后，在「隐私与安全性」放行一次，之后正常使用。放行后，包里的命令行程序经 PATH 链接运行，不再被拦。
- 自动更新：
  - 本机 A→B 真实更新通过。在有任务运行时更新：先选等待；关掉窗口再打开，提示还在；选「立即停止」后安装并重启为新版本，没有残留。
  - 签名链核对通过。
  - 「退出从不变成安装」通过。
  - 从磁盘映像运行时，程序拒绝安装更新。
- 把命令行加入 PATH：
  - 链接建得对；已有的外来文件不被覆盖；关掉开关只删自己建的链接。
  - 托盘程序没在运行时，`issuerelay status`、`issuerelay start` 与 MCP 服务的文字都与设计一致，MCP 服务不会替用户启动托盘程序。
- 程序坞图标（M3 新加）：
  - 默认常驻。模拟登录启动（`--autostart`）时有程序坞图标、没有窗口、不抢焦点。
  - 由系统发起的退出（程序坞的「退出」）：有任务在跑时不询问，1.8 秒内停止运行并退出，退出前记下的 20 个后代进程全部结束。
- 发布：`main` 上带私钥的手动试打包全绿（`updater=true`，三个安装包的签名都 `verified`）。推送 `v0.1.0` 产出了草稿 Release（见「第一个草稿」），等维护者公开。
- 新发现 N1–N5 见下。按 N1、N2 改了 `docs/install.md` 的两处说法。
- 未验证：
  - Windows 全部。
  - macOS 15、26 上的提示：本机是 macOS 27。
  - 同事按说明安装（设计第 17 节的完成标志）。
  - 计划第 6 步「失败的更新只提示」的四种情形，见「未闭环」。

## 环境

- 日期：2026-10-14，日志时间是 UTC。
- 软件版本：macOS 27.0.1（26A434），Apple 芯片；Claude Code 2.1.285；rustc 1.99.0。
- IssueRelay：`main` 的 `f172358`，也就是 M3 的合并提交、`v0.1.0` 标签所在。用了三份构建：
  - A：0.1.0。更新清单地址临时改成本机 `127.0.0.1:8765`，只用于更新测试，不提交。
  - B：0.1.1。在同一提交的一次性 worktree 里升了版本号，用真正的更新签名私钥签名：私钥只以路径交给构建，没有读出。B 的 `latest.json`、`.app.tar.gz` 与 `.sig` 由本机 `python3 -m http.server` 提供。
  - 发布配置的 dmg：`CI=true pnpm --dir app tauri build --bundles dmg --config src-tauri/tauri.bundle.conf.json`。这是本机 Apple 芯片的包，不是通用包；更新清单地址是 GitHub Releases。安装、Gatekeeper、PATH 与程序坞几项都用它测。
  - CI 打出的通用 dmg 没有下载：本机两次从 GitHub 下载 Actions 产物都失败了（`gh run download` 420 秒超时；直接下载 zip，在 27 MB 的 11.6 MB 处断开）。
- 安装位置：个人「应用程序」（`~/Applications`）。
- 沙箱：Demo Project 与 `~/issuerelay-sandbox/demo`，同 M2；绑定的执行者是 Claude Code。新建了三条测试需求 DEMO-27 至 DEMO-29。
- 设置：检查间隔 30 秒，界面语言跟随系统（繁体中文）。本报告引用的界面原文是繁体中文；`docs/install.md` 按简体中文界面写，括号里注了简体的对应。

## 步骤与结果

| 步骤 | 做法 | 结果 |
|---|---|---|
| 1 前提 | 在 `main` 上手动运行发布工作流（运行 38557545639） | 全部通过：<br>• `verify` 输出 `updater=true`，门禁自测有 `ok command line`。<br>• `bundle-dry-run` 两个打包任务的签名核对输出 `verified IssueRelay.app.tar.gz`、`verified IssueRelay_0.1.0_x64-setup.exe` 和 `verified IssueRelay_0.1.0_x64_en-US.msi`。<br>• `verify-dry-run` 通过：资产门查到 `.app.tar.gz` 与三个 `.sig`，由产物得出的 `latest.json` 每个平台键都有签名。 |
| 2 安装 | 给 dmg 加上浏览器会加的下载标记，双击挂载，在访达里把 IssueRelay 拷进 `~/Applications`（访达拷贝与拖动同效） | 拷出来的每个文件都带下载标记 `0183`：访达拷贝会加上「不做随机路径隔离」那一位。<br>磁盘映像里有一个指向 `/Applications` 的快捷方式。要装进个人「应用程序」，得按说明用「前往文件夹」打开它。 |
| 2 对照：终端拷贝 | 同一个磁盘映像，用 `ditto` 在终端拷进 `~/Applications` 再打开 | 下载标记是 `0083`，没有访达加的那一位。macOS 把应用放到随机的只读位置运行（App Translocation，见 N2）。 |
| 3 Gatekeeper 提示 | 双击 | 被拦下。系统日志记下提示事件，进程等用户回应；提示不是「已损坏」。<br>按系统的本地化表，这类提示的标题是「未打开“IssueRelay”」，正文是「Apple 无法验证“IssueRelay”是否包含可能危害 Mac 安全或泄漏隐私的恶意软件。」，按钮是「完成」和「移到废纸篓」。本机繁体界面是「未打開「IssueRelay」」，按钮是「完成」「丟到垃圾桶」。<br>自动化截不到系统对话框，这段文字没有截图核对。 |
| 3 误点「移到废纸篓」 | — | 用户连续三次在提示一出现就点了「移到废纸篓」，应用进了废纸篓，数据目录不受影响。本机的应用清理工具随即弹窗问要不要删相关文件，没有删。说明里本来就写了「点『完成』（不要点『移到废纸篓』）」，实测仍然容易误点（N1）。 |
| 3 放行 | 用户双击，点「完成」，再到「系统设置 → 隐私与安全性 → 安全性」点「仍要打开」（繁体界面是「強制打開」），输入登录密码后再确认 | 程序从 `~/Applications` 正常启动，没有被随机路径隔离。下载标记变成 `01c3`，多了「用户已批准」那一位，包里的命令行程序也一样。 |
| 3 钥匙串 | 新构建第一次读 Linear 凭据 | 弹出钥匙串授权，用户输入密码后连上 Linear。<br>同一个构建再打开不再询问；换了构建（包括更新之后）会再问一次（见第 5 步）。 |
| 4 PATH：打开 | 设置 →「把命令行加入 PATH」 | • `~/.local/bin/issuerelay` 指向 `~/Applications/IssueRelay.app/Contents/MacOS/issuerelay`。<br>• 本机登录 shell 的 PATH 里没有 `~/.local/bin`，面板给出要加的一行 `export PATH="$HOME/.local/bin:$PATH"`，旁边有「复制」。<br>• `~/.local/bin/issuerelay --version` 输出 `issuerelay 0.1.0`，`status` 输出正常。<br>• 经链接运行没有再被 Gatekeeper 拦下，系统日志里没有新的提示事件。 |
| 4 PATH：外来文件 | 关掉开关：链接被删，同一目录里别的链接没动。放一个自己写的 `issuerelay` 脚本，再回设置页 | • 面板显示「`~/.local/bin/issuerelay` 已存在，且不是 IssueRelay 建立的。删除或改名后再试。」<br>• 再点开关，仍只显示这句；文件内容与哈希不变。<br>• 删掉这个文件后再点开关，链接重新建好。 |
| 4 命令行 | 从托盘「结束 IssueRelay」，再运行 `issuerelay status`；然后运行 `issuerelay start` 两次 | • `status`：`Error: IssueRelay is not running; start it with \`issuerelay start\``，退出码 1。<br>• 第一次 `start`：`IssueRelay started.`，托盘程序启动并打开面板。<br>• 第二次 `start`：`IssueRelay is already running.` |
| 4 MCP | 托盘程序不在时，经标准输入输出直接调用 `issuerelay mcp` 的 `status` 工具。没有改用户的 Claude Code 配置；注册与移除在 M2 已验收 | 工具列表是 `list_jobs`、`pause`、`poll_now`、`resume` 和 `status`。<br>`status` 返回 `isError: true`，文字是 `error: IssueRelay is not running; ask the user to open the IssueRelay app; this server does not start it`；托盘程序没有被启动。 |
| 4 从磁盘映像运行 | 把 A 做成只读磁盘映像，挂载后直接运行里面的应用 | • 「把命令行加入 PATH」不可用，提示「IssueRelay 正從磁碟映像檔或暫存副本執行。請先把它移到「應用程式」。」<br>• 点「安装并重启」，提示「只能更新已安裝的應用程式：請先把 IssueRelay 移到「應用程式」」。 |
| 5 A→B 更新 | A 装在 `~/Applications`，DEMO-27 正在运行。横幅出现后点「安装并重启」，选「等待完成」，关窗再打开（提示还在），再选「立即停止」 | • 运行记为已取消，日志记下 `installing the update version=0.1.1`。<br>• 重启后只有一个进程，侧栏显示 0.1.1，Linear 已连接；没有残留进程，也没有 `.git/index.lock`。<br>• B 重启后，钥匙串又要了一次密码：ad hoc 签名，换了构建就会再问。 |
| 5 签名链 | 本机核对 | 更新签名私钥对应仓库里提交的公钥（密钥 ID `07A519E768117FBB`）。<br>B 的更新包经 `scripts/verify-update-signature.mjs` 核对，输出 `verified IssueRelay.app.tar.gz`。 |
| 5 退出不变成安装 | 用 A，DEMO-28 正在运行。点横幅「安装并重启」（下载并验签完成），选「继续执行」；再从托盘「结束 IssueRelay」，选「立即停止」 | 运行记为已取消，程序退出，版本仍是 0.1.0，日志里没有新的 `installing the update`。 |
| 程序坞：登录启动 | 默认设置下 `open -g … --args --autostart` | 应用类型是 Foreground（程序坞有图标），没有窗口，前台应用没变（不抢焦点）。 |
| 程序坞：退出 | DEMO-29 正在运行（`ping` 已开始），向应用发系统的退出事件（与程序坞「退出」相同） | • 不询问；1.8 秒内运行记为已取消，程序退出。<br>• 退出前记下的 20 个后代进程全部结束，包括执行者、它启动的各个 MCP 服务、在自己进程组里的 `ping`。没有 `.git/index.lock`。<br>• 退出之后另有一个用户插件起的后台进程，存活约 1.5 分钟（N3）。 |
| 托盘图标 | — | 本机菜单栏较挤，新启动的托盘图标被菜单栏管理工具收进隐藏区，要先展开才点得到。这正是 M3 加程序坞图标的原因。 |
| 发布前的更新检查 | 发布配置的构建启动 1 分钟后自动检查 | 设置的版本行显示「连不上更新服务器」，细节是 `Could not fetch a valid release JSON from the remote`。草稿没公开时，`releases/latest/download/latest.json` 不存在。没有横幅，也没有通知（N5）。 |

## 新发现

| 编号 | 发现 | 处理 |
|---|---|---|
| N1 | Gatekeeper 提示一出现，用户就点了「移到废纸篓」，连续三次。熟悉电脑的用户也会把它当成可疑弹窗。 | `docs/install.md` 补上提示框的标题，说明这是预期中的；还补了误点后的恢复办法：从废纸篓放回「应用程序」再打开一次。 |
| N2 | 在终端用 `ditto` 或 `cp` 从下载的磁盘映像拷贝，拷出的应用没有访达加的「不做随机路径隔离」位。放行后，macOS 从随机的只读位置运行它；程序按设计判为「未安装」，PATH 开关与「安装并重启」都不可用。界面却提示「请先移到应用程序」，而应用其实已经在里面。 | `docs/install.md` 第 1 步写明用访达拖，并说明遇到时用「终端里的办法」去掉下载标记。记入遗留：被随机路径隔离时换一句提示，说明原因与解决办法。 |
| N3 | 运行被取消时，用户自装的 Claude Code 插件在会话结束钩子里另起后台进程：它建了新进程组，父进程退出后归 launchd，所以不在 IssueRelay 能结束的进程树里。本次它约 1.5 分钟后自行结束；它用小模型总结会话，没有工具权限，往沙箱仓库里插件自己的目录写文件。另外，执行者照常加载用户的全部插件、钩子与 MCP 服务，包括本机控制类的服务，与 M2 精简运行评估所见一致。设计已写明，离开进程组且父进程已退出的后代无从查找。 | 记入遗留，并入精简运行的后续任务：无人值守运行不加载用户的钩子与插件，做法需实测，默认值由维护者决定。 |
| N4 | 每次启动后的第一轮检查都记为失败（`connecting to Linear`）：引擎在连上之前用一个「未连接」的占位，这是设计如此。概览随之显示「最近一次检查 Linear 失败了」，侧栏显示「需要处理」，要到下一轮检查（30 秒）才恢复；要等用户在钥匙串里输入密码时更久。 | 记入遗留：连上后立即补一轮检查，或者连接中不算失败。与 M2 的 N4 是同一类改法。 |
| N5 | 第一个版本发布之前，更新检查显示「连不上更新服务器」，实际原因是清单不存在（404），文案不准。 | 记入遗留：清单不存在时说「还没有发布的版本」。草稿公开后不会再出现。 |

## 与设计不一致之处

- 计划写的是有外来文件时「开关不可用」。实际上开关可以点，但点了只给出同一句提示，不动文件，效果相同，不改。
- 其余与设计第 4.1、12 节和 `docs/install.md` 一致。钥匙串在更新后再要一次密码，安装说明已经写了。

## 第一个草稿

- 标签：`v0.1.0`，指向 `f172358`。推送前确认远端没有这个标签，版本号取自 `origin/main` 的工作区版本。
- 发布工作流运行 38563382724 全部通过：`verify`、两个 `release-test`、`create-release`、`bundle-release` 的两个打包任务、`verify-release`；`bundle-dry-run` 与 `verify-dry-run` 按设计跳过。从推送标签到 `verify-release` 结束约 24 分钟，其中 Windows 打包约 16 分钟。
- 草稿的标题是 `IssueRelay v0.1.0`，状态是草稿、不是预发布，标题里没有「incomplete」。发布说明写了安装说明链接、下载哪个文件，以及「未经 Apple 公证」「Windows 安装包未签名」两条。
- 资产（字节）：

  | 文件 | 大小 |
  |---|---|
  | `IssueRelay_0.1.0_universal.dmg` | 27536917 |
  | `IssueRelay_0.1.0_universal.app.tar.gz` 与 `.sig` | 28208345 / 428 |
  | `IssueRelay_0.1.0_x64-setup.exe` 与 `.sig` | 9959635 / 440 |
  | `IssueRelay_0.1.0_x64_en-US.msi` 与 `.sig` | 14340096 / 440 |
  | `latest.json` | 5354 |

- `latest.json`：版本 `0.1.0`。平台键有 `darwin-aarch64`、`darwin-x86_64`、`darwin-universal`（各自还有带 `-app` 的一份，都指向同一个 `.app.tar.gz`）、`windows-x86_64` 与 `windows-x86_64-nsis`（指向 NSIS 安装包）、`windows-x86_64-msi`，每个键都带签名。下载地址都是 `api.github.com` 的资产地址（见「未闭环」里的限额一条）。
- 从草稿下载了通用 dmg，这次下载顺利：
  - 大小与资产列表一致。
  - 应用版本 0.1.0，最低系统 13.3；主程序与命令行程序都是 x86_64 加 arm64。
  - `codesign --verify --deep --strict` 通过（ad hoc 签名）；包里的 `issuerelay --version` 输出 `issuerelay 0.1.0`。
  - 没有用它重装一遍：放行流程与上面用本机 dmg 测的相同，免得再请维护者放行一次。
- 草稿没有公开，由维护者检查后在 GitHub 页面上公开。

## 未闭环

- Windows 全部待实机（没有 Windows 机器）：
  - NSIS 按用户安装不需要管理员，以及 SmartScreen 提示的原文；智能应用控制开着时的表现。
  - 「把命令行加入 PATH」往返：打开开关，新终端里 `issuerelay --version`，再关闭；用户 `Path` 里的其他条目逐字不变，`REG_EXPAND_SZ` 不变成 `REG_SZ`。
  - 从终端 `issuerelay start`，关掉终端后托盘程序仍在。
  - 正在运行的 `issuerelay.exe` 是否挡住 NSIS 更新与卸载。候选改法：用 NSIS 钩子把它交给重启管理器，或在安装说明里要求先关掉。
  - MSI 安装要管理员；更新经 `msiexec` 弹管理员确认，确认被拒时的表现；经 MSI 重启的程序是否带管理员令牌。
  - NSIS 卸载程序是否提供「删除应用数据」，以及它删哪些目录。
- macOS 15 与 26 上的提示与「仍要打开」流程：本机是 macOS 27。Intel Mac 上的通用包也没测。
- 同事按说明用安装包装好并跑通，这是设计第 17 节的完成标志。
- 草稿公开后再核对三点：`releases/latest/download/latest.json` 可用；已经装上的 0.1.0 不再显示「连不上更新服务器」；用草稿的通用 dmg 重装一遍（本次只核对了内容）。
- 失败的更新只提示（计划第 6 步）：另一把密钥签的签名、版本相同、服务器停掉、只有 Windows 平台键，这四种本次都没做，靠单元测试覆盖。其中「连不上更新服务器」在发布前自然出现了一次，见上。
- 有任务在跑时注销或关机：与程序坞「退出」走同一条路径，本次只测了程序坞「退出」。
- 启动失败后关掉面板时程序坞图标保留：没有实测，靠单元测试覆盖。
- 系统通知：权限仍未授予（同 M2）。
- `latest.json` 里的下载地址指向 GitHub：同一出口 IP 的未登录 API 每小时限 60 次，多人同时更新时是否够用，待观察。
- 沙箱里的 DEMO-27 至 DEMO-29 是测试需求，运行都被取消了，保持原样。
