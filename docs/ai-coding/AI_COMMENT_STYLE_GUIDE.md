# AI 注释规范

## 1. 总原则

- 注释写「为什么」和代码看不出来的约束；代码本身能说清的不写。
- 源码注释用英文；文档（`docs/`、`README.md`、`AGENTS.md`）用中文，短句，先结论后细节。
- 注释和代码在同一次修改里更新；过时的注释比没有注释更糟。

## 2. 模块注释 `//!`

每个源文件第一行用 `//!` 一句话说明这个模块负责什么，必要时第二句说明关键约束。例如：

```rust
//! Runs one executor process: feeds the prompt on stdin, streams output into a JSONL log and
//! always kills the whole process tree when the run ends.
```

## 3. 文档注释 `///`

- 写在公开的类型、字段、函数上：做什么、关键约束、什么时候返回错误或 `None`。
- 不复述函数签名，不写「This function…」。

## 4. 实现注释 `//`

- 解释不显然的原因，例如「wait on the native child: the Job Object wrapper also waits for leftover processes」。
- 外部事实写明来源与日期，例如「confirmed with codex-cli 0.159.2 on 2026-10-10」。

## 5. unsafe

每个 `unsafe` 块前写 `// SAFETY:`，说明为什么在这里成立。

## 6. 测试

- 测试函数名用完整的 snake_case 句子说清期望的行为，例如 `failed_issue_requeues_only_after_linear_update`。
- 场景不直观时，在测试开头用一行注释说明在验证什么。

## 7. Shell 脚本

文件头写用途、用法和退出码，并使用 `set -euo pipefail`。

## 8. Markdown

- 中文，短句，一段一个意思；先给结论再给细节。
- 示例只用占位符与虚构数据（`<LINEAR_API_KEY>`、`DEMO-1`、`Demo Project`、`Jane Doe`、`~/…`）。

## 9. TOML 与 JSON 夹具

- TOML 只在取值不显然时加注释。
- JSON 不能写注释：夹具的来源和日期写在使用它的测试里，或写在同目录的 README 里。

## 10. 检查清单

1. 每个新文件都有 `//!` 吗？
2. 公开接口都有 `///` 吗？
3. 每个 `unsafe` 都有 `// SAFETY:` 吗？
4. 外部事实写了日期和版本吗？
5. 注释里有没有真实姓名、需求内容、本机路径、令牌？
