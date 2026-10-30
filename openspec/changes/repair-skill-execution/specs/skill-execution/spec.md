## ADDED Requirements

### Requirement: Consistent skill discovery

自动挂载和 skill_tool SHALL 使用相同发现目录、排序和同名优先级，SHALL 发现工作区 skills 与运行包技能，SHALL 支持 YAML 多行描述。

#### Scenario: Duplicate skill in workspace roots
- **WHEN** .agents/skills 与 skills 存在同名技能
- **THEN** 两个入口读取优先级最高的同一份技能，仅占一个挂载名额

#### Scenario: External application workspace
- **WHEN** 工作区是本地安装应用且无技能目录
- **THEN** 仍可从运行包发现技能并使用实际绝对资源路径

### Requirement: Real skill resources

技能指引 SHALL 使用所选 SKILL.md 所在目录的实际资源，SHALL NOT 声称缺失脚本存在或已经验证。

#### Scenario: Script path with spaces
- **WHEN** 技能位于含空格或中文的路径
- **THEN** 注入的命令引用实际存在脚本的绝对路径并带引号

### Requirement: Evidence bounded rule assessment

本地应用可行性调查 SHALL 先检查用户指定对象和真实接口；SHALL NOT 换成同名网站、生成无关模板或把鉴权错误当成功。最终结论 SHALL 区分静态校验、列表/搜索/详情与实际播放验证。

#### Scenario: Authentication error with HTTP 200
- **WHEN** 接口 HTTP 200 但业务返回鉴权失败
- **THEN** 记录业务失败，不宣称已有可用规则

### Requirement: Skill verification and completion agreement

明确命名的本地校验脚本 SHALL 在直接解释器运行、前台成功退出且无复合命令时产生质量执行收据。文本 Todo SHALL NOT 覆盖结构化任务证据；单产物简单任务 SHALL NOT 被强迫创建两套计划。

#### Scenario: Successful custom validator
- **WHEN** 生成产物后执行 node validate-rule.cjs 并真实退出 0
- **THEN** 运行时记录新的验证收据，后续真实修改仍使其过期；勾选 Todo 本身不算验证

### Requirement: Configured model visibility

有效单模型配置 SHALL 在模型列表通过 configured 标记可见；SDK SHALL 接受该模型且优先配置默认值。SHALL NOT 因无 accounts 数组而把可调用服务判为离线，也不得将配置完整冒充上游健康成功。

#### Scenario: Custom alias without accounts
- **WHEN** enabled、baseUrl、apiKey 和自定义 model 完整且 accounts 为空
- **THEN** /v1/models 返回该模型，SDK 可选，配置与响应均不新增明文密钥副本

### Requirement: Citable foreground process observations

真实成功退出的前台 run_command/verify_task_tool 输出 SHALL 可作为 Observation 被任务计划引用。SHALL NOT 仅因 ProcessControl 风险分类而丢弃事实；SHALL NOT 把普通命令输出提升为 Mutation 或 Verification，也不改变工具授权策略。

#### Scenario: Shell based local application investigation
- **WHEN** 模型通过已授权的前台命令读取目录和接口，实际 exit_code=0
- **THEN** 进度列表提供真实 receipt ID，可按验收条件引用；后台启动和失败命令不能满足此要求
