# 更新签名密钥

IssueRelay 的自动更新只安装用这把密钥签过名的更新包（设计第 12 节）。公钥提交在 `app/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`；私钥只在两处：维护者本机的 `~/.tauri/issuerelay.key`（权限 600）和 GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`。私钥不进仓库、不进聊天、不进日志和截图；`scripts/check-secrets.sh` 会拦下它。

## 生成（只做一次）

由主智能体或维护者在维护者自己的电脑上、仓库根目录运行；已有 `~/.tauri/issuerelay.key` 时不要重新生成（会让已安装的程序再也收不到更新）：

```bash
mkdir -p ~/.tauri && chmod 700 ~/.tauri
pnpm --dir app tauri signer generate --ci -w ~/.tauri/issuerelay.key > /dev/null
chmod 600 ~/.tauri/issuerelay.key
```

`--ci` 不提问、不设密码：GitHub 的 secret 不能是空值，没有密码就不需要密码 secret，也不会有密码要经过聊天或命令行历史。私钥靠文件权限与备份保护。`-w` 把私钥写进文件、公钥写进同名的 `.pub` 文件；命令的输出丢弃只是预防。

公钥写进配置：把 `~/.tauri/issuerelay.key.pub` 里的那一行（只读这个文件；公钥可以公开）粘进 `app/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。同一段里的 `endpoints`、`windows.installMode` 和 `requireSignedVersion` 不用动。`tauri.updater.conf.json` 不放 `plugins`，信任的公钥只有 `tauri.conf.json` 这一处（`app/src-tauri/tests/packaging.rs` 核对）。

## 上传到 GitHub（维护者自己运行）

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo sunkailun1992/issuerelay < ~/.tauri/issuerelay.key
```

从文件读入，私钥不会出现在屏幕或 shell 历史里；不要用 `--body`。不建 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（现在的密钥没有密码；工作流里 `CI=true`，CLI 用空密码）。上传后 `gh secret list --repo sunkailun1992/issuerelay` 能看到这个名字（看不到值）。

## 备份

把 `~/.tauri/issuerelay.key` 作为附件存进密码管理器。丢了私钥，已安装的程序就再也收不到自动更新（它们只认这把公钥），用户只能手动下载安装带新公钥的版本。

## 核对

发布工作流用 `scripts/verify-update-signature.mjs` 拿提交的公钥核对每个更新包的签名，并核对签名里记的版本（`tauri build` 在私钥与公钥不配对时只警告、不失败，所以要单独核对）。已安装的程序收到更新时再核对一次：签名要对得上公钥，签名里记的版本要与清单宣布的版本一致（`requireSignedVersion`），否则不安装。

本机试验签名时，另用 `mktemp -d` 下的一把一次性密钥，用完删掉整个目录，不要动 `~/.tauri/issuerelay.key`。

## 轮换与泄露

- 计划内轮换：生成新密钥对之前，先用旧私钥签发一个把 `pubkey` 换成新公钥的版本；已安装的程序装上它以后才认新公钥。之后的版本用新私钥签，替换 secret。
- 私钥泄露：攻击者要发出能被安装的更新，还得能往本仓库的 Releases 上传资产（需要写权限）或冒充 github.com 的 HTTPS。立刻按上一条轮换；同时检查仓库的协作者、Actions 日志与 Releases。
