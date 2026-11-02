# Spec Delta

## Purpose

为智能体通过反馈改进经验、技能及代码提供可审计的晋升边界，区分候选生成、验证通过和实际生效。该能力要求在保留用户工作与运行连续性的前提下验证新版本，并允许拒绝或撤回有退化的候选，避免将重启请求误报为升级成功。

## ADDED Requirements

### Requirement: Lessons are candidates until independently validated
系统 SHALL 将纠错生成的经验和策略保存为带来源的候选，保留纠错前失败，并在独立留出任务验证后才使其影响后续任务；训练示例改善不能独立证明迁移能力。

#### Scenario: Explicitly coached correction
- **WHEN** 用户提供具体错误反馈后任务成功
- **THEN** 系统记录辅导后成功，候选经验需再经独立任务验证，不能回填原首次尝试成绩

### Requirement: Code evolution preserves user workspace
系统 MUST 在隔离候选工作区生成限定代码变更并验证候选二进制，记录基线和用户未提交修改；验证失败或回退只能撤销候选及切换，不得覆盖整份用户源码或扩大授权范围。

#### Scenario: Candidate fails regression
- **WHEN** 自修改候选构建成功但回归或留出验收失败
- **THEN** 系统拒绝该候选，保留当前可用版本及用户未提交修改，并交付失败证据

#### Scenario: Candidate requests broader privileges
- **WHEN** 候选执行需要现有授权范围外的操作
- **THEN** 系统不以进化名义自动扩大权限，候选不能据此晋升

### Requirement: Upgrade success requires new runtime evidence
系统 SHALL 仅在候选版本切换后核对进程身份、健康状态及会话连续性，才报告升级成功；发送重载请求或编译成功不能单独满足该条件。

#### Scenario: Reload request accepted but candidate unhealthy
- **WHEN** 重载请求已接收而新进程未健康就绪
- **THEN** 系统不报告升级完成，按已验证方案回退并保留状态和原因

#### Scenario: Active run during upgrade
- **WHEN** 升级发生时仍有活动任务
- **THEN** 系统先排空或按已验证恢复协议保留任务，不能将丢失任务伪报为连续执行
