# Hub-Rs Architecture Manifesto (2026-08-22)

基于最新的架构审查报告，Hub-Rs 的下一步进化必须从“能力堆叠”走向“可信本地运行时”。请你在后续的自我演化和代码修改中，严格遵循以下架构规范：

## 1. 核心战略：Capability Certification（能力认证优先）
- **现状**：我们有 52 个单元测试，但很多能力（如真实 CLI 执行、MCP 协议、取消、重启）缺乏端到端（E2E）的可信认证。
- **规范**：在增加任何新功能之前，优先构建“能力矩阵（Capability Matrix）”。任何新的插件或功能，都必须具备 Golden Evals（黄金测试标准），产出明确的认证状态（Certified / Experimental / Disabled），并与健康检查（Runtime Health）对齐。

## 2. 收拢 Deep Hub Runtime（深度运行时）
- **规范**：不要让执行语义分散在 HTTP Agent、Supervisor 和 SuperAgent 中。所有的运行逻辑最终都应指向同一个 Deep Hub Runtime 模块。消除无用的“死副本（dead copies）”。

## 3. Durable Run Ledger（持久化运行账本）
- **规范**：将 Task、Worker、Process 句柄、状态机、事件和重启恢复，收敛到一个统一的、可持久化的运行账本中。状态的“真相”必须只有一份。

## 4. Workspace Capability Guard（工作区能力门控）
- **规范**：安全不能仅仅依赖“提示词（Prompt）约束”。所有触达操作系统（OS）的动作（读写文件、Git 操作、Shell 进程），必须统一穿过同一个 Workspace Capability Guard（权限门控模块），实现底层拦截。

## 5. Canonical LLM Gateway（标准 LLM 网关）
- **规范**：消除多处重复的 Provider 路由和复杂的协议转换。Canonical（标准格式）只在网关内部存在，每个提供商（OpenAI, Anthropic, Gemini）只允许存在一个真实的 Adapter。

## 你的行动指南
在执行接下来的底层能力开发（如桌面原生组件、音频挂载）时，请主动将代码设计向以上 5 条规范靠拢。例如：开发新工具前，先写测试用例将其加入“认证矩阵”；调用系统命令时，考虑它是否经过了集中权限控制。
