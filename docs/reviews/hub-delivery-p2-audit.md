# Hub P2 真实规则交付验收

截至2026-11-02，P2尚未整体通过。已取得Hub自行编写和带反馈修复后的真实业务成功证据；桥接超时及三次独立生成仍须通过。模型为gemini-3.8-flash-high，沿生产SDK、真实HTTP/SSE及worker执行；全程未操作桌面。

## 尝试记录

| 记录 | 分类与结果 |
|---|---|
| hub-delivery-p2-live-v2.json | 采集器嵌套JSON脱敏缺陷，基础设施失败；取消活动任务，保留原结果 |
| hub-delivery-p2-live-v3-part1.json | 首次自然任务写出三个产物，但规则lazy错误、耗尽预算；不通过。验证器端口缺陷另有修复记录，不回填旧成绩 |
| hub-delivery-p2-coached-v4.json | 首次带反馈纠错仍耗尽32轮，媒体请求403，不通过；569686毫秒正确结束 |
| hub-delivery-p2-coached-v4-20261102.json | 第二次带反馈纠错通过规则及真实解码；444952毫秒、32轮、31工具调用，不能计为独立首次成功 |
| hub-delivery-p2-package-v4-20261102.json | 三文件中文新目录中的规则及解码通过，并发读取通过；不完整HTTP请求8秒未关闭，整体包复核失败 |
| hub-delivery-p2-bridge-negative-v5-20261102.json | 新桥接验证器再次拒绝旧产物的无读取超时问题，负例保留 |
| hub-delivery-p2-coached-v5.json | Hub已修复桥接及README，但新验证器stdout编码错误；宿主取消，按基础设施失败保留，不计独立成功 |
| hub-delivery-p2-package-v6-reviewed.json | 对Hub修复的三文件独立复核：中文新目录规则、解码、并发、超时、故障输入及关闭均通过 |
| hub-delivery-p2-coached-v6.json | 规则及桥接验证均通过，调查Run门禁却误拦截权威证据，最终failed；原结果不回填 |
| hub-delivery-p2-independent-v6-part1.json | 原计划独立第1次；因运行时误拦截需要修复而取消，不能计为成功 |
| hub-delivery-p2-coached-v7.json | 新二进制下173480毫秒、11轮完成，规则和桥接验证通过；人工文档复核仍发现列表/搜索分页混淆，继续纠正，不计独立成功 |
| hub-delivery-p1-investigation-v7.json | 新Session只调用一次受信任验证，54524毫秒完成，证明调查Run误拦截已修复 |
| hub-delivery-p2-independent-v7-part1.json | 真正独立生成并自行修正分类错误，规则/桥接/媒体通过；但第32轮验证后耗尽预算，872303毫秒以failed结束，不计成功 |
| hub-delivery-p2-coached-v7-bounds.json / coached-v7-package.json | Hub纠正搜索/列表分页及详情字段验收边界后，三文件冷目录全链通过；作为带反馈参考交付，不计独立成功 |
| hub-delivery-p2-independent-v8-part1.json | 启动身份宣称48轮，但worker实际只允许40轮；742621毫秒失败，仅写出bridge。配置声明不符与调查过长均保留，不计成功 |
| hub-delivery-p2-independent-v9-part1.json | 40轮、1191896毫秒失败；已写三个文件，但沿用标准模板普通方括号首页后缀，原生桥接返回500，规则首页为空 |
| hub-delivery-p2-independent-v10-part1.json | 修正通用模板及冲突Skill约束后，再次冻结相同40轮/1500秒条件；独立第1次进行中 |

## 已有业务证据

成功Run为`4139605f-7eaa-4b6e-8dd8-0689f03ba63f`。Hub写出的rule、bridge和README哈希及工具来源见hub-delivery-p2-authorship-v4-20261102.json；宿主未编写这些业务源码。后续教导使用原样副本，并明确属于纠错任务，不能混入独立任务。

- 生产Hiker运行库通过分类20、21，两个关键词仙逆、斗破，第一页及不同的第二页；同次上游核对详情及前/中/后选集，观测165条选集卡片。
- 实际lazy返回值发起媒体请求，再由已安装libmpv以vo=null、ao=null解码；观测1920×816、aac、time-pos=1.002秒。只说明本次样本，不外推全站画质和长期稳定性。
- 既有真实HTTP垃圾数据、空流和410过期负例不能通过，见hub-delivery-media-negatives-v2.log；损坏规则、空列表、重复分页及替代来源由生产引擎负例拒绝。
- 额外请求头导致403的定位、原生空headers返回206，以及Hub收到实时教导的决策快照指纹均已保留。实时交互返回200本身不算消费证据；hub-delivery-p2-live-coaching-consumed-20261102.json记录了快照确实包含教导内容。
- 同一候选二进制重启后读取到completed任务，状态事件仍只有一次，见hub-delivery-p1-recovery-restart-20261102.json。

