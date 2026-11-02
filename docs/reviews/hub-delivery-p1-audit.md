# Hub P0–P1 实施与验收

范围：用户同意执行 evolve-hub-autonomous-delivery 的 P0–P2；本文先记录已完成的 P0/P1，不将进行中的规则生成认证为通过。

## 事实与修改

- 已冻结 20 案例、60 条自然表达、10 条关键任务和四个案例级留出集。定义不等于已执行；本阶段真实在线样本为复核、边界修复及会话延续。
- 模型沿生产 SDK → HTTP/SSE → 真实 worker 调用 gemini-3.8-flash-high。初次 Hub/上游代理未运行的失败保留，用户恢复代理后再测；没有复制或回显密钥。
- TaskContract 可选 delivery v1，声明目标及必需/可选文件。运行时生成路径、SHA-256、Session/Run 和验证器身份；文件变化使旧验证失效。
- verify_delivery 执行宿主固定程序/脚本，核对配置、依赖和目标。普通 exit 0、validate 文件名和模型文字不能替代领域验证。该工具同时注册并投影给模型，避免只注册不暴露。
- 业务领域门禁由宿主配置或明确交付声明接入；本轮只为指定规则业务准备冻结 profile，不声称所有普通聊天已经自动获得领域认证。
- worker 不再自动上升到 origin；只读知识源和执行根分开。原项目记忆可检索，新记忆写当前工作区；隔离准备失败直接失败，不回退原项目。
- 大结果网关与读取工具共用可轮换 Run 存储，引用带命名空间盐，跨 Run 不能读旧引用。验证实际大输出末尾诊断可读回。
- Windows json 协议进程加入 Job Object，取消、超时、正常结束均清理后代；实际驱动/真实后代进程测试通过。SDK 新增可选 resumeSessionId 透传，继续隔离成果需显式使用上次工作副本。

## 真实结果与保留的失败

| 记录 | 结果 |
|---|---|
| hub-delivery-p0-live-initial.json | Hub 未运行，未取得配置模型；保留 |
| hub-delivery-p0-live.json | 原模型代理未监听，两任务在工具前失败；保留 |
| hub-delivery-p0-live-reconnected.json | 复核与合成边界修复通过，约 26/67 秒；旧版会改 origin |
| hub-delivery-p1-live.json | 实际仍命中旧测试端口，独立隔离验收识别 origin 被改且副本未修好，修复任务判失败；不能作候选成功证据 |
| hub-delivery-p1-candidate-service.json / candidate-live.json | 固定候选二进制、独立端口；复核与隔离修复通过，origin 保持不变 |
| hub-delivery-p1-continuation.json | 三任务通过；同一真实 Session 恢复，上一轮修改保留，下一轮只读不改代码 |
| hub-delivery-restart-receipt.json | 新候选加载此前 completed 任务，完成事件仍为一次 |
| hub-delivery-driver-lifecycle-initial.log | 新测试误将 backend timeout 期待为 failed；原判保留，按既有协议修正测试期望 |

没有修改既有失败报告或用候选哈希回填旧版本执行身份。测试二进制身份分别保存在对应 service.json。

## 工程与负例

- 全库 Rust：525 passed、0 failed、2 ignored、1 filtered；排除既有慢速全仓审查。见 hub-delivery-p1-final-rust.log。
- SDK 定向回归通过，TypeScript 全量检查通过。见 hub-delivery-sdk-regressions.log、hub-delivery-typescript.log。
- 生产 Hiker 引擎无界面入口走过 home/search/detail/lazy，并拒绝故意报错、空数据、重复第一页和替代来源。见 hub-delivery-runtime-negatives.log。
- 实际本地 HTTP 200 垃圾数据不能通过安装 libmpv 的空音视频输出解码，空媒体及 410 过期也被拒绝。见 hub-delivery-media-negatives-v2.log。
- 安装应用的 28 个可执行/Lua 资源指纹与 P0 相同；未执行桌面自动化。原应用账号状态不属于允许读取的数据，探针使用新临时状态。

## 适用边界

上述在线改码样本仍是小工程，不代表复杂仓库普遍成功。前端窗口、完整安装包、手机独立运行及自修改升级均未认证；不会自动进入 P3–P7。候选使用独立测试端口，未将开发验证误称为已替换用户安装版。

P2 首批采集遇到测试器脱敏 JSON 缺陷，按基础设施失败保留记录并取消对应任务；修复采集器、重新冻结后再测试，不能将该失败解释成模型不具备生成能力，也不能将未取得的结果计为成功。

## P2 反证后的追加修复

首次完整采集的 H01 写出了三个文件，但生产一级页为空，32 轮预算耗尽，后台后代持有管道又将终态拖至 HTTP 总超时。该失败保存在 hub-delivery-p2-live-v3-part1.json，不能计为完成。P1 的 2.4/2.8 已重新打开，追加结构化领域错误反馈、明确终态停止收集、中文输出和成功经验写入作用域修复；最新认证以 hub-delivery-p1-recovery-rust.log 及后续真实重放为准，前述 525 次通过不覆盖尚未验证的新改动。

