# Spec Delta

## Purpose

定义用户自然需求到业务成果之间的交付证据契约，使任务完成状态与实际产物、验证结果和适用范围一致。该能力同时支持只读调查和生成修改任务，避免把计划、观察或局部抽样误报为完整业务交付。

## ADDED Requirements

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