## 剩余门槛

v5固定规则验证器及桥接验证器，两项均是宿主只读程序；规则验证上限125秒、桥接45秒，总工具预算内执行。新增桥接检查不降低旧规则、数据或解码标准。只有桥接故障输入、服务端超时、关闭与冷目录包通过，再以相同冻结条件从三个空项目连续独立生成并全部验收，才能勾选3.9。

v6修复验证器UTF-8输出，旧无超时产物仍被拒绝；真实修复产物冷目录实测慢请求约4598毫秒关闭，进程树关闭约183毫秒释放端口。随后发现调查Run仅按工具风险识别观察，遗漏已成功的verify_delivery；P1 2.7重新打开，修复后必须重新构建、冻结并重开独立测试。

交付文档还需严格校正范围：枚举8个频道不代表每频道都已完整验证，前/中/后选集名称核对不代表三集都已解析媒体或解码。README不能沿用超过实际验证器覆盖范围的旧陈述。

桥接独立通过并已保存旧无超时负例；3.6完成。独立重复性测试另使用每Run全新HUB_APP_DATA_ROOT，settings通过目录引用既有配置，未复制凭据。EvolutionMemory初始为空、经验写入各自私有目录，避免第一轮的新经验改变下一轮输入。脚本及判定器身份另存hub-delivery-p2-v7-isolated-preflight.json；模型、技能、二进制及业务验证仍沿v7冻结记录。

3.8参考交付通过：Hub在`00d56d4e-fcb8-49e7-80cb-0708ae16a186`修正README，明确默认列表两页、两分类列表、两个关键词仅搜索第一页、详情标题及选集名称、首集实际媒体与解码，其他字段及平台未单独认证。将该Run三个文件原样复制到新中文目录，仅用Python标准库及声明的已安装原生依赖启动，规则、媒体、桥接故障输入及关闭均通过；产物哈希未变。来源、原始哈希及全检查见hub-delivery-p2-coached-v7-package.json。P2整体仍等待3.9的三次独立Run。

预算修订见hub-delivery-budgets-v2.json：32轮独立样本已自行修复真实分类错误，最后一轮完成全部受信任验证，但没有余量更新进度和交付结论，因此最终failed。该原判保持；v8只调整规则类预算到48轮/1500秒，运行前重新冻结，不降低A–E验收。其他类型仍沿v1预算，不由增加预算宣称速度或泛化提升。

上述v2声明后续被实际快照反证：运行时默认硬上限40，limit_model_turns只能收紧，v8从未真正生效为48轮。现行预算为hub-delivery-budgets-v3.json的40轮/1500秒；启动器拒绝超过40，认证器从首轮worker快照核对有效预算。没有扩大运行时硬上限。v8原始配置及失败记录保留，不能当作48轮样本。

v9针对实际断点修订Skill：本地已识别应用优先走已核实ABI与精简PC入口契约，先写三个文件再验证，避免pip/临时目录/全仓扫描及重新定位裁判。PC契约只描述生产库接口，不包含最终业务实现。技能全部相关文本资源、Node/Python程序、前端依赖清单均纳入本轮指纹；v8未单列的引用资源也已与P0基线核对，26项未变，记录另存hub-delivery-p2-v8-skill-resource-baseline.json。v9首轮实际上下文包含新Skill，且实际调用skill_tool读取了pc-native-rule-contract.md与原探针，未将技能名称注册当成消费证据。

v9后定位到Skill模板自身的契约漂移：标准HTML模板使用普通`[网址]`首页后缀、search_rule、首页request(input)及旧lazy:前缀；主Skill还强制虚拟URL和额外请求头，与PC精简契约冲突。实际Hub产物的坏地址返回500，移除后缀的统一分页地址返回200及10条数据，见hiker-template-native-url-forensic.json。宿主只修改通用模板和技能文档，未修补该Run的实际业务产物。

三份通用模板已同步改用统一分页、MY_URL、searchFind和inline @lazyRule函数源码；未解析媒体显式失败，移除默认强加请求头。静态校验补上searchFind/search_find的脚本检查。生产运行库HTTP夹具覆盖首页两页、搜索、详情、实际lazy及无播放器配置负例，2组测试全过，见hiker-template-contract-final.log；旧失败和首次静态占位域名误命中结果均保留。主Skill三份及生命周期引用同步校正PC参数和平台适用范围。v10把这些通用资源与协议回归脚本一起冻结，不把仿真夹具作为原目标业务成功。

