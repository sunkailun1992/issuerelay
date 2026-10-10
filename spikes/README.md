# spikes

一次性验证用的独立 cargo 工程，不属于工作区，不随产品发布。`linear-mcp-probe` 是 M0 用来验证 Linear MCP OAuth 授权与工具返回格式的探针，结论见 `docs/gates/` 的 M0 验收报告。运行时生成的原始输出在 `samples-raw/`（已被 .gitignore 排除），脱敏后才放进 `docs/gates/samples/`。
