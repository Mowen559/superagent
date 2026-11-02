# Spec Delta

## Purpose

约束代码理解、项目记忆和终端压缩向任务提供上下文的方式，使证据可以追溯到正确项目、版本和调用。通过独立状态与对照评测区分组件存在、实际参与以及是否改善工作结果，避免空检索、陈旧知识和压缩失真被误认为可靠事实。

## ADDED Requirements

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