另确认既有SDK interact方法仍发送steeringPrompt，而后端要求response/action；对应形状实际返回422，正确形状已验证可被worker消费。见hub-interact-existing-contract-gap-20261102.json。该消费方契约问题记入P6后续，不把本次直接HTTP教导冒称用户前端交互已修复。

v10首个独立Run（73dcb326-a024-4b83-9568-d520f6c0a373）在40轮、1047402毫秒后失败，仅写出bridge.py，不能计为成功。运行期间生产hk-db.js被并发开发修改，冻结输入校验如实拒绝。另从该Run原始tool-result复现：包含filterState的源码输出有SmartLog omitted标记，没有full_result_reference，关键expandRuleUrl签名未交给模型。Shell在Gateway持久化前压缩，长度低于Gateway阈值时原文丢失；P1 2.5重新打开补修，不归咎于模型猜错后继续盲跑。

后续独立测试改用生产源码等哈希快照：复制技能、验证器、候选程序、生产native模块和SDK代码，逐文件记录原位置、执行位置及相同SHA；不复制模型设置或用户数据，node_modules维持明确引用。验收执行冻结副本，原开发树后续修改不会使本轮换成其他源码。验收标准、目标、预算及三次独立条件不变。

原文补修版候选为86C649E5CD23AD993A6343EEEE2FAF7F78CF20CA56D4B040030EFF013EDEC93D；Rust528项通过。v11快照919项，生产模板运行库协议测试2组通过；但自动启动调用旧Windows PowerShell，找不到Get-FileHash，在提交任务前失败，保存series.json为基础设施失败，不计模型Run。启动器编排改为优先使用本机pwsh，v12重新冻结919项并开始三次顺序独立试验，每次新服务及经验根，失败停止本系列并保留分母要求，不跳过失败后拼接成绩。

v12首个Run f50d2343-80f6-4c2a-a98d-5813fe379ab3首次独立完成：38轮、1291789毫秒，规则、上游数据、真实媒体解码及冷目录均通过，未发生源漂移。但人工源码复核发现桥接--host可接收任意地址，违反仅127.0.0.1契约。新增无网络绑定的拦截检查实际观察其尝试0.0.0.0，判定完整交付未通过；原自动通过报告不覆盖，另存manual review。第二次Run f0df6b27-17d7-45ca-8237-02090d053b42通过SDK取消并保留cancelled；第三次未执行，不能计三连成功。

补强CLI覆盖地址负例，默认必须尝试本机绑定，任意地址覆盖须拒绝或仍绑定127.0.0.1。检查在socket.bind之前中止，正负夹具及无关启动错误三组测试通过，旧合格桥接拒绝--host的正例也通过；宿主没有修补实际bridge.py。技能补充该约束及禁止把本机桥接直接宣传为手机局域网方案。后续顺序编排在每个Run后等待人工来源及说明复核，不再预先启动下一次。v12宽范围扫描曾递归进入依赖目录而超时，快照改以NODE_PATH显式引用已安装依赖，移除源码树内可遍历的node_modules连接；生产模块保持逐文件等哈希副本。

v13冻结921项，生产模板回归2组通过；首Run 98433cf5-f342-4f46-a37f-9b9e3c3d07ff以39轮、1500490毫秒总超时结束，外部独立规则/解码验证通过，但Run完成门槛未满足。此轮桥接已显式拒绝非本机地址，另复现P1声明反馈断点：plan_task确认了与宿主不符的字符串目标，verify_delivery才返回不含字段的笼统错误；模型随后反复排查HTTP超时与验证器位置，没有修正声明。两种路径都解析到同一实际安装目录，产物声明经默认值规范化后完全相同，见hub-delivery-p1-plan-path-alias-v13.json。原先forensic中的原始字典不等仅因README validator缺省与null写法不同，不是产物要求实际变化。

P1 2.2重新打开补修：Run绑定plan_task须先校验再确认，返回具体字段；已存在绝对路径解析为同一实际目标时统一回宿主原字符串。URL、不同目录及不可解析目标仍严格拒绝，不降低原目标指纹与验证标准。本轮没有重开后两次或拼接其他版本成功。

P1源码回归532通过，2.2恢复关闭；因磁盘满失败的编译记录保留，Cargo按当前包清理19.4GiB后成功完成回归，并重建两个原有可执行目标。构建身份及下一组业务结果另存。

