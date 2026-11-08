# Spec Delta

## Purpose

约束代码理解、项目记忆和终端压缩向任务提供上下文的方式，使证据可以追溯到正确项目、版本和调用。通过独立状态与对照评测区分组件存在、实际参与以及是否改善工作结果，避免空检索、陈旧知识和压缩失真被误认为可靠事实。

## ADDED Requirements

### Requirement: Natural note queries reuse scoped context without artificial tool calls
系统 SHALL 使明确引用有效项目笔记标识的自然长问题保持可召回，不因附加礼貌或验证范围说明而稀释精确标识匹配；ASCII标识需要完整边界，不把近似或前缀标识当成同一笔记。实际完整注入的项目笔记 SHALL 在运行时作为历史观察证据，独立记录数量，不伪造工具调用或代码质量凭据。笔记意图优先指向super_memory，无命中不等于笔记必不存在，其他业务调查仍按实际证据推进。

#### Scenario: Long query has a complete note identifier
- **WHEN** 长中文问题含完整有效笔记标识且存储作用域和版本仍有效
- **THEN** 运行时注入有界且完整的历史证据块，模型可注明来源直接回答，无需重复调用工具凑完成门槛

#### Scenario: Similar identifier and code task remain protected
- **WHEN** 查询包含另一标识的后缀或前缀，或任务要求改代码和执行测试
- **THEN** 不把相近标识当精确命中，历史上下文不能完成源码变更或质量测试步骤；撤销/过期/跨项目过滤继续生效

### Requirement: Runtime project notes survive isolated Runs without writing origin source
系统 SHALL 将super_memory新笔记保存到由运行时按规范化来源项目确定的程序管理缓存，内容和来源元数据同一原子检查点写入；模型不能指定其他项目或任意存储文件。笔记写入/撤销是变更操作，查询是只读操作。原始项目文件保持不变。

#### Scenario: Store then recall after service restart
- **WHEN** Hub在隔离Run记录项目约定，随后服务重启并在同一来源项目的新Session中召回
- **THEN** 实际召回相同项目笔记并标记为历史记录及验证范围，不依赖上个Session正文或原项目源码写回

#### Scenario: Explicit note operation is not a code edit
- **WHEN** 用户仅要求原生项目笔记的保存或撤销，工具原子写入后读回核实状态
- **THEN** 运行时依据该操作的读回证据完成笔记任务，不要求无关代码测试；笔记回执不计作源码变更或代码质量验证，不能帮助代码任务绕过实施及测试门槛

### Requirement: Note context is explicit and recall respects validity
记忆 MAY 带项目、类型、置信度、来源、修订号、来源文件相对路径/哈希、有效期和候选/有效/撤销状态。系统 SHALL 在自动上下文及原生记忆召回时排除跨项目、过期、撤销或来源哈希失效的记录；旧无元数据记录保留为明确未验证的历史数据，不自动升级事实或Skill。

#### Scenario: Candidate history and stale source
- **WHEN** 同时存在候选笔记、旧无元数据记录和来源文件已改变的摘要
- **THEN** 前两者以未验证历史来源呈现，失效摘要不注入，匹配或置信度不充当业务认证

### Requirement: Stored memory rejects identifiable credentials
记忆检查点 SHALL 拒绝可识别的密钥、Bearer凭据及带敏感签名参数的媒体URL，错误不回显敏感值，已有检查点保持不变。该检查不被宣称为完整秘密检测；新增有效经验仍须独立验证才可晋升。

#### Scenario: Non-secret canary resembles a credential
- **WHEN** 测试写入明确非真实的密钥/Bearer/签名URL canary
- **THEN** 写入失败且原文件哈希不变，不出现可召回敏感canary

### Requirement: Context capabilities expose distinct readiness and validation
系统 SHALL 分别报告原生图谱、外部 Understand、记忆及压缩能力的注册、可用、就绪和已验证状态，并附证据范围；一个能力通过不得替代另一个能力认证。

