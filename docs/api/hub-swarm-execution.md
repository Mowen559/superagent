# Hub蜂群执行增量契约（P8，2026-11-06）

范围：Rust Hub现有RuFlo适配HTTP接口，保留既有JSON主体风格及URL；Java微服务Result契约不在此变更。本文保留A阶段全局真实性保护，并在B阶段增补父Run受管执行绑定字段。未发现当前deepcloud-ui/src与packages的TypeScript直接消费这些路径，后续消费者必须按本文区分注册/执行/已验。

## A阶段状态读取

- GET /api/swarm/status：status=metadata_only，managerReady=true、executionReady=false，activeAgents=0表示当前未接入的蜂群运行时无已观测执行节点；registeredAgents/totalAgents为角色数。nodes中status=registered、executionState=unavailable，不把目录idle当进程已就绪。
- GET /api/swarm/health：HTTP 200，health=unverified、healthy=false，managerReady=true、executionReady=false；保留node_count/active_count/timestamp，active_count=0，reason说明未绑定协调器。
- GET /api/swarm/agents及/{id}/status：保留角色与模型配置，status=registered、executionReady=false。未知ID保留明确未找到的响应。
- GET /api/swarm/agents/{id}/health：存在仅表示registered，不返回healthy；未知节点unknown/offline。运行健康须B阶段真实PID/心跳及任务状态证据。
- GET /api/system/capabilities和/api/plugins：蜂群与工作流执行未就绪，角色目录和消息结构的可用性不算执行认证。其他未改功能保持原行为，不据本次认证全部插件。

## A阶段执行请求

POST /api/swarm/agents/spawn仍接收type/name及可选model/capabilities，但未接协调器时HTTP 501：

```json
{"ok":false,"error_code":"swarm_execution_unavailable","operation":"spawn","executionStarted":false,"executionReady":false,"error":"Swarm execution runtime is not connected"}
```

不登记新角色、不启动进程。POST /api/swarm/shutdown同样501，operation=shutdown，不声称已经停止进程。未来注册角色与真正启动分开实现，不把旧登记行为叫执行成功。

POST /api/workflows/{id}/execute：已知工作流无执行器返回HTTP 501及ok=false、error_code=workflow_execution_unavailable，步骤、阶段、输出、索引和时间戳不变；未知ID返回404及workflow_not_found。空定义也不得自动Succeeded。POST /api/workflows仅创建待执行定义，预置模板所有步骤Pending且无输出；创建定义不等于执行。

POST /api/swarm/init仍更新目录配置，增加executionStarted=false/executionReady=false。POST /api/swarm/tasks/enqueue只证明入队，返回state=queued、executionStarted=false；队列统计保持实际值。

## 内存黑板

/api/hive-mind/status报告status=memory_only、protocol=local_in_memory、durable=false，不声明已实现Raft。/api/memory与stats增加storage=process_memory、durable=false；该存储不是跨Run持久化super_memory。B/C阶段若新增持久化，须提供真实重启证据和来源映射后再修改durable。

## B阶段约束

后续真实执行须关联parentRunId、parentSessionId、来源项目、工作区/隔离模式、角色权限、实际PID、子任务ID与验收收据。准备接线不能解除A阶段保护；只有相应路径实际接入并过正反例，才逐项取消501。子模型文字不得产生success，父终态由现有Run账本及领域验证统一裁决。

### B1执行器基础（先于管理API启用）

模型工具invoke_subagent与两种worker入口共用同一注册函数；parentSessionId来自实际SessionLedger，parentRunId在AgentLoop开始Run时绑定，禁止模型传入另一执行根。未绑定活动Run、跨父身份、未知隔离模式或递归委派均拒绝，每父最多2个活动子任务。幂等键作用域为父Run，同键不同输入拒绝，旧Run结果不能充当新Run任务。

子任务快照增量字段：parentRunId、workspacePath、originProject、toolPolicy、processId、executionWorkspace、lastHeartbeatAt、processReaped；processId仅在实际spawn后填充，业务验证另行裁决。工作区路径规范化；worktree创建失败明确Failed，禁止回退，已创建worktree保留以供后续指纹验收和冲突检查，不自动合并或删除成果。reviewer/researcher/investigator及其中文审查/调查角色强制plan策略，其他角色继承父策略，未分类角色不提升权限。

