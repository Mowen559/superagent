# 移动规则执行证据与版本绑定候选经验

状态：candidate_unpromoted。人工监督提炼，尚未做未知目标迁移，不自动更新生产Skill或声明模型已学会。

日期：2026-11-05。

## 可复用的方法

- 将规则入口独立编译、生成lazy编译、宿主注入后的eval、真实媒体执行和Android实机分开。任一前置检查通过都不能替代后续检查。
- 为每份证据记录规则哈希、解析器哈希、实际APK哈希、来源及验证范围。参考源码、备份目录名、公开JAR不能直接视为当前APK或手机版本。
- 只依据当前生成动作测量URL体积，不能拿整份规则库长度、旧失败版本或印象代替当前数据。
- 子方案涉及常驻函数、preRule作用域或解密代码精简时，将这些写成待验证假设，不能宣称已经无缝共享或保证具体缩减行数。
- 对嵌套eval检查当前页面对象而非只检查完整规则包；分界标记也可能出现在被引用的模板字符串中。捕获真正传给eval的源码、记录绑定字段及报错行；修订后从真实页面再次生成动作，不直接复用旧动作作为通过证据。
- 通过登记资源的状态/哈希寻找解析器，不递归猜测工具安装路径；默认公开解析器只作对照，不冒充手机内置版本。
- 命令成功与安全风险不等于文件改变。进度证据须保持二者分离；潜在写命令依旧失效旧验证，实施进度须真实编辑或可信变更证明。
- 连续观察停滞时要求明确剩余假设与区分性检查，允许诚实报告未复现。提醒不是根因证据，也不能迫使无依据编辑。
- 用户要求先调查、看方案后再改时，当前阶段不能被“修改方案”中的动词错误提升为实施任务。当前词面路由仍有此缺口，已用显式阶段澄清绕开，未宣称核心语义已解决。

## 来源证据

- docs/reviews/hub-dmghg-mobile-repair-review-v1.json：完整原始失败、宿主参与、当前修订状态。
- artifacts/dmghg-android-feedback-20261104/process-evidence-tests-final.log：573项通过，覆盖进程观察、真实编辑、验证失效与结构化Mutation绑定。
- artifacts/dmghg-android-feedback-20261104/inspection-plateau-tests.log：577项通过，额外覆盖调查范围、停滞提醒及真实进度/答复重置。
- artifacts/dmghg-android-feedback-20261104/original-mobile-syntax.json 与 original-rhino-1.7.14.json：同规则在两个公开/仓库解析器中的不同编译结果，均非手机实机认证。
- artifacts/dmghg-android-feedback-20261104/apk-engine-fingerprint.json：用户指定APK的静态类存在性证据；不等价于已确定运行时配置。
- docs/reviews/hub-dmghg-apk-proposal-v1.json：Hub实际反编译APK并提出方案，报告仍有待复核表述，不作为已修复证据。
- artifacts/dmghg-android-feedback-20261104/hub-mobile-candidate-v2/manifest.json：监督者提供根因证据后Hub实现修订，真实PC与当前页对照编译检查通过，手机实测待反馈；不能证明无引导生成或未知目标迁移。
- artifacts/dmghg-android-feedback-20261104/mobile-diagnostics-final-tests.log：11项移动语法、当前页捕获及资源发现测试通过。

## 晋升门槛

另选未参与提炼的移动规则及非规则调查场景，真实验证版本归因、跨Run来源、无假修改、阶段澄清与用户选择；记录模型/工具版本及原失败。未获得迁移证据前保持候选。
