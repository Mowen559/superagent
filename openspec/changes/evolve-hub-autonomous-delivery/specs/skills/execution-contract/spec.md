# Spec Delta

## Purpose

使技能的发现和调用能够进一步形成可检验的执行契约，明确资源、依赖、产物和适用运行环境。该能力兼容已有技能，同时阻止未经实际业务验证的脚本、提示文本或新技能直接获得交付认证。

## ADDED Requirements

### Requirement: Skills expose versioned execution expectations
系统 SHALL 支持技能声明适用任务、版本、依赖、资源、预期产物及验证器引用，并使自动匹配与显式调用消费相同信息；旧技能保持可读取，缺失声明不得被解释为业务已认证。

#### Scenario: Legacy skill without execution manifest
- **WHEN** 用户调用仅有 Markdown 的旧技能
- **THEN** 系统仍可读取并执行该技能，但只有实际满足交付验证时才认证其业务成果

#### Scenario: Missing runtime dependency
- **WHEN** 技能要求的本地运行时不可用
- **THEN** 系统指出具体依赖并在授权作用域内恢复或提供有依据的降级，不伪报技能执行通过

### Requirement: Skill resources remain scoped and attributable
系统 MUST 仅读取所选技能允许范围内的真实资源，记录所用技能和资源版本，遵循大小及路径边界；指向外部目录的越界资源不得被当成技能自身可信内容。

#### Scenario: Resource escapes skill directory
- **WHEN** 技能资源相对路径或链接解析到允许根之外
- **THEN** 系统拒绝该资源读取并保留明确错误，不通过路径改写绕过限制

### Requirement: Generic reads can resolve selected skill resources
系统 SHALL 将已注册skill_tool确认属于当前所选技能根的真实文件读取请求路由到只读skill_tool action=read，保留来源和行范围；不扩大普通文件工具权限，不转换写入或执行请求。旧工具不声明资源路由时行为不变。

#### Scenario: View tool targets a selected skill script
- **WHEN** 模型用view_file或其读别名请求所选技能根内的真实脚本
- **THEN** 网关按技能目录解析并调用skill_tool读取，记录路由来源且不执行该脚本

#### Scenario: Generic path or symlink escapes skill root
- **WHEN** 请求为普通外部文件、未选择的影子技能资源或canonicalize后位于所选技能根之外的链接
- **THEN** 网关不将其认定为可信技能资源，保留原文件权限检查及明确失败

### Requirement: Business verification is independent of generated output
系统 MUST 区分技能自检与受信任的业务验收，后者验证冻结的用户目标、产物及真实运行结果；模型不得通过改写验收器、降低断言或更换目标取得通过。

#### Scenario: Validator only prints success
- **WHEN** 名称含 validate 的脚本退出成功但未检查任务要求的业务数据
- **THEN** 系统可记录脚本执行成功，但业务验收仍为未通过

#### Scenario: Generated rule fails independent runtime check
- **WHEN** 技能静态检查通过而独立规则运行检查失败
- **THEN** 系统向任务返回运行失败证据并继续有界修复，不能凭静态结果结题

### Requirement: Skill promotion requires held-out validation
系统 SHALL 将新增或修订技能先保存为候选，在独立负例和未用于修订的留出任务上通过验证后才晋升，并保留来源、版本、原始失败和回退入口。

#### Scenario: Coached case improves but unseen case regresses
- **WHEN** 候选技能修好已提示案例却使留出任务退化
- **THEN** 系统不晋升该候选，现有有效技能保持不变，并报告退化证据