默认子进程使用当前Hub的superagent-worker入口及父模型，免除运行现场cargo编译依赖。显式HUB_SUPERAGENT_BIN仍支持旧IPC入口并继承相同环境守卫。父任务结束前回收所属活动子进程；未结束子任务不得随父任务文字返回而被认定已完成。当前管理API仍保持A阶段501，B1测试不等于9.5～9.10全量验收，未验证重启恢复、工作流自动推进、产物合并及真实双模型业务。

### B2父Run控制通道（实施契约）

新增GET/POST `/api/swarm/runs/{taskId}/children` 和 POST `/api/swarm/runs/{taskId}/children/{childId}/cancel`。仅接受当前TaskManager管理、Running且已有真实worker绑定事件的superagent父任务；未知父404、非活动/尚未绑定/控制已断开409。POST请求为`{role,prompt,correlationId,workspaceMode?,timeoutSeconds?}`，correlationId必填，输入有界；不接受客户端替换父工作区、来源、模型、权限、Run或Session。返回202仅表示控制消息入队，executionStarted=false，实际接收/拒绝须在GET的controlReceipts查询，按correlationId关联。重发同键由实际runtime幂等裁决。

GET返回parent绑定及children快照、controlReceipts；statusSource=worker_events表明它是现有任务事件流投影，子进程状态仍归SubagentRuntime，父终态仍归TaskManager/SessionLedger。父任务终止后保留最后已观测快照，parentActive=false，observedOnly=true，不声称子任务仍运行或已被恢复。角色登记及旧无父spawn/shutdown仍501，工作流execute仍501；本接口不启用全局蜂群或隐式消费队列。事件使用现有AgentMessage::Log（level=subagent）承载版本1受管worker JSON，不另建竞争终态表。

## 版本与验证

本增量有意将原来占位HTTP 200成功改为501/404。旧主要字段保留或在失败体中提供ok=false；客户端须使用HTTP状态及error_code处理失败，不继续展示执行完成。验证使用实际Axum路由，覆盖默认角色、假健康、启动未执行、关闭未执行、模板/空流程不变、未知ID和黑板存储范围；另保留旧二进制实际HTTP负例快照，不把A阶段测试称为真实蜂群协作通过。

## 2026-11-06 B2候选实施状态
父Run控制通道已接入候选程序，旧无父spawn/shutdown及全局工作流保护保持501。受控模型桩的真实HTTP→父worker stdin→SubagentRuntime→同一Hub子worker→事件投影共30次HTTP观察通过：两子进程完成、只读写入拒绝、同键重发不重复启动、第三任务取消回收及父终态后拒绝。snapshot.businessVerified固定false，worker结束不代表业务通过。最新全库621通过/0失败/2既有忽略/1既有全仓扫描排除，原17请求全局保护回归通过。候选尚未替换默认安装；真实gemini协作另存结果，不由受控模型桩推导智能性。

B2续修契约：实际发送给父模型的schema必须同时包含invoke_subagent、check_worker_status、intervene_worker、cancel_subagent，不能仅注册后被核心工具白名单隐藏；叶子worker不暴露这四项。check_worker_status支持可选WaitSeconds整数0～30，在同一个所有权范围内有界等待终态，不能借等待改变状态或证明业务通过。普通工具60秒超时保护保持不变。受控模型桩须从真实HTTP模型请求验证工具可见性，不能仅检查注册表。

P8-B只读守卫补强：plan模式下run_command一律按ProcessControl拒绝，不再依据echo/git等前缀推断只读（重定向、删除分支及构建命令可有写入副作用）。只读子任务使用受限文件/图谱读取工具，其他父工作模式保持原策略；这属于工具网关权限，不宣称操作系统沙箱。用实际模型桩发出echo重定向负例并核对文件不存在，正向view_file仍须成功。

P8-B隔离根守卫：worktree模式须有父执行根自身的.git入口，且git rev-parse --show-toplevel的规范化路径等于父执行根；禁止沿目录向上误用宿主仓库。父Run复制目录不是Git根时明确失败，不从origin/宿主HEAD替换当前用户代码。本增量尚未提供非Git执行根的独立快照基座及产物合并，因此9.6继续保持未完成。

