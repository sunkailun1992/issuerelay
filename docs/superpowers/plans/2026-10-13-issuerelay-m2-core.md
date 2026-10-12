# IssueRelay M2 第一部分：引擎与宿主 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 M1 的引擎改造成桌面应用能直接嵌入的样子：执行者拿到正确的 PATH 和代理，Windows 不弹窗、管道只认当前用户，日志会清理，宿主崩溃后能清掉残留进程，面板需要的引擎接口与事件齐全，一键配置命令可用，宿主逻辑抽成可复用的 `core::host`。

**Architecture:** 全部改动在 `crates/core`（外加 `crates/cli`、`crates/devhost` 的配套修改），不引入界面。新增模块 `env`（执行者环境）、`logs`（日志清理）、`setup`（一键配置命令）、`host`（宿主组装：IPC、后台循环、可替换的 Linear 连接）。M2 第二部分（桌面应用）只依赖这里定下的接口。

**Tech Stack:** Rust 1.99（edition 2024）、tokio、process-wrap 10、rusqlite、rmcp 3.5、keyring 4、`windows` 0.62（仅 Windows）、`winreg` 0.56（仅 Windows）。

**Spec:** `docs/superpowers/specs/2026-10-10-issuerelay-design.md`（第 5.6、6.2、6.4、7、8、10.2、11、13 节）；上一份计划 `docs/superpowers/plans/2026-10-10-issuerelay-m0-m1.md` 末尾的「本计划不覆盖」表和「评审遗留的小项」。

## Global Constraints

- 工具链 Rust 1.99.0（`rust-toolchain.toml`），edition 2024；工作区成员 `crates/*`。
- 应用标识 `com.sunkailun.issuerelay`；配置 `dirs::config_dir()/com.sunkailun.issuerelay/config.toml`；数据目录 `dirs::data_local_dir()/com.sunkailun.issuerelay/`（`state.db`、`logs/`）。
- 提示词三条固定规则不可删：不把状态设为 Done、不推主分支、不发正式环境。
- 「标记完成」只由桌面面板在进程内调用 `Engine::mark_done`；IPC 方法表、命令行、MCP 控制服务都不得提供，「重试」「取消」同样不经 IPC。`ipc::protocol` 与 `ipc::server` 的绊线测试不得削弱。
- 执行者子进程只用固定参数：提示词走标准输入，工作目录用 `current_dir`；参数里不放提示词、路径或任何来自 Linear 的文本。
- 结束运行必须结束整棵进程树（Unix：进程组以及离开该组的后代；Windows：Job Object）。
- 是否重新派活只看 Linear 的 `updatedAt`，不用本机时钟。
- 程序对 Linear 的写操作只有「标记完成」一处。
- 两个平台同等对待：每个 `#[cfg(unix)]` 都要有对应的 `#[cfg(windows)]`（或明确写出 Windows 不需要的原因）；Windows 代码只能在 CI 上编译和测试，写法要保守。
- 单元与集成测试不访问网络、不碰真实钥匙串、不依赖本机装的 `claude` / `codex`；与时间有关的上限用毫秒级或 tokio 暂停时钟；临时文件用 `tempfile`。
- 测试数据只用虚构值：`DEMO-1`、`Demo Project`、`Jane Doe`、`user-1`、`proj-1`、`team-1`、`https://linear.app/demo/issue/DEMO-1`。
- 公开仓库红线：不出现令牌、API key、公司名、内网地址、真实 Linear 工作区或需求内容、本机绝对路径。
- 注释与标识符用英文，遵守 `docs/ai-coding/AI_COMMENT_STYLE_GUIDE.md`；文档用中文。
- 每个任务结束前跑四条验证命令：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`bash scripts/check-secrets.sh --strict`（必须无输出）。
- 新依赖用 `cargo add` 取当前兼容的最新版本，`Cargo.lock` 一并提交；`windows` 必须与 process-wrap 用的同一大版本（0.62），否则 `CreationFlags` 的类型对不上。
- 提交信息用英文 Conventional Commits，末尾写实际执行的模型：`Co-Authored-By: <模型名> <noreply@anthropic.com>`。
- 分支 `feat/m2-core`，所有提交经 PR #（由主智能体创建）合入 `main`；禁止强推，提交前与推送前 `git fetch`。

## Review Focus

1. 从访达或开机自启启动的程序只有最小 PATH、没有代理变量：执行者仍要能被找到、能连上 Linear（Task 1 的 `combine` 优先级测试与假登录 shell 测试）。
2. 登录 shell 卡住、输出杂乱或不存在：读取最多等 10 秒，不能拖住宿主，原因写进 `problems`（Task 1 的超时测试）。
3. 宿主被强杀后留下仍在跑的执行者进程树：下次启动要结束它，但绝不碰进程号已被复用的无关进程（Task 4 的启动时间核对测试）。
4. 代理地址里带用户名和密码：环境检查和面板只显示打码后的地址（Task 1 的 `mask_credentials` 测试）。
5. 运行进行中重新登录 Linear：换连接不能让运行失败或重复，结束核对用新连接（Task 5 的 `set_linear` 测试）。

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `crates/core/src/env.rs`（新） | 执行者环境：登录 shell 的 PATH、代理变量的来源与合并、打码显示 | 1 |
| `crates/core/src/runner.rs` | 执行者进程：应用环境、Windows 建进程标志、输出限流、日志权限、记录进程号与启动时间、清扫残留 | 1、2、3、4 |
| `crates/core/src/doctor.rs` | 检查命令进程组与超时结束、在用户主目录运行、按已绑定项目的团队检查工作区、报告附带环境 | 1、2、6 |
| `crates/core/src/ipc/server.rs`、`ipc/client.rs` | Windows 管道只允许当前用户，客户端核对服务端进程的用户 | 2 |
| `crates/core/src/logs.rs`（新） | 日志按保留天数与总大小清理 | 3 |
| `crates/core/src/store.rs` | 表结构第 2 版（进程号、启动时间），拒绝更新版本的数据库 | 4 |
| `crates/core/src/engine.rs` | 面板接口：事件、需求视图、设置读写、换 Linear 连接、等待完成、启动失败退避；日志清理与残留清扫的调度 | 3、4、5 |
| `crates/core/src/linear/*`、`text.rs` | 中文链接、翻页、字段检查、项目团队、回环端口释放、不可见字符 | 6 |
| `crates/core/src/config.rs`、`executor/mod.rs`、`crates/cli/src/*` | 新设置项、未知键提醒、临时文件清扫、语言值校验、子目录识别 git 仓库、MCP 输出与注解、绑定路径规范化 | 7 |
| `crates/core/src/setup.rs`（新） | 一键配置命令：给执行者加 Linear MCP、登录、注册或移除 IssueRelay 的 MCP 服务 | 8 |
| `crates/core/src/host.rs`（新）、`crates/devhost/src/main.rs` | 宿主组装：IPC、后台循环、Linear 连接状态与替换、两种停止方式；devhost 改用它 | 9 |

---

### Task 1: 执行者环境（登录 shell 的 PATH 与代理）

设计第 8 节：图形程序只有最小 PATH，Claude Code 不读系统代理（M1 验收 H1）。新模块 `env` 一次算出子进程该拿到的环境，运行器和环境检查都用它；环境检查顺带在用户主目录运行、整组结束超时的检查命令（M1 计划「本计划不覆盖」6.4 行）。

**Files:**
- Create: `crates/core/src/env.rs`
- Modify: `crates/core/src/lib.rs`（`pub mod env;`）
- Modify: `crates/core/src/executor/mod.rs`（`resolve_program_in`）
- Modify: `crates/core/src/runner.rs`（`run` 多一个 `env` 参数）
- Modify: `crates/core/src/engine.rs`（`ProcessJobRunner` 持有共享环境）
- Modify: `crates/core/src/doctor.rs`（`SystemRunner`、`SystemReadiness`、`ReadinessProbe::env`、`DoctorReport.env`）
- Modify: `crates/core/src/bin/fake_agent.rs`（新步骤 `print_env`）
- Modify: `crates/cli/src/format.rs`（doctor 输出环境一节）
- Modify: `crates/devhost/src/main.rs`（启动时解析环境）
- Modify: `crates/core/Cargo.toml`（`dirs` 已在依赖里则不动；Windows 加 `winreg`）
- Test: `crates/core/src/env.rs` 末尾 `mod tests`、`crates/core/tests/runner.rs`、`crates/core/tests/doctor.rs`、`crates/cli/src/format.rs` 的测试

**Interfaces:**
- Produces:
  - `pub const env::PROXY_VARS: [&str; 8]`、`pub const env::REMOVED_VARS: [&str; 2]`
  - `pub enum env::EnvSource { Host, LoginShell, SystemSettings, None }`（serde kebab-case）
  - `pub struct env::ExecEnv { pub path: Option<String>, pub path_source: EnvSource, pub proxy: Vec<(String, String)>, pub proxy_source: EnvSource, pub problems: Vec<String> }`
  - `pub type env::SharedEnv = Arc<RwLock<ExecEnv>>`
  - `ExecEnv::inherit() -> ExecEnv`、`ExecEnv::shared(self) -> SharedEnv`、`ExecEnv::apply(&self, &mut tokio::process::Command)`、`ExecEnv::proxy_for_display(&self) -> Vec<(String, String)>`
  - `pub async fn env::resolve() -> ExecEnv`、`pub fn env::combine(host: &Vars, shell: Result<Vars, String>, system: Result<Vec<(String, String)>, String>) -> ExecEnv`、`pub fn env::merge_paths(first: &str, second: Option<&str>) -> String`、`pub fn env::mask_credentials(url: &str) -> String`、`pub fn env::parse_env_output(text: &str) -> Option<Vars>`、`pub fn env::parse_scutil_proxy(text: &str) -> Vec<(String, String)>`、`pub fn env::parse_windows_proxy(enabled: bool, server: &str, overrides: &str) -> Vec<(String, String)>`、`#[cfg(unix)] pub async fn env::shell_vars(shell: &Path, limit: Duration) -> Result<Vars, String>`；`pub type env::Vars = BTreeMap<String, String>`
  - `pub fn executor::resolve_program_in(kind: ExecutorKind, path: Option<&str>) -> Result<PathBuf>`（`resolve_program(kind)` 保留，等于 `resolve_program_in(kind, None)`）
  - `pub async fn runner::run(spec: CommandSpec, kind: ExecutorKind, log_path: &Path, limits: RunLimits, cancel: CancellationToken, env: &ExecEnv) -> RunOutput`
  - `ProcessJobRunner::new(env: SharedEnv) -> ProcessJobRunner`（`ProcessJobRunner` 不再是单元结构体）
  - `SystemRunner::with_env(env: SharedEnv) -> SystemRunner`（`Default` 等于 `with_env(ExecEnv::inherit().shared())`）；`SystemRunner::with_limits(check, mcp_list)` 不变
  - `SystemReadiness::new(ttl: Duration, env: SharedEnv) -> SystemReadiness`：`executor_reports(true)` 先重新 `env::resolve()` 写回共享环境，再查执行者
  - `ReadinessProbe::env(&self) -> Option<ExecEnv>`（默认 `None`）
  - `DoctorReport.env: Option<ExecEnv>`（`#[serde(default)]`）
- Consumes: `process_wrap::tokio::{CommandWrap, ProcessGroup, JobObject, KillOnDrop}`（同 `runner.rs`）

- [ ] **Step 1: 写 `env.rs` 的失败测试**

新建 `crates/core/src/env.rs`，先只放模块注释、类型声明的空壳（让测试能编译）和下面的测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_hosts_own_proxy_wins_and_is_passed_on_unchanged() {
        let host = vars(&[("PATH", "/usr/bin"), ("HTTPS_PROXY", "http://host-proxy:1")]);
        let shell = Ok(vars(&[("PATH", "/opt/demo/bin:/usr/bin"), ("HTTPS_PROXY", "http://shell-proxy:2")]));
        let env = combine(&host, shell, Ok(vec![("HTTPS_PROXY".into(), "http://system:3".into())]));
        assert_eq!(env.proxy_source, EnvSource::Host);
        assert!(env.proxy.is_empty(), "the host's own variables are inherited, not set again");
        assert_eq!(env.path.as_deref(), Some(merge_paths("/opt/demo/bin:/usr/bin", Some("/usr/bin")).as_str()));
        assert_eq!(env.path_source, EnvSource::LoginShell);
        assert!(env.problems.is_empty());
    }

    #[test]
    fn the_login_shells_proxy_comes_before_the_system_settings() {
        let host = vars(&[("PATH", "/usr/bin"), ("NO_PROXY", "only-no-proxy-is-not-a-proxy")]);
        let shell = Ok(vars(&[("PATH", "/usr/bin"), ("https_proxy", "http://shell-proxy:2"), ("NO_PROXY", "localhost")]));
        let env = combine(&host, shell, Ok(vec![("HTTPS_PROXY".into(), "http://system:3".into())]));
        assert_eq!(env.proxy_source, EnvSource::LoginShell);
        assert_eq!(
            env.proxy,
            vec![
                ("NO_PROXY".to_string(), "localhost".to_string()),
                ("https_proxy".to_string(), "http://shell-proxy:2".to_string()),
            ]
        );
    }

    #[test]
    fn system_settings_are_the_last_resort_and_nothing_means_no_proxy() {
        let host = vars(&[("PATH", "/usr/bin")]);
        let system = vec![("HTTPS_PROXY".to_string(), "http://127.0.0.1:7890".to_string())];
        let env = combine(&host, Ok(Vars::new()), Ok(system.clone()));
        assert_eq!(env.proxy_source, EnvSource::SystemSettings);
        assert_eq!(env.proxy, system);
        assert_eq!(env.path, None, "no login shell PATH: the host's PATH is kept");
        assert_eq!(env.path_source, EnvSource::Host);

        let none = combine(&host, Ok(Vars::new()), Ok(Vec::new()));
        assert_eq!(none.proxy_source, EnvSource::None);
        assert!(none.proxy.is_empty());
    }

    #[test]
    fn what_could_not_be_read_is_listed_as_a_problem() {
        let env = combine(&Vars::new(), Err("timed out".into()), Err("no scutil".into()));
        assert_eq!(env.problems.len(), 2);
        assert!(env.problems[0].contains("login shell") && env.problems[0].contains("timed out"));
        assert!(env.problems[1].contains("system proxy") && env.problems[1].contains("no scutil"));
        assert_eq!(env.path_source, EnvSource::Host);
        assert_eq!(env.proxy_source, EnvSource::None);
    }

    #[cfg(unix)]
    #[test]
    fn paths_merge_in_order_without_duplicates_or_empty_entries() {
        assert_eq!(merge_paths("/a:/b::/a", Some("/b:/c")), "/a:/b:/c");
        assert_eq!(merge_paths("/a", None), "/a");
    }

    #[cfg(windows)]
    #[test]
    fn paths_merge_in_order_without_duplicates_or_empty_entries() {
        assert_eq!(merge_paths(r"C:\a;C:\b;;C:\a", Some(r"C:\b;C:\c")), r"C:\a;C:\b;C:\c");
    }

    #[test]
    fn credentials_in_a_proxy_url_are_masked() {
        assert_eq!(mask_credentials("http://jane:s3cret@proxy.example:8080"), "http://***@proxy.example:8080");
        assert_eq!(mask_credentials("http://token@proxy.example:8080/path@x"), "http://***@proxy.example:8080/path@x");
        assert_eq!(mask_credentials("http://proxy.example:8080"), "http://proxy.example:8080");
        assert_eq!(mask_credentials("localhost,127.0.0.1"), "localhost,127.0.0.1");
        let env = ExecEnv {
            proxy: vec![("HTTPS_PROXY".into(), "http://jane:s3cret@proxy.example:8080".into())],
            ..ExecEnv::inherit()
        };
        assert_eq!(env.proxy_for_display()[0].1, "http://***@proxy.example:8080");
    }

    #[test]
    fn env_output_is_read_between_the_markers_only() {
        let text = format!(
            "rc noise PATH=/wrong\n\n{BEGIN}\nPATH=/opt/demo/bin:/usr/bin\nHOME=/home/demo\nHTTPS_PROXY=http://p:1/?a=b\n\n{END}\nmore noise HTTP_PROXY=x\n"
        );
        let parsed = parse_env_output(&text).unwrap();
        assert_eq!(parsed.get("PATH").map(String::as_str), Some("/opt/demo/bin:/usr/bin"));
        assert_eq!(parsed.get("HTTPS_PROXY").map(String::as_str), Some("http://p:1/?a=b"));
        assert!(!parsed.contains_key("HOME"), "only PATH and proxy variables are kept");
        assert!(!parsed.contains_key("HTTP_PROXY"), "text after the end marker is ignored");
        assert_eq!(parse_env_output("no markers at all"), None);
    }

    /// `scutil --proxy` as macOS prints it, with a per-interface dictionary whose values must not
    /// override the top level.
    const SCUTIL: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 6152
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 6152
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 6153
  SOCKSProxy : 127.0.0.1
  __SCOPED__ : <dictionary> {
    en0 : <dictionary> {
      HTTPSEnable : 1
      HTTPSPort : 9999
      HTTPSProxy : 203.0.113.9
    }
  }
}
";

    #[test]
    fn scutil_output_maps_to_proxy_variables() {
        let vars = parse_scutil_proxy(SCUTIL);
        let get = |name: &str| vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        assert_eq!(get("HTTPS_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("https_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("HTTP_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("http_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("ALL_PROXY"), None, "SOCKS is used only when there is no HTTP(S) proxy");
        assert_eq!(get("NO_PROXY"), Some("localhost,127.0.0.1,::1,.local,169.254/16"));
        assert_eq!(get("no_proxy"), get("NO_PROXY"));
    }

    #[test]
    fn scutil_with_only_socks_or_nothing_enabled() {
        let socks_only = "<dictionary> {\n  SOCKSEnable : 1\n  SOCKSPort : 1080\n  SOCKSProxy : 127.0.0.1\n}\n";
        let vars = parse_scutil_proxy(socks_only);
        assert!(vars.contains(&("ALL_PROXY".to_string(), "socks5h://127.0.0.1:1080".to_string())));
        assert!(vars.iter().any(|(n, _)| n == "NO_PROXY"));

        let off = "<dictionary> {\n  HTTPEnable : 0\n  HTTPPort : 8080\n  HTTPProxy : proxy\n  ProxyAutoConfigEnable : 1\n  ProxyAutoConfigURLString : http://wpad/x.pac\n}\n";
        assert!(parse_scutil_proxy(off).is_empty(), "a PAC file cannot be expressed in variables");
        assert!(parse_scutil_proxy("").is_empty());
    }

    #[test]
    fn windows_internet_settings_map_to_proxy_variables() {
        assert!(parse_windows_proxy(false, "proxy:8080", "").is_empty());
        assert!(parse_windows_proxy(true, "  ", "").is_empty());

        let single = parse_windows_proxy(true, "proxy.example:8080", "<local>;*.corp.example");
        let get = |vars: &[(String, String)], name: &str| vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
        assert_eq!(get(&single, "HTTPS_PROXY").as_deref(), Some("http://proxy.example:8080"));
        assert_eq!(get(&single, "HTTP_PROXY").as_deref(), Some("http://proxy.example:8080"));
        assert_eq!(get(&single, "NO_PROXY").as_deref(), Some("localhost,127.0.0.1,::1,.corp.example"));

        let per_protocol = parse_windows_proxy(true, "http=h1:1;https=h2:2;socks=h3:3", "");
        assert_eq!(get(&per_protocol, "HTTP_PROXY").as_deref(), Some("http://h1:1"));
        assert_eq!(get(&per_protocol, "HTTPS_PROXY").as_deref(), Some("http://h2:2"));
        assert_eq!(get(&per_protocol, "ALL_PROXY"), None);

        let socks = parse_windows_proxy(true, "socks=h3:3", "");
        assert_eq!(get(&socks, "ALL_PROXY").as_deref(), Some("socks5h://h3:3"));

        let with_scheme = parse_windows_proxy(true, "https=https://h4:4", "");
        assert_eq!(get(&with_scheme, "HTTPS_PROXY").as_deref(), Some("https://h4:4"));
    }

    #[tokio::test]
    async fn apply_sets_path_and_proxy_and_removes_the_nested_session_markers() {
        let env = ExecEnv {
            path: Some("/opt/demo/bin".into()),
            proxy: vec![("HTTPS_PROXY".into(), "http://p:1".into())],
            ..ExecEnv::inherit()
        };
        let mut command = tokio::process::Command::new("unused");
        env.apply(&mut command);
        let envs: Vec<(String, Option<String>)> = command
            .as_std()
            .get_envs()
            .map(|(n, v)| (n.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        assert!(envs.contains(&("PATH".into(), Some("/opt/demo/bin".into()))));
        assert!(envs.contains(&("HTTPS_PROXY".into(), Some("http://p:1".into()))));
        for name in REMOVED_VARS {
            assert!(envs.contains(&(name.to_string(), None)), "{name} must be removed");
        }
        let mut untouched = tokio::process::Command::new("unused");
        ExecEnv::inherit().apply(&mut untouched);
        assert!(untouched.as_std().get_envs().all(|(n, v)| v.is_none() && REMOVED_VARS.contains(&n.to_str().unwrap())));
    }

    #[cfg(unix)]
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-shell");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_login_shell_reports_its_path_and_proxy() {
        let dir = tempfile::tempdir().unwrap();
        // The fake shell gets `-l -i -c <script>`: it prints rc-file noise, sets what a profile
        // would, then runs the script it was given.
        let shell = fake_shell(
            dir.path(),
            "echo 'welcome from the rc file'\nPATH=/opt/demo/bin:$PATH; export PATH\nHTTPS_PROXY=http://127.0.0.1:9; export HTTPS_PROXY\neval \"$4\"",
        );
        let vars = shell_vars(&shell, Duration::from_secs(5)).await.unwrap();
        assert!(vars["PATH"].starts_with("/opt/demo/bin:"));
        assert_eq!(vars["HTTPS_PROXY"], "http://127.0.0.1:9");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hanging_login_shell_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 30");
        let started = std::time::Instant::now();
        let error = shell_vars(&shell, Duration::from_millis(300)).await.unwrap_err();
        assert!(error.contains("did not finish"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let missing = shell_vars(&dir.path().join("no-such-shell"), Duration::from_secs(1)).await.unwrap_err();
        assert!(missing.contains("cannot start"), "{missing}");
    }
}
```

- [ ] **Step 2: 跑测试，确认失败**

Run: `cargo test -p issuerelay-core --lib env::`
Expected: 编译失败或测试失败（函数尚未实现）。

- [ ] **Step 3: 实现 `env.rs`**

把空壳替换为完整实现（测试模块保留在文件末尾）：

```rust
//! The environment of executor runs and check commands: the PATH `claude` and `codex` are
//! found on, and the proxy they reach Linear through.
//!
//! A desktop app started from Finder or at login gets launchd's minimal PATH, without Homebrew
//! or npm's global directory, and Claude Code ignores the macOS system proxy: it reads only
//! `HTTPS_PROXY` and its relatives (design section 8; M1 acceptance H1). `resolve` looks at the
//! host's own environment, then the user's login shell, then the system proxy settings, and
//! `ExecEnv::apply` hands the result to every child.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Proxy variables in both spellings: Claude Code and Codex read the upper-case names, curl and
/// many other tools the lower-case ones.
pub const PROXY_VARS: [&str; 8] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// The variables that name a proxy; `NO_PROXY` alone sets none.
const PROXY_SETTING_VARS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Removed from every child. A host started from inside a Claude Code session inherits them, and
/// Claude then treats the run as a nested session (M1 final review).
pub const REMOVED_VARS: [&str; 2] = ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

/// Hosts that never go through the proxy; every `NO_PROXY` built from system settings has them.
const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// How long the login shell may take to print its environment. An interactive shell runs the
/// user's rc files, which can be slow (version managers, prompts) or hang.
#[cfg(unix)]
const SHELL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `scutil --proxy` may take.
#[cfg(target_os = "macos")]
const SCUTIL_TIMEOUT: Duration = Duration::from_secs(5);

/// Printed before and after `env` so that whatever the rc files print is ignored.
const BEGIN: &str = "__ISSUERELAY_ENV_BEGIN__";
const END: &str = "__ISSUERELAY_ENV_END__";

/// The shell used when `SHELL` is not set.
#[cfg(target_os = "macos")]
const DEFAULT_SHELL: &str = "/bin/zsh";
#[cfg(all(unix, not(target_os = "macos")))]
const DEFAULT_SHELL: &str = "/bin/sh";

/// PATH and proxy variables by name.
pub type Vars = BTreeMap<String, String>;

/// Where a part of the environment came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvSource {
    /// The host's own environment, passed on unchanged.
    Host,
    /// The user's login shell.
    LoginShell,
    /// The operating system's proxy settings.
    SystemSettings,
    /// Nowhere: no proxy is set.
    None,
}

/// The environment children get on top of the host's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecEnv {
    /// The PATH children get and executors are looked up on; `None` keeps the host's.
    pub path: Option<String>,
    pub path_source: EnvSource,
    /// Proxy variables set on children; empty when the host's own are passed on or none exist.
    pub proxy: Vec<(String, String)>,
    pub proxy_source: EnvSource,
    /// What could not be read, for the environment check.
    pub problems: Vec<String>,
}

/// One environment, shared by the job runner and the readiness checks; the environment check
/// replaces it with a freshly resolved one.
pub type SharedEnv = Arc<RwLock<ExecEnv>>;

impl ExecEnv {
    /// The host's environment as it is, without any lookup.
    pub fn inherit() -> Self {
        Self {
            path: None,
            path_source: EnvSource::Host,
            proxy: Vec::new(),
            proxy_source: EnvSource::Host,
            problems: Vec::new(),
        }
    }

    /// Wraps `self` for sharing between the job runner and the readiness checks.
    pub fn shared(self) -> SharedEnv {
        Arc::new(RwLock::new(self))
    }

    /// Sets the PATH and the proxy variables on `command` and removes `REMOVED_VARS`; everything
    /// else is inherited from the host as before.
    pub fn apply(&self, command: &mut tokio::process::Command) {
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        for (name, value) in &self.proxy {
            command.env(name, value);
        }
        for name in REMOVED_VARS {
            command.env_remove(name);
        }
    }

    /// The proxy variables for display, with any user name and password in a URL masked.
    pub fn proxy_for_display(&self) -> Vec<(String, String)> {
        self.proxy
            .iter()
            .map(|(name, value)| (name.clone(), mask_credentials(value)))
            .collect()
    }
}

/// `url` with its user-info part (`user:password@`) replaced by `***@`. Text that is not a URL
/// with an authority is returned as it is.
pub fn mask_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{scheme}://***@{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

fn interesting(name: &str) -> bool {
    name == "PATH" || PROXY_VARS.contains(&name)
}

fn has_proxy(vars: &Vars) -> bool {
    PROXY_SETTING_VARS
        .iter()
        .any(|name| vars.get(*name).is_some_and(|v| !v.trim().is_empty()))
}

/// The host's own PATH and proxy variables.
fn host_vars() -> Vars {
    std::env::vars().filter(|(name, _)| interesting(name)).collect()
}

/// Works out the environment children should get (see the module documentation). Never fails:
/// what could not be read is listed in `problems`.
pub async fn resolve() -> ExecEnv {
    let host = host_vars();
    let shell = login_shell_vars().await;
    let system = system_proxy().await;
    combine(&host, shell, system)
}

/// Puts the three sources together. PATH: the login shell's, followed by the host's entries it
/// lacks; without a login shell PATH, the host's. Proxy: the host's own when it sets one, else
/// the login shell's, else the system settings'.
pub fn combine(
    host: &Vars,
    shell: Result<Vars, String>,
    system: Result<Vec<(String, String)>, String>,
) -> ExecEnv {
    let mut problems = Vec::new();
    let shell = shell.unwrap_or_else(|e| {
        problems.push(format!("could not read the login shell's environment: {e}"));
        Vars::new()
    });
    let system = system.unwrap_or_else(|e| {
        problems.push(format!("could not read the system proxy settings: {e}"));
        Vec::new()
    });
    let (path, path_source) = match shell.get("PATH").filter(|p| !p.trim().is_empty()) {
        Some(shell_path) => (
            Some(merge_paths(shell_path, host.get("PATH").map(String::as_str))),
            EnvSource::LoginShell,
        ),
        None => (None, EnvSource::Host),
    };
    let (proxy, proxy_source) = if has_proxy(host) {
        (Vec::new(), EnvSource::Host)
    } else if has_proxy(&shell) {
        let proxy = shell
            .iter()
            .filter(|(name, value)| PROXY_VARS.contains(&name.as_str()) && !value.trim().is_empty())
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        (proxy, EnvSource::LoginShell)
    } else if !system.is_empty() {
        (system, EnvSource::SystemSettings)
    } else {
        (Vec::new(), EnvSource::None)
    };
    ExecEnv {
        path,
        path_source,
        proxy,
        proxy_source,
        problems,
    }
}

/// `first`'s entries, then those of `second` that `first` lacks; empty entries are dropped.
pub fn merge_paths(first: &str, second: Option<&str>) -> String {
    let mut seen = HashSet::new();
    let entries: Vec<PathBuf> = std::env::split_paths(first)
        .chain(second.into_iter().flat_map(std::env::split_paths))
        .filter(|entry| !entry.as_os_str().is_empty() && seen.insert(entry.clone()))
        .collect();
    std::env::join_paths(entries)
        .map(|joined| joined.to_string_lossy().into_owned())
        .unwrap_or_else(|_| first.to_string())
}

/// The PATH and proxy variables `env` printed between the two markers; `None` when the markers
/// are missing (the shell did not run the script).
pub fn parse_env_output(text: &str) -> Option<Vars> {
    let start = text.find(BEGIN)? + BEGIN.len();
    let end = start + text[start..].rfind(END)?;
    Some(
        text[start..end]
            .lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(name, _)| interesting(name))
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    )
}

/// The login shell's PATH and proxy variables. The shell runs interactively, so the rc files
/// that usually set PATH (`.zshrc`, `.bashrc`) are read as well as the profile.
#[cfg(unix)]
async fn login_shell_vars() -> Result<Vars, String> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SHELL));
    shell_vars(&shell, SHELL_TIMEOUT).await
}