#### Scenario: Native graph works but external plugin absent
- **WHEN** 原生图谱查询成功而外部 Understand 所需资源缺失
- **THEN** 系统报告原生查询可用及外部插件未就绪，不显示外部插件已认证

### Requirement: Grounded context matches project and source version
系统 MUST 为任务使用的图谱或源码证据记录项目、来源指纹和时效，识别索引与源码不匹配；不可用时显式降级，不能将检索失败解释为不存在依赖或影响。

#### Scenario: Source changes after graph build
- **WHEN** 当前源码与索引指纹不一致
- **THEN** 系统刷新相关证据或使用标明来源的源码调查，在结果中保留降级状态

#### Scenario: Graph service returns an error
- **WHEN** 图谱查询因服务或路径错误失败
- **THEN** 系统保留失败并采用可用调查方式，不以空依赖列表作为无影响证明

### Requirement: Project memory is scoped and correctable
系统 SHALL 使项目记忆带来源、类型、置信度、适用版本及失效状态，在跨 Run 和重启后保持可检索；不得将其他项目或已撤销经验静默应用于当前任务，不得将私有凭据沉淀为可召回经验。

#### Scenario: Conflicting or expired lesson
- **WHEN** 检索到与当前版本冲突或已撤销的经验
- **THEN** 系统不将其作为当前事实，优先取得新证据并允许修正记忆

#### Scenario: New task after restart
- **WHEN** 同一项目重启后发起需要既有非敏感事实的新任务
- **THEN** 系统能够召回仍有效的项目事实并标注来源，同时隔离其他项目内容

### Requirement: Compression preserves diagnostic evidence
系统 MUST 记录压缩真实输入输出大小、调用身份及原文引用，保留退出状态、关键错误和产物定位；无法安全压缩时允许原样输出，不得虚构节省量。

#### Scenario: Short output has no savings
- **WHEN** 压缩前后字节数相同
- **THEN** 系统记录节省为零，不将其报告为正收益

#### Scenario: Error appears beyond displayed summary
- **WHEN** 关键失败细节未保留在压缩展示内容中
- **THEN** 任务可通过同作用域原文引用读回该错误，不能凭摘要认定成功

### Requirement: Context contribution is measured without tool-call gaming
系统 SHALL 通过冻结任务的成对实验报告知识能力参与、任务正确率及成本；有归属证据的自动上下文注入可算参与，不能要求重复显式调用才能通过，也不能将命中直接当成因果收益。

#### Scenario: Relevant evidence arrives through automatic prelude
- **WHEN** 任务输入已包含可核对项目、任务和版本的图谱证据
- **THEN** 评测允许确认该能力参与，无需模型重复请求相同图谱信息

#### Scenario: Memory hit does not improve result
- **WHEN** 对照任务显示记忆命中但正确率未提高且成本增加
- **THEN** 报告如实呈现结果，不以命中次数宣称智能提升


### Requirement: Automatic experience uses traceable project persistence
系统 SHALL 将自动经验与显式项目笔记绑定同一来源项目身份和可追溯存储，记录每Run的stored/skipped/rejected/failed决策；stored须真实写入与读回，经验内容须关联匹配工具ID、修订和验证，不能只从error关键词与工具名宣称已学会。只读任务不自动写可召回经验，候选不得无留出验证进入活动Skill。

#### Scenario: Write fails or no applicable lesson exists
- **WHEN** 经验保存失败或本Run没有足够修订/验证证据
- **THEN** 分别记录failed或skipped及原因，不打印已持久化成功，不生成虚构通用修法

#### Scenario: Restart after an actual repair
- **WHEN** 已验修复的经验经来源项目缓存保存后在新Session重启
- **THEN** 自然请求可召回同一有效记录并标注来源，其他项目、撤销、过期或源码变更不得误用