父任务只读范围由既有CognitiveRuntime的宿主判定在Run准入时冻结，parent.childReadOnly=true时所有子角色强制plan，不能因自由命名角色而继承yolo。实施任务中的review/research/investigate/audit/inspect/analysis及中文审查/调查/审计/分析/只读角色同样降为plan。该能力沿用有界请求解析，不宣称任意自然语言权限理解。

只读范围解析补例：未引用的请求以独立子句“不要修改文件/代码”“不要修改任何文件/代码”等通用禁止修改指令收尾时，当前阶段按调查处理；指定某个文件、common等局部保护及代码/引号内的同名文字不作为全局只读。若后面还有实际实施请求，不由前一条禁改文字覆盖新的末尾指令。保留原生解析负例并验证父Run实际绑定字段。

## 本轮最终候选验收
最终候选0d7e857b…c5d7d：626项回归通过/0失败/2原忽略/1原全仓扫描排除，受控模型桩31次HTTP观察通过，父childReadOnly=true、自由角色plan、写文件和shell拒绝，worktree祖先根拒绝且无子PID。此处是受控执行契约认证，businessVerified仍false。真实模型两次父任务均未完成：首轮429无委派，提醒后第二轮实际启动两子任务但900秒timeout；OS实查子进程均已回收，最后running投影仅observedOnly。全局17项保护曾在ea57fd05候选通过，最终候选的额外同组复跑因自动审批拒绝启动服务（blocked by policy）未执行，不回填通过。候选未安装，下一项仍为9.6快照/冲突检查及9.7终态对账。

## B3隔离产物与持久生命周期（9.6/9.7实施契约）

worktree模式改为程序管理目录内建立父执行根**当前内容**的独立Git快照基座，再创建工作树；不使用来源仓库HEAD，不更改原仓库Git元数据。沿用现有copy_workspace/capture_baseline的代码文件过滤与上限，复制前后及基座文件指纹必须一致；失败明确失败，禁止降级共享根。所有权绑定父Session/Run、子ID、父执行根和origin；快照、所有权收据、未跟踪新文件及完整成果保留。

基座只复制capture_baseline冻结清单中的文件，每个文件按冻结字节上限读取并核对SHA-256；二进制、大资源、生成目录及其他被现有代码扫描器排除的内容不会暗中进入Git基座。快照因此是代码范围，不是安装应用/依赖目录的全量副本；需要额外资源时必须明确恢复依赖，不以缺资源时的进程失败伪报完成。

新增模型工具collect_subagent_changes：Action=inspect返回版本1变更清单、冲突和proposalDigest；Action=apply须携带ExpectedDigest，核对子进程Succeeded且reaped、当前父Run、只读范围/父工具权限、全部路径归属、冻结基线及当前成果指纹，复用ChangeSet冲突与回滚机制。父任务受管回执等保护文件仍不可覆盖；不自动将成果写入来源项目，apply只写当前父执行根，businessVerified保持false。相同产物重复apply只返回原收据，不重新覆盖；源成果变化、父同文件变更、跨Run/链接逃逸及失败审查均不得合并。

管理接口新增POST `/api/swarm/runs/{taskId}/children/{childId}/changes`，请求`{action:"inspect"|"apply",expectedDigest?:"..."}`，以现有父控制通道排队，结果从controlReceipts读取；202不代表合并完成。客户端不能指定任意目录或force。

子生命周期写入程序管理的Session级追加账本，父SessionLedger继续唯一裁决父Run；账本持有独占会话租约，启动/心跳/终态/回传关联同一子ID。恢复不重启历史子任务、不自动重放合并。失去父进程且无终态收据的子任务明确标失败/取消/超时，并区分OS已确认回收与清理未验证；PID复用不作为旧进程存活，不按裸PID杀进程。受管父进程回收后将对账帧加入原TaskManager事件流，消除仅留下running的展示；失败对账明确报错，不伪造healthy或businessVerified。

### B3验收结果与历史条款覆盖

