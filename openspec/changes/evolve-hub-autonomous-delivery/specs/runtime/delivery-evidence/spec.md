# Spec Delta

## Purpose

定义用户自然需求到业务成果之间的交付证据契约，使任务完成状态与实际产物、验证结果和适用范围一致。该能力同时支持只读调查和生成修改任务，避免把计划、观察或局部抽样误报为完整业务交付。

## ADDED Requirements

### Requirement: SDK interactions follow the correlated server contract
SDK SHALL 将补充要求提交为action=steer和response，用户回答提交为action=respond、requestId和response。旧steeringPrompt调用可映射，但不得将旧审批token/布尔值猜测转换为相关用户回答。

#### Scenario: Frontend submits a live supplement
- **WHEN** 运行中的前端SDK调用interact携带steeringPrompt
- **THEN** 实际HTTP请求符合后端契约并将补充指令送达工作循环，成功响应不能伪报未投递的指令

### Requirement: Delivered supplements invalidate affected acceptance
系统 MUST 在补充指令成功入账并确认投递后更新当前验收轮次，使验证及其依赖步骤与物理收据待复验，保留观察、实际修改及单一任务契约，不用旧检查为新约束结题。

#### Scenario: Constraint arrives after a passing check
- **WHEN** 同Run已有成功检查，用户又补充验收约束
- **THEN** 旧检查仍是已发生的事实但不能满足新验收；系统要求当前约束下的新检查，不伪造新的修改动作

### Requirement: Completed default stages do not force ceremonial planning
系统 SHALL 在默认执行阶段及显式契约无缺口时建议有据答复；没有交付声明时明确verify_delivery不适用，不强迫为它新增计划。真实剩余要求仍须处理。

#### Scenario: Ordinary adjustment is already checked
- **WHEN** 当前Run实际完成修改和有效质量检查，且没有待完成的显式子任务或交付声明
- **THEN** 系统建议据实答复，不要求为了调用verify_delivery补建计划或重新检查

### Requirement: Structured plans and receipts correct advisory routing
系统 SHALL 用当前Run已确认计划的步骤类型及规范化真实收据校正初始词面路由，保留已经观察到的事实和当前验证新鲜度；计划不得被当成实施证据，也不得降级已有执行或宿主验收要求。

#### Scenario: Editing wording was initially conversational
- **WHEN** 词面分类漏识别编辑请求，但当前Run提交mutation计划或实际写入成功
- **THEN** 系统采用执行推进与严格验证，先前真实观察可以沿用，不要求重写同一计划或重复读取

#### Scenario: Explicit read-only redirection
- **WHEN** 新Run用户明确只做复核，模型提交mutation计划或调用已识别原生写工具
- **THEN** 在确认计划或执行工具前拒绝，历史已完成实施不能授权本轮修改；通用shell限制仍如实披露

### Requirement: Projected history retains Run provenance
系统 MUST 在模型上下文中区分当前工具结果、历史Run结果和来源未知的旧结果，保留工具配对、折叠原文规则及原账本，不把历史结果重新记作当前成功工具。

#### Scenario: Resume a session after earlier file reads
- **WHEN** 前Run读过业务文件而当前Run只更新说明文档
- **THEN** 历史结果带原Run标签，当前事实摘要只计本轮操作；历史内容仍可复用但不能被声称为本轮新读取

#### Scenario: Legacy or ambiguous tool history
- **WHEN** 旧会话结果缺少可匹配Run边界或跨边界恢复配对
- **THEN** 投影标记unknown，不伪造current身份，旧会话仍能读取

### Requirement: Current Run summaries contain only host facts
系统 SHALL 按绑定Session/Run输出有界事实摘要，包含规范化工具状态计数和本轮收据引用；不注入原命令、文件正文或工具自带指令，不将普通进程成功或物理文件检查提升为领域验证。

#### Scenario: Provider payload claims additional checks
- **WHEN** 工具返回自称已运行其他检查或重复收据被观察
- **THEN** 摘要只按宿主规范化收据记录一次该调用，不信任其中的额外操作声明；超容量时明确计数为下界

### Requirement: Identical plan retransmission preserves evidence
系统 MUST 在完全相同TaskContract重传时保留计划版本、已完成绑定和当前验证；有变化的完整替换仍按既有规则失效，新增要求优先走单一契约增量修订。

#### Scenario: Model repeats an acknowledged plan
- **WHEN** 当前计划已有有效步骤收据，模型再次提交完全相同plan_task
- **THEN** 计划版本和进度不变，不因重传要求重读或重验；真实修订仍失效受影响步骤

### Requirement: Recovery budgets survive unrelated successful observations
系统 MUST 按当前Run、规范化调用指纹和工作区修订记录失败类别及预算；参数/缺依赖错误须改变输入或修复状态后再试，网络/业务/未知错误的原样执行最多两次。成功的无关观察不能重置预算，真实工作区变更或同一调用成功后可更新恢复状态。

