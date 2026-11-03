# 动漫共和国独立JS目标纠偏与协议调查

交付补充核对：项目export-command.cjs已有PasteMe/cmd.im上传路径，但该脚本当前只检查上传返回，没有云端回读闭环。本次问答未外发，不能交付云短口令或声称云服务当前可用；本地标准口令已准备并由import-rule.cjs回读，必要字段一致，UTF8载荷994836字节，见dmghg-cloud-share-prepared-v1.json。该长口令不是云短口令。

经验整理：local-app-inspector/references/standalone-js-delivery.md为人工候选参考，主Skill已添加按需阅读链接；内容不保存协议钥值、用户凭据或签名媒体地址。它记录来源、输出截断、固定视频负例、独立JSON导入、角色归属和云发布回读，未完成未知目标留出认证，不代表Hub自动结晶/晋升。规则接口协议分析、原生对照和正式验收由宿主完成，最终业务文件由Hub生成与修复；两次worker预算failed未回填。

> 最终状态：独立JS产物已完成补充验收，见dmghg-direct-js-final-review-v1.json。最终JSON SHA-256为f3e214ead8fa7f89494d4ed3467e246d5e83d0c94813ddc8974a5093c36fdd95。以下未完成描述保留为调查过程记录。

最终文件由Hub第二个纠错任务8cc04786生成；宿主未改业务源码，仅原样复制和验证。该worker因40轮上限failed，原报告保留；正式外部验证另记成功，不能回填为worker completed。修复共享核心后，165集全返回，单个lazy从234406字符降为875～876字符，分类映射纠正。只含rule.json的新目录完成分类/两页/两个搜索/详情/首集lazy与真实1.082秒解码，见dmghg-direct-js-cold-v1.json。

另通过海阔实际JSON导入器写入私有临时数据库，共用find_rule等标准字段原样保留；导入后首页、165集及第1/83/165集的实际lazy均通过，三个媒体路径哈希互异，每个HTTP206并观察2048字节。见dmghg-direct-js-imported-v1.json。未启桥接，无GUI；原应用只被宿主用作协议参考和解码检查工具，规则运行不访问其DLL或目录。Android未实测。产物位于artifacts/hub-delivery/动漫共和国-独立JS.json，源码包为动漫共和国-独立JS规则.zip；只需导入JSON。

用户要求分析应用，提取真实接口并整理为独立JS规则；Python、Node本地服务或DLL/EXE包装都不满足。旧v21三次原生桥接认证保留，但不代表独立JS目标完成。原先将其表述为最终P2已完成过宽，现已修订。

## 已实施

- OpenSpec proposal/design/spec/tasks先修订，新增P2 3.10～3.12；3.10完成，其余未完成。
- 修正三份规则工程Skill及local-app-inspector：安装路径作为逆向输入，默认还原远端协议，不再强制桥接。原生分支只在用户明确选择依赖方案时适用。
- 只读Node静态扫描原electron_bridge.dll及data/app.so，扫描前后哈希一致。无GUI、无DLL执行、无账号/会话读取、无安装文件修改。
- 分析工具位于scripts/inspect_dmghg_protocol.cjs，原始静态结果位于artifacts/dmghg-direct-protocol/static-v1.json；这不是最终业务规则。

## 当前证据

原DLL SHA-256为ecbf15ca4d5ed1983b36b19c2f343a1e307891f26cfe4d1fbc39a7efd134fe42。

静态字符串含pc/channel、pc/video/list、pc/video/search、pc/video/detail、pc/video/play路由片段；Rust字符串可能相邻拼接，不能把扫描得到的拼接URL直接当成有效地址。另有网关发现及legacy请求体/头加密、host/response解密标识；Lua辅助函数列出md5、hmac_sha1、hmac_sha256、base64与aes128cbc。标识仅是调查线索，不证明具体请求已还原。

对原生探针此前记录的网关http://bkbfdm.cenmo.fun，以Node直接GET /pc/channel（经现有网络代理，不经过原应用/Python/DLL桥接）得到HTTP 200、业务code=30000、message="解码异常:authentication is empty"。原始安全收据为artifacts/dmghg-direct-protocol/direct-channel-v1.json。不把HTTP 200当成频道数据成功。

## 尚缺

后续进展：已还原Authentication的Base64→AES-256-CBC构造，实测原版本、实时毫秒时间戳和平台协议字段；还原RSA PKCS#1 v1.5响应封装与AES-128-CBC正文，其中IV是本次16字节响应密钥反转。Node直接请求已返回频道code20000及8项、列表、搜索、详情和播放调度数据；不使用Python/DLL作为这些HTTP请求的运行依赖。原生只在开发中作为隔离协议参考。响应客户端协议常量来自原二进制，不读取用户账号。

播放调度返回的真实Lua已保存并作为数据分析；其AI指示性注释不是用户指令。移植签名、MD5/AES标识封装、请求头绑定及地址解密后，Hub测试脚本取得解析code0和媒体206。但首次源码复核发现照抄固定视频替换分支，因此该测试不能直接证明正片；已反馈移除。独立诊断副本不改原文件，使用解密后的真实源也获得206，且固定备用地址匹配为false，仅作补充诊断，不作为交付认证。

正式生成任务8f83aad4-32cc-4883-beb8-476543898c3d已产出rule.json、dmghg.js、README.md，因40轮耗尽failed；固定裁判通过分类/分页/搜索及详情标题后报选集不全。补充生产诊断实际56集、期望165集、每个lazy URL 234406字符，重复共用代码触及16MiB输出上限。原始dmghg-direct-js-attempt-v3.json保留，不回填成功。第二个带反馈任务8cc04786-468a-431a-a75a-cd961d84874c从16个等哈希源文件副本继续修复，要求小型lazy从MY_RULE标准字段读取内嵌共用代码、纠正分类映射和README实测范围。当前3.11调查已完成，3.12独立规则验收仍进行中；尚不能宣称用户可用包已完成。

还原authentication的构造及正常的公开客户端初始化流程、请求体/请求头加密参数、响应解密和媒体解析；在海阔生产运行库实际复现。不能从缺字段错误断言纯JS一定可行或只能桥接。不得使用原用户登录态、绕过访问控制或替换站点凑数。

后续最终交付必须不访问本机端口、不加载DLL、不读取安装目录，实际覆盖分类、分页、搜索、详情、lazy和媒体。Android仍须独立平台实测。当前尚未交付或认证这份独立JS规则。