/// Windows has no login shell: a program started from the Start menu or at login already gets
/// the user's full PATH from the system.
#[cfg(windows)]
async fn login_shell_vars() -> Result<Vars, String> {
    Ok(Vars::new())
}

/// Runs `shell -l -i -c <script>`, where the script prints `env` between two markers. The shell
/// runs in a process group of its own; the group, with anything the rc files left running in it,
/// is killed once the output is read, or after `limit` if the shell has not finished by then.
#[cfg(unix)]
pub async fn shell_vars(shell: &Path, limit: Duration) -> Result<Vars, String> {
    use std::process::Stdio;

    use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
    use tokio::io::AsyncReadExt;

    let script = format!("printf '\\n%s\\n' {BEGIN}; env; printf '\\n%s\\n' {END}");
    let mut command = CommandWrap::with_new(shell, |c| {
        c.args(["-l", "-i", "-c", script.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
    });
    command.wrap(ProcessGroup::leader());
    command.wrap(KillOnDrop);
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", shell.display()))?;
    let Some(mut stdout) = child.stdout().take() else {
        let _ = child.start_kill();
        return Err(format!("{} has no output pipe", shell.display()));
    };
    let read = async {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    };
    let outcome = tokio::time::timeout(limit, read).await;
    // Ends the shell and whatever its rc files left running in its group.
    let _ = child.start_kill();
    match outcome {
        Ok(Ok(bytes)) => parse_env_output(&String::from_utf8_lossy(&bytes))
            .ok_or_else(|| format!("{} printed no environment", shell.display())),
        Ok(Err(e)) => Err(format!("cannot read from {}: {e}", shell.display())),
        Err(_) => Err(format!("{} did not finish within {limit:?}", shell.display())),
    }
}

/// macOS: the system proxy settings as `scutil --proxy` prints them.
#[cfg(target_os = "macos")]
async fn system_proxy() -> Result<Vec<(String, String)>, String> {
    let run = tokio::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(SCUTIL_TIMEOUT, run)
        .await
        .map_err(|_| format!("scutil --proxy did not answer within {SCUTIL_TIMEOUT:?}"))?
        .map_err(|e| format!("cannot run scutil: {e}"))?;
    if !output.status.success() {
        return Err(format!("scutil --proxy exited with {}", output.status));
    }
    Ok(parse_scutil_proxy(&String::from_utf8_lossy(&output.stdout)))
}

/// Windows: the current user's Internet Settings.
#[cfg(windows)]
async fn system_proxy() -> Result<Vec<(String, String)>, String> {
    tokio::task::spawn_blocking(read_windows_proxy)
        .await
        .map_err(|e| format!("reading the proxy settings stopped: {e}"))?
}

#[cfg(windows)]
fn read_windows_proxy() -> Result<Vec<(String, String)>, String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .map_err(|e| format!("cannot open the Internet Settings key: {e}"))?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let server: String = key.get_value("ProxyServer").unwrap_or_default();
    let overrides: String = key.get_value("ProxyOverride").unwrap_or_default();
    Ok(parse_windows_proxy(enabled != 0, &server, &overrides))
}

/// Other Unix systems (not target platforms) have no single place for proxy settings.
#[cfg(all(unix, not(target_os = "macos")))]
async fn system_proxy() -> Result<Vec<(String, String)>, String> {
    Ok(Vec::new())
}

/// The proxy variables for what `scutil --proxy` prints. Only the top-level dictionary counts
/// (`__SCOPED__` holds per-interface copies). HTTPS and HTTP proxies map to `HTTPS_PROXY` and
/// `HTTP_PROXY`; a SOCKS proxy maps to `ALL_PROXY` only when neither is set. A proxy
/// auto-configuration (PAC) file cannot be expressed in variables and is left out.
pub fn parse_scutil_proxy(text: &str) -> Vec<(String, String)> {
    let mut values = HashMap::new();
    let mut exceptions = Vec::new();
    let mut depth = 0usize;
    let mut in_exceptions = false;
    for line in text.lines().map(str::trim) {
        if line == "}" {
            depth = depth.saturating_sub(1);
            if depth < 2 {
                in_exceptions = false;
            }
            continue;
        }
        let Some((key, value)) = line.split_once(" : ") else {
            if line.ends_with('{') {
                depth += 1;
            }
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if value.ends_with('{') {
            if depth == 1 && key == "ExceptionsList" {
                in_exceptions = true;
            }
            depth += 1;
        } else if in_exceptions && depth == 2 {
            exceptions.push(value.to_string());
        } else if depth == 1 {
            values.insert(key.to_string(), value.to_string());
        }
    }
    let proxy = |enable: &str, host: &str, port: &str, scheme: &str| -> Option<String> {
        if values.get(enable).map(String::as_str) != Some("1") {
            return None;
        }
        let host = values.get(host).filter(|h| !h.is_empty())?;
        let port = values.get(port).filter(|p| !p.is_empty())?;
        Some(format!("{scheme}://{host}:{port}"))
    };
    proxy_vars(
        proxy("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
        proxy("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        proxy("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5h"),
        exceptions.iter().map(String::as_str),
    )
}

/// The proxy variables for the current Windows user's Internet Settings: `ProxyEnable`,
/// `ProxyServer` (`host:port` for every protocol, or `http=host:port;https=host:port;socks=…`)
/// and `ProxyOverride` (`;`-separated; `<local>` means plain host names, already covered by the
/// local hosts).
pub fn parse_windows_proxy(enabled: bool, server: &str, overrides: &str) -> Vec<(String, String)> {
    let server = server.trim();
    if !enabled || server.is_empty() {
        return Vec::new();
    }
    let url = |value: &str, scheme: &str| {
        if value.contains("://") {
            value.to_string()
        } else {
            format!("{scheme}://{value}")
        }
    };
    let (https, http, socks) = if server.contains('=') {
        let (mut https, mut http, mut socks) = (None, None, None);
        for part in server.split(';') {
            let Some((protocol, value)) = part.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match protocol.trim().to_ascii_lowercase().as_str() {
                "https" => https = Some(url(value, "http")),
                "http" => http = Some(url(value, "http")),
                "socks" => socks = Some(url(value, "socks5h")),
                _ => {}
            }
        }
        (https, http, socks)
    } else {
        let value = url(server, "http");
        (Some(value.clone()), Some(value), None)
    };
    let exceptions = overrides
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case("<local>"));
    proxy_vars(https, http, socks, exceptions)
}

fn push_both(vars: &mut Vec<(String, String)>, name: &str, value: &str) {
    vars.push((name.to_string(), value.to_string()));
    vars.push((name.to_ascii_lowercase(), value.to_string()));
}

/// Both spellings of each variable for the proxies found, plus a `NO_PROXY` that always holds
/// the local hosts; nothing at all when no proxy is set.
fn proxy_vars<'a>(
    https: Option<String>,
    http: Option<String>,
    socks: Option<String>,
    exceptions: impl Iterator<Item = &'a str>,
) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    if let Some(value) = &https {
        push_both(&mut vars, "HTTPS_PROXY", value);
    }
    if let Some(value) = &http {
        push_both(&mut vars, "HTTP_PROXY", value);
    }
    if https.is_none() && http.is_none() {
        if let Some(value) = &socks {
            push_both(&mut vars, "ALL_PROXY", value);
        }
    }
    if vars.is_empty() {
        return vars;
    }
    let mut hosts: Vec<String> = LOCAL_HOSTS.iter().map(|h| h.to_string()).collect();
    for entry in exceptions
        .flat_map(|e| e.split([',', ';']))
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        // `*.local` means every host under `.local`, which `NO_PROXY` writes as `.local`.
        let entry = entry.strip_prefix('*').unwrap_or(entry).to_string();
        if !hosts.contains(&entry) {
            hosts.push(entry);
        }
    }
    push_both(&mut vars, "NO_PROXY", &hosts.join(","));
    vars
}
```

`crates/core/Cargo.toml` 的 Windows 依赖段加入 `winreg`：

```bash
cargo add -p issuerelay-core winreg@0.56 --target 'cfg(windows)'
```

`crates/core/src/lib.rs` 按字母序加 `pub mod env;`。

- [ ] **Step 4: 跑 `env` 测试，确认通过**

Run: `cargo test -p issuerelay-core --lib env::`
Expected: 全部 PASS（Unix 上包括两个假 shell 测试）。

- [ ] **Step 5: `resolve_program_in` 与运行器、引擎接上环境（先写失败测试）**

`crates/core/src/bin/fake_agent.rs`：模块注释的步骤列表加一行 `` - `{"print_env": name}`: print `name=<value>` (or `name=<unset>`) to stdout ``，并在步骤循环里（`out` 分支之后）加：

```rust
        if let Some(name) = step.get("print_env").and_then(Value::as_str) {
            let value = std::env::var(name).unwrap_or_else(|_| "<unset>".into());
            println!("{name}={value}");
            std::io::stdout().flush().ok();
        }
```

`crates/core/tests/runner.rs` 加测试（沿用文件里现有的脚本与 `fake_agent_spec` 一类辅助函数；名字以文件里实际的为准）：

```rust
#[tokio::test]
async fn the_run_gets_the_resolved_environment() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        r#"[{"print_env": "ISSUERELAY_TEST_PROXY"}, {"print_env": "CLAUDECODE"}]"#,
    );
    let log = dir.path().join("1.jsonl");
    let env = ExecEnv {
        proxy: vec![("ISSUERELAY_TEST_PROXY".into(), "http://127.0.0.1:9".into())],
        ..ExecEnv::inherit()
    };
    // SAFETY-free: the variable is only set on the child through `env`, never on this process.
    let output = runner::run(
        fake_agent_spec(&script, dir.path()),
        ExecutorKind::Claude,
        &log,
        limits(),
        CancellationToken::new(),
        &env,
    )
    .await;
    assert_eq!(output.exit, ExitKind::Exited(Some(0)));
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("ISSUERELAY_TEST_PROXY=http://127.0.0.1:9"), "{text}");
    assert!(text.contains("CLAUDECODE=<unset>"), "{text}");
}
```

`crates/core/src/executor/mod.rs` 的测试模块加：

```rust
    #[test]
    fn executors_are_looked_up_on_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "claude.cmd" } else { "claude" };
        let program = dir.path().join(name);
        std::fs::write(&program, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = dir.path().to_string_lossy().into_owned();
        assert_eq!(resolve_program_in(ExecutorKind::Claude, Some(&path)).unwrap(), program);
        assert!(resolve_program_in(ExecutorKind::Codex, Some(&path)).is_err());
    }
```

Run: `cargo test -p issuerelay-core --test runner the_run_gets_the_resolved_environment` 与 `cargo test -p issuerelay-core --lib executors_are_looked_up_on_the_given_path`
Expected: 编译失败（`run` 还没有 `env` 参数，`resolve_program_in` 不存在）。

- [ ] **Step 6: 实现 `resolve_program_in`、`runner::run` 的 `env` 参数、`ProcessJobRunner::new`**

`crates/core/src/executor/mod.rs`：

```rust
/// Resolves `claude` / `codex` on PATH, including `.cmd` shims on Windows.
pub fn resolve_program(kind: ExecutorKind) -> Result<PathBuf> {
    resolve_program_in(kind, None)
}

/// Resolves `claude` / `codex` on `path` (the environment check's resolved PATH), or on the
/// host's own PATH when `path` is `None`.
pub fn resolve_program_in(kind: ExecutorKind, path: Option<&str>) -> Result<PathBuf> {
    let name = kind.program_name();
    let found = match path {
        Some(path) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            which::which_in(name, Some(path), cwd)
        }
        None => which::which(name),
    };
    found.with_context(|| format!("`{name}` was not found on PATH"))
}
```

`crates/core/src/runner.rs`：`run` 的签名末尾加 `env: &ExecEnv`（`use crate::env::ExecEnv;`），文档注释补一句「`env` supplies PATH and proxy variables on top of the host's environment and removes the nested-session markers (`env::REMOVED_VARS`)」；`CommandWrap::with_new` 的闭包在 `.stderr(Stdio::piped())` 之后加 `env.apply(c);`。文件里其余调用 `run` 的测试一律传 `&ExecEnv::inherit()`。

`crates/core/src/engine.rs`：

```rust
/// Runs real executor processes, each with the environment the readiness checks resolved.
pub struct ProcessJobRunner {
    env: SharedEnv,
}

impl ProcessJobRunner {
    pub fn new(env: SharedEnv) -> Self {
        Self { env }
    }
}

#[async_trait::async_trait]
impl JobRunner for ProcessJobRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
    ) -> RunOutput {
        let env = self.env.read().unwrap_or_else(PoisonError::into_inner).clone();
        runner::run(spec, kind, &log_path, limits, cancel, &env).await
    }
}
```

（`use crate::env::SharedEnv;`。）

Run: Step 5 的两条命令
Expected: PASS。

- [ ] **Step 7: 环境检查改用共享环境、整组结束超时命令、在主目录运行（先写失败测试）**

`crates/core/src/doctor.rs` 测试模块加（`FakeRunner` 是文件里现有的记录型假实现；`SystemRunner` 的真实进程测试用 `fake-agent`，与现有 `system_runner_*` 测试同一写法）：

```rust
    #[tokio::test]
    async fn a_fresh_check_resolves_the_environment_and_reports_it() {
        let env = ExecEnv::inherit().shared();
        let readiness = SystemReadiness::with_parts(
            Duration::from_secs(300),
            FAILED_CHECK_TTL,
            Arc::new(FakeRunner::new(&[])),
            Arc::new(|_| None),
            env.clone(),
            Arc::new(|| Box::pin(async {
                ExecEnv { path: Some("/opt/demo/bin".into()), path_source: EnvSource::LoginShell, ..ExecEnv::inherit() }
            })),
        );
        assert_eq!(readiness.env().unwrap().path, None, "nothing resolved before the first fresh check");
        readiness.executor_reports(false).await;
        assert_eq!(readiness.env().unwrap().path, None, "a cached check does not re-resolve");
        readiness.executor_reports(true).await;
        assert_eq!(readiness.env().unwrap().path.as_deref(), Some("/opt/demo/bin"));
        assert_eq!(env.read().unwrap().path.as_deref(), Some("/opt/demo/bin"), "the job runner sees it too");
    }
```

`crates/core/tests/doctor.rs` 加（Unix；`fake_agent_path()` 为文件里现有的取路径函数）：

```rust
#[cfg(unix)]
#[tokio::test]
async fn a_timed_out_check_ends_its_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let script = dir.path().join("script.json");
    std::fs::write(
        &script,
        format!(r#"[{{"heartbeat": "{}", "every_ms": 50}}, {{"sleep_ms": 30000}}]"#, beat.display()),
    )
    .unwrap();
    let runner = SystemRunner::with_limits(Duration::from_millis(300), Duration::from_millis(300));
    let error = runner
        .run(&fake_agent_path(), &[script.to_str().unwrap()])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error:#}");
    // The heartbeat grandchild was in the check's process group: it must stop growing the file.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let size = std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0), size);
}
```

Run: `cargo test -p issuerelay-core --lib a_fresh_check_resolves_the_environment` 与 `cargo test -p issuerelay-core --test doctor a_timed_out_check_ends_its_whole_process_group`
Expected: 编译失败（`with_parts` 的新参数、`env()` 还不存在）；第二个测试在旧实现下失败（孙进程继续写文件）。

- [ ] **Step 8: 实现环境检查的改动**

`crates/core/src/doctor.rs`：

1. `SystemRunner` 加字段 `env: SharedEnv`；`Default` 改为 `Self::with_env(ExecEnv::inherit().shared())`；新增：

```rust
    /// Runs check commands with `env` (the PATH and proxy the executors will get) and the default
    /// limits.
    pub fn with_env(env: SharedEnv) -> Self {
        Self {
            check_timeout: CHECK_TIMEOUT,
            mcp_list_timeout: MCP_LIST_TIMEOUT,
            env,
        }
    }
```

`with_limits(check, mcp_list)` 保留（测试用），环境取 `ExecEnv::inherit().shared()`。

2. `CommandRunner for SystemRunner` 的 `run` 改为在进程组 / Job Object 里运行、工作目录取用户主目录（`claude mcp list` 的结果随目录而变，宿主从访达启动时目录是 `/`；固定用主目录，和用户级 `--scope user` 注册一致）：

```rust
#[async_trait::async_trait]
impl CommandRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
        use process_wrap::tokio::{CommandWrap, KillOnDrop};
        #[cfg(windows)]
        use process_wrap::tokio::JobObject;
        #[cfg(unix)]
        use process_wrap::tokio::ProcessGroup;
        use tokio::io::AsyncReadExt;

        let limit = self.limit_for(args);
        let env = self.env.read().unwrap_or_else(PoisonError::into_inner).clone();
        let mut command = CommandWrap::with_new(program, |c| {
            c.args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(home) = dirs::home_dir() {
                c.current_dir(home);
            }
            env.apply(c);
            c.env("NO_COLOR", "1");
        });
        #[cfg(unix)]
        command.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(JobObject);
        command.wrap(KillOnDrop);
        let mut child = command.spawn()?;
        let mut stdout = child.stdout().take();
        let mut stderr = child.stderr().take();
        let collect = async {
            let read = |pipe: Option<_>| async move {
                let mut bytes = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = AsyncReadExt::read_to_end(&mut pipe, &mut bytes).await;
                }
                bytes
            };
            let (out, err) = tokio::join!(read(stdout.take()), read(stderr.take()));
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((status, out, err))
        };
        let outcome = tokio::time::timeout(limit, collect).await;
        match outcome {
            Ok(result) => {
                let (status, out, err) = result?;
                Ok(CmdOutput {
                    code: status.code(),
                    stdout: String::from_utf8_lossy(&out).into_owned(),
                    stderr: String::from_utf8_lossy(&err).into_owned(),
                })
            }
            Err(_) => {
                // The whole group: `claude mcp list` starts stdio MCP servers of its own.
                let _ = child.start_kill();
                bail!("timed out after {limit:?}")
            }
        }
    }
}
```

（`read` 闭包里的两个管道类型不同：`ChildStdout` 与 `ChildStderr`。若泛型闭包写不通，就拆成两个 `async move` 块，各读一路。）更新 `SystemRunner` 的文档注释：命令在用户主目录、在自己的进程组（Windows：Job Object）里运行，超时连同它启动的进程一起结束。

3. `ReadinessProbe` 加默认方法：

```rust
    /// The environment executors are found and run with, when this probe resolves one.
    fn env(&self) -> Option<ExecEnv> {
        None
    }
```

4. `SystemReadiness`：字段 `resolve` 改为 `Arc<dyn Fn(ExecutorKind) -> Option<PathBuf> + Send + Sync>`，加字段 `env: SharedEnv` 与 `refresh_env: Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ExecEnv> + Send>> + Send + Sync>`。`new(ttl, env)`：

```rust
    /// `ttl` is how long `executor_reports(false)` reuses a check whose answers were all usable.
    /// `env` is shared with the job runner: executors are looked up on its PATH, check commands
    /// run with it, and every fresh check resolves it again first.
    pub fn new(ttl: Duration, env: SharedEnv) -> Self {
        let lookup = env.clone();
        Self::with_parts(
            ttl,
            FAILED_CHECK_TTL,
            Arc::new(SystemRunner::with_env(env.clone())),
            Arc::new(move |kind| {
                let path = lookup.read().unwrap_or_else(PoisonError::into_inner).path.clone();
                executor::resolve_program_in(kind, path.as_deref()).ok()
            }),
            env,
            Arc::new(|| Box::pin(crate::env::resolve())),
        )
    }
```

`with_parts` 多两个参数 `env: SharedEnv`、`refresh_env`（顺序同上面测试）。`executor_reports` 在拿缓存锁之后、计算 `due` 之前加：

```rust
        if fresh {
            let resolved = (self.refresh_env)().await;
            *self.env.write().unwrap_or_else(PoisonError::into_inner) = resolved;
        }
```

并实现 `fn env(&self) -> Option<ExecEnv> { Some(self.env.read().unwrap_or_else(PoisonError::into_inner).clone()) }`。现有测试里所有 `with_parts(...)` 调用补上 `ExecEnv::inherit().shared()` 和一个返回 `ExecEnv::inherit()` 的 `refresh_env`。

5. `DoctorReport` 加字段：

```rust
    /// The PATH and proxy executors get; `None` when the probe does not resolve one.
    #[serde(default)]
    pub env: Option<ExecEnv>,
