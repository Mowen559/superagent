## ADDED Requirements

### Requirement: Consistent Run workspace

原生 AgentLoop SHALL 使用显式传入的工作区读取 Prelude、清单和项目记忆，SHALL NOT 回退到另一个进程目录或全局记忆库来代表项目事实。

#### Scenario: Worker launched elsewhere
- **WHEN** Hub 从其他目录启动并传入项目路径
- **THEN** 前置上下文、记忆和文件工具均使用该项目，不读取启动目录中的同名文件

### Requirement: Honest native capability readiness

doctor 与原生状态工具 SHALL 区分不可用、未初始化、就绪及错误，SHALL NOT 仅以数据库文件存在宣称 understand 可用。只读诊断 SHALL NOT 发起模型请求或生成索引。

#### Scenario: Missing or empty graph
- **WHEN** 数据库缺失、Schema 无效或没有节点
- **THEN** 图谱与 understand 不可标记为 ready，并返回可操作原因

### Requirement: Callable requirement engines with bounded artifacts

原生 worker SHALL 注册需求分析与规划工具并绑定当前工作区及选择的模型。产物 SHALL 明确为分析/计划，SHALL NOT 成为代码变更或验证收据。无索引或非法 JSON 计划 SHALL 失败而非声称完成。

#### Scenario: Plan generation
- **WHEN** 原生 Agent 调用需求规划
- **THEN** 执行现有规划器并返回经过结构校验的真实产物路径，保留既有 API 行为

### Requirement: Production worker verification

验收 SHALL 进入 hub superagent-worker，记录真实工具调用与独立结果检查。固定夹具、模型路线错误、缺失索引器与未运行能力 SHALL 单独记录，SHALL NOT 以组件成功推断整体业务已认证。

#### Scenario: Forward code task
- **WHEN** 对临时工程提出代码修改要求
- **THEN** 验收同时检查真实文件内容、模型发起的测试与评测方独立测试

#### Scenario: Reverse understanding task
- **WHEN** 提出代码理解与项目记忆查询要求
- **THEN** 验收检查工作区来源、图谱/understand 调用、记忆命中与真实源码依据

#### Scenario: Explicit investigation without code changes
- **WHEN** 用户明确以只做调查、只做分析等前缀限定任务
- **THEN** 运行时按调查任务检查事实与结论，不要求无关的代码变更；先调查再修复的任务仍需修改和验证

### Requirement: Windows indexing path compatibility

Windows 索引适配器 SHALL 为非 ASCII 根目录或缓存提供独立 ASCII junction/cache，索引原工作区源码而非生成假图谱。SHALL 在数据库验证后发布到原项目缓存，SHALL NOT 修改或删除原源码。staging 不可用时 SHALL 明确失败并保留原图谱。

#### Scenario: Unicode project path
- **WHEN** 原生索引器无法读取含中文路径而 ASCII 别名可用
- **THEN** 服务与 CLI 通过同一适配器产生可查询的真实图谱并保留原项目归属