#### Scenario: Successful probes between identical failures
- **WHEN** 相同验证输入失败两次，中间夹有成功的文件读取或接口探针且未修订工作区
- **THEN** 第三次相同失败调用在执行前拒绝，返回分类及有界恢复指引

### Requirement: Trusted physical acceptance is explicit and scoped
系统 SHALL 仅按固定宿主profile显式physical_artifacts策略接受纯物理交付；缺文件、旧哈希、变更profile、业务validator或受控动作均不能借此绕过原门禁，模型声明不能启用该策略。

#### Scenario: Documentation has fresh physical receipts
- **WHEN** 宿主明确仅验收文档文件，当前Run写入并通过完整物理verify_delivery且显式子任务无缺口
- **THEN** 默认代码质量检查不再阻止结题，domain_verified保持false

#### Scenario: Physical delivery guidance before files are written
- **WHEN** 受信任宿主已声明physical_artifacts，当前文件尚未交付
- **THEN** 默认认知推进、验证阶段标题及建议工具都指向文件与verify_delivery，不同时强制无关的代码质量或运行环境探测；原质量模式和业务验收保持不变

#### Scenario: Legacy intent classifier misses an editing phrase
- **WHEN** 宿主已固定physical_artifacts但旧词面分类把编辑措辞归为普通对话
- **THEN** 文件交付指引仍按该宿主策略呈现，系统默认编译测试anchor不再与之冲突，用户显式检查与自定义约束保持有效；不得将该展示修正冒称完整分类引擎已验证

#### Scenario: Historical README claims an unproven certification
- **WHEN** 文档任务读到旧README的AST或平台认证声明，本Run未取得对应验证收据
- **THEN** 指导要求把它标为未独立复核的历史文案或移除，不得将物理文件验收改写成新增业务认证

#### Scenario: Physical acceptance attempts to bypass business verification
- **WHEN** profile同时声明physical_artifacts与业务validator或受控动作
- **THEN** 配置加载失败，原业务验收不降级

### Requirement: Internal live traces are not business evidence
系统 MUST 默认阻止模型用直接文件工具和已识别的显式shell路径读取当前Run内部trace，提供作用域正确的工具结果读取替代；只有受信任宿主显式诊断配置才可放开该路径，不宣称任意shell隔离。

#### Scenario: Agent starts inspecting its own prompts
- **WHEN** 模型调用文件读取、目录列举或显式shell命令访问当前Run的.superagent/traces
- **THEN** 在执行前拒绝，保留普通业务文件和Skill资源读取能力

### Requirement: Delivery scope follows user intent
系统 SHALL 明确每次任务的只读或实施范围、目标身份、必需成果、可选成果和验收条件，并在多轮变更时保持当前有效版本，不能通过无关写盘满足调查任务的结题条件。

#### Scenario: Read-only review
- **WHEN** 用户要求只复核已有报告
- **THEN** 系统允许凭有来源的调查结论完成，不强制创建或修改业务文件

#### Scenario: Generate usable rule
- **WHEN** 用户要求根据技能生成指定目标的可用规则
- **THEN** 调查报告和执行计划不能代替必需的规则文件及运行验证

### Requirement: Completion requires matching physical evidence
系统 MUST 将每项必需交付绑定真实产物及适用的验证记录，记录目标、任务、路径、哈希、来源、验证器版本和验证时间；缺失或不匹配时不得宣称该业务交付完成。

#### Scenario: Artifact changes after validation
- **WHEN** 规则文件在成功验证后发生变化
- **THEN** 旧验证不再证明新文件有效，必需项恢复为待验证

#### Scenario: Observation mistaken for delivery
- **WHEN** 应用探针退出成功但尚无规则文件
- **THEN** 系统只确认应用调查证据，不确认规则生成完成

### Requirement: Rule delivery is verified through its actual runtime
系统 SHALL 对规则的分类、列表、搜索、详情及播放解析运行目标生产规则执行库，并从该规则的播放结果验证媒体数据及无界面解码；静态检查或简化仿真不能独立认证真实可用。

#### Scenario: Independent media probe succeeds but rule fails
- **WHEN** 外部探针可播放而生成规则的 lazy 链路报错
- **THEN** 规则交付验收失败，并保留两种结果及其来源

#### Scenario: Local native dependency
- **WHEN** 规则依赖本机原生桥接服务
- **THEN** 交付列出依赖、启动方法和已验证平台，不能声明手机独立运行已通过

### Requirement: Validation preserves target identity and coverage
系统 MUST 校验交付与用户目标的来源一致，并在报告中声明样本、分页、选集、平台及有效期范围；不得以替代网站或少量样本冒充原目标的全量支持。

#### Scenario: Substitute website
- **WHEN** 用户指定安装应用而规则实际请求另一个未获认可的网站
- **THEN** 原目标转换验收不通过，即使替代规则可以返回数据

#### Scenario: Unsupported pagination
- **WHEN** 同次上游证据证明某频道不存在分页
- **THEN** 系统将该频道分页标为不适用并保留依据，不生成重复第一页冒充下一页

