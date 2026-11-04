# Hub 交付证据增量契约 v1

实施范围：evolve-hub-autonomous-delivery，P0–P3已实施切片。本文先于代码登记；实际通过范围由 reviews 报告决定。

## 兼容性与状态所有权

规则技能通用PC执行入口：scripts/test-rule-live.cjs --rule <工作区JSON> --entry home|search|detail|action，可用--request <工作区JSON>传递page/keyword/targetUrl/input/actionCode/params。使用真实生产导入与执行，结果success仅表示该入口执行成功，data为空不代表业务通过；不认证GUI、Android、媒体解码或用户验收。测试数据库在当前工作区.superagent内隔离，规则文件越界及被执行期间修改须拒绝。PC导入参考为references/pc-rule-import.md。

用户交互等待：ask_user的外层工具时限遵循有效TimeoutSeconds（默认300秒、上限1800秒）加5秒结算余量，不再套用普通工具60秒。交互运行时仍拒绝0及超限值，父任务整体时限和取消不变。协议requestId/response及唯一终态不变。

SDK交互修复（2026-11-04）：POST /api/agents/tasks/{taskId}/interact的body为{action:"steer",response:string}或{action:"respond",requestId:string,response:string}。SDK兼容旧{steeringPrompt}并在传输前转换；旧{token,approve}不等于当前相关回答，显式拒绝而非猜测授权。失败HTTP或success不为true不得报告已送达。现有主UI的analyzeApi同构body保持不变。

补充指令在Run账本持久化并确认后更新current_run_facts.scope_revision；验证及其依赖步骤和物理收据失效，旧观察与修改事实保留。相同Run的旧检查仍计入发生过的调用，不能算新约束的当前有效验收。只读Run不因补充文本自动扩大权限，扩大工作模式使用新的获授权Run。

2026-11-04推进增量：TaskProgress快照可选新增current_run_facts（session_id/run_id、工具成功/失败/规范化验证计数、容量截断标记）；该摘要仅来自当前Run宿主收据，计数不等于不同文件数，也不证明命令内具体行为。工具结果与历史助手文本仅在LLM投影中附来源Run/current或historical/unknown标签，原始事件、SSE结果及工具ID配对不变。旧无Run记录标unknown。

TaskContract的mutation/verification步骤和实际变更可上调初始路由，不能降低已有执行要求或修改授权/宿主验收；不凭计划生成观察或验证事实。显式只读约束拒绝mutation计划和已识别原生变更/受控发布，任意shell仍需原权限策略。文档无宿主物理验收配置时保留原质量门禁。

不新增 HTTP 生命周期或第二个 completed 状态。SessionLedger 仍负责唯一终态；plan_task 的 task_contract 可选新增 delivery，旧 JSON 无该字段仍可读取。update_task_progress 的快照新增 delivery_evidence；现有字段不删除。旧消费者忽略新增字段即可，缺少交付认证证据不自动显示业务已认证。

SDK AgentRunRequest 可选新增 resumeSessionId，原样传递给 HTTP 已支持的 resumeSessionId。继续尚未 Apply 的隔离成果时，project 应指向上一任务的隔离工作区，服务会从其创建下一份工作副本；不把原项目目录未 Apply 的文件视为已经修改。恢复 Session 只提供历史对话，每个 Run 重新建立任务范围和交付证据。

## 任务交付声明

delivery 包含 version=1、scope（read_only / implementation）、target 及 artifacts。每项 artifact 包含 id、相对工作区 path、required（默认 true）和可选 validator（受信任配置中的 ID）。不允许绝对产物路径、父目录跳转、重复 ID 或只读声明中要求实施产物。该声明是模型计划，不是事实证据。

运行时可从宿主只读配置预置必需交付；模型不能通过省略声明、改目标或改成可选来移除预置要求。后续计划修改保留原声明，不在增量修订中隐式弱化验收。

运行绑定的plan_task在返回planned之前，须对照当前Run的宿主交付要求与既有必需项校验声明。不相符时立即返回包含具体字段、期望值及实际值的错误，不清除旧计划或进度，不把错误推迟到verify_delivery。纯结构规划工具继续兼容没有宿主交付约束的旧调用。