```

`Engine::doctor` 在 `executor_reports(true)` 之后填 `env: self.inner.readiness.env()`。

`crates/devhost/src/main.rs` 的 `Command::Run`：

```rust
            let env = issuerelay_core::env::resolve().await.shared();
            let engine = Engine::new(EngineDeps {
                config_path: paths.config_file(),
                store: Store::open(&paths.state_db())?,
                linear,
                readiness: Arc::new(SystemReadiness::new(Duration::from_secs(300), env.clone())),
                job_runner: Arc::new(ProcessJobRunner::new(env)),
                logs_dir: paths.logs_dir(),
            })?;
```

Run: Step 7 的两条命令，再跑 `cargo test -p issuerelay-core`
Expected: PASS。

- [ ] **Step 9: 命令行的 doctor 输出加环境一节（先写失败测试）**

`crates/cli/src/format.rs` 的测试模块加（`doctor_report()` 用文件里现有的样例构造函数；没有就照 `DoctorReport` 字段写一个）：

```rust
    #[test]
    fn doctor_shows_where_path_and_proxy_come_from_with_credentials_masked() {
        let mut report = doctor_report();
        report.env = Some(ExecEnv {
            path: Some("/opt/demo/bin:/usr/bin".into()),
            path_source: EnvSource::LoginShell,
            proxy: vec![("HTTPS_PROXY".into(), "http://jane:s3cret@proxy.example:8080".into())],
            proxy_source: EnvSource::SystemSettings,
            problems: vec!["could not read the login shell's environment: timed out".into()],
        });
        let text = format_doctor(&report);
        assert!(text.contains("PATH: login shell"), "{text}");
        assert!(text.contains("Proxy: system settings"), "{text}");
        assert!(text.contains("HTTPS_PROXY=http://***@proxy.example:8080"), "{text}");
        assert!(!text.contains("s3cret"), "{text}");
        assert!(text.contains("timed out"), "{text}");
    }
```

Run: `cargo test -p issuerelay-cli doctor_shows_where_path_and_proxy_come_from`
Expected: FAIL。

- [ ] **Step 10: 实现命令行输出**

`format_doctor`（以文件里实际的函数名为准）在执行者一节之后加：

```rust
    if let Some(env) = &report.env {
        let source = |s: EnvSource| match s {
            EnvSource::Host => "inherited from the host",
            EnvSource::LoginShell => "login shell",
            EnvSource::SystemSettings => "system settings",
            EnvSource::None => "none",
        };
        lines.push(format!("PATH: {}", source(env.path_source)));
        lines.push(format!("Proxy: {}", source(env.proxy_source)));
        for (name, value) in env.proxy_for_display() {
            lines.push(format!("  {name}={}", clean(&value)));
        }
        for problem in &env.problems {
            lines.push(format!("  ! {}", clean(problem)));
        }
    }
```

（`clean` 为文件里现有的外来文本清洗函数；`lines` 换成函数里实际的输出缓冲。）

Run: `cargo test -p issuerelay-cli`
Expected: PASS。

- [ ] **Step 11: 文档与四条验证命令**

- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节表格加一行：`| 执行者环境（PATH、代理） | crates/core/src/env.rs |`。
- `.github/CODEOWNERS` 加 `/crates/core/src/env.rs`（它决定执行者子进程拿到什么环境）。
- 设计第 6.4 节最后一段补一句：「检查命令在用户主目录、在自己的进程组（Windows：Job Object）里运行，超时连同它启动的进程一起结束；『环境检查』页重查时同时重新读取 PATH 与代理。」

Run: 四条验证命令
Expected: 全部通过，`check-secrets` 无输出。

- [ ] **Step 12: Commit**

```bash
git add crates/core crates/cli crates/devhost docs .github Cargo.lock
git commit -m "feat(core): give executors the login shell's PATH and the system proxy"
```

---

### Task 2: Windows：不弹控制台窗口、执行者不收 Ctrl+C、命名管道只认当前用户

> 执行说明（2026-10-13）：已落地代码以提交 `6fb6db4` 与修复 `0983008` 为准，与下文有三处不同。① 管道描述符改为运行时按宿主所在用户的 SID 生成：`pipe_sddl(user_sid)` 给出 `O:<sid>D:P(A;;GA;;;<sid>)(A;;GA;;;SY)`，取代下文的常量 `PIPE_SDDL` 与 `OW`；`OW` 在宿主提权运行时会指向 Administrators 组（控制者裁决 R11）。② `server_runs_as_current_user` 接收 `BorrowedHandle<'_>`，不是 `RawHandle`。③ 同一任务还改写了 `SystemRunner::run`（裁决 R9）：两路输出各由自己的任务读取，等命令自己的进程退出（Windows 等原生子进程），再结束整组 / Job Object，输出最多再读 2 秒；读取出错或读取任务失败时，检查记为未完成。

M1 计划「本计划不覆盖」6.2、10.2 两行与评审遗留「`ipc` Windows 属主 SID」。Windows 代码只能在 CI 上编译和测试：这一任务的提交推上去后，主智能体要看 Windows 任务的结果，失败就进入修复轮。函数签名一律以本机 `~/.cargo/registry/src/*/windows-sys-0.61.2/` 里的声明为准（下面的写法已对照过）。

**Files:**
- Modify: `crates/core/Cargo.toml`（Windows 依赖：`windows` 0.62 的 `Win32_System_Threading`；`windows-sys` 0.61 的 `Win32_Foundation`、`Win32_Security`、`Win32_Security_Authorization`、`Win32_System_Threading`、`Win32_System_Pipes`）
- Modify: `crates/core/src/runner.rs`（`creation_flags()`，执行者带上它）
- Modify: `crates/core/src/doctor.rs`（检查命令带上它）
- Modify: `crates/core/src/env.rs`（无改动；Windows 不读登录 shell）
- Create: `crates/core/src/ipc/pipe_security.rs`
- Modify: `crates/core/src/ipc/mod.rs`、`ipc/server.rs`、`ipc/client.rs`
- Test: `crates/core/tests/ipc.rs`（Windows 测试）、`crates/core/src/runner.rs` 的测试模块

**Interfaces:**
- Produces:
  - `#[cfg(windows)] pub(crate) fn runner::creation_flags() -> process_wrap::tokio::CreationFlags`（`CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` = `0x0800_0200`）
  - `#[cfg(windows)] pub const ipc::pipe_security::PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)"`
  - `#[cfg(windows)] pub struct ipc::pipe_security::PipeSecurity`：`new() -> io::Result<Self>`、`attributes(&self) -> *mut c_void`
  - `#[cfg(windows)] pub fn ipc::pipe_security::server_runs_as_current_user(handle: RawHandle) -> io::Result<bool>`
- Consumes: Task 1 的 `SystemRunner::run`（进程组 / Job Object 写法）

- [ ] **Step 1: 写失败测试**

`crates/core/src/runner.rs` 测试模块加：

```rust
    #[cfg(windows)]
    #[test]
    fn executors_start_without_a_console_window_and_out_of_reach_of_ctrl_c() {
        // CREATE_NO_WINDOW (0x0800_0000) | CREATE_NEW_PROCESS_GROUP (0x0000_0200).
        assert_eq!(creation_flags().0.0, 0x0800_0200);
    }
```

`crates/core/tests/ipc.rs` 加（文件顶部已有的 `use` 不重复写）：

```rust
#[cfg(windows)]
#[tokio::test]
async fn the_pipe_admits_only_its_owner_and_system() {
    use std::os::windows::io::AsRawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(dir.path());
    let endpoint = paths.ipc_endpoint();
    let IpcEndpoint::Pipe(name) = &endpoint else {
        unreachable!("Windows uses a named pipe")
    };
    let _server = IpcServer::bind(&endpoint).await.unwrap();
    let client = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(name)
        .unwrap();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: a live pipe handle; the descriptor is freed below.
    let status = unsafe {
        GetSecurityInfo(
            client.as_raw_handle(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, 0, "GetSecurityInfo failed");
    let mut text: *mut u16 = null_mut();
    // SAFETY: `descriptor` came from GetSecurityInfo; `text` is freed below.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            null_mut(),
        )
    };
    assert_ne!(ok, 0, "cannot read the DACL as SDDL");
    // SAFETY: NUL-terminated UTF-16 from the call above.
    let sddl = unsafe {
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    unsafe {
        LocalFree(text.cast());
        LocalFree(descriptor);
    }
    assert!(sddl.contains(";;;OW)") && sddl.contains(";;;SY)"), "{sddl}");
    assert!(!sddl.contains(";;;WD)") && !sddl.contains(";;;AN)"), "Everyone and Anonymous must have no access: {sddl}");
}

#[cfg(windows)]
#[tokio::test]
async fn a_client_checks_that_the_server_runs_as_the_same_user() {
    use std::os::windows::io::AsRawHandle;
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(dir.path());
    let endpoint = paths.ipc_endpoint();
    let IpcEndpoint::Pipe(name) = &endpoint else {
        unreachable!("Windows uses a named pipe")
    };
    let _server = IpcServer::bind(&endpoint).await.unwrap();
    let client = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(name)
        .unwrap();
    assert!(issuerelay_core::ipc::pipe_security::server_runs_as_current_user(client.as_raw_handle()).unwrap());
}
```

Run: `cargo test -p issuerelay-core`（在 macOS 上这两个测试不编译；它们在 CI 的 Windows 任务里跑）
Expected: macOS 上其余测试照常通过；Windows 上编译失败（`creation_flags`、`pipe_security` 不存在）。

- [ ] **Step 2: 依赖**

```bash
cargo add -p issuerelay-core windows@0.62 --target 'cfg(windows)' --features Win32_System_Threading
cargo add -p issuerelay-core windows-sys@0.61 --target 'cfg(windows)' --features Win32_Foundation,Win32_Security,Win32_Security_Authorization,Win32_System_Threading,Win32_System_Pipes
```

两个都加注释说明用途：`windows` 只为 process-wrap 的 `CreationFlags` 类型（必须与 process-wrap 同版本），`windows-sys` 用于命名管道的安全描述符与属主核对。

- [ ] **Step 3: 建进程标志**

`crates/core/src/runner.rs`：

```rust
/// Windows: how executors and check commands are created. `CREATE_NO_WINDOW` keeps a console
/// window from opening for every run when the host is the tray app (closing that window would
/// end the run); `CREATE_NEW_PROCESS_GROUP` keeps a Ctrl+C in the devhost's console from
/// reaching the executor, so a stopped run is recorded as stopped by the host instead of as a
/// failure. process-wrap keeps these flags when the Job Object adds `CREATE_SUSPENDED`.
#[cfg(windows)]
pub(crate) fn creation_flags() -> process_wrap::tokio::CreationFlags {
    use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
    process_wrap::tokio::CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
}
```

在 `run` 里 `command.wrap(JobObject);` 之前加 `command.wrap(creation_flags());`（同在 `#[cfg(windows)]` 下）。`doctor.rs` 的 `SystemRunner::run` 同样在 `command.wrap(JobObject);` 之前加 `command.wrap(crate::runner::creation_flags());`。

- [ ] **Step 4: 管道安全描述符与属主核对**

新建 `crates/core/src/ipc/pipe_security.rs`：

```rust
//! Windows named-pipe security: only the user the host runs as (and SYSTEM) may open
//! IssueRelay's pipe, and a client talks only to a pipe whose server runs as the same user
//! (design section 10.2). Without a descriptor of its own a pipe lets Everyone read, and any
//! local user could create the pipe first under IssueRelay's name.
use std::ffi::c_void;
use std::io;
use std::os::windows::io::RawHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Full access for the pipe's owner, the user the host runs as (`OW`), and for SYSTEM; nobody
/// else, not even the read access Everyone gets by default. `P` keeps inherited entries out.
pub const PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

/// A security descriptor built from `PIPE_SDDL`, kept for the server's life: every pipe
/// instance is created with it.
pub struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is a heap block owned by this value and never changed after `new`;
// CreateNamedPipeW only reads it, from whichever thread creates an instance.
unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    pub fn new() -> io::Result<Self> {
        let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16. On success the system allocates the
        // descriptor with LocalAlloc; `Drop` frees it.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    /// The attributes for `ServerOptions::create_with_security_attributes_raw`. Valid while
    /// `self` lives and is not moved; pass it straight to the call.
    pub fn attributes(&self) -> *mut c_void {
        std::ptr::from_ref(&self.attributes).cast_mut().cast()
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW, freed once.
        unsafe { LocalFree(self.descriptor) };
    }
}

/// The user SID of a process's token, kept in the buffer GetTokenInformation filled.
struct TokenUserSid {
    buffer: Vec<u8>,
}

impl TokenUserSid {
    fn of_process(process: HANDLE) -> io::Result<Self> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: `process` has query access; the token handle is closed below.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = Self::of_token(token);
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(token) };
        result
    }

    fn of_token(token: HANDLE) -> io::Result<Self> {
        let mut needed = 0u32;
        // SAFETY: a size query: no buffer, only `needed` is written.
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u8; needed as usize];
        // SAFETY: `buffer` has `needed` writable bytes.
        let ok = unsafe {
            GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), needed, &mut needed)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { buffer })
    }

    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation(TokenUser) wrote a TOKEN_USER at the start of the buffer,
        // whose `Sid` points into the buffer. A Vec<u8> is not aligned for TOKEN_USER, hence the
        // unaligned read.
        unsafe { std::ptr::read_unaligned(self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// Whether the process serving the pipe behind `handle` (a client end) runs as the same user as
/// this process. An error means it could not be told, which callers treat like a "no".
pub fn server_runs_as_current_user(handle: RawHandle) -> io::Result<bool> {
    let mut pid = 0u32;
    // SAFETY: `handle` is an open client end of a named pipe.
    if unsafe { GetNamedPipeServerProcessId(handle as HANDLE, &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is checked and closed below.
    let server = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if server.is_null() {
        return Err(io::Error::last_os_error());
    }
    let theirs = TokenUserSid::of_process(server);
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(server) };
    let theirs = theirs?;
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing.
    let ours = TokenUserSid::of_process(unsafe { GetCurrentProcess() })?;
    // SAFETY: both SIDs point into buffers that live until the end of this function.
    Ok(unsafe { EqualSid(theirs.sid(), ours.sid()) } != 0)
}
```

`crates/core/src/ipc/mod.rs` 加 `#[cfg(windows)] pub mod pipe_security;`。

`crates/core/src/ipc/server.rs`：`Listener::Pipe` 加字段 `security: crate::ipc::pipe_security::PipeSecurity`；`bind_pipe(name)` 改为返回 `(NamedPipeServer, PipeSecurity)`，先 `PipeSecurity::new()?`，再：

```rust
    // SAFETY: the attributes point at a live descriptor owned by `security`, which the server
    // keeps for as long as it creates instances.
    let created = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, security.attributes())
    };
```

`new_pipe_instance(name)` 改为 `new_pipe_instance(name, &security)`，同样用 `create_with_security_attributes_raw`。`bind` 与 `run` 里的调用点相应传入 `security`。

`crates/core/src/ipc/client.rs` 的 `open_pipe` 在 `Ok(client)` 分支里核对：

```rust
            Ok(client) => {
                use std::os::windows::io::AsRawHandle;
                match crate::ipc::pipe_security::server_runs_as_current_user(client.as_raw_handle()) {
                    Ok(true) => return Ok(client),
                    Ok(false) => bail!("IssueRelay's pipe is served by another user's process; refusing to talk to it"),
                    Err(e) => bail!("cannot check who serves IssueRelay's pipe: {e}"),
                }
            }
```

- [ ] **Step 5: 验证**

Run: 四条验证命令（macOS）。然后由主智能体推送分支，看 CI 的 `test (windows-latest)`。
Expected: macOS 全部通过；Windows 任务里两个新测试和全部 IPC、运行器测试通过。

- [ ] **Step 6: 文档与提交**

- 设计第 10.2 节「只允许当前用户访问」后补：「Windows 管道的安全描述符只给属主（宿主所在用户）和 SYSTEM 访问权；客户端连接后核对管道服务端进程的用户，与自己不同就拒绝通信。」第 6.2 节补：「Windows 上执行者以 `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` 启动：不弹控制台窗口，控制台的 Ctrl+C 不直接打到执行者。」
- M0–M1 计划「本计划不覆盖」表的 6.2（`CREATE_NO_WINDOW`）与 10.2 两行末尾加「（M2 第一部分 Task 2 已完成，待 Windows 实机）」。
- `.github/CODEOWNERS` 加 `/crates/core/src/ipc/pipe_security.rs`。

```bash
git add crates/core docs .github Cargo.lock
git commit -m "feat(core): hide executor consoles on Windows and lock the pipe to its user"
```

---

### Task 3: 日志清理与运行器的输出保护

设计第 11 节：日志按保留天数和总大小清理（默认 30 天、1 GB）。评审遗留：通道与单行没有上限、0600 只对新建文件生效、写日志失败无人知道、日志被清理后 `log_tail` 报错。

**Files:**
- Create: `crates/core/src/logs.rs`
- Modify: `crates/core/src/lib.rs`（`pub mod logs;`）
- Modify: `crates/core/src/config.rs`（`log_retention_days`、`log_max_total_mb`）
- Modify: `crates/core/src/engine.rs`（`clean_logs`、`run_forever` 定期清理、`log_tail` 对缺失文件返回空）
- Modify: `crates/core/src/runner.rs`（有界通道、单行上限、已有日志文件收紧权限、首次写失败告警）
- Test: `crates/core/src/logs.rs`、`crates/core/src/runner.rs`、`crates/core/src/config.rs` 的测试模块，`crates/core/tests/engine.rs`、`crates/core/tests/runner.rs`

**Interfaces:**
- Produces:
  - `pub struct logs::CleanupReport { pub removed: usize, pub freed_bytes: u64 }`（`Debug, Clone, Copy, Default, PartialEq, Eq`）
  - `pub fn logs::cleanup(dir: &Path, keep: Duration, max_total: u64, in_use: &HashSet<PathBuf>, now: SystemTime) -> Result<CleanupReport>`
  - `Settings.log_retention_days: u64`（默认 30，至少 1）、`Settings.log_max_total_mb: u64`（默认 1024，至少 1）
  - `pub async fn Engine::clean_logs(&self) -> Result<CleanupReport>`
  - `Engine::log_tail` 在日志文件已不存在时返回 `Ok(vec![])`
- Consumes: Task 1 的 `runner::run(..., env)` 签名

- [ ] **Step 1: `logs::cleanup` 的失败测试**

新建 `crates/core/src/logs.rs`，先放模块注释、`CleanupReport` 与 `cleanup` 的空实现（返回默认值），再加测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(86_400);

    /// Writes `bytes` bytes to `dir/name` and dates the file `age` before `now`.
    fn log(dir: &Path, name: &str, bytes: usize, age: Duration, now: SystemTime) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(now - age).unwrap();
        path
    }

    #[test]
    fn old_logs_go_and_recent_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let old = log(dir.path(), "1.jsonl", 10, 31 * DAY, now);
        let recent = log(dir.path(), "2.jsonl", 10, 29 * DAY, now);
        let report = cleanup(dir.path(), 30 * DAY, u64::MAX, &HashSet::new(), now).unwrap();
        assert_eq!(report, CleanupReport { removed: 1, freed_bytes: 10 });
        assert!(!old.exists());
        assert!(recent.exists());
    }

    #[test]
    fn the_oldest_logs_go_until_the_rest_fits_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let a = log(dir.path(), "1.jsonl", 100, 3 * DAY, now);
        let b = log(dir.path(), "2.jsonl", 100, 2 * DAY, now);
        let c = log(dir.path(), "3.jsonl", 100, DAY, now);
        let report = cleanup(dir.path(), 30 * DAY, 150, &HashSet::new(), now).unwrap();
        assert_eq!(report.removed, 2);
        assert!(!a.exists() && !b.exists());
        assert!(c.exists(), "the newest log fits the limit on its own");
    }

    #[test]
    fn logs_of_runs_in_progress_and_other_files_are_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let running = log(dir.path(), "7.jsonl", 100, 40 * DAY, now);
        let note = log(dir.path(), "notes.txt", 100, 40 * DAY, now);
        let odd = log(dir.path(), "x7.jsonl", 100, 40 * DAY, now);
        let in_use: HashSet<PathBuf> = [running.clone()].into();
        let report = cleanup(dir.path(), 30 * DAY, 0, &in_use, now).unwrap();
        assert_eq!(report.removed, 0);
        assert!(running.exists() && note.exists() && odd.exists());
    }

    #[test]
    fn a_missing_directory_is_nothing_to_clean() {
        let dir = tempfile::tempdir().unwrap();
        let report = cleanup(&dir.path().join("logs"), DAY, 0, &HashSet::new(), SystemTime::now()).unwrap();
        assert_eq!(report, CleanupReport::default());
    }
}
```

Run: `cargo test -p issuerelay-core --lib logs::`
Expected: FAIL（空实现什么都不删）。

- [ ] **Step 2: 实现 `logs.rs`**

```rust
//! Log retention: run logs older than the retention period, and the oldest ones beyond the total
//! size limit, are deleted (design section 11; 30 days and 1 GB by default).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

/// What one cleanup removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed: usize,
    pub freed_bytes: u64,
}

/// A run log as the engine names it: `<job id>.jsonl`.
fn is_run_log(name: &str) -> bool {
    name.strip_suffix(".jsonl")
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Deletes the run logs in `dir` last modified before `now - keep`, then the oldest of the rest
/// until they take at most `max_total` bytes. Logs in `in_use` (runs in progress) and files that
/// are not run logs are never deleted. A missing directory is nothing to clean; a log that cannot
/// be deleted is logged and skipped.
pub fn cleanup(
    dir: &Path,
    keep: Duration,
    max_total: u64,
    in_use: &HashSet<PathBuf>,
    now: SystemTime,
) -> Result<CleanupReport> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CleanupReport::default()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", dir.display())),
    };
    let mut logs: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read {}", dir.display()))?;
        if !entry.file_name().to_str().is_some_and(is_run_log) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        logs.push((meta.modified().unwrap_or(now), meta.len(), entry.path()));
    }
    // Oldest first: expired logs come first, and the size limit removes the oldest.
    logs.sort_by_key(|(modified, _, _)| *modified);
    let cutoff = now.checked_sub(keep);
    let mut total: u64 = logs.iter().map(|(_, len, _)| *len).sum();
    let mut report = CleanupReport::default();
    for (modified, len, path) in &logs {
        let expired = cutoff.is_some_and(|cutoff| *modified < cutoff);
        if !(expired || total > max_total) || in_use.contains(path) {
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {
                report.removed += 1;
                report.freed_bytes += len;
                total -= len;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => total -= len,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not delete an old run log")
            }
        }
    }
    Ok(report)
}
```

Run: `cargo test -p issuerelay-core --lib logs::`
Expected: PASS。

- [ ] **Step 3: 设置项（先写失败测试）**

`crates/core/src/config.rs` 测试模块加：

```rust
    #[test]
    fn log_retention_has_defaults_and_lower_bounds() {
        let settings = Settings::default();
        assert_eq!(settings.log_retention_days, 30);
        assert_eq!(settings.log_max_total_mb, 1024);
        for change in [
            (|s: &mut Settings| s.log_retention_days = 0) as fn(&mut Settings),
            |s: &mut Settings| s.log_max_total_mb = 0,
        ] {
            let mut bad = Settings::default();
            change(&mut bad);
            assert!(bad.validate().is_err());
        }
        let old_file: Config = toml::from_str("[settings]\npoll_interval_secs = 30\n").unwrap();
        assert_eq!(old_file.settings.log_retention_days, 30, "files from M1 get the default");
    }
```

Run: `cargo test -p issuerelay-core --lib log_retention_has_defaults`
Expected: 编译失败。

- [ ] **Step 4: 实现设置项**

`Settings` 在 `stall_timeout_mins` 之后加：

```rust
    /// Days a run log is kept (at least 1).
    pub log_retention_days: u64,
    /// The most megabytes all run logs together may take before the oldest are deleted (at least 1).
    pub log_max_total_mb: u64,
```

`Default` 里 `log_retention_days: 30, log_max_total_mb: 1024`；`validate` 加：

```rust
        anyhow::ensure!(self.log_retention_days >= 1, "log_retention_days must be at least 1");
        anyhow::ensure!(self.log_max_total_mb >= 1, "log_max_total_mb must be at least 1");