B3的当前代码独立基座替代上文B2“父执行根必须已有.git，否则失败”的临时限制；仍禁止借用祖先仓库。B2原失败及审批记录保留。
2026-11-06 B3验收：9.6/9.7完成，计划62/106。当前代码独立快照、指纹绑定inspect/apply、来源/冲突/保护文件守卫、幂等收集及独占子账本落地；父退出后明确对账失败/取消/超时，不重跑任务或重放合并。功能代码636项回归通过/0失败/2原忽略/1原过滤；末次工具说明修正19项针对测试通过，重构建候选e5fb57ec…8765b的控制HTTP68次及交付HTTP202次观察通过（含轮询，不是断言数量），冻结17请求保护复验通过。真实模型调用0，受控模型桩不认证智能性；默认程序仍4021fbc5未替换。证据docs/reviews/hub-swarm-execution-review-v3.json。9.8～9.20保持待验。

## B4父Run收据工作流（9.8实施契约）
TaskProgress增加delegation投影，阶段来自实际子进程、合并和父验证收据，不能由模型更新状态。只读成功子任务不认证修改；失败/取消/超时或未回收阻止结题，隔离改动须显式收集；有修改时要求父Run前台成功质量检查且执行前后代码指纹一致、当前仍一致。已有业务验证器仍负责业务认证。code_review关键问题须真实复审消除，不因后续成功文字或普通测试放行；重复/无改动收集不增加修改证据。既有全局工作流未绑定执行器时维持501，当前接入的是父Run的TaskContract依赖工作流。

2026-11-07 B4候选切片：父TaskProgress投影实际子状态，隔离成果须收集，有改动须父Run真实前台验证且执行前后及当前代码指纹一致；关键审查问题不能由无关测试/空审查/不同文件审查清除。重复及无改动收集不生成新修改证据，管理API真实合并同样使旧依赖验证与物理交付收据失效。640项全库回归通过/0失败/2原忽略/1原过滤，真实子进程+Node验证针对测试、原四组HTTP场景与17请求保护通过；严格规范校验通过。候选dd41d5bb1bdc未安装，模型调用0；9.8尚缺普通父Run完整结题HTTP正反例，保持未勾选，整体62/106。证据docs/reviews/hub-swarm-execution-review-v4.json。

2026-11-07 B5复验发现worker缺少code_review注册：补齐受管worker与CLI同一审查工具，按只读工具守卫，指定文件限执行根内且有界；端到端须取得实际critical_count关键发现，工具缺失仅算不可用拒绝，不能充当审查业务通过。

2026-11-07 B5验收：加强断言发现worker漏注册code_review，原工具不可用拒绝不计关键发现成功；失败目录completion-final保留。补齐执行根审查工具及只读策略，显式路径限工作区内。640项回归通过/0失败/2原忽略/1原过滤；普通父Run四组实际结题HTTP通过：正例completed，未合并、真实critical审查、检查后变更均failed且原因匹配，取得实际父指纹收据。9.8完成，63/106；9.9真实Gemini双子任务另记，候选未安装。证据docs/reviews/hub-swarm-execution-review-v5.json。

2026-11-08 B6故障诊断契约：Provider Route失败收据增加可选diagnosticCode；仅识别结构化代理错误中明确的upstream_network_error，保留HTTP状态、原outcome及有界重试。不得把普通502自动解释为网络连接失败，不透传原错误正文、账号、凭据或URL。该标记不证明上游恢复或任务完成。

2026-11-08 B6：9.9本案例两次真实尝试均未完成，第二轮自行启动两位子任务但上游502导致失败并回收。成功模型响应13轮，报告输入173205/输出2608 tokens；按模型/时间/任务特征关联的代理日志含8次502和1次429，原始失败及分母保留。只读代理日志明确upstream_network_error/Connect；Hub请求超时180秒，不把代理45秒连接失败当作Hub超时。新增脱敏diagnosticCode及真实HTTP 502/凭据canary负例，642项回归和普通父结题四场景通过；本次60秒预检超时，未重复提交完整任务。计划仍63/106，9.9未勾选，候选未安装。详见docs/reviews/hub-swarm-execution-review-v6.json。

2026-11-08 B7：按执行前冻结的候选/夹具/每父2并行/深度1复验9.10：20项实际子进程测试通过（含exit17失败回收、第三并行拒绝、跨Run与深度守卫），真实HTTP只读越权/取消、父进程崩溃不重放、超时，以及关键审查/旧指纹拒绝均通过，原17请求保护通过。受控模型不计真实智能性；9.10完成，64/106。9.9仍未完成，当前Gemini预检60秒超时。证据docs/reviews/hub-swarm-execution-review-v7.json。

