# Proposal

## Why

Hub 已有模型、技能、图谱、记忆和工具执行组件，但现有实测主要证明小工程修复、合成技能产物及预写应用探针可运行，尚未证明能从用户自然请求独立生成并验证真实业务规则。此次提升以可复跑的业务交付为核心，解决需求理解、能力选择、实施、验证、纠错和经验复用之间的断点。

## What Changes

2026-11-03用户明确纠正交付目标：应用逆向后应整理真实远端接口及签名/解密为独立JS规则，运行时不依赖Python、原应用DLL或本地HTTP服务。此前v21仅通过原生桥接切片验收，保留其真实证据，不再将其作为用户最终规则目标已完成的结论。P2增补协议还原及独立运行验收。

- 在现有 TaskContract、TaskProgress、工具收据和 SessionLedger 上建立交付证据契约，区分观察、变更、验证、模型推断与实际可用范围。
- 将“动漫共和国安装应用转换”为首个真实业务纵向切片：Hub 读取技能、调查现有接口，在隔离输出目录亲自编写规则和必要的本地适配层，再通过真实规则执行器和无界面媒体验证交付；已有探针允许复用，但必须披露其作者和范围。
- 补齐技能资源、依赖、产物、领域验证器和失败恢复契约；验证规则不再仅依赖命令名称或退出码。
- 让原生图谱、外部 Understand、Supermemory 和 RTK 按任务需要参与，带来源、作用域和版本证据，分别认证，并通过对照任务检验收益。
- 建立固定样本、独立裁判、首次尝试与辅导后表现分离的评测；保留既有五项 Golden Evals 全通过发布要求，增加真实业务认证门槛。
- 最后实施受控进化：经验候选经留出集验证才能晋升；代码自修改必须走隔离候选、验证、切换和回退，不能直接覆盖当前运行工作区。

本提案只制定计划，不实现代码、不修改运行配置、不启动新一轮付费模型测试。后续实施不依赖桌面操作；不扩展为界面重做、模型权重训练、自动外发、任意站点兼容或手机端未经实测的可用承诺。

## Capabilities

### New Capabilities

- `runtime/delivery-evidence`：自然需求到交付产物的证据绑定、领域验收、完成判定和失败恢复。
- `skills/execution-contract`：技能资源与依赖解析、产物声明、独立验证及技能版本晋升。
- `intelligence/context-grounding`：图谱、理解、记忆与终端压缩的作用域、来源、时效及可衡量参与。
- `runtime/controlled-evolution`：纠错经验和代码候选的隔离验证、晋升、回退及运行连续性。

### Modified Capabilities

- `evals/capability-certification`：扩展防伪造验收到真实产物和运行时；增加自然业务任务、独立裁判与首次尝试/辅导后分栏记录。保留原有五项 Golden Evals 的 100% 发布门槛。

## Impact

- 主要实现区域：`src/services/brain/` 的 worker、认知运行时、TaskContract、TaskProgress、执行网关和工具结果存储；`src/domains/agent_cognitive/` 的执行器与技能发现；原生能力工具、项目图谱、Supermemory、技能结晶及 supervisor/request_reload。
- 边缘规则区域：`skills/local-app-inspector/`、规则工程技能、DeepCloud 现有规则执行器与本地引擎接口。先复用现有端侧职责，不把私有凭据或应用数据移到云端。
- 接线与发行：既有 SDK、LocalGateway、`/api/agents/run`、SSE、Sidecar 和随包技能资源；先文档化增量字段，再实现兼容消费。`hub ask/chat` 的普通对话语义单独说明。
- 验证资产：现有 worker/SDK 实测脚本、Golden Evals、独立产物检查、固定业务样本和留出样本。既有失败报告保持不可覆盖。
- 详细阶段、估算、文件落点、风险与验收见 `design.md`，实施清单见 `tasks.md`。本次不归档旧变更，不覆盖其他未提交修改。