```

Run: `cargo test -p issuerelay-core --lib config::`
Expected: PASS。

- [ ] **Step 5: 引擎的清理与 `log_tail`（先写失败测试）**

`crates/core/tests/engine.rs` 加（`Harness`、`issue` 为 `tests/common` 里现有的辅助；`ScriptedRunner` 会在任务的日志路径写一行）：

```rust
#[tokio::test]
async fn old_logs_are_cleaned_and_a_cleaned_log_reads_as_empty() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    let log_path = job.log_path.clone().expect("the run wrote a log");
    let file = std::fs::File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::SystemTime::now() - Duration::from_secs(40 * 86_400))
        .unwrap();
    let report = h.engine.clean_logs().await.unwrap();
    assert_eq!(report.removed, 1);
    assert!(!log_path.exists());
    assert_eq!(h.engine.log_tail(job.id, 10).unwrap(), Vec::<String>::new());
}
```

Run: `cargo test -p issuerelay-core --test engine old_logs_are_cleaned`
Expected: 编译失败。

- [ ] **Step 6: 实现引擎部分**

`crates/core/src/engine.rs`：

```rust
/// How often the background loop deletes old run logs (also once when it starts).
const LOG_CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
```

```rust
    /// Deletes run logs past the retention settings (`log_retention_days`, `log_max_total_mb`),
    /// never the log of a run in progress. The background loop calls it when it starts and every
    /// `LOG_CLEANUP_INTERVAL`.
    pub async fn clean_logs(&self) -> Result<CleanupReport> {
        let settings = self.config().settings;
        let in_use: HashSet<PathBuf> = self
            .store()
            .list_by_state(&[JobState::Running])?
            .into_iter()
            .filter_map(|job| job.log_path)
            .collect();
        let dir = self.inner.logs_dir.clone();
        let keep = Duration::from_secs(settings.log_retention_days.saturating_mul(86_400));
        let max_total = settings.log_max_total_mb.saturating_mul(1024 * 1024);
        let report = tokio::task::spawn_blocking(move || {
            logs::cleanup(&dir, keep, max_total, &in_use, std::time::SystemTime::now())
        })
        .await
        .context("the log cleanup stopped unexpectedly")??;
        if report.removed > 0 {
            tracing::info!(removed = report.removed, freed_bytes = report.freed_bytes, "old run logs deleted");
        }
        Ok(report)
    }
```

`run_forever`：在 `while` 之前加

```rust
        let mut last_cleanup = tokio::time::Instant::now();
        if let Err(e) = self.clean_logs().await {
            tracing::warn!(error = %format!("{e:#}"), "log cleanup failed");
        }
```

在循环体 `tick` 之后加

```rust
            if last_cleanup.elapsed() >= LOG_CLEANUP_INTERVAL {
                last_cleanup = tokio::time::Instant::now();
                if let Err(e) = self.clean_logs().await {
                    tracing::warn!(error = %format!("{e:#}"), "log cleanup failed");
                }
            }
```

`log_tail`：`File::open` 失败且 `kind() == NotFound` 时返回 `Ok(Vec::new())`（文档注释加「a log deleted by the cleanup reads as empty」），其余错误照旧。

Run: `cargo test -p issuerelay-core --test engine`
Expected: PASS。

- [ ] **Step 7: 运行器的输出保护（先写失败测试）**

`crates/core/src/runner.rs` 测试模块加：

```rust
    #[tokio::test]
    async fn an_overlong_line_is_cut_and_the_next_line_still_arrives() {
        let input = [vec![b'a'; 50], b"\nshort\r\nlast".to_vec()].concat();
        let mut reader = tokio::io::BufReader::with_capacity(8, &input[..]);
        let mut buf = Vec::new();
        let read = read_line_capped(&mut reader, &mut buf, 10).await.unwrap();
        assert_eq!(read, 51, "the whole line, newline included, is consumed");
        assert_eq!(buf, vec![b'a'; 10]);
        buf.clear();
        assert_eq!(read_line_capped(&mut reader, &mut buf, 10).await.unwrap(), 7);
        assert_eq!(buf, b"short\r\n");
        buf.clear();
        assert_eq!(read_line_capped(&mut reader, &mut buf, 10).await.unwrap(), 4);
        assert_eq!(buf, b"last");
        buf.clear();
        assert_eq!(read_line_capped(&mut reader, &mut buf, 10).await.unwrap(), 0);
    }
```

`crates/core/tests/runner.rs` 加（Unix）：

```rust
#[cfg(unix)]
#[tokio::test]
async fn a_reused_log_file_is_made_private_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(dir.path(), r#"[{"out": "hello"}]"#);
    let log = dir.path().join("3.jsonl");
    std::fs::write(&log, "old attempt\n").unwrap();
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
    let output = runner::run(
        fake_agent_spec(&script, dir.path()),
        ExecutorKind::Claude,
        &log,
        limits(),
        CancellationToken::new(),
        &ExecEnv::inherit(),
    )
    .await;
    assert_eq!(output.exit, ExitKind::Exited(Some(0)));
    let mode = std::fs::metadata(&log).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!std::fs::read_to_string(&log).unwrap().contains("old attempt"));
}
```

Run: `cargo test -p issuerelay-core --lib an_overlong_line_is_cut` 与 `cargo test -p issuerelay-core --test runner a_reused_log_file_is_made_private_again`
Expected: 第一个编译失败（`read_line_capped` 不存在），第二个 FAIL（权限仍是 0644）。

- [ ] **Step 8: 实现运行器部分**

`crates/core/src/runner.rs`：

```rust
/// How many output lines may wait between the pipe readers and the log writer. A full queue makes
/// the readers wait, and with them the executor's writes, instead of growing without bound.
const LINE_QUEUE: usize = 1024;

/// The most bytes of one output line that are kept; the rest of a longer line is read and
/// dropped, and a note in the log says so.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Reads one line into `buf` like `read_until(b'\n', buf)`, but keeps at most `cap` bytes of it:
/// the rest of an overlong line is read and dropped, so one runaway line cannot use up memory.
/// Returns how many bytes were read, newline included; 0 at the end of the input.
async fn read_line_capped<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<usize> {
    let mut read = 0;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(read);
        }
        let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
            Some(end) => (&available[..=end], true),
            None => (available, false),
        };
        let room = cap.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = chunk.len();
        reader.consume(used);
        read += used;
        if done {
            return Ok(read);
        }
    }
}
```

`for_each_line` 换成：

```rust
/// Sends each line of `reader` to `tx` (wrapped by `wrap`) until the end of input, a read error,
/// or the receiver going away. Lines end at `\n` (a `\r` before it is dropped) and are decoded
/// lossily: one line of invalid UTF-8 must not end the stream, or every later event, including
/// the final result, would be lost and the executor would get a broken pipe. A line longer than
/// `MAX_LINE_BYTES` is cut, followed by a stderr note saying so.
async fn forward_lines<R: AsyncBufRead + Unpin>(
    mut reader: R,
    tx: mpsc::Sender<Line>,
    wrap: fn(String) -> Line,
) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = match read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        if buf.ends_with(b"\n") {
            buf.pop();
            if buf.ends_with(b"\r") {
                buf.pop();
            }
        }
        let cut = read > buf.len() + 2;
        if tx.send(wrap(String::from_utf8_lossy(&buf).into_owned())).await.is_err() {
            return;
        }
        if cut {
            let note = format!("[IssueRelay] a {read}-byte output line was cut to {MAX_LINE_BYTES} bytes");
            if tx.send(Line::Err(note)).await.is_err() {
                return;
            }
        }
    }
}
```

`run` 里改成 `let (tx, mut rx) = mpsc::channel(LINE_QUEUE);`，两个读取任务分别 `forward_lines(BufReader::new(stdout), tx, Line::Out).await;` 与 `forward_lines(BufReader::new(stderr), tx, Line::Err).await;`（各自 clone 一个 `tx`）。`record` 改为返回 `std::io::Result<()>`；`run` 维护 `let mut log_failed = false;`，第一次失败时：

```rust
                        if let Err(e) = record(&mut log, &mut tracker, line).await {
                            if !log_failed {
                                log_failed = true;
                                tracing::warn!(path = %log_path.display(), error = %e, "cannot write the run log; later lines may be missing");
                            }
                        }
```

（收尾 drain 那段同样处理。）`create_log` 在打开之后、Unix 上收紧已有文件的权限：

```rust
async fn create_log(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    make_private(&mut options);
    let file = tokio::fs::OpenOptions::from(options).open(path).await?;
    // `mode` applies only to a file being created; a retry reuses the previous attempt's file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).await?;
    }
    Ok(file)
}
```

Run: Step 7 的两条命令，再跑 `cargo test -p issuerelay-core`
Expected: PASS。

- [ ] **Step 9: 文档与提交**

- 设计第 11 节「日志按保留天数和总大小清理（默认 30 天、1 GB）」后补：「配置项 `log_retention_days`、`log_max_total_mb`；宿主启动时和之后每 6 小时清理一次，正在运行的任务的日志不删。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 日志清理 | crates/core/src/logs.rs |`；第 6 节 `logs/` 一行注明清理策略。
- M0–M1 计划「本计划不覆盖」表第 11 行末尾加「（M2 第一部分 Task 3 已完成）」。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates/core docs Cargo.lock
git commit -m "feat(core): clean old run logs and bound runaway executor output"
```

---

### Task 4: 表结构第 2 版与宿主崩溃后的残留清扫

M1 计划「本计划不覆盖」6.2 行「宿主崩溃后的残留进程」与评审遗留「`store` 拒绝更新版本的数据库」。macOS 上，宿主被强杀后执行者成了孤儿继续跑（它领着自己的进程组），下次启动时要结束它；Windows 上 Job Object 带 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`（运行器注册了 `KillOnDrop`，process-wrap 10 因此设置该标志），宿主一死整棵树随之结束，不需要清扫。

**Files:**
- Modify: `crates/core/src/model.rs`（`ProcessTag`）
- Modify: `crates/core/src/store.rs`（第 2 版表结构、迁移、拒绝更新版本、`set_process`、`recover_interrupted` 返回清单、`mark_started` 清空进程列）
- Modify: `crates/core/src/runner.rs`（`process_tag`、`SpawnHook`、`run` 的 `on_spawn` 参数、`end_leftover`）
- Modify: `crates/core/src/engine.rs`（`JobRunner::run` 多一个 `on_spawn`；启动时记下待清扫清单，`run_forever` 开头清扫；开跑时登记进程）
- Modify: `crates/core/tests/common/engine_kit.rs`（`ScriptedRunner` 实现新签名）
- Test: `crates/core/src/store.rs` 测试模块、`crates/core/tests/runner.rs`

**Interfaces:**
- Produces:
  - `pub struct model::ProcessTag { pub pid: u32, pub started: String }`（`Debug, Clone, PartialEq, Eq, Serialize, Deserialize`；`started` 为 `"<秒>.<6 位微秒>"`）
  - `pub struct store::InterruptedRun { pub job_id: i64, pub process: Option<ProcessTag> }`
  - `Store::recover_interrupted(&self) -> Result<Vec<InterruptedRun>>`（原来返回 `usize`）
  - `Store::set_process(&self, id: i64, process: &ProcessTag) -> Result<()>`（只改 `running` 的行）
  - `pub type runner::SpawnHook = Box<dyn FnOnce(ProcessTag) + Send>`
  - `pub fn runner::process_tag(pid: u32) -> Option<ProcessTag>`（只有 macOS 返回 `Some`）
  - `pub async fn runner::end_leftover(tag: &ProcessTag) -> bool`
  - `runner::run(spec, kind, log_path, limits, cancel, env, on_spawn: Option<SpawnHook>) -> RunOutput`
  - `JobRunner::run(&self, spec, kind, log_path, limits, cancel, on_spawn: SpawnHook) -> RunOutput`
- Consumes: Task 1、3 的 `runner::run` 签名

- [ ] **Step 1: 存储的失败测试**

`crates/core/src/store.rs` 测试模块：删掉 `an_existing_schema_version_is_kept`（新规则下版本更高要报错），加：

```rust
    /// The M1 layout (schema 1), with one running row.
    const SCHEMA_V1: &str = "CREATE TABLE jobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT, issue_uuid TEXT NOT NULL, issue_identifier TEXT NOT NULL,
        title TEXT NOT NULL, binding_id TEXT, directory TEXT, executor TEXT, state TEXT NOT NULL,
        reason TEXT, priority INTEGER NOT NULL, issue_created_at TEXT NOT NULL,
        discovered_updated_at TEXT NOT NULL, verified_updated_at TEXT, created_at TEXT NOT NULL,
        started_at TEXT, finished_at TEXT, log_path TEXT, summary TEXT);
        INSERT INTO jobs (issue_uuid, issue_identifier, title, state, priority, issue_created_at,
            discovered_updated_at, created_at)
        VALUES ('u-1', 'DEMO-1', 'Demo', 'running', 0, '2026-10-01T08:00:00Z',
            '2026-10-01T08:00:00Z', '2026-10-01T08:00:00Z');
        PRAGMA user_version = 1;";

    #[test]
    fn an_m1_database_is_migrated_to_schema_2() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        Connection::open(&path).unwrap().execute_batch(SCHEMA_V1).unwrap();
        let store = Store::open(&path).unwrap();
        let version: i64 = store.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 2);
        let interrupted = store.recover_interrupted().unwrap();
        assert_eq!(interrupted, vec![InterruptedRun { job_id: 1, process: None }]);
        assert_eq!(store.get(1).unwrap().unwrap().state, JobState::Failed);
    }

    #[test]
    fn a_database_from_a_newer_release_is_refused() {
        let (_dir, path, store) = file_store();
        drop(store);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 3).unwrap();
        let error = Store::open(&path).err().expect("a newer schema must be refused");
        assert!(format!("{error:#}").contains("newer IssueRelay"), "{error:#}");
    }

    #[test]
    fn interrupted_runs_come_back_with_their_process() {
        let store = Store::open_in_memory().unwrap();
        let tagged = queued(&store, "1");
        let untagged = queued(&store, "2");
        let finished = queued(&store, "3");
        for job in [&tagged, &untagged, &finished] {
            assert!(store.mark_started(job.id, Path::new("x.jsonl")).unwrap());
        }
        let tag = ProcessTag { pid: 4242, started: "1700000000.000123".into() };
        store.set_process(tagged.id, &tag).unwrap();
        store.set_process(finished.id, &tag).unwrap();
        store.mark_finished(finished.id, JobState::AwaitingAcceptance, None, None, None).unwrap();
        let interrupted = store.recover_interrupted().unwrap();
        assert_eq!(
            interrupted,
            vec![
                InterruptedRun { job_id: tagged.id, process: Some(tag.clone()) },
                InterruptedRun { job_id: untagged.id, process: None },
            ]
        );
        assert!(store.recover_interrupted().unwrap().is_empty(), "they are no longer running");
    }

    #[test]
    fn a_restarted_run_forgets_the_previous_process() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        assert!(store.mark_started(job.id, Path::new("x.jsonl")).unwrap());
        store.set_process(job.id, &ProcessTag { pid: 1, started: "1.000001".into() }).unwrap();
        store.mark_finished(job.id, JobState::Failed, Some("boom"), None, None).unwrap();
        store.requeue(job.id).unwrap();
        assert!(store.mark_started(job.id, Path::new("x.jsonl")).unwrap());
        assert_eq!(store.recover_interrupted().unwrap()[0].process, None);
    }

    #[test]
    fn set_process_only_touches_running_rows() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        store.set_process(job.id, &ProcessTag { pid: 7, started: "1.000001".into() }).unwrap();
        assert!(store.mark_started(job.id, Path::new("x.jsonl")).unwrap());
        assert_eq!(store.recover_interrupted().unwrap()[0].process, None);
    }
```

Run: `cargo test -p issuerelay-core --lib store::`
Expected: 编译失败。

- [ ] **Step 2: 实现存储部分**

`crates/core/src/model.rs`：

```rust
/// The executor process of a run, as recorded when it started: the PID and the kernel's start
/// time, `"<seconds>.<microseconds>"` since the epoch. A later host ends the process only while
/// that PID still has that start time; otherwise the PID belongs to someone else now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessTag {
    pub pid: u32,
    pub started: String,
}
```

`crates/core/src/store.rs`：

- `SCHEMA` 的建表语句末尾（`summary TEXT` 之后）加 `process_pid INTEGER, process_started TEXT`；`SCHEMA_VERSION` 改为 `2`；`COLUMNS` 与 `row_to_job` 不变（进程列不进 `Job`）。
- 新类型：

```rust
/// A run still marked running when the host started: the previous host stopped without
/// recording its end. `process` is what was recorded about its executor, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedRun {
    pub job_id: i64,
    pub process: Option<ProcessTag>,
}
```

- `init` 改为：

```rust
    fn init(conn: Connection) -> Result<Self> {
        // 0 means a fresh database.
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        anyhow::ensure!(
            version <= SCHEMA_VERSION,
            "the run records were written by a newer IssueRelay (schema {version}; this build \
             knows up to {SCHEMA_VERSION}); install that release or a newer one"
        );
        // One transaction: a migration that stops halfway must not leave a half-changed table.
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(SCHEMA)?;
        if version == 1 {
            tx.execute_batch(
                "ALTER TABLE jobs ADD COLUMN process_pid INTEGER;
                 ALTER TABLE jobs ADD COLUMN process_started TEXT;",
            )?;
        }
        if version < SCHEMA_VERSION {
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        tx.commit()?;
        Ok(Self { conn })
    }
```

- `mark_started` 的 `UPDATE` 加 `process_pid = NULL, process_started = NULL`（一次新的运行不继承上一次的进程）。
- 新方法与改写：

```rust
    /// Records the executor process of a running job, so that a host started after a crash can
    /// end what the run left behind. Rows that are not running are left alone.
    pub fn set_process(&self, id: i64, process: &ProcessTag) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET process_pid = ?2, process_started = ?3 WHERE id = ?1 AND state = ?4",
            params![id, process.pid, process.started, JobState::Running.as_str()],
        )?;
        Ok(())
    }

    /// Only the single running host may call this, at startup, before any run starts: a second
    /// instance would mark the first one's runs as failed. Marks every run still marked running
    /// as failed (interrupted runs keep their verified value and are not retried) and returns
    /// them, oldest first, with the executor process recorded for each.
    pub fn recover_interrupted(&self) -> Result<Vec<InterruptedRun>> {
        let tx = self.conn.unchecked_transaction()?;
        let runs = {
            let mut statement = tx.prepare(
                "SELECT id, process_pid, process_started FROM jobs WHERE state = ?1 ORDER BY id",
            )?;
            let rows = statement.query_map(params![JobState::Running.as_str()], |row| {
                let pid: Option<i64> = row.get(1)?;
                let started: Option<String> = row.get(2)?;
                let process = match (pid.and_then(|pid| u32::try_from(pid).ok()), started) {
                    (Some(pid), Some(started)) => Some(ProcessTag { pid, started }),
                    _ => None,
                };
                Ok(InterruptedRun { job_id: row.get(0)?, process })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.execute(
            "UPDATE jobs SET state = ?2, reason = 'interrupted: IssueRelay stopped while the run was in progress', finished_at = ?1 WHERE state = ?3",
            params![fmt(Utc::now()), JobState::Failed.as_str(), JobState::Running.as_str()],
        )?;
        tx.commit()?;
        Ok(runs)
    }
```

（`use crate::model::ProcessTag;`。）

Run: `cargo test -p issuerelay-core --lib store::`
Expected: PASS。

- [ ] **Step 3: 运行器的失败测试**

`crates/core/tests/runner.rs` 加（macOS；其余平台不记进程，`process_tag` 返回 `None`）：

```rust
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_run_reports_its_executor_process_when_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(dir.path(), r#"[{"out": "hi"}]"#);
    let (tx, rx) = std::sync::mpsc::channel();
    let hook: runner::SpawnHook = Box::new(move |tag| tx.send(tag).unwrap());
    runner::run(
        fake_agent_spec(&script, dir.path()),
        ExecutorKind::Claude,
        &dir.path().join("1.jsonl"),
        limits(),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        Some(hook),
    )
    .await;
    let tag = rx.try_recv().expect("the hook ran");
    assert!(tag.pid > 1);
    let (secs, micros) = tag.started.split_once('.').unwrap();
    assert!(secs.parse::<u64>().unwrap() > 1_600_000_000);
    assert_eq!(micros.len(), 6);
}

/// Starts fake-agent the way a run does (leading a process group of its own), with a heartbeat
/// child, as an executor a crashed host left behind.
#[cfg(target_os = "macos")]
fn orphan(dir: &std::path::Path) -> (std::process::Child, std::path::PathBuf) {
    use std::os::unix::process::CommandExt;
    let beat = dir.join("beat");
    let script = write_script(
        dir,
        &format!(r#"[{{"heartbeat": "{}", "every_ms": 50}}, {{"sleep_ms": 30000}}]"#, beat.display()),
    );
    let child = std::process::Command::new(fake_agent_path())
        .arg(&script)
        .process_group(0)
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    (child, beat)
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_leftover_executor_tree_is_ended_on_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, beat) = orphan(dir.path());
    let tag = runner::process_tag(child.id()).expect("a live process has a start time");
    assert!(runner::end_leftover(&tag).await);
    let status = child.wait().unwrap();
    assert!(!status.success());
    let size = std::fs::metadata(&beat).unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(std::fs::metadata(&beat).unwrap().len(), size, "the heartbeat child ended too");
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_pid_with_another_start_time_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, _beat) = orphan(dir.path());
    let mut tag = runner::process_tag(child.id()).unwrap();
    tag.started = "1.000001".into();
    assert!(!runner::end_leftover(&tag).await);
    assert!(child.try_wait().unwrap().is_none(), "an unrelated process must keep running");
    child.kill().unwrap();
    child.wait().unwrap();
    let gone = runner::ProcessTag { pid: u32::MAX - 7, started: "1.000001".into() };
    assert!(!runner::end_leftover(&gone).await);
}
```

（`fake_agent_path()` 与 `write_script` 为文件里现有的辅助函数；`runner::ProcessTag` 若未从 `runner` 重新导出，就写 `issuerelay_core::model::ProcessTag`。）文件里其余 `runner::run(...)` 调用末尾补 `None`。

Run: `cargo test -p issuerelay-core --test runner`
Expected: 编译失败。

- [ ] **Step 4: 实现运行器部分**

`crates/core/src/runner.rs`（`use crate::model::ProcessTag;`）：

```rust
/// Called once with the executor's process, right after it started.
pub type SpawnHook = Box<dyn FnOnce(ProcessTag) + Send>;

/// The live process `pid` with its start time, for `Store::set_process`. macOS only: Windows
/// needs no record (the Job Object ends the whole tree when the host dies) and other Unix systems
/// are not target platforms.
#[cfg(target_os = "macos")]
pub fn process_tag(pid: u32) -> Option<ProcessTag> {
    let proc = descendants::process(libc::pid_t::try_from(pid).ok()?)?;
    Some(ProcessTag {
        pid,
        started: format_started(proc.started),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn process_tag(_pid: u32) -> Option<ProcessTag> {
    None
}

#[cfg(target_os = "macos")]
fn format_started((seconds, micros): (u64, u64)) -> String {
    format!("{seconds}.{micros:06}")
}

/// Ends what a run left behind after the host that started it died without ending it. If the
/// executor in `tag` still runs (same PID, same start time), its descendants are recorded, its
/// process group gets SIGTERM, it gets up to `TERM_GRACE` to exit, the descendants are recorded
/// again, and then the group (while its leader still runs) and every recorded descendant get
/// SIGKILL, as `TreeGuard::end` does for a run that is stopped. Returns whether the executor was
/// still running. A PID with another start time belongs to an unrelated process and is left
/// alone.
#[cfg(target_os = "macos")]
pub async fn end_leftover(tag: &ProcessTag) -> bool {
    let Ok(pid) = libc::pid_t::try_from(tag.pid) else {
        return false;
    };
    let Some(leader) = descendants::process(pid) else {
        return false;
    };
    if format_started(leader.started) != tag.started {
        return false;
    }
    let mut recorded = Vec::new();
    descendants::record(&[leader], &mut recorded);
    // SAFETY: a plain signal to the group of a process whose PID and start time were checked.
    unsafe { libc::killpg(pid, libc::SIGTERM) };
    let deadline = tokio::time::Instant::now() + TERM_GRACE;
    while descendants::process(pid) == Some(leader) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let leader_alive = descendants::process(pid) == Some(leader);
    let mut roots = recorded.clone();
    if leader_alive {
        roots.push(leader);
    }
    descendants::record(&roots, &mut recorded);
    if leader_alive {
        // SAFETY: as above; the leader still has the recorded start time.
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    descendants::kill(&recorded);
    true
}

/// Windows: the Job Object of a run ends with the host that created it
/// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), so nothing is left to end. Other Unix systems record
/// no process.
#[cfg(not(target_os = "macos"))]
pub async fn end_leftover(_tag: &ProcessTag) -> bool {
    false
}
```

`run` 加最后一个参数 `on_spawn: Option<SpawnHook>`；在 `let mut guard = match command.spawn() { … }` 之后加：

```rust
    if let Some(hook) = on_spawn
        && let Some(tag) = guard.child.id().and_then(process_tag)
    {
        hook(tag);
    }
```

`descendants` 模块（macOS 版）里 `Proc` 的 `started` 字段与 `process`、`record`、`kill` 已是 `pub(super)`，可直接用；非 macOS 的 Unix 版不变。

- [ ] **Step 5: 引擎接上（先跑现有测试确认编译失败，再改）**

`crates/core/src/engine.rs`：

- `JobRunner::run` 加参数 `on_spawn: SpawnHook`（文档：「`on_spawn` gets the executor's process right after it starts, for the record a host started after a crash uses」）；`ProcessJobRunner::run` 把它作为 `Some(on_spawn)` 传给 `runner::run`。
- `Inner` 加字段 `leftovers: Mutex<Vec<ProcessTag>>`。`Engine::new`：

```rust
        let interrupted = deps.store.recover_interrupted()?;
        if !interrupted.is_empty() {
            tracing::warn!(
                recovered = interrupted.len(),
                "runs interrupted by the previous shutdown were marked failed"
            );
        }
        let leftovers = interrupted.into_iter().filter_map(|run| run.process).collect();
```

  并在 `Inner` 初始化里放入 `leftovers: Mutex::new(leftovers)`。