C1记忆契约：自动反思和显式笔记写入同一来源项目托管supermemory.json，自动项标记automatic_reflection/experience_candidate，保存成功需读回；执行经验文件为同目录evolution.json并校验sourceProject。保留旧全局/项目文件，不静默迁移、删除或跨项目召回。历史API全局视图保留，其条目不自动成为执行器的项目经验。

C1路径细节：新托管项目笔记固定在projects_root/.project-memory/{规范项目身份哈希}/supermemory.json；evolution.json与其同目录。旧shadow_git_root/project-notes下笔记只读召回并标记legacy_managed_project_notes来源，旧全局执行经验不自动导入；这样开发态的cwd/影子目录变化也不切换笔记归属。旧只读历史条目不在新库中自动变成可修改条目。

C1召回接线：worker移除直接读取旧.supermemory并声明必须遵守的重复前缀，统一由现有RunPrelude投影带来源/历史可靠性标记的笔记；Hermes显式工具也绑定项目执行经验并输出scope。此变更不认证经验修复内容，不关闭9.12～9.15。

2026-11-08 C1：9.11完成，计划65/106；显式笔记、自动Reflection与执行经验统一来源项目和projects_root托管目录，执行经验加载校验sourceProject，旧托管库仅带legacy来源只读召回、旧全局不自动采用。worker移除重复的旧库强制遵从前缀，统一RunPrelude。644项回归通过/0失败/2原忽略/1原过滤，最终候选实际HTTP保存→服务重启→新Run读回→另一项目不泄露三任务通过，规范校验通过。候选未安装；仅认证作用域与路径，9.12～9.15经验真实性等仍待处理；9.9真实模型业务仍因上游未恢复而未完成。证据docs/reviews/hub-memory-scope-review-v8.json。

2026-11-08 回归补修：进程exit0与质量验收收据必须在模型可见结果中区分。网关对verify_task_tool及内联Node/Python检查返回quality_evidence；未识别的命令不获得验证资格，并提示保存独立check-*.cjs等检查脚本或使用已有项目测试命令。网关剥离提供方伪造同名字段；结题门槛、受管验证器和原收据资格不放宽。

2026-11-08 真实模型回归：Gemini预检200/4.59秒恢复。24轮首例两子任务因node -e验收资格不明失败；修后两任务成功收集但父24轮耗尽；按默认40轮另测，Hub完成集成并通过事后宿主独立oracle，冻结检查脚本未变，但取消旧任务后无受控替代关系导致父Run按既有门禁failed。三例均不回填完成，9.9仍未勾选，65/106。已修质量资格反馈及防伪，645项回归/4组拒绝门禁/规范通过。实现由Hub/Gemini生成，宿主仅需求、检查及运行时修补；预算变化单独记录，候选未安装。证据docs/reviews/hub-real-regression-review-v10.json。

B11受控替代契约：retry_subagent只接受所属父Run的旧子ID，复用内存中冻结的原请求（角色、提示、权限、工作区模式、超时不能改）；仅对已终态且回收、未收集的非只读隔离实现任务允许一次替代，审查/调查任务暂不支持此替代以保留失败审查门禁。显式replacesSubagentId持久保存，旧终态不改；替代失败/未收集仍阻止交付，父验证绑定子生命周期证据epoch且需重新检查。恢复后无原请求配方时拒绝重新启动，不重放任务。管理重试沿同一控制通道返回202入队而非成功。

B11管理接口：POST /api/swarm/runs/{taskId}/children/{childId}/retry，请求仅为空对象{}，拒绝额外提示/角色/权限/超时字段；返回202仅排队，结果查询controlReceipts的retry:{childId}。模型与管理接口共享冻结请求和所有权判定。父质量收据新增delegationEpoch，子状态/替代关系/收集收据变化会使旧父验收失效，即便文件字节未变。

2026-11-08 B11：受控替代恢复落地，原请求/权限冻结、每旧任务一次、禁止链式替代和审查绕过；旧终态保留、关系持久，父验证绑定子epoch及文件指纹。648项全库回归通过/0失败/2原忽略/1原过滤，23项进程针对测试、4项管理API测试和最终三组父Run替代/审查拒绝/旧指纹拒绝通过。真实Gemini使用默认工作者时限，双任务成功收集、父集成completed、冻结检查及宿主独立oracle通过；约837秒、86次成功模型响应，输入报告2264116/输出16414 tokens，运行中0人工追加提示。当前案例完整分母1/7（早期失败及预算调整保留），属于修后成功，不称首次或泛化认证。9.9完成，66/106，候选未安装。证据docs/reviews/hub-swarm-replacement-review-v11.json。


