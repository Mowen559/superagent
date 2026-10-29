# SuperAgent (hub-rs) OpenSpec 规范驱动开发体系

> **定位宗旨**：从“能力堆叠”迈向“可信本地运行时”。
> 本工程已全面接入官方开源标准框架 **`@fission-ai/openspec`**，作为 Rust 内核演化与代码改动的唯一事实源 (SSOT)。

---

## 一、主规格库目录索引 (`openspec/specs/`)

`super-agent` 内部规范全部遵循官方 OpenSpec 标准，通过 `openspec validate --specs` 校验：

| 领域路径 | 规格标识 | 架构宣言 (Manifesto) 对应原则 |
|---|---|---|
| `specs/runtime/durable-run-ledger` | 运行时与账本 | **原则 2 & 3**：收拢 Deep Hub Runtime 消除死副本；Durable Run Ledger 持久化运行账本 |
| `specs/security/capability-guard` | 工作区能力门控 | **原则 4**：Workspace Capability Guard 操作系统硬权限门控与工具网关 |
| `specs/gateway/canonical-llm-gateway` | 标准模型网关 | **原则 5**：Canonical LLM Gateway 唯一适配器与 RTK 源码级压缩 |
| `specs/mcp/mcp-protocol-stack` | MCP 协议总线 | MCP 双向 JSON-RPC 2.0 异步调度、核心工具防覆盖与 30s 熔断超时 |
| `specs/intelligence/code-graph-blast-radius` | 代码图谱与影响面 | AST 增量解析、SQLite WAL 图数据库与 CRITICAL 级爆炸半径阻断 |
| `specs/evals/capability-certification` | 能力认证与评测 | **原则 1**：Capability Certification 能力认证优先与 5 大 Golden Evals 100% 满分交付基线 |

---

## 二、开发流转契约 (Workflow)

1. **提案改动**：在修改任何 Rust 逻辑前，执行 `/opsx-propose "改动描述"`，生成 `proposal.md`、`design.md`、`specs/` 与 `tasks.md`；
2. **人类确认**：确认边界无破坏性改动与安全漏洞；
3. **编码自测**：执行 `/opsx-apply` 逐项实现 `tasks.md`，执行 `cargo test` 与 `node scripts/run_evals.cjs` 确保 100% 通过；
4. **归档合并**：执行 `openspec archive <change-name>`，自动化将新能力规范合并回 `openspec/specs/`。