- `run_forever` 开头（设置 `shutdown` 之后、日志清理之前）：

```rust
        // A previous host that died while runs were in progress may have left their executors
        // running: end them before anything new starts.
        let leftovers = std::mem::take(&mut *self.inner.leftovers.lock().unwrap_or_else(PoisonError::into_inner));
        for tag in leftovers {
            if runner::end_leftover(&tag).await {
                tracing::warn!(pid = tag.pid, "ended an executor the previous host left running");
            }
        }
```

- `start_job` 里 `runner.run(spec, kind, log_path, limits, cancel)` 改为传入：

```rust
            let recorder = engine.clone();
            let on_spawn: SpawnHook = Box::new(move |tag| {
                if let Err(e) = recorder.store().set_process(job_id, &tag) {
                    tracing::warn!(job_id, error = %format!("{e:#}"), "could not record the executor process");
                }
            });
```

  （放在 `tokio::spawn(async move { … })` 的外层，`engine` 被 move 之前 clone。）
- `crates/core/tests/common/engine_kit.rs`：`ScriptedRunner::run` 加参数 `on_spawn: SpawnHook` 并在开始时调用 `on_spawn(ProcessTag { pid: 4242, started: "1700000000.000001".into() })`（假进程号只进存储，从不被用来发信号）。

Run: `cargo test -p issuerelay-core`
Expected: PASS。

- [ ] **Step 6: 文档与提交**

- 设计第 6.2 节补：「macOS 上每次运行开始时记下执行者的进程号和启动时间；宿主被强杀后，下次启动先清扫：同一进程号且启动时间吻合才按上面的方式结束整棵树。Windows 的 Job Object 随宿主一起关闭，整棵树自动结束。」第 11 节补：「`state.db` 带表结构版本号；遇到比当前程序新的版本拒绝打开并提示安装新版。」
- M0–M1 计划「本计划不覆盖」6.2「宿主崩溃后的残留进程」一行、「评审遗留」M3 表的 `store` 一行末尾加「（M2 第一部分 Task 4 已完成）」。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates/core docs Cargo.lock
git commit -m "feat(core): record executor processes and end leftovers after a host crash"
```

---

### Task 5: 给面板用的引擎接口

> 执行说明（2026-10-13）：已落地代码以提交 `5ee6b48` 与修复 `ed5d6a4` 为准。与下文不同之处：每轮检查、派活和需求列表各自只取一次 Linear 连接，`viewer` 缓存只回答填它的那个连接（修正下文写法里换连接时的竞态）；`run_forever` 让正在进行的检查与停止信号竞速，第一次停止信号不再等检查结束；`emit_job` 在读出记录的同一把锁内发送；启动失败的退避只在任务仍排队且期间没被重试时写入；`projects()` 清洗项目名；`update_settings` 保存后立即唤醒循环。Step 6 要求的遗留表标注改为如实写法（「等待完成」是不保存的进程内开关；`MissingArguments` 由 Task 9 与第二部分处理）；设计 9.2 改为「最近更新的 50 条」。

面板（M2 第二部分）在进程内直接调用引擎。它需要：事件（任务变化、检查结果、配置保存）、「需求」页的视图（设计 9.2：当前用户所有带 `ai-ready` 的需求、本地状态与不满足条件的原因）、设置读写、换 Linear 连接（登录、登出、重新授权，评审遗留「重新授权后要重建引擎」）、退出时的「等待完成」（评审遗留「能持久的暂停」）、没有连接时也能运行的占位实现。顺带修评审遗留：`count()` 吞错误、队首任务启动失败饿死其他任务、`waiting_binding` 的日志措辞。

**Files:**
- Modify: `crates/core/src/linear/mod.rs`（`LinearApi::labeled_issues`、`Disconnected`）
- Modify: `crates/core/src/linear/mcp.rs`（实现 `labeled_issues`）
- Modify: `crates/core/src/engine.rs`
- Modify: `crates/core/tests/common/fake_linear.rs`（`labeled_issues`、`fail_issue_reads`、`clear_issue_failures`）
- Modify: `crates/core/tests/common/fake_server.rs`（`list_issues` 不带 `state` 时返回所有状态，若尚不支持）
- Test: `crates/core/tests/engine.rs`、`crates/core/tests/linear_mcp.rs`

**Interfaces:**
- Produces:
  - `LinearApi::labeled_issues(&self, label: &str) -> Result<Vec<Issue>>`：指派给当前用户、带 `label`、任意状态，按 `updatedAt` 倒序的一页（最多 50 条）
  - `pub struct linear::Disconnected`：`Disconnected::new(reason: impl Into<String>) -> Self`，每个方法都返回 `Err(reason)`
  - `pub enum engine::EngineEvent { Status, Job { job: Job }, Config }`（serde `tag = "kind"`, `rename_all = "snake_case"`；`Debug, Clone, PartialEq, Serialize, Deserialize`）
  - `pub struct engine::IssueView { pub issue: Issue, pub job: Option<Job>, pub status: IssueStatus }`
  - `pub enum engine::IssueStatus { Eligible, NotTodo { state: String }, NotCandidate, Waiting { reason: String }, Paused }`（serde `tag = "kind"`, `rename_all = "snake_case"`）
  - `StatusSnapshot.draining: bool`（`#[serde(default)]`）
  - `Engine::subscribe(&self) -> tokio::sync::broadcast::Receiver<EngineEvent>`
  - `Engine::issues(&self) -> Result<Vec<IssueView>>`（async）
  - `Engine::settings(&self) -> Settings`、`Engine::update_settings(&self, settings: Settings) -> Result<()>`（保留当前的 `paused`）
  - `Engine::job(&self, id: i64) -> Result<Option<Job>>`
  - `Engine::projects(&self) -> Result<Vec<Project>>`（async）
  - `Engine::set_linear(&self, linear: Arc<dyn LinearApi>)`
  - `Engine::set_draining(&self, on: bool)`
- Consumes: Task 3 的 `clean_logs`、Task 4 的 `JobRunner::run(…, on_spawn)`

- [ ] **Step 1: 假实现补齐**

`crates/core/tests/common/fake_linear.rs`：`State` 加 `failing_issue_reads: HashSet<String>`；

```rust
    /// Makes every `issue(uuid)` read fail until `clear_issue_failures`.
    pub fn fail_issue_reads(&self, uuid: &str) {
        self.state.lock().unwrap().failing_issue_reads.insert(uuid.into());
    }

    pub fn clear_issue_failures(&self) {
        self.state.lock().unwrap().failing_issue_reads.clear();
    }
```

`issue()` 在查表之前：`if s.failing_issue_reads.contains(id) { return Err(anyhow!("Linear is unreachable")); }`（`id` 可能是 uuid 或编号，按现有查找方式比较 uuid）。`LinearApi` 实现加：

```rust
    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>> {
        self.take_failure()?;
        let s = self.state.lock().unwrap();
        let mut issues: Vec<Issue> = s
            .issues
            .values()
            .filter(|i| i.labels.iter().any(|l| same_name(l, label)) && i.assignee_id.as_deref() == Some(s.me.as_str()))
            .cloned()
            .collect();
        issues.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        issues.truncate(50);
        Ok(issues)
    }
```

（`take_failure`、`issues`、`me` 换成文件里实际的名字。）

- [ ] **Step 2: 写失败测试**

`crates/core/tests/linear_mcp.rs` 加（沿用文件里启动 `FakeLinearServer` 的写法）：

```rust
#[tokio::test]
async fn labeled_issues_reads_one_page_of_every_status() {
    let (server, linear) = connect_fake().await;
    server.add_issue(issue_json("1", "Todo"));
    server.add_issue(issue_json("2", "In Testing"));
    let issues = linear.labeled_issues("ai-ready").await.unwrap();
    assert_eq!(issues.len(), 2);
    let call = server.calls_to("list_issues").pop().unwrap();
    assert_eq!(call["orderBy"], "updatedAt");
    assert_eq!(call["limit"], 50);
    assert!(call.get("state").is_none(), "every status");
    assert!(call.get("cursor").is_none(), "one page only");
}
```

（`connect_fake`、`add_issue`、`issue_json`、`calls_to` 以文件里实际的辅助为准；没有就按现有测试的写法补。）

`crates/core/tests/engine.rs` 加（`use issuerelay_core::engine::{EngineEvent, IssueStatus};`、`use issuerelay_core::linear::Disconnected;`）：

```rust
#[tokio::test]
async fn subscribers_hear_about_every_job_change_and_each_check() {
    let h = Harness::new(1, false);
    let mut events = h.engine.subscribe();
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    h.engine.mark_done(h.latest("1").id).await.unwrap();
    let (mut states, mut saw_status, mut saw_config) = (Vec::new(), false, false);
    while let Ok(event) = events.try_recv() {
        match event {
            EngineEvent::Job { job } => states.push(job.state),
            EngineEvent::Status => saw_status = true,
            EngineEvent::Config => saw_config = true,
        }
    }
    assert!(saw_config, "binding the project saved the configuration");
    assert!(saw_status, "the check reported its result");
    assert_eq!(
        states,
        [JobState::Queued, JobState::Running, JobState::AwaitingAcceptance, JobState::Done]
    );
}

#[tokio::test]
async fn retry_and_stop_are_announced_too() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.runner.script("DEMO-1", Script::exit_code(3));
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let mut events = h.engine.subscribe();
    let id = h.latest("1").id;
    h.engine.retry_job(id).unwrap();
    h.engine.cancel_job(id).unwrap();
    let states: Vec<JobState> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|e| match e { EngineEvent::Job { job } => Some(job.state), _ => None })
        .collect();
    assert_eq!(states, [JobState::Queued, JobState::Canceled]);
}

#[tokio::test]
async fn the_issue_list_explains_what_each_labeled_issue_is_waiting_for() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(Issue { state: "In Testing".into(), ..issue("2", "proj-1") });
    h.linear.upsert(issue("3", "proj-2"));
    let views = h.engine.issues().await.unwrap();
    let status_of = |views: &[issuerelay_core::engine::IssueView], uuid: &str| {
        views.iter().find(|v| v.issue.uuid == uuid).unwrap().status.clone()
    };
    assert_eq!(status_of(&views, "1"), IssueStatus::Eligible);
    assert_eq!(status_of(&views, "2"), IssueStatus::NotTodo { state: "In Testing".into() });
    assert!(matches!(status_of(&views, "3"), IssueStatus::Waiting { .. }));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let views = h.engine.issues().await.unwrap();
    let first = views.iter().find(|v| v.issue.uuid == "1").unwrap();
    assert_eq!(first.job.as_ref().unwrap().state, JobState::AwaitingAcceptance);
    h.engine.pause().unwrap();
    let views = h.engine.issues().await.unwrap();
    assert_eq!(status_of(&views, "3"), IssueStatus::Paused);
}

#[tokio::test]
async fn saving_settings_validates_them_and_keeps_the_pause_switch() {
    let h = Harness::new(1, false);
    h.engine.pause().unwrap();
    let mut settings = h.engine.settings();
    settings.poll_interval_secs = 120;
    settings.paused = false;
    h.engine.update_settings(settings).unwrap();
    let saved = h.engine.settings();
    assert_eq!(saved.poll_interval_secs, 120);
    assert!(saved.paused, "the settings page never switches monitoring on or off");
    let mut bad = h.engine.settings();
    bad.max_concurrent = 0;
    assert!(h.engine.update_settings(bad).is_err());
    assert_eq!(h.engine.settings().max_concurrent, 1, "a refused change changes nothing");
}

#[tokio::test]
async fn reconnecting_linear_moves_every_later_call_to_the_new_connection() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "repo-a");
    // The run leaves the old connection's copy of the issue alone.
    h.runner.script("DEMO-1", Script { set_state: None, ..Script::success() });
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    let other = FakeLinear::new("user-1");
    other.upsert(Issue { state: "In Testing".into(), ..issue("1", "proj-1") });
    h.engine.set_linear(Arc::new(other.clone()));
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(
        h.latest("1").state,
        JobState::AwaitingAcceptance,
        "the result was read through the new connection"
    );
    assert_eq!(h.runner.started().len(), 1, "switching connections starts nothing twice");
}

#[tokio::test]
async fn without_a_linear_connection_checks_fail_visibly_until_one_is_set() {
    let h = Harness::new(1, false);
    h.engine.set_linear(Arc::new(Disconnected::new("not logged in to Linear")));
    assert!(h.engine.poll_once().await.is_err());
    let status = h.engine.status();
    assert!(!status.linear_ok);
    assert!(status.last_error.unwrap().contains("not logged in to Linear"));
    h.engine.set_linear(Arc::new(h.linear.clone()));
    h.engine.poll_once().await.unwrap();
    let status = h.engine.status();
    assert!(status.linear_ok);
    assert_eq!(status.viewer.unwrap().id, "user-1");
}

#[tokio::test]
async fn draining_lets_runs_finish_but_starts_nothing_new() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "repo-a");
    h.bind("proj-2", "repo-b");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    h.engine.set_draining(true);
    assert!(h.engine.status().draining);
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.runner.started(), ["DEMO-1"], "nothing new starts while draining");
    assert_eq!(h.latest("2").state, JobState::Queued, "new issues are still recorded");
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    h.engine.set_draining(false);
    h.engine.tick().await.unwrap();
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn a_job_that_cannot_start_does_not_hold_up_the_queue() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.bind("proj-2", "repo-b");
    let earlier = issue("1", "proj-1").created_at - chrono::Duration::hours(1);
    h.linear.upsert(Issue { created_at: earlier, ..issue("1", "proj-1") });
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.poll_once().await.unwrap();
    h.linear.fail_issue_reads("1");
    h.engine.dispatch().await.unwrap();
    assert!(h.runner.started().is_empty());
    assert!(h.engine.status().last_error.unwrap().contains("DEMO-1"));
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-2"], "the failing job waits while the next one runs");
    assert_eq!(h.latest("1").state, JobState::Queued, "and stays queued for a later try");
}
```

Run: `cargo test -p issuerelay-core --test engine` 与 `cargo test -p issuerelay-core --test linear_mcp labeled_issues`
Expected: 编译失败。

- [ ] **Step 3: `linear` 部分**

`crates/core/src/linear/mod.rs` 的 `LinearApi` 在 `candidate_issues` 之后加：

```rust
    /// The 50 most recently updated issues assigned to the authorized user with `label`, in any
    /// status: the panel's issue list. Fails if any issue cannot be read.
    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>>;
```

同文件末尾：

```rust
/// Stands in for Linear while there is no usable connection (not logged in yet, or connecting
/// failed): every call fails with `reason`. The host keeps running and the engine's checks fail
/// visibly, instead of the host refusing to start.
pub struct Disconnected {
    reason: String,
}

impl Disconnected {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }

    fn fail<T>(&self) -> Result<T> {
        Err(anyhow::anyhow!("{}", self.reason))
    }
}

#[async_trait::async_trait]
impl LinearApi for Disconnected {
    async fn viewer(&self) -> Result<Viewer> {
        self.fail()
    }
    async fn candidate_issues(&self, _label: &str, _state: &str) -> Result<Vec<Issue>> {
        self.fail()
    }
    async fn labeled_issues(&self, _label: &str) -> Result<Vec<Issue>> {
        self.fail()
    }
    async fn issue(&self, _id: &str) -> Result<Option<Issue>> {
        self.fail()
    }
    async fn set_state(&self, _id: &str, _state: &str) -> Result<()> {
        self.fail()
    }
    async fn projects(&self) -> Result<Vec<Project>> {
        self.fail()
    }
    async fn labels(&self) -> Result<Vec<String>> {
        self.fail()
    }
    async fn teams(&self) -> Result<Vec<Team>> {
        self.fail()
    }
    async fn statuses(&self, _team_id: &str) -> Result<Vec<String>> {
        self.fail()
    }
}
```

`crates/core/src/linear/mcp.rs` 的 `impl LinearApi for McpLinear` 在 `candidate_issues` 之后加：

```rust
    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>> {
        // One page, the most recently updated first: the panel lists recent work, not every
        // issue that ever carried the label.
        let args = json!({"assignee": "me", "label": label, "limit": 50, "orderBy": "updatedAt", "fields": ISSUE_FIELDS});
        Ok(parse::parse_issue_page(&self.call("list_issues", args).await?)?.items)
    }
```

（参数都已在 `REQUIRED_ARGUMENTS["list_issues"]` 里，表不用改。）

- [ ] **Step 4: 引擎部分**

`crates/core/src/engine.rs`，逐项改：

1. 新类型（放在 `StatusSnapshot` 之后）：

```rust
/// What changed in the engine, for the desktop panel. A subscriber that falls behind gets
/// `RecvError::Lagged` and should read the whole state again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    /// The status line may have changed: a check finished, monitoring was paused or resumed,
    /// draining was switched, or Linear was reconnected.
    Status,
    /// A job was recorded or changed; `job` is the row as stored now.
    Job { job: Job },
    /// The settings or the bindings were saved.
    Config,
}

/// One issue of the panel's issue list: an issue assigned to the user with the trigger label,
/// its newest local run, and whether it would be picked up now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueView {
    /// The issue, with its title cleaned for display.
    pub issue: Issue,
    pub job: Option<Job>,
    pub status: IssueStatus,
}

/// Whether an issue of the issue list would be picked up now, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueStatus {
    /// It meets every condition. `IssueView::job` says what happened with it so far.
    Eligible,
    /// Its status is not the trigger's Todo status.
    NotTodo { state: String },
    /// It does not match the trigger's label or assignee as IssueRelay compares them.
    NotCandidate,
    /// It waits for a binding, a folder or a ready executor.
    Waiting { reason: String },
    /// Monitoring is paused.
    Paused,
}

/// How many events wait for a slow subscriber before it lags.
const EVENT_CAPACITY: usize = 256;

/// How long a job whose start failed is passed over: doubled after each further failure, up to
/// `START_RETRY_MAX`. Meanwhile the jobs behind it can start.
const START_RETRY_BASE: Duration = Duration::from_secs(30);
const START_RETRY_MAX: Duration = Duration::from_secs(5 * 60);

struct StartBackoff {
    failures: u32,
    until: tokio::time::Instant,
}
```

`StatusSnapshot` 加：

```rust
    /// Nothing new starts while the runs in progress finish (the "wait for completion" exit).
    #[serde(default)]
    pub draining: bool,
```

2. `Inner`：`linear` 改为 `RwLock<Arc<dyn LinearApi>>`；加 `events: tokio::sync::broadcast::Sender<EngineEvent>`、`draining: std::sync::atomic::AtomicBool`、`start_backoff: Mutex<HashMap<i64, StartBackoff>>`。`Engine::new` 相应初始化（`broadcast::channel(EVENT_CAPACITY).0`、`AtomicBool::new(false)`、`Mutex::new(HashMap::new())`）。

3. 辅助方法：

```rust
    fn linear(&self) -> Arc<dyn LinearApi> {
        self.inner.linear.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn emit(&self, event: EngineEvent) {
        // No subscriber is fine: the CLI-only host has none.
        let _ = self.inner.events.send(event);
    }

    /// Announces the job as it is stored now. Takes the store lock, so the caller must not hold
    /// it; a poisoned lock is used as it is, because `abandon_run` calls this while unwinding.
    fn emit_job(&self, job_id: i64) {
        let job = self
            .inner
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(job_id);
        if let Ok(Some(job)) = job {
            self.emit(EngineEvent::Job { job });
        }
    }

    fn draining(&self) -> bool {
        self.inner.draining.load(std::sync::atomic::Ordering::SeqCst)
    }
```

把所有 `self.inner.linear.` 改成 `self.linear().`（`viewer`、`poll_inner`、`start_job`、`verify_read`、`mark_done`、`doctor`）。

4. 新的公开方法：

```rust
    /// Tells subscribers about every later change; see `EngineEvent`.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// The current settings.
    pub fn settings(&self) -> Settings {
        self.config().settings
    }

    /// Validates and saves `settings`. `paused` keeps its current value: monitoring is switched
    /// with `pause` and `resume`, never by saving the settings page.
    pub fn update_settings(&self, settings: Settings) -> Result<()> {
        self.update_config(|c| {
            let paused = c.settings.paused;
            c.settings = settings;
            c.settings.paused = paused;
        })
    }

    /// One job by id.
    pub fn job(&self, id: i64) -> Result<Option<Job>> {
        self.store().get(id)
    }

    /// The Linear projects, for choosing what to bind.
    pub async fn projects(&self) -> Result<Vec<Project>> {
        self.linear().projects().await
    }

    /// Uses `linear` for every later call: after a login, a logout or a new API key. The cached
    /// user is forgotten and a check runs at once. Runs in progress continue; their results are
    /// read through the new connection.
    pub fn set_linear(&self, linear: Arc<dyn LinearApi>) {
        *self.inner.linear.write().unwrap_or_else(PoisonError::into_inner) = linear;
        *self.inner.viewer.lock().expect("viewer lock") = None;
        self.emit(EngineEvent::Status);
        self.poll_now();
    }

    /// While on, nothing new starts but the runs in progress go on; polls still record new
    /// issues. Not saved: it serves the "wait for completion" exit and ends with the host.
    pub fn set_draining(&self, on: bool) {
        self.inner.draining.store(on, std::sync::atomic::Ordering::SeqCst);
        self.emit(EngineEvent::Status);
        if !on {
            self.poll_now();
        }
    }

    /// The panel's issue list (spec 9.2): the issues assigned to the user with the trigger label,
    /// each with its newest local run and whether it would be picked up now.
    pub async fn issues(&self) -> Result<Vec<IssueView>> {
        let cfg = self.config();
        let names = &cfg.settings.trigger;
        let me = self.viewer().await?;
        let issues = self.linear().labeled_issues(&names.label).await?;
        let (ready, _) = self.readiness().await;
        let store = self.store();
        let mut views = Vec::with_capacity(issues.len());
        for issue in issues {
            let job = store.latest_for_issue(&issue.uuid)?;
            let status = if !same_name(&issue.state, &names.todo) {
                IssueStatus::NotTodo { state: clean_text(&issue.state) }
            } else if cfg.settings.paused {
                IssueStatus::Paused
            } else {
                match trigger::evaluate(&issue, &cfg, &me.id, &ready) {
                    Eligibility::Eligible(_) => IssueStatus::Eligible,
                    Eligibility::Waiting(reason) => IssueStatus::Waiting { reason: reason.describe() },
                    Eligibility::NotCandidate => IssueStatus::NotCandidate,
                }
            };
            let issue = Issue { title: clean_text(&issue.title), ..issue };
            views.push(IssueView { issue, job, status });
        }
        Ok(views)
    }
```

（`use crate::config::Settings; use crate::linear::Project;`；`trigger::evaluate` 的参数顺序以 `poll_inner` 里的调用为准。）

5. `update_config` 成功后（`*current = next;` 之后、`Ok(())` 之前，先 `drop(known); drop(current);`）调用 `self.emit(EngineEvent::Config);`。

6. `status()`：

```rust
    fn count(&self, state: JobState) -> Result<usize> {
        Ok(self.store().list_by_state(&[state])?.len())
    }

    /// The state the panel, CLI and MCP show. The job counts come from the store; if it cannot be
    /// read the counts show 0 and `last_error` says why.
    pub fn status(&self) -> StatusSnapshot {
        let paused = self.config().settings.paused;
        let poll = self.inner.poll.lock().expect("poll lock").clone();
        let viewer = self.inner.viewer.lock().expect("viewer lock").clone();
        let running = self.inner.running.lock().expect("running lock").len();
        let counts = self
            .count(JobState::Queued)
            .and_then(|queued| Ok((queued, self.count(JobState::WaitingBinding)?)));
        let (queued, waiting, store_error) = match counts {
            Ok((queued, waiting)) => (queued, waiting, None),
            Err(e) => (0, 0, Some(clean_text(&format!("cannot read the job records: {e:#}")))),
        };
        StatusSnapshot {
            paused,
            linear_ok: poll.linear_ok,
            viewer,
            last_poll_at: poll.at,
            last_error: store_error.or(poll.error),
            running,
            queued,
            waiting,
            draining: self.draining(),
        }
    }
```

7. `poll_once`：写入 `PollState` 之后加 `self.emit(EngineEvent::Status);`。`poll_inner`：`enqueue` 改为返回 `Result<Option<i64>>`（插入的任务 id；`NotCandidate` 时 `None`），`reassess` 改为返回 `Result<bool>`（是否写了存储）；循环里把有变化的任务 id 收进 `let mut changed = Vec::new();`；取消不再匹配的任务时 `set_state_if` 返回 `true` 也收进去；函数末尾（所有存储锁都已释放）`for id in changed { self.emit_job(id); }`。`enqueue` 的日志按状态区分：

```rust
        let message = if job.state == JobState::Queued { "job queued" } else { "job waiting" };
        tracing::info!(job_id = job.id, issue = %job.issue_identifier, state = job.state.as_str(), "{message}");
```

（`reassess` 里「从等待进入排队」那条日志本来就只在进入 `Queued` 时写，保留。）

8. `dispatch`：开头与循环里的条件 `cfg.settings.paused || self.stopping()` 都改为 `cfg.settings.paused || self.stopping() || self.draining()`；构造 `candidates` 前读出退避表并清掉不再排队的项：