对于已存在的绝对文件系统目标，允许解析到同一实际目录/文件的路径写法，并将计划及物理收据统一为宿主原目标字符串；不同目录、不可解析的路径及URL/逻辑目标仍不得通过该等价判断。此处理只消除路径分隔及别名差异，不修改目标内容、校验器或资源指纹要求。

## verify_delivery 工具

增量实施（2026-11-03，实际通过范围见对应评测）：宿主profile可选actions数组，每项{id,source,executor}；source引用已声明且有业务validator的产物，executor引用validators中的固定程序/脚本。perform_delivery_action({action_id})不接受命令、URL、正文或模型指定源文件。执行前复核source当前Run业务收据、目标/路径/SHA及宿主/执行器指纹；每动作最多调用一次，超时/失败同样消耗额度。执行器argv新增{sha256}绑定来源哈希，最后行仍须passed/target/checks。动作结果不替代其他交付物的业务验证。未配置actions的旧profile行为不变。

显式artifact_ids子集成功可返回status=success及completion_ready=false，记录本地通过事实；最终结题仍校验所有required。模型每轮优先收到本地失败/缺失/待验指引。启用actions时，对已知云端点/导出CLI的shell及工作区脚本调用在进程启动前拒绝；此检测不是任意shell的网络安全沙箱。合法发布由固定适配器读取已验证文件，最多一次create，保存公开ID后读回，失败不另建ID，不保存auth_code等管理凭据。

输入：可选 artifact_ids；省略则检查所有声明产物。工具从实际文件生成绝对路径、SHA-256、字节数、Session/Run 和来源。需要 validator 的产物由宿主注册的固定程序及脚本执行；模型不能提交自己的验证命令或成功结果。

验证器配置来自 HUB_DELIVERY_PROFILE_PATH，含固定目标、交付声明及验证器的绝对 program/script、各自 SHA-256、参数模板和时限。配置自身及程序/脚本哈希在使用时重新检查；配置/验证器不得位于模型执行工作区。参数以 argv 传递，不拼接 shell。验证失败、超时、目标不符、输出不合法或产物执行期间变化都不产生通过记录。

受信任验证器最后一行输出 JSON：passed:boolean、target:string、checks:非空数组；只有真实 exit_code=0、passed=true、目标一致且输入产物指纹未变才通过。业务语义由冻结的独立验证器负责，不由名称含 validate 或普通进程 exit 0 推断。

产物记录和验证记录仅属于当前 Run；完成时重新核对哈希。未变化的已验证产物可复用本 Run 记录；改动后旧验证失效。必需项失败阻止业务完成，可选项失败显示限制。只读任务不需要制造文件变更。

## 工作区及大结果

2026-11-04范围消费增量：自动Skill命中仅为目录推荐，正文通过skill_tool按需读取；完整点名的Skill优先加载正文。候选与正文都不能将通用全流程变成本轮额外目标，Skill读取结果增加scope_guidance，资源content保持原文。worker与Run Prelude复用同一发现排序策略。完全相同TaskContract重传为无变更，plan_revision、已有步骤收据和验证不被重置；有变化的完整替换仍清理绑定，新增要求优先使用revise_task_plan。

受信任宿主已声明physical_artifacts时，默认认知推进与验证阶段展示改为修改必需文件并用verify_delivery验收；不再同时要求验证代码行为。该模式来源于宿主profile而非文件后缀/任务关键词，不限制合法工具，不跳过显式子任务或物理指纹门禁；quality_checks模式保持原验证指导。

该物理推进指导独立于旧意图引擎的TaskKind：即使编辑措辞被分类为Conversational，也仍提供文件交付指导。仅替换系统内置的默认编译测试anchor展示，自定义约束和显式TaskContract检查保留。全部物理收据新鲜时提示完成剩余显式步骤或直接答复；产物改变后恢复待验提示。历史README声明不等于过去测试证据，不应在文档修订中把旧AST/平台认证当成本Run新增验收。

2026-11-04恢复增量：固定宿主profile可选completion_policy（默认quality_checks；physical_artifacts仅接受非空必需物理产物、无业务validator且无actions），以及allow_internal_diagnostics（默认false）。物理验收只放行默认RunPlan/DoD步骤，保留显式子任务、当前Run/路径/哈希和profile指纹门禁，domain_verified及工具verification事实不被提升。

