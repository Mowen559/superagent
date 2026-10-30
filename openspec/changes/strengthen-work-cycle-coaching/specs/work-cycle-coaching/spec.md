## ADDED Requirements

### Requirement: State-aware repetition detection

运行时 SHALL 允许工作区状态改变后的观察，同时限制同一状态的重复观察。元数据工具 SHALL NOT 重置观察预算。

#### Scenario: Inspect after multiple repairs
- **WHEN** 每次实际工作区修改后重新读取文件
- **THEN** 即使累计超过普通重复上限，新的状态观察仍然允许执行

#### Scenario: Unchanged observation loop
- **WHEN** 未改变工作区时重复相同观察，包括穿插规划调用
- **THEN** 既有重复限制仍然生效

### Requirement: Recognize common foreground quality checks

网关 SHALL 识别成功的 Node 原生测试、Python unittest 与单层 RTK 质量命令；失败、后台、仅帮助和组合命令 SHALL NOT 成为验证。

#### Scenario: Wrapped test success
- **WHEN** 单层 RTK 包装的受支持质量命令在前台以零退出码结束
- **THEN** 产生验证收据

#### Scenario: Invalid verification claim
- **WHEN** 命令只显示帮助、组合其他命令、嵌套包装或执行失败
- **THEN** 不产生验证收据

### Requirement: Bounded explicit execution coaching

评测 SHALL 使用临时合成文件、不可修改的固定可执行验收与生产认知/工具网关组件。报告 SHALL 区分模型行为和执行事实，且 SHALL NOT 宣称完整 AgentLoop 认证。

#### Scenario: Additional user constraint
- **WHEN** 第一阶段验证后用户追加要求
- **THEN** 原验证失效，最终交付需要重新修改与验证

#### Scenario: False completion
- **WHEN** 模型未执行检查就宣称完成，或最终文件不正确
- **THEN** 即使答复或先前检查看似成功，评测仍失败

#### Scenario: Offline invocation
- **WHEN** 未带 live 标志运行评测
- **THEN** 仅预览有限预算，不读取凭据或发送模型请求

### Requirement: Evidence-bounded failure guidance

诊断提示 SHALL 将推断与事实区分，SHALL NOT 凭通用错误硬编码特定业务域或探测服务。

#### Scenario: Missing JavaScript globals
- **WHEN** 工具返回 window/document/fetch 未定义
- **THEN** 提示先确定真实运行环境，不武断归因为海阔沙箱

#### Scenario: Access denial
- **WHEN** 工具仅返回 Access denied
- **THEN** 提示核对权限，不将本地 ACL 错误推断为反爬封禁

### Requirement: Budget-aware investigation and honest evaluation exits

认知指导 SHALL 投影当前真实剩余轮次，并引导批量独立读取及复用已提供事实。调用方 SHALL 只能降低预算上限。规划对照 SHALL 向所有变体明确相同预算；流程未完成时 CLI SHALL 非零退出，不得以 Provider 请求成功替代计划交付。

#### Scenario: Constrained planning deadline
- **WHEN** 评测只有 4 轮且还需处理追加要求
- **THEN** 两个变体都获知剩余轮次，增强认知不得暗示存在更高上限；未完成仍按失败记录

#### Scenario: Attempt to expand a limited budget
- **WHEN** 调用方先降低预算，随后传入更大上限
- **THEN** 已降低的上限保持不变