```rust
        let now = tokio::time::Instant::now();
        let held_back: HashSet<i64> = {
            let mut backoff = self.inner.start_backoff.lock().expect("backoff lock");
            let queued_ids: HashSet<i64> = queued.iter().map(|job| job.id).collect();
            backoff.retain(|id, _| queued_ids.contains(id));
            backoff.iter().filter(|(_, b)| b.until > now).map(|(id, _)| *id).collect()
        };
```

`candidates` 的 `filter_map` 第一行加 `if held_back.contains(&job.id) { return None; }`。处理 `started_job` 的 `match` 里：`Ok(true)` 与 `Ok(false)` 都 `self.inner.start_backoff.lock().expect("backoff lock").remove(&id);`；`Err` 分支记一次失败：

```rust
                    let mut backoff = self.inner.start_backoff.lock().expect("backoff lock");
                    let entry = backoff.entry(id).or_insert(StartBackoff { failures: 0, until: now });
                    entry.failures += 1;
                    let pause = START_RETRY_BASE
                        .saturating_mul(1u32 << (entry.failures - 1).min(10))
                        .min(START_RETRY_MAX);
                    entry.until = tokio::time::Instant::now() + pause;
```

`start_job` 里登记运行那段的条件 `self.stopping() || …` 加上 `|| self.draining()`；`mark_started` 成功、写完 `job started` 日志之后加 `self.emit_job(job.id);`；`assign_if_queued` 返回 `true` 的三处与「issue changed before the run started」那处 `set_state_if` 返回 `true` 时，都 `self.emit_job(job.id)`（在存储锁之外）。

9. 其余写入点：`finish_job` 在 `recorded` 处理完（含失败时的 `set_state(Failed)`）之后、`slot.release()` 之前 `self.emit_job(job_id);`；`abandon_run` 在释放存储锁的块之后 `self.emit_job(job_id);`；`cancel_job` 里 `set_state_if` 返回 `true` 时 `self.emit_job(job_id);`；`retry_job` 在释放存储锁的块之后 `self.emit_job(job_id);`；`mark_done` 在 `set_state_if` 返回 `true` 之后 `self.emit_job(job_id);`。

Run: Step 2 的两条命令，再跑 `cargo test -p issuerelay-core`
Expected: PASS。现有测试（`tests/engine.rs` 里构造 `StatusSnapshot` 的断言等）若因新字段失败，补上 `draining: false`。

- [ ] **Step 5: IPC 与命令行跟上**

`StatusSnapshot` 多了 `draining`：`crates/cli/src/format.rs` 的状态输出在暂停一行旁加「Draining: finishing runs, starting nothing new」（只在 `draining` 为真时显示），并加测试：

```rust
    #[test]
    fn status_says_when_the_host_is_draining() {
        let status = StatusSnapshot { draining: true, ..StatusSnapshot::default() };
        assert!(format_status(&status).contains("Draining"));
        assert!(!format_status(&StatusSnapshot::default()).contains("Draining"));
    }
```

（函数名以文件里实际的为准。）IPC 方法表不变：`EngineEvent`、`issues`、`update_settings`、`set_linear`、`set_draining`、`mark_done`、`retry_job`、`cancel_job` 都不进 `METHODS`，`ipc::protocol` 的绊线测试保持原样。

Run: `cargo test --workspace`
Expected: PASS。

- [ ] **Step 6: 文档与提交**

- 设计第 4.1 节「托盘程序」一条补：「面板在进程内调用引擎，并订阅引擎事件（任务变化、检查结果、配置保存）刷新界面；『标记完成』『重新运行』『停止』只走这条进程内通道。」第 13 节「Linear 连接失败或令牌失效」一行补：「没有凭据或连接失败时，宿主照常启动，用占位连接让每次检查都报出原因；重新登录后换上新连接，正在运行的任务用新连接核对结果。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 4 节数据流补一句面板经事件订阅刷新。
- M0–M1 计划「评审遗留的小项」M2 引擎表里 `viewer` 重建、`count()`、「等待完成」、队首饿死、`job queued` 措辞、`MissingArguments`（只做了「接口已变化」的展示数据，面板文案在第二部分）各行末尾加「（M2 第一部分 Task 5）」。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates docs
git commit -m "feat(core): add engine events, the issue list and live Linear reconnects for the panel"
```

---

### Task 6: Linear 修补与按团队检查工作区

M1 验收 H2（中文标题的链接被丢掉）与评审遗留：翻页对重复游标默默停下、连接时只查参数名不查 `fields` 枚举、回环端口在换令牌期间仍被占着、不可见字符漏网（标签字符、U+061C、软连字符、韩文填充字符；而去掉 ZWJ / ZWNJ 会弄坏 emoji 和印度系文字）、工作区检查覆盖所有团队。

**Files:**
- Modify: `crates/core/src/linear/parse.rs`（`safe_url` 保留到编号为止、`Project.team_ids`）
- Modify: `crates/core/src/linear/mod.rs`（`Project.team_ids`、`PROJECT_FIELDS`、`REQUIRED_FIELDS`）
- Modify: `crates/core/src/linear/mcp.rs`（`follow` 严格化、`fields` 枚举检查、`projects` 请求 `teams`）
- Modify: `crates/core/src/linear/auth.rs`（拿到回调后释放监听；`oauth_error` 用 `text::is_invisible`）
- Modify: `crates/core/src/text.rs`（补字符、保留 ZWJ / ZWNJ）
- Modify: `crates/core/src/doctor.rs`（`check_workspace` 只查已绑定项目所属的团队，报告列出查了哪些团队）
- Modify: `crates/core/src/engine.rs`（`doctor` 传入绑定）
- Modify: `crates/core/tests/common/fake_linear.rs`（项目、团队与按团队的状态）、`tests/common/fake_server.rs`（若要返回 `teams`）
- Modify: `crates/cli/src/main.rs`、`crates/devhost/src/main.rs`（构造 `Project` 的地方，如有）
- Test: `parse.rs`、`mcp.rs`、`text.rs` 的测试模块，`crates/core/tests/doctor.rs`、`crates/core/tests/linear_mcp.rs`

**Interfaces:**
- Produces:
  - `Project { id: String, name: String, team_ids: Vec<String> }`（`team_ids` 带 `#[serde(default)]`）
  - `pub const linear::PROJECT_FIELDS: [&str; 4] = ["id", "uuid", "name", "teams"]`
  - `pub const linear::REQUIRED_FIELDS: &[(&str, &[&str])]`：`list_issues` 与 `get_issue` 要 `ISSUE_FIELDS`，`list_projects` 要 `PROJECT_FIELDS`
  - `pub async fn doctor::check_workspace(api: &dyn LinearApi, names: &TriggerNames, bindings: &[Binding]) -> Result<WorkspaceReport>`
  - `WorkspaceReport.teams_checked: Vec<String>`（团队名，`#[serde(default)]`）
- Consumes: Task 5 的 `Engine::linear()`

- [ ] **Step 1: 写失败测试**

`crates/core/src/linear/parse.rs` 测试模块加：

```rust
    #[test]
    fn a_link_with_a_non_ascii_slug_keeps_everything_up_to_the_identifier() {
        let mut v = sample_issue_json();
        v["identifier"] = "DEMO-3".into();
        v["url"] = "https://linear.app/demo/issue/DEMO-3/新增-hello".into();
        assert_eq!(parse_issue(&v).unwrap().url, "https://linear.app/demo/issue/DEMO-3");
        v["url"] = "https://linear.app/demo/issue/DEMO-3/add-hello".into();
        assert_eq!(parse_issue(&v).unwrap().url, "https://linear.app/demo/issue/DEMO-3/add-hello");
        v["url"] = "https://linear.app/demo/项目/DEMO-3".into();
        assert_eq!(parse_issue(&v).unwrap().url, "", "without /issue/<identifier> nothing is kept");
        v["url"] = "https://linear.app/demo/issue/DEMO-30/x".into();
        assert_eq!(parse_issue(&v).unwrap().url, "https://linear.app/demo/issue/DEMO-30/x");
    }

    #[test]
    fn projects_carry_their_team_ids() {
        let page = parse_projects(
            r#"{"projects":[{"id":"P-DEMO-1","uuid":"proj-1","name":"Demo Project","teams":[{"id":"team-1","name":"Demo","key":"DEMO"}]},{"id":"P-DEMO-2","uuid":"proj-2","name":"Other","teams":null}],"hasNextPage":false}"#,
        )
        .unwrap();
        assert_eq!(page.items[0].team_ids, ["team-1"]);
        assert!(page.items[1].team_ids.is_empty(), "no teams reads as none");
        let bad = parse_projects(
            r#"{"projects":[{"id":"P-DEMO-1","uuid":"proj-1","name":"Demo","teams":[{"name":"no id"}]}],"hasNextPage":false}"#,
        );
        assert!(bad.is_err(), "a team without an id is an error, like any unreadable entry");
    }
```

（`sample_issue_json` 换成文件里现有的样例构造；没有就照 `parse_issue` 的必填字段写一个。）

`crates/core/src/linear/mcp.rs` 测试模块：把 `paging_stops_on_empty_pages_and_repeated_cursors` 换成：

```rust
    #[test]
    fn paging_follows_every_new_cursor_and_rejects_a_repeated_one() {
        assert_eq!(follow(Some("c2".into()), None).unwrap().as_deref(), Some("c2"));
        assert_eq!(follow(None, Some("c1")).unwrap(), None);
        assert_eq!(
            follow(Some("c3".into()), Some("c2")).unwrap().as_deref(),
            Some("c3"),
            "an empty page with a new cursor is followed: later pages may still hold issues"
        );
        let error = follow(Some("c2".into()), Some("c2")).unwrap_err();
        assert!(error.to_string().contains("repeated"), "{error}");
    }

    #[test]
    fn a_missing_fields_value_fails_the_connect_check() {
        let mut tools = m0_tools_json();
        let projects = tools.iter_mut().find(|t| t["name"] == "list_projects").unwrap();
        let fields = projects["inputSchema"]["properties"]["fields"]["items"]["enum"]
            .as_array_mut()
            .unwrap();
        fields.retain(|f| f != "teams");
        let missing = missing_arguments(&read_as_tools(tools));
        assert_eq!(missing, ["list_projects.fields: teams"]);
        assert!(missing_arguments(&m0_tools()).is_empty(), "the real M0 list has every field");
    }
```

`crates/core/src/text.rs` 测试模块：`invisible_and_bidirectional_characters_are_removed` 的字符表去掉 `'\u{200c}'`、`'\u{200d}'`，加 `'\u{00ad}'`、`'\u{061c}'`、`'\u{115f}'`、`'\u{1160}'`、`'\u{3164}'`、`'\u{ffa0}'`、`'\u{e0001}'`、`'\u{e0041}'`、`'\u{e007f}'`；再加：

```rust
    #[test]
    fn joiners_stay_so_emoji_and_indic_text_keep_their_shape() {
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(clean_text(family), family);
        let hindi = "क्\u{200d}ष";
        assert_eq!(clean_text(hindi), hindi);
        assert!(!is_invisible('\u{200c}') && !is_invisible('\u{200d}'));
    }
```

`crates/core/tests/doctor.rs` 加（扩展后的 `FakeLinear` 接口见 Step 2）：

```rust
#[tokio::test]
async fn only_the_teams_of_bound_projects_are_checked() {
    let linear = FakeLinear::new("user-1");
    linear.set_teams(&[("team-1", "Demo"), ("team-2", "Unrelated")]);
    linear.set_projects(&[("proj-1", "Demo Project", &["team-1"])]);
    linear.set_team_statuses("team-1", &["Todo", "In Progress", "In Testing", "Done"]);
    linear.set_team_statuses("team-2", &["Backlog"]);
    let binding = Binding {
        id: "proj-1".into(),
        project_id: "proj-1".into(),
        project_name: "Demo Project".into(),
        directory: std::env::temp_dir(),
        executor: None,
        permission: None,
        model: None,
        prompt_override: None,
    };
    let report = check_workspace(&linear, &TriggerNames::default(), &[binding]).await.unwrap();
    assert!(report.missing_states.is_empty(), "{:?}", report.missing_states);
    assert_eq!(report.teams_checked, ["Demo"]);
    let all = check_workspace(&linear, &TriggerNames::default(), &[]).await.unwrap();
    assert_eq!(all.teams_checked, ["Demo", "Unrelated"], "before any binding every team is checked");
    assert!(all.missing_states.iter().any(|m| m.starts_with("Unrelated: ")));
}
```

文件里其余 `check_workspace(&linear, &names)` 调用补第三个参数 `&[]`。

Run: `cargo test -p issuerelay-core`
Expected: 编译失败或上述测试失败。

- [ ] **Step 2: 实现**

1. `crates/core/src/text.rs`：

```rust
/// Characters that show nothing yet change how text reads: zero-width spaces, direction marks,
/// line and paragraph separators, bidi overrides and isolates, invisible operators, the byte
/// order mark, the soft hyphen, the Arabic letter mark, Hangul fillers and the tag characters.
/// `char::is_control` lets them through. The zero-width joiner and non-joiner (U+200C, U+200D)
/// are kept: emoji sequences and Indic scripts need them, and they cannot reorder text.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{200B}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{E0000}'..='\u{E007F}'
    )
}
```

2. `crates/core/src/linear/auth.rs`：`is_unsafe_to_show` 改为 `c.is_control() || crate::text::is_invisible(c)`（文档注释同步）；`login` 里在 `tokio::time::timeout(…, wait_for_callback(&listener, &expected_state))` 那条语句之后加 `drop(listener);`，注释「The callback has arrived; nothing else should reach the loopback port while the token is exchanged」。

3. `crates/core/src/linear/parse.rs`：`safe_url` 改为接收编号：

```rust
/// An `https://` URL made only of printable ASCII, or nothing. A URL with other characters
/// (Linear adds the title as a slug, which can be Chinese) is cut after `/issue/<identifier>`,
/// which Linear resolves the same way; without that part nothing is kept. The URL reaches the
/// prompt, so it must hold nothing that is not plainly an address.
fn safe_url(text: Option<String>, identifier: &str) -> String {
    let Some(url) = text else {
        return String::new();
    };
    let usable = |candidate: &str| {
        candidate.strip_prefix("https://").is_some_and(|rest| !rest.is_empty())
            && candidate.chars().all(|c| c.is_ascii_graphic())
    };
    if usable(&url) {
        return url;
    }
    let marker = format!("/issue/{identifier}");
    let Some(at) = url.find(&marker) else {
        return String::new();
    };
    let end = at + marker.len();
    // `/issue/DEMO-3` must not match inside `/issue/DEMO-30`.
    if url[end..].chars().next().is_some_and(|c| c != '/' && c != '?' && c != '#') {
        return String::new();
    }
    let kept = &url[..end];
    if usable(kept) { kept.to_string() } else { String::new() }
}
```

`parse_issue` 里改为 `url: safe_url(text(v, "url"), &identifier)`。

`parse_projects` 读取 `teams`：

```rust
pub fn parse_projects(raw: &str) -> Result<Page<Project>> {
    list(raw, "projects", |p| {
        let id = non_empty(p, "uuid").ok_or_else(|| {
            anyhow!("project has no `uuid` (its `id` is the P-<KEY>-N identifier, which issues do not use)")
        })?;
        let team_ids = match p.get("teams") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(teams)) => teams
                .iter()
                .map(|team| non_empty(team, "id").ok_or_else(|| anyhow!("project {id:?} has a team without an `id`")))
                .collect::<Result<Vec<_>>>()?,
            Some(_) => bail!("project {id:?} has `teams` that is not a list"),
        };
        Ok(Project { id, name: text(p, "name").unwrap_or_default(), team_ids })
    })
}
```

4. `crates/core/src/linear/mod.rs`：`Project` 加 `#[serde(default)] pub team_ids: Vec<String>,`（文档：「The ids of the teams the project belongs to; the workspace check looks only at these teams once projects are bound.」）；加

```rust
/// The `fields` requested from `list_projects`.
pub const PROJECT_FIELDS: [&str; 4] = ["id", "uuid", "name", "teams"];

/// Every `fields` value this module requests, per tool. The connect check finds each in the
/// tool's `fields` enum, so a field Linear stopped offering fails at connect time.
pub const REQUIRED_FIELDS: &[(&str, &[&str])] = &[
    ("list_issues", &ISSUE_FIELDS),
    ("get_issue", &ISSUE_FIELDS),
    ("list_projects", &PROJECT_FIELDS),
];
```

5. `crates/core/src/linear/mcp.rs`：`missing_arguments` 在检查参数名之后，对 `REQUIRED_FIELDS` 的每个工具读 `input_schema["properties"]["fields"]["items"]["enum"]`，缺的值记成 `"<tool>.fields: <value>"`（工具不在列表里的情况已由 `REQUIRED_TOOLS` 报出，这里跳过）。`projects()` 的请求改为 `json!({"limit": 50, "fields": PROJECT_FIELDS})`。`follow` 与 `all_pages`：

```rust
/// The cursor to request next, or `None` after the last page (Linear's last page has
/// `hasNextPage: false` and no cursor; M1 acceptance 5b.5). A page may be empty and still have
/// a next one. A cursor that repeats the one just used would loop forever, so it is an error.
fn follow(next: Option<String>, last: Option<&str>) -> Result<Option<String>> {
    match next {
        Some(cursor) if Some(cursor.as_str()) == last => {
            Err(anyhow!("Linear repeated the page cursor {cursor:?}"))
        }
        next => Ok(next),
    }
}
```

`all_pages` 里 `match follow(page.next_cursor, last.as_deref())? { … }`，删掉 `empty` 变量。

6. `crates/core/tests/common/fake_linear.rs`：`State` 加 `teams: Vec<Team>`、`projects: Vec<Project>`、`team_statuses: HashMap<String, Vec<String>>`；加方法 `set_teams(&[(&str, &str)])`、`set_projects(&[(&str, &str, &[&str])])`、`set_team_statuses(team_id, &[&str])`；`teams()`、`projects()` 返回这些（为空时保持原来的默认行为，现有测试才不受影响）；`statuses(team_id)` 先查 `team_statuses`，没有就退回现有的全局 `statuses`。

7. `crates/core/src/doctor.rs`：

```rust
/// What the Linear workspace is missing for IssueRelay's trigger names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReport {
    /// The trigger label exists.
    pub label_ok: bool,
    /// Entries like `Demo: In Testing` (team name and missing status).
    pub missing_states: Vec<String>,
    /// The names of the teams whose statuses were checked.
    #[serde(default)]
    pub teams_checked: Vec<String>,
}

/// Looks for the trigger label and, in the teams of the bound projects, the four statuses
/// IssueRelay moves issues through; before any project is bound, in every team. A team that
/// never uses IssueRelay need not have them. Names compare like everywhere else (`same_name`).
/// Errors when Linear cannot be queried.
pub async fn check_workspace(
    api: &dyn LinearApi,
    names: &TriggerNames,
    bindings: &[Binding],
) -> Result<WorkspaceReport> {
    let label_ok = api.labels().await?.iter().any(|l| same_name(l, &names.label));
    let mut teams = api.teams().await?;
    if !bindings.is_empty() {
        let bound: HashSet<&str> = bindings.iter().map(|b| b.project_id.as_str()).collect();
        let team_ids: HashSet<String> = api
            .projects()
            .await?
            .into_iter()
            .filter(|p| bound.contains(p.id.as_str()))
            .flat_map(|p| p.team_ids)
            .collect();
        teams.retain(|team| team_ids.contains(&team.id));
    }
    let mut missing_states = Vec::new();
    for team in &teams {
        let statuses = api.statuses(&team.id).await?;
        for wanted in [&names.todo, &names.in_progress, &names.in_testing, &names.done] {
            if !statuses.iter().any(|s| same_name(s, wanted)) {
                missing_states.push(format!("{}: {wanted}", team.name));
            }
        }
    }
    Ok(WorkspaceReport {
        label_ok,
        missing_states,
        teams_checked: teams.into_iter().map(|team| team.name).collect(),
    })
}
```

`Engine::doctor` 改为 `doctor::check_workspace(self.linear().as_ref(), &cfg.settings.trigger, &cfg.bindings)`。`crates/cli/src/format.rs` 的工作区一节在有 `teams_checked` 时加一行「Teams checked: Demo, …」（经 `clean`），并在其测试里补断言。

Run: `cargo test --workspace`
Expected: PASS。

- [ ] **Step 3: 文档与提交**

- 设计第 7 节「实测口径」补：「项目的 `teams` 是 `{id, name, key}` 列表；链接末尾的标题 slug 可能是中文，程序只保留到 `/issue/<编号>`。」第 8 节第 3 步改为「检查工作区：有没有 `ai-ready` 标签；已绑定项目所属团队里有没有 In Progress、In Testing、Done（还没有绑定时检查所有团队）」。
- M0–M1 计划「本计划不覆盖」第 7 行（中文 slug）末尾加「（M2 第一部分 Task 6 已完成）」；「评审遗留」里 `linear::mcp`、`text`、`doctor` 工作区、`linear::auth` 回环端口各行末尾加「（M2 第一部分 Task 6）」。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates docs
git commit -m "fix(core): keep Chinese issue links, check fields and only bound teams"
```

---

### Task 7: 配置、命令行与执行者的小修

评审遗留：未知配置键静默丢弃、保存中途崩溃留下的临时文件无人清扫、语言值能混进不可见字符、仓库子目录不被识别为 git 仓库、MCP 输出未清洗且缺工具注解、`bind add` 不规范化路径、`doctor` 等待时没有任何提示。顺带加入面板要用的设置项：四种通知的开关与自动更新开关（设计 9.5、12）。

**Files:**
- Modify: `crates/core/src/config.rs`
- Modify: `crates/core/src/engine.rs`（启动时提醒未知键）
- Modify: `crates/core/src/executor/mod.rs`（`is_git_repo` 向上找）
- Modify: `crates/cli/src/mcp.rs`、`crates/cli/src/main.rs`
- Test: `config.rs`、`executor/mod.rs` 的测试模块，`crates/cli/tests/mcp_tools.rs`、`crates/cli/src/main.rs` 的测试

**Interfaces:**
- Produces:
  - `pub struct config::NotificationSettings { pub run_started: bool, pub awaiting_acceptance: bool, pub needs_info: bool, pub failed: bool }`（`#[serde(default)]`，默认全开）
  - `Settings.notifications: NotificationSettings`、`Settings.auto_update: bool`（默认 `true`）
  - `Settings.onboarding_completed: bool`、`Settings.full_access_acknowledged: bool`（默认 `false`；桌面面板的首次引导是否走完、第一次选「完全放开」时的风险提示是否已确认，设计第 8、14 节）
  - `pub fn config::unknown_keys(text: &str) -> Vec<String>`（形如 `settings.foo`、`bindings[1].bar`）
- Consumes: Task 6 的 `text::is_invisible`

- [ ] **Step 1: 写失败测试**

`crates/core/src/config.rs` 测试模块加：

```rust
    #[test]
    fn notification_and_update_switches_default_on() {
        let settings = Settings::default();
        assert!(settings.notifications.run_started && settings.notifications.awaiting_acceptance);
        assert!(settings.notifications.needs_info && settings.notifications.failed);
        assert!(settings.auto_update);
        assert!(!settings.onboarding_completed, "a new install starts with the guide");
        assert!(!settings.full_access_acknowledged, "the full-access warning has not been shown yet");
        let partial: Config = toml::from_str("[settings.notifications]\nfailed = false\n").unwrap();
        assert!(!partial.settings.notifications.failed);
        assert!(partial.settings.notifications.run_started, "unnamed switches keep their default");
    }

    #[test]
    fn unknown_keys_are_named() {
        let text = "typo = 1\n[settings]\npoll_interval_secs = 30\npol_interval = 5\n[settings.trigger]\nlabel = \"ai-ready\"\ncolour = \"red\"\n[[bindings]]\nid = \"proj-1\"\nproject_id = \"proj-1\"\nproject_name = \"Demo Project\"\ndirectory = \"/tmp/demo\"\nfolder = \"x\"\n";
        assert_eq!(
            unknown_keys(text),
            ["typo", "settings.pol_interval", "settings.trigger.colour", "bindings[0].folder"]
        );
        assert!(unknown_keys(&toml::to_string_pretty(&Config::default()).unwrap()).is_empty());
        assert!(unknown_keys("not toml [").is_empty(), "a file that does not parse fails elsewhere");
    }

    #[test]
    fn language_values_reject_invisible_characters() {
        for bad in ["zh\u{200b}-CN", "en\u{2028}", "\u{202e}ja"] {
            let mut settings = Settings::default();
            settings.language = bad.into();
            assert!(settings.validate().is_err(), "{bad:?}");
            let mut settings = Settings::default();
            settings.comment_language = bad.into();
            assert!(settings.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_save_sweeps_temp_files_left_by_a_crashed_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let stale = dir.path().join("config.toml.99999.0.tmp");
        let fresh = dir.path().join("config.toml.99999.1.tmp");
        let unrelated = dir.path().join("notes.tmp");
        for file in [&stale, &fresh, &unrelated] {
            std::fs::write(file, "x").unwrap();
        }
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        Config::default().save(&path).unwrap();
        assert!(!stale.exists(), "an hour-old temp file is a leftover");
        assert!(fresh.exists(), "a recent one may belong to a save in progress");
        assert!(unrelated.exists());
    }
```

`crates/core/src/executor/mod.rs` 测试模块加：

```rust
    #[test]
    fn a_folder_inside_a_repository_counts_as_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let nested = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(is_git_repo(&nested));
        let outside = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(outside.path()));
    }
```

