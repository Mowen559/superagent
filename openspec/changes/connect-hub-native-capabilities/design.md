# Design

## Decisions

1. 新增显式工作区构造入口，保留 with_model 的当前目录兼容入口。worker 将 effective_root 传入。Run 的 Prelude、磁盘清单和工作区动态插件都使用同一根目录，不修改进程全局 cwd。
2. Prelude 检索当前项目 Supermemory，保留来源与实际命中；读取失败不能伪装为空库。图谱无数据/损坏须以状态表示，不把数据库存在当成已索引。
3. `capability_status` 为只读工具，doctor 复用同一快照；报告图数据库、understand 语义状态、项目记忆、进程内 RTK 统计和需求引擎接线。doctor 不调用模型，不下载模型或索引器，不把 ready 当作已调用成功。
4. `requirement_engine` 使用绑定工作区与模型的 RequirementAnalyzer，Action analyze/plan 分别落既有 .deepcloud 报告。显式限制参数、超时；缺图谱/空图谱先失败，规划输出需为非空 JSON 数组。输出包含产物路径和范围，仅为需求分析与未来实施计划，不推进代码 Act/Verify。通过同步工具边界中的独立有界 Runtime 调用已有异步应用逻辑。
5. 原生项目工具在 worker 与独立 superagent 二进制注册一致；不强迫所有任务使用所有工具。了解架构时由 Prelude 与工具描述引导选择可用能力，图谱不可用时允许文件检查并诚实报告限制。
6. 验证使用实际 Hub CLI 子进程，隔离 AppData、项目、Session、模型设置。确定性 SQLite 测试数据只证明组件行为；有可用索引器才增加真实索引检查。模型凭据仅留进程环境，报告保留工具调用、退出码、最终文件/断言与实际限制。

既有需求引擎在数据库缺失时直接返回 Ok，导致 HTTP 调用方收到伪成功；本轮改为在调用模型之前返回错误，保留原 HTTP 错误结构。规划正文先校验为非空对象数组再写盘，非法输出不得覆盖旧计划。此行为修复须在契约台账双写登记。

Windows 实测：同一工程在含中文的路径下 definitions/calls 均报错且 dump 失败，换 ASCII 路径产生 23 个真实节点、23 条边；ASCII junction 指向原中文目录同样成功。适配器仅在根目录或缓存含非 ASCII 时建立独立 staging（可通过 HUB_INDEX_STAGING_ROOT 指定；默认原生索引器所在目录下的专用目录），先校验 staging 路径，再以 junction 只读索引原源码，使用 ASCII 临时缓存。子进程结束后验证真实 nodes/edges 数据再原子发布到 Hub 按原项目标识计算的缓存位置。清理先移除 junction 本身，禁止递归进入目标工程。CLI `hub index --project` 与分析服务共用适配器；原工程路径与源码指纹仍按真实工作区记录。

真实 worker 初测已成功调用图谱、understand 与项目记忆，但“只做调查”被词面意图误分类为 Execution，完成门禁要求无关改码。显式只读调查前缀须投影为 Investigative，并清除变更/验证工具建议；“先调查再修复”仍为 Execution。此修正不放宽验证要求或工具权限，不允许凭无关写入结题。

实际初测还暴露 RequirementAnalyzer 查询不存在的 nodes.summary 字段（真实索引使用 properties JSON）；改为读取真实 Schema，提供有工作区路径边界、字符预算与来源的源码摘录。模型错误不得以正文字符串伪装成功。最终收尾发现 manage_task 忽略其 workspace_root，已要求清单读写均使用选定工作区，并在真实 worker 验收中检查启动目录清单完全不变。

## Non-goals

不重写索引算法，不把 HTTP 规划接成自动 Apply，不修改模型权重或生产安装包，不为 casual chat 强制索引和写记忆，不运行旧 Supervisor 的全目录回滚。