网关按Run/规范化指纹/工作区修订记录失败类别。参数、缺依赖失败原样最多一次；网络、业务及未知失败原样最多两次，无关成功读取或探针不刷新额度。失败或恢复状态转换时附加recovery_state（固定类别、次数、建议和是否停滞；不保存命令正文/原始错误），无恢复状态的正常成功结果保持原结构。该字段由网关专有生成，工具自带同名提示会被移除。原样预算耗尽在执行前返回bounded_recovery_exhausted；修改参数或成功工作区写入允许新尝试，新Run清理状态。授权状态由原策略存储每次重查，待授权记录不会阻止后来正式获准的调用。

当前Run的.superagent/traces默认不作为普通业务输入：直接路径和已识别的显式shell访问返回internal_trace_not_task_evidence，改用read_tool_result读取同Run原文及真实业务文件。固定宿主profile可显式授权内部诊断；该规则不构成任意shell或操作系统安全沙箱。

通用云适配器支持宿主固定参数--reuse-only：只读复验现有匹配来源哈希的公开回执；即使回执文件缺失也直接拒绝，绝不进入创建分支。该参数由宿主profile固定，模型不能在受控动作中移除；用于成功发布后的文档收尾复验。

2026-11-04发布后收尾指引：受控动作一经尝试，动态指导不再提示调用该动作；展示尚未存在的required产物ID及相对路径，要求保留既有公开ID、完成真实剩余文件并以verify_delivery全量复核。文件齐备也不等于业务验证齐备，必须继续通过原完成门禁。该提示不恢复动作额度、不允许伪造动作输出。

2026-11-04技能资源读取路由：view_file及读别名的AbsolutePath若被当前注册skill_tool识别为所选技能根内的真实文件，网关转换为skill_tool({action:"read",skill_name,resource_path})并保留start_line/end_line（兼容StartLine/EndLine）。资源根及链接经真实路径检查，读取仍受SkillTool大小/边界限制。收据工具名为skill_tool，结果附resource_read_route说明原工具；不会执行脚本。普通文件、未选影子技能、逃逸链接和所有写入不转换，旧文件工具的工作区权限保持有效。

origin 是只读知识来源，不能取代运行根。shell cwd、文件操作、清单及交付产物绑定隔离工作区；原项目记忆/图谱可通过只读来源检索。工具大结果写入和 read_tool_result 读取必须共用 Session/Run 作用域，跨 Run 旧引用无效。

Shell 的 RTK 压缩和 verify_task_tool 的尾部摘要同样受原文可恢复契约约束，即使摘要低于 Gateway 长度上限，也须先保存未经摘要的 stdout/stderr，返回 full_result_reference；原文不直接注入模型上下文。内部原文传输字段由 Gateway 消费并移除。保存失败须明确返回恢复错误，不能显示一个可用的空引用。退出码、失败状态及现有摘要保持原语义。

独立评测使用生产运行库、技能、验证器和候选程序的逐文件等哈希快照。清单保留原位置与复制指纹，执行只引用冻结副本；并发开发不能改变本轮测试输入。副本沿用实际生产模块及依赖，不得改成简化执行器。每轮项目、Session及经验目录仍独立。

## 校验样例

- 旧 TaskContract 无 delivery：仍按既有收据门禁处理。
- 模型宣称写入但文件不存在：交付检查失败。
- 探针成功但未生成规则：规则必需项失败。
- 验证通过后改规则：原验证失效。
- 只读复核及可选附件失败：分别按调查证据和必需项判定，不要求无关写盘。
- 生产规则执行失败但静态脚本通过：业务验证失败。

取消/恢复维持原有契约；新增验证进程必须有时限和进程树清理。移动端、发布及分享认证不由 PC 运行记录推导。

Windows json 协议执行进程放入随句柄关闭而清理的 Job Object；取消、超时和正常退出均回收其后代。隔离目录准备失败直接报告失败，不回退到原项目执行。此处平台保证由 Windows 进程树实测认证，不能推断其他平台已经验证。

收到 SuperAgent 明确的 completed/failed/error 终态标记后，适配器结束收集并清理该进程树，不再等待后台后代关闭继承管道。领域验证失败须把有界结构化 error 与退出码一并返给模型，不能只返回 exit 1。Windows PowerShell/Python 的文本输出统一使用 UTF-8，避免中文证据在压缩前丢失。

