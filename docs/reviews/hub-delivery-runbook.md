# Hub P0–P2 无界面复跑

先冻结 `hub_delivery_evals.py baseline/suite`，所有输出使用新路径，不覆盖已存在的尝试。

```powershell
python scripts/hub_delivery_evals.py baseline --output docs/reviews/<new-baseline>.json
python scripts/hub_delivery_evals.py suite --output docs/reviews/<new-suite>.json
# 从 deepcloud-ui 执行，SDK 实际连接测试 Hub，不运行 Electron 窗口。
node scripts/hub-delivery-replay.cjs --base http://127.0.0.1:<test-port> --output ../super-agent/docs/reviews/<new-attempt>.json
```

P0 初次请求遇到 Hub 未运行；独立实例启动后，两条任务又因既有模型代理 8045 未监听而失败。用户恢复代理后，使用同一冻结二进制和模型再次执行，只读复核及边界修复均完成。原错误报告、重连结果及运行二进制身份分别保留。

正式预算见 `hub-delivery-budgets-v1.json`。总 HTTP deadline 包括嵌套工具/需求引擎时间，超时要取消对应任务及进程树，不能将已生成文件当成完成。context-usage 为上下文实际观测，不冒充上游总计费 token。三次业务重放前再次固定二进制、技能、验证器及任务；宿主在开始后代写业务实现必须单独记为 assisted。

P0 的旧二进制会自动在 origin 执行，边界修复测试因此命中原项目。P1 修正为隔离工作区后，必须改为对真实隔离工作区执行独立验收，同时确认 origin 不变；不能将 Apply 之前 origin 保持不变判成失败。独立测试文件保持冻结，执行目标通过明确参数传入。

留出案例在案例级隔离，P0 只探测三个公开目标是否可访问，不把其内容注入教导任务。60 个任务目前为冻结定义，未声称已全部执行。P0 模型前的网络故障也在记录中，不能静默删掉。

P2规则类当前预算为v3：40轮、1500秒。v2曾宣称48轮，但后续快照证明被既有40轮硬上限收紧，因此不能作48轮样本；该失败与原32轮失败均保留。prepare-rule显式传入`--model-turn-budget 40 --timeout-seconds 1500`；用新的输出名生成冻结条件，并从worker首轮快照核对生效预算。

独立试验每Run通过start_delivery_eval_service.ps1创建全新的数据及经验目录，仅用settings目录引用既有模型配置，不复制凭据。启动时传入相同的ModelTurnBudget，保存运行身份及配置哈希。三个Run使用三个不同的私有数据目录；不继承Session、不复制既有业务产物。

生成任务结束后还须调用verify_dmghg_package.py，仅复制交付的三个文件到新中文目录并重复实际运行、解码和故障输入检查。certify_p2_runs.py要求三个预定ID、独立Session、独立经验根、冻结版本及预算一致、完成状态与冷包结果全部通过；人工还须核对三份源码的来源、原生状态隔离及README陈述范围。不要只凭机械报告替代文档及源码复核。
# 生产源码快照

独立认证前运行 scripts/snapshot_delivery_inputs.py --output <新收据.json>。在收据所指的 stage/super-agent 内运行 hub_delivery_evals.py prepare-rule，仍将报告写入原 reviews 绝对目录。用该快照中的 start_delivery_eval_service.ps1 启动每个全新服务，用快照 deepcloud-ui/scripts/hub-delivery-replay.cjs 提交任务。冻结manifest包含每个副本及原位置指纹；仅副本参与运行中复核，不能因其他开发任务编辑生产树而切换本轮输入。node_modules及原应用资源仍按清单引用；不复制用户账户、设置凭据或应用状态。

v13以NODE_PATH引用安装依赖，源码副本内不放依赖目录连接。顺序运行脚本run_independent_delivery_series.py会在每个Run及冷目录验证完成后暂停到awaiting_manual_review状态；人工核对实际源码、Hub写盘来源、README依赖与证据边界后，在manual_review_path创建新JSON，包含task_id、passed、artifacts（文件名→原报告SHA）及审查依据。失败保持false，不修补业务文件或回填首轮结果。通过才启动下一次，最终certify_p2_runs.py必须传入三份--reviews，不能只凭自动通过报告认证完整交付。

v19起媒体解码最多三次，每次沿用同一实际lazy返回值及原始请求头、16秒期限；必须一次真正超过1秒，不能累加不足1秒的进度。报告保留所有decoder_attempts。原Run失败后宿主补测成功只能单列，不能回填。diagnose_delivery_decode.py仅用于只读补充诊断，其certification固定false，不可替代独立报告。

v20起完整规则验证选择不同于规则URL默认值的许可端口，实际注入MY_PARAMS.bridgeBase，原JSON不改写。报告记录declared_default_port、bridge_port；两者相同不满足配置切换验证。默认端口自动通过但备用端口失败须拒绝人工验收。certify_p2_runs.py同时检查Run和冷包的该项证据、真实解码进度及有界尝试记录。

