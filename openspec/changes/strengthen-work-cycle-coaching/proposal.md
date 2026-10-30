# Proposal

## Why

规划夹具不能衡量执行闭环。当前工具循环按相同读参数计数，即使工作区已改变仍可能阻断必要重读；验证识别遗漏 Node 原生测试、Python unittest 与 RTK 包装的质量命令，造成有效执行无法结题。

## What Changes

- 将工作区观察的重复身份绑定工作区代次，保留未改变状态的循环限制。
- 有界识别常见质量命令与单层 RTK 包装，继续拒绝后台、帮助、组合命令和失败退出码。
- 失败诊断只提出有证据的下一步，不再把普通权限错误推断为反爬，也不把所有缺失 DOM 全局变量归因为海阔沙箱。
- 增加显式、有限的执行对话评测：真实临时 JSON 文件、不可修改的 Node 验收测试、追加需求和独立最终复验。
- 用实际失败反馈进行对话纠正，记录模型身份、用量、门禁、修改与复验结果；离线回归不得冒充真实模型能力。
- 根据真实 Gemini 对照中的过度调查失败，投影实际轮次预算、引导批量读取与适时交付；评测流程未完成时退出失败，不以请求成功冒充任务成功。

## Capabilities

### New Capabilities

- `work-cycle-coaching`: 有界执行对话与追加要求验收。

### Modified Capabilities

无外部 HTTP 契约变更。

## Impact

仅影响工具循环、验证识别、评测模块及 example。复用 Canonical Gateway、ToolExecutionGateway、CognitiveRuntime 与规划元工具。不扩大权限，不运行任意模型生成程序，不修改用户工作区，不自动归档。评测覆盖组件闭环，不宣称完整 AgentLoop 或五项 Golden Evals 认证。
