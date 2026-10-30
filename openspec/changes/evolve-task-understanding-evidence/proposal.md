# Proposal

## Why

用户希望优先提升复杂任务拆解和主动补齐上下文。当前分支已有追踪、调度与工具策略，但进程成功、清单完成和正文格式仍可能代替实际验收，且追加要求没有稳定的版本与证据绑定。

## What Changes

- 为当前 Run 增加有预算的需求契约、来源明确的待查问题及依赖子目标。
- 允许原子增量修订，保留原约束和未受影响进度，要求受影响步骤重新取证。
- 将规范化工具收据接到顺序、并行和所有结束路径；普通进程、清单与规划结果不能证明验证完成。
- 主动投影真实工作区规则，保留来源和截断事实；继承当前分支的思考等级、追踪、策略与模型调度。
- 提供不联网的规划夹具预览与确定性回归；真实模型评测只能显式启动并记录实际配置。

## Capabilities

### New Capabilities

- `runtime/task-understanding`：需求解释、调查问题、子目标、证据绑定和增量修订。
- `runtime/execution-evidence`：验证新鲜度、完成门禁及有限失败收敛。

### Modified Capabilities

- `evals/capability-certification`：追加需求理解评测的预算、模型身份与结论边界要求。

## Impact

影响原生 AgentLoop、CognitiveRuntime、RunPlan、Tool Gateway、Prelude、工具注册与风险分类；增加评测 example。复用旧功能分支中经过回归的独立机制，按当前分支适配，不整体合并旧架构与提示词。不改 HTTP API，不扩大权限，不自动启动子智能体、外部探测或模型评测。

本轮不实现跨进程规划恢复、自然语言需求语义完备性证明或全部五项 Golden Evals；现有缺失的总评测脚本不能被本次规划夹具冒充已交付。能力保持 Experimental。