`crates/cli/tests/mcp_tools.rs` 加（沿用文件里列出工具的写法）：

```rust
#[tokio::test]
async fn control_tools_say_they_change_state_harmlessly_and_repeatably() {
    let tools = list_tools().await;
    for name in ["pause", "resume", "poll_now"] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let annotations = tool.annotations.as_ref().expect("annotated");
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
        assert_eq!(annotations.destructive_hint, Some(false), "{name}");
        assert_eq!(annotations.idempotent_hint, Some(true), "{name}");
    }
}

#[tokio::test]
async fn tool_output_carries_no_invisible_characters() {
    let reply = call_with_host_reply("status", serde_json::json!({"last_error": "boom\u{202e}gnp.exe"})).await;
    assert!(!reply.contains('\u{202e}'), "{reply:?}");
}
```

（`list_tools`、`call_with_host_reply` 换成文件里实际的辅助：前者列出 MCP 工具，后者让录制型假传输返回给定 JSON。）

`crates/cli/src/main.rs` 测试模块加：

```rust
    #[test]
    fn bound_folders_are_stored_without_dot_segments() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a");
        std::fs::create_dir(&nested).unwrap();
        let messy = nested.join("..").join("a").join(".");
        assert_eq!(normalize_folder(&messy).unwrap(), std::path::absolute(&nested).unwrap());
        assert!(normalize_folder(&dir.path().join("missing")).is_err());
    }
```

Run: `cargo test --workspace`
Expected: 编译失败或上述测试失败。

- [ ] **Step 2: 实现**

1. `config.rs`：

```rust
/// Which system notifications are shown (design section 9.5); each can be switched off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    pub run_started: bool,
    pub awaiting_acceptance: bool,
    pub needs_info: bool,
    pub failed: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { run_started: true, awaiting_acceptance: true, needs_info: true, failed: true }
    }
}
```

`Settings` 在 `trigger` 之后加 `pub notifications: NotificationSettings,`、`/// Check for and install updates automatically (from M3). pub auto_update: bool,`、`/// The desktop panel's first-run guide was finished. pub onboarding_completed: bool,` 与 `/// The user confirmed the full-access warning (design section 14) once. pub full_access_acknowledged: bool,`；`Default` 补 `notifications: NotificationSettings::default(), auto_update: true, onboarding_completed: false, full_access_acknowledged: false`。`validate` 里语言两项的条件改为：

```rust
        let plain_value = |value: &str| {
            !value.trim().is_empty()
                && !value.chars().any(|c| c.is_control() || crate::text::is_invisible(c))
        };
        anyhow::ensure!(plain_value(&self.comment_language), "comment_language must be a non-empty single-line value");
        anyhow::ensure!(plain_value(&self.language), "language must be a non-empty single-line value (\"system\" or a language tag)");
```

`unknown_keys`：

```rust
/// The keys in a config file's text that this version does not know, such as `settings.foo` or
/// `bindings[1].bar`. They are ignored on load and lost on the next save, so the host warns
/// about them. Text that does not parse gives none (loading reports that).
pub fn unknown_keys(text: &str) -> Vec<String> {
    let Ok(file) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    // Every key this version writes: the defaults, plus a binding with every optional field set.
    let mut sample = Config::default();
    sample.bindings.push(Binding {
        id: "x".into(),
        project_id: "x".into(),
        project_name: "x".into(),
        directory: PathBuf::from("/x"),
        executor: Some(ExecutorKind::Claude),
        permission: Some(PermissionProfile::Full),
        model: Some("x".into()),
        prompt_override: Some("x".into()),
    });
    let known = toml::Table::try_from(&sample).expect("the config serializes");
    let mut unknown = Vec::new();
    collect_unknown(&file, &known, "", &mut unknown);
    unknown
}

fn collect_unknown(file: &toml::Table, known: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    for (key, value) in file {
        let name = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match (value, known.get(key)) {
            (_, None) => out.push(name),
            (toml::Value::Table(inner), Some(toml::Value::Table(known_inner))) => {
                collect_unknown(inner, known_inner, &name, out)
            }
            (toml::Value::Array(items), Some(toml::Value::Array(known_items))) => {
                let Some(toml::Value::Table(template)) = known_items.first() else { continue };
                for (i, item) in items.iter().enumerate() {
                    if let toml::Value::Table(item) = item {
                        collect_unknown(item, template, &format!("{name}[{i}]"), out);
                    }
                }
            }
            _ => {}
        }
    }
}
```

（`toml::Table` 的键是有序的 `BTreeMap` 时，测试里的期望顺序要按实际迭代顺序调整：以 `toml` crate 的 `preserve_order` 是否开启为准，测试可先排序再比较。）

`save` 开头调用：

```rust
/// Temp files older than this next to the config file are leftovers of a save that crashed.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Deletes `<file name>.<pid>.<n>.tmp` files next to `path` older than `STALE_TEMP_AGE`: a save
/// that crashed between writing and renaming leaves one behind. Recent ones may belong to a
/// save in progress and are left alone; errors are ignored (this is housekeeping).
fn sweep_stale_temp_files(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let prefix = format!("{name}.");
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else { continue };
        let Some(middle) = file_name.strip_prefix(&prefix).and_then(|rest| rest.strip_suffix(".tmp")) else { continue };
        let is_temp = middle.split('.').count() == 2 && middle.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() > STALE_TEMP_AGE);
        if is_temp && stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
```

2. `engine.rs` 的 `Engine::new` 在读到 `config_file` 之后：

```rust
        if let Some(bytes) = &config_file {
            let unknown = crate::config::unknown_keys(&String::from_utf8_lossy(bytes));
            if !unknown.is_empty() {
                tracing::warn!(keys = ?unknown, "config.toml has keys this version does not know; they are ignored and dropped on the next save");
            }
        }
```

3. `executor/mod.rs`：

```rust
/// Whether `dir` is inside a git checkout: it, or a folder above it, has a `.git` directory or
/// file (worktrees use a file). A folder that cannot be read counts as not a checkout, which only
/// adds Codex's `--skip-git-repo-check`.
pub fn is_git_repo(dir: &Path) -> bool {
    dir.ancestors().any(|folder| folder.join(".git").exists())
}
```

4. `crates/cli/src/mcp.rs`：`pause`、`resume`、`poll_now` 三个工具的属性改为 `#[rmcp::tool(description = "…", annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true))]`；`forward` 在把宿主回复交给 MCP 之前去掉不可见字符：

```rust
        Ok(reply.chars().filter(|c| !issuerelay_core::text::is_invisible(*c)).collect())
```

（JSON 已经转义了控制字符，剩下能误导读者的是不可见与改向字符。）

5. `crates/cli/src/main.rs`：

```rust
/// The folder to bind, absolute and without `.` or `..` segments, without resolving symlinks
/// (the engine compares folders by their canonical path itself). It must exist.
fn normalize_folder(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut normal = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    anyhow::ensure!(normal.is_dir(), "{} is not a folder", normal.display());
    Ok(normal)
}
```

`bind add` 用它代替原来的 `std::path::absolute` 加存在性检查。`doctor` 子命令发请求之前，若标准错误是终端，先 `eprintln!("Checking Linear, Claude Code and Codex… (this can take up to a minute)");`。

Run: `cargo test --workspace`
Expected: PASS。

- [ ] **Step 3: 文档与提交**

- 设计第 11 节补：「加载时提醒本版本不认识的配置键（下次保存会丢掉它们）；保存时清扫同目录下超过 10 分钟的临时文件。」
- M0–M1 计划「评审遗留」M2 表里 `config`、`executor`、`cli` 各行末尾加「（M2 第一部分 Task 7）」。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates docs
git commit -m "fix(core,cli): warn about unknown config keys and tidy MCP output"
```

---

### Task 8: 一键配置命令

设计第 8 节第 4 步：执行者没配 Linear MCP 时，可一键执行官方添加命令，再引导用户在浏览器里完成授权；第 10.3 节：是否把 IssueRelay 的 MCP 控制服务注册到 Claude Code / Codex 由用户在设置里选，可一键注册或移除。这里只做命令本身（固定参数、可测试），按钮在第二部分。实测（Claude Code 2.1.285、codex-cli 0.159.2）：两个执行者都有 `mcp login <名称>`；`mcp get <名称>` 找不到时退出码 1，找到时 0。

**Files:**
- Create: `crates/core/src/setup.rs`
- Modify: `crates/core/src/lib.rs`（`pub mod setup;`）
- Modify: `crates/core/src/doctor.rs`（`CommandRunner::run_limited`，`SystemRunner` 实现它）
- Test: `crates/core/src/setup.rs` 末尾 `mod tests`

**Interfaces:**
- Produces:
  - `pub enum setup::SetupAction { AddLinearMcp(ExecutorKind), LoginLinearMcp(ExecutorKind), RegisterControlServer(ExecutorKind), RemoveControlServer(ExecutorKind) }`（`Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`，serde `tag = "kind", content = "executor", rename_all = "snake_case"`）
  - `pub const setup::CONTROL_SERVER_NAME: &str = "issuerelay"`
  - `pub fn setup::arguments(action: SetupAction, cli: Option<&Path>) -> Result<Vec<String>>`
  - `pub fn setup::time_limit(action: SetupAction) -> Duration`（登录 5 分钟，其余 60 秒）
  - `pub async fn setup::run(action: SetupAction, program: &Path, cli: Option<&Path>, runner: &dyn CommandRunner) -> Result<CmdOutput>`：命令退出码非 0 时返回错误，错误里带清洗过、截断过的输出
  - `pub async fn setup::control_server_registered(program: &Path, runner: &dyn CommandRunner) -> Result<bool>`
  - `CommandRunner::run_limited(&self, program: &Path, args: &[&str], limit: Duration) -> Result<CmdOutput>`（默认实现调 `run`；`SystemRunner` 用给定上限）
- Consumes: Task 1 的 `SystemRunner`（主目录、进程组、环境）

- [ ] **Step 1: 写失败测试**

新建 `crates/core/src/setup.rs`，先放模块注释与空壳，再加测试（`Recorder` 是测试里自带的记录型假实现）：

```rust
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Records every command and answers with `reply`.
    struct Recorder {
        reply: CmdOutput,
        calls: Mutex<Vec<(PathBuf, Vec<String>, Duration)>>,
    }

    impl Recorder {
        fn new(code: i32, stdout: &str, stderr: &str) -> Self {
            Self {
                reply: CmdOutput { code: Some(code), stdout: stdout.into(), stderr: stderr.into() },
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for Recorder {
        async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
            self.run_limited(program, args, Duration::ZERO).await
        }

        async fn run_limited(&self, program: &Path, args: &[&str], limit: Duration) -> Result<CmdOutput> {
            self.calls.lock().unwrap().push((
                program.to_path_buf(),
                args.iter().map(|a| a.to_string()).collect(),
                limit,
            ));
            Ok(self.reply.clone())
        }
    }

    fn args(action: SetupAction, cli: Option<&Path>) -> Vec<String> {
        arguments(action, cli).unwrap()
    }

    #[test]
    fn every_action_runs_a_fixed_official_command() {
        use ExecutorKind::{Claude, Codex};
        let cli = Path::new("/Applications/IssueRelay.app/Contents/MacOS/issuerelay");
        assert_eq!(
            args(SetupAction::AddLinearMcp(Claude), None),
            ["mcp", "add", "--scope", "user", "--transport", "http", "linear-server", "https://mcp.linear.app/mcp"]
        );
        assert_eq!(
            args(SetupAction::AddLinearMcp(Codex), None),
            ["mcp", "add", "linear", "--url", "https://mcp.linear.app/mcp"]
        );
        assert_eq!(args(SetupAction::LoginLinearMcp(Claude), None), ["mcp", "login", "linear-server"]);
        assert_eq!(args(SetupAction::LoginLinearMcp(Codex), None), ["mcp", "login", "linear"]);
        assert_eq!(
            args(SetupAction::RegisterControlServer(Claude), Some(cli)),
            ["mcp", "add", "--scope", "user", "issuerelay", "--", cli.to_str().unwrap(), "mcp"]
        );
        assert_eq!(
            args(SetupAction::RegisterControlServer(Codex), Some(cli)),
            ["mcp", "add", "issuerelay", "--", cli.to_str().unwrap(), "mcp"]
        );
        assert_eq!(args(SetupAction::RemoveControlServer(Claude), None), ["mcp", "remove", "--scope", "user", "issuerelay"]);
        assert_eq!(args(SetupAction::RemoveControlServer(Codex), None), ["mcp", "remove", "issuerelay"]);
        assert!(arguments(SetupAction::RegisterControlServer(Claude), None).is_err(), "registering needs the CLI");
    }

    #[test]
    fn a_login_may_wait_for_the_browser() {
        assert_eq!(time_limit(SetupAction::LoginLinearMcp(ExecutorKind::Claude)), Duration::from_secs(300));
        assert_eq!(time_limit(SetupAction::AddLinearMcp(ExecutorKind::Codex)), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_cleaned_output() {
        let runner = Recorder::new(1, "", "error: server exists\u{1b}[2J already\n");
        let error = run(SetupAction::AddLinearMcp(ExecutorKind::Claude), Path::new("/fake/claude"), None, &runner)
            .await
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("server exists") && text.contains("exit code 1"), "{text}");
        assert!(!text.contains('\u{1b}'), "{text:?}");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].0, PathBuf::from("/fake/claude"));
        assert_eq!(calls[0].2, Duration::from_secs(60));
        let ok = Recorder::new(0, "Added", "");
        assert!(run(SetupAction::RemoveControlServer(ExecutorKind::Codex), Path::new("/fake/codex"), None, &ok).await.is_ok());
    }

    #[tokio::test]
    async fn registration_is_read_from_mcp_get() {
        let found = Recorder::new(0, "issuerelay:\n  Type: stdio", "");
        assert!(control_server_registered(Path::new("/fake/claude"), &found).await.unwrap());
        assert_eq!(found.calls.lock().unwrap()[0].1, ["mcp", "get", "issuerelay"]);
        let missing = Recorder::new(1, "", "No MCP server named \"issuerelay\"");
        assert!(!control_server_registered(Path::new("/fake/claude"), &missing).await.unwrap());
    }
}
```

Run: `cargo test -p issuerelay-core --lib setup::`
Expected: FAIL（空壳）。

- [ ] **Step 2: `CommandRunner::run_limited`**

`crates/core/src/doctor.rs` 的 `CommandRunner` 加默认方法：

```rust
    /// `run` with an explicit time limit instead of the runner's own choice (the setup commands:
    /// a login waits for the user in the browser). The default ignores `limit`; `SystemRunner`
    /// honours it.
    async fn run_limited(&self, program: &Path, args: &[&str], limit: Duration) -> Result<CmdOutput> {
        let _ = limit;
        self.run(program, args).await
    }
```

`SystemRunner`：把现有 `run` 的主体移到 `run_limited`（用参数 `limit`），`run` 变成 `self.run_limited(program, args, self.limit_for(args)).await`。

- [ ] **Step 3: 实现 `setup.rs`**

```rust
//! One-click setup commands offered by the environment check (design section 8) and the
//! settings page (design section 10.3): adding Linear's MCP server to an executor, starting the
//! executor's own browser login for it, and registering or removing IssueRelay's MCP control
//! server. Every command is a fixed list of arguments; the only variable part is the path of
//! IssueRelay's own command-line program.
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::config::ExecutorKind;
use crate::doctor::{CmdOutput, CommandRunner};
use crate::linear::auth::LINEAR_MCP_URL;
use crate::text::clean_text;

/// The name IssueRelay's control server is registered under.
pub const CONTROL_SERVER_NAME: &str = "issuerelay";

/// The names the official instructions give Linear's server (design section 8).
const CLAUDE_LINEAR_NAME: &str = "linear-server";
const CODEX_LINEAR_NAME: &str = "linear";