实际媒体解码失败须指出固定采样位置及已观察到的数值状态（视频宽高、time-pos或缺失），不能只说未通过，也不能回显媒体URL或请求头。保持真实解码超过1秒的门槛；旧探针默认超过0.2秒的结果不等于该门槛已通过。补充诊断及网络重试不能回填独立Run的失败终态。

外部媒体解码允许在评测开始前冻结最多三次尝试，每次沿用同一实际lazy返回的URL、请求头、解码器和16秒期限；不重新挑选影片、不改地址、不累计多次的播放进度。至少一次须在本次解码中真实超过1秒，否则失败。成功及失败报告均保留全部decoder_attempts；总验证时限和Run预算不扩展。该策略仅用于后续新冻结版本，旧失败不改写。

P2 v5 的受信任 profile 分别认证规则及桥接产物：规则走生产运行库和真实解码；桥接在仅复制三个交付文件的新目录中检查依赖、并发、错误输入、服务端读取超时及进程关闭。桥接 HTTP 验收契约为 GET /ping、GET /channels 与 POST /call（command/payload），账户命令必须被拒绝。不完整请求须在8秒内由服务端响应或关闭；客户端自己的超时不能算服务端通过。两验证器均通过后才满足本次交付证据门禁。

仅本机监听同样适用于CLI覆盖参数：不存在--host选项或显式拒绝非127.0.0.1地址均可，不能只以默认绑定地址推断任意参数都受限。负例验收在独立子进程拦截Python socket.bind，阻止任何实际绑定后观察请求地址；默认参数必须尝试127.0.0.1，非本机覆盖不得尝试绑定。验收脚本不代写桥接实现。

规则的动态桥接配置须实际验证：从规则首页URL识别声明的默认端口，在18000/18001/18002中选择不同的可用端口启动交付桥接，通过MY_PARAMS.bridgeBase传入生产运行库，原规则文件保持不变。首页、搜索、详情、lazy及媒体均须沿该端口完成；报告保存默认端口与实测端口。只有默认端口可用时不能冒称配置切换通过。失败信息须带这两个端口，帮助区分原URL未重设基址和真正业务错误。

不完整HTTP请求的8秒期限从发送该请求时开始计算，必须包括并行健康请求耗时；不能在并行检查结束后再重新计时8秒。服务端读取时限应留出调度余量，例如5秒。客户端超时或在期限后才观察到关闭，均不能算期限内通过。

受规范化收据认定为成功的真实验证同时提供观察证据，可满足调查Run的inspect步骤；验证失败或普通exit 0不产生此效果。该观察不替代生成任务的实际写盘步骤，也不能覆盖后续写盘导致的交付指纹失效。验证器stdout必须为UTF-8，包含中文路径时同样适用。

当前Run默认轮次上限为40，HUB_AGENT_MAX_MODEL_TURNS只可收紧，不能扩展既有上限。评测配置必须在该范围内，并从worker首轮快照中的Turn Budget核对实际生效值，不能仅凭启动环境变量宣称使用了更大预算。

规则Skill模板须与生产PC入口一致：统一分页URL不附加普通`[网址]`后缀；搜索使用searchFind；首页/搜索/详情使用MY_URL取得当前地址；选集以@lazyRule携带实际可执行脚本。模板遇到未解析的播放器配置应报错，不能返回原网页URL冒充媒体。模板本身仍需按目标选择器适配，不能由通用示例推断任意网站或Android已经通过。
# 持久暂存与基础设施中断

生产PC规则运行兼容性补充：class_name/class_url按位置对应，空的URL值可表示“全部”，不得独立过滤URL空项后错配名称。request的真实资源路径含普通空格时编码为%20，不能当作无关后缀截断；仅对明确粘连的空白+hiker://后缀保留原容错。原有私网/出网安全校验保持执行。此前NASA错位/详情无播放的测量保留，原因须按宿主兼容性问题另行归档。

长时间独立验收的源码快照、服务、Run、冷包和轨迹应置于工作区内专用持久暂存根。编排进程仅在自身及子进程作用域覆盖 TEMP/TMP，不修改用户全局环境；各 Run 继续使用独立空目录和经验根，不复用旧业务实现。报告记录实际物理位置及哈希。若系统临时区文件在验收后消失，保留原自动结果并另记基础设施中断；没有完整文件及来源时不得补签人工通过或认证三连成功。新系列重新冻结，不能拼接旧系列成绩。

