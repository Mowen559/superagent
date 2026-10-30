# Hub-RS Domain Context

本文定义智能体内核的稳定领域词汇。实现与架构文档应优先使用这些名称，避免同一概念在不同 Module 中漂移。

## 核心概念

- **Session**：可跨多次用户输入恢复的长期对话；由 `SessionLedger` 持久化。
- **Run**：Session 内由一次用户输入触发的有限执行；具有唯一 `run_id` 和一个终态收据。
- **Task**：对外可观察的后台执行句柄；持久化状态、事件和取消信号，不直接持有具体 Agent CLI 进程。
- **Terminal Status**：Task/Agent 执行的规范终态：`completed | failed | cancelled | timeout`；四者不得互相折叠。
- **Tool Call**：模型提出的外部能力调用；必须与一个 `ToolResult` 配对。
- **Tool Receipt**：`ToolExecutionGateway` 返回的规范化执行事实，包括状态、风险、耗时和有界结果。
- **Approval**：绑定 Tool Call 指纹、可过期且只能消费一次的人工授权。
- **Cognitive Runtime**：Run 级决策控制面；分类任务、投影动态策略、累计工具证据并决定草稿能否成为最终答案。
- **Run Plan**：由 Cognitive Runtime 持有的结构化步骤状态；Execution 使用 Inspect → Act → Verify → Respond，Investigative 使用 Evidence → Synthesize → Respond。
- **Replan**：连续执行失败触发的策略修订信号；保留已验证事实，但要求重新检查假设并改变工具或证据来源。
- **Evolution Strategy**：从规范化 Run 结果累计证据的有界策略族；只包含系统定义的行为指导，不包含用户原文、凭据、权限或源码修改。
- **Candidate / Promoted / Rejected**：Evolution Strategy 的选择状态。Candidate 不进入模型上下文；达到成功与失败率门槛后成为 Promoted；无成功且重复失败则成为 Rejected。
- **Completion Review**：Cognitive Runtime 对复杂任务最多触发一次的完成度复核；它不是第二个独立 Agent，也不修改工具事实。
- **Evidence**：工具收据或用户提供内容中可支撑结论的事实；模型推断不能伪装成 Evidence。
- **Compaction Checkpoint**：从旧 Session 事件生成的确定性、有界事实摘要；提交后替代被压缩事件，但不得破坏当前 Run 或 Tool Call/Tool Result 配对。
- **Fusion Runtime**：Hub-RS 对 OpenClaw、Hermes 与 Harness 运行思想的本地收敛；只吸收不变量和深层机制，不复制三套产品外壳。
- **Steering Item**：运行中到达 Session 的纠偏、故障恢复或子智能体结果；必须经过持久化队列租约后才能注入 Run。
- **Steering Lease**：Steering Item 的临时交付所有权；只有 Session Ledger 已记录注入事实后才能 ack，失败必须 release。
- **Subagent Handle**：父 Session 获得的不可变子执行身份，包含 `subagent_id / parent_session_id / correlation_id / depth`；控制操作必须通过该身份定位记录。
- **Subagent Runtime**：拥有子执行准入、谱系、深度/并发配额、超时、取消、进程回收和结果交付的 Module；具体进程协议是其内部 Adapter。
- **Model Attempt**：一次带编号的模型请求尝试；失败必须先分类，再由有限策略决定重试或终止，禁止由调用点自行无界重试。
- **Run Prelude**：Run 在第一次模型调用前生成的一次性准备结果；包含 Capability Route 与有来源、有限长、不可执行的 Knowledge Evidence。
- **Task Contract**：模型通过 `plan_task` 提交的 Run 级需求理解，包括目标解释、验收条件、约束、带依赖的子目标和待查问题；是可修订假设，不是执行或验证证据。
- **Plan Revision**：通过 `revise_task_plan` 提交的、绑定 `base_revision` 的增量修订；追加验收条件/约束/问题，局部更新步骤。未提及的要求、字段和既有依赖保持原样，过期或无效修订不得部分生效。
- **Subgoal Evidence**：当前 Run 的成功 Tool Receipt 与某个子目标的显式绑定；其种类必须符合子目标声明的 `observation | mutation | verification | user_input`。绑定证明工具事实存在，不自动证明模型对业务验收条件的解释正确。
- **Context Resolution**：待查问题与匹配来源收据的绑定；工作区、外部资料与真实用户回应分别核对，不能用规划文字消除事实缺口。
- **Context Need**：Task Contract 中尚待补齐的信息；明确由 `workspace | external | user` 哪类来源解决。先查询可获得的事实，仅对无法自行确定且影响结果的选择询问用户。
- **Capability Route**：根据 Objective 与已注册工具确定的能力使用提示；它不执行写操作，也不能绕过 Tool Policy。
- **Interaction Request**：具有 `request_id`、问题、可选选项、有限等待时间和一个终态响应的用户输入请求。
- **Knowledge Document**：知识语料库中以规范化 Workspace 路径标识的文档；以内容哈希形成版本，相同哈希的重复导入必须幂等。
- **Knowledge Chunk**：从 Knowledge Document 的结构化段落、标题或 PDF 页面生成的稳定检索单元；必须保留 Document、版本、位置与 Citation。
- **Corpus Manifest**：与 Knowledge Chunk 在同一持久化检查点提交的文档清单；记录文档版本、格式、来源、分块和底层 Memory Record 的对应关系。
- **Citation**：Knowledge Evidence 指向原始文档位置的可验证引用；文本使用行范围、DOCX 使用段落范围、PDF 使用页码范围。
- **Canonical Model Request**：Agent 内部唯一的模型请求形态；采用 OpenAI Chat 语义承载消息、工具与预算，但不等同于某个外部 Provider 协议。
- **Provider Route**：针对模型别名、Routing Tags、账户优先级和权重生成的有限 Provider Attempt 顺序；包含解析后的真实模型名，但不得暴露凭据。
- **Provider Attempt Receipt**：一次 Provider 调用的规范化事实；记录账户身份、Provider、模型、耗时和错误分类，不记录 API Key、Refresh Token 或请求正文。
- **Model Requirements**：Canonical Model Request 对工具、图像、流式返回和最小上下文窗口提出的能力约束；能力元数据缺失表示 Unknown，不等于 Unsupported。
- **Provider Health State**：Provider Account 在当前进程内的健康事实，包含连续可恢复失败、延迟均值和 `closed | open | half-open` Circuit 状态；不保存凭据或模型正文。
- **Gateway Eval Report**：对当前模型配置执行的只读、确定性预检；区分 Eligible、Verified 与 Unknown 能力路线，报告重复账户、不可用默认路线和打开的 Circuit，不发起外部模型请求。
- **Model Catalog Snapshot**：由显式控制面请求从某个已配置 Provider Account 拉取并有界规范化的模型目录；只保存模型标识和受限元数据，不保存凭据、请求正文或原始响应正文。
- **Capability Observation**：某个 Provider Account、真实模型和能力 Profile 在特定时间的证据；来源只能是 `catalog | probe`，结论只能是 `verified | unknown | unsupported | failed`，并保留来源而不把目录推断伪装成真实验证。
- **Capability Probe**：由本机控制面显式确认后，对已配置 Account/Model 发起的有限、固定输入真实请求；它不是 Provider Health 的 half-open Probe，也不得由健康检查、Gateway Eval 或普通对话隐式触发。
- **Capability Freshness**：Capability Observation 在 `fresh | expired | unversioned | missing | invalid` 中的时间状态；过期或无法解析的证据只能降级为 Unknown，不能继续作为 Verified 或 Unsupported 路由事实。
- **Recertification Plan**：从已过期 Observation 确定性生成的有限 Account/Model/Profile Probe 清单；生成计划不联网，执行仍必须经过 Capability Probe 的本机控制面授权和显式网络确认。
- **Workspace Capability**：Run 获得的、绑定一个已规范化项目根目录的本地授权；文件、Git、QA、Agent cwd 与知识读取只能消费该能力，调用方提交的路径本身不是授权。
- **Sequenced Task Event**：Task 事件日志中的单调序号事件；历史快照与实时订阅必须在同一同步边界建立，客户端可用序号检测缺口并恢复。
- **Verification Receipt**：由明确质量门禁产生的结构化验证事实；普通进程控制成功不等于代码行为已验证。
- **Project Analysis Receipt**：一次项目分析的规范化结果；绑定项目规范路径与源码指纹，包含图数据库验证、结构概览、文件清单、构建/测试/文档入口、告警和可选 LLM 概览状态。
- **Workspace Readiness**：用户显式打开 Workspace 后生成的有界资格事实；区分可访问目录、可分析代码项目、空目录/非代码目录、图谱是否可用与 Agent 是否可执行。
- **Change Set Receipt**：隔离运行目录与原 Workspace 的结构化差异；逐文件标记 `added | modified | deleted`，只允许经过用户选择、路径校验和验证门禁的条目进入 Apply。
- **Visual Pipeline**：由 Screenshot、OCR、Translation 三个可独立替换阶段组成的显式本地执行链；编排层只能依赖 Provider Interface。
- **Visual Provider**：实现单一视觉阶段稳定契约的内置 Adapter 或外部插件；其身份、能力、配置要求和就绪状态必须可发现。
- **Visual Artifact**：保存在 Hub 数据目录、通过不透明 UUID 引用的截图文件；调用方不得提交任意文件系统路径作为 Artifact。
- **Visual Pipeline Receipt**：记录每个阶段的 Provider、状态、耗时、告警与输出引用的结构化事实，不保存翻译密钥或模型原始凭据。

