# Skill 执行链路与本地应用无界面验收

## 结论与范围

本轮修复自动技能挂载、显式读取、真实资源路径、技能校验收据和任务收尾，并重放前端同款 HTTP / SSE 请求。没有桌面操作，没有覆盖根目录既有 dmghg-rule.json，也没有将其他网站的规则冒充安装应用规则。

## 已复现的问题

1. Scout 会找 skills/，skill_tool 只找 .agents/skills；同名技能占用两次挂载名额。多行 description 被解析成 `>`。
2. 注入固定 super-agent/skills/... 相对命令，换工作区后路径失效；全局技能 references 不能通过工作区文件工具读取。
3. 实际 validate.cjs 退出 0，但质量命令识别没有覆盖该类技能脚本。Markdown Todo 与结构化 TaskContract 的完成指令冲突。
4. Electron 默认 hub_data 没有配置密钥，而既有 .superagent 配置有效；SDK 将无 accounts 的单模型配置全部过滤，前端可能把有效服务误判离线。
5. 发行打包没有同包携带 skills，且收集新 Hub 后未同步本轮 ASAR staging。
6. 最终 SDK 接线重放走到结构化计划分支，成功的 PowerShell/Python 命令输出被 TaskProgress 忽略（ProcessControl 不是已支持的证据种类），模型拿不到可引用的收据 ID，诊断请求在 180 秒截止时超时。该失败另存为 local-hub-sdk-initial-timeout.json。已为有真实前台 exit_code=0 的命令记录 Observation，不将其提升为 Mutation/Verification；最终 SDK 重放使用 300 秒截止，产品默认任务截止仍为 1800 秒。

## 修复

- 共用技能发现和确定性同名优先级；挂载、list、read 指向同一绝对路径。补充运行包、用户目录和多行描述。
- 仅列出所选技能中真实存在的脚本。skill_tool read 支持资源相对路径，限制在该技能目录内并保留大小上限。
- 直接前台运行明确命名的校验脚本可获得验证收据，仍须实际 exit_code=0，仍拒绝帮助、复合命令和后台执行。收据只说明校验执行事实，业务正确性另验。
- 简单单产物直接执行和验证；复杂任务使用结构化计划及匹配收据。Todo 仅为可选笔记。
- 成功的前台命令输出可作为可引用观察事实，失败和后台启动不能；命令输出本身不能证明写盘或测试通过。
- /v1/models 增加 configured/configured_default 元数据，SDK 允许完整的单模型配置并优先默认模型；不制造账号来源或健康成功。
- skills 与 Hub 同包进入 bin/skills、ASAR staging 和 zip。没有执行发布/上传操作。
- 新增 local-app-inspector 技能，仅对已验证的 DMGHG v1 协议提供原生桥接适配。未知 DLL 不猜测调用。

## HTTP 实测

模型请求使用 gemini-3.8-flash-high，沿 POST /api/agents/run → 真实 worker → SSE 路径，无浏览器/桌面。密钥只从既有本地配置读取，不复制到夹具或报告。

- 原始失败：[第一次重放](2026-10-30-skill-worker-http.json)。两个任务均耗尽 12 轮；生成夹具已落盘并验过，但未正常结题。保留失败，不将“有文件”计作任务成功。
- 修复后：[原始复测](2026-10-30-skill-worker-http-retest.json)及[独立审查](2026-10-30-skill-worker-http-reviewed.json)。两条任务均正常 completed。
- 生成场景是明确的合成夹具：模型实际读取外部 Skill、执行 generate.cjs 和 validate.cjs，独立检查随机标记和产物内容。它证明调用/生成/校验/结题链路，不认证任意业务规则质量。
- 安装应用场景使用用户自然请求，没有在请求中指定工具名。模型自动调用随包探针、读取报告并交付结论；独立检查目标路径、DLL 哈希和实际解码输出。

## 动漫共和国的实际结果

[脱敏探针收据](2026-10-30-dmghg-headless.json)：安装版本 1.4.4，真实返回 8 个频道、10 个列表项；搜索“仙逆”有 3 项，详情有 164 集；解析到 HLS 并请求媒体数据。随后使用安装目录中已有 libmpv，强制 vo=null/ao=null，无窗口、无声音，读到 1920×816 视频参数、AAC 和超过 0.2 秒的播放进度。

这验证了 Windows 原生桥接方案可行，仍依赖 electron_bridge.dll、resources/lua 和本机网络。未实现/交付手机独立 Hiker 规则，没有手机端实测。根目录现有 dmghg-rule.json 使用其他网站，不属于这个安装应用的转换验收。

复现命令：