P0 实体输入补充为 20 组、44 文件及独立验收事实表，见 hub-delivery-suite-v3.json。初版记忆夹具缺 stats 必填字段，保留原输入并另建修正版；实际 doctor 验证修正版为 ready，未据此宣称记忆迁移任务已经模型验收。

## 2026-11-02 继续复核

- v4 带反馈任务 `e2d27ec5-24d7-4875-b07f-6be8b50b4e8d` 因32轮预算耗尽失败，569686毫秒结束，唯一 status 为 failed；没有拖至900秒，也没有将文件存在误判成业务通过。该任务没有调用 verify_delivery，独立验收仍报媒体请求403，不能计为 P2 成功。
- 同一候选二进制 `19FED5BB5EE742E1775C2913104481FD09E878AABDBBB06797ACA3FF69C82F5C` 重启后仍读到上述 failed 任务。服务身份另存 hub-delivery-p2-v4-resumed-service.json，未覆盖原运行身份。
- 新纠错任务 `4139605f-7eaa-4b6e-8dd8-0689f03ba63f` 首次工具即 verify_delivery，真实返回结构化 `trusted verifier failed: exit Some(1); HTTP Error 403: Forbidden`；模型已收到完整领域错误。见 hub-delivery-p2-coached-v4-20261102.json。结合最新526项 Rust 回归（0失败、2忽略、1过滤）和指纹失效等既有专项，2.4 的补充修复通过。
- 独立定位记录 hub-delivery-p2-403-location-20261102.log 证明失败位于实际 lazy 的媒体请求。原生同剧首集返回空 headers，原样请求得到206及2048字节，见 hub-delivery-p2-native-media-20261102.json；业务规则的多加请求头分支仍交由 Hub 修改，宿主未补写业务实现。
- 后续真实Run在444952毫秒以completed结束，唯一status为completed；相同二进制的新服务再次载入该completed任务，完成事件仍为一次。见hub-delivery-p1-recovery-restart-20261102.json。结合526项回归中实际取消、超时和后代进程终止专项，2.8追加修复通过；P1收口不代表P2已经通过。

后续反证：v6纠错Run的规则及桥接领域验证都通过，但调查型Run的RunPlan只认可ReadOnly/Network风险，未认可成功验证收据，造成Gather authoritative evidence误拦截。2.7再次打开，补真实Gateway→CognitiveRuntime回归；首次误用了--bin hub而运行0个测试，日志保留，不算回归通过，实际测试应运行--lib。

追加修复：新测试先实际复现同一误拦截，再让调查Run将规范化成功验证也视为观察。反例确认domain_verified=false、失败验证及生成任务只有验证没有写盘均不能通过。库级回归527通过、0失败、2忽略、1过滤，见hub-delivery-p1-authoritative-evidence-filtered.log；此前误用过滤名导致全仓慢审查运行超60秒后停止，该中断日志保留，不算完整回归。新二进制构建成功，SHA-256为801FC9AD1AB1D62429E22155D5481B55FA2C0C7122290A4D602A6848E38F2D2E。

真实SDK复测见hub-delivery-p1-investigation-v7.json：全新数据目录及Session，只调用一次verify_delivery，无业务写盘，54524毫秒、2轮完成，独立验收通过。2.7补修完成，P1各项恢复通过。
# RTK 原文补修

后续v13复现同一安装目录的双分隔写法被误当目标改变，同时计划工具过早确认、错误延后且不含字段。已增加Run绑定的计划校验；既有绝对路径解析到同一物理目标时统一回宿主原字符串，其他目标仍拒绝。真正不符的目标、范围或必需产物在planned之前报告具体字段，失败不清除既有计划/收据。原无宿主约束规划调用继续兼容。

新增四项实质回归覆盖目标字段诊断、原状态保留、Windows双分隔与正斜杠等价路径、不同实际目录/缺失目标/URL不可混同，以及绑定规划工具拒绝弱化计划。完整既有范围回归532通过、0失败、2忽略、1沿基线过滤，见hub-delivery-p1-plan-feedback-path-regressions.log；2.2恢复关闭。早一次编译因D盘空间不足失败，保留hub-delivery-p1-plan-feedback-regressions.log。使用Cargo当前包级清理回收19.4GiB，源码、报告和冻结Hub副本未删除；本轮重建hub及superagent两个原有可执行目标。

v10真实报告证明Shell的SmartLog摘要没有原文引用，见hub-delivery-p1-rtk-v10-forensic.json。现由Shell保留未经压缩的stdout/stderr内部载荷，Gateway在注入模型前消费并移除；即使摘要不足长度阈值也持久化原文并返回引用。verify_task_tool的尾部摘要同样保留完整流。保存失败明确报告恢复错误，退出码与领域状态不因保存变成成功。

实际LocalShellProvider产生220行源码，函数签名在压缩摘要中消失，从Gateway返回引用可完整恢复，换Run后拒绝旧引用；定向测试通过。Rust回归528通过、0失败、2忽略、1沿既有基线过滤，见hub-delivery-p1-rtk-regressions.log。2.5重新关闭；构建及真实业务重复性另记，不由单元回归宣称P2完成。

