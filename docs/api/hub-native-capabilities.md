# Hub 原生能力诊断与需求引擎契约

日期：2026-10-30。变更规范：[接线与验收](../../openspec/changes/connect-hub-native-capabilities/design.md)。

## CLI

- `hub index --project <path>`：使用原生 codebase-memory 索引器构建真实图谱，校验后发布到原项目缓存。成功返回 `status/indexed`、`project`、`database_path`、`nodes`、`ascii_path_adapter`、`duration_ms`；失败非零退出，并保留原有效缓存。该动作不请求模型。
- `hub doctor --project <path>`：输出 JSON 能力快照，包含 `workspace`、`graph`、`understand`、`supermemory`、`rtk`、`requirement_engine`、`execution`。诊断不请求模型、不生成索引。图谱状态为 `missing | invalid | empty | ready`，ready 仅表示 Schema 与节点可读，不是业务任务认证。
- `hub superagent-worker --project <path> --model <model> --tool-policy guarded --prompt <task>`：实际执行入口。`hub ask/chat` 仍为直接模型对话，不能以其文字输出证明工具被执行。

Windows 非 ASCII 路径使用 ASCII staging/junction 索引原源码，验证后发布到真实项目缓存。可用 `HUB_INDEX_STAGING_ROOT` 指定可写 ASCII staging 位置；默认使用原生索引器旁的专用目录。定制索引脚本不强制使用原生二进制兼容适配。

## 原生工具

`capability_status` 无必填参数，返回当前工作区快照及实际已注册工具名；不会隐式测试模型路线。`project_index` 的状态检查增加 `ready/readiness`，数据库存在但为空或损坏时不可报告 ready；inspect 操作在无有效图谱时明确失败。

`requirement_engine` 接受 `Action: analyze | plan` 和 1～4000 字符的 `requirement`。工作区及模型由宿主绑定，调用方不能提交其他项目路径或 API Key。前台最大 45 秒；成功返回 `status: generated`、`action`、`artifact_path`、`scope: analysis_or_future_plan_only` 及有界摘录。

- analyze：调用既有 RequirementAnalyzer，写入 `.deepcloud/requirement_impact.md`。
- plan：写入 `.deepcloud/execution_plan.json`，内容须为非空对象数组。

这些产物只表示分析与未来计划，不表示源码已变更、测试已通过或计划已实施。缺失、空或非法 Schema 的图谱会在请求模型前失败；非法计划在写入前拒绝，保留旧计划。模型调用错误不能被保存成成功的“LLM Error”报告。

## 既有 HTTP 行为修复

`POST /api/reverse/analyze-requirement`、`POST /api/reverse/plan-requirement` 的请求字段和既有成功/错误响应结构保持不变。底层过去在数据库不存在时返回 Ok，造成伪成功；现在会进入既有错误分支，返回 HTTP 500 与 `{ "error": ... }`，不会返回 success=true。图谱摘要从真实 `nodes.properties` JSON 读取，不再查询不存在的 `nodes.summary` 字段。

## 观测边界

RTK 的源码过滤与终端压缩已经内联于 Hub。`rtk_metrics.tracker_stats` 为当前进程实际调用指标，与独立 `rtk gain` 的持久统计不同。短输出可能调用了压缩器但节省为零，不能伪造节省比例。understand 的原生 SQL 理解能力和外部 Understand-Anything 元数据分别报告，外部元数据缺失不等于已完成插件认证。