另从v13实际工具记录观察到慢请求约8.128秒才关闭。旧桥接验收在完成四个并行请求后才开始计8秒，可能误通过恰好8秒的服务器配置。已修正为从发送不完整请求起算总期限，并行请求耗时扣除；v13同一未修改桥接被新检查拒绝（hub-delivery-p2-eight-second-negative.json），旧合格5秒桥接通过全部桥接检查（hub-delivery-p2-total-deadline-positive.json）。原报告不追改；新Skill明确建议5秒读取时限预留余量，后续使用重新冻结的验收器。

新候选Hub SHA为262E7EED1225CEFD5077E8C3D64A15E97DDF59D5A6C7B459EC6351E705D739CB，hub与superagent均构建成功。v14重新冻结921项及修正后的桥接验收，生产模板运行库测试2组通过，开始新三次独立系列；预算仍40轮/1500秒，失败不与旧版本成功拼接，逐Run人工审查后才继续。

v14首Run 68901ba9-92e6-45b3-9efd-73e23f072283已自行生成三个文件，外部规则/解码通过。人工预审发现测试器误拒合法CLI形式：桥接在非本机参数下明确打印拒绝并sys.exit(1)，检查器只接受argparse的exit 2或未捕获的明确异常。该项属于验收器错误；本轮经SDK取消并保留cancelled，不算Hub自主能力失败或三连成功，后两次未执行。补充检查已证明该桥接实际未绑定网络，修正后认可明确非零CLI拒绝；默认参数仍必须先通过本机绑定正例，非本机绑定尝试仍被拦截并判失败。四组回归包含exit 1合法拒绝、默认直接退出不合格、exit 0不合格及原任意地址负例，全部通过。原判不覆盖，完整冷包补验另记为基础设施纠正后的补充证据。

v15验收器正例先通过：对v14三个Hub原文件，在冻结924项的生产运行库及修正验收器中冷启，解码time-pos=1.082秒，完整慢请求5009毫秒关闭，四并发、CLI范围、错误输入、关闭全部通过。此前手工补验未传NODE_PATH而失败的记录保留；新冻结oracle会从已指纹固定的frozen-source.json自行恢复依赖引用，缺结构化Node输出也返回明确依赖诊断。该正例不是新的独立Run成功。

v15首独立Run d479a294-f5e1-4043-9447-d6bd6c7665ac仍以40轮、1138602毫秒failed结束。新计划门禁已实际指出README validator被填为字符串null，模型随后修正计划并完成探针步骤；但规则验收又被文字匹配误拒：README明确写“未在 Android 原生移动端上独立运行验证”，旧检查仅接受连写“未验证”等标记。该自然语言判断过窄属于验收器缺陷，原failed不改写、不计独立成功；Hub在错误后无效定位测试器的工具调用仍全部保留。

已把移动端边界检查限定到包含手机/Android的同一语句，并接受“未在…验证”、待实测及英文not been tested等自然表述；同时拒绝“Android已验证，其他项未验证”这种无关否定混入。两组13条正反文字样例全部通过，真实HTTP空流、垃圾媒体及410过期负例回归也通过。后续重新冻结，不回填v15成绩或只靠放松文字匹配宣称业务成功。

v16补验发现v15产物除文档检查误拒外，还存在实际lazy序列化错误：详情脚本再次求值时消耗正则反斜杠，生成的脚本出现双斜杠注释。只读定位报告hub-delivery-p2-v16-nested-lazy-forensic.json记录原文件SHA、实际解析错误及源文件未修改；不把宿主诊断当作Hub修复。v14原文件在v16验收器中完整冷启通过，见hub-delivery-p2-v16-known-positive-package.json，仍不是新独立Run成绩。

续接时v16纠错测试的服务及重放进程均已退出，原报告停在submitted；其工作区及原样复制项目均不再包含三个交付文件。另存hub-delivery-p2-coached-v16-interruption.json记录观察和原报告SHA，不覆盖原记录、不猜测删除原因、不将中断算成功。模型代理8045已连通。无可复测原文件，重新从空项目开展独立生成。

v17只补充通用嵌套脚本指导：检查详情实际生成的lazy，使用函数源码和JSON序列化保持边界；未将目标业务实现写入Skill。冻结925项，候选仍为262E7EED…39CB，预算40轮/1500秒。沿真实SDK在39201开始新独立系列，每次全新Session、项目及经验目录，逐Run冷包和人工审查后才继续；旧失败不与新版本拼接。

2026-11-03续验：v17独立Run d26b4b87-dbce-45a3-b732-2d0cf10a5839以40轮、1426143毫秒failed结束，外部原版验收同样未达1秒解码。Hub自行写出三个文件并从PowerShell内联命令转义错误恢复，但浪费了较多轮次；没有宿主修改业务源码。三个文件按原始SHA完整复制到artifacts/hub-delivery/p2-v17-d26b4b87，provenance.json记录失败身份和原样复制，不作为独立完成样本。