/// A setup step the user can start with one click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "executor", rename_all = "snake_case")]
pub enum SetupAction {
    /// Add Linear's MCP server to the executor (user scope for Claude Code).
    AddLinearMcp(ExecutorKind),
    /// Start the executor's own browser login for Linear's MCP server.
    LoginLinearMcp(ExecutorKind),
    /// Register IssueRelay's control server (`issuerelay mcp`) with the executor.
    RegisterControlServer(ExecutorKind),
    /// Remove IssueRelay's control server from the executor.
    RemoveControlServer(ExecutorKind),
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// The arguments for `action`, after the executor's program. `cli` is IssueRelay's
/// command-line program, which registering the control server needs.
pub fn arguments(action: SetupAction, cli: Option<&Path>) -> Result<Vec<String>> {
    use ExecutorKind::{Claude, Codex};
    Ok(match action {
        SetupAction::AddLinearMcp(Claude) => owned(&[
            "mcp", "add", "--scope", "user", "--transport", "http", CLAUDE_LINEAR_NAME, LINEAR_MCP_URL,
        ]),
        SetupAction::AddLinearMcp(Codex) => owned(&["mcp", "add", CODEX_LINEAR_NAME, "--url", LINEAR_MCP_URL]),
        SetupAction::LoginLinearMcp(Claude) => owned(&["mcp", "login", CLAUDE_LINEAR_NAME]),
        SetupAction::LoginLinearMcp(Codex) => owned(&["mcp", "login", CODEX_LINEAR_NAME]),
        SetupAction::RegisterControlServer(kind) => {
            let cli = cli.ok_or_else(|| anyhow!("IssueRelay's command-line program was not found"))?;
            let cli = cli
                .to_str()
                .ok_or_else(|| anyhow!("the path of IssueRelay's command-line program is not valid Unicode"))?;
            match kind {
                Claude => owned(&["mcp", "add", "--scope", "user", CONTROL_SERVER_NAME, "--", cli, "mcp"]),
                Codex => owned(&["mcp", "add", CONTROL_SERVER_NAME, "--", cli, "mcp"]),
            }
        }
        SetupAction::RemoveControlServer(Claude) => owned(&["mcp", "remove", "--scope", "user", CONTROL_SERVER_NAME]),
        SetupAction::RemoveControlServer(Codex) => owned(&["mcp", "remove", CONTROL_SERVER_NAME]),
    })
}

/// How long `action` may take: a login waits for the user to finish in the browser.
pub fn time_limit(action: SetupAction) -> Duration {
    match action {
        SetupAction::LoginLinearMcp(_) => Duration::from_secs(5 * 60),
        _ => Duration::from_secs(60),
    }
}

/// The most characters of a failed command's output an error carries.
const MAX_OUTPUT_CHARS: usize = 400;

/// Runs `action` with `program`, the executor as the environment check found it. A command that
/// exits non-zero is an error carrying its (cleaned, shortened) output.
pub async fn run(
    action: SetupAction,
    program: &Path,
    cli: Option<&Path>,
    runner: &dyn CommandRunner,
) -> Result<CmdOutput> {
    let args = arguments(action, cli)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = runner.run_limited(program, &refs, time_limit(action)).await?;
    if output.code != Some(0) {
        let text = if output.stderr.trim().is_empty() { &output.stdout } else { &output.stderr };
        let shown: String = clean_text(text).chars().take(MAX_OUTPUT_CHARS).collect();
        let code = output.code.map_or_else(|| "none".to_string(), |c| c.to_string());
        bail!("`{} {}` failed (exit code {code}): {shown}", program.display(), args.join(" "));
    }
    Ok(output)
}

/// Whether IssueRelay's control server is registered with the executor at `program`:
/// `mcp get issuerelay` exits 0 only then (measured with Claude Code 2.1.285 and codex-cli
/// 0.159.2).
pub async fn control_server_registered(program: &Path, runner: &dyn CommandRunner) -> Result<bool> {
    let output = runner
        .run_limited(program, &["mcp", "get", CONTROL_SERVER_NAME], Duration::from_secs(60))
        .await?;
    Ok(output.code == Some(0))
}
```

Run: `cargo test -p issuerelay-core --lib setup::`
Expected: PASS。

- [ ] **Step 4: 文档与提交**

- 设计第 8 节第 4 步补：「授权命令为 `claude mcp login linear-server`、`codex mcp login linear`（会打开浏览器，程序最多等 5 分钟）。」第 10.3 节补：「注册命令为 `claude mcp add --scope user issuerelay -- <命令行程序> mcp`、`codex mcp add issuerelay -- <命令行程序> mcp`；是否已注册看 `mcp get issuerelay` 的退出码。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 5 节加 `| 一键配置命令（Linear MCP、控制服务注册） | crates/core/src/setup.rs |`；`.github/CODEOWNERS` 加 `/crates/core/src/setup.rs`（它替用户改执行者的配置）。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates/core docs .github
git commit -m "feat(core): add the one-click executor setup commands"
```

---

### Task 9: 可复用的宿主 `core::host`，devhost 改用它

把 devhost 里组装引擎的代码抽成 `Host`，桌面应用直接复用。宿主在没有 Linear 凭据、连接失败或 Linear 接口已变化时照常启动（用 `Disconnected` 占位，设计第 13 节），连接失败时每分钟自动重试；运行中可以登录、改用 API key、登出（换连接用 Task 5 的 `set_linear`）；停止分「等待完成」与「立即停止」两种（设计 6.2）。改用一种登录方式时清掉另一种保存的凭据，免得重启后用了旧的。

**Files:**
- Create: `crates/core/src/host.rs`
- Modify: `crates/core/src/lib.rs`（`pub mod host;`）
- Modify: `crates/core/src/linear/auth.rs`（`clear_oauth_credentials`、`clear_api_key`）
- Modify: `crates/devhost/src/main.rs`（`run` 改用 `Host`）
- Test: `crates/core/tests/host.rs`（新），复用 `tests/common`

**Interfaces:**
- Produces:
  - `pub fn auth::clear_oauth_credentials() -> Result<()>`、`pub fn auth::clear_api_key() -> Result<()>`（会阻塞，异步调用方放到阻塞线程池）
  - `pub enum host::AuthMethod { OAuth, ApiKey }`（serde 名为 `"oauth"`、`"api_key"`，面板的 TypeScript 类型照此读取）
  - `pub enum host::LinearSetup { Connected { method: AuthMethod }, NotConfigured, InterfaceChanged { message: String }, ConnectFailed { message: String } }`（serde `tag = "kind"`, `rename_all = "snake_case"`）
  - `pub enum host::StopMode { WaitForRuns, StopRuns }`
  - `#[async_trait] pub trait host::Connector`：`stored() -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>>`、`login(open_url: Box<dyn Fn(&str) + Send + Sync>) -> Result<Arc<dyn LinearApi>>`、`api_key(key: &str) -> Result<Arc<dyn LinearApi>>`、`logout() -> Result<()>`
  - `pub struct host::SystemConnector`（钥匙串 + Linear MCP）
  - `#[derive(Default)] pub struct host::HostParts { pub env: Option<SharedEnv>, pub readiness: Option<Arc<dyn ReadinessProbe>>, pub job_runner: Option<Arc<dyn JobRunner>>, pub retry_every: Option<Duration> }`
  - `#[derive(Clone)] pub struct host::Host`：`start(paths: AppPaths, connector: Arc<dyn Connector>, parts: HostParts) -> Result<Host>`（async）、`engine(&self) -> &Engine`、`env(&self) -> ExecEnv`、`paths(&self) -> &AppPaths`、`linear_setup(&self) -> LinearSetup`、`reconnect(&self) -> LinearSetup`（async）、`login(&self, open_url: impl Fn(&str) + Send + Sync + 'static) -> Result<Viewer>`（async）、`use_api_key(&self, key: &str) -> Result<Viewer>`（async）、`logout(&self) -> Result<()>`（async）、`stop(&self, mode: StopMode)`（async，可重复调用）
- Consumes: Task 1 `env::resolve`、`SystemReadiness::new(ttl, env)`、`ProcessJobRunner::new(env)`；Task 5 `Engine::set_linear`、`set_draining`、`Disconnected`

- [ ] **Step 1: 钥匙串的单项清除（先写失败测试）**

`crates/core/src/linear/auth.rs` 测试模块（用文件里现有的内存钥匙串与 `*_in(service)` 写法）加：

```rust
    #[test]
    fn each_kind_of_credential_can_be_cleared_on_its_own() {
        let service = test_service("clear-one");
        save_api_key_in(&service, "lin_api_demo").unwrap();
        entry(&service, OAUTH_ACCOUNT).unwrap().set_password("{\"token\":\"demo\"}").unwrap();
        clear_account_in(&service, OAUTH_ACCOUNT).unwrap();
        assert_eq!(load_api_key_in(&service).unwrap().as_deref(), Some("lin_api_demo"));
        assert!(matches!(entry(&service, OAUTH_ACCOUNT).unwrap().get_password(), Err(keyring::Error::NoEntry)));
        clear_account_in(&service, API_KEY_ACCOUNT).unwrap();
        assert_eq!(load_api_key_in(&service).unwrap(), None);
        clear_account_in(&service, API_KEY_ACCOUNT).unwrap();
    }
```

（`test_service` 换成文件里给每个测试取独立服务名的辅助。）

实现：

```rust
/// Deletes the saved OAuth credentials only (switching to an API key). Blocks; see
/// `clear_credentials`.
pub fn clear_oauth_credentials() -> Result<()> {
    clear_account_in(APP_ID, OAUTH_ACCOUNT)
}

/// Deletes the saved API key only (switching to OAuth). Blocks; see `clear_credentials`.
pub fn clear_api_key() -> Result<()> {
    clear_account_in(APP_ID, API_KEY_ACCOUNT)
}

/// Deletes one keychain entry; one that does not exist is fine.
fn clear_account_in(service: &str, account: &str) -> Result<()> {
    entry(service, account)
        .and_then(|item| match item.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        })
        .map_err(keychain_error)
}
```

`clear_credentials_in` 改为依次调用 `clear_account_in`（两个都试，报告第一个失败），行为不变。

Run: `cargo test -p issuerelay-core --lib linear::auth`
Expected: PASS。

- [ ] **Step 2: 宿主的失败测试**

新建 `crates/core/tests/host.rs`：

```rust
mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use common::{
    engine_kit::{FakeReadiness, ScriptedRunner},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    config::Binding,
    env::ExecEnv,
    host::{AuthMethod, Connector, Host, HostParts, LinearSetup, StopMode},
    ipc::server::AlreadyRunning,
    linear::{LinearApi, mcp::LinearError},
    model::JobState,
    paths::AppPaths,
};

/// What the fake connector answers.
#[derive(Clone)]
enum Stored {
    Nothing,
    Works(FakeLinear, AuthMethod),
    Fails(&'static str),
    InterfaceChanged,
}

struct FakeConnector {
    stored: Mutex<Vec<Stored>>,
    linear: FakeLinear,
    logouts: Mutex<usize>,
}

impl FakeConnector {
    /// `stored` answers the first calls to `stored()` in order; the last answer repeats.
    fn new(stored: Vec<Stored>) -> Arc<Self> {
        Arc::new(Self { stored: Mutex::new(stored), linear: FakeLinear::new("user-1"), logouts: Mutex::new(0) })
    }
}

#[async_trait::async_trait]
impl Connector for FakeConnector {
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>> {
        let answer = {
            let mut stored = self.stored.lock().unwrap();
            if stored.len() > 1 { stored.remove(0) } else { stored[0].clone() }
        };
        match answer {
            Stored::Nothing => Ok(None),
            Stored::Works(linear, method) => Ok(Some((Arc::new(linear), method))),
            Stored::Fails(message) => Err(anyhow!(message)),
            Stored::InterfaceChanged => Err(LinearError::MissingTools(vec!["list_issues".into()]).into()),
        }
    }

    async fn login(&self, open_url: Box<dyn Fn(&str) + Send + Sync>) -> Result<Arc<dyn LinearApi>> {
        open_url("https://mcp.linear.app/authorize?state=demo");
        Ok(Arc::new(self.linear.clone()))
    }

    async fn api_key(&self, key: &str) -> Result<Arc<dyn LinearApi>> {
        if key == "lin_api_good" { Ok(Arc::new(self.linear.clone())) } else { Err(anyhow!("Linear rejected the API key")) }
    }

    async fn logout(&self) -> Result<()> {
        *self.logouts.lock().unwrap() += 1;
        Ok(())
    }
}

/// A host under a temporary root with the scripted runner and fake readiness.
async fn host(connector: Arc<FakeConnector>, runner: Option<Arc<ScriptedRunner>>) -> (Host, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let parts = HostParts {
        env: Some(ExecEnv::inherit().shared()),
        readiness: Some(Arc::new(FakeReadiness)),
        job_runner: runner.map(|r| r as Arc<dyn issuerelay_core::engine::JobRunner>),
        retry_every: Some(Duration::from_millis(50)),
    };
    let host = Host::start(AppPaths::under(root.path()), connector, parts).await.unwrap();
    (host, root)
}

#[tokio::test]
async fn without_credentials_the_host_runs_and_waits_for_a_login() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    let (host, _root) = host(connector.clone(), None).await;
    assert_eq!(host.linear_setup(), LinearSetup::NotConfigured);
    assert!(host.engine().poll_once().await.is_err());
    let opened = Arc::new(Mutex::new(None));
    let seen = opened.clone();
    let viewer = host.login(move |url| *seen.lock().unwrap() = Some(url.to_string())).await.unwrap();
    assert_eq!(viewer.id, "user-1");
    assert!(opened.lock().unwrap().as_deref().unwrap().starts_with("https://mcp.linear.app/"));
    assert_eq!(host.linear_setup(), LinearSetup::Connected { method: AuthMethod::OAuth });
    host.engine().poll_once().await.unwrap();
    host.stop(StopMode::StopRuns).await;
}

#[tokio::test]
async fn changed_linear_tools_are_told_apart_from_other_failures() {
    let (host, _root) = host(FakeConnector::new(vec![Stored::InterfaceChanged]), None).await;
    assert!(matches!(host.linear_setup(), LinearSetup::InterfaceChanged { .. }));
    host.stop(StopMode::StopRuns).await;
}

#[tokio::test]
async fn a_failed_connection_is_retried_until_it_works() {
    let linear = FakeLinear::new("user-1");
    let connector = FakeConnector::new(vec![Stored::Fails("network is down"), Stored::Works(linear, AuthMethod::ApiKey)]);
    let (host, _root) = host(connector, None).await;
    assert!(matches!(host.linear_setup(), LinearSetup::ConnectFailed { ref message } if message.contains("network is down")));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while host.linear_setup() != (LinearSetup::Connected { method: AuthMethod::ApiKey }) {
        assert!(tokio::time::Instant::now() < deadline, "never reconnected");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    host.stop(StopMode::StopRuns).await;
}

#[tokio::test]
async fn an_api_key_and_a_logout_switch_the_connection() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    let (host, _root) = host(connector.clone(), None).await;
    assert!(host.use_api_key("lin_api_bad").await.is_err());
    assert_eq!(host.linear_setup(), LinearSetup::NotConfigured, "a rejected key changes nothing");
    host.use_api_key("lin_api_good").await.unwrap();
    assert_eq!(host.linear_setup(), LinearSetup::Connected { method: AuthMethod::ApiKey });
    host.logout().await.unwrap();
    assert_eq!(*connector.logouts.lock().unwrap(), 1);
    assert_eq!(host.linear_setup(), LinearSetup::NotConfigured);
    assert!(host.engine().poll_once().await.is_err());
    host.stop(StopMode::StopRuns).await;
}

#[tokio::test]
async fn a_second_host_for_the_same_user_is_refused() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    let root = tempfile::tempdir().unwrap();
    let parts = || HostParts {
        env: Some(ExecEnv::inherit().shared()),
        readiness: Some(Arc::new(FakeReadiness)),
        ..HostParts::default()
    };
    let first = Host::start(AppPaths::under(root.path()), connector.clone(), parts()).await.unwrap();
    let error = Host::start(AppPaths::under(root.path()), connector, parts()).await.err().unwrap();
    assert!(error.downcast_ref::<AlreadyRunning>().is_some(), "{error:#}");
    first.stop(StopMode::StopRuns).await;
}

/// A host with one bound project whose issue is running (the scripted runner holds it).
async fn host_with_a_running_job(gated: bool) -> (Host, Arc<ScriptedRunner>, tempfile::TempDir) {
    let linear = FakeLinear::new("user-1");
    let runner = Arc::new(ScriptedRunner::new(linear.clone(), gated));
    let connector = FakeConnector::new(vec![Stored::Works(linear.clone(), AuthMethod::OAuth)]);
    let (host, root) = host(connector, Some(runner.clone())).await;
    let dir = root.path().join("repo-a");
    std::fs::create_dir_all(&dir).unwrap();
    host.engine()
        .add_binding(Binding {
            id: "proj-1".into(),
            project_id: "proj-1".into(),
            project_name: "Demo Project".into(),
            directory: dir,
            executor: None,
            permission: None,
            model: None,
            prompt_override: None,
        })
        .unwrap();
    linear.upsert(issue("1", "proj-1"));
    host.engine().poll_now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while host.engine().status().running == 0 {
        assert!(tokio::time::Instant::now() < deadline, "the run never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (host, runner, root)
}

#[tokio::test]
async fn waiting_for_runs_lets_the_running_job_finish() {
    let (host, runner, _root) = host_with_a_running_job(true).await;
    let stopping = tokio::spawn({
        let host = host.clone();
        async move { host.stop(StopMode::WaitForRuns).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!stopping.is_finished(), "the stop waits for the run");
    assert!(host.engine().status().draining);
    runner.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), stopping).await.unwrap().unwrap();
    let job = host.engine().list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn stopping_runs_ends_the_running_job() {
    let (host, _runner, _root) = host_with_a_running_job(true).await;
    tokio::time::timeout(Duration::from_secs(5), host.stop(StopMode::StopRuns)).await.unwrap();
    let job = host.engine().list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::Canceled);
    host.stop(StopMode::StopRuns).await;
}
```

Run: `cargo test -p issuerelay-core --test host`
Expected: 编译失败（`host` 模块不存在）。

- [ ] **Step 3: 实现 `host.rs`**

```rust
//! The host: what a desktop or headless program runs around the engine. It binds the IPC
//! endpoint (one host per user), resolves the executors' environment, opens the run records,
//! connects to Linear with the stored credentials, runs the engine's background loop and the IPC
//! server, and lets the Linear connection change while it runs. Without credentials, with a
//! failed connection or with changed Linear tools it still starts: the engine gets a
//! `Disconnected` stand-in whose checks fail with the reason (design section 13), and a failed
//! connection is retried every minute.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::doctor::{ReadinessProbe, SystemReadiness};
use crate::engine::{Engine, EngineDeps, JobRunner, ProcessJobRunner};
use crate::env::{ExecEnv, SharedEnv};
use crate::ipc::server::IpcServer;
use crate::linear::mcp::{LinearError, McpLinear};
use crate::linear::{Disconnected, LinearApi, Viewer, auth};
use crate::paths::AppPaths;
use crate::store::Store;
use crate::text::clean_text;

/// How long the readiness checks reuse a complete result (design section 6.4).
const READINESS_TTL: Duration = Duration::from_secs(5 * 60);

/// How often a failed connection is tried again.
const RETRY_EVERY: Duration = Duration::from_secs(60);

/// How the stored credentials authorize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    // snake_case would make it "o_auth".
    #[serde(rename = "oauth")]
    OAuth,
    ApiKey,
}

/// What the host knows about the Linear connection. Whether the last check worked is in
/// `Engine::status` (`linear_ok`, `viewer`, `last_error`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinearSetup {
    /// Connected with stored credentials or a login made in this session.
    Connected { method: AuthMethod },
    /// No stored credentials: the user has to log in.
    NotConfigured,
    /// Linear's tools or their arguments changed (design section 7): checks stay off until
    /// IssueRelay is updated.
    InterfaceChanged { message: String },
    /// Credentials exist but connecting failed; tried again every minute.
    ConnectFailed { message: String },
}

/// How `Host::stop` treats the runs in progress (design section 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopMode {
    /// Start nothing new and wait for the runs in progress to finish.
    WaitForRuns,
    /// Stop the runs in progress (their process trees end) and wait until that is recorded.
    StopRuns,
}

/// How the host reaches Linear. `SystemConnector` is the real one; tests use a fake.
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    /// Connects with stored credentials, OAuth first, then a saved API key; `None` when neither
    /// is stored.
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>>;
    /// Browser OAuth; `open_url` shows or opens the authorization page. Keeps the credentials.
    async fn login(&self, open_url: Box<dyn Fn(&str) + Send + Sync>) -> Result<Arc<dyn LinearApi>>;
    /// Connects with a personal API key and keeps it, but only once it worked.
    async fn api_key(&self, key: &str) -> Result<Arc<dyn LinearApi>>;
    /// Forgets every stored credential.
    async fn logout(&self) -> Result<()>;
}

/// The system keychain and Linear's MCP server.
pub struct SystemConnector;

/// Runs a blocking keychain call on the blocking pool (it may show a system prompt).
async fn blocking<T: Send + 'static>(call: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(call).await.context("the keychain call stopped unexpectedly")?
}

#[async_trait::async_trait]
impl Connector for SystemConnector {
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>> {
        if let Some(client) = auth::connect_stored(auth::LINEAR_MCP_URL).await? {
            return Ok(Some((Arc::new(McpLinear::new(client).await?), AuthMethod::OAuth)));
        }
        if let Some(key) = blocking(auth::load_api_key).await? {
            let client = auth::connect_api_key(auth::LINEAR_MCP_URL, &key).await?;
            return Ok(Some((Arc::new(McpLinear::new(client).await?), AuthMethod::ApiKey)));
        }
        Ok(None)
    }

    async fn login(&self, open_url: Box<dyn Fn(&str) + Send + Sync>) -> Result<Arc<dyn LinearApi>> {
        let client = auth::login(auth::LINEAR_MCP_URL, move |url| open_url(url)).await?;
        let linear = McpLinear::new(client).await?;
        // OAuth now: a saved key would only stand in the way of the next start.
        blocking(auth::clear_api_key).await?;
        Ok(Arc::new(linear))
    }

    async fn api_key(&self, key: &str) -> Result<Arc<dyn LinearApi>> {
        let linear = McpLinear::new(auth::connect_api_key(auth::LINEAR_MCP_URL, key).await?).await?;
        linear.viewer().await.context("Linear did not accept the API key")?;
        let key = key.to_string();
        blocking(move || {
            // Stored OAuth credentials win at the next start; the user chose the key.
            auth::clear_oauth_credentials()?;
            auth::save_api_key(&key)
        })
        .await?;
        Ok(Arc::new(linear))
    }

    async fn logout(&self) -> Result<()> {
        blocking(auth::clear_credentials).await
    }
}

/// Replaceable parts, for tests; `HostParts::default()` uses the real ones.
#[derive(Default)]
pub struct HostParts {
    pub env: Option<SharedEnv>,
    pub readiness: Option<Arc<dyn ReadinessProbe>>,
    pub job_runner: Option<Arc<dyn JobRunner>>,
    /// How often a failed connection is retried; `RETRY_EVERY` by default.
    pub retry_every: Option<Duration>,
}

struct Inner {
    engine: Engine,
    env: SharedEnv,
    paths: AppPaths,
    connector: Arc<dyn Connector>,
    setup: Mutex<LinearSetup>,
    shutdown: CancellationToken,
    worker: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    ipc: tokio::sync::Mutex<Option<JoinHandle<Result<()>>>>,
}

/// Cheap to clone; every clone is the same host.
#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

/// Whether connecting failed because Linear's tools or arguments changed.
fn is_interface_change(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<LinearError>(),
            Some(LinearError::MissingTools(_) | LinearError::MissingArguments(_))
        )
    })
}

/// Connects with the stored credentials. Never fails: without a connection the engine gets a
/// stand-in that says why.
async fn connect_stored(connector: &dyn Connector) -> (Arc<dyn LinearApi>, LinearSetup) {
    match connector.stored().await {
        Ok(Some((linear, method))) => (linear, LinearSetup::Connected { method }),
        Ok(None) => (
            Arc::new(Disconnected::new("not connected to Linear: log in first")),
            LinearSetup::NotConfigured,
        ),
        Err(e) => {
            let message = clean_text(&format!("{e:#}"));
            let setup = if is_interface_change(&e) {
                LinearSetup::InterfaceChanged { message: message.clone() }
            } else {
                LinearSetup::ConnectFailed { message: message.clone() }
            };
            (Arc::new(Disconnected::new(message)), setup)
        }
    }
}

impl Host {
    /// Binds the IPC endpoint (fails with `AlreadyRunning` when another host serves this user),
    /// connects to Linear and starts the engine's loop and the IPC server.
    pub async fn start(paths: AppPaths, connector: Arc<dyn Connector>, parts: HostParts) -> Result<Host> {
        let server = IpcServer::bind(&paths.ipc_endpoint()).await?;
        let env = match parts.env {
            Some(env) => env,
            None => crate::env::resolve().await.shared(),
        };
        let (linear, setup) = connect_stored(connector.as_ref()).await;
        let engine = Engine::new(EngineDeps {
            config_path: paths.config_file(),
            store: Store::open(&paths.state_db())?,
            linear,
            readiness: parts
                .readiness
                .unwrap_or_else(|| Arc::new(SystemReadiness::new(READINESS_TTL, env.clone()))),
            job_runner: parts
                .job_runner
                .unwrap_or_else(|| Arc::new(ProcessJobRunner::new(env.clone()))),
            logs_dir: paths.logs_dir(),
        })?;
        let shutdown = CancellationToken::new();
        let ipc = tokio::spawn(server.run(engine.clone(), shutdown.clone()));
        let worker = {
            let engine = engine.clone();
            let shutdown = shutdown.clone();
            tokio::spawn(async move { engine.run_forever(shutdown).await })
        };
        let host = Host {
            inner: Arc::new(Inner {
                engine,
                env,
                paths,
                connector,
                setup: Mutex::new(setup),
                shutdown,
                worker: tokio::sync::Mutex::new(Some(worker)),
                ipc: tokio::sync::Mutex::new(Some(ipc)),
            }),
        };
        tokio::spawn(host.clone().retry_failed_connections(parts.retry_every.unwrap_or(RETRY_EVERY)));
        Ok(host)
    }

    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    /// The executors' environment as last resolved.
    pub fn env(&self) -> ExecEnv {
        self.inner.env.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn paths(&self) -> &AppPaths {
        &self.inner.paths
    }

    pub fn linear_setup(&self) -> LinearSetup {
        self.inner.setup.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_setup(&self, setup: LinearSetup) {
        *self.inner.setup.lock().unwrap_or_else(PoisonError::into_inner) = setup;
    }

    /// Connects again with the stored credentials (after a failure, or after an update fixed an
    /// interface change) and returns what came of it.
    pub async fn reconnect(&self) -> LinearSetup {
        let (linear, setup) = connect_stored(self.inner.connector.as_ref()).await;
        self.inner.engine.set_linear(linear);
        self.set_setup(setup.clone());
        setup
    }

    /// Logs in with the browser; `open_url` opens the authorization page. On success the engine
    /// uses the new connection at once.
    pub async fn login(&self, open_url: impl Fn(&str) + Send + Sync + 'static) -> Result<Viewer> {
        let linear = self.inner.connector.login(Box::new(open_url)).await?;
        let viewer = linear.viewer().await?;
        self.inner.engine.set_linear(linear);
        self.set_setup(LinearSetup::Connected { method: AuthMethod::OAuth });
        Ok(viewer)
    }

    /// Switches to a personal API key, kept only if Linear accepts it. A rejected key changes
    /// nothing.
    pub async fn use_api_key(&self, key: &str) -> Result<Viewer> {
        let linear = self.inner.connector.api_key(key).await?;
        let viewer = linear.viewer().await?;
        self.inner.engine.set_linear(linear);
        self.set_setup(LinearSetup::Connected { method: AuthMethod::ApiKey });
        Ok(viewer)
    }

    /// Forgets the stored credentials; checks fail until the next login.
    pub async fn logout(&self) -> Result<()> {
        self.inner.connector.logout().await?;
        self.inner.engine.set_linear(Arc::new(Disconnected::new("logged out of Linear: log in again")));
        self.set_setup(LinearSetup::NotConfigured);
        Ok(())
    }

    /// Stops the host: the engine's loop, the runs as `mode` says, and the IPC server. Returns
    /// once the runs have ended and their results are recorded. Calling it again does nothing.
    pub async fn stop(&self, mode: StopMode) {
        let engine = &self.inner.engine;
        if mode == StopMode::WaitForRuns {
            engine.set_draining(true);
            engine.wait_idle().await;
        }
        // `run_forever` tells the runs still in progress to stop on its way out.
        self.inner.shutdown.cancel();
        if let Some(worker) = self.inner.worker.lock().await.take()
            && let Err(e) = worker.await
        {
            tracing::error!(error = %e, "the engine task failed; the runs still going end with the host");
        }
        engine.wait_idle().await;
        if let Some(ipc) = self.inner.ipc.lock().await.take() {
            match ipc.await {
                Ok(Err(e)) => tracing::warn!(error = %format!("{e:#}"), "the IPC server stopped with an error"),
                Err(e) => tracing::warn!(error = %e, "the IPC server task failed"),
                Ok(Ok(())) => {}
            }
        }
    }

    /// Tries a failed connection again every `every` until it works, the user changes the
    /// connection, or the host stops.
    async fn retry_failed_connections(self, every: Duration) {
        loop {
            tokio::select! {
                _ = self.inner.shutdown.cancelled() => return,
                _ = tokio::time::sleep(every) => {}
            }
            if matches!(self.linear_setup(), LinearSetup::ConnectFailed { .. }) {
                let setup = self.reconnect().await;
                if let LinearSetup::ConnectFailed { message } = &setup {
                    tracing::debug!(%message, "Linear is still unreachable");
                }
            }
        }
    }
}
```

`host.rs` 末尾加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_reads_stable_names() {
        assert_eq!(serde_json::to_string(&AuthMethod::OAuth).unwrap(), "\"oauth\"");
        assert_eq!(serde_json::to_string(&AuthMethod::ApiKey).unwrap(), "\"api_key\"");
        let setup = serde_json::to_value(LinearSetup::ConnectFailed { message: "down".into() }).unwrap();
        assert_eq!(setup, serde_json::json!({"kind": "connect_failed", "message": "down"}));
    }
}
```

Run: `cargo test -p issuerelay-core --test host` 与 `cargo test -p issuerelay-core --lib host::`
Expected: PASS。

- [ ] **Step 4: devhost 改用 `Host`**

`crates/devhost/src/main.rs`：删掉 `serve`；`Command::Run` 改为：

```rust
        Command::Run => {
            let host = Host::start(AppPaths::system()?, Arc::new(SystemConnector), HostParts::default()).await?;
            match host.linear_setup() {
                LinearSetup::Connected { .. } => {}
                LinearSetup::NotConfigured => tracing::warn!(
                    "not logged in to Linear: run `issuerelay-devhost login` (or `api-key`); checks fail until then"
                ),
                LinearSetup::InterfaceChanged { message } | LinearSetup::ConnectFailed { message } => {
                    tracing::warn!(%message, "not connected to Linear")
                }
            }
            tracing::info!("IssueRelay devhost is running; press Ctrl+C to stop");
            shutdown_signal().await.context("cannot listen for stop signals")?;
            tracing::info!("stopping: waiting for running jobs to end (press Ctrl+C again to stop at once)");
            tokio::select! {
                () = host.stop(StopMode::StopRuns) => Ok(()),
                second = shutdown_signal() => {
                    second.context("cannot listen for a second stop signal")?;
                    tracing::warn!("second stop signal: ending the remaining runs now");
                    Ok(())
                }
            }
        }
```

（`connect()` 仍供 `Projects` 用；它的报错文案不变。用不到的 `use` 一并删掉。）devhost 的行为变化：没有凭据时不再拒绝启动，而是告警后照常运行（之后可以用命令行 `issuerelay bind add`，登录后检查恢复）。

Run: `cargo test --workspace`，再 `cargo build -p issuerelay-devhost`
Expected: PASS。

- [ ] **Step 5: 文档与提交**

- 设计第 4.1 节补：「宿主逻辑在 `crates/core/src/host.rs`：托盘程序与开发宿主共用。」第 13 节「Linear 连接失败或令牌失效」一行补：「连接失败时每分钟自动重试；改用另一种登录方式时清掉之前那种保存的凭据。」
- `docs/ai-coding/AI_CODING_GUIDE.md` 第 3 节分层表的入口层加 `core::host`（宿主组装），第 5 节加 `| 宿主组装（IPC、后台循环、Linear 连接切换） | crates/core/src/host.rs |`；`.github/CODEOWNERS` 加 `/crates/core/src/host.rs`。
- `README.md` 开发一节里 devhost 的说明补一句「没有登录时也会启动，并提示先登录」。

Run: 四条验证命令
Expected: 全部通过。

```bash
git add crates docs .github README.md
git commit -m "feat(core): extract the host so the tray app and the devhost share it"
```

---

## 自查记录（写计划时）

- 设计覆盖：第 8 节 PATH 与代理（Task 1）、6.2 Windows 进程标志与崩溃残留（Task 2、4）、6.4 检查命令进程树（Task 1）、10.2 管道权限（Task 2）、11 日志清理（Task 3）、5.6 / 9.2 面板所需的引擎接口（Task 5）、7 中文链接与字段检查（Task 6）、8 第 3 步按团队检查（Task 6）、8 第 4 步与 10.3 一键命令（Task 8）、13 无连接照常运行与重连（Task 5、9）。界面、托盘、通知、开机自启、图标、多语言、首次引导、精简运行的评估都在 M2 第二部分。
- 「评审遗留的小项」M2 两张表逐行认领：引擎与运行各行（Task 3、4、5）；Linear、环境检查、IPC、命令行与面板各行（Task 1、2、6、7、8）；`linear::mcp` 改用 rmcp 自带超时与取消一项不做——现有的 30 秒包装已覆盖超时，换实现没有新的行为收益，作为已知取舍保留；`ToolFailed` 原文转义在面板显示处做（第二部分）；面板「默认模型」与带 `[ ]` 的模型名说明在第二部分。测试打磨、扫描脚本、文档文字三组不在本计划，按原表保留。
- 类型一致：`ExecEnv`/`SharedEnv`（Task 1）→ Task 4 `runner::run`、Task 9 `Host`；`JobRunner::run` 的 `on_spawn`（Task 4）→ Task 5、9 的测试 kit；`check_workspace` 第三个参数（Task 6）→ `Engine::doctor`；`CommandRunner::run_limited`（Task 8）由 Task 1 改写过的 `SystemRunner` 实现。
