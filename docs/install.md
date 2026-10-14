# 安装 IssueRelay

IssueRelay 是每个开发者自己装一份的托盘程序（macOS、Windows）。安装包在 [GitHub Releases](https://github.com/sunkailun1992/issuerelay/releases/latest)。

## 下载哪个文件

| 电脑 | 文件 | 说明 |
|---|---|---|
| Mac（Apple 芯片或 Intel） | `IssueRelay_<版本>_universal.dmg` | 一个包两种芯片通用 |
| Windows（推荐） | `IssueRelay_<版本>_x64-setup.exe` | 只装给你自己，不需要管理员权限 |
| Windows（为所有用户安装） | `IssueRelay_<版本>_x64_en-US.msi` | 需要管理员权限；之后的更新也会要求管理员确认 |

只从上面这个 Releases 页面下载。Windows 的两个安装包二选一，不要都装。资产列表里还有 `.app.tar.gz`、`.sig` 文件和 `latest.json`：这些不是安装包，不用下载（自动更新会自己取用）。

系统要求：macOS 13.3 及以后（Apple 芯片或 Intel）；Windows 10 或 11（x64）。每个版本的发布说明写明这一版是否经过 Apple 公证、Windows 安装包是否签名；不带自动更新包的版本也会写明。

## macOS 安装

1. 双击 `.dmg`，把 IssueRelay 拖进「应用程序」。没有管理员权限时，拖进你自己的「应用程序」文件夹（在访达里选「前往 → 前往文件夹…」，输入 `~/Applications`；没有这个文件夹就新建）。
2. 推出磁盘映像。不要直接在磁盘映像里或「下载」文件夹里运行 IssueRelay：那样「把命令行加入 PATH」和自动更新都用不了。

## macOS 第一次打开

没有经过 Apple 公证的版本（发布说明里写着「not notarized」），第一次打开会被 macOS 拦下来，只需放行一次；经过公证的版本没有这一步，直接打开即可：

- **macOS 15 及以后**：双击 IssueRelay，看到提示后点「完成」（不要点「移到废纸篓」）；打开「系统设置 → 隐私与安全性」，往下找到「安全性」，点 IssueRelay 旁边的「仍要打开」（被拦后大约一小时内可以点），输入登录密码，再确认一次。之后就能正常打开。
- **macOS 14 及更早**：按住 Control 点按 IssueRelay，选「打开」，再点「打开」。
- **终端里的办法**（只对从上面的 Releases 下载的包这么做）：`xattr -dr com.apple.quarantine /Applications/IssueRelay.app`（装在个人「应用程序」里就把路径换成 `~/Applications/IssueRelay.app`）。它去掉下载标记，macOS 就不再为这个应用做首次检查。
- 如果提示「已损坏，无法打开」：下载的文件不完整或不是从 Releases 下载的，删掉后重新下载。

## Windows 安装

1. 双击 `IssueRelay_<版本>_x64-setup.exe`。没有代码签名的安装包（发布说明里写着「not code-signed」）会出现「Windows 已保护你的电脑」：点「更多信息」，再点「仍要运行」。签过名的安装包通常不会这样，但新证书在积累信誉之前也可能出现同样的提示。
2. 按提示安装；默认装到 `%LOCALAPPDATA%\IssueRelay`，不需要管理员权限。
3. 若安装包或程序被直接拦截、没有「仍要运行」：打开「Windows 安全中心 → 应用和浏览器控制 → 智能应用控制」查看是否开启。智能应用控制是 Windows 11 的功能，开着（强制模式）时可能直接拦下未签名的程序，而且没有只放行某一个程序的办法，要不要关掉它由你自己决定。公司管理的电脑请联系管理员。

## 首次引导

第一次打开时，面板会自动进入首次引导，一共六步：

1. 选择界面语言。
2. 连接 Linear：在浏览器里授权。
3. 检查 Linear 工作区：需要有 `ai-ready` 标签，以及 In Progress、In Testing、Done 三个状态。IssueRelay 不会替你创建，缺什么，引导会写明在 Linear 的哪里添加；名字不一样时，可以在「设置 → 高级 → 触发用的名字」里改。
4. 检查 Claude Code 和 Codex：至少要有一个已安装、已登录并连上 Linear。
5. 绑定第一个项目：把一个 Linear 项目对应到你电脑上的一个目录。
6. 使用说明。

之后随时可以在面板的「环境检查」里重新检查，或点那里的「重新走一遍引导」。

关掉面板窗口不会退出 IssueRelay：它继续在菜单栏（Windows：通知区域）里运行。macOS 点菜单栏的图标，Windows 右键点通知区域的图标，选「打开面板」可以再打开它，选「退出 IssueRelay」才会真正退出。

## 把命令行加入 PATH

想在终端里直接输入 `issuerelay`：打开面板「设置 → 系统」，打开「把命令行加入 PATH」。不需要管理员权限。

- **macOS**：IssueRelay 在 `~/.local/bin/issuerelay` 建一个指向应用里命令行程序的链接，不改你的 shell 配置文件。
  - 如果开关下面出现一行 `export PATH="$HOME/.local/bin:$PATH"`（旁边有「复制」），说明 `~/.local/bin` 还不在 PATH 里：把它加到 shell 配置文件（zsh 是 `~/.zshrc`），再新开一个终端。
  - 那里已有别的同名文件（不是 IssueRelay 建的）时，IssueRelay 不会覆盖它：开关保持关闭、仍然可以点，面板写着「已存在，且不是 IssueRelay 建的。删除或改名后再试。」把那个文件删掉或改名，再打开一次开关，链接就建好了；文件还在的话，面板照旧写着这句话，什么也不会被改动。面板不会自己发现文件已经没了：点一下开关，或离开设置页再回来，它才会重新看一眼。
  - 挪动过 IssueRelay、或装了另一份之后，面板会写「链接指向另一份 IssueRelay」：打开开关，链接就改为指向现在这一份。
  - 面板写「IssueRelay 正从磁盘映像或临时副本运行」时，开关不可用：先把 IssueRelay 移到「应用程序」（见「macOS 安装」）。
- **Windows**：IssueRelay 把安装目录加进你自己的 PATH（用户级的，所以不需要管理员权限）。新开的终端才看得到；已经开着的终端要重开。
- 打开开关后新开一个终端，运行 `issuerelay --version`，能看到版本号就是成功了。
- 托盘程序没在运行时，`issuerelay status` 会提示；运行 `issuerelay start` 即可启动托盘程序（已经在运行时它什么也不做，关掉终端也不影响托盘程序）。
- 在 Claude Code、Codex 里注册 IssueRelay 的 MCP 控制服务（「设置 → MCP 控制服务」）用的是命令行程序的完整路径，不依赖 PATH。托盘程序没运行时，这些工具只会报错，不会替你启动它。

## 自动更新

- 「设置 → 系统 → 自动检查更新」打开时（默认打开），IssueRelay 启动 1 分钟后检查一次，之后每 6 小时一次；也可以随时点同一组里的「检查更新」。那一行还写着当前版本（侧栏底部也有）和上次检查的结果：「已是最新版本」、新版本的版本号，或失败的原因。
- 有新版本时，面板顶部出现横幅「IssueRelay <版本> 已发布」，带「安装并重启」和「稍后」两个按钮；「稍后」把这个版本的横幅收起，直到下次启动 IssueRelay（更新的版本发布时会再出现）。自动检查发现新版本时，还会发一条系统通知（每个版本一次，IssueRelay 重新启动后可能再发一次）；关掉「自动检查更新」就不再自动检查，也不发通知。
- 点「安装并重启」：IssueRelay 先下载并校验签名（按钮写「正在下载并校验…」）；校验不通过的更新不会安装。有任务在运行时，会像退出时一样问你：「等待完成」（任务跑完后安装）、「立即停止」（停掉任务，马上安装）或「继续运行」（这次先不装，横幅还在）；之后安装并重启。安装期间关窗口或再选退出，都不会中断安装。
- 在「等待完成」期间，如果从菜单栏图标（Windows：通知区域的图标）选「退出 IssueRelay」，已下载的更新会被丢掉：任务跑完后 IssueRelay 只退出，不安装，面板里的提示也改成「……完成后 IssueRelay 会自动退出」。
- Windows 上安装时会出现一个小的进度窗口，不需要操作，装完 IssueRelay 自动重新打开。安装更新前，请先关掉正在运行 `issuerelay` 的终端窗口或 AI 工具，否则安装程序可能无法替换它。
- 某个版本的发布说明写着「不含自动更新包」时，已安装的程序不会自动更新到它，需要手动下载安装：照常装一遍新版的安装包即可，设置和数据都保留。

## 卸载

1. 先在面板里关掉「把命令行加入 PATH」和「开机自启」（都在「设置 → 系统」）。
2. **macOS**：退出 IssueRelay（菜单栏图标里的「退出 IssueRelay」），把它从「应用程序」拖到废纸篓。想连数据一起删：在访达里用「前往 → 前往文件夹…」打开并删除 `~/Library/Application Support/com.sunkailun.issuerelay`（设置、绑定、运行记录与日志），并在「钥匙串访问」里搜索 `com.sunkailun.issuerelay`，删除 Linear 的登录凭据。
3. **Windows**：先退出 IssueRelay，并关掉正在运行 `issuerelay` 的终端窗口或 AI 工具，再在「设置 → 应用」的应用列表（Windows 11 是「已安装的应用」，Windows 10 是「应用和功能」）里卸载 IssueRelay。想连数据一起删：删除 `%APPDATA%\com.sunkailun.issuerelay` 和 `%LOCALAPPDATA%\com.sunkailun.issuerelay`，并在「凭据管理器 → Windows 凭据」里删除名称带 `com.sunkailun.issuerelay` 的条目。
4. 忘了第 1 步时：macOS 手动删 `~/.local/bin/issuerelay`；Windows 在「编辑账户的环境变量」里从 `Path` 删掉 IssueRelay 的安装目录。「开机自启」没关就卸载的话，系统里会留下一个指向不存在程序的启动项：重新装回 IssueRelay、关掉开关、再卸载，就能去掉它。

## 常见问题

- **找不到 claude 或 codex**：打开「环境检查」，在「运行环境」里看「PATH 来自」是不是「你的登录 shell」。从 Dock 或访达打开的程序只有最小的 PATH，所以 IssueRelay 从你的登录 shell 读取 PATH（Windows 没有登录 shell，读的是系统和用户的 `Path`）；在终端里能运行、在 IssueRelay 里找不到时，把它们所在的目录加到 shell 配置文件里（Windows：加进用户的 `Path`），重启 IssueRelay，再点「重新检查」。
- **需要代理才能访问 Linear 或 GitHub**：IssueRelay 依次读取它自己的环境、登录 shell 和系统设置里的代理；「环境检查」里能看到用的是哪个（密码已打码）。自动更新也要访问 `github.com` 与 `api.github.com`，这两个地址在你的网络里不通时，更新会失败（见下一条）。
- **Linear 授权**：在「环境检查」里点「重新授权」，或在「设置 → 高级 → Linear 访问」里改用个人 API key。
- **更新失败**：失败时什么都不会被安装，监控照常进行。检查时连不上（「连不上更新服务器」）、或新版本还没有适合这台电脑的安装包，会写在「设置 → 系统」里「检查更新」那一行下面（后台检查失败也一样，不会打扰你）；点「安装并重启」之后下载不下来、更新包的签名校验没有通过（「更新包的签名校验没有通过，什么都没安装」），或 IssueRelay 不是从已安装的位置运行的（「只能更新已安装的应用：请先把 IssueRelay 移到「应用程序」」，从磁盘映像或临时副本运行时都会这样），会弹出提示，横幅还在，可以再试。稍后再试，或从 Releases 手动下载。已经开始安装却没能装上时，IssueRelay 会用原来的版本重新启动，面板顶部写「更新没能安装」和原因，点「知道了」收起；这条只在重启后显示一次，原因也记在程序日志里。