C2经验收据（候选9.12）：ToolResult._execution_evidence为网关保留字段，提供方同名内容先移除。字段包括schemaVersion、sessionId、runId、callId、tool、actionDigest、before/afterFingerprint、status、checkEligible、verification、exitCode；不包含原始参数或输出。该收据仅证明观测事实，不能认证修改因果、检查完整性或通用解法；旧记录无此字段不参与自动修复提炼。

2026-11-08 C2/v12：9.12完成，67/106；655项全库回归通过，实际HTTP双Run验证同一失败/修订/检查收据同时进入两条候选路径，普通错误文字不生成修法经验，蜂群三组门禁继续通过。候选未安装；每Run保存决策、只读限制、统一晋升及真实跨Session迁移仍待9.13～9.15。报告docs/reviews/hub-recovery-candidate-review-v12.json。


C3内部审计增量（9.13，2026-11-09）：SessionEvent增加MemoryPersistenceRecorded，payload为receipt（schemaVersion、runId、sessionId、projectDigest、decisions）。每项包含stage、status、reason、recordIds、readbackDigests；仅stored允许携带经真实读回核验的ID与摘要。审计不进入项目召回或模型投影，不另建生命周期。异常IO只记录固定原因。终态后统一写入，原worker后置Reflection改由执行器协调，避免重复写入。

2026-11-09 C3/v13：9.13完成，68/106；四阶段自动记忆保存由最终Run统一审计、写入后实际读回；只读/计划/叶子不自动写，原中间摘要先缓存，写盘失败与敏感拒绝如实记录。666项库回归及5组HTTP验收通过（敏感笔记Run按真实结果failed），蜂群替代成功/审查拒绝/旧指纹拒绝仍通过，6个叶子四阶段均skipped。候选未安装；保存决策审计不等于候选已晋升或具备迁移能力。报告docs/reviews/hub-memory-persistence-review-v13.json。


C4管理契约（9.14）：GET /api/evolution/candidates?projectPath=<已打开项目>；POST /api/evolution/candidates/{id}/actions，请求严格为{projectPath,action:validate|promote|revoke,expectedRevision,suiteId?}，suiteId仅validate使用。409表示版本/状态/证据不满足，422表示受信配置/留出检查失败，403表示项目未打开；不接收调用方passed、command或证据文件。validate通过程序托管evolution/holdout-profiles.json中固定Node裁判执行，响应是实际验证状态；promote要求内容、来源、套件/目标/裁判指纹及时效仍匹配，revoke立即阻止后续挂载。SkillTool save/update返回candidate而非活动成功，read_candidate仅用于显式查看隔离草稿；普通list/read只显示当前有效技能。

C4读取增量：skill_tool read的有效目录查询无匹配时返回{status:"unavailable",available:false,inspection_verified:true,reason:"not_in_effective_catalog"}，不含Skill正文；不再将预期不可用当作执行失败。inspection_verified仅表示本次只读目录查询，不能释放实现验证门禁或将候选激活。

C4经验查询一致性：hermes_workflow query_lessons与super_memory及RunPrelude共用当前项目治理门禁，仅返回有效治理经验（id/name/observation/source/reliability），旧ELO历史不自动成为指导。只读查询不需要网络或写入权限；经验就绪计数同样包含有效治理记录，不把未晋升候选算作可用。


2026-11-09 C4/v14：9.14完成，69/106。候选隔离、固定不同目标Node验证、版本晋升/撤销、来源及记忆原副本失效门禁落地；Hermes模型工具菜单与只读查询、原生记忆和就绪计数一致。681项库回归通过（0失败/2原忽略/1原过滤），精确候选二进制17次受控Run全部完成/34模拟响应，另5组记忆审计及3组蜂群门禁通过。留出范围仅1个不同目标/3行断言，真实Node通过2次/预期失败1次；不能作为真实模型迁移评分。候选未安装，3项原资产指纹不变。详细证据docs/reviews/hub-candidate-governance-review-v14.json；9.15真实模型重启自然召回仍待验证。
