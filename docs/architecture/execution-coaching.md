# 执行对话与工作循环校准

日期：2026-10-30。规范：[strengthen-work-cycle-coaching](../../openspec/changes/strengthen-work-cycle-coaching/design.md)。

## 运行时改进

工作区观察工具的重复调用身份加入状态代次。写入后相同路径可以再次读取；失败写入也可能部分落盘，因此允许重新观察。清单更新不重置观察预算；不变状态的重复调用仍受原有连续/窗口上限保护。网络、用户交互和写工具不享有重读豁免。

验证支持 `node --test`、`python -m unittest`、`python3 -m unittest` 和单层 `rtk` / `rtk proxy` 包装的既有质量命令。实际前台成功退出仍是必要条件；后台、组合命令、帮助、仅收集及嵌套包装不成为验证事实。识别质量命令不授予执行权限，也不保证工具已安装。

失败提示只提供有依据的下一步：缺少 JavaScript 全局变量先确认 Node/SSR/Worker/沙箱环境；权限错误先确认本地 ACL 或远端授权；HTTP 防护错误先查看响应及已有 Provider。不得从普通错误推断特定业务域或要求探测硬编码服务。

## 可复现执行对话

```powershell
cargo run --locked --example execution_coaching
cargo run --locked --example execution_coaching -- --live --case private-routing --output outputs/execution-coaching-private.json
cargo run --locked --example execution_coaching -- --live --case public-capacity --output outputs/execution-coaching-public.json
```

默认离线预览不加载模型设置。`--live` 使用既有 Canonical Model Gateway，固定温度 0，最多 1536 输出 Token/请求、10 请求/案例、8 工具/轮、60 秒/模型请求；服务商错误停止整个批次。两个案例共最多 20 次模型请求，不代表账户内部有限 failover 尝试次数。

每案例使用新的临时目录，模型仅可读取 `README.md`、`settings.json`，写入最多 4096 字节的 JSON 对象，以及运行唯一固定命令 `node --test acceptance.test.cjs`。验收脚本由评测方持有、不可由模型读取/改写，执行最多等待 5 秒。不会执行模型生成的程序、任意 shell 或网络工具。

两次用户输入分别要求修复配置和追加参数；必须保留中文备注、嵌套扩展和无关字段。第二次输入建立新的 Run 级认知与收据状态；模型仍可看见同一对话的历史，但旧工具事实不能完成新 Run。完成门禁拒绝过早交付时，将具体缺口作为对话反馈，最多一次纠正。每阶段交付后执行独立验收；该复验不作为模型已调用验证的证据。

报告保存合成对话、工具状态、验证事实、原始夹具指纹、实际模型身份、Token 用量和耗时。不保存模型设置、凭据或 Provider 错误原文。不自动写入 EvolutionMemory；对话纠正不等于权重训练或长期学习。

## 结论边界

这是 `CognitiveRuntime + ToolExecutionGateway + Canonical Gateway` 的组件执行评测。它不覆盖完整 AgentLoop、动态 MCP、技能、压缩/黑板、真实代码修改或五项 Golden Evals，不据此宣称整体智能提高了某个百分比。两个合成案例均通过也只能证明对应配置执行闭环。

## 本轮验证

2026-10-30：`cargo test --lib --locked --quiet -- --skip domains::code_review::tests::test_review_current_project_workspace` 为 **498 通过、0 失败、2 忽略、1 排除**。排除既有慢速全项目审查，与上轮基线一致。新增 8 项回归，包括状态改变后的重读、元数据不能重置预算、部分写失败后的重新验证、质量命令边界、诊断来源边界和 4 项执行评测回归。

首次真实模型尝试使用既有配置，所请求模型为 `claude-sonnet-4-6`，端点为 `http://127.0.0.1:8045`。首轮返回 `provider_error`，零工具调用、零成功阶段、未获得实际模型身份；本机端口未发现监听，额外 TCP 连接检查未成功。该次真实模型对话未完成，不能作为效果成绩。原始有界报告见[真实请求失败记录](../reviews/2026-10-30-execution-coaching-live.json)。未修改模型配置或自动切换其他账户。

确定性测试中，预设模型先产生过早交付，收到完成门禁反馈后实际写入合成文件并通过验收；这验证的是纠偏机制，不是一次成功的真实模型教学。不向长期经验库灌入这组脚本结果。

`cargo check --locked --bins --example execution_coaching --example task_understanding_eval --quiet` 通过。新 example 离线运行成功，输出 2 个案例、每案例 2 次用户输入、最多 20 次模型请求的预算，并确认未请求模型、未加载设置。`openspec validate --all --strict` 为 8 项通过、0 失败；`git diff --check` 通过。没有重建或替换用户已安装的生产二进制，也没有自动归档 OpenSpec 变更。

## Gemini 真实测试跟进

用户显式启用 `gemini-3.8-flash-high` 后，通过相同本机网关执行了两组真实对话。密钥只作为子进程环境变量使用，读取隔离的默认设置，不修改用户持久模型配置。

初测结果：[执行对话原始报告](../reviews/2026-10-30-execution-coaching-gemini-initial.json)。`private-routing` 与 `public-capacity` 都完成两次用户输入，每案例 7 次模型请求，合计 13 次实际工具调用，4 个模型发起的验证收据及 4 次独立阶段复验均通过；没有工具失败或完成门禁纠正。分别耗时 46.210 秒、45.254 秒。网关路由记录为 `anthropic:gemini-3.8-flash-high`，其中 anthropic 表示本机代理使用的协议。

随后进行 4 轮预算的追加约束规划对照：[规划初测报告](../reviews/2026-10-30-planning-coaching-gemini.json)。基础模式在 4 轮内提交了修订计划；增强模式连续 4 轮只读取文件，未提交计划，按 `turn_budget_exhausted` 记录。原需求文字逐字保留指标不是语义完整性证明，不把基础模式的改写自动判为遗漏。

根据该失败补充预算感知引导：认知运行时显示真实剩余轮次，外部宿主只能降低上限；两个规划变体获知相同的截止约束与追加要求预留轮次，并被引导批量读取及复用已提供事实。评测入口现在会对未完成的计划流程非零退出，不再仅对 Provider 错误退出失败。复测仍使用原模型、每请求最多 1536 输出 Token 和每变体 4 轮，不增加预算。

复测完成：基础与增强规划模式均在 3 轮内交付；增强模式使用一次成功的 `revise_task_plan` 并保留原条款。私有配置执行案例重新通过两阶段独立复验，7 轮、6 次工具调用、零失败。本轮全库回归更新为 **499 通过、0 失败、2 忽略、1 排除**；两个 example 构建、两个二进制检查与 8 项 OpenSpec 严格校验均通过。详见[Gemini 真实测试报告与原始记录](../reviews/2026-10-30-superagent-gemini-coaching.md)。
