## 改动说明 / Summary

<!-- 做了什么、为什么。PR 标题请用英文 Conventional Commits，例如 `fix(runner): kill the whole process tree on timeout`。 -->

## 关联 issue / Related issue

<!-- 例如 Closes #<编号>；没有就写「无」 -->

## 风险等级 / Risk level

<!-- 见 docs/ai-coding/AI_ENGINEERING_GUARDRAILS.md；勾一个 -->
- [ ] 低 / Low
- [ ] 中 / Medium
- [ ] 高 / High
- [ ] 极高 / Critical（执行者参数与权限档位、提示词固定规则、进程树结束、凭据存储与授权、IPC 方法表、标记完成）

## 验证 / Verification

<!-- 先 `pnpm --dir app install`，并在 cargo 命令之前 `pnpm --dir app build`（应用 crate 的发布构建内嵌 app/dist） -->
- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `pnpm --dir app lint`
- [ ] `pnpm --dir app test`
- [ ] `bash scripts/check-secrets.sh --strict`（无输出）
- 本机测试过的平台 / Platforms tested locally: macOS / Windows / 仅 CI

## 实机验收 / Real-machine check

<!-- 涉及真实 Linear、Claude Code 或 Codex 行为时：写结果（并更新 docs/gates/），或写「待实机」及原因；不涉及就写「不涉及」 -->

## 自查 / Checklist

- [ ] 描述、截图、日志、夹具里没有令牌或凭据、公司名、内网地址、真实 Linear 内容、本机绝对路径
- [ ] 行为变化已同步文档
- [ ] 含 AI 生成的内容，我已逐行审过 / Contains AI-generated changes, reviewed line by line（不含就不勾）