## 不变量

1. Session 可以包含多个 Run；Run 的认知状态不能泄漏到下一个 Run。
2. Tool Call 在执行前持久化，恢复后不得留下无结果的孤儿调用。
3. Approval 不保存完整工具参数，只保存规范化指纹。
4. Cognitive Runtime 可以要求复核，但每个 Run 最多一次，避免自我反思死循环。
5. 达到模型轮次预算后 Run 必须失败收敛，不能无限推理。
6. Run Plan 的 Inspect、Act、Verify 只能由 Tool Receipt 推进，模型文字不能直接宣告完成。
7. 连续两次执行失败必须触发 Replan；Replan 不得清除已完成步骤。
8. 只有 Promoted Evolution Strategy 可以影响后续 Run；Candidate 和 Rejected 只能用于审计。
9. Evolution Memory 无权修改源码、工具权限和审批策略；自举重载仍必须经过 Tool Policy。
10. Task 终态只能提交一次；取消成功后，任何迟到的 Adapter 结果都不得覆盖 `cancelled`。
11. Adapter 持有并回收子进程；TaskManager 在关闭 SSE 前必须先持久化并广播终态事件。
12. Compaction 必须由真实 token 预算触发，禁止维护独立字符阈值；不得通过递归调用同一 LLM 才能从超限中恢复。
13. Compaction Checkpoint 必须保留当前 Run、配对的 Tool Call/Tool Result 和最近证据；提交失败时原事件不可丢失。
14. Steering Item 必须遵循 `pending → leased → delivered`；崩溃恢复以 Session Ledger 中的注入事实为准，禁止静默丢失或无限重复注入。
15. Subagent 必须具有父 Session、深度和有限超时；子结果通过父 Session 的 Steering Queue 交付，禁止依赖父 Agent 主动轮询才能完成汇聚。
16. Subagent Runtime 必须限制全局和单父并发，终态不可覆盖；取消必须到达并回收子进程。
17. 同一账户的 Model Attempt 只允许对 transient/rate-limit 错误有限退避；账户级鉴权失败不得原地重试，但可按有限 Provider Route 切换账户。上下文预算和永久请求错误必须立即返回给上层恢复机制。
18. Run Prelude 每个 Run 只准备一次；Knowledge Evidence 必须标记为数据而非指令，带来源并受字符预算限制。
19. Interaction Request 在一个 Agent 中最多只能有一个 pending；响应必须按 request-id 关联，超时或错误后不可消费迟到响应。
20. Knowledge Document 只能从 Workspace Guard 允许的真实文件导入；读取大小、分块数量、检索查询和返回数量都必须有硬上限。
21. 相同来源与内容哈希的重复导入不得产生新 Knowledge Chunk；来源内容变化时，旧版本分块、Corpus Manifest 与新版本必须在同一 Store 检查点中原子替换。
22. 每个检索结果必须带总分、关键词分、可用时的向量分与 Citation；Knowledge Chunk 始终是 Evidence 数据，不能作为系统指令执行。
23. 删除或重建必须通过 Corpus Manifest 定位文档及其全部 Knowledge Chunk；失败不得留下半个版本或无清单的孤儿分块。
24. Agent 的消息、工具和预算只能先形成 Canonical Model Request；Anthropic、Gemini、OpenAI-compatible 与 Copilot 的协议差异必须停留在 Provider Adapter Implementation 内。
25. Provider Route 只包含已启用且满足模型与 Routing Tags 的账户；无匹配时只能回退到显式 active account 或可用的顶层配置，尝试次数不得超过 Route 长度。
26. Rate-limit、网络/5xx 和账户级鉴权失败可以进入下一个 Provider Attempt；上下文预算和永久请求错误必须立即收敛。每个 Attempt 必须产生不含凭据的 Receipt。
27. Provider Health 只能由成功、rate-limit、网络/5xx 和账户级鉴权事实改变；上下文溢出和永久请求错误不得污染账户健康。Circuit 打开后在冷却期内拒绝流量，冷却结束只能放行一个 half-open Probe。
28. Provider Route 必须满足 Model Requirements；能力元数据显式为 false 或上下文上限明确不足时必须排除，Unknown 保持兼容但不得被宣称为已验证能力。
29. Provider Health State 是进程级、有限大小且可自动恢复的运行事实；重启可回到 closed，不得把请求正文、响应正文、API Key 或 Refresh Token 写入健康记录。
30. Gateway Eval 只能读取配置与 Provider Health Snapshot，禁止为了“验证”而调用外部模型；Unknown 能力必须与显式 Verified 分开呈现，预检报告不得包含凭据或请求正文。
31. Model Catalog 与 Capability Probe 只能由带本机控制面授权和显式网络确认的请求触发；目标 URL 必须从既有 Provider Account 派生，调用方不得提交任意 URL、API Key 或请求正文。
32. 能力发现必须限制超时、响应字节、模型数量、Profile 数量、并发和 Probe 输出预算；健康检查、Gateway Eval 与普通 Agent Run 不得产生发现流量或额外模型费用。
33. Catalog 元数据只能形成带来源的 Observation；只有明确 Provider 元数据或对该 Account/Model/Profile 成功完成的 Capability Probe 才能写入 Verified，鉴权、限流、网络和 5xx 失败不得被记录为 Unsupported。
34. 能力发现不得记录或返回 API Key、Refresh Token、固定 Probe 的请求/响应正文；配置更新必须在全部结果规范化后原子提交，提交失败时旧配置保持可读且不被部分覆盖。
35. Catalog 与 Probe Observation 必须包含 `observedAt` 和有限 `expiresAt`；Probe 默认有效期长于 Catalog，TTL 必须有系统默认值和有界配置，调用方不得自行决定证据是否新鲜。
36. Fresh Observation 才能产生 Verified 或 Unsupported；Expired/Invalid Observation 必须降级为 Unknown。过期 Unsupported 不得永久排除模型，过期 Verified 不得继续被 Gateway Eval 计为已验证。
37. Recertification Plan 只能选择已有、已过期且可安全探测的 Account/Model/Profile，受单次 Probe 总量限制；失败 Observation 不得覆盖最近一次仍有效的能力事实。
38. 自动再认证只能自动选择目标，不能自动获得网络权限；健康检查、Gateway Eval、普通 Agent Run 和后台定时器不得隐式执行 Recertification Plan。
39. 调用方提交的绝对路径、`project` 字符串或进程 `current_dir` 不得自行成为 Workspace 授权；所有路径型 HTTP Interface 与原生 Agent 工具必须从既有 Workspace Capability 解析目标。
40. 本地 Web/IDE 控制面默认只监听 loopback；任何可读文件、写文件、执行命令、应用 Patch 或读取凭据的 Interface 都必须经过启动期会话授权。分发包和构建产物不得包含可用 Provider 凭据。
41. Task 历史快照与实时事件订阅必须原子建立；每个事件具有 Task 内单调序号，断连、lag 或重连后必须能从最后确认序号继续，禁止静默跳过。
42. Execution Run 的 Verify 只能由 Verification Receipt 推进；普通 Shell、进程启动、目录读取或无断言命令不得关闭 Verify 步骤。
43. Native Brain、外部 Agent Adapter、HTTP 与 CLI 必须消费同一个 Run Workspace；`--project` 不得在 Adapter Seam 丢失，也不得通过修改进程全局 cwd 影响其他并发 Run。
44. Project Analysis Task 只有在索引进程成功、图数据库 Schema 可读且 Project Analysis Receipt 已持久化后才能进入 `completed`；索引进程退出码本身不是完成证据。
45. Project Analysis Receipt 必须记录有界源码指纹和扫描截断事实；可选 LLM 概览失败只能形成告警，不能抹去确定性图谱与清单结果，也不能被宣称为已生成。
46. `/api/projects/analyze` 与 `/api/workspaces/analyze` 必须使用进程内共享 TaskManager；取消、终态、事件流和结果读取不得因临时 TaskManager 分叉。
47. Workspace 控制面必须在用户显式 Open 后登记规范路径；分析、Agent cwd、知识文档与 Apply 只能消费已打开 Workspace 或受管项目根目录，不得把请求中的任意绝对路径直接当成授权。
48. Agent Change Set 只能在 Task 进入成功终态后生成；Apply 只能处理 Change Set Receipt 中的规范相对路径，必须正确区分新增、修改和删除，禁止通过遍历隔离目录覆盖整个原 Workspace。
49. Apply 默认要求成功的 Verification Receipt；无可用验证计划或验证失败时，只能由用户显式 `force`，并在 Apply Receipt 与超级记忆中记录绕过事实。
50. Workspace Intelligence UI 必须在切换 Workspace 时自动 Open 并加载快照；Task 状态、事件序号、交互请求、取消、Receipt、知识 Citation 和 Apply 结果不得只存在于日志或 Toast 中。
51. Visual Pipeline 的三个阶段必须分别经过可发现的 Provider Interface；内置 SDK 与外部命令插件不得泄漏到路由或前端契约。
52. 截图、OCR 和联网翻译只能由用户显式请求触发；能力发现、就绪检查和页面加载不得截图、上传 Artifact 或产生外部模型费用。
53. Visual Artifact 只能从 Hub 数据目录按 UUID 解析，具有数量和生命周期上限；HTTP 请求不得提供任意图片绝对路径。
54. 视觉设置只能持久化 Provider 身份与非秘密配置。DeepL/Google 密钥只能从环境或安全凭据存储读取，LLM 翻译必须复用 Canonical Model Gateway。
55. OCR 模型不得由运行时隐式下载；模型缺失必须表现为 Provider `notReady` 并返回可操作的配置要求。
56. Task Contract 只能在当前 Run 中更新和投影；依赖必须引用更早的唯一子目标，必须具有可观察的验收条件。规划和 Todo 更新不得推进 Inspect、Act 或 Verify，也不得替代原始用户目标。
57. Run Prelude 的工作区上下文只能来自当前 Run Workspace 的实际文件；摘录必须带路径、行号和截断状态，不得由文件名推断框架、版本或架构规则，也不得回退读取无关进程目录。
58. Verification 必须基于前台质量命令的成功退出码；普通工具 `Ok(Value)`、后台进程启动、文件读取或帮助输出不能成为验证事实。后续写操作或失败的进程检查使已有 Verify 失效。
59. 完成复核预算最多一次；复核后仍缺执行证据必须失败收敛，Fast 模式不能绕过证据门禁，失败结果不得作为成功策略学习。
60. 子目标完成必须通过 `update_task_progress` 引用当前 Run 已观察到的成功收据；未知、失败、跨 Run、规划、Todo 和进度更新结果不得充当子目标证据。
61. 子目标依赖必须先获得有效证据；后续写操作或失败的进程检查使旧验证证据失效，依赖该验证的子目标不能继续被计为完成。
62. 完整替换 Task Contract 会清除原进度和问题绑定，并使旧验证失效；增量 Plan Revision 保留未受影响的进度和事实，只使变更步骤及相关依赖失效。成功收据缓存上限 256，已绑定收据应优先保留；每个绑定最多引用 8 个收据。
63. 执行型 Run 声明的子目标或待查问题仍有证据缺口时，不得通过完成门禁，Evolution Report 也不得将执行计划记为闭合。明确以“只做规划/只给方案”等范围指令开始的 Run 只交付提案；未来实施步骤不要求本轮执行，也不得被宣称已执行。基于现有仓库的提案仍需调查证据。
64. 真实模型评测必须显式调用，固定请求配置和输出预算，仅使用合成工作区资料；缺少实际模型收据时不能宣称同模型对照，词面覆盖率不能冒充业务成功率。
65. Plan Revision 必须先合并、校验和计算影响，再一次提交；版本冲突、未知依赖、循环或超出预算均不得改变计划、证据绑定和版本。相同内容的增量为空操作，不推进版本。
66. Task Contract 的 Schema 与运行时边界由同一模块维护：最多 12 个步骤、12 条验收条件、16 条约束、8 个待查问题，规范化 JSON 总计最多 12,000 字符。校验不得通过静默截断或删除要求来达标。
67. 变更步骤及其后续依赖需要修订后产生的新证据；新增要求使旧验证失效，并重开 Run 的 Verify。修订是元数据，不属于代码变更证据，也不能绕过审批和工作区能力边界。