只读补充诊断分别记录HTTP206/2048字节、解码未初始化、0.322秒停滞及同URL再次真实解码1.082秒。原生video.play与实际规则lazy返回的URL和headers完全相同；原生直接解码也出现失败，见hub-delivery-p2-v17-mpv-diagnostic.json。直接原生探针还遇到TLS握手超时。由此确认暂时性媒体/环境问题参与失败，不能把问题全部归于模型生成错误，也不能据补测成功回填原failed。

验证器增加安全诊断和预先冻结的最多三次同URL解码，每次仍为16秒且必须单次真实超过1秒，不改影片、不累计进度、不扩大Run或验证总时限。全部decoder_attempts在成功/失败时保存。五项回归通过，包含实际垃圾媒体三次仍失败、原地址不变、保留失败和不足1秒不可累计；日志hub-delivery-p2-bounded-decode-tests.log。Skill补充Windows生成脚本先落盘、避免检查已知运行时及0.2秒探针与1秒正式验收的区别。

v18仅完成冻结、未提交模型任务。v19冻结926项，仍用262E7EED…39CB和40轮/1500秒。原样v17产物在新中文冷目录完整通过生产规则、两分类/两关键词/分页/选集、真实1.042秒解码、CLI地址守卫、四并发、5003毫秒慢请求关闭、错误输入及端口回收，见hub-delivery-p2-v19-known-source-package.json；不计独立生成成绩。v19在39211启动新的顺序三次独立系列，3.9继续待验。

v19首Run 0572c601-a95e-4710-9d91-bffeb7a6a777在29轮completed，外部规则及完整冷包通过；但人工源码审查发现首页find_rule用默认端口MY_URL覆盖了bridgeBase。只读生产运行库备用端口负例返回home must contain real content，三个文件SHA不变，见hub-delivery-p2-v19-alternate-port-negative.json。人工review1明确false，系列停止，后两次未执行；不把自动成功当成完整交付。原三文件另存artifacts/hub-delivery/p2-v19-0572c601。

v20将备用端口实际运行纳入固定验证：从原规则URL识别默认值，选不同许可端口，用实际MY_PARAMS.bridgeBase执行全链路，原JSON不改写。成功报告记录两个端口，失败带端口上下文；认证器同时核查Run与冷包，不接受相同端口或缺失解码尝试。七项回归通过，见hub-delivery-p2-alternate-port-tests.log。v17原样产物在v20完整冷启通过，默认18002、实际18001、真实解码1.082秒，证明该检查并非不可满足，见hub-delivery-p2-v20-known-positive-package.json。

v20纠错反馈只提供上述真实断点及文档范围，不提供替换业务源码；Hub基于自己v19三个文件的原样副本执行修复。其自建Node模拟器曾因函数无return而把undefined当成生产lazy失败，补充教导通过实际interact接口response/action=steer送达（200），后续真实模型messages确认消费，反馈文件和SHA独立保存；不冒称现有前端SDK交互字段已修复。

纠错Run 11f6e46f-b82f-4161-a0b4-84aeb0ba99fe在32轮、980171毫秒completed，外部验证通过；仅复制三个文件的新中文目录也完整通过，默认18002、实际18001、真实解码1.082秒。源码及README由Hub修复，宿主只做原样复制和固定验收，见hub-delivery-p2-coached-v20.json、-package.json、-review.json。永久副本位于artifacts/hub-delivery/p2-v20-corrected-11f6e46f。每attempt分类corrected，不计3.9首次生成；同一v20冻结版在39231启动全新空项目三次顺序独立系列。

v20独立首轮84de916d-d8c2-472b-a567-31275784a121在21轮完成；固定裁判及完整冷包均通过，原默认18002/实际18001、真实解码1.082秒。三个产物和工具来源已人工复核，README明确PC依赖及Android边界；规则以MY_PARAMS.class/fyclass处理PC分类，不要求额外的MY_URL-only入口。桥接使用顺序HTTPServer及5秒socket超时，四客户端与不完整请求在既定8秒上限内通过，不认定并行吞吐。review1.json通过后才启动独立第二轮282b7d1b；原始三文件等哈希副本保存在artifacts/hub-delivery/p2-v20-independent-1-84de916d。当前完整成功为1/3，3.9仍未完成。

这是Windows本地原生依赖方案。Android独立运行、用户前端窗口、完整安装包及P3～P7均未认证；没有发布、上传、推送设备或替换用户安装版。
