# Proposal

## Why

用户无法检验 Hub 内置 understand、RTK、Supermemory 及需求正逆向能力。现有组件评测未进入生产 worker。源码显示 worker 的文件工具与 AgentLoop Prelude 可能使用不同工作区，普通记忆召回使用全局库；需求引擎只暴露 HTTP 路由。RTK 已在本地 Shell 输出层使用，但 CLI gain 与进程指标不同，不能据此判定完全未使用。

## What Changes

- 为 AgentLoop 显式绑定当前工作区，统一 Prelude、清单和原生工具上下文；项目记忆按工作区召回。
- 提供原生能力就绪诊断与需求分析/规划工具，复用 understand 和既有 RequirementAnalyzer；规划产物不冒充代码变更或验证。
- 增加可离线运行的 Hub doctor，与真实 worker 执行收据分开报告已注册、就绪、已调用和已验证。
- 使用真正的 hub superagent-worker 完成临时工程正向改码/测试及逆向理解/记忆召回测试，保留失败和实际产物。
- 针对已复现的 Windows 原生索引器中文路径故障，增加仅影响索引进程的 ASCII 路径适配，验证后将真实图谱发布回原项目缓存，不复制或修改业务源码。

## Capabilities

### New Capabilities

- `hub-capability-wiring`: 原生能力接线与运行入口验收。

## Impact

修改内核构造、worker/bin 原生工具注册、Prelude、CLI doctor 与诊断/需求工具，不改现有 HTTP API。普通 ask/chat 仍作为直接对话入口，明确它们不等于工具执行入口；本轮不静默改变其权限和交互。外部索引器缺失时报告不可用，不制造图谱。不自动下载、不修改用户业务工程，不将固定夹具认证泛化到整个项目。
