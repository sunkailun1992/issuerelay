# AI 编程规范目录

本目录集中维护 IssueRelay 的 AI 编程规范。根目录 `AGENTS.md` 只放入口和最高优先级的边界；细分规范都在这里。结构与作者其他个人仓库（imctl 等）的 `docs/ai-coding/` 一致，内容按 Rust 工作区、Tauri 桌面托盘程序与 React 面板、Linear MCP 和「无人值守调用 Claude Code / Codex」的场景收敛。

## 目录职责

```text
AGENTS.md
  -> docs/ai-coding/README.md            本文件：目录、阅读顺序、维护规则
    -> AI_CODING_GUIDE.md                分层、数据流、运行数据、外部接口变更、版本与提交、桌面面板
    -> AI_DIRECTORY_STRUCTURE_GUIDE.md   crate、模块、前端工程、测试、文档、脚本放在哪里
    -> AI_COMMENT_STYLE_GUIDE.md         Rust / TypeScript / Shell / Markdown / TOML / JSON 的注释写法
    -> AI_DESIGN_PATTERN_GUIDE.md        既有模式（接口 + 假实现、纯函数决策、面板的后端接口……）与禁止过度抽象
    -> AI_ENGINEERING_GUARDRAILS.md      风险分级、完成标准、必查项、本地与云端、交付说明模板
    -> SECURITY_CODING_SPEC.md           凭据、执行者权限、不可信输入与面板显示、子进程、本机通信与网页视图、开源卫生
    -> TESTING_SPEC.md                   测试分层、假实现、前端测试、共用用例表、夹具脱敏、验收报告模板
    -> AI_AUTOMATION_WORKFLOW.md         设计 → 计划 → 实现 → 验收 → 推送的标准流程
```

`.github/copilot-instructions.md` 只做转发，避免与 `AGENTS.md` 冲突。

版本、分支与提交的规则在 `AI_CODING_GUIDE.md` 第 10、11 节，不单独成文；M3 的发布流程落地后再看是否拆出。

## 阅读顺序

1. 根目录 `AGENTS.md`
2. 本文件
3. `AI_CODING_GUIDE.md`
4. 与任务相关的其余规范
5. 当前的设计（`docs/superpowers/specs/`）、计划（`docs/superpowers/plans/`）、验收报告（`docs/gates/`）
6. 要改的源码及其上下游

## 维护规则

- 新规范放在本目录；同一条规则只保留一个主版本，其他地方链接过去。
- `README.md` 与本目录冲突时以本目录为准，并同步修正 `README.md`。
- 写进规范的规则必须是代码现在的样子，能指到落实它的代码或测试；还没落地的写明从什么时候起（「M3 起」「M2 Task 12 落地」），不写成已经存在。
- 缺规范时按本项目自己的职责补，不照搬其他仓库的模板。
- 实践与成文规则不一致时，改实践，或先问维护者；不悄悄改规则去迁就实践。
- 已有章节不重新编号：计划和其他文档按节号引用它们。新内容加在现有节里，或作为新的一节加在末尾。
- 规则改动单独提交，规则见 `AI_CODING_GUIDE.md` 第 11 节。
- 示例只用占位符和虚构数据（`<LINEAR_API_KEY>`、`DEMO-1`、`Demo Project`、`Jane Doe`），路径写 `~/…` 或 `<REPO>`，不写本机绝对路径。
- 只改文档或规范时不升版本号。
- 发现疑似密钥只报告文件和行号，不自动改写 git 历史；是否改写由用户决定。
