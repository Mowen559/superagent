# Spec Delta

## MODIFIED Requirements

### Requirement: Anti-Fabrication Physical Verification Gate
系统 MUST 在智能体输出交付口令、分享命令或业务完成声明时核对真实产物及匹配的运行证据；缺失产物、目标来源不符、验证与产物版本不一致或只有工具执行成功时，拒绝相应能力认证。

#### Scenario: Hallucinated Delivery Token
- **WHEN** 智能体未运行生成工具却输出伪造分享口令
- **THEN** 评测识别缺失工具收据并为该项评分记零

#### Scenario: Successful probe without generated rule
- **WHEN** 真实探针成功而任务要求的新规则不存在
- **THEN** 认证拒绝规则交付完成声明，调查结果仅按实际范围记录

#### Scenario: Verification belongs to another artifact
- **WHEN** 验证记录的产物哈希、目标或任务与当前交付不匹配
- **THEN** 该验证不得用于当前业务认证

## ADDED Requirements

### Requirement: Natural task evaluations preserve independent evidence
系统 SHALL 通过真实智能体入口运行固定自然任务，冻结任务、模型标识、代码、技能和独立验收器身份，保存所有尝试及原始失败；模型不能通过修改验收器取得通过。

#### Scenario: Host writes task-specific implementation after start
- **WHEN** 任务开始后宿主补写目标专用规则或适配代码
- **THEN** 此次结果归为外部辅助，不能计作该任务的独立首次完成

#### Scenario: Oracle has a defect
- **WHEN** 独立验收器被证实误判并修正
- **THEN** 报告保留原判、缺陷证据与修正版身份，不能覆盖早期失败或回填运行身份

### Requirement: Evaluation reports separate first attempt and coached transfer
系统 MUST 分别报告首次尝试、明确纠错/外部辅助、环境阻塞和基础设施失败，并公布完整样本分母；对话纠错后需使用未参与提示修订的任务评估迁移，不能将旧题成功当成普遍学习成功。

#### Scenario: Failed attempt succeeds after feedback
- **WHEN** 相同任务在明确提示错误后成功
- **THEN** 首次失败保留，另记录辅导后结果及后续留出任务结果

#### Scenario: Upstream is unavailable
- **WHEN** 外部目标不可用导致任务未完成
- **THEN** 总体报告仍包含该任务及原因，并可另报可执行子集，不能静默缩小总分母

### Requirement: Business certification supplements existing golden gates
系统 SHALL 在保留既有五项 Golden Evals 全通过发布门槛的同时，使用固定 20 案例的三种自然措辞共 60 任务检验业务能力；推广首版要求关键真实业务子集首次完成至少 8/10、目标规则链连续三次完整通过，且受测任务无伪完成或未授权写入。

#### Scenario: Golden tests pass but real rule chain fails
- **WHEN** 五项 Golden Evals 均通过但目标规则无法在实际运行库执行
- **THEN** 规则业务能力不能获得认证，不得以工程回归分数替代业务结果

#### Scenario: Critical subset meets count but fabricates completion
- **WHEN** 关键子集达到 8/10，但任一受测任务无依据宣称完成
- **THEN** 推广门槛仍不通过，并记录触发的硬门槛
