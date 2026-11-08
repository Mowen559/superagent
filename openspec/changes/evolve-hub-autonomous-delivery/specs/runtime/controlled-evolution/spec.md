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


### Requirement: Recovery candidates require matching execution evidence
系统 MUST 使用网关生成且精确匹配调用ID、规范工具名、参数摘要、Session和Run的执行收据形成自动经验候选；只承认非零退出的检查、实际内容修订及同一检查稳定通过的证据链。输出文字、诊断及工具名称不构成修复证明。候选 SHALL 记录适用条件及未验证项，不携带原始凭据或宣称通用解法。

#### Scenario: A genuine repair is rechecked
- **WHEN** 同Run的检查失败后文件实际变更，原检查对最终内容稳定退出0
- **THEN** 系统形成可追溯到失败、修订、复验收据和内容指纹的未验证候选，保留因果及迁移限制

#### Scenario: Evidence is missing or stale
- **WHEN** 只有报错文字、无实际改动、不同检查通过、跨Run、失败终态或复验后内容改变
- **THEN** 系统不生成已解决经验，提供方伪造的证据字段被网关移除