### Requirement: Failures lead to bounded recovery
系统 SHALL 在明确的总时限、轮次和模型预算内区分工具、依赖、网络及业务失败，利用错误证据修正方法；同一输入无新证据的重试不得超过两次，预算耗尽时报告未完成项。

#### Scenario: Repeated failing invocation
- **WHEN** 相同工具输入已连续失败两次且没有新增证据
- **THEN** 系统改变调查或执行方法，或报告阻塞，不继续无效循环

#### Scenario: Required check unavailable
- **WHEN** 必需运行验证因外部环境失败未完成
- **THEN** 系统可交付已有文件并注明未验证，但不能将业务验收标为通过

### Requirement: Execution stays within authorized workspace
系统 MUST 区分只读知识来源与获授权的执行输出根；命令、文件变更、验证和结果读取保持任务作用域一致，不因识别到原项目而自动扩大写入范围。

#### Scenario: Isolated workspace with origin knowledge
- **WHEN** 任务在隔离工作区执行并读取原项目图谱或记忆
- **THEN** 业务变更留在获授权的隔离输出根，原项目和冻结验收器不受修改

#### Scenario: Truncated tool output
- **WHEN** 工具输出截断并返回原文引用
- **THEN** 同一任务能够按引用读回对应完整输出或分片，不能读到另一任务的内容

### Requirement: Independent rule preserves the requested runtime dependencies
系统 MUST 在用户要求应用逆向后生成独立JS规则时，优先还原实际远端接口及必要签名、解密与解析，不得将已存在的原生探针直接转化为最终规则的强制依赖。旧桥接验收不能作为独立规则完成证据。

#### Scenario: Installed application exposes a native ABI
- **WHEN** 用户要求无需Python桥接的独立规则且应用含可用DLL
- **THEN** DLL仅可作为隔离调查和原目标对照材料，最终规则不依赖DLL、本地HTTP端口、原应用安装目录或常驻进程

#### Scenario: Endpoint found but protocol unresolved
- **WHEN** 已找到真实接口地址但签名或响应解密尚未还原
- **THEN** 记录具体未解步骤，不能把地址清单、一次性媒体链接或替代站点规则判为完成

### Requirement: Cloud publication requires remote roundtrip evidence
系统 MUST 在用户授权的云发布任务中由Hub实际上传并记录短口令，使用生产口令解析和私有数据库导入回读内容，与冻结的原规则必要字段一致后方可认证。宿主监督与反馈必须披露，不把离线编码或上传接口返回ID当成完成证据。

#### Scenario: Offline command presented as cloud share
- **WHEN** 产物仅为长Base64口令、示例ID或没有远端可回读内容
- **THEN** 云发布验证不通过，保留已有本地文件

#### Scenario: Coached publication completes
- **WHEN** Hub在监督者反馈后修复并完成上传与回读
- **THEN** 记录带反馈完成及原失败，不能改为首次无人干预成功

### Requirement: New target certification preserves assistance boundaries
系统 MUST 对新目标自然需求测试固定初始提示、技能版本、候选版本、预算和独立业务验收；宿主不提供目标协议或业务实现，最多两次基于实际错误的简短纠错，所有尝试分类保留。

#### Scenario: New public video target succeeds
- **WHEN** Hub从新网址自行生成独立JS并完成分类、分页、搜索、详情、真实媒体及云回读验证
- **THEN** 只认证该公开视频目标及实际测试平台，保存作者、原样哈希及反馈次数，不推导未知加密目标已可自主逆向

#### Scenario: Host oracle implementation is inspected
- **WHEN** Hub读取了包含目标协议的宿主验收器实现或宿主提供目标业务代码
- **THEN** 显式记录污染，不将该尝试计为严格首次自主生成成功

### Requirement: Managed actions require fresh business evidence
系统 MUST 仅允许宿主预先注册的动作消费当前Run已通过业务验证、路径与哈希未变的来源产物；在调用执行器前消耗单次执行额度。模型不能替换正文或任意命令。

#### Scenario: Source invalid or changed
- **WHEN** 模型请求受控发布但来源未验、失败、已修改或验证器指纹变化
- **THEN** 执行器不启动，模型收到本地修复与复验指引

#### Scenario: Partial local verification before publication
- **WHEN** 本地规则通过业务验证而云发布产物尚不存在
- **THEN** 可回传该规则的有效验证收据，completion_ready仍为false；受控动作可用，完整结题仍被阻止

#### Scenario: Raw clipboard probing in a managed Run
- **WHEN** 启用受控动作的Run试图通过shell引用已知云剪贴板端点或导出CLI，包括已读取的本地脚本
- **THEN** 网关拒绝进程执行并引导使用受控动作；此检测不被声明为操作系统网络隔离

#### Scenario: Upload response precedes readback failure
- **WHEN** 发布已返回ID但回读失败
- **THEN** 保存该ID供只读复验，禁止再次创建发布副本，且不泄露管理凭据
