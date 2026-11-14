# IssueRelay 独立验收流程实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** 在同一工单跟踪原生会话、可选独立验收、失败退回、修复及复验，并读回每轮 Wiki 归档。

**Architecture:** 保留已执行的运行。第一步复用运行 ID、原生会话及更新事件。后续在工作流存储中绑定独立验收与对应开发轮次，经现有确认体系提交，面板和 MCP 共用同一状态。

**Tech Stack:** Rust、SQLite、Tokio、Tauri、React、TypeScript。

**Spec:** [独立验收、退回与原生会话设计](../specs/2026-11-14-issuerelay-acceptance-workflow-design.md)

## 实施顺序

1. 原生会话。覆盖 `executor/outcome.rs`、`engine/workflow_support.rs`、`store.rs`、`ProviderSessionCard.tsx`。先在真实运行中的测试夹具等待 gate 前断言 ID，再存首个有效事件并发通知，终态保留 ID。面板覆盖 Codex、Claude、等待项、无效 UUID、运行中不读终态预览。
2. 验收与退回工作流。用 `workflow_store.rs`、`engine/control_actions.rs` 和调度模块保存来源运行、所验版本、验证运行和用户决定。用现有预览确认和目录互斥开始新会话；需要完整主控宿主接入时采用可回读的原生路径。测试覆盖失败退回、修复新轮次、同目录并发、预览过期、旧版本通过不能推进新版本、取消及测试报告缺失。
3. 桌面与 MCP。将独立验证、退回和复验显示到运行详情、任务时间线和控制工具清单。逐项核主控桌面与聊天使用同一预览权限，五语完整，UI 可识别实际启动和未获确认。两端分别验证能找到对应原生会话和回执。
4. Wiki。已有 `record_archive` 验证本机路径和回执格式，扩充归档状态与阶段关联。测试完成、失败、中断、从未启动、Wiki 根变化、页面消失、归档登记失败及历史数据读取。提交及推送由配置的 Wiki 工作流负责。
5. 构建并跑仓库六项验证。在真实沙箱通过两种执行器分别确认运行、查看实际会话、验收退回、修复复验、归档后收口。明确平台、安装版本和未验项，保留合成证据。