```powershell
python skills/local-app-inspector/scripts/dmghg_probe.py --app-dir 'D:\太极\动漫共和国概念版' --output scratch/dmghg-evidence.json --decode
```

探针为每次调用建立新的临时状态，不读取或改写原应用账号与会话；报告不包含鉴权值或签名媒体链接。

## 本地接线

[配置调整收据](2026-10-30-local-hub-config.json)：当前开发版 Sidecar 使用已有 .superagent 数据目录，复用原凭据且不复制密钥；默认模型及快模型改为用户指定别名。原空 hub_data 保留。最终服务与模型列表验证见本轮服务收据。

## 验证边界

本轮只认证列明的脚本夹具和该安装应用样本。没有执行 UI 自动化、完整安装包构建、手机规则导入、付费内容处理、云发布或局域网推送。其他预先存在的播放器与规则改动不属于本轮认证。


## 工程检查

- Rust 库回归：514 passed，0 failed，2 ignored；排除会审查整个工作区的既有 code_review 测试 1 项。
- 前端相关 Node 回归：11 passed，0 failed。既有录制测试补齐缺失的 image-compressor mock 后通过。
- TypeScript：npx tsc --noEmit --incremental false 通过。
- OpenSpec：10 passed，0 failed；变更未归档。
- 原始技能回归曾因路径分隔符断言失败；断言按真实 canonical 路径统一后通过，不把此测试缺陷作为产品修复证据。


## 临时文件说明

曾尝试清理本轮生成的 scripts/__pycache__/dmghg_headless_probe.cpython-314.pyc，自动审批以策略限制拒绝该删除。未重试或绕过；缓存保留且发行资源过滤器明确排除 __pycache__/*.pyc，不影响运行与验收。


## 最终 SDK 验收的校正记录

- ProcessControl 观察修复后，正常端口上的任务已 completed；验收脚本最初误把先前的 Test-Path 当成探针执行，未定位真实 output 文件，造成 passed=false 假阴性。该原始报告保存在 local-hub-sdk-oracle-initial.json；新增调用定位回归，要求真正带 --app-dir/--output 的执行，未修改 completed 终态或数据检查条件。
- 模型该次答复仍将一次抽样泛化成“所有 API/分页/稳定可用”。已在 local-app-inspector 中明确只测第一页、一个关键词与首集，未测分页、持续稳定性、HTTP 桥接服务、Hiker 规则或手机运行；使用相同请求复测答复边界。原始过度表述保留在上述报告中，不采用它作为能力认证结论。


## 最终接线收据

[本机真实 SDK 最终请求](2026-10-30-local-hub-sdk-final.json) 为 completed 且 passed=true；SDK 从实际 /v1/models 选择 gemini-3.8-flash-high，真正调用探针，独立检查所选 DLL 哈希、输出文件、无界面标志、实际视频解码和手机未验证标志。运行服务保留在 127.0.0.1:3001，开发 Sidecar 配置已指向同一份已有模型配置。

最终模型答复已明确第一页、单关键词和首集的抽样边界，但仍出现对 DLL 实现语言、手机 JS 引擎和移植路径的无依据推断。增加了相应技能纠偏，并进行明确提供反馈的复核；该复核不当成未经提示的通用智能评分。具体以探针结构化数据及源码事实为准，不能将模型措辞视为完整协议认证。


## 只读复核门禁回归

明确反馈的首次复核任务返回 failed（local-hub-coached-review-initial.json）：运行时将“只做复核”误分为执行任务，要求无关写盘。新增“只做复核/只做审查”调查范围识别与回归，不以勾选清单或放宽执行验证绕过。修复后使用同一复核请求再次验证。


补充网关验证：[真实 LocalGateway 类与 SDK](2026-10-30-local-gateway-models.json) 在独立无界面 Node 进程中转发到本机 Hub，无令牌请求为 403，带该测试网关会话令牌的真实 SDK 正确获得默认模型。令牌仅留内存，没有写入报告。这验证生产网关组件，不声称自动操作过 Electron 窗口。

最终运行身份见 [当前服务](2026-10-30-local-hub-ready.json)。SDK 主调查先通过，随后只读复核范围识别更新另行编译与测试，两个版本的真实二进制哈希分别保留，不回填替换早期执行身份。


纠偏复核最终 completed：[明确反馈后的复核](2026-10-30-local-hub-coached-review.json)。模型读取已有证据而未重跑探针，按两段答复纠正实现语言、手机引擎、LAN/HTTP 服务及移植路径推断，并明确只测样本、尚未生成规则。它是带明确反馈的纠偏成功，不作为未经提示的普遍推理能力证明。
